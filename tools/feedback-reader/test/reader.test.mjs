// Dev-reader tests — all with TEST keys + a mock relay. The live relay run with
// the real AxeNStax secret is the owner boundary (see the spec); these prove the
// crypto + ledger + privacy logic solo.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { mkdtempSync, rmSync } from 'node:fs';

import { NT, nip59 } from '../lib/nostr.mjs';
import { signerFromSecretKey } from '../lib/signer.mjs';
import { Ledger } from '../lib/ledger.mjs';
import { ingest, reresolveHandles } from '../lib/reader.mjs';

// AxeNStax (dev) + a kid persona, both test keys.
const DEV_SK = NT.generateSecretKey();
const dev = signerFromSecretKey(DEV_SK);
const KID_SK = NT.generateSecretKey();
const kid = signerFromSecretKey(KID_SK);
const KID_NPUB = NT.nip19.npubEncode(kid.pubkey);

const RECENT = Math.floor(Date.now() / 1000);
function fakeRelay() {
  const published = [];
  return { published, async publish(ev) { published.push(ev); } };
}
function tmpLedger() {
  const dir = mkdtempSync(path.join(tmpdir(), 'axe-ledger-'));
  return { path: path.join(dir, 'feedback-ledger.jsonl'), cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}

test('ingest decrypts a kid report into a new ledger row', async () => {
  // Kid wraps a /bug report addressed to the dev (AxeNStax) pubkey.
  const relay = fakeRelay();
  const rumor = { kind: 14, content: 'doors too tall', tags: [['t', 'bug'], ['report-id', 'rep-1'], ['client', 'axenstax']] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  await relay.publish(gift);

  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    const rec = await ingest(relay.published[0], { signer: dev, ledger });
    assert.equal(rec.type, 'bug');
    assert.equal(rec.body, 'doors too tall');
    assert.equal(rec.status, 'new');
    assert.equal(rec.verdict, null);
    assert.equal(rec.handle, null, 'no resolver passed → handle is null, never undefined');
    assert.equal(rec.handleSource, null, 'no handle → no source');
    assert.equal(rec.fromNpub, KID_NPUB);
    assert.equal(rec.id, 'rep-1');
    // Persisted + reloadable.
    const reloaded = new Ledger(lp).all();
    assert.equal(reloaded.length, 1);
    assert.equal(reloaded[0].body, 'doors too tall');
  } finally { cleanup(); }
});

test('ingest is idempotent — the same report is not double-recorded', async () => {
  const rumor = { kind: 14, content: 'dup', tags: [['t', 'idea'], ['report-id', 'rep-dup']] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    const first = await ingest(gift, { signer: dev, ledger });
    const second = await ingest(gift, { signer: dev, ledger });
    assert.ok(first);
    assert.equal(second, null, 'a re-seen report returns null (already in ledger)');
    assert.equal(ledger.all().length, 1);
  } finally { cleanup(); }
});

test('ledger supports the new → triaged → resolved transition in place', async () => {
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    ledger.append({ id: 'r9', fromNpub: KID_NPUB, handle: 'Secret Pete', type: 'bug', body: 'x', ts: RECENT, status: 'new', verdict: null });
    // Triage is INTERNAL only — a verdict (bug/idea/discard), never a GitHub issue.
    ledger.update('r9', { status: 'triaged', verdict: 'bug' });
    assert.equal(ledger.get('r9').status, 'triaged');
    assert.equal(ledger.get('r9').handle, 'Secret Pete');
    ledger.update('r9', { status: 'resolved' });
    // Reload from disk — the in-place update persisted.
    const reloaded = new Ledger(lp).get('r9');
    assert.equal(reloaded.status, 'resolved');
    assert.equal(reloaded.verdict, 'bug');
  } finally { cleanup(); }
});

test('ingest falls back to the resolver (persona credential) when no handle is embedded', async () => {
  const rumor = { kind: 14, content: 'wheel scrolls too fast', tags: [['t', 'bug'], ['report-id', 'rep-h']] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    // resolveHandle stands in for the kind-31000 persona lookup (read.mjs).
    const resolveHandle = async (pubkeyHex) => (pubkeyHex === kid.pubkey ? 'Secret Pete' : null);
    const rec = await ingest(gift, { signer: dev, ledger, resolveHandle });
    assert.equal(rec.handle, 'Secret Pete', 'persona handle resolved + logged alongside the npub');
    assert.equal(rec.handleSource, 'persona', 'resolver path → source persona');
    assert.equal(rec.fromNpub, KID_NPUB);
    // Persisted with the handle.
    assert.equal(new Ledger(lp).get('rep-h').handle, 'Secret Pete');
  } finally { cleanup(); }
});

test('ingest prefers the SIGNED-IN handle the client embedded over the relay resolver', async () => {
  // The client tags the report with the signed-in display name at send time. It
  // MUST win over any relay-resolved handle — the whole point: a player who
  // edited their Hash Dash leaderboard name (a kind-0) is still logged under the
  // name they signed in as.
  const rumor = { kind: 14, content: 'tnt too loud', tags: [['t', 'bug'], ['report-id', 'rep-emb'], ['handle', 'SignedInName']] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    // The resolver returns a DIFFERENT (e.g. Hash-Dash-polluted) name — ignored.
    const resolveHandle = async () => 'HashDashEditedName';
    const rec = await ingest(gift, { signer: dev, ledger, resolveHandle });
    assert.equal(rec.handle, 'SignedInName', 'embedded send-time handle wins over the resolver');
    assert.equal(rec.handleSource, 'embedded', 'embedded path → source embedded');
    assert.equal(new Ledger(lp).get('rep-emb').handle, 'SignedInName');
  } finally { cleanup(); }
});

test('a pre-2026-10-01 native report still records origin and personaNpub', async () => {
  // Older native builds tagged the report with the client id + the signed-in
  // persona pubkey (hex). Current builds send neither a persona nor a handle
  // (status-board spec S5) but the reader still ingests the old shape.
  const rumor = { kind: 14, content: 'native bug', tags: [['t', 'bug'], ['report-id', 'rep-native'], ['client', 'axenstax-native'], ['build', '0.2.19'], ['persona', dev.pubkey]] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    const rec = await ingest(gift, { signer: dev, ledger });
    assert.equal(rec.origin, 'axenstax-native');
    assert.equal(rec.personaNpub, NT.nip19.npubEncode(dev.pubkey));
    // The build that sent it — so a report from a stale install is obvious at
    // triage time (a web row, which carries no build tag, logs null).
    assert.equal(rec.build, '0.2.19');
  } finally { cleanup(); }
});

test('a current native report (one-time seal key, no persona tag) logs as anonymous native', async () => {
  const burner = signerFromSecretKey(NT.generateSecretKey());
  const rumor = { kind: 14, content: 'anon native bug', tags: [['t', 'bug'], ['report-id', 'a'.repeat(32)], ['client', 'axenstax-native'], ['build', '0.2.28']] };
  const gift = await nip59.wrap(rumor, dev.pubkey, burner);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const rec = await ingest(gift, { signer: dev, ledger: new Ledger(lp) });
    assert.equal(rec.origin, 'axenstax-native');
    assert.equal(rec.personaNpub, null, 'no persona tag, none invented');
    assert.equal(rec.handle, null);
    assert.equal(rec.id, 'a'.repeat(32), 'the report id (the ticket) is the ledger id');
    assert.equal(rec.fromNpub, NT.nip19.npubEncode(burner.pubkey), 'seal author is the one-time key');
  } finally { cleanup(); }
});

test('native report with a persona tag resolves the handle from the persona hex, not the device seal key', async () => {
  // Native reports seal with a DEVICE key (rumor.pubkey after unwrap == kid's
  // key here, standing in for the device) — the player's persona is a
  // DIFFERENT key carried in the 'persona' tag. With no embedded handle, the
  // resolver fallback must be called with the persona hex, never the seal key.
  const PERSONA_SK = NT.generateSecretKey();
  const personaPubkey = NT.getPublicKey(PERSONA_SK);
  const rumor = { kind: 14, content: 'native, no embedded handle', tags: [['t', 'bug'], ['report-id', 'rep-native-persona'], ['client', 'axenstax-native'], ['persona', personaPubkey]] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    let calledWith = null;
    const resolveHandle = async (pubkeyHex) => { calledWith = pubkeyHex; return 'PersonaName'; };
    const rec = await ingest(gift, { signer: dev, ledger, resolveHandle });
    assert.equal(calledWith, personaPubkey, 'resolver called with the PERSONA hex');
    assert.notEqual(calledWith, kid.pubkey, 'never called with the device/seal pubkey');
    assert.equal(rec.handle, 'PersonaName');
    assert.equal(rec.handleSource, 'persona');
  } finally { cleanup(); }
});

test('web report records origin axenstax and null persona', async () => {
  // The web taster only ever tags ['client','axenstax'] — no persona tag.
  const rumor = { kind: 14, content: 'web bug', tags: [['t', 'bug'], ['report-id', 'rep-web'], ['client', 'axenstax']] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    const rec = await ingest(gift, { signer: dev, ledger });
    assert.equal(rec.origin, 'axenstax');
    assert.equal(rec.personaNpub, null);
  } finally { cleanup(); }
});

test('invalid persona hex yields null personaNpub without throwing', async () => {
  const rumor = { kind: 14, content: 'bad persona', tags: [['t', 'bug'], ['report-id', 'rep-badpersona'], ['client', 'axenstax-native'], ['persona', 'nothex']] };
  const gift = await nip59.wrap(rumor, dev.pubkey, kid);
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    const rec = await ingest(gift, { signer: dev, ledger });
    assert.equal(rec.origin, 'axenstax-native');
    assert.equal(rec.personaNpub, null, 'malformed persona hex is dropped, never throws');
  } finally { cleanup(); }
});

test('reresolveHandles repairs old kind-0 rows but never clobbers an embedded handle', async () => {
  const { path: lp, cleanup } = tmpLedger();
  try {
    const ledger = new Ledger(lp);
    // A pre-fix row: handle resolved the OLD way, NO handleSource field (as on disk).
    ledger.append({ id: 'old', fromNpub: KID_NPUB, handle: 'HashDashTypedName', type: 'bug', body: 'a', ts: RECENT, status: 'new', verdict: null });
    // A post-fix row whose handle the client embedded at send time — authoritative.
    ledger.append({ id: 'emb', fromNpub: KID_NPUB, handle: 'SignedInName', handleSource: 'embedded', type: 'bug', body: 'b', ts: RECENT, status: 'new', verdict: null });
    // A row whose reporter has no persona credential on the relay — left as-is.
    const STRANGER = NT.nip19.npubEncode(NT.getPublicKey(NT.generateSecretKey()));
    ledger.append({ id: 'none', fromNpub: STRANGER, handle: 'OldName', type: 'idea', body: 'c', ts: RECENT, status: 'new', verdict: null });

    // kind-31000 lookup: the kid has a real persona handle; the stranger has none.
    const resolveByNpub = async (npub) => (npub === KID_NPUB ? 'TrueSignedIn' : null);
    const r = await reresolveHandles({ ledger, resolveByNpub });

    assert.equal(ledger.get('old').handle, 'TrueSignedIn', 'old kind-0 row re-resolved to the persona credential');
    assert.equal(ledger.get('old').handleSource, 'persona');
    assert.equal(ledger.get('emb').handle, 'SignedInName', 'embedded handle is NEVER clobbered');
    assert.equal(ledger.get('none').handle, 'OldName', 'no persona on relay → row left untouched');
    assert.deepEqual(r, { updated: 1, looked: 2, skipped: 1 });
    // Persisted across a reload.
    assert.equal(new Ledger(lp).get('old').handle, 'TrueSignedIn');
  } finally { cleanup(); }
});
