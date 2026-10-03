#!/usr/bin/env node
// Dev reader (OWNER-RUN, holds the AxeNStax secret). Catches up on + then
// subscribes to inbound NIP-17 feedback DMs, decrypting each into the local
// ledger. The live run with the real key is the verification boundary.
//
//   KEY_FILE=~/.config/axenstax/axenstax-official.json \
//   LEDGER=./feedback-ledger.jsonl \
//   node read.mjs [--since <unix-secs>] [--once] [--reresolve-handles] [--auto-received]
//
// --once : catch up + exit (no live subscription).
// --auto-received : mark each NEWLY ingested report `received` on the public
//   status board (default off; see status.mjs). Never overrides a status
//   already set. Only scrambled ticket hashes are published.
// --reresolve-handles : fix handles on EXISTING rows from the persona credential
//   (kind-31000), then exit. Repairs rows logged before the signed-in-handle fix
//   (which read the player-editable kind-0). Embedded send-time handles are kept.
import { homedir } from 'node:os';
import path from 'node:path';
import { loadAxenstaxSigner } from './lib/signer.mjs';
import { Ledger } from './lib/ledger.mjs';
import { ingest, reresolveHandles } from './lib/reader.mjs';
import { makeRelay } from './lib/relay.mjs';
import { NT } from './lib/nostr.mjs';
import { relaysFromEnv, makeAutoReceiver } from './lib/board.mjs';

// The project's inbox relays by default; RELAY=url or RELAYS=a,b overrides.
const RELAYS = relaysFromEnv();
const KEY_FILE = process.env.KEY_FILE || path.join(homedir(), '.config', 'axenstax', 'axenstax-official.json');
const LEDGER = process.env.LEDGER || path.resolve('feedback-ledger.jsonl');
const args = process.argv.slice(2);
const once = args.includes('--once');
const reresolve = args.includes('--reresolve-handles');
const autoReceived = args.includes('--auto-received');
const sinceIdx = args.indexOf('--since');
const since = sinceIdx >= 0 ? Number(args[sinceIdx + 1]) : undefined;

const signer = loadAxenstaxSigner(KEY_FILE);
const ledger = new Ledger(LEDGER);
const relay = makeRelay(RELAYS);
const BOARD = process.env.BOARD || path.resolve('feedback-status-board.json');
const boardRelay = autoReceived ? makeRelay(RELAYS) : null;
const auto = autoReceived
  ? makeAutoReceiver({ file: BOARD, signer, relay: boardRelay, log: (m) => console.log('reader: ' + m) })
  : null;

function shortNpub(npub) { return npub.slice(0, 12) + '…' + npub.slice(-4); }
function preview(s) { return (s || '').replace(/\s+/g, ' ').slice(0, 60); }

// Resolve a reporter's SIGNED-IN handle (Signet persona credential, kind-31000)
// for the ledger. NOT kind-0 — that's player-editable via the Hash Dash board.
// Only used as a fallback when the report didn't embed the handle at send time.
const resolveHandle = async (pubkeyHex) => {
  try { return (await relay.fetchPersonaHandle(pubkeyHex)) || null; }
  catch { return null; }
};

// handle/origin(client)/personaNpub are sender-asserted tags — never trusted
// as a proven identity (2026-09-27 audit, REVIEW-W6 should-fix #2). Only
// fromNpub is cryptographically known.
function whoLine(rec) {
  const handle = rec.handle ? `${rec.handle} (unverified) · ` : '';
  const client = rec.origin ? ` · client=${rec.origin} (unverified)` : '';
  const persona = rec.personaNpub ? ` · persona=${rec.personaNpub.slice(0, 12)}… (unverified)` : '';
  return `${handle}${shortNpub(rec.fromNpub)}${client}${persona}`;
}

async function handle(ev) {
  const rec = await ingest(ev, { signer, ledger, resolveHandle });
  if (rec) {
    console.log(`📥 [${rec.type}] ${whoLine(rec)} (${rec.id}): ${preview(rec.body)}`);
    if (auto) auto.add(rec.id);
  }
}

console.log(`reader: AxeNStax ${signer.pubkey.slice(0, 12)}… on ${RELAYS.join(', ')}\n        ledger: ${LEDGER}`);

// Maintenance: re-resolve handles on existing rows from kind-31000, then exit.
if (reresolve) {
  console.log('reader: re-resolving handles from persona credentials (kind-31000)…');
  const resolveByNpub = async (npub) => {
    let hex = null;
    try { const d = NT.nip19.decode(npub); if (d && d.type === 'npub') hex = d.data; } catch { /* skip bad npub */ }
    return hex ? resolveHandle(hex) : null;
  };
  const r = await reresolveHandles({ ledger, resolveByNpub, log: (m) => console.log('  ' + m) });
  console.log(`reader: re-resolve done — ${r.updated} updated, ${r.looked} looked up, ${r.skipped} skipped (embedded/none).`);
  relay.close();
  process.exit(0);
}

const catchup = await relay.fetchGiftWraps(signer.pubkey, since);
console.log(`reader: catch-up — ${catchup.length} stored wrap(s)`);
for (const ev of catchup) await handle(ev);
console.log(`reader: ${ledger.newReports().length} report(s) awaiting triage (status 'new')`);

if (auto) await auto.flush();

if (once) {
  relay.close();
  if (boardRelay) boardRelay.close();
} else {
  console.log('reader: subscribing for new reports (Ctrl-C to stop)…');
  relay.subscribe(signer.pubkey, handle, since);
  process.on('SIGINT', () => { relay.close(); if (boardRelay) boardRelay.close(); process.exit(0); });
}
