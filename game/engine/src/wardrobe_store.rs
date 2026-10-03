//! Wardrobe persistence seam (Spec 40) — save/load the player's GLOBAL wardrobe
//! (`OverrideSet`) behind one small interface, platform-gated only at the I/O edge.
//!
//! WASM/PWA → the PRIVATE Stash (NIP-44 encrypted-to-self), a player-scoped
//! cross-world blob (`kind:"wardrobe"`, mirroring the cosmetic skin), via the
//! `window.AxeCloud`/Stash bridge in `cloud.js` (externs in `wasm_save.rs`) —
//! wired in a later task.
//! Native → a local profile file under `profile/wardrobe.blob`.
//!
//! The wire format is the existing version-prefixed `OverrideSet` blob
//! (`to_blob_bytes` / `from_blob_bytes`, v1→v2 migration-safe) — NOT a new format.

use crate::override_registry::OverrideSet;

/// Serialise the player wardrobe to the persisted blob (version-prefixed bincode).
pub fn pack(set: &OverrideSet) -> Result<Vec<u8>, String> {
    set.to_blob_bytes()
}

/// Parse a persisted wardrobe blob (version-checked + migrated), never panics.
pub fn unpack(bytes: &[u8]) -> Result<OverrideSet, String> {
    OverrideSet::from_blob_bytes(bytes)
}

// ── Native profile file ─────────────────────────────────────────────────────
#[cfg(not(target_arch = "wasm32"))]
mod native_io {
    use super::*;
    use std::path::{Path, PathBuf};

    /// The single-user native profile path for the wardrobe blob. Native has no
    /// per-player identity, so one file suffices, kept OUT of `worlds/` so it never
    /// shows up as a world card.
    pub fn profile_path() -> PathBuf {
        crate::data_dir::profile_dir().join("wardrobe.blob")
    }
    pub fn remember_path() -> PathBuf {
        crate::data_dir::profile_dir().join("wardrobe_remember")
    }

    pub fn save_to_path(path: &Path, set: &OverrideSet) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir profile: {e}"))?;
        }
        crate::save::check_blob_writable(path)?;
        let bytes = pack(set)?;
        crate::save::write_atomic(path, &bytes).map_err(|e| format!("write wardrobe: {e}"))
    }
    /// A failed load (audit 2026-09-27) renames the damaged blob aside to
    /// `<path>.corrupt-<ts>` and blocks every `save_to_path` for this path for
    /// the rest of the session, so an empty wardrobe never replaces it.
    pub fn load_from_path(path: &Path) -> Result<Option<OverrideSet>, String> {
        match std::fs::read(path) {
            Ok(bytes) => unpack(&bytes).map(Some).map_err(|e| {
                crate::save::mark_blob_load_failed(path);
                let _ = crate::save::quarantine_corrupt(path);
                log::error!("wardrobe {} is damaged ({e}); saving is off this session", path.display());
                e
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => {
                crate::save::mark_blob_load_failed(path);
                Err(format!("read wardrobe: {e}"))
            }
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub use native_io::{load_from_path, profile_path, remember_path, save_to_path};

/// Persist the player wardrobe (best-effort). Native = profile file.
/// (The WASM body — Stash — is added in a later task.)
#[cfg(not(target_arch = "wasm32"))]
pub fn save(set: &OverrideSet) {
    if let Err(e) = save_to_path(&profile_path(), set) {
        log::warn!("wardrobe save failed: {e}");
    }
}

/// Load the player wardrobe synchronously (native only). `None` when none stored.
#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Option<OverrideSet> {
    match load_from_path(&profile_path()) {
        Ok(opt) => opt,
        Err(e) => {
            log::warn!("wardrobe load failed: {e}");
            None
        }
    }
}

/// Read the "remember on entry" preference (default true).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_remember() -> bool {
    std::fs::read_to_string(remember_path())
        .map(|s| s.trim() != "0")
        .unwrap_or(true)
}

/// Persist the "remember on entry" preference (best-effort).
#[cfg(not(target_arch = "wasm32"))]
pub fn save_remember(on: bool) {
    let path = remember_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, if on { "1" } else { "0" });
}

/// Persist the player wardrobe to the private Stash (best-effort, fire-and-forget).
#[cfg(target_arch = "wasm32")]
pub fn save(set: &OverrideSet) {
    match pack(set) {
        Ok(bytes) => wasm_bindgen_futures::spawn_local(async move {
            crate::wasm_save::wardrobe_save_wasm(bytes).await;
        }),
        Err(e) => log::warn!("wardrobe pack failed: {e}"),
    }
}
/// Read the "remember on entry" preference (default true) — localStorage on WASM.
#[cfg(target_arch = "wasm32")]
pub fn load_remember() -> bool { crate::wasm_save::wardrobe_remember_get_wasm() }
/// Persist the "remember on entry" preference — localStorage on WASM.
#[cfg(target_arch = "wasm32")]
pub fn save_remember(on: bool) { crate::wasm_save::wardrobe_remember_set_wasm(on); }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::override_registry::{AuthoredFaces, DesignLibrary, NamedDesign, OverrideSet};

    fn sample_set() -> OverrideSet {
        let mut set = OverrideSet::default();
        let mut lib = DesignLibrary::default();
        lib.push_design(NamedDesign {
            id: 0, name: "Design 1".into(),
            faces: Some(AuthoredFaces::solid([3, 6, 9, 255])),
            micro_model: None, author_npub: String::new(), derivation_chain: vec![],
        });
        set.block_designs.push((crate::block::STONE, lib));
        set
    }

    #[test]
    fn pack_unpack_round_trips() {
        let set = sample_set();
        let bytes = pack(&set).unwrap();
        let back = unpack(&bytes).unwrap();
        assert_eq!(back.block_designs.len(), 1);
        assert_eq!(back.block_designs[0].0, crate::block::STONE);
        assert_eq!(back.block_designs[0].1.designs.len(), 1);
    }

    #[test]
    fn unpack_rejects_garbage() {
        assert!(unpack(&[]).is_err());
        assert!(unpack(&[0xff, 1, 2, 3]).is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_profile_round_trips_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("axe_wardrobe_test_{}", std::process::id()));
        let path = dir.join("wardrobe.blob");
        let set = sample_set();
        save_to_path(&path, &set).unwrap();
        let back = load_from_path(&path).unwrap().unwrap();
        assert_eq!(back.block_designs.len(), 1);
        // Missing file ⇒ Ok(None), not an error.
        let _ = std::fs::remove_file(&path);
        assert!(load_from_path(&path).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn pin_save_load_reproduces_wardrobe_native() {
        // The headline acceptance (native-scriptable): author a design into a
        // wardrobe, persist it, then load it into a fresh registry — the design AND
        // the active selection must survive. (The WASM/visual round-trip is the
        // owner's live playtest.)
        let base = crate::texture_gen::texture_count();
        let mut authored = crate::override_registry::OverrideRegistry::new();
        let out = authored.add_block_design(
            crate::block::STONE,
            NamedDesign {
                id: 0,
                name: "Design 1".into(),
                faces: Some(AuthoredFaces::solid([8, 8, 8, 255])),
                micro_model: None,
                author_npub: String::new(),
                derivation_chain: vec![],
            },
            base,
        );
        let dir = std::env::temp_dir().join(format!("axe_wardrobe_e2e_{}", std::process::id()));
        let path = dir.join("wardrobe.blob");
        save_to_path(&path, authored.set()).unwrap();

        let loaded = load_from_path(&path).unwrap().unwrap();
        let reloaded = crate::override_registry::OverrideRegistry::from_set(loaded, base);
        let lib = &reloaded
            .set()
            .block_designs
            .iter()
            .find(|(b, _)| *b == crate::block::STONE)
            .unwrap()
            .1;
        assert_eq!(lib.active, Some(out.active_id), "active selection survives reload");
        assert!(
            reloaded.block_face_layer(crate::block::STONE, 0).is_some(),
            "design renders after reload"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_load_quarantines_the_blob_and_blocks_every_save() {
        let dir = std::env::temp_dir()
            .join(format!("axe_wardrobe_store_corrupt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wardrobe.blob");
        let torn = b"\x02torn";
        std::fs::write(&path, torn).unwrap();

        assert!(load_from_path(&path).is_err());
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("wardrobe.blob.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read(aside[0].path()).unwrap(), torn);
        assert!(save_to_path(&path, &OverrideSet::default()).is_err());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
