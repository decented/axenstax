//! npub (NIP-19) <-> hex at the UI boundary. Internally AxeNStax keys on hex
//! everywhere; npub is presentation + input only (feedback_npub_only_display).
//! Rust has no bech32 — encode/decode live in JS (bech32.js + npub-decode.js);
//! this module is the cross-platform seam (wasm → JS; native → hex fallback).

#[cfg(target_arch = "wasm32")]
mod wasm {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = axenstax_npub_to_hex)]
        pub fn npub_to_hex(npub: &str) -> JsValue;
        #[wasm_bindgen(js_name = axenstax_npub_encode)]
        pub fn npub_encode(hex: &str) -> JsValue;
    }
}

/// Decode an npub (bech32) to lowercase 64-hex. `None` if not a valid npub.
/// Native: always `None` (npub input is a PWA path; native callers pass hex).
pub fn npub_to_hex(npub: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm::npub_to_hex(npub).as_string().filter(|s| s.len() == 64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = npub;
        None
    }
}

/// Render a 64-hex pubkey as an npub for display. Falls back to the hex string
/// if encoding is unavailable (native, or malformed hex) — never panics.
#[allow(dead_code)] // display-boundary helper; wired by the Task 9 browse printer
pub fn hex_to_npub(hex: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        wasm::npub_encode(hex).as_string().unwrap_or_else(|| hex.to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        hex.to_string()
    }
}
