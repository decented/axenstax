// Resilient live feedback subscriber — raw WebSocket REQ (the plain NIP-01
// shape every relay we use parses cleanly, unlike nostr-tools' framing), one
// socket per inbox relay, auto-reconnect, keepalive so the process never
// silently exits. Reuses the reader's decrypt + ledger pipeline.
import { homedir } from 'node:os';
import path from 'node:path';
import { loadAxenstaxSigner } from './lib/signer.mjs';
import { Ledger } from './lib/ledger.mjs';
import { ingest } from './lib/reader.mjs';
import { makeRelay } from './lib/relay.mjs';
import { relaysFromEnv, makeAutoReceiver } from './lib/board.mjs';

// The project's inbox relays by default; RELAY=url or RELAYS=a,b overrides.
const RELAYS = relaysFromEnv();
const KEY_FILE = process.env.KEY_FILE || path.join(homedir(), '.config', 'axenstax', 'axenstax-official.json');
const LEDGER = process.env.LEDGER || path.resolve('feedback-ledger.jsonl');

// --auto-received: mark each newly ingested report `received` on the public
// status board (default off; spec S8). Hashes only — see status.mjs.
const autoReceived = process.argv.includes('--auto-received');
const BOARD = process.env.BOARD || path.resolve('feedback-status-board.json');

const signer = loadAxenstaxSigner(KEY_FILE);
const ledger = new Ledger(LEDGER);
const auto = autoReceived
  ? makeAutoReceiver({
      file: BOARD,
      signer,
      relay: makeRelay(RELAYS),
      log: (m) => console.log(`[${new Date().toISOString().slice(11, 19)}] ${m}`),
    })
  : null;
const hex = signer.pubkey;
const resolveHandle = async () => null; // embedded send-time handle is preferred anyway

const ts = () => new Date().toISOString().slice(11, 19);
// handle/origin(client)/personaNpub are sender-asserted tags — never trusted
// as a proven identity (2026-09-27 audit, REVIEW-W6 should-fix #2). Only
// fromNpub is cryptographically known.
function whoLine(rec) {
  const handleLabel = rec.handle ? `${rec.handle} (unverified) · ` : '';
  const client = rec.origin ? ` · client=${rec.origin} (unverified)` : '';
  const persona = rec.personaNpub ? ` · persona=${rec.personaNpub.slice(0, 12)}… (unverified)` : '';
  return `${handleLabel}${rec.fromNpub.slice(0, 12)}…${client}${persona}`;
}
console.log(`live reader: AxeNStax ${hex.slice(0, 12)}… on ${RELAYS.join(', ')} (raw WS, auto-reconnect)`);
setInterval(() => {}, 1 << 30); // keepalive: never let the event loop drain

// The same report usually arrives from every inbox relay; handle it once.
const seen = new Set();

function connect(url) {
  const ws = new WebSocket(url);
  ws.addEventListener('open', () => {
    console.log(`[${ts()}] ${url} connected — subscribing kind-1059 #p=${hex.slice(0, 8)}…`);
    ws.send(JSON.stringify(['REQ', 'fb', { kinds: [1059], '#p': [hex] }]));
  });
  ws.addEventListener('message', async (e) => {
    let m; try { m = JSON.parse(e.data.toString()); } catch { return; }
    if (m[0] === 'EVENT' && m[1] === 'fb') {
      const id = m[2] && m[2].id;
      if (id) { if (seen.has(id)) return; seen.add(id); }
      try {
        const rec = await ingest(m[2], { signer, ledger, resolveHandle });
        if (rec) {
          console.log(`📥 [${rec.type}] ${whoLine(rec)} (${rec.id}): ${(rec.body || '').replace(/\s+/g, ' ').slice(0, 90)}`);
          if (auto) auto.add(rec.id);
        }
      } catch (err) { console.warn('ingest error:', err.message); }
    } else if (m[0] === 'EOSE')   console.log(`[${ts()}] ${url} caught up on stored — now LIVE, waiting for new reports…`);
    else if (m[0] === 'NOTICE')   console.log(`NOTICE (${url}):`, m[1]);
    else if (m[0] === 'CLOSED')   console.log(`CLOSED (${url}):`, m[1], m[2]);
  });
  ws.addEventListener('close', () => { console.log(`[${ts()}] ${url} socket closed — reconnecting in 1s`); setTimeout(() => connect(url), 1000); });
  ws.addEventListener('error', () => { try { ws.close(); } catch {} });
}
for (const url of RELAYS) connect(url);
