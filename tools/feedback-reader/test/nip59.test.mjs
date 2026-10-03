// Tests for lib/nip59.cjs — the NIP-59 gift-wrap helper shared by the readers.
//
// The helper reads its nostr primitives from globalThis.AxeNostr; here we
// inject that surface from the npm package.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as NT from 'nostr-tools';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

globalThis.AxeNostr = {
  nip44: NT.nip44,
  generateSecretKey: NT.generateSecretKey,
  getPublicKey: NT.getPublicKey,
  finalizeEvent: NT.finalizeEvent,
  getEventHash: NT.getEventHash,
  verifyEvent: NT.verifyEvent,
};

const require = createRequire(import.meta.url);
const nip59 = require(
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../lib/nip59.cjs')
);

// Mimics the retained Signet signer surface (persona-key nip44 + signEvent),
// built from a raw test secret key. The real signer is a bunker round-trip;
// the shape is identical.
function makeSigner(sk) {
  const pubkey = NT.getPublicKey(sk);
  return {
    pubkey,
    capabilities: { hasNip44: true },
    nip44: {
      encrypt: (peerHex, pt) => NT.nip44.encrypt(pt, NT.nip44.getConversationKey(sk, peerHex)),
      decrypt: (peerHex, ct) => NT.nip44.decrypt(ct, NT.nip44.getConversationKey(sk, peerHex)),
    },
    signEvent: (tmpl) => NT.finalizeEvent({ ...tmpl, pubkey }, sk),
  };
}

function freshPair() {
  const sk = NT.generateSecretKey();
  return { sk, signer: makeSigner(sk) };
}

test('wrap → unwrap round-trips the rumour', async () => {
  const sender = freshPair().signer;
  const recipient = freshPair().signer;

  const rumor = { kind: 14, content: 'doors too tall', tags: [['t', 'bug']] };
  const wrap = await nip59.wrap(rumor, recipient.pubkey, sender);
  const out = await nip59.unwrap(wrap, recipient);

  assert.equal(out.content, 'doors too tall');
  assert.equal(out.kind, 14);
  assert.deepEqual(out.tags, [['t', 'bug']]);
  // The rumour carries the real sender's pubkey (lets the recipient confirm authorship).
  assert.equal(out.pubkey, sender.pubkey);
});

test('gift-wrap (1059) is signed by an ephemeral key, never the persona', async () => {
  const sender = freshPair().signer;
  const recipient = freshPair().signer;

  const wrap = await nip59.wrap({ kind: 14, content: 'x', tags: [] }, recipient.pubkey, sender);
  assert.equal(wrap.kind, 1059);
  assert.notEqual(wrap.pubkey, sender.pubkey, 'outer wrap must NOT reveal the sender');
  assert.notEqual(wrap.pubkey, recipient.pubkey);

  const pTag = wrap.tags.find((t) => t[0] === 'p');
  assert.ok(pTag, 'p tag present');
  assert.equal(pTag[1], recipient.pubkey);

  assert.ok(NT.verifyEvent(wrap), 'gift-wrap signature verifies');
});

test('the seal (kind 13) is signed by the persona, hidden inside the wrap', async () => {
  const sender = freshPair().signer;
  const recipient = freshPair().signer;

  const wrap = await nip59.wrap({ kind: 14, content: 'hi', tags: [] }, recipient.pubkey, sender);
  const sealJson = recipient.nip44.decrypt(wrap.pubkey, wrap.content);
  const seal = JSON.parse(sealJson);
  assert.equal(seal.kind, 13);
  assert.equal(seal.pubkey, sender.pubkey, 'seal is signed by the persona');
  assert.ok(NT.verifyEvent(seal), 'seal signature verifies');
});

test('two wraps of the same rumour use different ephemeral keys', async () => {
  const sender = freshPair().signer;
  const recipient = freshPair().signer;
  const rumor = { kind: 14, content: 'same', tags: [] };
  const a = await nip59.wrap(rumor, recipient.pubkey, sender);
  const b = await nip59.wrap(rumor, recipient.pubkey, sender);
  assert.notEqual(a.pubkey, b.pubkey, 'fresh ephemeral key per wrap');
  assert.notEqual(a.id, b.id);
});

test('unwrap rejects a seal whose author differs from the rumour author (spoofing)', async () => {
  const attacker = freshPair();          // signs the seal with its own key
  const recipient = freshPair().signer;
  const victimHex = NT.getPublicKey(NT.generateSecretKey()); // the identity being framed

  // Hand-build a wrap: a VALID seal signed by the attacker, but the rumour
  // inside claims `victimHex` as its author.
  const rumor = { kind: 14, created_at: 0, tags: [], content: 'i never said this', pubkey: victimHex };
  rumor.id = NT.getEventHash(rumor);
  const sealContent = NT.nip44.encrypt(JSON.stringify(rumor), NT.nip44.getConversationKey(attacker.sk, recipient.pubkey));
  const seal = NT.finalizeEvent({ kind: 13, created_at: 0, tags: [], content: sealContent }, attacker.sk);
  const ephSk = NT.generateSecretKey();
  const wrapContent = NT.nip44.encrypt(JSON.stringify(seal), NT.nip44.getConversationKey(ephSk, recipient.pubkey));
  const forged = NT.finalizeEvent({ kind: 1059, created_at: 0, tags: [['p', recipient.pubkey]], content: wrapContent }, ephSk);

  await assert.rejects(() => nip59.unwrap(forged, recipient), /spoof|pubkey/i);
});

test('unwrap rejects a seal with an invalid signature', async () => {
  const attacker = freshPair();
  const recipient = freshPair().signer;
  const rumor = { kind: 14, created_at: 0, tags: [], content: 'forged seal', pubkey: attacker.signer.pubkey };
  rumor.id = NT.getEventHash(rumor);
  const sealContent = NT.nip44.encrypt(JSON.stringify(rumor), NT.nip44.getConversationKey(attacker.sk, recipient.pubkey));
  const seal = NT.finalizeEvent({ kind: 13, created_at: 0, tags: [], content: sealContent }, attacker.sk);
  seal.sig = '0'.repeat(128); // corrupt the seal signature after signing
  const ephSk = NT.generateSecretKey();
  const wrapContent = NT.nip44.encrypt(JSON.stringify(seal), NT.nip44.getConversationKey(ephSk, recipient.pubkey));
  const forged = NT.finalizeEvent({ kind: 1059, created_at: 0, tags: [['p', recipient.pubkey]], content: wrapContent }, ephSk);

  await assert.rejects(() => nip59.unwrap(forged, recipient), /signature|invalid|seal/i);
});
