//! WASM entry point.
//!
//! `wasm_main` runs automatically when the WASM module loads — it installs
//! the panic hook + logger, then registers the `window.__axenstax_set_pubkey`
//! and `window.__axenstax_start` hooks for `auth.js` to call. No engine
//! initialisation happens until `__axenstax_start` fires.

use wasm_bindgen::prelude::*;
use winit::platform::web::EventLoopExtWebSys;

#[wasm_bindgen(start)]
pub fn wasm_main() {
    console_error_panic_hook::set_once();
    // Console logger: forwards every record to the browser console, routed by
    // level. Nothing is stored or sent anywhere.
    super::web_logger::init(log::Level::Info);
    log::info!("Axe'n'Stax WASM init");
    super::wasm_auth::register_boot_hooks();

    // Detect touch capability once at startup. Drives two things that can't reach
    // a `TouchInput` instance: the in-game control overlay shows on load (a
    // touch player expects to SEE the controls before the first tap), and the
    // menu's text fields swap egui text entry for an OS soft-keyboard prompt
    // (winit+egui won't summon the on-screen keyboard on a tablet/Chromebook).
    if let Some(win) = web_sys::window() {
        let touch_points = win.navigator().max_touch_points();
        super::touch_input::set_touch_device(touch_points > 0);
        log::info!("touch device: {} (maxTouchPoints={touch_points})", touch_points > 0);
    }
}

/// Spawn the winit event loop. Called by `auth.js` via `window.__axenstax_start`
/// once auth has resolved. A pubkey is OPTIONAL: the web tier is
/// anonymous-by-default, so `auth.js bootWasm(null)` boots a not-signed-in
/// visitor straight into GUEST mode (no pubkey written to `save::WASM_PUBKEY`) —
/// identical to native's default. The `/game` sign-in gate was dropped in
/// db76a90d ("anonymous-by-default tier"); a missing pubkey is therefore a normal
/// guest boot, NOT a contract violation. Every downstream `WASM_PUBKEY` read
/// tolerates `None` (unwrap_or_default / filter), so the engine runs correctly
/// with no identity. Treating a guest boot as fatal here was the regression that
/// left signed-out visitors stuck on a bouncing loading bar.
pub fn start_engine() {
    let pubkey_present = crate::save::WASM_PUBKEY.with(|p| p.borrow().is_some());
    if pubkey_present {
        log::info!("start_engine: booting signed-in");
    } else {
        log::info!("start_engine: booting as guest (anonymous web tier — no pubkey)");
    }

    let event_loop = winit::event_loop::EventLoop::new().expect("event loop");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let app = super::App::new();
    event_loop.spawn_app(app);
    log::info!("engine event loop spawned");
}
