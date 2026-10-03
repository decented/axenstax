//! Native OS file-picker seam — world Export/Import + custom-skin upload.
//!
//! NATIVE-ONLY. `rfd::FileDialog` (xdg-portal backend on Linux) BLOCKS the
//! calling thread while the OS dialog is shown. The egui render loop must never
//! block, so every dialog runs on a worker thread and reports its result back
//! through an `mpsc` channel — exactly the same off-thread pattern used by
//! `native_signin.rs` for the NIP-46 handshake.
//!
//! Callers:
//! 1. Call [`spawn_dialog`] with the desired [`FileDialogRequest`].
//! 2. Store the returned [`Receiver`] somewhere (e.g. `MenuState::pending_dialog`).
//! 3. Call `try_recv()` each frame (or use the [`MenuState::poll_dialog`] helper)
//!    to drain [`FileDialogResult`]s without blocking.
//!
//! All file I/O (read + write) happens on the worker thread — never on the
//! caller's thread.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

/// What kind of OS dialog to open.
// Android's BRIDGE stub (below) reads no payloads and builds no success results.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub enum FileDialogRequest {
    /// Show a "Save As" dialog; if the user confirms, write `bytes` to the
    /// chosen path on the worker thread.
    SaveWorld { default_name: String, bytes: Vec<u8> },
    /// Show an "Open" dialog filtered to `*.axeworld`; read the chosen file's
    /// bytes on the worker thread.
    OpenWorld,
    /// Show an "Open" dialog filtered to `*.png`; read the chosen file's bytes
    /// on the worker thread.
    OpenSkin,
    /// Show a "Save As" dialog filtered to `*.png`; on confirm, write `bytes`
    /// (a 64×64 Minecraft skin PNG) to the chosen path on the worker thread.
    SaveSkin { default_name: String, bytes: Vec<u8> },
    /// Campaign G — show an "Open" dialog filtered to `*.axeghost`; read the
    /// chosen ghost file's bytes on the worker thread.
    OpenGhost,
    /// Campaign G — show a "Save As" dialog filtered to `*.axeghost`; on
    /// confirm, write `bytes` (a ghost share file) on the worker thread.
    SaveGhost { default_name: String, bytes: Vec<u8> },
    /// "Take your worlds to native" — show an "Open" dialog filtered to
    /// `*.axeprofile`; read the chosen whole-profile bundle's bytes on the
    /// worker thread.
    OpenProfile,
}

/// Result reported by the worker thread once the dialog closes and any I/O
/// finishes.
///
/// Named `FileDialogResult` (not `DialogResult`) because `menu.rs` already has
/// a private `DialogResult` enum for its create/rename forms.
#[derive(Debug)]
#[cfg_attr(target_os = "android", allow(dead_code))]
pub enum FileDialogResult {
    /// World bytes were written successfully. Carries the path that was chosen.
    WorldSaved(PathBuf),
    /// The user picked an `*.axeworld` file and the worker read its bytes.
    WorldToImport(Vec<u8>),
    /// The user picked a `*.png` file and the worker read its bytes.
    SkinPng(Vec<u8>),
    /// A skin PNG was written successfully. Carries the chosen path.
    SkinSaved(PathBuf),
    /// The user picked a `*.axeghost` file and the worker read its bytes.
    GhostJson(Vec<u8>),
    /// A ghost share file was written successfully. Carries the chosen path.
    GhostSaved(PathBuf),
    /// The user picked an `*.axeprofile` file and the worker read its bytes.
    ProfileToImport(Vec<u8>),
    /// The user dismissed the dialog without picking anything.
    Cancelled,
    /// Something went wrong (write error, read error, …). Carries a short
    /// human-readable message.
    Err(String),
}

/// Spawn a worker thread that opens the requested OS dialog, performs any
/// file read/write on that thread, and sends exactly one [`FileDialogResult`]
/// back through the returned [`Receiver`].
///
/// The Receiver yields `Ok(result)` once the dialog closes (or `Err` on
/// channel disconnect — treat that the same as no-op and clear the slot).
pub fn spawn_dialog(req: FileDialogRequest) -> Receiver<FileDialogResult> {
    let (tx, rx) = channel::<FileDialogResult>();
    std::thread::spawn(move || {
        let result = run_dialog(req);
        // Ignore send errors — the menu may have been closed; the result is
        // intentionally discarded in that case.
        let _ = tx.send(result);
    });
    rx
}

// ── Worker ───────────────────────────────────────────────────────────────────

/// BRIDGE: Android file dialogs — replace when the Android port reaches world
/// Export/Import, skin upload, ghost files and profile import.
///
/// `rfd` has no Android backend at all (0.17 ships xdg-portal/win/mac only), so
/// it is cut out of the Android dependency graph in Cargo.toml and this stub
/// stands in. The real implementation is a Storage Access Framework round-trip
/// — `ACTION_CREATE_DOCUMENT` / `ACTION_OPEN_DOCUMENT` fired through the
/// Activity, with the chosen `content://` URI coming back via
/// `onActivityResult` — which needs a JNI hop that does not exist yet.
///
/// Deliberately reports `Err` rather than `Cancelled`: callers treat `Cancelled`
/// as "the user dismissed it" and show nothing, which would make the button look
/// silently broken. A kid-readable line is the honest surface.
#[cfg(target_os = "android")]
fn run_dialog(req: FileDialogRequest) -> FileDialogResult {
    let what = match req {
        FileDialogRequest::SaveWorld { .. } | FileDialogRequest::OpenWorld => "Worlds",
        FileDialogRequest::OpenSkin | FileDialogRequest::SaveSkin { .. } => "Skins",
        FileDialogRequest::OpenGhost | FileDialogRequest::SaveGhost { .. } => "Ghost files",
        FileDialogRequest::OpenProfile => "Profiles",
    };
    log::warn!("file dialog requested on Android, but there is no SAF bridge yet");
    FileDialogResult::Err(format!("{what} can't be imported or exported on this device yet."))
}

#[cfg(not(target_os = "android"))]
fn run_dialog(req: FileDialogRequest) -> FileDialogResult {
    match req {
        FileDialogRequest::SaveWorld { default_name, bytes } => {
            let path = rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter("Axe'n'Stax world", &["axeworld"])
                .save_file();
            match path {
                None => FileDialogResult::Cancelled,
                Some(p) => match std::fs::write(&p, &bytes) {
                    Ok(()) => FileDialogResult::WorldSaved(p),
                    Err(e) => FileDialogResult::Err(format!("Could not write file: {e}")),
                },
            }
        }
        FileDialogRequest::OpenWorld => {
            let path = rfd::FileDialog::new()
                .add_filter("Axe'n'Stax world", &["axeworld"])
                .pick_file();
            match path {
                None => FileDialogResult::Cancelled,
                Some(p) => match std::fs::read(&p) {
                    Ok(bytes) => FileDialogResult::WorldToImport(bytes),
                    Err(e) => FileDialogResult::Err(format!("Could not read file: {e}")),
                },
            }
        }
        FileDialogRequest::OpenSkin => {
            let path = rfd::FileDialog::new()
                .add_filter("PNG skin", &["png"])
                .pick_file();
            match path {
                None => FileDialogResult::Cancelled,
                // Kid-readable surface (rendered verbatim in SkinStatus::Error):
                // never leak the raw std::io::Error — log it, show a friendly line.
                Some(p) => match std::fs::read(&p) {
                    Ok(bytes) => FileDialogResult::SkinPng(bytes),
                    Err(e) => {
                        log::warn!("skin upload read failed for {p:?}: {e}");
                        FileDialogResult::Err(
                            "Couldn't open that skin — try a different file.".to_string(),
                        )
                    }
                },
            }
        }
        FileDialogRequest::OpenGhost => {
            let path = rfd::FileDialog::new()
                .add_filter("Axe'n'Stax ghost", &[crate::trials::GHOST_FILE_EXT])
                .pick_file();
            match path {
                None => FileDialogResult::Cancelled,
                Some(p) => match std::fs::read(&p) {
                    Ok(bytes) => FileDialogResult::GhostJson(bytes),
                    Err(e) => {
                        log::warn!("ghost import read failed for {p:?}: {e}");
                        FileDialogResult::Err(
                            "Couldn't open that ghost file — try a different file.".to_string(),
                        )
                    }
                },
            }
        }
        FileDialogRequest::SaveGhost { default_name, bytes } => {
            let path = rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter("Axe'n'Stax ghost", &[crate::trials::GHOST_FILE_EXT])
                .save_file();
            match path {
                None => FileDialogResult::Cancelled,
                Some(p) => match std::fs::write(&p, &bytes) {
                    Ok(()) => FileDialogResult::GhostSaved(p),
                    Err(e) => {
                        log::warn!("ghost export write failed for {p:?}: {e}");
                        FileDialogResult::Err(
                            "Couldn't save the ghost — try a different folder.".to_string(),
                        )
                    }
                },
            }
        }
        FileDialogRequest::OpenProfile => {
            let path = rfd::FileDialog::new()
                .add_filter(
                    "Axe'n'Stax profile",
                    &[crate::profile_bundle::PROFILE_FILE_EXT],
                )
                .pick_file();
            match path {
                None => FileDialogResult::Cancelled,
                Some(p) => match std::fs::read(&p) {
                    Ok(bytes) => FileDialogResult::ProfileToImport(bytes),
                    Err(e) => {
                        // Kid-readable surface — log the raw error, show a line
                        // that says what to do next.
                        log::warn!("profile import read failed for {p:?}: {e}");
                        FileDialogResult::Err(
                            "Couldn't open that profile file — try a different file.".to_string(),
                        )
                    }
                },
            }
        }
        FileDialogRequest::SaveSkin { default_name, bytes } => {
            let path = rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter("PNG skin", &["png"])
                .save_file();
            match path {
                None => FileDialogResult::Cancelled,
                // Kid-readable surface (rendered verbatim in SkinStatus::Error):
                // never leak the raw std::io::Error — log it, show a friendly line.
                Some(p) => match std::fs::write(&p, &bytes) {
                    Ok(()) => FileDialogResult::SkinSaved(p),
                    Err(e) => {
                        log::warn!("skin save write failed for {p:?}: {e}");
                        FileDialogResult::Err(
                            "Couldn't save your skin — try a different folder.".to_string(),
                        )
                    }
                },
            }
        }
    }
}
