//! `world.dat` format-version footer (gap-audit T1-7, Spec 02 §8.4).
//!
//! `world.dat` is positional bincode 1 (`save::WorldSave`), and `WorldSave` only
//! ever APPENDS a field. Before this footer existed, a build reading a save from a
//! NEWER build decoded the fields it knew and ignored the rest — then a re-save
//! silently dropped the newer fields (an AppImage rollback lost data).
//!
//! Every `world.dat` is now written as
//!
//! ```text
//! bincode(WorldSave) || SAVE_FORMAT_VERSION: u32 LE || SAVE_FOOTER_MAGIC (8 bytes)
//! ```
//!
//! A FOOTER, not a header, because builds from before the footer already ignore
//! unknown trailing bytes: they keep opening new saves exactly as before (no
//! worse), while every build from now on can read the version and refuse a save
//! it does not understand. A header would have made every new save unreadable to
//! the builds already shipped.
//!
//! Reading (`save::read_world_save`): no footer → the legacy footer-less path,
//! unchanged; footer with a version ≤ ours → strip it and decode as before;
//! version > ours → [`WorldSaveError::NewerVersion`], and nothing writes to that
//! world (the write paths check the file on disk first, see
//! `save::refuse_write_over_newer_save`).

use std::fmt;

use crate::save::WorldSave;

/// Positional `WorldSave` fields this build reads and writes. Pinned by the
/// `world_save_field_count_tripwire` test against serde's own field list, so
/// appending a `WorldSave` field fails the test until this is bumped — which bumps
/// [`SAVE_FORMAT_VERSION`] with it.
pub const WORLD_SAVE_FIELD_COUNT: u32 = 52;

/// Bump by hand for a wire change that is NOT an appended `WorldSave` field: a
/// field or enum variant added inside a nested saved type, a retype. Such a change
/// is invisible to the field-count tripwire, but an older build cannot read it
/// either, so it must still move the version. Only ever increases.
pub const SAVE_LAYOUT_REVISION: u32 = 0;

/// The `world.dat` format version this build writes, and the newest it opens.
/// Monotonic: both terms only ever increase.
pub const SAVE_FORMAT_VERSION: u32 = WORLD_SAVE_FIELD_COUNT + SAVE_LAYOUT_REVISION;

/// Last 8 bytes of every footer-bearing `world.dat`. The `v1` names this footer
/// layout, not the save format (that is the `u32` before it).
pub const SAVE_FOOTER_MAGIC: [u8; 8] = *b"AXSAVEv1";

/// `u32` version + magic.
pub const SAVE_FOOTER_LEN: usize = 4 + SAVE_FOOTER_MAGIC.len();

/// What the lobby shows for a world it refuses to open. UK English.
pub const NEWER_WORLD_MESSAGE: &str =
    "This world was saved by a newer version of Axe'n'Stax. Update the game to open it.";

/// Why a `world.dat` byte stream could not be turned into a `WorldSave`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldSaveError {
    /// Written by a newer build in a format this build does not understand.
    /// Refused outright: decoding it would drop the newer state on re-save.
    NewerVersion { found: u32, supported: u32 },
    /// Not a decodable save (damaged, or not a world save at all).
    Undecodable(String),
}

impl WorldSaveError {
    pub fn is_newer_version(&self) -> bool {
        matches!(self, Self::NewerVersion { .. })
    }
}

impl fmt::Display for WorldSaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The player-facing text, exactly: it reaches the lobby banner through
            // the `String` error paths (`load_world`, `unpack_world`).
            Self::NewerVersion { .. } => f.write_str(NEWER_WORLD_MESSAGE),
            Self::Undecodable(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for WorldSaveError {}

impl From<WorldSaveError> for String {
    fn from(e: WorldSaveError) -> Self {
        e.to_string()
    }
}

/// Whether an error string that came through a `String` error path (an archive
/// unpack, a load) is the newer-version refusal, so the caller can surface it to
/// the player rather than only log it. Used by the web load path (native checks
/// the folder up front with `save::world_open_refusal`).
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn is_newer_world_error(err: &str) -> bool {
    err.contains(NEWER_WORLD_MESSAGE)
}

/// The footer for `version`.
pub fn footer_bytes(version: u32) -> [u8; SAVE_FOOTER_LEN] {
    let mut out = [0u8; SAVE_FOOTER_LEN];
    out[..4].copy_from_slice(&version.to_le_bytes());
    out[4..].copy_from_slice(&SAVE_FOOTER_MAGIC);
    out
}

/// Encode a `WorldSave` as the bytes of a `world.dat`: bincode payload + footer.
/// The ONE encoder every `world.dat` writer uses (native save, autosave, the
/// dedicated server, and `world_archive::pack_world` for the web, cloud, export,
/// profile bundle and replay paths).
pub fn encode_world_save(save: &WorldSave) -> Result<Vec<u8>, String> {
    let mut out = bincode::serialize(save).map_err(|e| format!("serialize world.dat: {e}"))?;
    out.extend_from_slice(&footer_bytes(SAVE_FORMAT_VERSION));
    Ok(out)
}

/// Read the version from a footer-sized tail, or `None` if it is not a footer.
pub fn footer_version(tail: &[u8]) -> Option<u32> {
    if tail.len() != SAVE_FOOTER_LEN || tail[4..] != SAVE_FOOTER_MAGIC {
        return None;
    }
    Some(u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]))
}

/// Split a `world.dat` into its bincode payload and footer version. A stream
/// without the footer (every save written before it existed) comes back whole,
/// with `None`.
pub fn split_save_footer(data: &[u8]) -> (&[u8], Option<u32>) {
    if data.len() >= SAVE_FOOTER_LEN {
        let at = data.len() - SAVE_FOOTER_LEN;
        if let Some(v) = footer_version(&data[at..]) {
            return (&data[..at], Some(v));
        }
    }
    (data, None)
}

/// The refusal for a footer version, if this build must not open it.
pub fn check_version(found: Option<u32>) -> Result<(), WorldSaveError> {
    match found {
        Some(found) if found > SAVE_FORMAT_VERSION => Err(WorldSaveError::NewerVersion {
            found,
            supported: SAVE_FORMAT_VERSION,
        }),
        _ => Ok(()),
    }
}

/// The footer version of the `world.dat` at `path`, reading only its last
/// [`SAVE_FOOTER_LEN`] bytes. `None` for a missing, short or footer-less file.
#[cfg(not(target_arch = "wasm32"))]
pub fn file_footer_version(path: &std::path::Path) -> Option<u32> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len < SAVE_FOOTER_LEN as u64 {
        return None;
    }
    f.seek(SeekFrom::End(-(SAVE_FOOTER_LEN as i64))).ok()?;
    let mut tail = [0u8; SAVE_FOOTER_LEN];
    f.read_exact(&mut tail).ok()?;
    footer_version(&tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::minimal_world_save_for_tests;

    #[test]
    fn footer_round_trips() {
        let save = minimal_world_save_for_tests(7);
        let bytes = encode_world_save(&save).unwrap();
        let (payload, version) = split_save_footer(&bytes);
        assert_eq!(version, Some(SAVE_FORMAT_VERSION));
        assert_eq!(payload, bincode::serialize(&save).unwrap().as_slice());
        assert_eq!(&bytes[bytes.len() - 8..], b"AXSAVEv1");
        assert_eq!(
            bytes[bytes.len() - SAVE_FOOTER_LEN..bytes.len() - 8],
            SAVE_FORMAT_VERSION.to_le_bytes(),
            "version is a little-endian u32 just before the magic"
        );
    }

    #[test]
    fn footerless_bytes_come_back_whole() {
        let plain = bincode::serialize(&minimal_world_save_for_tests(7)).unwrap();
        assert_eq!(split_save_footer(&plain), (plain.as_slice(), None));
        assert_eq!(split_save_footer(b"short"), (&b"short"[..], None));
        assert_eq!(split_save_footer(&[]), (&[][..], None));
        // The magic alone, with no room for a version, is not a footer.
        assert_eq!(split_save_footer(b"AXSAVEv1").1, None);
    }

    #[test]
    fn only_a_newer_version_is_refused() {
        assert!(check_version(None).is_ok(), "footer-less = legacy, opened as before");
        assert!(check_version(Some(1)).is_ok());
        assert!(check_version(Some(SAVE_FORMAT_VERSION)).is_ok());
        assert_eq!(
            check_version(Some(SAVE_FORMAT_VERSION + 1)),
            Err(WorldSaveError::NewerVersion {
                found: SAVE_FORMAT_VERSION + 1,
                supported: SAVE_FORMAT_VERSION,
            })
        );
    }

    #[test]
    fn newer_version_error_reads_as_the_lobby_message() {
        let e = WorldSaveError::NewerVersion { found: 99, supported: 52 };
        assert!(e.is_newer_version());
        let s: String = e.into();
        assert_eq!(
            s,
            "This world was saved by a newer version of Axe'n'Stax. Update the game to open it."
        );
        assert!(is_newer_world_error(&format!("import failed: {s}")));
        assert!(!is_newer_world_error("deserialize (legacy fallback): io error"));
        assert!(!WorldSaveError::Undecodable("x".into()).is_newer_version());
    }

    /// TRIPWIRE: appending a `WorldSave` field without bumping
    /// `WORLD_SAVE_FIELD_COUNT` (and so `SAVE_FORMAT_VERSION`) fails here. serde's
    /// JSON object for `WorldSave` has one key per serialised field — the same
    /// fields, in the same order, that bincode writes positionally (the struct has
    /// no `skip`/`skip_serializing_if`, which would make the two differ).
    #[test]
    fn world_save_field_count_tripwire() {
        let v = serde_json::to_value(minimal_world_save_for_tests(1)).unwrap();
        let fields = v.as_object().expect("WorldSave serialises as a struct").len();
        assert_eq!(
            fields as u32, WORLD_SAVE_FIELD_COUNT,
            "WorldSave has {fields} fields but WORLD_SAVE_FIELD_COUNT is \
             {WORLD_SAVE_FIELD_COUNT}. You appended a field: bump WORLD_SAVE_FIELD_COUNT \
             (which bumps SAVE_FORMAT_VERSION) so older builds refuse the new saves \
             instead of truncating them, and add the field to the tolerant reader."
        );
        assert_eq!(SAVE_FORMAT_VERSION, WORLD_SAVE_FIELD_COUNT + SAVE_LAYOUT_REVISION);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_footer_version_reads_only_the_tail() {
        let dir = std::env::temp_dir()
            .join(format!("axenstax-footer-{:?}", std::thread::current().id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("world.dat");
        std::fs::write(&p, encode_world_save(&minimal_world_save_for_tests(3)).unwrap()).unwrap();
        assert_eq!(file_footer_version(&p), Some(SAVE_FORMAT_VERSION));
        std::fs::write(&p, bincode::serialize(&minimal_world_save_for_tests(3)).unwrap()).unwrap();
        assert_eq!(file_footer_version(&p), None, "footer-less legacy file");
        std::fs::write(&p, b"tiny").unwrap();
        assert_eq!(file_footer_version(&p), None);
        assert_eq!(file_footer_version(&dir.join("missing.dat")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
