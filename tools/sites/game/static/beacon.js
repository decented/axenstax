// AxeNStax Beacon glue — PUBLIC content publish/discover/adopt (sibling of cloud.js).
//
// Replaces openstash.js's manifest role: the @forgesworn/beacon SDK
// (window.AxeBeacon) owns manifest build/parse/schnorr-verify; this file supplies
// the injected transports + signer, exactly like cloud.js does for
// window.AxeStash. See beacon/CONSUMING.md.
//
// AxeNStax specifics (app:'axenstax', the blossom-url / axenstax-relay meta tags,
// the official-pubkey seam) live HERE, never in the vendored bundle. The Blossom
// PUT/GET bodies + the BUD-02 signed-auth event are LIFTED VERBATIM from
// openstash.js (kind 24242, 300s expiry, t/x/size tags) so behaviour is identical
// to the hand-rolled path — only the manifest/relay/verify is delegated to the SDK.
//
// wasm-bindgen extern targets at the bottom. This file is loaded AFTER
// openstash.js so it wins on any name collision; openstash.js is retired in a
// later task once the engine rewires to the new surface.
(function () {
  'use strict';

  const MAX_BLOB_BYTES = 2 * 1024 * 1024;   // 2 MiB cap (override sets are small); CONSUMING.md §9
  const MAX_MANIFEST_ITEMS = 256;
  const FETCH_TIMEOUT_MS = 8000;

  // --- lifted VERBATIM from openstash.js (helper bodies unchanged) ------------
  function metaContent(name) {
    const m = document.querySelector(`meta[name="${name}"]`);
    return m ? m.content : '';
  }
  function blossomUrl() {
    return metaContent('blossom-url').replace(/\/$/, '');
  }
  function relayUrl() {
    return metaContent('axenstax-relay') || 'wss://relay.trotters.cc';
  }
  function rawSigner() {
    return typeof window.__axenstax_get_signer === 'function' ? window.__axenstax_get_signer() : null;
  }
  async function sha256Hex(bytes) {
    const digest = await crypto.subtle.digest('SHA-256', bytes);
    return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
  }
  async function buildBlossomAuth(verb, sha256, sizeBytes) {
    const s = rawSigner();
    if (!s || typeof s.signEvent !== 'function') throw new Error('beacon: no signer');
    const tags = [
      ['t', verb],
      ['expiration', String(Math.floor(Date.now() / 1000) + 300)],
    ];
    if (sha256) tags.push(['x', sha256]);
    if (verb === 'upload' && sizeBytes != null) tags.push(['size', String(sizeBytes)]);
    const signed = await s.signEvent({
      kind: 24242,
      created_at: Math.floor(Date.now() / 1000),
      tags,
      content: verb === 'upload' ? 'Upload axenstax beacon blob' : 'Fetch axenstax beacon blob',
    });
    return 'Nostr ' + btoa(JSON.stringify(signed));
  }

  // Plaintext Blossom transport, SIZE-CAPPED + fetch-timeout (CONSUMING.md §9).
  // put() body lifted from openstash.js blossomPut (auth + PUT + sha256
  // cross-check); get() body lifted from blossomGet, hardened with an
  // AbortController timeout and a both-ends size cap (content-length header AND
  // realised byte length), since a followed publisher could list an oversized
  // blob and fetch pulls the whole thing into memory before the SDK hashes it.
  const plaintextBlossomTransport = {
    async put(bytes) {
      const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
      if (u8.length > MAX_BLOB_BYTES) throw new Error('beacon: blob exceeds ' + MAX_BLOB_BYTES + ' bytes');
      const hash = await sha256Hex(u8);
      const auth = await buildBlossomAuth('upload', hash, u8.length);
      const resp = await fetch(blossomUrl() + '/upload', {
        method: 'PUT',
        headers: { Authorization: auth, 'Content-Type': 'application/octet-stream' },
        body: u8,
      });
      if (!resp.ok) throw new Error('beacon blossom upload failed: ' + resp.status);
      const descriptor = await resp.json();
      if (descriptor.sha256 && descriptor.sha256 !== hash) {
        throw new Error('beacon: blossom stored a different hash than uploaded');
      }
      return descriptor.sha256 || hash;
    },
    async get(blobHash) {
      // Public GET — no auth, no decryption (content is plaintext by design).
      const ctrl = new AbortController();
      const timer = setTimeout(() => ctrl.abort(), FETCH_TIMEOUT_MS);
      try {
        const resp = await fetch(blossomUrl() + '/' + blobHash, { signal: ctrl.signal });
        if (!resp.ok) throw new Error('beacon blossom download failed: ' + resp.status);
        // Reject early if the server advertises an oversized body.
        const len = Number(resp.headers.get('content-length'));
        if (Number.isFinite(len) && len > MAX_BLOB_BYTES) {
          throw new Error('beacon: blob exceeds ' + MAX_BLOB_BYTES + ' bytes');
        }
        const u8 = new Uint8Array(await resp.arrayBuffer());
        // Cross-check the realised length too (no/forged content-length header).
        if (u8.length > MAX_BLOB_BYTES) throw new Error('beacon: blob exceeds ' + MAX_BLOB_BYTES + ' bytes');
        return u8;
      } finally {
        clearTimeout(timer);
      }
    },
  };

  // --- Build a Beacon instance (SDK owns manifest/follows/verify) or null -----
  function makeBeacon() {
    const s = rawSigner();
    if (!s || typeof s.signEvent !== 'function') return null;
    if (!window.AxeBeacon || !window.AxeRelay) return null;
    const relay = window.AxeRelay.makeRelayClient([relayUrl()]);
    return window.AxeBeacon.createBeacon({
      app: 'axenstax',
      signer: { getPublicKey: () => s.pubkey, signEvent: (t) => s.signEvent(t) },
      blossom: plaintextBlossomTransport,
      relay,
      official: window.__axenstax_official_pubkey_hex || undefined,
    });
  }

  // --- wasm-bindgen extern targets (the engine calls these) -------------------
  // The engine treats publish/list/fetch as best-effort and reads:
  //   axenstax_openstash_available()              -> bool
  //   axenstax_openstash_publish(name, ct, bytes) -> blobHash hex string
  //   axenstax_openstash_list(pubkeyHex?)         -> JSON [{name,blobHash,contentType,size,updated}]
  //   axenstax_openstash_download(blobHash)       -> Uint8Array
  //   axenstax_openstash_follow/unfollow(hex)     -> "ok"
  //   axenstax_openstash_following()              -> JSON string[] of hex pubkeys
  window.axenstax_openstash_available = () => !!makeBeacon();

  window.axenstax_openstash_publish = async (name, contentType, bytes) => {
    const b = makeBeacon();
    if (!b) throw new Error('beacon: unavailable');
    const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    const ref = await b.publish(name, u8, contentType); // ItemRef { blobHash, size }
    return ref.blobHash;
  };

  window.axenstax_openstash_list = async (pubkeyHex) => {
    const b = makeBeacon();
    if (!b) throw new Error('beacon: unavailable');
    const items = await b.list(pubkeyHex); // BeaconItem[] { name, blobHash, contentType, size, updated }
    return JSON.stringify(items.slice(0, MAX_MANIFEST_ITEMS));
  };

  window.axenstax_openstash_download = async (blobHash) => {
    const b = makeBeacon();
    if (!b) throw new Error('beacon: unavailable');
    return b.fetch(blobHash); // fetch accepts a blobHash string directly
  };

  window.axenstax_openstash_follow = async (pubkeyHex) => {
    const b = makeBeacon();
    if (!b) throw new Error('beacon: unavailable');
    await b.follow(pubkeyHex);
    return 'ok';
  };

  window.axenstax_openstash_unfollow = async (pubkeyHex) => {
    const b = makeBeacon();
    if (!b) throw new Error('beacon: unavailable');
    await b.unfollow(pubkeyHex);
    return 'ok';
  };

  window.axenstax_openstash_following = async () => {
    const b = makeBeacon();
    if (!b) throw new Error('beacon: unavailable');
    return JSON.stringify(await b.following());
  };
})();
