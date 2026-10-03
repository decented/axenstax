//! Web (WASM) texture-pack fetching (texture-pack spec P4 / Spec 03 §11.6/§11.7).
//!
//! The browser has no filesystem, so packs are **fetched over HTTPS and decoded
//! in-WASM**. The game site serves an aggregated index at
//! `/static/packs/index.json` (a `[WebPackDescriptor]`) and each pack's files at
//! `/static/packs/<name>/<key>.png`. Selecting a pack fetches its declared files,
//! decodes them with the `image` crate, and installs them via
//! [`crate::texture_registry::set_wasm_pack`] so the existing atlas-rebuild path
//! applies them — exactly like a native disk pack (P4a's shared core).
//!
//! Async bridges to the sync game loop through result slots (the established
//! wasm pattern): `select()` spawns the fetch, `take_applied()` is drained each
//! frame by `update_and_render`, which installs the pack + rebuilds the atlas.
//!
//! Only static per-key overrides for now; web animated textures + a hash-keyed
//! byte cache are follow-ups (the browser HTTP-caches same-origin packs already).

#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};

use wasm_bindgen::prelude::*;

use crate::resource_pack::{parse_pack_index, WebPackDescriptor};
use crate::texture_registry::DecodedTexture;

/// localStorage key holding the selected pack name (mirrors native `active.txt`).
const STORAGE_KEY: &str = "axenstax_texpack";

#[wasm_bindgen]
extern "C" {
    /// Resolve to the JSON text of `/static/packs/index.json` (or `"[]"`).
    #[wasm_bindgen(js_name = axenstax_list_texture_packs, catch)]
    async fn js_list_texture_packs() -> Result<JsValue, JsValue>;
    /// Resolve to a `Uint8Array` of `/static/packs/<name>/<key>.png`.
    #[wasm_bindgen(js_name = axenstax_fetch_pack_file, catch)]
    async fn js_fetch_pack_file(name: String, key: String) -> Result<JsValue, JsValue>;
}

/// A fetched + decoded pack ready to install.
pub struct Applied {
    pub name: String,
    pub resolution: u32,
    pub textures: Vec<DecodedTexture>,
}

thread_local! {
    /// The fetched pack index, `None` until the one-shot fetch resolves.
    static LIST: RefCell<Option<Vec<WebPackDescriptor>>> = const { RefCell::new(None) };
    static LIST_FETCHING: Cell<bool> = const { Cell::new(false) };
    /// A completed fetch/decode (or revert), drained by the game loop.
    static APPLY_SLOT: RefCell<Option<Result<Applied, String>>> = const { RefCell::new(None) };
    /// Saved-pack re-apply deferred until the index is available.
    static PENDING_SAVED: RefCell<Option<String>> = const { RefCell::new(None) };
    static INIT_DONE: Cell<bool> = const { Cell::new(false) };
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The persisted picker selection (`"Default"` when none) — what the web picker
/// shows as selected.
pub fn saved_selection() -> String {
    local_storage()
        .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Default".to_string())
}

fn persist_selection(name: &str) {
    if let Some(s) = local_storage() {
        let _ = s.set_item(STORAGE_KEY, name);
    }
}

/// Kick off the one-shot index fetch if it hasn't started/completed. Idempotent —
/// safe to call every frame the picker is open.
pub fn ensure_list_fetched() {
    if LIST.with(|l| l.borrow().is_some()) || LIST_FETCHING.with(Cell::get) {
        return;
    }
    // No pack-listing bridge (dedicated-server page, which ships no /static JS):
    // settle on an empty index rather than calling an absent async bridge, which
    // would `JsFuture::from(undefined).then` → uncaught crash. Default pack only.
    if !crate::wasm_save::bridge_present("axenstax_list_texture_packs") {
        LIST.with(|l| *l.borrow_mut() = Some(Vec::new()));
        return;
    }
    LIST_FETCHING.with(|f| f.set(true));
    wasm_bindgen_futures::spawn_local(async {
        let json = match js_list_texture_packs().await {
            Ok(v) => v.as_string().unwrap_or_default(),
            Err(_) => String::new(),
        };
        let packs = parse_pack_index(&json);
        LIST.with(|l| *l.borrow_mut() = Some(packs));
        LIST_FETCHING.with(|f| f.set(false));
    });
}

/// Pack names available in the picker (the index's `name`s), empty until fetched.
pub fn pack_names() -> Vec<String> {
    LIST.with(|l| {
        l.borrow()
            .as_ref()
            .map(|v| v.iter().map(|d| d.name.clone()).collect())
            .unwrap_or_default()
    })
}

/// User picked `name`: persist it, then fetch + decode the pack (or queue a revert
/// to the default). The result lands in `APPLY_SLOT` for the game loop to install.
pub fn select(name: &str) {
    persist_selection(name);
    if name.is_empty() || name == "Default" {
        APPLY_SLOT.with(|s| {
            *s.borrow_mut() = Some(Ok(Applied {
                name: "Default".to_string(),
                resolution: 16,
                textures: Vec::new(),
            }));
        });
        return;
    }
    let desc = LIST.with(|l| {
        l.borrow().as_ref().and_then(|v| v.iter().find(|d| d.name == name).cloned())
    });
    let Some(desc) = desc else {
        APPLY_SLOT.with(|s| *s.borrow_mut() = Some(Err(format!("pack '{name}' not in index"))));
        return;
    };
    let name_owned = name.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        let result = fetch_decode(&name_owned, &desc).await;
        APPLY_SLOT.with(|s| *s.borrow_mut() = Some(result));
    });
}

async fn fetch_decode(name: &str, desc: &WebPackDescriptor) -> Result<Applied, String> {
    let mut textures = Vec::new();
    for key in &desc.files {
        let val = match js_fetch_pack_file(name.to_string(), key.clone()).await {
            Ok(v) => v,
            Err(_) => {
                log::warn!("web pack {name}: {key}.png fetch failed — skipping");
                continue;
            }
        };
        let bytes = js_sys::Uint8Array::new(&val).to_vec();
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let img = img.to_rgba8();
                let (w, h) = img.dimensions();
                // An animated strip contributes its first frame as a static
                // override for now (web animation is a follow-up).
                let (rgba, width, height) = if w > 0 && h > w && h % w == 0 {
                    (crate::texture_anim::first_frame(&img.into_raw(), w), w, w)
                } else {
                    (img.into_raw(), w, h)
                };
                textures.push(DecodedTexture { key: key.clone(), rgba, width, height });
            }
            Err(e) => log::warn!("web pack {name}: {key}.png decode failed ({e}) — skipping"),
        }
    }
    if textures.is_empty() {
        return Err(format!("pack '{name}' fetched 0 usable textures"));
    }
    Ok(Applied { name: name.to_string(), resolution: desc.resolution, textures })
}

/// Drain a completed apply (a decoded pack, or a revert to default) — the game
/// loop installs it into the registry and rebuilds the atlas.
pub fn take_applied() -> Option<Result<Applied, String>> {
    APPLY_SLOT.with(|s| s.borrow_mut().take())
}

/// Per-frame pump (called from `update_and_render`): on the first call it queues
/// the saved pack for re-apply (so a web user's choice survives reload); once the
/// index is available it fires the deferred select. Cheap no-op afterwards.
pub fn pump() {
    if !INIT_DONE.with(|f| f.replace(true)) {
        let saved = saved_selection();
        if saved != "Default" {
            PENDING_SAVED.with(|p| *p.borrow_mut() = Some(saved));
            ensure_list_fetched();
        }
    }
    let pending = PENDING_SAVED.with(|p| p.borrow().clone());
    if let Some(name) = pending {
        if LIST.with(|l| l.borrow().is_some()) {
            PENDING_SAVED.with(|p| *p.borrow_mut() = None);
            select(&name);
        }
    }
}
