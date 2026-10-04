// AxeNStax web taster — local-only bridge between the WASM engine and the page.
//
// The browser build is an anonymous, offline, same-origin-only taster (T0-4,
// 2026-10-05): no sign-in, no cloud save, no Stash, no relay, no Blossom, no
// Beacon sharing. Everything here is either a localStorage / download helper or
// a same-origin fetch (/mc-skin), plus INERT stubs for the engine externs that
// the wasm still imports (game/engine/src/wasm_save.rs + open_stash.rs). The
// Rust side calls some of those without a presence guard, so they must keep
// existing even though there is nothing behind them; they answer "unavailable"
// exactly as the old code did when no signer was present (always, on web).
//
// History: this file used to host the serverless Stash cloud save client (a
// Nostr relay manifest plus Blossom blobs) and, before 2026-10-01, the lobby
// mailbox bridge. Both are gone; the vendored stash/relay/beacon bundles and
// beacon.js were deleted with them. The strings that must never return are
// pinned by tools/smoke/forbidden-symbol.mjs.
(function () {
  'use strict';

  // Per-pubkey localStorage namespace. auth.js publishes window.__axenstax_pubkey
  // (an anonymous guest key on web); empty -> the stable `local` namespace.
  function _cosmeticPubkey() {
    const fromAuth = typeof window.__axenstax_pubkey === 'string' ? window.__axenstax_pubkey : '';
    return /^[0-9a-f]{64}$/i.test(fromAuth) ? fromAuth.toLowerCase() : '';
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

  // --- Cosmetics: the player's 64x64 PNG skin, localStorage only. ---
  window.axenstax_cosmetic_save = async (bytes) => {
    const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    const key = _cosmeticKey();
    if (key) { try { localStorage.setItem(key, _b64FromBytes(u8)); } catch (_) {} }
  };
  window.axenstax_cosmetic_load = async () => {
    const key = _cosmeticKey();
    if (key) {
      try {
        const s = localStorage.getItem(key);
        if (s) return _bytesFromB64(s);
      } catch (_) {}
    }
    return null;
  };
  window.axenstax_cosmetic_reset = async () => {
    const key = _cosmeticKey();
    if (key) { try { localStorage.removeItem(key); } catch (_) {} }
  };

  // --- Skin wardrobe (Phase 1c): the avatar-skin wardrobe blob. LOCAL ONLY
  // (web-taster minimal-data posture, spec 2026-06-29 section 10). Per-pubkey
  // when a key is published; a stable `local` ns for the anonymous taster so
  // skins survive a refresh on this device.
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

  // --- Workshop wardrobe (Spec 40): was Stash-only, so on web it never held
  // anything. Kept as inert no-ops: load -> null, save/reset -> nothing.
  window.axenstax_wardrobe_save = async () => {};
  window.axenstax_wardrobe_load = async () => null;
  window.axenstax_wardrobe_reset = async () => {};
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

  // --- Inert stubs for engine externs that must still resolve ---
  // Cloud save (wasm_save.rs): never available on web. The engine polls
  // `cloud_available()` and `sync_stash_status()` (menu.rs::poll_cloud_worlds)
  // without a try/catch around a missing global, so these must exist.
  window.axenstax_cloud_available = () => false;
  window.axenstax_cloud_list = async () => '[]';
  window.axenstax_cloud_restore = async () => { throw new Error('cloud save unavailable on web'); };
  window.axenstax_sync_stash_status = () =>
    JSON.stringify({ state: 'idle', message: '', done: 0, total: 0 });

  // Open-stash sharing (open_stash.rs): `beacon_available()` is called unguarded.
  window.axenstax_openstash_available = () => false;
  const _unavailable = async () => { throw new Error('open-stash unavailable on web'); };
  window.axenstax_openstash_publish = _unavailable;
  window.axenstax_openstash_list = _unavailable;
  window.axenstax_openstash_download = _unavailable;
  window.axenstax_openstash_follow = _unavailable;
  window.axenstax_openstash_unfollow = _unavailable;
  window.axenstax_openstash_following = _unavailable;
})();
