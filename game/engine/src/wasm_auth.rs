//! WASM auth — minimal Rust shim. The real auth flow lives in
//! `tools/sites/game/static/auth.js`; this module just exposes two hooks on
//! `window` so auth.js can hand off the verified pubkey and kick off the
//! engine once the user is signed in.

use wasm_bindgen::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use js_sys::Reflect;

use crate::save::WASM_PUBKEY;

/// Publish `window.__axenstax_set_pubkey(hex)` and `window.__axenstax_start()`.
///
/// Called once from `wasm_main` after the panic hook and logger are installed.
/// The returned `Closure`s are leaked via `forget()` — they live for the page
/// lifetime, which matches how `wasm-bindgen` intends single-use JS bindings.
pub fn register_boot_hooks() {
    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            log::error!("wasm_auth: no window object");
            return;
        }
    };

    let set_pubkey = Closure::wrap(Box::new(|hex: String| {
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            log::error!("set_pubkey: rejected invalid hex (len={})", hex.len());
            return;
        }
        let lower = hex.to_lowercase();
        WASM_PUBKEY.with(|p| *p.borrow_mut() = Some(lower.clone()));
        log::info!("auth: pubkey set {}…", &lower[..16]);
    }) as Box<dyn Fn(String)>);

    if let Err(e) = Reflect::set(
        &window,
        &JsValue::from_str("__axenstax_set_pubkey"),
        set_pubkey.as_ref().unchecked_ref(),
    ) {
        log::error!("wasm_auth: failed to publish set_pubkey: {e:?}");
    }
    set_pubkey.forget();

    let start = Closure::wrap(Box::new(|| {
        crate::web_main::start_engine();
    }) as Box<dyn Fn()>);

    if let Err(e) = Reflect::set(
        &window,
        &JsValue::from_str("__axenstax_start"),
        start.as_ref().unchecked_ref(),
    ) {
        log::error!("wasm_auth: failed to publish start: {e:?}");
    }
    start.forget();

    log::info!("wasm_auth: boot hooks registered");
}

// ─── Phase 4: web join signing bridge ────────────────────────────────────────
//
// Spike result (2026-06-16): the page already retains a sign-capable Signet
// signer (`window.__axenstax_get_signer().signEvent`, used by beacon/gamestr/
// cloud), so signing a kind-21236 join auth event in the browser needs NO
// upstream Signet change. `auth.js` exposes `__axenstax_sign_auth_event`; this
// module calls it and adapts the async JS Promise into the synchronous
// `RemoteClient` handshake via a channel. LIVE browser+phone verification is the
// owner boundary. A missing/failed signer degrades to a guest join
// (`has_js_signer` gate at the call site), which a dedicated server refuses
// unless its operator admits guests: sign-in is required by default since
// 2026-10-06 (`--allow-guests` opens it; Spec 04 §1.8). The web build is the
// anonymous offline taster and has no signer today (audit 2026-10-04), so in
// practice this path is a guest join.

/// Whether a sign-capable signer is currently retained on the page. The WASM
/// JoinGame path uses this to choose an authenticated vs guest join — false on
/// the dedicated-server guest-boot page (no signet-login), so that path stays a
/// guest join.
pub fn has_js_signer() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let Ok(getter) = Reflect::get(&window, &JsValue::from_str("__axenstax_get_signer")) else {
        return false;
    };
    let Ok(getter) = getter.dyn_into::<js_sys::Function>() else { return false };
    let Ok(signer) = getter.call0(&JsValue::NULL) else { return false };
    if signer.is_null() || signer.is_undefined() {
        return false;
    }
    Reflect::get(&signer, &JsValue::from_str("signEvent"))
        .map(|v| v.is_function())
        .unwrap_or(false)
}

/// Build a [`crate::remote_client::SignDriverFn`] that signs the join auth event
/// in the browser. Given the server's nonce and the client-built origin
/// (`signet::client_join_origin`: `axenstax-join:ws-host:<the host the page's
/// WebSocket dialled>`, v66) it calls the JS hook,
/// awaits the Promise on the microtask queue (`spawn_local`), parses the signed
/// event, and delivers it over the channel `RemoteClient::poll` drains.
pub fn js_sign_driver() -> crate::remote_client::SignDriverFn {
    Box::new(move |nonce_hex: String, origin: String| {
        let (tx, rx) = std::sync::mpsc::channel();
        wasm_bindgen_futures::spawn_local(async move {
            let result = sign_join_via_js(&nonce_hex, &origin).await.map(|wire| {
                crate::remote_client::SignedJoin { auth_event: wire, credential: None }
            });
            let _ = tx.send(result);
        });
        rx
    })
}

/// Invoke `window.__axenstax_sign_auth_event(challenge, origin)` and parse the
/// returned signed Nostr event into the engine's wire DTO.
async fn sign_join_via_js(
    challenge_hex: &str,
    origin: &str,
) -> Result<crate::signet::SignetAuthEventWire, String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let f = Reflect::get(&window, &JsValue::from_str("__axenstax_sign_auth_event"))
        .map_err(|_| "sign hook lookup failed".to_string())?;
    let f: js_sys::Function = f
        .dyn_into()
        .map_err(|_| "sign hook missing (sign in first)".to_string())?;
    let promise = f
        .call2(
            &JsValue::NULL,
            &JsValue::from_str(challenge_hex),
            &JsValue::from_str(origin),
        )
        .map_err(|e| format!("sign hook threw: {e:?}"))?;
    let promise: js_sys::Promise = promise
        .dyn_into()
        .map_err(|_| "sign hook did not return a Promise".to_string())?;
    let event_js = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("signing rejected: {e:?}"))?;
    let json = js_sys::JSON::stringify(&event_js)
        .ok()
        .and_then(|s| s.as_string())
        .ok_or_else(|| "signed event not serialisable".to_string())?;
    parse_signed_event_json(&json)
}

/// Parse a JSON Nostr event (hex pubkey/id/sig) into `SignetAuthEventWire`.
fn parse_signed_event_json(
    json: &str,
) -> Result<crate::signet::SignetAuthEventWire, String> {
    #[derive(serde::Deserialize)]
    struct JsEvent {
        pubkey: String,
        created_at: u64,
        kind: u32,
        tags: Vec<Vec<String>>,
        content: String,
        id: String,
        sig: String,
    }
    let e: JsEvent = serde_json::from_str(json).map_err(|err| format!("event parse: {err}"))?;
    let pubkey: [u8; 32] = hex::decode(&e.pubkey)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or_else(|| "bad pubkey hex".to_string())?;
    let id: [u8; 32] = hex::decode(&e.id)
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .ok_or_else(|| "bad id hex".to_string())?;
    let sig = hex::decode(&e.sig).map_err(|_| "bad sig hex".to_string())?;
    Ok(crate::signet::SignetAuthEventWire {
        pubkey,
        created_at: e.created_at as u32,
        kind: e.kind,
        tags: e.tags,
        content: e.content,
        id,
        sig,
        from_np: false,
    })
}

/// Call `window.axenstax_exit_to_lobby()` — JS navigates back to the entrance
/// WITHOUT clearing the session cookie / cached pubkey, so the still-valid
/// Signet session is reused on return (no QR re-login). #5.
pub fn exit_to_lobby() {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let Ok(fn_val) = Reflect::get(&window, &JsValue::from_str("axenstax_exit_to_lobby")) else {
        log::warn!("exit_to_lobby: window.axenstax_exit_to_lobby missing");
        return;
    };
    let Ok(f) = fn_val.dyn_into::<js_sys::Function>() else {
        log::warn!("exit_to_lobby: axenstax_exit_to_lobby is not a function");
        return;
    };
    let _ = f.call0(&JsValue::NULL);
}
