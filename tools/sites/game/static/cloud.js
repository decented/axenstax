// AxeNStax cloud save — serverless (destination tier).
//
// (The lobby mailbox / `/bug` `/idea` feedback bridge that used to live in this
// file was removed 2026-10-01: the browser build has no feedback channel.)
//
// World-cloud-save PUSH was removed 2026-07-09 (confirmed dead end-to-end: its
// only trigger, the lobby/pause-menu Stash toggle, is `#[cfg(not(wasm32))]` in
// the engine, so `CardAction::ToggleStash` is never constructed on a wasm32
// build; separately `window.__axenstax_get_signer`/`_set_signer` — the only way
// this file could ever obtain a real signer — were never re-wired after web
// login was gutted to a guest-only stub 2026-06-27. See
// docs/superpowers/specs/2026-07-09-give-aliases-and-dead-web-stash-code.md.
// What remains below (`available`/`list`/`restore`, `makeStash`) backs the
// still-live-but-equally-inert cloud-world-list READ merge in
// `menu.rs::poll_cloud_worlds`, plus cosmetics/wardrobe cloud sync — neither
// audited/removed by that spec, so left in place.
//
// The browser talks DIRECTLY to Blossom (player-key BUD-02 auth) and to a Nostr
// relay (player-signed encrypted manifest). No AxeNStax server in the save path:
//   restore: read manifest from relay → GET ciphertext from Blossom → decrypt
//
// Everything is keyed to the persona's own key (held by the Signet signer, which
// also signs the Blossom auth + the manifest event). The page never holds a
// private key. Zero-knowledge end to end: Blossom + relay see only ciphertext.
//
// Built on @forgesworn/stash (window.AxeStash, destination tier) + a vendored
// relay client (window.AxeRelay) + the retained signer (window.__axenstax_get_signer).
(function () {
  'use strict';

  const APP = 'axenstax';
  // Build marker — confirms which cloud.js a tester is actually running.
  try { console.log('[axe-cloud] cloud.js rev 2026-06-10-r8 (bunker-call diagnostics)'); } catch (_) { /* noop */ }

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
  function metaEnabled() {
    return metaContent('cloud-save') === 'enabled' && !!blossomUrl();
  }

  // --- The retained Signet signer, adapted to what Stash + Blossom auth need ---
  function rawSigner() {
    return typeof window.__axenstax_get_signer === 'function' ? window.__axenstax_get_signer() : null;
  }
  let _capLog = '';
  function capable() {
    const s = rawSigner();
    // Needs NIP-44 (encrypt worlds) AND signEvent (Blossom auth + manifest events).
    const ok = !!s && !!(s.capabilities && s.capabilities.hasNip44) && !!s.nip44 && typeof s.signEvent === 'function';
    // Diagnostic (throttled to state changes): why is cloud on/off right now?
    const why = ok ? 'ok'
      : !s ? 'no-signer'
      : !(s.capabilities && s.capabilities.hasNip44) ? 'signer-not-nip44-capable'
      : !s.nip44 ? 'no-nip44-method'
      : 'no-signEvent';
    if (why !== _capLog) { _capLog = why; console.log('[axe-cloud] capable=' + ok + ' (' + why + ')'); }
    return ok;
  }
  // --- Per-bunker-call diagnostics ---------------------------------------------
  // Every NIP-46 round-trip (sign / nip44) is logged with a sequence id and
  // timing. A call that logs "→" but never "ok"/"FAIL" is HUNG awaiting the
  // bunker (signet-app not serving: locked, backgrounded, or stay-awake window
  // closed — it answers 'serving paused' when reachable-but-paused, and nothing
  // at all when torn down). This is the evidence layer for the stash-sync
  // investigation: docs/superpowers/notes/2026-06-10-stash-sync-never-completes-handoff.md
  let _bunkerCallSeq = 0;
  let _lastOpError = '';
  function logCall(tag, method, fn) {
    return async function (...args) {
      const id = ++_bunkerCallSeq;
      const t0 = Date.now();
      console.log('[bunker-call] #' + id + ' ' + tag + '.' + method + ' →');
      try {
        const out = await fn.apply(this, args);
        console.log('[bunker-call] #' + id + ' ' + tag + '.' + method + ' ok (' + (Date.now() - t0) + 'ms)');
        return out;
      } catch (e) {
        _lastOpError = (e && e.message) ? e.message : String(e);
        console.warn('[bunker-call] #' + id + ' ' + tag + '.' + method + ' FAIL (' + (Date.now() - t0) + 'ms):', _lastOpError);
        throw e;
      }
    };
  }
  function stashSigner() {
    const s = rawSigner();
    if (!capable()) return null;
    return {
      pubkey: s.pubkey,
      nip44Encrypt: logCall('stash', 'nip44Encrypt', (peer, pt) => s.nip44.encrypt(peer, pt)),
      nip44Decrypt: logCall('stash', 'nip44Decrypt', (peer, ct) => s.nip44.decrypt(peer, ct)),
      signEvent: logCall('stash', 'signEvent', (t) => s.signEvent(t)),
    };
  }

  // --- Player-key Blossom transport (BUD-01/02), signed in the browser ---
  // Builds the kind-24242 auth event with the persona's signer and PUTs/GETs
  // directly to the Blossom server (CORS-enabled, browser-reachable).
  async function buildBlossomAuth(verb, sha256, sizeBytes) {
    const s = rawSigner();
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
      content: verb === 'upload' ? 'Upload axenstax world' : 'Fetch axenstax world',
    });
    return 'Nostr ' + btoa(JSON.stringify(signed));
  }

  async function sha256Hex(bytes) {
    const digest = await crypto.subtle.digest('SHA-256', bytes);
    return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('');
  }

  const blossomTransport = {
    async put(ciphertext) {
      const hash = await sha256Hex(ciphertext);
      const auth = await buildBlossomAuth('upload', hash, ciphertext.length);
      const resp = await fetch(blossomUrl() + '/upload', {
        method: 'PUT',
        headers: { Authorization: auth, 'Content-Type': 'application/octet-stream' },
        body: ciphertext,
      });
      if (!resp.ok) throw new Error('blossom upload failed: ' + resp.status);
      const descriptor = await resp.json();
      if (descriptor.sha256 && descriptor.sha256 !== hash) {
        throw new Error('blossom stored a different hash than uploaded');
      }
      return descriptor.sha256 || hash;
    },
    async get(blobHash) {
      // GET is public on blossom-server by default — no auth header needed.
      const resp = await fetch(blossomUrl() + '/' + blobHash);
      if (!resp.ok) throw new Error('blossom download failed: ' + resp.status);
      return new Uint8Array(await resp.arrayBuffer());
    },
  };

  // --- Build a destination-tier Stash instance (relay manifest) or null ---
  function makeStash() {
    if (!metaEnabled()) return null;
    if (!window.AxeStash || !window.AxeRelay) {
      console.warn('cloud: stash/relay bundle not loaded');
      return null;
    }
    const signer = stashSigner();
    if (!signer) return null; // no capable signer → cloud off
    const relay = window.AxeRelay.makeRelayClient([relayUrl()]);
    return window.AxeStash.createStash({
      app: APP,
      signer,
      blossom: blossomTransport,
      manifest: window.AxeStash.nostrManifestStore({ signer, relay }),
    });
  }

  // --- High-level API the engine bridge calls (cheap to rebuild per call) ---
  // `save` (world push) was removed 2026-07-09 with its only callers
  // (axenstax_cloud_save/sync, runSyncBatch) — see the file-header note.
  const AxeCloud = {
    available() {
      return makeStash() !== null;
    },
    async list() {
      const stash = makeStash();
      if (!stash) return [];
      // Worlds only — sibling kinds (e.g. the `cosmetic` skin) share this
      // manifest, so filter them out or they'd appear as phantom tiles in the
      // world picker (and "opening" one would try to unpack a PNG as a world).
      const entries = await stash.list();
      return entries.filter((e) => !e.kind || e.kind === 'world');
    },
    async restore(blobHash) {
      const stash = makeStash();
      if (!stash) throw new Error('cloud save unavailable');
      return stash.restore(blobHash);
    },
    async remove(blobHash) {
      const stash = makeStash();
      if (!stash) return;
      return stash.remove(blobHash);
    },
    // Cross-game view: everything this persona owns across all apps.
    async listAllApps() {
      const stash = makeStash();
      if (!stash) return {};
      return stash.listAllApps();
    },
  };

  window.AxeCloud = AxeCloud;

  // --- Cosmetics (Phase 2): a player's 64×64 PNG skin, kind:"cosmetic"/name:"avatar".
  // Two tiers. Local = localStorage (instant, offline, per-pubkey base64) so a
  // skin shows immediately and survives offline. Cloud = Stash (one replaceable
  // entry that follows the persona to any device); best-effort, never blocks.
  //
  // The Stash entry is a sibling of worlds under the same manifest — it's keyed
  // by kind/name (`cosmetic`/`avatar`), so list() returns it alongside worlds and
  // we filter on `.kind`/`.name` (the stash descriptor shape: see stash.iife.js).
  // We can't use AxeCloud.save() here because it hardcodes kind:"world"; cosmetics
  // go straight to a fresh stash handle so they carry the right kind.
  const COSMETIC_KIND = 'cosmetic';
  const COSMETIC_NAME = 'avatar';

  // Per-pubkey localStorage key. The validated signed-in pubkey is published by
  // auth.js as window.__axenstax_pubkey; fall back to the retained signer's
  // pubkey. Returns null only when nobody is signed in (local tier then no-ops,
  // which is correct — there's no persona to scope to).
  function _cosmeticPubkey() {
    const fromAuth = typeof window.__axenstax_pubkey === 'string' ? window.__axenstax_pubkey : '';
    if (/^[0-9a-f]{64}$/i.test(fromAuth)) return fromAuth.toLowerCase();
    const s = rawSigner();
    const fromSigner = s && typeof s.pubkey === 'string' ? s.pubkey : '';
    return /^[0-9a-f]{64}$/i.test(fromSigner) ? fromSigner.toLowerCase() : '';
  }
  function _cosmeticKey() {
    const pk = _cosmeticPubkey();
    return pk ? 'axenstax_cosmetic_' + pk : null;
  }
  function _b64FromBytes(u8) {
    let s = '';
    for (let i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]);
    return btoa(s);
  }
  function _bytesFromB64(b64) {
    const s = atob(b64);
    const u8 = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) u8[i] = s.charCodeAt(i);
    return u8;
  }
  // The Stash manifest dedupes ONLY by blobHash (kind/name live inside the
  // encrypted descriptor, invisible to that dedup — see stash.iife.js record()).
  // A skin's blobHash is the hash of its bytes, so every changed skin appends a
  // NEW manifest entry without superseding the old one. So we must NOT use
  // Array.find (which returns the OLDEST appended match) — pick the NEWEST.
  // Recency rule: max by `updated` (epoch-seconds, set by stash.save), breaking
  // ties toward the LAST array element (later manifest append = newer), which
  // covers two saves landing in the same wall-clock second.
  function _findCosmetic(entries) {
    const matches = entries.filter((x) => x.kind === COSMETIC_KIND && x.name === COSMETIC_NAME);
    if (!matches.length) return undefined;
    return matches.reduce((a, b) => ((b.updated || 0) >= (a.updated || 0) ? b : a));
  }

  window.axenstax_cosmetic_save = async (bytes) => {
    const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    const key = _cosmeticKey();
    if (key) { try { localStorage.setItem(key, _b64FromBytes(u8)); } catch (_) {} }
    const stash = makeStash();
    if (stash) {
      try {
        // stash.save(kind, name, bytes) -> { blobHash, size }. The save appended
        // a fresh manifest entry; older cosmetic entries (different blobHash) were
        // NOT superseded by the blobHash-only dedup, so sweep them now to keep the
        // cosmetic a true singleton (otherwise load gets the wrong skin + entries
        // grow unbounded across re-uploads).
        const saved = await stash.save(COSMETIC_KIND, COSMETIC_NAME, u8); // { blobHash, size }
        try {
          const stale = (await stash.list()).filter(
            (x) => x.kind === COSMETIC_KIND && x.name === COSMETIC_NAME && x.blobHash !== saved.blobHash
          );
          for (const e of stale) { try { await stash.remove(e.blobHash); } catch (_) {} }
        } catch (_) {}
      }
      catch (e) { console.warn('[cosmetic] stash save failed', e); }
    }
  };
  window.axenstax_cosmetic_load = async () => {
    const key = _cosmeticKey();
    if (key) {
      const s = localStorage.getItem(key);
      if (s) { try { return _bytesFromB64(s); } catch (_) {} }
    }
    const stash = makeStash();
    if (stash) {
      try {
        const e = _findCosmetic(await stash.list());
        if (e) {
          const b = await stash.restore(e.blobHash);
          const u8 = b instanceof Uint8Array ? b : new Uint8Array(b);
          if (key) { try { localStorage.setItem(key, _b64FromBytes(u8)); } catch (_) {} }
          return u8;
        }
      } catch (e) { console.warn('[cosmetic] stash load failed', e); }
    }
    return null;
  };
  window.axenstax_cosmetic_reset = async () => {
    const key = _cosmeticKey();
    if (key) { try { localStorage.removeItem(key); } catch (_) {} }
    const stash = makeStash();
    if (stash) {
      try {
        // Remove ALL cosmetic entries, not just one — re-uploads accumulate
        // orphan entries (blobHash-only dedup), so removing a single one would
        // leave the cloud skin alive on other devices.
        const stale = (await stash.list()).filter(
          (x) => x.kind === COSMETIC_KIND && x.name === COSMETIC_NAME
        );
        for (const e of stale) { try { if (stash.remove) await stash.remove(e.blobHash); } catch (_) {} }
      } catch (e) { console.warn('[cosmetic] stash reset failed', e); }
    }
  };

  // --- Skin wardrobe (Phase 1c): the player's avatar-skin wardrobe blob.
  // Web is LOCAL ONLY (no Stash) per the web-taster minimal-data posture
  // (spec 2026-06-29 §10). Per-pubkey when signed in; a stable `local` ns for
  // the anonymous taster so a taster's skins survive a refresh on this device.
  function _skinWardrobeKey() {
    const pk = _cosmeticPubkey();
    return 'axenstax_skinwardrobe_' + (pk || 'local');
  }
  window.axenstax_skinwardrobe_save = async (bytes) => {
    const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    try { localStorage.setItem(_skinWardrobeKey(), _b64FromBytes(u8)); } catch (_) {}
  };
  window.axenstax_skinwardrobe_load = async () => {
    try {
      const s = localStorage.getItem(_skinWardrobeKey());
      if (s) return _bytesFromB64(s);
    } catch (_) {}
    return null;
  };

  // --- Ghost EXPORT (Campaign G): download a race ghost as an .axeghost
  // JSON file. Same Blob + <a download> click as the skin export.
  window.axenstax_ghost_download = async (name, bytes) => {
    try {
      const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
      const blob = new Blob([u8], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      try {
        const a = document.createElement('a');
        a.href = url;
        a.download = name || 'axenstax-ghost.axeghost';
        a.style.display = 'none';
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
      } finally {
        setTimeout(() => URL.revokeObjectURL(url), 0);
      }
    } catch (e) { console.warn('[ghost] download failed', e); }
  };

  // --- Skin EXPORT: download the painted/selected skin as a 64x64 PNG.
  // Mirrors world_store.exportWorld's Blob + <a download> click. `name` already
  // carries the .png extension (built engine-side by export_skin_filename).
  window.axenstax_skin_download = async (name, bytes) => {
    try {
      const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
      const blob = new Blob([u8], { type: 'image/png' });
      const url = URL.createObjectURL(blob);
      try {
        const a = document.createElement('a');
        a.href = url;
        a.download = name || 'axenstax-my-skin.png';
        a.style.display = 'none';
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
      } finally {
        setTimeout(() => URL.revokeObjectURL(url), 0);
      }
    } catch (e) { console.warn('[skin] download failed', e); }
  };

  // --- Skin IMPORT from Minecraft (web): relay through our /mc-skin proxy
  // (Mojang has no CORS). Returns { uuid, name, slim, bytes } on success, or
  // { error: "<code>" } on a structured failure, or null on a transport fault.
  window.axenstax_mc_skin_fetch = async (query) => {
    try {
      const r = await fetch('/mc-skin?' + query, { cache: 'no-store' });
      if (!r.ok) {
        let code = 'offline';
        try { code = (await r.json()).error || 'offline'; } catch (_) {}
        return { error: code };
      }
      const buf = new Uint8Array(await r.arrayBuffer());
      return {
        uuid: r.headers.get('X-Mc-Uuid') || '',
        name: r.headers.get('X-Mc-Name') || '',
        slim: (r.headers.get('X-Mc-Slim') || '0') === '1',
        bytes: buf,
      };
    } catch (_) {
      return { error: 'offline' };
    }
  };

  // --- Wardrobe (Spec 40): the player's GLOBAL Workshop wardrobe (OverrideSet blob).
  // Player-scoped, cross-world singleton in the private Stash, exactly like the
  // cosmetic skin: one replaceable entry keyed kind:"wardrobe"/name:"designs".
  const WARDROBE_KIND = 'wardrobe';
  const WARDROBE_NAME = 'designs';
  function _findWardrobe(entries) {
    const m = entries.filter((x) => x.kind === WARDROBE_KIND && x.name === WARDROBE_NAME);
    if (!m.length) return undefined;
    return m.reduce((a, b) => ((b.updated || 0) >= (a.updated || 0) ? b : a));
  }
  window.axenstax_wardrobe_save = async (bytes) => {
    const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    const stash = makeStash();
    if (!stash) return;
    try {
      const saved = await stash.save(WARDROBE_KIND, WARDROBE_NAME, u8);
      try {
        const stale = (await stash.list()).filter(
          (x) => x.kind === WARDROBE_KIND && x.name === WARDROBE_NAME && x.blobHash !== saved.blobHash);
        for (const e of stale) { try { await stash.remove(e.blobHash); } catch (_) {} }
      } catch (_) {}
    } catch (e) { console.warn('[wardrobe] stash save failed', e); }
  };
  window.axenstax_wardrobe_load = async () => {
    const stash = makeStash();
    if (!stash) return null;
    try {
      const e = _findWardrobe(await stash.list());
      if (e) { const b = await stash.restore(e.blobHash); return b instanceof Uint8Array ? b : new Uint8Array(b); }
    } catch (e) { console.warn('[wardrobe] stash load failed', e); }
    return null;
  };
  window.axenstax_wardrobe_reset = async () => {
    const stash = makeStash();
    if (!stash) return;
    try {
      const stale = (await stash.list()).filter((x) => x.kind === WARDROBE_KIND && x.name === WARDROBE_NAME);
      for (const e of stale) { try { if (stash.remove) await stash.remove(e.blobHash); } catch (_) {} }
    } catch (e) { console.warn('[wardrobe] stash reset failed', e); }
  };
  // Remember-on-entry preference (per-pubkey localStorage; default true).
  function _wardrobeRememberKey() { const pk = _cosmeticPubkey(); return pk ? 'axenstax_wardrobe_remember_' + pk : null; }
  window.axenstax_wardrobe_remember_get = () => {
    const k = _wardrobeRememberKey(); if (!k) return true;
    try { const v = localStorage.getItem(k); return v === null ? true : v !== '0'; } catch (_) { return true; }
  };
  window.axenstax_wardrobe_remember_set = (on) => {
    const k = _wardrobeRememberKey(); if (!k) return;
    try { localStorage.setItem(k, on ? '1' : '0'); } catch (_) {}
  };

  // --- "Stash" status toast (over the game canvas) ---
  // Tiny, non-blocking, top-right. pointer-events:none so it never eats input.
  // "Stash" is the player-facing word for cloud save (local save is "Save").
  let _toastEl = null;
  let _toastTimer = null;
  function toast(text, tone) {
    if (typeof document === 'undefined') return;
    if (!_toastEl) {
      _toastEl = document.createElement('div');
      _toastEl.style.cssText = [
        'position:fixed', 'top:12px', 'right:12px', 'z-index:99999',
        'pointer-events:none', 'font:600 14px system-ui,sans-serif',
        'padding:8px 14px', 'border-radius:8px', 'color:#fff',
        'box-shadow:0 2px 10px rgba(0,0,0,.35)', 'transition:opacity .3s',
        'max-width:280px',
      ].join(';');
      document.body.appendChild(_toastEl);
    }
    const bg = tone === 'ok' ? 'rgba(40,140,70,.95)'
      : tone === 'warn' ? 'rgba(170,120,40,.95)'
      : 'rgba(40,60,90,.95)';
    _toastEl.style.background = bg;
    _toastEl.textContent = text;
    _toastEl.style.opacity = '1';
    if (_toastTimer) clearTimeout(_toastTimer);
    const linger = tone === 'pending' ? 0 : (tone === 'warn' ? 5000 : 2500);
    if (linger) _toastTimer = setTimeout(() => { if (_toastEl) _toastEl.style.opacity = '0'; }, linger);
  }

  // --- wasm-bindgen extern targets (the Rust engine calls these) ---
  // Thin, throw-safe wrappers over AxeCloud. `axenstax_cloud_save`/`_sync`
  // (world push) were removed 2026-07-09 — see the file-header note.
  //
  //   axenstax_cloud_available()           -> bool
  //   axenstax_cloud_list()                -> JSON string of [{name,blobHash,size,updated,...}]
  //   axenstax_cloud_restore(blobHash)     -> Uint8Array (throws if missing/undecryptable)
  window.axenstax_cloud_available = () => AxeCloud.available();
  window.axenstax_cloud_list = async () => {
    try {
      return JSON.stringify(await AxeCloud.list());
    } catch (e) {
      console.warn('cloud list failed:', e);
      return '[]';
    }
  };
  window.axenstax_cloud_restore = (blobHash) => AxeCloud.restore(blobHash);

  // "Sync Stash" batch status. The start/cancel/upload side (`runSyncBatch`,
  // `axenstax_sync_stash_start`/`_cancel`) was removed 2026-07-09 — no button
  // ever called it (0 Rust callers found pre-removal; the engine's
  // `sync_button_label`/`_is_cancel` were `#[allow(dead_code)]`, "no button
  // widget calls this yet"). `_syncStatus` now just stays permanently idle;
  // `menu.rs::poll_cloud_worlds` polls this every frame and degrades safely.
  const _syncStatus = { state: 'idle', message: '', done: 0, total: 0 };
  window.axenstax_sync_stash_status = () => JSON.stringify(_syncStatus);
})();
