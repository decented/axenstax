// Texture-pack fetch bridge for the WASM engine (texture-pack spec P4d,
// Spec 03 §11.6/§11.7). The browser has no filesystem, so the engine fetches
// packs from the game site over HTTPS and decodes them in-WASM.
//
//   /static/packs/index.json            -> [{ name, resolution, files:[keys] }]
//   /static/packs/<name>/<key>.png      -> a single override texture
//
// A pack only ships the textures it overrides (Spec 03 §11.4); `files` lists the
// texture keys it provides (no `.png`). These helpers are called by the engine's
// texture_packs_web.rs via #[wasm_bindgen] externs. Same-origin static fetches —
// no signer or relay involved. Errors resolve to empty/throw so the engine falls
// back to the procedural default rather than bricking.
(function () {
  'use strict';

  // Resolve to the raw JSON text of the pack index (or "[]" if absent/broken).
  async function listTexturePacks() {
    try {
      const resp = await fetch('/static/packs/index.json', { cache: 'no-cache' });
      if (!resp.ok) return '[]';
      return await resp.text();
    } catch (e) {
      console.warn('texture packs: index fetch failed', e);
      return '[]';
    }
  }

  // Resolve to a Uint8Array of one pack PNG. Names/keys are sanitised to a safe
  // path charset (defence-in-depth against traversal). Throws on HTTP error so
  // the engine skips that texture and keeps the procedural one.
  async function fetchPackFile(name, key) {
    const safeName = String(name).replace(/[^a-z0-9_-]/gi, '');
    const safeKey = String(key).replace(/[^a-z0-9_/-]/gi, '');
    if (!safeName || !safeKey) throw new Error('empty pack name/key');
    const url = '/static/packs/' + safeName + '/' + safeKey + '.png';
    const resp = await fetch(url, { cache: 'force-cache' });
    if (!resp.ok) throw new Error('pack file ' + url + ' HTTP ' + resp.status);
    const buf = await resp.arrayBuffer();
    return new Uint8Array(buf);
  }

  window.axenstax_list_texture_packs = listTexturePacks;
  window.axenstax_fetch_pack_file = fetchPackFile;
})();
