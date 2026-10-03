//! `.axeprofile` — the whole-profile bundle ("Take your worlds to native").
//!
//! Web saves live in the browser (IndexedDB) and are local-only by design (the
//! web build is an anonymous local sandbox). A player who wants to carry on in
//! the desktop app needs a way to take **everything** across in one go, not one
//! world at a time. `.axeprofile` is that container: an ordered list of
//! entries, each one either a world (the exact `.axeworld` bytes the per-world
//! export already produces — see `crate::world_archive`) or the player's Trials
//! records (`TrialBests::to_json`).
//!
//! It is a *container*, not a second world format. World entries carry the
//! existing archive verbatim, so a `.axeprofile` never drifts from `.axeworld`.
//!
//! Purely local: the file is written by a browser download and read by the
//! desktop app's file picker. There is no upload, no server call, and no
//! identity anywhere in this path.
//!
//! ## Byte layout (little-endian throughout)
//!
//! ```text
//! offset  size  field
//! 0       8     magic          b"AXEPROFL"
//! 8       1     format version u8 (currently 1)
//! 9       4     entry count    u32
//! then, `count` times:
//!         1     kind           u8  (1 = World, 2 = Trials; others reserved)
//!         4     name length    u32 (bytes of UTF-8)
//!         n     name           UTF-8
//!         4     payload length u32
//!         m     payload        raw bytes
//! ```
//!
//! Forward compatibility: an entry whose `kind` byte isn't recognised is
//! **skipped**, not fatal — its length fields still describe it exactly, so a
//! newer writer can add entry kinds without breaking older readers. A short
//! read anywhere is a clean [`BundleError::Truncated`], never a panic.

/// Magic bytes at the head of every `.axeprofile` file.
pub const MAGIC: &[u8; 8] = b"AXEPROFL";

/// Container format version. Bump only for a layout change; new *entry kinds*
/// need no bump (unknown kinds are skipped).
pub const FORMAT_VERSION: u8 = 1;

/// File extension for a profile bundle (native dialog filter + web filename).
pub const PROFILE_FILE_EXT: &str = "axeprofile";

/// Upper bound on a single entry's payload, as a corrupt-input guard: a bogus
/// length must not make us try to allocate gigabytes before we notice the file
/// is short. 512 MiB is far above any real profile.
// Reading a bundle is the native half of the feature — dead on a web build.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_ENTRY_BYTES: usize = 512 * 1024 * 1024;

/// What one entry in the bundle carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A packed world — byte-identical to a `.axeworld` export.
    World,
    /// The player's Trials store, as `TrialBests::to_json`.
    Trials,
}

impl EntryKind {
    /// The on-disk discriminant. Only the writer (`pack`) needs it, and the
    /// only production writer is the web export — invisible to a native build.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn as_byte(self) -> u8 {
        match self {
            EntryKind::World => 1,
            EntryKind::Trials => 2,
        }
    }

    /// Parse a discriminant. `None` for an unrecognised (future) kind — the
    /// reader skips those rather than failing. Reader-side, so dead on web.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(EntryKind::World),
            2 => Some(EntryKind::Trials),
            _ => None,
        }
    }
}

/// One entry: what it is, what it's called, and its raw bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    /// For a world, the name it was saved under (the web world name). For the
    /// Trials entry, a label — readers key off `kind`, not this.
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Why a `.axeprofile` couldn't be read. Reader-side, so dead on a web build.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// The file doesn't start with [`MAGIC`] — not a profile bundle at all.
    BadMagic,
    /// A profile bundle, but a format version this build doesn't understand.
    UnsupportedVersion(u8),
    /// The file ends in the middle of a field — cut short in transit or on disk.
    Truncated,
    /// An entry name isn't valid UTF-8.
    BadName,
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::BadMagic => {
                write!(f, "That's not an Axe'n'Stax profile file.")
            }
            BundleError::UnsupportedVersion(v) => write!(
                f,
                "That profile was made by a newer version of Axe'n'Stax (format {v}) — update the app and try again."
            ),
            BundleError::Truncated => {
                write!(f, "That profile file is incomplete — try exporting it again.")
            }
            BundleError::BadName => {
                write!(f, "That profile file is damaged (an unreadable name).")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pack
// ---------------------------------------------------------------------------

/// Serialise entries into `.axeprofile` bytes. Pure — no I/O, no allocation
/// surprises: the output is exactly the layout documented at the top of this
/// module, in the order given.
///
/// Writing a bundle is the web half of the feature (native only ever reads
/// one), so this is dead on a native non-test build.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn pack(entries: &[Entry]) -> Vec<u8> {
    let payload_total: usize = entries
        .iter()
        .map(|e| 9 + e.name.len() + e.bytes.len())
        .sum();
    let mut out = Vec::with_capacity(13 + payload_total);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        out.push(e.kind.as_byte());
        out.extend_from_slice(&(e.name.len() as u32).to_le_bytes());
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&(e.bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&e.bytes);
    }
    out
}

// ---------------------------------------------------------------------------
// Unpack
// ---------------------------------------------------------------------------

/// A tiny forward-only reader over the bundle bytes. Every read is bounds
/// checked and returns [`BundleError::Truncated`] rather than panicking.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], BundleError> {
        let end = self.pos.checked_add(n).ok_or(BundleError::Truncated)?;
        if end > self.buf.len() {
            return Err(BundleError::Truncated);
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, BundleError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, BundleError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A length-prefixed run of bytes, guarded against an absurd length so a
    /// corrupt file can't drive a huge allocation before the bounds check.
    fn blob(&mut self) -> Result<&'a [u8], BundleError> {
        let len = self.u32()? as usize;
        if len > MAX_ENTRY_BYTES {
            return Err(BundleError::Truncated);
        }
        self.take(len)
    }
}

/// Parse `.axeprofile` bytes back into entries.
///
/// Unknown entry kinds are skipped (their length fields still describe them
/// exactly), so a bundle written by a newer build still yields everything this
/// build understands. Any short read is [`BundleError::Truncated`].
///
/// Reading a bundle is the native half of the feature — dead on a web build.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn unpack(bytes: &[u8]) -> Result<Vec<Entry>, BundleError> {
    let mut cur = Cursor { buf: bytes, pos: 0 };

    let magic = cur.take(MAGIC.len()).map_err(|_| BundleError::BadMagic)?;
    if magic != MAGIC {
        return Err(BundleError::BadMagic);
    }
    let version = cur.u8()?;
    if version != FORMAT_VERSION {
        return Err(BundleError::UnsupportedVersion(version));
    }
    let count = cur.u32()? as usize;

    let mut out = Vec::new();
    for _ in 0..count {
        let kind_byte = cur.u8()?;
        let name_bytes = cur.blob()?;
        let payload = cur.blob()?;
        // Read (and therefore validate the framing of) every entry, including
        // ones this build doesn't know — then drop the unknown ones.
        let Some(kind) = EntryKind::from_byte(kind_byte) else {
            continue;
        };
        let name = std::str::from_utf8(name_bytes).map_err(|_| BundleError::BadName)?;
        out.push(Entry {
            kind,
            name: name.to_string(),
            bytes: payload.to_vec(),
        });
    }
    Ok(out)
}

/// Build the download filename for a profile exported on `date` (an ISO
/// `YYYY-MM-DD` string). Pure so the naming rule is testable without a clock.
/// Web-side only — native reads bundles, it never names one.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn profile_filename(date: &str) -> String {
    format!("{date}-axenstax-profile.{PROFILE_FILE_EXT}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn world(name: &str, bytes: &[u8]) -> Entry {
        Entry { kind: EntryKind::World, name: name.to_string(), bytes: bytes.to_vec() }
    }

    #[test]
    fn round_trips_worlds_and_trials_in_order() {
        let entries = vec![
            world("first", b"\x1f\x8b packed world one"),
            world("second — with a dash and an accent é", b""),
            Entry {
                kind: EntryKind::Trials,
                name: "trials".to_string(),
                bytes: br#"{"bests":{}}"#.to_vec(),
            },
        ];
        let packed = pack(&entries);
        assert_eq!(&packed[..8], MAGIC, "magic must lead the file");
        assert_eq!(packed[8], FORMAT_VERSION, "version byte follows the magic");

        let back = unpack(&packed).expect("round trip must succeed");
        assert_eq!(back, entries, "entries must survive verbatim and in order");
    }

    #[test]
    fn empty_bundle_round_trips() {
        let packed = pack(&[]);
        assert_eq!(packed.len(), 13, "header only: 8 magic + 1 version + 4 count");
        assert_eq!(unpack(&packed).expect("empty must parse"), Vec::<Entry>::new());
    }

    #[test]
    fn unknown_entry_kind_is_skipped_not_fatal() {
        // Hand-build a bundle with a known entry, then a kind byte no build
        // understands, then another known entry — the reader must return the
        // two it knows and step cleanly over the middle one.
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.push(FORMAT_VERSION);
        b.extend_from_slice(&3u32.to_le_bytes());
        let mut push = |kind: u8, name: &str, payload: &[u8], out: &mut Vec<u8>| {
            out.push(kind);
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(payload);
        };
        push(EntryKind::World.as_byte(), "keep-me", b"aaa", &mut b);
        push(200, "from-the-future", b"who knows", &mut b);
        push(EntryKind::Trials.as_byte(), "trials", b"{}", &mut b);

        let got = unpack(&b).expect("unknown kinds must not be fatal");
        assert_eq!(got.len(), 2, "the unknown entry is skipped, the rest survive");
        assert_eq!(got[0].name, "keep-me");
        assert_eq!(got[1].kind, EntryKind::Trials, "framing stayed in step after the skip");
        assert_eq!(got[1].bytes, b"{}".to_vec());
    }

    #[test]
    fn truncation_is_a_clean_error_at_every_cut() {
        let packed = pack(&[world("a-world", b"payload bytes here")]);
        // Every prefix shorter than the whole file must be an error, never a
        // panic and never a partially-believed entry.
        for cut in 0..packed.len() {
            let err = unpack(&packed[..cut]).expect_err("a short file must not parse");
            assert!(
                matches!(err, BundleError::Truncated | BundleError::BadMagic),
                "cut at {cut} gave {err:?}"
            );
        }
        assert!(unpack(&packed).is_ok(), "the untruncated file still parses");
    }

    #[test]
    fn absurd_entry_length_does_not_allocate() {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.push(FORMAT_VERSION);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(EntryKind::World.as_byte());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(b"hi");
        b.extend_from_slice(&u32::MAX.to_le_bytes()); // payload claims 4 GiB
        assert_eq!(unpack(&b), Err(BundleError::Truncated));
    }

    #[test]
    fn version_mismatch_is_reported_not_guessed() {
        let mut packed = pack(&[world("w", b"x")]);
        packed[8] = FORMAT_VERSION + 1;
        assert_eq!(unpack(&packed), Err(BundleError::UnsupportedVersion(FORMAT_VERSION + 1)));
        // And the message names the fix rather than leaking internals.
        let msg = BundleError::UnsupportedVersion(FORMAT_VERSION + 1).to_string();
        assert!(msg.contains("update the app"), "unhelpful message: {msg}");
    }

    #[test]
    fn bad_magic_is_rejected() {
        assert_eq!(unpack(b"not a profile file at all"), Err(BundleError::BadMagic));
        assert_eq!(unpack(b""), Err(BundleError::BadMagic));
    }

    #[test]
    fn non_utf8_name_is_a_clean_error() {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.push(FORMAT_VERSION);
        b.extend_from_slice(&1u32.to_le_bytes());
        b.push(EntryKind::World.as_byte());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&[0xff, 0xfe]);
        b.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(unpack(&b), Err(BundleError::BadName));
    }

    #[test]
    fn filename_carries_the_date_and_extension() {
        assert_eq!(
            profile_filename("2026-09-06"),
            "2026-09-06-axenstax-profile.axeprofile"
        );
    }
}
