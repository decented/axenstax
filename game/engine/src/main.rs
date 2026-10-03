//! Thin binary shim.
//!
//! Everything that used to live here now lives in `lib.rs`, because Android
//! needs a **cdylib** — `NativeActivity` `dlopen()`s `libaxenstax_engine.so` and
//! calls `android_main`, which a `bin` target cannot provide. A bin-only crate
//! simply cannot be packaged into an APK.
//!
//! The desktop entry point is unchanged in behaviour: `main()` just forwards to
//! [`axenstax_engine::run`], which carries all the `--server` / `--admin-*` /
//! `--screenshot` argument dispatch.
//!
//! The `[lib] crate-type` in Cargo.toml is `["rlib", "cdylib"]` — the rlib is
//! what this binary links against, the cdylib is what the APK (and the web
//! bundle, via Trunk's `data-target-name`) ships.

fn main() {
    // Native desktop + dedicated server: the real entry point.
    #[cfg(not(target_arch = "wasm32"))]
    axenstax_engine::run();

    // WASM: the browser entry point is `web_main::wasm_main`, invoked by
    // `#[wasm_bindgen(start)]` inside the library — this `main` exists only to
    // satisfy the compiler, exactly as the old one did.
}
