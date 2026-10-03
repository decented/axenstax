# Cloud Save Phase 5 — Wire upload into world exit

**Status:** ⚠️ **SUPERSEDED 2026-06-01** — kept for history. This was the narrow "wire upload into exit, unencrypted/server-encrypted" design. Later in the same brainstorm the requirement was reframed: the real goal is **identity-portable, zero-knowledge, cross-game saves** (log in as your persona anywhere → get your stuff), which is a **Forgesworn-level primitive**, not an AxeNStax upload step. See:
> - `docs/architecture/2026-06-01-persona-save-vault.md` — the per-persona encrypted save vault (the real design).
> - `docs/superpowers/specs/2026-06-01-signer-retention-nip44-design.md` — the first concrete, unblocked step (retain signer + expose NIP-44).
>
> "Phase 5" is now reduced to "AxeNStax becomes a consumer of the vault." The original content below is retained only as the record of the upload-trigger/UX decisions (save-on-exit not autosave, per-world toggle default-off, DOM indicator) which still inform the consumer wiring.

**Branch:** (original) `feat/cloud-save-phase-5`.

---

## Goal

When a player deliberately saves/exits a world, push the same blob that was just written to IndexedDB up to the cloud — fire-and-forget, never blocking play, with a small DOM status indicator. Autosave stays **local-only**. This is the first phase that makes "save online" do anything at runtime.

## Decisions (owner, 2026-06-01)

1. **Trigger:** upload on **deliberate save/exit only** (the in-game pause-menu "Save" and "Save & Quit"), **not** on autosave. Autosave remains local IndexedDB only.
2. **Exit scope:** **leave-to-menu only.** No `sendBeacon`/tab-close upload (local autosave already protects the bytes; cloud is as fresh as the last proper save/exit).
3. **Failure UX:** non-blocking. Local save always succeeds first; the cloud upload shows a **subtle DOM status indicator** (saving / synced / failed) and never blocks or errors into gameplay.
4. **Transport:** engine → JS bridge (`window.axenstax_cloud_upload`) → existing `blossom.js` `Blossom.upload` → `POST /worlds/upload`. Engine stays transport-agnostic (mirrors `world_store.js`).
5. **Indicator location:** DOM overlay injected by `blossom.js` over the `/game` canvas (no engine HUD work, no rebuild to tweak).

## Why this is the natural seam

The codebase already splits the two cases by **function**, so behaviour rides on which one a caller already invokes — no call-site changes:

- Deliberate save/exit → `save::save_world` — only WASM callers are `game_loop.rs:2073` (`PAUSE_SAVE`) and `game_loop.rs:2082` (`PAUSE_SAVE_QUIT`). (`main.rs:756` is `#[cfg(not(wasm32))]`, native-only — irrelevant.)
- Autosave → `save::autosave_world` — only caller `game_loop.rs:550`.

Today `autosave_world` just delegates to `save_world` (`save.rs:1635`). We split them so the cloud upload lives only in the deliberate path.

---

## Components

### 1. `save.rs` — extract local-only inner, add upload to the outer (WASM)

Refactor the WASM `save_world` (`save.rs:790`) so the existing body becomes a local-only inner that returns what the upload needs:

```
#[cfg(target_arch = "wasm32")]
fn save_world_local(name, world, players, seed) -> Result<SavedBlob, String>
    // = everything save_world does today: build save_data + meta, pack_world,
    //   spawn_local IndexedDB write (unchanged).
    // Returns SavedBlob { pubkey: String, name: String, blob: Vec<u8> }
    //   so the caller can upload WITHOUT re-packing (reuse `compressed`).

#[cfg(target_arch = "wasm32")]
pub fn save_world(name, world, players, seed) -> Result<(), String>
    let saved = save_world_local(name, world, players, seed)?;
    // fire-and-forget cloud upload (separate spawn_local; never blocks, never errors out)
    spawn_local(async move {
        crate::wasm_save::cloud_upload_wasm(&saved.pubkey, &saved.name, saved.blob).await;
    });
    Ok(())

#[cfg(target_arch = "wasm32")]
pub fn autosave_world(name, world, players, seed) -> Result<(), String>
    save_world_local(name, world, players, seed)?;   // local ONLY — no upload
    Ok(())
```

`SavedBlob` is a tiny private struct in `save.rs`. The IndexedDB `spawn_local` inside `save_world_local` already moves a clone of `compressed`; the returned `blob` is the same bytes (clone once for the upload). Native `save_world`/`autosave_world` are untouched.

> Edge case: if `pubkey` is empty, `save_world_local` already returns `Err` before any blob exists (`save.rs:926-928`), so the upload path is never reached without a pubkey.

### 2. `wasm_save.rs` — extern binding + wrapper

Add to the existing `extern "C"` block (mirrors `js_save_world`):

```
#[wasm_bindgen(js_name = axenstax_cloud_upload, catch)]
async fn js_cloud_upload(pubkey: String, name: String, blob: Uint8Array) -> Result<JsValue, JsValue>;
```

And a wrapper (mirrors `save_world_wasm`), fire-and-forget semantics — logs, never propagates:

```
#[cfg(target_arch = "wasm32")]
pub async fn cloud_upload_wasm(pubkey: &str, name: &str, blob: Vec<u8>) {
    let u8 = Uint8Array::new_with_length(blob.len() as u32);
    u8.copy_from(&blob);
    match js_cloud_upload(pubkey.to_string(), name.to_string(), u8).await {
        Ok(_)  => log::info!("Cloud upload OK: '{name}'"),
        Err(e) => log::warn!("Cloud upload failed for '{name}': {}", jsval_err(e)),
    }
}
```

The JS hook itself decides whether cloud save is even on (returns a no-op resolve when disabled), so the engine never needs to know.

### 3. `blossom.js` — `axenstax_cloud_upload` hook + status indicator

Add a global the engine can call, plus a small DOM indicator. All in the existing `blossom.js` IIFE:

```
async function cloudUpload(pubkey, name, bytes) {
    if (!enabled()) return;            // inert when disabled — engine stays unaware
    setStatus('saving');
    try {
        await upload(name, bytes);     // existing Blossom.upload → POST /worlds/upload
        setStatus('synced');
    } catch (e) {
        console.warn('cloud upload failed:', e);
        setStatus('failed');           // local save already succeeded — non-blocking
    }
}
window.axenstax_cloud_upload = cloudUpload;
```

`setStatus(state)` manages one fixed-position DOM node (created lazily, top-right over the canvas):
- `saving` → "Saving to cloud…" (neutral)
- `synced` → "✓ Saved to cloud" then auto-fade after ~3s
- `failed` → "⚠ Cloud save failed — saved on this device" then auto-fade after ~5s

The node uses inline styles (CSP on `/game` allows `style-src 'unsafe-inline'`), `position: fixed`, high `z-index`, `pointer-events: none` so it never intercepts game input.

> `pubkey` is accepted for signature symmetry with `world_store.js` but unused — the server derives identity from the session cookie. Kept so the JS contract matches the engine extern and future phases (manifest labelling) have it.

### 4. `game/engine/index.html` — load the client on `/game`

Add, alongside the existing `/static/*` script loads (after `world_store.js`, ~line 98) and a meta tag in `<head>`:

```
<meta name="cloud-save" content="enabled">     <!-- or rendered; see note -->
<script src="/static/blossom.js"></script>
```

**Meta-tag note:** `/game` is served as static bytes from `dist/index.html` (`app.py:582`), NOT through Jinja, so we can't template `{{ cloud_save }}` there like the lobby does. Two options, pick at implementation:
  - (a) Hard-code `<meta name="cloud-save" content="enabled">` and rely on the **server** being the real gate — `blossom.js`'s `enabled()` would be true, but every `/worlds/upload` still returns 503 when the server has cloud save off, and `cloudUpload` catches that → "failed" indicator. Simplest; the client meta becomes advisory.
  - (b) Have `app.py`'s `game_gate` inject the meta into the served bytes (it already rewrites CSP/headers there), mirroring the lobby's enabled/disabled. Truthful client-side gate. **Recommended** — small string injection in `game_gate`, keeps `enabled()` honest so a disabled build shows no indicator and skips the fetch entirely.

This also closes the audit gap "blossom.js not loaded in /game".

---

## Data flow

```
player clicks Save / Save&Quit in pause menu (game_loop.rs:2073 / 2082)
  → save::save_world (WASM)
      → save_world_local: pack_world → spawn_local → js_save_world → IndexedDB   [unchanged, always first]
      → spawn_local → cloud_upload_wasm → js_cloud_upload
          → blossom.js cloudUpload
              → enabled()? no  → return (no-op, no indicator)
              → enabled()? yes → setStatus('saving') → Blossom.upload → POST /worlds/upload
                  → 200 → setStatus('synced')
                  → throw/503/offline → setStatus('failed')   [local copy already safe]

autosave loop (game_loop.rs:550)
  → save::autosave_world → save_world_local → IndexedDB only   [NO upload, ever]
```

## Error handling

- Upload is fire-and-forget in its own `spawn_local`: a failure cannot affect the local save (which already completed) or gameplay.
- `cloudUpload` swallows all errors → indicator only. No exception crosses the wasm-bindgen boundary (the `catch` extern + the JS `try/catch` both hold).
- Disabled / offline: `enabled()` short-circuits before any fetch; no indicator, no network call.
- Empty pubkey: `save_world_local` errors before packing (existing behaviour), so upload is never attempted unauthenticated.

## Testing

- **Rust (native-compilable):** the `save_world` / `autosave_world` split is `#[cfg(wasm32)]`, so it can't run under `cargo test` (native). Guard the *intent* instead: a comment-level invariant + ensure `check.sh` builds both native and WASM. (No new native unit test is meaningful here — the logic is JS-boundary glue.)
- **JS (`blossom.js`):** extend `test_worlds_routes.py` to assert `blossom.js` exposes `window.axenstax_cloud_upload` (served-content string check, mirroring the existing `window.Blossom` test). Assert `/game` (the served `dist/index.html` via `game_gate`) contains the `blossom.js` script tag and the `cloud-save` meta (covers the audit gap + option (b) injection).
- **End-to-end (deferred, needs docker + browser):** click Save&Quit in a real world → world appears in Blossom → indicator shows "synced". This is the Phase-10-style playtest gate; **cannot run on this host** (no docker), so it's documented, not executed.

## Out of scope (later phases)

- Download/restore wiring + world-list merge (Phase 7/8) — this phase is **upload only**.
- Manifest (Phase 6) — upload still returns the hash; nothing persists per-pubkey ownership yet.
- `sendBeacon` tab-close upload — explicitly declined.
- Encryption (Phase 4), destination-tier player-key signing (Spec 13).

## Memory-rule check

- **`feedback_npub_only_display`:** the indicator shows generic text ("Saved to cloud"), no pubkey. ✓
- **`project_pwa_priority` / shared-infra:** engine stays transport-agnostic; cloud logic is JS-layer + server. ✓
- **UK English** in any user-facing string. ✓
