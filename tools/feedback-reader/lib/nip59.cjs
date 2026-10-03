// NIP-59 gift-wrap helper — the metadata-private envelope for the NATIVE mailbox
// (feedback-reader). The browser build no longer carries a
// mailbox (removed 2026-10-01); this file is dev-side tooling only. It is a
// CommonJS file (.cjs) because the reader packages are ES modules.
//
//   wrap(rumor, recipientPubkeyHex, signer)  -> kind-1059 gift-wrap event
//   unwrap(giftWrapEvent, signer)            -> the original rumour
//
// A NIP-17 private DM is three nested events:
//   rumour (kind 14)  — unsigned; carries the message + the real sender pubkey.
//   seal   (kind 13)  — nip44-encrypts the rumour to the recipient, SIGNED BY
//                       THE SENDER (here: the persona/bunker via signer.signEvent).
//   wrap   (kind 1059)— nip44-encrypts the seal to the recipient with a FRESH
//                       EPHEMERAL key, signed by that ephemeral key. The public
//                       relay therefore never sees who sent it.
//
// This composes the seal + wrap by hand (rather than nostr-tools' all-in-one
// nip59.wrapEvent) because the seal must be signed by the BUNKER — we never
// hold the persona secret key, so we can't hand a privkey to wrapEvent. The
// outer wrap uses a throwaway key we DO hold, so it's finalized locally.
//
// The nostr primitives are injected as globalThis.AxeNostr from the npm
// package (see lib/nostr.mjs).
(function () {
  'use strict';

  // Lazily resolve the nostr primitives so script load order doesn't matter.
  function nt() {
    var n = (typeof globalThis !== 'undefined' && globalThis.AxeNostr) ? globalThis.AxeNostr : null;
    if (!n || !n.nip44 || typeof n.finalizeEvent !== 'function' || typeof n.generateSecretKey !== 'function') {
      throw new Error('nip59: nostr primitives (AxeNostr) not loaded');
    }
    return n;
  }

  var TWO_DAYS = 2 * 24 * 60 * 60;
  function nowSec() { return Math.round(Date.now() / 1000); }
  // NIP-59 §"Timestamps": randomise seal/wrap created_at up to two days into the
  // past so the relay can't correlate messages by timing.
  function randomPastTimestamp() { return Math.round(nowSec() - Math.random() * TWO_DAYS); }

  // rumor: { kind, content, tags?, created_at? }. Returns a signed kind-1059.
  async function wrap(rumor, recipientPubkeyHex, signer) {
    var N = nt();

    // 1. Normalise the rumour (kind 14). It stays UNSIGNED (no sig) per NIP-59;
    //    its pubkey is the real author so the recipient can confirm who wrote it.
    var inner = {
      kind: rumor.kind,
      created_at: typeof rumor.created_at === 'number' ? rumor.created_at : nowSec(),
      tags: rumor.tags || [],
      content: rumor.content || '',
      pubkey: signer.pubkey,
    };
    inner.id = N.getEventHash(inner);

    // 2. Seal (kind 13): persona-key nip44 to the recipient, signed by the bunker.
    var sealContent = await signer.nip44.encrypt(recipientPubkeyHex, JSON.stringify(inner));
    var seal = await signer.signEvent({
      kind: 13,
      created_at: randomPastTimestamp(),
      tags: [],
      content: sealContent,
    });

    // 3. Gift-wrap (kind 1059): ephemeral-key nip44 to the recipient, signed by
    //    the ephemeral key. Fresh key per wrap → the wrap pubkey leaks nothing.
    var ephSk = N.generateSecretKey();
    var convKey = N.nip44.getConversationKey(ephSk, recipientPubkeyHex);
    var wrapContent = N.nip44.encrypt(JSON.stringify(seal), convKey);
    return N.finalizeEvent({
      kind: 1059,
      created_at: randomPastTimestamp(),
      tags: [['p', recipientPubkeyHex]],
      content: wrapContent,
    }, ephSk);
  }

  // Reverse of wrap, using the recipient's signer (persona-key nip44 only — no
  // ephemeral crypto needed on this side). Throws if either layer fails to
  // decrypt OR if the seal isn't authentic; callers (mailbox inbound, dev
  // reader) skip + log such events.
  //
  // Authenticity (NIP-17/NIP-59): the gift-wrap's nip44 MAC proves the outer
  // ciphertext is intact, but NOT who wrote the rumour inside. We MUST verify
  // the SEAL's Schnorr signature and that the rumour's author equals the seal's
  // signer — otherwise a sender could forge `rumor.pubkey` (the value the dev
  // reader records as `fromNpub` and the lobby inbox shows as the sender).
  async function unwrap(giftWrap, signer) {
    var N = nt();
    var sealJson = await signer.nip44.decrypt(giftWrap.pubkey, giftWrap.content);
    var seal = JSON.parse(sealJson);
    if (seal.kind !== 13) throw new Error('nip59: expected seal kind 13, got ' + seal.kind);
    if (!N.verifyEvent(seal)) throw new Error('nip59: seal signature invalid');
    var rumorJson = await signer.nip44.decrypt(seal.pubkey, seal.content);
    var rumor = JSON.parse(rumorJson);
    if (rumor.pubkey !== seal.pubkey) {
      throw new Error('nip59: rumor.pubkey != seal.pubkey — sender spoofing attempt');
    }
    return rumor;
  }

  var api = { wrap: wrap, unwrap: unwrap };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
})();
