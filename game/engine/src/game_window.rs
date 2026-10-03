//! Headless-capable window handle (headless GameState harness, 2026-07-11).
//!
//! `GameState` holds this instead of a raw `Arc<Window>` so the full game —
//! world, chunks, ticks, input dispatch, ECS — can be constructed and driven
//! with no display server: by the test harness (`test_game_harness.rs`) and,
//! later, by screenshot tooling. `Real` forwards to winit; `Headless` is a
//! fixed-size, always-focused stand-in whose UI side effects are no-ops.
//!
//! Call sites that genuinely need the winit window — egui `begin_frame`,
//! surface `render` — gate on [`GameWindow::winit_arc`] and are skipped
//! headless: the harness exercises the SIM (input → dispatch → world/ECS),
//! not the painted UI. Everything else (`inner_size`, cursor grab, titles,
//! focus) goes through the forwarding methods below, so the ~40 existing
//! call sites compile unchanged.

use std::sync::Arc;
use winit::window::Window;

pub(crate) enum GameWindow {
    Real(Arc<Window>),
    /// No display server: fixed logical size, always focused. Native-only in
    /// practice (the WASM path always has a canvas-backed Real window).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Headless { width: u32, height: u32 },
}

impl GameWindow {
    /// The real winit window, if any. Render/egui paths gate on this —
    /// headless runs the simulation (and all egui UI logic via a default
    /// `RawInput`) but paints nothing.
    pub(crate) fn winit_arc(&self) -> Option<&Arc<Window>> {
        match self {
            Self::Real(w) => Some(w),
            Self::Headless { .. } => None,
        }
    }

    /// Borrowed form of [`Self::winit_arc`] for callers wanting `&Window`.
    pub(crate) fn winit(&self) -> Option<&Window> {
        match self {
            Self::Real(w) => Some(w),
            Self::Headless { .. } => None,
        }
    }

    pub(crate) fn inner_size(&self) -> winit::dpi::PhysicalSize<u32> {
        match self {
            Self::Real(w) => w.inner_size(),
            Self::Headless { width, height } => winit::dpi::PhysicalSize::new(*width, *height),
        }
    }

    pub(crate) fn request_redraw(&self) {
        if let Self::Real(w) = self {
            w.request_redraw();
        }
    }

    pub(crate) fn set_title(&self, title: &str) {
        if let Self::Real(w) = self {
            w.set_title(title);
        }
    }

    pub(crate) fn set_cursor_visible(&self, visible: bool) {
        if let Self::Real(w) = self {
            w.set_cursor_visible(visible);
        }
    }

    pub(crate) fn set_cursor_grab(
        &self,
        mode: winit::window::CursorGrabMode,
    ) -> Result<(), winit::error::ExternalError> {
        match self {
            Self::Real(w) => w.set_cursor_grab(mode),
            Self::Headless { .. } => Ok(()),
        }
    }

    /// Headless is always "focused" so the cursor-grab bookkeeping and any
    /// focus-gated gameplay behave as in an active session.
    pub(crate) fn has_focus(&self) -> bool {
        match self {
            Self::Real(w) => w.has_focus(),
            Self::Headless { .. } => true,
        }
    }

    /// WASM: the backing canvas (winit `WindowExtWebSys`). The web path
    /// always has a Real window, but match anyway for shape-consistency.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn canvas(&self) -> Option<web_sys::HtmlCanvasElement> {
        use winit::platform::web::WindowExtWebSys;
        match self {
            Self::Real(w) => w.canvas(),
            Self::Headless { .. } => None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn fullscreen(&self) -> Option<winit::window::Fullscreen> {
        match self {
            Self::Real(w) => w.fullscreen(),
            Self::Headless { .. } => None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_fullscreen(&self, fullscreen: Option<winit::window::Fullscreen>) {
        if let Self::Real(w) = self {
            w.set_fullscreen(fullscreen);
        }
    }
}
