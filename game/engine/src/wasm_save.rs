//! WASM world persistence — IndexedDB wrappers + re-exports of archive helpers.
//!
//! World blobs are tar+gzip (same format as native), written to IndexedDB via
//! the small `world_store.js` helper (no encryption at rest for alpha; per-pubkey
//! keying is the only isolation). Serialisation stays on the Rust side; JS owns
//! the IDB transaction.
//!
//! Scoping is per-pubkey: the JS layer keys records by "<pubkey>:<world_name>"
//! and won't return another pubkey's entries.
//!
//! The pack/unpack logic and `dedupe_world_name` now live in
//! `crate::world_archive` (cross-platform, no cfg gate). This file re-exports
//! them so existing WASM call sites are unchanged.

// Re-export cross-platform archive helpers so call sites in this file don't need
// to change (they reference `pack_world`, `dedupe_world_name` directly). Only the
// WASM-gated functions below actually call them, so the re-export is gated the
// same way (else it's unused on the native build). The play path unpacks with
// `world_archive::unpack_world_to_play` (Spec 02 §8.4), so `unpack_world` is not
// re-exported here.
#[cfg(target_arch = "wasm32")]
pub use crate::world_archive::{dedupe_world_name, pack_world};

#[cfg(target_arch = "wasm32")]
use crate::save::{WorldMeta, WorldSave};
#[cfg(target_arch = "wasm32")]
use crate::world::World;

#[cfg(target_arch = "wasm32")]
use js_sys::Uint8Array;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------------
// JS glue — window.axenstax_* IndexedDB wrappers from world_store.js
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = axenstax_save_world, catch)]
    async fn js_save_world(
        pubkey: String,
        name: String,
        blob: Uint8Array,
        meta_json: String,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = axenstax_load_world, catch)]
    async fn js_load_world(pubkey: String, name: String) -> Result<JsValue, JsValue>;

    /// Exhibits (Creator Gallery) — fetch one authored exhibit image same-origin
    /// (`/exhibits/<ref>`), served by the dedicated server from the world's
    /// exhibit set. `catch` so an absent bridge / 404 surfaces as Err and the
    /// engine skips that exhibit. NOTE: callers MUST `bridge_present(...)` first —
    /// the dedicated-server page ships no /static JS, and calling an absent async
    /// bridge throws an uncaught TypeError (see `bridge_present`).
    #[wasm_bindgen(js_name = axenstax_load_exhibit_art, catch)]
    pub(crate) async fn js_load_exhibit_art(
        world: String,
        image_ref: String,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = axenstax_list_worlds, catch)]
    async fn js_list_worlds(pubkey: String) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = axenstax_delete_world, catch)]
    async fn js_delete_world(pubkey: String, name: String) -> Result<JsValue, JsValue>;

    // `axenstax_export_world` (raw-blob download) is deliberately NOT bound:
    // the raw blob carries the PoP secret. `export_world_wasm` re-packs.

    // "Take your worlds to native" — download a packed `.axeprofile` bundle.
    // `name` already carries the extension (built by `profile_filename`).
    #[wasm_bindgen(js_name = axenstax_profile_download, catch)]
    async fn js_profile_download(name: String, bytes: Uint8Array) -> Result<JsValue, JsValue>;

    // Resolves with an object { name: String, bytes: Uint8Array }, or null if cancelled.
    #[wasm_bindgen(js_name = axenstax_pick_world_file, catch)]
    async fn js_pick_world_file() -> Result<JsValue, JsValue>;

    // --- Cloud save (Stash, serverless) — window.AxeCloud bridge ---
    // The push/sync side (world upload) was removed 2026-07-09 (confirmed dead:
    // no reachable web trigger since web login retirement — see
    // docs/superpowers/specs/2026-07-09-give-aliases-and-dead-web-stash-code.md).
    // `available`/`list`/`restore` remain: they back the read-only cloud-world
    // merge in `menu.rs::poll_cloud_worlds`, which is itself inert (no signer
    // reachable) but shares list/rendering code with the live local world list.
    #[wasm_bindgen(js_name = axenstax_cloud_available)]
    fn js_cloud_available() -> bool;

    // Returns a JSON string: [{ name, blobHash, size, updated, kind }].
    #[wasm_bindgen(js_name = axenstax_cloud_list, catch)]
    async fn js_cloud_list() -> Result<JsValue, JsValue>;

    // Returns the decrypted world bytes for a blob hash.
    #[wasm_bindgen(js_name = axenstax_cloud_restore, catch)]
    async fn js_cloud_restore(blob_hash: String) -> Result<JsValue, JsValue>;

    // --- Cosmetics (Phase 2): the player's 64×64 PNG skin ---
    // Picker resolves with { name: String, bytes: Uint8Array }, or null if cancelled.
    #[wasm_bindgen(js_name = axenstax_pick_skin_file, catch)]
    async fn js_pick_skin_file() -> Result<JsValue, JsValue>;

    // Persist the raw PNG to localStorage (per-pubkey) + Stash (best-effort).
    #[wasm_bindgen(js_name = axenstax_cosmetic_save, catch)]
    async fn js_cosmetic_save(bytes: Uint8Array) -> Result<JsValue, JsValue>;

    // Load the persona's skin PNG, or null if none stored.
    #[wasm_bindgen(js_name = axenstax_cosmetic_load, catch)]
    async fn js_cosmetic_load() -> Result<JsValue, JsValue>;

    // Clear the persona's skin from both tiers.
    #[wasm_bindgen(js_name = axenstax_cosmetic_reset, catch)]
    async fn js_cosmetic_reset() -> Result<JsValue, JsValue>;

    // --- Wardrobe (Spec 40): the player's global Workshop wardrobe blob ---
    #[wasm_bindgen(js_name = axenstax_wardrobe_save, catch)]
    async fn js_wardrobe_save(bytes: Uint8Array) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_wardrobe_load, catch)]
    async fn js_wardrobe_load() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_wardrobe_reset, catch)]
    async fn js_wardrobe_reset() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_wardrobe_remember_get)]
    fn js_wardrobe_remember_get() -> bool;
    #[wasm_bindgen(js_name = axenstax_wardrobe_remember_set)]
    fn js_wardrobe_remember_set(on: bool);

    // --- Skin wardrobe (Phase 1c): the player's avatar-skin wardrobe blob.
    // Web = localStorage only (no Stash), per the web-taster minimal-data posture.
    #[wasm_bindgen(js_name = axenstax_skinwardrobe_save, catch)]
    async fn js_skinwardrobe_save(bytes: Uint8Array) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = axenstax_skinwardrobe_load, catch)]
    async fn js_skinwardrobe_load() -> Result<JsValue, JsValue>;

    // --- Skin EXPORT: trigger a browser download of the PNG. `name` already
    // has the `.png` extension; `bytes` is the raw PNG (not RGBA).
    #[wasm_bindgen(js_name = axenstax_skin_download, catch)]
    async fn js_skin_download(name: String, bytes: Uint8Array) -> Result<JsValue, JsValue>;

    // --- Ghost EXPORT (Campaign G): download an `.axeghost` share file.
    #[wasm_bindgen(js_name = axenstax_ghost_download, catch)]
    async fn js_ghost_download(name: String, bytes: Uint8Array) -> Result<JsValue, JsValue>;

    // --- Ghost IMPORT (Campaign G): picker resolves with
    // { name: String, bytes: Uint8Array }, or null if cancelled.
    #[wasm_bindgen(js_name = axenstax_pick_ghost_file, catch)]
    async fn js_pick_ghost_file() -> Result<JsValue, JsValue>;

    // --- Skin IMPORT from Minecraft (web proxy). `query` is `name=<u>` or
    // `uuid=<hex>`. Returns a JS object: { uuid, name, slim, bytes } on success,
    // or { error: "<code>" } on structured failure. Any transport fault → null.
    #[wasm_bindgen(js_name = axenstax_mc_skin_fetch, catch)]
    async fn js_mc_skin_fetch(query: String) -> Result<JsValue, JsValue>;

    // "Sync Stash" batch status (cloud.js). The start/cancel side was removed
    // 2026-07-09 — no button ever called `sync_stash_start` (0 Rust callers found
    // pre-removal; `menu.rs::sync_button_label` was `#[allow(dead_code)]`, "no
    // button widget calls this yet"). `status` remains: `poll_cloud_worlds`
    // reads it every frame and degrades safely to a permanent "idle" JSON.
    #[wasm_bindgen(js_name = axenstax_sync_stash_status)]
    fn js_sync_stash_status() -> String;
}

/// A cloud world entry from the Stash manifest (decrypted client-side).
#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize, Clone, Debug)]
pub struct CloudWorldEntry {
    pub name: String,
    #[serde(rename = "blobHash")]
    pub blob_hash: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub updated: i64,
}

#[cfg(target_arch = "wasm32")]
fn jsval_err(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

/// True when the named global glue function (e.g. `window.axenstax_list_worlds`)
/// is actually present on the page.
///
/// The dedicated-server page (`index.dedicated.html`) — and any minimal
/// embedder — serves the engine WITHOUT the website's `/static/*.js` bridge
/// layer. Calling an absent `catch` async bridge constructs
/// `JsFuture::from(undefined)`, whose `.then` lookup throws an uncaught
/// `TypeError: Cannot read properties of undefined (reading 'then')` and
/// hard-crashes the boot — exactly what the lobby's `list_worlds` fetch did on
/// the Docker page (red error bar, stuck on "Starting game…"). Each wrapper
/// below consults this first and degrades to a safe default (empty world list /
/// cloud-off / no skin) instead of crashing. On the full website the bridges are
/// present, so this always returns `true` and behaviour is unchanged.
#[cfg(target_arch = "wasm32")]
pub(crate) fn bridge_present(name: &str) -> bool {
    js_sys::Reflect::get(js_sys::global().as_ref(), &JsValue::from_str(name))
        .map(|v| v.is_function())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// IndexedDB wrappers (delegate to world_store.js)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
pub async fn save_world_wasm(
    pubkey: &str,
    name: &str,
    blob: Vec<u8>,
    meta_json: String,
) -> Result<(), String> {
    if !bridge_present("axenstax_save_world") {
        return Ok(());
    }
    let u8 = Uint8Array::new_with_length(blob.len() as u32);
    u8.copy_from(&blob);
    js_save_world(pubkey.to_string(), name.to_string(), u8, meta_json)
        .await
        .map_err(jsval_err)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub async fn load_world_wasm(pubkey: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
    if !bridge_present("axenstax_load_world") {
        return Ok(None);
    }
    let jsval = js_load_world(pubkey.to_string(), name.to_string())
        .await
        .map_err(jsval_err)?;
    if jsval.is_null() || jsval.is_undefined() {
        return Ok(None);
    }
    let u8 = Uint8Array::new(&jsval);
    Ok(Some(u8.to_vec()))
}

#[cfg(target_arch = "wasm32")]
pub async fn list_worlds_wasm(pubkey: &str) -> Result<Vec<LocalWorldEntry>, String> {
    // No local world-store bridge (dedicated-server page) → no local worlds.
    if !bridge_present("axenstax_list_worlds") {
        return Ok(Vec::new());
    }
    let jsval = js_list_worlds(pubkey.to_string())
        .await
        .map_err(jsval_err)?;
    let txt = js_sys::JSON::stringify(&jsval)
        .map_err(|e| format!("stringify: {e:?}"))?
        .as_string()
        .unwrap_or_else(|| "[]".to_string());
    serde_json::from_str(&txt).map_err(|e| format!("parse: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub async fn delete_world_wasm(pubkey: &str, name: &str) -> Result<(), String> {
    if !bridge_present("axenstax_delete_world") {
        return Ok(());
    }
    js_delete_world(pubkey.to_string(), name.to_string())
        .await
        .map_err(jsval_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Cloud save (Stash, serverless) — best-effort bridge to window.AxeCloud
// ---------------------------------------------------------------------------

/// Whether cloud save is usable right now (enabled + a NIP-44-capable signer).
#[cfg(target_arch = "wasm32")]
pub fn cloud_available() -> bool {
    if !bridge_present("axenstax_cloud_available") {
        return false;
    }
    js_cloud_available()
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Set when a cloud save completes (the manifest gained an entry). The lobby polls
    /// this so a Stash-on world's traffic light flips amber → green on its own, without
    /// the player needing to re-enter the lobby to refresh.
    static CLOUD_DIRTY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Signal that the cloud manifest changed — the lobby re-fetches on the next poll.
#[cfg(target_arch = "wasm32")]
pub fn mark_cloud_dirty() {
    CLOUD_DIRTY.with(|c| c.set(true));
}

/// Consume the cloud-dirty flag (true once after a cloud save lands).
#[cfg(target_arch = "wasm32")]
pub fn take_cloud_dirty() -> bool {
    CLOUD_DIRTY.with(|c| c.replace(false))
}

/// Current "Sync Stash" status as a JSON string `{state,message,done,total}`.
/// `state` ∈ {idle, syncing, done, cancelled, error}.
#[cfg(target_arch = "wasm32")]
pub fn sync_stash_status() -> String {
    if !bridge_present("axenstax_sync_stash_status") {
        return String::new();
    }
    js_sync_stash_status()
}

/// List the persona's cloud worlds from the Stash manifest.
#[cfg(target_arch = "wasm32")]
pub async fn cloud_list_wasm() -> Result<Vec<CloudWorldEntry>, String> {
    let jsval = js_cloud_list().await.map_err(jsval_err)?;
    let txt = jsval.as_string().unwrap_or_else(|| "[]".to_string());
    serde_json::from_str(&txt).map_err(|e| format!("cloud list parse: {e}"))
}

/// Download + decrypt a cloud world blob by hash into the packed bytes the
/// normal load path (`unpack`) consumes.
#[cfg(target_arch = "wasm32")]
pub async fn cloud_restore_wasm(blob_hash: &str) -> Result<Vec<u8>, String> {
    let jsval = js_cloud_restore(blob_hash.to_string())
        .await
        .map_err(jsval_err)?;
    if jsval.is_null() || jsval.is_undefined() {
        return Err("cloud restore: empty".to_string());
    }
    let u8 = Uint8Array::new(&jsval);
    Ok(u8.to_vec())
}

// ---------------------------------------------------------------------------
// Cosmetics (Phase 2) — pick a PNG skin + persist it (localStorage + Stash)
// ---------------------------------------------------------------------------

/// Open a PNG file picker and return the raw bytes of the chosen file.
///
/// `Err("cancelled")` means the picker was dismissed — callers treat this as a
/// no-op, not an error to surface. Validation (64×64, real PNG) is the caller's
/// job; this just hands back the bytes.
#[cfg(target_arch = "wasm32")]
pub async fn pick_skin_wasm() -> Result<Vec<u8>, String> {
    let picked = js_pick_skin_file().await.map_err(jsval_err)?;
    if picked.is_null() || picked.is_undefined() {
        return Err("cancelled".to_string());
    }
    let bytes_val = js_sys::Reflect::get(&picked, &JsValue::from_str("bytes"))
        .map_err(|_| "couldn't read that file".to_string())?;
    Ok(Uint8Array::new(&bytes_val).to_vec())
}

/// Persist the player's skin PNG to both tiers (localStorage + Stash).
/// Fire-and-forget: the JS side swallows failures (cloud is best-effort, the
/// local tier already succeeded), so this never errors.
#[cfg(target_arch = "wasm32")]
pub async fn cosmetic_save_wasm(png: Vec<u8>) {
    let arr = Uint8Array::from(png.as_slice());
    let _ = js_cosmetic_save(arr).await;
}

/// Load the persona's stored skin PNG, or `None` if none is stored (or on any
/// error — a missing skin and a failed read both degrade to the default avatar).
#[cfg(target_arch = "wasm32")]
pub async fn cosmetic_load_wasm() -> Option<Vec<u8>> {
    if !bridge_present("axenstax_cosmetic_load") {
        return None;
    }
    match js_cosmetic_load().await {
        Ok(v) if !v.is_null() && !v.is_undefined() => Some(Uint8Array::new(&v).to_vec()),
        _ => None,
    }
}

/// Clear the persona's skin from both tiers. Best-effort, never errors.
#[cfg(target_arch = "wasm32")]
pub async fn cosmetic_reset_wasm() {
    let _ = js_cosmetic_reset().await;
}

/// Persist the player's global wardrobe blob to the private Stash (best-effort).
#[cfg(target_arch = "wasm32")]
pub async fn wardrobe_save_wasm(bytes: Vec<u8>) {
    let arr = Uint8Array::from(bytes.as_slice());
    let _ = js_wardrobe_save(arr).await;
}
/// Load the player's global wardrobe blob, or `None` if none stored / on error.
#[cfg(target_arch = "wasm32")]
pub async fn wardrobe_load_wasm() -> Option<Vec<u8>> {
    if !bridge_present("axenstax_wardrobe_load") {
        return None;
    }
    match js_wardrobe_load().await {
        Ok(v) if !v.is_null() && !v.is_undefined() => Some(Uint8Array::new(&v).to_vec()),
        _ => None,
    }
}

/// Persist the avatar-skin wardrobe blob to this device's localStorage
/// (per-pubkey or the `local` ns for the anonymous taster). Web is local-only
/// — no Stash (spec §10). Fire-and-forget: the JS side swallows failures.
#[cfg(target_arch = "wasm32")]
pub async fn skinwardrobe_save_wasm(bytes: Vec<u8>) {
    let arr = Uint8Array::from(bytes.as_slice());
    let _ = js_skinwardrobe_save(arr).await;
}

/// Load the avatar-skin wardrobe blob, or `None` if none stored / on error.
#[cfg(target_arch = "wasm32")]
pub async fn skinwardrobe_load_wasm() -> Option<Vec<u8>> {
    if !bridge_present("axenstax_skinwardrobe_load") {
        return None;
    }
    match js_skinwardrobe_load().await {
        Ok(v) if !v.is_null() && !v.is_undefined() => Some(Uint8Array::new(&v).to_vec()),
        _ => None,
    }
}

/// Trigger a browser download of an exported skin PNG. Fire-and-forget: drive
/// via `spawn_local`; the JS side swallows failures (export is best-effort).
#[cfg(target_arch = "wasm32")]
pub async fn skin_download_wasm(name: String, png: Vec<u8>) {
    if !bridge_present("axenstax_skin_download") {
        return;
    }
    let arr = Uint8Array::from(png.as_slice());
    let _ = js_skin_download(name, arr).await;
}

/// Campaign G — whether the ghost-download JS bridge is on this page. The
/// caller checks this BEFORE toasting success (review fix: a stale cached
/// page must produce an error toast, not a false "downloaded!").
#[cfg(target_arch = "wasm32")]
pub fn ghost_download_bridge_present() -> bool {
    bridge_present("axenstax_ghost_download")
}

/// Campaign G — trigger a browser download of an `.axeghost` share file.
/// Fire-and-forget, mirroring `skin_download_wasm`. Callers gate on
/// [`ghost_download_bridge_present`] first.
#[cfg(target_arch = "wasm32")]
pub async fn ghost_download_wasm(name: String, bytes: Vec<u8>) {
    if !bridge_present("axenstax_ghost_download") {
        return;
    }
    let arr = Uint8Array::from(bytes.as_slice());
    let _ = js_ghost_download(name, arr).await;
}

/// Campaign G — open an `.axeghost` file picker and return the chosen file's
/// bytes. `Err("cancelled")` means the picker was dismissed (a no-op, not an
/// error to surface). Parse/validation is the caller's job.
#[cfg(target_arch = "wasm32")]
pub async fn pick_ghost_wasm() -> Result<Vec<u8>, String> {
    if !bridge_present("axenstax_pick_ghost_file") {
        // Review fix: a stale cached page has no picker bridge — say so
        // instead of silently doing nothing ("cancelled" is a no-op drain).
        return Err("this page needs a refresh first (press reload)".to_string());
    }
    let picked = js_pick_ghost_file().await.map_err(jsval_err)?;
    if picked.is_null() || picked.is_undefined() {
        return Err("cancelled".to_string());
    }
    let bytes_val = js_sys::Reflect::get(&picked, &JsValue::from_str("bytes"))
        .map_err(|_| "couldn't read that file".to_string())?;
    Ok(Uint8Array::new(&bytes_val).to_vec())
}

/// Fetch a Minecraft skin via our /mc-skin proxy. `query` is `name=<u>` or
/// `uuid=<hex>`. Returns a structured outcome; any transport fault → Offline.
#[cfg(target_arch = "wasm32")]
pub async fn mc_skin_fetch_wasm(query: String) -> crate::mc_import::McImportOutcome {
    use crate::mc_import::{McImportError, McImportOutcome};
    if !bridge_present("axenstax_mc_skin_fetch") {
        return McImportOutcome::Err(McImportError::Offline);
    }
    let v = match js_mc_skin_fetch(query).await {
        Ok(v) if !v.is_null() && !v.is_undefined() => v,
        _ => return McImportOutcome::Err(McImportError::Offline),
    };
    // Structured error?
    if let Ok(err) = js_sys::Reflect::get(&v, &JsValue::from_str("error")) {
        if let Some(code) = err.as_string() {
            let e = match code.as_str() {
                "not_found" => McImportError::NotFound,
                "no_custom_skin" => McImportError::NoCustomSkin,
                "rate_limited" => McImportError::RateLimited,
                "bad_name" => McImportError::BadName,
                _ => McImportError::Offline,
            };
            return McImportOutcome::Err(e);
        }
    }
    let get_str = |k: &str| {
        js_sys::Reflect::get(&v, &JsValue::from_str(k))
            .ok()
            .and_then(|x| x.as_string())
            .unwrap_or_default()
    };
    let slim = js_sys::Reflect::get(&v, &JsValue::from_str("slim"))
        .ok()
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let bytes = match js_sys::Reflect::get(&v, &JsValue::from_str("bytes")) {
        Ok(b) if !b.is_null() && !b.is_undefined() => Uint8Array::new(&b).to_vec(),
        _ => return McImportOutcome::Err(McImportError::Offline),
    };
    McImportOutcome::Ok {
        png: bytes,
        uuid: get_str("uuid"),
        name: get_str("name"),
        slim,
    }
}

#[cfg(target_arch = "wasm32")]
pub async fn wardrobe_reset_wasm() {
    let _ = js_wardrobe_reset().await;
}
#[cfg(target_arch = "wasm32")]
pub fn wardrobe_remember_get_wasm() -> bool {
    if !bridge_present("axenstax_wardrobe_remember_get") {
        return false;
    }
    js_wardrobe_remember_get()
}
#[cfg(target_arch = "wasm32")]
pub fn wardrobe_remember_set_wasm(on: bool) { js_wardrobe_remember_set(on); }

// ---------------------------------------------------------------------------
// Local World Backup — export to file / import from file (2026-05-27 spec)
// ---------------------------------------------------------------------------

/// Trigger a browser download of the packed world blob (`<name>.axeworld`).
/// Fire-and-forget: callers drive this via `spawn_local`, so failures are
/// logged rather than surfaced.
#[cfg(target_arch = "wasm32")]
pub async fn export_world_wasm(pubkey: String, name: String) {
    // The IndexedDB blob is owner persistence and carries the host-only PoP
    // secret; a download is a share, so re-pack it stripped (Spec 06 §2.2)
    // and hand the bytes to the generic download bridge.
    let result: Result<(), String> = async {
        let blob = load_world_wasm(&pubkey, &name)
            .await?
            .ok_or_else(|| format!("world not found: {name}"))?;
        let shared = crate::world_archive::strip_secret_from_archive(&blob)?;
        let u8 = Uint8Array::new_with_length(shared.len() as u32);
        u8.copy_from(&shared);
        js_profile_download(format!("{name}.axeworld"), u8)
            .await
            .map_err(jsval_err)?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        log::warn!("Export world failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Whole-profile export — "Take your worlds to native" (roadmap)
// ---------------------------------------------------------------------------

/// Today's date as `YYYY-MM-DD`, for the profile filename. Falls back to a
/// plain label if the browser hands back something unexpected — a filename is
/// never worth failing an export over.
#[cfg(target_arch = "wasm32")]
fn today_iso() -> String {
    let iso = js_sys::Date::new_0().to_iso_string();
    let s = iso.as_string().unwrap_or_default();
    if s.len() >= 10 && s.is_char_boundary(10) {
        s[..10].to_string()
    } else {
        "profile".to_string()
    }
}

/// Gather EVERYTHING this browser profile holds — every saved world plus the
/// Trials records — into one `.axeprofile` bundle and download it.
///
/// Web saves are local-only by design, so this is how a player carries their
/// work to the desktop app: one file, one click, no account and no upload. The
/// worlds go in as the exact `.axeworld` bytes the per-world export already
/// produces (see `crate::profile_bundle`), so nothing is re-encoded.
///
/// Returns the status line to show. A profile with no worlds still exports
/// (the Trials records are worth carrying on their own) and the message says so.
#[cfg(target_arch = "wasm32")]
pub async fn export_profile_wasm(pubkey: String) -> Result<String, String> {
    use crate::profile_bundle::{pack, profile_filename, Entry, EntryKind};

    if !bridge_present("axenstax_profile_download") {
        // A stale cached page has no download bridge — say what to do rather
        // than silently producing nothing.
        return Err("This page needs a refresh first (press reload).".to_string());
    }

    let listed = list_worlds_wasm(&pubkey).await.unwrap_or_default();
    let mut entries: Vec<Entry> = Vec::with_capacity(listed.len() + 1);
    let mut skipped = 0usize;
    for e in &listed {
        match load_world_wasm(&pubkey, &e.name).await {
            // A bundle is a file that can be shared: strip the PoP secret
            // (Spec 06 §2.2); the native import gives the world a fresh one.
            Ok(Some(blob)) => match crate::world_archive::strip_secret_from_archive(&blob) {
                Ok(bytes) => entries.push(Entry {
                    kind: EntryKind::World,
                    name: e.name.clone(),
                    bytes,
                }),
                Err(err) => {
                    skipped += 1;
                    log::warn!("Profile export: couldn't re-pack world '{}': {err}", e.name);
                }
            },
            _ => {
                skipped += 1;
                log::warn!("Profile export: couldn't read world '{}'", e.name);
            }
        }
    }
    let worlds = entries.len();

    // The Trials store always travels, even when empty — the desktop side
    // merges it, and an empty merge is a no-op.
    entries.push(Entry {
        kind: EntryKind::Trials,
        name: "trials".to_string(),
        bytes: crate::trials::TrialBests::load().to_json().into_bytes(),
    });

    let packed = pack(&entries);
    let arr = Uint8Array::from(packed.as_slice());
    js_profile_download(profile_filename(&today_iso()), arr)
        .await
        .map_err(jsval_err)?;

    let mut msg = match worlds {
        0 => "No worlds saved here yet — exported your Trials records only. Open it in the desktop app with \"Import a web profile…\".".to_string(),
        1 => "Exported 1 world + your Trials records. Open it in the desktop app with \"Import a web profile…\".".to_string(),
        n => format!("Exported {n} worlds + your Trials records. Open it in the desktop app with \"Import a web profile…\"."),
    };
    if skipped > 0 {
        msg.push_str(&format!(" ({skipped} world(s) couldn't be read.)"));
    }
    Ok(msg)
}

/// Pick a `.axeworld` file, validate it's a real Axe'n'Stax world, and save it
/// under `pubkey`. On a name collision the world is imported as a copy (never
/// overwrites). Returns the final stored world name.
///
/// `Err("cancelled")` means the picker was dismissed — callers treat this as a
/// no-op, not an error to surface.
#[cfg(target_arch = "wasm32")]
pub async fn import_world_wasm(pubkey: String) -> Result<String, String> {
    let picked = js_pick_world_file().await.map_err(jsval_err)?;
    if picked.is_null() || picked.is_undefined() {
        return Err("cancelled".to_string());
    }

    // Extract { name, bytes } from the picker result.
    let name: String = js_sys::Reflect::get(&picked, &JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string())
        .ok_or_else(|| "bad file: missing name".to_string())?;
    let bytes_val = js_sys::Reflect::get(&picked, &JsValue::from_str("bytes"))
        .map_err(|_| "bad file: missing bytes".to_string())?;
    let bytes = Uint8Array::new(&bytes_val).to_vec();

    // Validate it unpacks to a real world + read its meta. A throwaway World
    // absorbs the chunks; we only need the meta for the stored record.
    // An imported file never keeps the exporter's PoP secret: unpack with a
    // fresh one and re-pack, so the stored blob (the web load path reads the
    // meta from it) carries the new secret (Spec 06 §2.2).
    let mut scratch = World::new();
    let (meta, save, images) = crate::world_archive::unpack_world_for_import(&bytes, &mut scratch)
        .map_err(|_| "That's not an Axe'n'Stax world.".to_string())?;
    let bytes = pack_world(&meta, &save, &scratch, &images)?;

    // Compute a non-colliding name against the player's existing worlds.
    let existing: Vec<String> = list_worlds_wasm(&pubkey)
        .await
        .map(|entries| entries.into_iter().map(|e| e.name).collect())
        .unwrap_or_default();
    let final_name = dedupe_world_name(&name, &existing);

    let meta_json = serde_json::to_string(&meta)
        .map_err(|e| format!("serialise meta: {e}"))?;
    let u8 = Uint8Array::new_with_length(bytes.len() as u32);
    u8.copy_from(&bytes);
    js_save_world(pubkey, final_name.clone(), u8, meta_json)
        .await
        .map_err(jsval_err)?;

    Ok(final_name)
}

/// Shape returned by `list_worlds_wasm` (mirrors the JSON emitted by world_store.js).
/// wasm32-only consumer (menu.rs's local-worlds fetch), invisible to a native
/// `cargo clippy` run.
#[derive(Clone, Debug, serde::Deserialize)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct LocalWorldEntry {
    pub name: String,
    pub size: u64,
    pub last_saved: i64,
    pub game_mode: String,
    pub display_name: String,
    pub description: String,
    #[serde(default = "default_difficulty")]
    pub difficulty: String,
    /// Per-world Stash opt-in, surfaced by world_store.js `list()`. Absent in
    /// legacy payloads → defaults to false (privacy-first).
    #[serde(default)]
    pub cloud_save: bool,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn default_difficulty() -> String {
    "normal".to_string()
}

// Tests for dedupe_world_name and pack/unpack round-trip live in
// `crate::world_archive` (they are cross-platform and run on native).
