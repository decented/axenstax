//! Browser-console logger for the web build.
//!
//! Installs a global `log::Log` that forwards every record to the browser
//! console, routed by level to the matching `console.*` method. Nothing is
//! stored, buffered or sent anywhere — records leave the engine only as console
//! output on the player's own device.
//!
//! (This module used to also keep a ring of recent warn/error lines for the web
//! feedback context. The browser build has no feedback channel any more —
//! removed 2026-10-01 — so the ring went with it.)
//!
//! Native builds keep using `env_logger`; this is cfg'd to wasm32.

#![cfg(target_arch = "wasm32")]

use std::sync::LazyLock;

struct ConsoleLogger {
    level: log::LevelFilter,
}

impl log::Log for ConsoleLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let formatted = format!("[{}] {}", record.target(), record.args());
        let js = wasm_bindgen::JsValue::from_str(&formatted);
        match record.level() {
            log::Level::Error => web_sys::console::error_1(&js),
            log::Level::Warn => web_sys::console::warn_1(&js),
            log::Level::Info => web_sys::console::info_1(&js),
            log::Level::Debug => web_sys::console::log_1(&js),
            log::Level::Trace => web_sys::console::debug_1(&js),
        }
    }

    fn flush(&self) {}
}

/// Install the console logger. Safe to call exactly once; idempotent if a
/// logger is already set (second call is a silent no-op).
pub fn init(level: log::Level) {
    static LOGGER: LazyLock<ConsoleLogger> =
        LazyLock::new(|| ConsoleLogger { level: log::LevelFilter::Info });
    let _ = log::set_logger(&*LOGGER);
    log::set_max_level(level.to_level_filter());
}
