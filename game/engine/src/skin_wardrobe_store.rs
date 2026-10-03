//! Persistence for the avatar-skin [`SkinWardrobe`]. Mirrors the *shape* of
//! `wardrobe_store.rs` (the Workshop block/mob store) but for skins: a
//! version-prefixed bincode blob; native writes `profile/skins.blob`; web uses
//! localStorage + Stash (wired in Phase 1c). Each custom skin is stored as PNG
//! (compact, lossless) and decoded back to 16384-byte RGBA on load.

use serde::{Deserialize, Serialize};

use crate::cosmetics::{decode_skin_64, SkinSource};
use crate::skin_uv::ArmModel;
use crate::skin_wardrobe::{SkinEntry, SkinId, SkinWardrobe};

/// Blob format version (independent of `CosmeticDescriptor::version`).
///
/// - v1 → v2 (v0.2.19): loading a v1 blob triggers the one-shot box-unwrap-fix
///   migration below; v2+ is a plain load, never re-migrated.
/// - v2 → v3 (2026-09-06, slim arms): entries carry an `arm_model`. bincode is
///   NOT self-describing, so the wire shape genuinely changed — v1/v2 blobs are
///   decoded with [`StoredEntry`] (no arm field) and every entry comes back
///   Classic; v3 blobs use [`StoredEntryV3`]. Saving always writes v3.
const BLOB_VERSION: u8 = 3;

/// Commit time (UTC, SECONDS since epoch) of 712ec12c ("avatar faces follow
/// the Minecraft box unwrap", v0.2.18) — the release that fixed the
/// box-unwrap direction. `modified` is stamped by `game_loop::wardrobe_now()`,
/// which returns seconds-since-epoch on both native (`SystemTime`) and web
/// (`js_sys::Date::now() / 1000.0`), so this constant is in the same unit.
///
/// A hand-painted (non-imported) entry with `modified` strictly before this
/// moment was painted under the OLD (backwards) box-unwrap and is mirrored
/// once on load of a v1 blob (see `from_blob_bytes`); `modified == 0`
/// (entries that predate timestamps entirely, e.g. `migrate_legacy_png`) also
/// counts as OLD. Anything at or after the cutoff was already painted under
/// the fixed renderer and is left alone.
const UNWRAP_FIX_CUTOFF: u64 = 1_785_436_892;

/// The v1/v2 on-disk entry — READ ONLY now. Still `Serialize` so the tests can
/// mint genuine legacy blobs rather than hand-rolling bincode.
#[derive(Serialize, Deserialize)]
struct StoredEntry {
    id: SkinId,
    name: String,
    /// True ⇒ bundled default skin; `png` is empty. False ⇒ `png` holds a
    /// lossless 64×64 PNG of the custom pixels.
    is_default: bool,
    png: Vec<u8>,
    mc_handle: Option<String>,
    mc_uuid: Option<String>,
    created: u64,
    modified: u64,
}

/// The v3 entry: v2 plus the per-entry arm model.
#[derive(Serialize, Deserialize)]
struct StoredEntryV3 {
    id: SkinId,
    name: String,
    is_default: bool,
    png: Vec<u8>,
    mc_handle: Option<String>,
    mc_uuid: Option<String>,
    arm_model: ArmModel,
    created: u64,
    modified: u64,
}

#[derive(Serialize, Deserialize)]
struct StoredWardrobe {
    version: u8,
    entries: Vec<StoredEntry>,
    equipped: SkinId,
    next_id: SkinId,
}

#[derive(Serialize, Deserialize)]
struct StoredWardrobeV3 {
    version: u8,
    entries: Vec<StoredEntryV3>,
    equipped: SkinId,
    next_id: SkinId,
}

/// Encode 64×64 RGBA (16384 bytes) to a lossless PNG.
fn rgba_to_png(rgba: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(64, 64, rgba.to_vec())
        .ok_or_else(|| "skin must be 64×64 RGBA (16384 bytes)".to_string())?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("encode skin png: {e}"))?;
    Ok(png)
}

/// Serialize a wardrobe to a version-prefixed bincode blob.
pub fn to_blob_bytes(w: &SkinWardrobe) -> Result<Vec<u8>, String> {
    let (entries, equipped, next_id) = w.parts();
    let mut stored = Vec::with_capacity(entries.len());
    for e in entries {
        let (is_default, png) = match &e.source {
            SkinSource::Default => (true, Vec::new()),
            SkinSource::Rgba64(px) => (false, rgba_to_png(px)?),
        };
        stored.push(StoredEntryV3 {
            id: e.id,
            name: e.name.clone(),
            is_default,
            png,
            mc_handle: e.minecraft_handle.clone(),
            mc_uuid: e.minecraft_uuid.clone(),
            arm_model: e.arm_model,
            created: e.created,
            modified: e.modified,
        });
    }
    let blob = StoredWardrobeV3 { version: BLOB_VERSION, entries: stored, equipped, next_id };
    bincode::serialize(&blob).map_err(|e| format!("serialize skin wardrobe: {e}"))
}

/// Deserialize a wardrobe from a bincode blob. Errors on a corrupt blob; the
/// caller degrades to a fresh wardrobe (fail-soft).
///
/// One-shot migration: a blob stored at `version == 1` predates the
/// box-unwrap fix (v0.2.18), so every hand-painted entry (not the default
/// entry, not one imported from Minecraft) with `modified < UNWRAP_FIX_CUTOFF`
/// gets its pixels mirrored back to the Minecraft-standard layout via
/// `skin_uv::mirror_every_face`. A blob at `version == 2` (or later) is a
/// plain load — already migrated, never re-mirrored.
pub fn from_blob_bytes(bytes: &[u8]) -> Result<SkinWardrobe, String> {
    // The version byte decides the SHAPE, not just the migration flags:
    // v3 grew a field and bincode can't discover that for itself.
    if peek_blob_version(bytes) >= Some(3) {
        let blob: StoredWardrobeV3 =
            bincode::deserialize(bytes).map_err(|e| format!("deserialize skin wardrobe: {e}"))?;
        let mut entries = Vec::with_capacity(blob.entries.len());
        for s in blob.entries {
            let source = if s.is_default {
                SkinSource::Default
            } else {
                SkinSource::Rgba64(decode_skin_64(&s.png)?)
            };
            entries.push(SkinEntry {
                id: s.id,
                name: s.name,
                source,
                minecraft_handle: s.mc_handle,
                minecraft_uuid: s.mc_uuid,
                arm_model: s.arm_model,
                created: s.created,
                modified: s.modified,
            });
        }
        return Ok(SkinWardrobe::from_parts(entries, blob.equipped, blob.next_id));
    }

    let blob: StoredWardrobe =
        bincode::deserialize(bytes).map_err(|e| format!("deserialize skin wardrobe: {e}"))?;
    let needs_migration_check = blob.version == 1;
    let mut migrated = 0u32;
    let mut entries = Vec::with_capacity(blob.entries.len());
    for s in blob.entries {
        let source = if s.is_default {
            SkinSource::Default
        } else {
            let mut rgba = decode_skin_64(&s.png)?;
            let hand_painted = s.mc_handle.is_none() && s.mc_uuid.is_none();
            if needs_migration_check && hand_painted && s.modified < UNWRAP_FIX_CUTOFF {
                crate::skin_uv::mirror_every_face(&mut rgba);
                migrated += 1;
            }
            SkinSource::Rgba64(rgba)
        };
        entries.push(SkinEntry {
            id: s.id,
            name: s.name,
            source,
            minecraft_handle: s.mc_handle,
            minecraft_uuid: s.mc_uuid,
            // Slim didn't exist before v3, so every pre-v3 entry is Classic.
            arm_model: ArmModel::Classic,
            created: s.created,
            modified: s.modified,
        });
    }
    if migrated > 0 {
        log::info!(
            "skin wardrobe migration: mirrored {migrated} hand-painted skin(s) to the Minecraft-standard box unwrap (blob v1 → v2)"
        );
    }
    Ok(SkinWardrobe::from_parts(entries, blob.equipped, blob.next_id))
}

/// One-time migration of the previous single-skin cosmetic (a stored PNG) into
/// a wardrobe with exactly one equipped entry (no default entry alongside it).
/// Unreadable input → a fresh default wardrobe (never an error). Called from
/// `game_loop.rs`'s legacy-PNG wardrobe-load arm, which is `#[cfg(target_arch
/// = "wasm32")]`-only — invisible to a plain native `cargo clippy` run.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn migrate_legacy_png(png: &[u8]) -> SkinWardrobe {
    match decode_skin_64(png) {
        Ok(rgba) => {
            let entry = SkinEntry {
                id: 1,
                name: "My Skin".into(),
                source: SkinSource::Rgba64(rgba),
                minecraft_handle: None,
                minecraft_uuid: None,
                arm_model: ArmModel::Classic,
                created: 0,
                modified: 0,
            };
            SkinWardrobe::from_parts(vec![entry], 1, 2)
        }
        Err(_) => SkinWardrobe::new(),
    }
}

/// Native on-disk location for the per-identity skin wardrobe. Mirrors
/// `wardrobe_store::profile_path` (the Workshop store) — sibling file under
/// `profile/`.
#[cfg(not(target_arch = "wasm32"))]
pub fn profile_path() -> std::path::PathBuf {
    crate::data_dir::profile_dir().join("skins.blob")
}

/// Write the wardrobe to `path`, creating the parent dir. Best-effort caller
/// semantics: returns the error so the caller can log, but never panics.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_to_path(path: &std::path::Path, w: &SkinWardrobe) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir profile: {e}"))?;
    }
    crate::save::check_blob_writable(path)?;
    let bytes = to_blob_bytes(w)?;
    crate::save::write_atomic(path, &bytes).map_err(|e| format!("write skins: {e}"))
}

/// Peek the version byte of a bincode wardrobe blob without decoding the PNG
/// entries (which can be large / fail independently). `StoredWardrobe`'s
/// first field is `version: u8`; bincode's default deserializer reads
/// exactly the bytes it needs for the requested type and doesn't require
/// consuming the rest of the buffer, so deserializing just this one-field
/// struct reads the same leading byte a full `StoredWardrobe` decode would.
fn peek_blob_version(bytes: &[u8]) -> Option<u8> {
    #[derive(Deserialize)]
    struct VersionOnly {
        version: u8,
    }
    bincode::deserialize::<VersionOnly>(bytes).ok().map(|v| v.version)
}

/// Native safety net for the box-unwrap-fix migration (v0.2.19): the first
/// time a v1 blob is loaded from `path`, copy the untouched original bytes to
/// `<path>.v1.bak` (skipped if that backup already exists), before anything
/// in the load path can overwrite `path`. A copy failure only logs a
/// warning — it never blocks the load. Web has no on-disk file to back up
/// (localStorage); see the wardrobe spec for that limitation.
#[cfg(not(target_arch = "wasm32"))]
fn backup_v1_blob_if_absent(path: &std::path::Path) {
    let mut bak = path.as_os_str().to_owned();
    bak.push(".v1.bak");
    let bak_path = std::path::PathBuf::from(bak);
    if bak_path.exists() {
        return;
    }
    if let Err(e) = std::fs::copy(path, &bak_path) {
        log::warn!("skin wardrobe v1 backup failed for {}: {e}", bak_path.display());
    }
}

/// Read the wardrobe from `path`. A missing file is `Ok(None)` (first run); a
/// corrupt file is `Err`. Backs up a v1 blob to `<path>.v1.bak` before the
/// box-unwrap-fix migration runs — see `backup_v1_blob_if_absent`.
///
/// On a failed load (audit 2026-09-27) the damaged file is renamed aside to
/// `<path>.corrupt-<ts>` and every later `save_to_path` for this path is refused
/// for the rest of the session — the caller's fresh in-memory wardrobe must
/// never replace the player's painted skins.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_from_path(path: &std::path::Path) -> Result<Option<SkinWardrobe>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if peek_blob_version(&bytes) == Some(1) {
                backup_v1_blob_if_absent(path);
            }
            from_blob_bytes(&bytes).map(Some).map_err(|e| {
                crate::save::mark_blob_load_failed(path);
                let _ = crate::save::quarantine_corrupt(path);
                log::error!("skin wardrobe {} is damaged ({e}); saving is off this session", path.display());
                e
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => {
            crate::save::mark_blob_load_failed(path);
            Err(format!("read skins: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosmetics::SkinSource;
    use crate::skin_wardrobe::SkinWardrobe;

    fn solid(v: u8) -> Vec<u8> {
        vec![v; 64 * 64 * 4]
    }

    #[test]
    fn blob_roundtrip_is_byte_identical() {
        let mut w = SkinWardrobe::new();
        let a = w.add_from_rgba("Knight".into(), solid(40), 1);
        w.set_equipped(a);
        let _b = w.add_blank(2);

        let bytes = to_blob_bytes(&w).expect("pack");
        let mut back = from_blob_bytes(&bytes).expect("unpack");

        assert_eq!(back.entries().len(), w.entries().len());
        assert_eq!(back.equipped_id(), w.equipped_id());
        // PNG is lossless, so custom pixels survive byte-for-byte.
        assert_eq!(back.active_descriptor().skin_rgba(), solid(40));
        // Default entry stays Default (not re-encoded to pixels).
        assert_eq!(back.get(1).unwrap().source, SkinSource::Default);
        // next_id preserved so new ids don't collide after reload.
        assert_eq!(back.add_blank(9), w.add_blank(9), "next_id survives the round-trip");
    }

    #[test]
    fn from_blob_rejects_garbage() {
        assert!(from_blob_bytes(b"not a wardrobe blob").is_err());
    }

    #[test]
    fn migrate_legacy_png_makes_one_equipped_entry() {
        // Encode a known skin to PNG the way the old single-skin path stored it.
        let rgba = solid(123);
        let img = image::RgbaImage::from_raw(64, 64, rgba.clone()).unwrap();
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();

        let w = migrate_legacy_png(&png);
        assert_eq!(w.entries().len(), 1);
        assert_eq!(w.active_descriptor().skin_rgba(), rgba, "migrated pixels match");
    }

    #[test]
    fn migrate_legacy_png_degrades_on_garbage() {
        let w = migrate_legacy_png(b"not a png");
        assert_eq!(w, SkinWardrobe::new(), "unreadable legacy skin → fresh default wardrobe");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_save_load_roundtrip() {
        let mut w = SkinWardrobe::new();
        let id = w.add_from_rgba("Diver".into(), solid(77), 1);
        w.set_equipped(id);

        let mut path = std::env::temp_dir();
        path.push("axenstax_skin_wardrobe_test_t4.blob");
        let _ = std::fs::remove_file(&path);

        save_to_path(&path, &w).expect("save");
        let loaded = load_from_path(&path).expect("load ok").expect("present");
        assert_eq!(loaded.active_descriptor().skin_rgba(), solid(77));

        std::fs::remove_file(&path).ok();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_load_missing_is_none() {
        let mut path = std::env::temp_dir();
        path.push("axenstax_skin_wardrobe_test_absent_t4.blob");
        let _ = std::fs::remove_file(&path);
        assert!(load_from_path(&path).expect("ok").is_none());
    }

    #[test]
    fn blob_preserves_transparent_coloured_texels() {
        // The outer layer relies on alpha=0 texels with non-zero RGB surviving
        // PNG persistence (spec §6.5 eraser→alpha). Pin one such pixel.
        let mut rgba = vec![0u8; 64 * 64 * 4];
        rgba[0] = 255; // R
        rgba[1] = 0;   // G
        rgba[2] = 0;   // B
        rgba[3] = 0;   // A — fully transparent, bright red
        let mut w = SkinWardrobe::new();
        let id = w.add_from_rgba("Ghost".into(), rgba.clone(), 1);
        w.set_equipped(id);
        let back = from_blob_bytes(&to_blob_bytes(&w).unwrap()).unwrap();
        assert_eq!(back.active_descriptor().skin_rgba(), rgba, "alpha=0, RGB!=0 texels must survive PNG round-trip");
    }

    /// A 64×64 RGBA buffer with a marker at the head-front base rect's left
    /// edge (x0=8, y=8) so a flip is easy to detect (moves to x1-1=15).
    fn head_front_marked(marker: [u8; 4]) -> Vec<u8> {
        let mut buf = vec![0u8; 64 * 64 * 4];
        let idx = (8 * 64 + 8) * 4;
        buf[idx..idx + 4].copy_from_slice(&marker);
        buf
    }

    fn v1_bytes(entries: Vec<StoredEntry>, equipped: SkinId, next_id: SkinId) -> Vec<u8> {
        let blob = StoredWardrobe { version: 1, entries, equipped, next_id };
        bincode::serialize(&blob).expect("serialize v1 blob")
    }

    #[test]
    fn v1_load_migrates_old_hand_painted_entries_only() {
        let old_painted_rgba = head_front_marked([9, 8, 7, 6]);
        let new_painted_rgba = solid(50);
        let imported_rgba = solid(60);

        let entries = vec![
            StoredEntry {
                id: 1,
                name: "Default".into(),
                is_default: true,
                png: Vec::new(),
                mc_handle: None,
                mc_uuid: None,
                created: 0,
                modified: 0,
            },
            StoredEntry {
                id: 2,
                name: "Old painted".into(),
                is_default: false,
                png: rgba_to_png(&old_painted_rgba).unwrap(),
                mc_handle: None,
                mc_uuid: None,
                created: 1_700_000_000,
                modified: 1_700_000_000, // before UNWRAP_FIX_CUTOFF → flipped
            },
            StoredEntry {
                id: 3,
                name: "New painted".into(),
                is_default: false,
                png: rgba_to_png(&new_painted_rgba).unwrap(),
                mc_handle: None,
                mc_uuid: None,
                created: 1_790_000_000,
                modified: 1_790_000_000, // after UNWRAP_FIX_CUTOFF → unchanged
            },
            StoredEntry {
                id: 4,
                name: "Notch".into(),
                is_default: false,
                png: rgba_to_png(&imported_rgba).unwrap(),
                mc_handle: Some("Notch".into()),
                mc_uuid: Some("uuid-1".into()),
                created: 1_700_000_000,
                modified: 1_700_000_000, // imported → never touched, regardless of age
            },
        ];
        let bytes = v1_bytes(entries, 1, 5);

        let w = from_blob_bytes(&bytes).expect("v1 blob decodes");

        let mut expected_flipped = old_painted_rgba.clone();
        crate::skin_uv::mirror_every_face(&mut expected_flipped);
        assert_ne!(expected_flipped, old_painted_rgba, "sanity: the marker actually moves under a flip");

        match &w.get(2).unwrap().source {
            SkinSource::Rgba64(px) => assert_eq!(*px, expected_flipped, "old hand-painted entry is mirrored"),
            SkinSource::Default => panic!("entry 2 must be Rgba64"),
        }
        assert_eq!(w.get(3).unwrap().source, SkinSource::Rgba64(new_painted_rgba), "entry modified after the cutoff is untouched");
        assert_eq!(w.get(4).unwrap().source, SkinSource::Rgba64(imported_rgba), "an imported entry is never mirrored, even if old");
        assert_eq!(w.get(1).unwrap().source, SkinSource::Default, "the default entry is untouched");

        assert_eq!(w.equipped_id(), 1, "equipped id preserved");
        assert_eq!(w.get(4).unwrap().minecraft_handle.as_deref(), Some("Notch"));

        // Re-save: the migrated wardrobe must persist at the CURRENT version
        // and never be re-flipped on a subsequent load.
        let bytes_v3 = to_blob_bytes(&w).expect("re-save");
        assert_eq!(peek_blob_version(&bytes_v3), Some(3), "a migrated wardrobe saves as the current version");
        let w2 = from_blob_bytes(&bytes_v3).expect("v3 blob decodes");
        assert_eq!(w2.get(2).unwrap().source, w.get(2).unwrap().source, "loading a v3 blob does not re-flip");
        assert_eq!(w2.get(3).unwrap().source, w.get(3).unwrap().source);
        assert!(
            w2.entries().iter().all(|e| e.arm_model == ArmModel::Classic),
            "every entry restored from a v1 blob is Classic"
        );
        assert_eq!(w2.get(4).unwrap().source, w.get(4).unwrap().source);
    }

    #[test]
    fn v1_load_modified_zero_counts_as_old() {
        // Entries with modified==0 predate timestamps entirely (e.g. the
        // legacy-PNG migration path) and must be treated as pre-fix.
        let old_painted_rgba = head_front_marked([1, 2, 3, 4]);
        let entries = vec![StoredEntry {
            id: 2,
            name: "Ancient".into(),
            is_default: false,
            png: rgba_to_png(&old_painted_rgba).unwrap(),
            mc_handle: None,
            mc_uuid: None,
            created: 0,
            modified: 0,
        }];
        let bytes = v1_bytes(entries, 2, 3);
        let w = from_blob_bytes(&bytes).expect("v1 blob decodes");

        let mut expected_flipped = old_painted_rgba.clone();
        crate::skin_uv::mirror_every_face(&mut expected_flipped);
        assert_eq!(w.get(2).unwrap().source, SkinSource::Rgba64(expected_flipped), "modified==0 is treated as pre-fix and flipped");
    }

    // ── v3: per-entry arm model ──────────────────────────────────────────────

    /// Mint a genuine v2 blob (the previous on-disk shape) — the shape a player
    /// upgrading from v0.2.22 actually has on disk.
    fn v2_bytes(entries: Vec<StoredEntry>, equipped: SkinId, next_id: SkinId) -> Vec<u8> {
        let blob = StoredWardrobe { version: 2, entries, equipped, next_id };
        bincode::serialize(&blob).expect("serialize v2 blob")
    }

    #[test]
    fn save_always_writes_v3() {
        let w = SkinWardrobe::new();
        assert_eq!(peek_blob_version(&to_blob_bytes(&w).unwrap()), Some(3));
        assert_eq!(BLOB_VERSION, 3);
    }

    #[test]
    fn v3_roundtrip_keeps_slim() {
        let mut w = SkinWardrobe::new();
        let slim = w.add_from_rgba("Alex".into(), solid(11), 1);
        let classic = w.add_from_rgba("Steve".into(), solid(22), 1);
        assert!(w.set_arm_model(slim, ArmModel::Slim));
        w.set_equipped(slim);

        let back = from_blob_bytes(&to_blob_bytes(&w).unwrap()).expect("v3 round-trip");
        assert_eq!(back.arm_model(slim), ArmModel::Slim, "a slim entry stays slim across a save");
        assert_eq!(back.arm_model(classic), ArmModel::Classic, "…and its neighbour stays classic");
        assert_eq!(
            back.active_descriptor().arm_model,
            ArmModel::Slim,
            "the worn look comes back slim"
        );
        assert_eq!(back.active_descriptor().skin_rgba(), solid(11), "pixels survive too");
    }

    #[test]
    fn a_v2_blob_loads_with_classic_arms() {
        // The upgrade path: no arm field on disk, so everything is Classic and
        // nothing errors on the missing bytes.
        let entries = vec![
            StoredEntry {
                id: 1,
                name: "Default".into(),
                is_default: true,
                png: Vec::new(),
                mc_handle: None,
                mc_uuid: None,
                created: 0,
                modified: 0,
            },
            StoredEntry {
                id: 2,
                name: "Painted".into(),
                is_default: false,
                png: rgba_to_png(&solid(33)).unwrap(),
                mc_handle: None,
                mc_uuid: None,
                created: 1_790_000_000,
                modified: 1_790_000_000,
            },
        ];
        let w = from_blob_bytes(&v2_bytes(entries, 2, 3)).expect("a v2 blob must still load");
        assert_eq!(w.entries().len(), 2);
        assert_eq!(w.arm_model(1), ArmModel::Classic);
        assert_eq!(w.arm_model(2), ArmModel::Classic);
        // …and the pixels are NOT re-mirrored (v2 is past the unwrap migration).
        assert_eq!(w.get(2).unwrap().source, SkinSource::Rgba64(solid(33)));
        // Re-saving upgrades it in place.
        assert_eq!(peek_blob_version(&to_blob_bytes(&w).unwrap()), Some(3));
    }

    #[test]
    fn peek_reads_the_version_of_every_shape() {
        assert_eq!(peek_blob_version(&v1_bytes(Vec::new(), 1, 2)), Some(1));
        assert_eq!(peek_blob_version(&v2_bytes(Vec::new(), 1, 2)), Some(2));
        assert_eq!(peek_blob_version(&to_blob_bytes(&SkinWardrobe::new()).unwrap()), Some(3));
        assert_eq!(peek_blob_version(b""), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_save_load_roundtrip_keeps_slim() {
        let mut w = SkinWardrobe::new();
        let id = w.add_from_rgba("Alex".into(), solid(88), 1);
        w.set_arm_model(id, ArmModel::Slim);
        w.set_equipped(id);

        let mut path = std::env::temp_dir();
        path.push("axenstax_skin_wardrobe_test_slim.blob");
        let _ = std::fs::remove_file(&path);

        save_to_path(&path, &w).expect("save");
        let loaded = load_from_path(&path).expect("load ok").expect("present");
        assert_eq!(loaded.arm_model(id), ArmModel::Slim);
        assert_eq!(loaded.active_descriptor().arm_model, ArmModel::Slim);

        std::fs::remove_file(&path).ok();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_load_backs_up_v1_blob_before_migrating() {
        let rgba = solid(77);
        let entries = vec![StoredEntry {
            id: 2,
            name: "Old".into(),
            is_default: false,
            png: rgba_to_png(&rgba).unwrap(),
            mc_handle: None,
            mc_uuid: None,
            created: 1_700_000_000,
            modified: 1_700_000_000,
        }];
        let bytes = v1_bytes(entries, 2, 3);

        let mut path = std::env::temp_dir();
        path.push("axenstax_skin_wardrobe_test_v1backup.blob");
        let mut bak_os = path.as_os_str().to_owned();
        bak_os.push(".v1.bak");
        let bak_path = std::path::PathBuf::from(bak_os);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&bak_path);

        std::fs::write(&path, &bytes).expect("seed v1 file");

        let _ = load_from_path(&path).expect("load ok").expect("present");
        let backed_up = std::fs::read(&bak_path).expect("backup file created");
        assert_eq!(backed_up, bytes, "backup preserves the original v1 bytes untouched");

        // A second load must not error, and must not disturb the existing backup.
        let _ = load_from_path(&path).expect("load ok").expect("present");
        let backed_up_again = std::fs::read(&bak_path).expect("backup still present");
        assert_eq!(backed_up_again, bytes, "second load does not overwrite an existing backup");

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&bak_path).ok();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_failed_load_quarantines_the_blob_and_blocks_every_save() {
        let dir = std::env::temp_dir()
            .join(format!("axe_skin_store_corrupt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("skins.blob");
        let torn = b"\x02truncated wardrobe";
        std::fs::write(&path, torn).unwrap();

        assert!(load_from_path(&path).is_err());
        // Kept aside byte for byte.
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("skins.blob.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read(aside[0].path()).unwrap(), torn);
        // The caller's fresh wardrobe can never be written over it this session.
        assert!(save_to_path(&path, &SkinWardrobe::new()).is_err());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_to_path_is_atomic_and_leaves_no_temp() {
        let dir = std::env::temp_dir()
            .join(format!("axe_skin_store_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("skins.blob");
        save_to_path(&path, &SkinWardrobe::new()).unwrap();
        assert!(load_from_path(&path).unwrap().is_some());
        assert!(!dir.join("skins.blob.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
