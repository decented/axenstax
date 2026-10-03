//! Kenspeckle contacts import, and the QR chunk transport this spec defines
//! for it (Kenspeckle itself has no QR transport for a roster blob — only for
//! small ceremony tokens).
//!
//! Spec: `docs/foundations/2026-09-05-world-chat.md` §3.1, §3.2, §8.1.
//!
//! Whole-module native gate: the world-chat feature is native-only by design
//! (spec §0 — "the web build carries none of this"), and there is no
//! Kenspeckle blob, no guardian phone, and no config directory to read one
//! from on the web taster in the first place. Unlike `comms.rs` (which stays
//! cross-platform on purpose because it is the safety property), this module
//! is an input boundary with nothing for a WASM build to do with it.
#![cfg(not(target_arch = "wasm32"))]

use crate::comms::Tier;

/// Where a contact came into the book from. Advisory (it drives UI copy and
/// lets a future "forget everyone I met by invite" do the right thing); it is
/// never a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddedVia {
    /// Admitted by, or admitted with, an invite bearer.
    Invite,
    /// Imported from a Signet persona-scoped contacts view (waits upstream).
    Import,
    /// The player pasted an npub.
    Paste,
    /// Came out of a Kenspeckle export.
    Kenspeckle,
}

/// One entry in a player's own address book, after the parse boundary has
/// already stripped everything Kenspeckle carries that isn't one of these
/// fields. Nothing else is retained — see the module docs and the
/// stripping test below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    /// x-only pubkey, hex on the wire, npub for display.
    pub pubkey: [u8; 32],
    /// Absent for a `ken` entry with no display name in the source (the null
    /// case the frozen fixture exists to cover).
    pub display_name: Option<String>,
    pub tier: Tier,
    /// Advisory only — drives UI copy, never a permission. See the doc
    /// comment on `convert_entry` for how (and why only how) this is derived.
    pub is_child: bool,
    /// The contact's per-install signalling key, learned from a verified
    /// rendezvous (online play by contact §2). `None` until they have called
    /// or answered once. Kenspeckle never carries it.
    pub runtime_pubkey: Option<[u8; 32]>,
    pub added_via: AddedVia,
    /// Unix seconds when this contact first entered the book. Preserved across
    /// upserts — it is "when you met", not "when the row was last touched".
    pub added_at: u64,
    /// Unix seconds of the last time this player joined a world **this contact
    /// was hosting** (spec §5.2 step 3 — the `my_servers`-style record, kept on
    /// the person rather than on a server row, because a friend's world is
    /// reached through them and not through an address). `None` until they have
    /// hosted for you. Never written for a contact who joined *your* world:
    /// this is your own history, not a log of who called.
    pub last_joined: Option<u64>,
}

/// The wire shape of one Kenspeckle `KindredEntry`, restricted to the fields
/// this importer keeps. Every other field Kenspeckle puts on an entry
/// (`sharedSecret`, `annotations`, `ownerPubkey`, `bondAssertion`,
/// `provenance`, `corroborations`, `nip05`, `rotation`, `previousPubkeys`,
/// `revoked`, `addedAt`, `verifiedAt`, …) is simply absent from this struct.
/// serde silently ignores unknown JSON fields by default, which is exactly
/// the stripping boundary this type exists to be — do **not** add
/// `#[serde(deny_unknown_fields)]` here: that would reject an entry for
/// carrying a field we don't want, when dropping it is the entire point.
#[derive(serde::Deserialize)]
struct KindredEntry {
    pubkey: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    tier: String,
    /// Only present on `kin` entries, and only sometimes. The single place
    /// `is_child` can be derived from — see `convert_entry`.
    #[serde(default)]
    relationship: Option<String>,
}

/// Every way parsing a Kenspeckle export can fail. Not a security boundary in
/// itself — the AEAD tag is what protects the plaintext — but distinct
/// variants make a bad fixture or a bad key easy to tell apart in a test
/// failure or a log line, which a single `String` error would not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContactsParseError {
    /// Shorter than a nonce + tag could possibly be.
    TooShort,
    /// AEAD decryption/authentication failed — wrong key, or the blob was
    /// tampered with. `chacha20poly1305` deliberately doesn't say which.
    Decrypt,
    /// Decrypted, but the plaintext isn't the expected JSON array shape.
    Json(String),
    /// An entry's `pubkey` isn't 64 lowercase-or-uppercase hex chars.
    BadPubkey(String),
    /// An entry's `tier` isn't one of `"kin" | "kith" | "ken"`.
    BadTier(String),
}

/// Decrypt and parse a Kenspeckle `exportEntriesEncrypted` blob.
///
/// Format (§8.1, verified against Kenspeckle's own source — not what you
/// would guess): XChaCha20-Poly1305 under a raw caller-supplied 32-byte key,
/// laid out `nonce(24) || ciphertext || tag(16)`, no envelope, no KDF. The
/// plaintext is a bare JSON array of entries.
pub fn parse_kenspeckle_export(
    blob: &[u8],
    key: &[u8; 32],
) -> Result<Vec<Contact>, ContactsParseError> {
    use chacha20poly1305::aead::Aead;
    use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};

    const NONCE_LEN: usize = 24;
    const TAG_LEN: usize = 16;
    if blob.len() < NONCE_LEN + TAG_LEN {
        return Err(ContactsParseError::TooShort);
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(chacha20poly1305::Key::from_slice(key));
    let nonce = XNonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| ContactsParseError::Decrypt)?;

    let entries: Vec<KindredEntry> =
        serde_json::from_slice(&plaintext).map_err(|e| ContactsParseError::Json(e.to_string()))?;
    entries.into_iter().map(convert_entry).collect()
}

fn parse_tier(s: &str) -> Option<Tier> {
    match s {
        "kin" => Some(Tier::Kin),
        "kith" => Some(Tier::Kith),
        "ken" => Some(Tier::Ken),
        _ => None,
    }
}

/// `is_child` is **derived**, never carried on the wire — Kenspeckle has no
/// child flag of its own. The only place the fact can come from is a `kin`
/// entry whose `relationship` is `"child"`; every other entry (including a
/// `kith` or `ken` entry, which has no `relationship` field at all) yields
/// `false`. This is a labelling convenience for UI copy, which is why
/// deriving it is acceptable here where deriving a *permission* would not be
/// — nothing downstream reads `is_child` to decide who may hear or speak to
/// whom (`comms::speak_ok`/`hear_ok` take a `Tier`, never a `bool`).
fn convert_entry(e: KindredEntry) -> Result<Contact, ContactsParseError> {
    let pubkey_bytes =
        hex::decode(&e.pubkey).map_err(|_| ContactsParseError::BadPubkey(e.pubkey.clone()))?;
    let pubkey: [u8; 32] = pubkey_bytes
        .try_into()
        .map_err(|_| ContactsParseError::BadPubkey(e.pubkey.clone()))?;
    let tier = parse_tier(&e.tier).ok_or_else(|| ContactsParseError::BadTier(e.tier.clone()))?;
    let is_child = tier == Tier::Kin && e.relationship.as_deref() == Some("child");
    Ok(Contact {
        pubkey,
        display_name: e.display_name,
        tier,
        is_child,
        // Kenspeckle carries neither of these: a runtime key can only come from
        // a verified rendezvous, and an export has no per-entry timestamp we
        // keep (§ the stripping boundary above).
        runtime_pubkey: None,
        added_via: AddedVia::Kenspeckle,
        added_at: 0,
        last_joined: None,
    })
}

/// The persisted form of a contacts book — what would actually be written to
/// disk. Exists mainly so the stripping test has something honest to grep:
/// it round-trips exactly the four `Contact` fields and nothing Kenspeckle
/// also carried, so a hit for `sharedSecret`/`annotations`/etc. in its output
/// would mean those fields leaked past `convert_entry`, not that this
/// function forgot to strip something it never had in the first place.
// Only the test below calls this today — real on-disk persistence for an
// imported book isn't built yet (Phase 4 populates `ServerPlayer.contacts`
// in memory only, via `load_local_book`).
#[allow(dead_code)]
fn to_persisted_json(contacts: &[Contact]) -> String {
    let entries: Vec<serde_json::Value> = contacts
        .iter()
        .map(|c| {
            serde_json::json!({
                "pubkey": hex::encode(c.pubkey),
                "display_name": c.display_name,
                "tier": match c.tier {
                    Tier::Kin => "kin",
                    Tier::Kith => "kith",
                    Tier::Ken => "ken",
                    Tier::Stranger => "stranger",
                },
                "is_child": c.is_child,
                "runtime_pubkey": c.runtime_pubkey.map(hex::encode),
                "added_via": match c.added_via {
                    AddedVia::Invite => "invite",
                    AddedVia::Import => "import",
                    AddedVia::Paste => "paste",
                    AddedVia::Kenspeckle => "kenspeckle",
                },
                "added_at": c.added_at,
                "last_joined": c.last_joined,
            })
        })
        .collect();
    serde_json::Value::Array(entries).to_string()
}

/// `~/.config/axenstax/`'s two contacts files, if `HOME` resolves.
fn config_paths() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let home = std::env::var("HOME").ok()?;
    if home.is_empty() {
        return None;
    }
    let dir = std::path::Path::new(&home).join(".config").join("axenstax");
    Some((dir.join("contacts-key.hex"), dir.join("contacts-export.bin")))
}

/// Best-effort local address book for the local player: the union of the
/// Kenspeckle export (if a guardian has dropped one in the config dir), the
/// player's own `profile/contacts.json` mirror (online play by contact §6) and
/// the Signet contacts snapshot (`profile/signet-contacts.json`, Signet
/// contacts sync D8), with every pubkey Signet marks blocked removed last.
/// Read at join (`hosted_server.rs`, alongside the `charter::comms_level`
/// lookup it sits next to). Any absence or failure on the Kenspeckle side — no
/// key file, no blob, bad key, bad signature-free decrypt, malformed JSON —
/// still yields the other sources, which is safe by construction:
/// `ServerPlayer::tier_of` falls back to `Stranger` for anyone not in it, so a
/// player simply talks to nobody until a real import or invite exists, never
/// the other way round.
///
/// There is no QR-scanning UI in this build — a guardian drops
/// `contacts-key.hex` (64 hex chars, the raw decryption key) and
/// `contacts-export.bin` (the Kenspeckle blob) into the config directory by
/// hand. That is the honest interim path the spec describes (§3.2); a UI that
/// reassembles the `chunk_blob`/`reassemble` QR format below into the same
/// two files is future work, not blocked on anything here.
///
/// This is a READ view. Never `save_mirror` what it returns — that would copy
/// Kenspeckle and Signet rows into the player's own mirror, where a Signet
/// block, re-tier or disconnect could no longer reach them. Write one contact
/// with [`record_in_mirror`] instead.
pub fn load_local_book() -> Vec<Contact> {
    load_local_book_with_blocks().0
}

/// [`load_local_book`] plus the pubkeys Signet blocks, from ONE read of the
/// snapshot (two reads could straddle a snapshot replace). A hosted world
/// needs both: the book to admit from and the blocks to refuse — and to drop
/// anybody already in (D8: a block beats every source).
pub fn load_local_book_with_blocks() -> (Vec<Contact>, Vec<[u8; 32]>) {
    let signet = crate::signet_contacts::book_part_now();
    let blocked = signet.as_ref().map(|p| p.blocked.clone()).unwrap_or_default();
    (assemble_book(load_kenspeckle(), load_mirror(&mirror_path()), signet), blocked)
}

/// The Kenspeckle export from the config dir, or nothing.
fn load_kenspeckle() -> Vec<Contact> {
    let Some((key_path, blob_path)) = config_paths() else {
        return Vec::new();
    };
    let Ok(key_hex) = std::fs::read_to_string(&key_path) else {
        return Vec::new();
    };
    let Ok(key_bytes) = hex::decode(key_hex.trim()) else {
        return Vec::new();
    };
    let Ok(key) = <[u8; 32]>::try_from(key_bytes.as_slice()) else {
        return Vec::new();
    };
    let Ok(blob) = std::fs::read(&blob_path) else {
        return Vec::new();
    };
    parse_kenspeckle_export(&blob, &key).unwrap_or_default()
}

/// What the Signet contacts snapshot contributes to the book (spec
/// `2026-10-01-signet-contacts-sync.md` D8–D10): the contacts it adds (empty
/// when the snapshot is stale) and the pubkeys it blocks (applied even then).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignetPart {
    pub contacts: Vec<Contact>,
    pub blocked: Vec<[u8; 32]>,
}

/// The pure book assembly (D8): Kenspeckle ∪ mirror ∪ Signet via [`upsert`]
/// (closest tier wins, so nothing the player set is loosened), THEN every
/// Signet-blocked pubkey is removed from the book whatever source it came
/// from — a block beats every other source and leaves that person a
/// `Stranger`.
pub fn assemble_book(
    kenspeckle: Vec<Contact>,
    mirror: Vec<Contact>,
    signet: Option<SignetPart>,
) -> Vec<Contact> {
    let mut book = merge_books(kenspeckle, mirror);
    let Some(part) = signet else {
        return book;
    };
    for c in part.contacts {
        upsert(&mut book, c);
    }
    book.retain(|c| !part.blocked.contains(&c.pubkey));
    book
}

/// Fold ONE contact into the player's own mirror at `path` (load the mirror
/// alone → [`upsert`] → save). The only write path for the mirror: it never
/// sees the Kenspeckle or Signet rows, so they can never leak into the file.
pub fn record_in_mirror(path: &std::path::Path, contact: Contact) -> Result<(), String> {
    let mut mirror = load_mirror(path);
    upsert(&mut mirror, contact);
    save_mirror(path, &mirror)
}

/// Record a visit to `contact`'s world in the mirror at `path`, folded in as
/// usual. The only production caller passes the row the join itself built
/// (`online_join`: `Kith`, `AddedVia::Invite` — being let in earns Kith, the
/// same rule as before Signet), so what lands in the mirror is what the join
/// earned, never Signet's own tier (a Signet Kin visited is written as Kith,
/// a Signet block still beats it through [`assemble_book`]).
///
/// Guard for a caller that does not exist yet: a row tagged `Import` (i.e.
/// copied out of the Kenspeckle/Signet book) that the mirror does not hold is
/// written at `Stranger`, so the visit and runtime key are kept without
/// copying an imported tier into the player's own file.
pub fn record_visit(path: &std::path::Path, mut contact: Contact) -> Result<(), String> {
    let mirror = load_mirror(path);
    if find(&mirror, &contact.pubkey).is_none() && contact.added_via == AddedVia::Import {
        contact.tier = Tier::Stranger;
    }
    record_in_mirror(path, contact)
}

// ─── The local mirror (online play by contact, spec §2/§6) ──────────────────
//
// `profile/contacts.json` — the player's own address book, fed by invites and
// pastes today and by the Signet persona-scoped contacts view when that ships
// upstream. It is a LOCAL FILE and nothing else: no directory, no sync, no
// server copy (CLAUDE.md red lines 1 and 3).

/// On-disk format version. Fields are append-only: an older build reading a
/// newer file must degrade, never refuse.
// No production reader of the version tag yet — a future migration is what
// consults it. `#[allow(dead_code)]` matches this file's "no caller yet"
// convention below (see the QR chunk transport section).
#[allow(dead_code)]
pub const MIRROR_VERSION: u32 = 1;

/// `profile/contacts.json` — the same `profile/` tree as the runtime key and
/// the Signet session.
pub fn mirror_path() -> std::path::PathBuf {
    crate::data_dir::profile_dir().join("contacts.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MirrorFile {
    v: u32,
    contacts: Vec<StoredContact>,
}

/// The persisted shape. Pubkeys are hex here and npub only at the display
/// boundary ([[feedback_npub_only_display]] governs what a *person* sees, not
/// what a file holds).
#[derive(serde::Serialize, serde::Deserialize)]
struct StoredContact {
    pubkey: String,
    #[serde(default)]
    display_name: Option<String>,
    tier: Tier,
    #[serde(default)]
    is_child: bool,
    #[serde(default)]
    runtime_pubkey: Option<String>,
    added_via: AddedVia,
    #[serde(default)]
    added_at: u64,
    /// Append-only addition (spec §5.2 step 3). `#[serde(default)]` so a file
    /// written by an older build still loads.
    #[serde(default)]
    last_joined: Option<u64>,
}

/// Read the mirror. Any absence, corruption, or unreadable entry yields an
/// empty book — safe by construction, because an empty book means `Stranger`
/// for everyone and therefore admits nobody.
pub fn load_mirror(path: &std::path::Path) -> Vec<Contact> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_slice::<MirrorFile>(&bytes) else {
        return Vec::new();
    };
    file.contacts
        .into_iter()
        .filter_map(|s| {
            let pubkey = hex::decode(&s.pubkey)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())?;
            let runtime_pubkey = match s.runtime_pubkey {
                Some(h) => Some(
                    hex::decode(&h)
                        .ok()
                        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())?,
                ),
                None => None,
            };
            Some(Contact {
                pubkey,
                display_name: s.display_name,
                tier: s.tier,
                is_child: s.is_child,
                runtime_pubkey,
                added_via: s.added_via,
                added_at: s.added_at,
                last_joined: s.last_joined,
            })
        })
        .collect()
}

/// Write the mirror (0600 on unix — it is a list of who a child knows).
pub fn save_mirror(path: &std::path::Path, contacts: &[Contact]) -> Result<(), String> {
    let file = MirrorFile {
        v: MIRROR_VERSION,
        contacts: contacts
            .iter()
            .map(|c| StoredContact {
                pubkey: hex::encode(c.pubkey),
                display_name: c.display_name.clone(),
                tier: c.tier,
                is_child: c.is_child,
                runtime_pubkey: c.runtime_pubkey.map(hex::encode),
                added_via: c.added_via,
                added_at: c.added_at,
                last_joined: c.last_joined,
            })
            .collect(),
    };
    let json = serde_json::to_vec_pretty(&file).map_err(|e| format!("serialise contacts: {e}"))?;
    // Created 0600, not written and then chmod'd. The old shape left the file
    // on disk at the umask's permissions until the chmod landed — a window in
    // which every account on the machine could read a list of who a child
    // knows — and it threw the chmod's result away, so a failure was silent.
    crate::runtime_identity::write_0600(path, &json)
}

/// Closeness rank, low = closer. Local to this module so `Tier` itself stays
/// un-`Ord` (see the derive comment in `comms.rs`).
fn closeness(t: Tier) -> u8 {
    match t {
        Tier::Kin => 0,
        Tier::Kith => 1,
        Tier::Ken => 2,
        Tier::Stranger => 3,
    }
}

/// Insert `incoming`, or fold it into the existing row for the same pubkey.
///
/// Merge rule, and why: the **closer** tier wins (an invite must never demote
/// somebody you already call Kin), the **earliest** `added_at` wins (it records
/// when you met, not when the row was last touched), a newly-learned
/// `runtime_pubkey` and display name overwrite, `added_via` records the most
/// recent route in, and `last_joined` keeps the LATER of the two (it is the most
/// recent visit, so an older row must never overwrite a newer one).
pub fn upsert(book: &mut Vec<Contact>, incoming: Contact) {
    if let Some(existing) = book.iter_mut().find(|c| c.pubkey == incoming.pubkey) {
        if closeness(incoming.tier) < closeness(existing.tier) {
            existing.tier = incoming.tier;
        }
        if incoming.runtime_pubkey.is_some() {
            existing.runtime_pubkey = incoming.runtime_pubkey;
        }
        if incoming.display_name.is_some() {
            existing.display_name = incoming.display_name;
        }
        existing.is_child |= incoming.is_child;
        existing.added_at = existing.added_at.min(incoming.added_at);
        existing.added_via = incoming.added_via;
        existing.last_joined = existing.last_joined.max(incoming.last_joined);
    } else {
        book.push(incoming);
    }
}

/// The union of the Kenspeckle export and the local mirror, with the mirror
/// folded in on top (so a tier the player set themselves wins over an import).
pub fn merge_books(kenspeckle: Vec<Contact>, mirror: Vec<Contact>) -> Vec<Contact> {
    let mut out = kenspeckle;
    for c in mirror {
        upsert(&mut out, c);
    }
    out
}

/// Look a contact up by pubkey.
pub fn find<'a>(book: &'a [Contact], pubkey: &[u8; 32]) -> Option<&'a Contact> {
    book.iter().find(|c| &c.pubkey == pubkey)
}

/// The tier a book records for `pubkey`, falling back to `Stranger` — the same
/// fail-closed default `ServerPlayer::tier_of` uses.
#[allow(dead_code)]
pub fn tier_of(book: &[Contact], pubkey: &[u8; 32]) -> Tier {
    find(book, pubkey).map(|c| c.tier).unwrap_or(Tier::Stranger)
}

// ─── The QR chunk transport (§3.2, §8.1) — ours, not Kenspeckle's ───
//
// `axenstax-contacts:v1:<n>/<total>:<base64url chunk>`, fixed 1200-byte raw
// chunks before encoding, 1-indexed so what a person reads off a screen
// ("chunk 2 of 5") matches what the wire says.
//
// No production caller yet: there is no QR-scanning UI in this build (see
// `load_local_book`'s doc comment — a guardian drops files by hand for now).
// These are pure, fully tested, and ready for that UI when it lands; the
// items below are `#[allow(dead_code)]` only for that reason, matching the
// rest of this codebase's "no caller yet" convention (e.g. `Tier` before
// this same phase landed).

/// Namespace prefix — deliberately ours, so this is never mistaken for a
/// Kenspeckle-defined format (it isn't one; Kenspeckle has no QR transport
/// for a roster blob at all, only for small ceremony tokens).
#[allow(dead_code)]
const QR_CHUNK_PREFIX: &str = "axenstax-contacts";
#[allow(dead_code)]
const QR_CHUNK_VERSION: &str = "v1";
/// Raw bytes per chunk, before base64url encoding.
#[allow(dead_code)]
pub const QR_CHUNK_RAW_LEN: usize = 1200;

/// Split `blob` into `axenstax-contacts:v1:n/total:<chunk>` lines. Pure and
/// total for any input, including empty (which yields one empty chunk,
/// `1/1`, rather than zero lines — `reassemble` always has something to work
/// with).
#[allow(dead_code)]
pub fn chunk_blob(blob: &[u8]) -> Vec<String> {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let raw_chunks: Vec<&[u8]> = if blob.is_empty() {
        vec![&[][..]]
    } else {
        blob.chunks(QR_CHUNK_RAW_LEN).collect()
    };
    let total = raw_chunks.len();
    raw_chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            format!(
                "{QR_CHUNK_PREFIX}:{QR_CHUNK_VERSION}:{}/{total}:{}",
                i + 1,
                engine.encode(chunk)
            )
        })
        .collect()
}

/// Why a set of chunk lines couldn't be reassembled. Every variant is a
/// distinct thing that can go wrong scanning QR codes off a phone screen —
/// a garbled line, chunks from two different exports mixed together, a
/// missed frame, or the same frame scanned twice.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ReassembleError {
    /// Not `axenstax-contacts:v1:n/total:...` shape at all.
    Malformed(String),
    /// Two lines disagree on how many chunks there are in total — almost
    /// always two different exports' chunks got mixed together.
    InconsistentTotal,
    /// The same chunk index appeared twice.
    DuplicateIndex(u32),
    /// A chunk in `1..=total` never showed up.
    MissingChunk(u32),
    /// A chunk's payload wasn't valid base64url.
    Base64(String),
}

/// Reassemble a set of `chunk_blob` lines back into the original bytes.
///
/// **Order-independent**: the lines may arrive in any order (that's the
/// whole reason each one carries its own index) — a scrambled-but-complete
/// set reassembles correctly. What is rejected is anything that isn't
/// actually a complete, consistent set: a missing index, an inconsistent
/// `total`, or the same index appearing twice (which — since `total` is
/// fixed — always means some *other* index went unrepresented, or that two
/// different scans of the same frame are being treated as chunks of one
/// export when they might not agree).
#[allow(dead_code)]
pub fn reassemble(lines: &[String]) -> Result<Vec<u8>, ReassembleError> {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    if lines.is_empty() {
        return Err(ReassembleError::Malformed(String::new()));
    }

    let mut total: Option<u32> = None;
    let mut parts: std::collections::HashMap<u32, Vec<u8>> = std::collections::HashMap::new();

    for line in lines {
        let mut top = line.splitn(4, ':');
        let (Some(prefix), Some(version), Some(idx_total), Some(payload)) =
            (top.next(), top.next(), top.next(), top.next())
        else {
            return Err(ReassembleError::Malformed(line.clone()));
        };
        if prefix != QR_CHUNK_PREFIX || version != QR_CHUNK_VERSION {
            return Err(ReassembleError::Malformed(line.clone()));
        }
        let mut idx_total_parts = idx_total.splitn(2, '/');
        let (Some(n_str), Some(total_str)) =
            (idx_total_parts.next(), idx_total_parts.next())
        else {
            return Err(ReassembleError::Malformed(line.clone()));
        };
        let n: u32 = n_str
            .parse()
            .map_err(|_| ReassembleError::Malformed(line.clone()))?;
        let this_total: u32 = total_str
            .parse()
            .map_err(|_| ReassembleError::Malformed(line.clone()))?;

        match total {
            None => total = Some(this_total),
            Some(t) if t == this_total => {}
            Some(_) => return Err(ReassembleError::InconsistentTotal),
        }

        let bytes = engine
            .decode(payload)
            .map_err(|e| ReassembleError::Base64(e.to_string()))?;
        if parts.insert(n, bytes).is_some() {
            return Err(ReassembleError::DuplicateIndex(n));
        }
    }

    let total = total.expect("checked non-empty above");
    let mut out = Vec::new();
    for n in 1..=total {
        match parts.remove(&n) {
            Some(bytes) => out.extend_from_slice(&bytes),
            None => return Err(ReassembleError::MissingChunk(n)),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── The frozen Kenspeckle fixture ───

    /// Bytes 0x00..0x1f — the fixture's decryption key. Built inline rather
    /// than stored: this repo gitignores `*.key`, and forcing a test key past
    /// that rule would blunt it for real ones.
    fn fixture_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = i as u8;
        }
        key
    }

    #[test]
    fn frozen_fixture_parses_to_exactly_the_expected_contacts() {
        let blob = include_bytes!("../assets/test/kenspeckle-export.v1.bin");
        let expected_json =
            include_str!("../assets/test/kenspeckle-export.v1.expected.json");
        let expected: Vec<serde_json::Value> = serde_json::from_str(expected_json).unwrap();

        let contacts = parse_kenspeckle_export(blob, &fixture_key()).unwrap();
        assert_eq!(contacts.len(), expected.len());

        for (c, want) in contacts.iter().zip(expected.iter()) {
            assert_eq!(hex::encode(c.pubkey), want["pubkey"].as_str().unwrap());
            assert_eq!(
                c.display_name.as_deref(),
                want["display_name"].as_str()
            );
            let want_tier = want["tier"].as_str().unwrap();
            assert_eq!(parse_tier(want_tier), Some(c.tier));
            assert_eq!(c.is_child, want["is_child"].as_bool().unwrap());
        }

        // The specific shape the fixture was built to cover (see the
        // generator script's comments): one of each tier, a kin/child entry,
        // and a ken with a null display name.
        assert!(contacts.iter().any(|c| c.tier == Tier::Kin && !c.is_child));
        assert!(contacts.iter().any(|c| c.tier == Tier::Kin && c.is_child));
        assert!(contacts.iter().any(|c| c.tier == Tier::Kith));
        assert!(
            contacts
                .iter()
                .any(|c| c.tier == Tier::Ken && c.display_name.is_none())
        );
    }

    #[test]
    fn frozen_fixture_wrong_key_fails_to_decrypt() {
        let blob = include_bytes!("../assets/test/kenspeckle-export.v1.bin");
        let wrong_key = [0xFFu8; 32];
        assert_eq!(
            parse_kenspeckle_export(blob, &wrong_key),
            Err(ContactsParseError::Decrypt)
        );
    }

    // ─── The stripping test — proves something because the fixture
    // deliberately contains every one of these fields. ───

    #[test]
    fn persisted_form_never_carries_the_stripped_fields() {
        let blob = include_bytes!("../assets/test/kenspeckle-export.v1.bin");
        let contacts = parse_kenspeckle_export(blob, &fixture_key()).unwrap();
        let persisted = to_persisted_json(&contacts);

        for banned in ["sharedSecret", "annotations", "note", "ownerPubkey", "PRIVATE NOTE"] {
            assert!(
                !persisted.contains(banned),
                "persisted form leaked {banned:?}: {persisted}"
            );
        }
    }

    // ─── Misc parse-boundary edges ───

    #[test]
    fn too_short_blob_is_rejected_without_touching_crypto() {
        assert_eq!(
            parse_kenspeckle_export(&[0u8; 10], &fixture_key()),
            Err(ContactsParseError::TooShort)
        );
    }

    #[test]
    fn is_child_never_true_off_a_kith_or_ken_entry() {
        // Even if a caller could somehow smuggle relationship="child" onto a
        // kith/ken entry, the tier gate means it still can't set is_child —
        // pinned down as its own test since this is the one property the
        // whole "derived, not carried" design exists to guarantee.
        let e = KindredEntry {
            pubkey: "a".repeat(64),
            display_name: None,
            tier: "kith".to_string(),
            relationship: Some("child".to_string()),
        };
        let c = convert_entry(e).unwrap();
        assert!(!c.is_child);
    }

    // ─── QR chunk transport ───

    #[test]
    fn chunk_and_reassemble_round_trips() {
        let blob: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
        let chunks = chunk_blob(&blob);
        assert!(chunks.len() > 1, "test blob should need multiple chunks");
        let back = reassemble(&chunks).unwrap();
        assert_eq!(back, blob);
    }

    #[test]
    fn chunk_and_reassemble_round_trips_empty_blob() {
        let chunks = chunk_blob(&[]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(reassemble(&chunks).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn reassemble_is_order_independent() {
        let blob: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
        let mut chunks = chunk_blob(&blob);
        // Reverse plus an internal swap — not just a simple reverse, so this
        // doesn't accidentally pass by symmetry.
        chunks.reverse();
        if chunks.len() > 2 {
            chunks.swap(0, 2);
        }
        assert_eq!(reassemble(&chunks).unwrap(), blob);
    }

    #[test]
    fn reassemble_rejects_a_missing_chunk() {
        let blob: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
        let mut chunks = chunk_blob(&blob);
        assert!(chunks.len() > 2);
        chunks.remove(1);
        assert!(matches!(
            reassemble(&chunks),
            Err(ReassembleError::MissingChunk(_))
        ));
    }

    #[test]
    fn reassemble_rejects_an_inconsistent_total() {
        let blob: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
        let mut chunks = chunk_blob(&blob);
        let total = chunks.len();
        assert!(total > 1, "test blob should need multiple chunks");
        // Rewrite line 0's `n/total` segment to claim a different, still
        // well-formed total than every other line agrees on.
        let parts: Vec<&str> = chunks[0].splitn(4, ':').collect();
        let bogus_idx_total = format!("1/{}", total + 1);
        chunks[0] = [parts[0], parts[1], &bogus_idx_total, parts[3]].join(":");
        assert_eq!(reassemble(&chunks), Err(ReassembleError::InconsistentTotal));
    }

    #[test]
    fn reassemble_rejects_a_duplicate_index() {
        let blob: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
        let mut chunks = chunk_blob(&blob);
        assert!(chunks.len() > 1);
        let dup = chunks[0].clone();
        chunks.push(dup);
        assert!(matches!(
            reassemble(&chunks),
            Err(ReassembleError::DuplicateIndex(1))
        ));
    }

    #[test]
    fn reassemble_rejects_malformed_lines() {
        assert!(matches!(
            reassemble(&["not-the-right-format".to_string()]),
            Err(ReassembleError::Malformed(_))
        ));
        assert!(matches!(
            reassemble(&["axenstax-contacts:v2:1/1:aGk".to_string()]),
            Err(ReassembleError::Malformed(_))
        ));
        assert!(matches!(
            reassemble(&[]),
            Err(ReassembleError::Malformed(_))
        ));
    }

    #[test]
    fn chunk_size_is_bounded_by_the_raw_chunk_length() {
        let blob = vec![7u8; QR_CHUNK_RAW_LEN * 3 + 1];
        let chunks = chunk_blob(&blob);
        assert_eq!(chunks.len(), 4);
        assert!(chunks[0].starts_with("axenstax-contacts:v1:1/4:"));
        assert!(chunks[3].starts_with("axenstax-contacts:v1:4/4:"));
    }

    // ─── The mirror (online play by contact, spec §2/§6) ───

    fn c(pk: u8, tier: Tier, via: AddedVia, at: u64) -> Contact {
        Contact {
            pubkey: [pk; 32],
            display_name: Some(format!("P{pk}")),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: via,
            added_at: at,
            last_joined: None,
        }
    }

    #[test]
    fn mirror_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("axe_mirror_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("contacts.json");
        let book = vec![
            Contact {
                runtime_pubkey: Some([0x5a; 32]),
                ..c(1, Tier::Kith, AddedVia::Invite, 1_700_000_000)
            },
            c(2, Tier::Kin, AddedVia::Paste, 1_700_000_001),
        ];
        save_mirror(&path, &book).unwrap();
        assert_eq!(load_mirror(&path), book);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// REGRESSION (whole-branch review, MINOR 8). The mirror used to be written
    /// at the umask's permissions and chmod'd afterwards — a window in which a
    /// list of who a child knows was readable by every account on the machine —
    /// and the chmod's result was discarded, so a failure was silent.
    #[cfg(unix)]
    #[test]
    fn the_mirror_is_owner_only_the_moment_it_exists() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("axe_mirror_mode_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("contacts.json");
        save_mirror(&path, &[c(1, Tier::Kith, AddedVia::Invite, 1_700_000_000)]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the contacts mirror must be owner-only");

        // …and a rewrite over a file an older build left wide open tightens it.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_mirror(&path, &[c(2, Tier::Kin, AddedVia::Paste, 1_700_000_001)]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "an existing world-readable mirror must be tightened");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_last_visit_survives_the_file_and_an_older_row_never_overwrites_it() {
        // Spec §5.2 step 3: the "when did I last play at theirs" stamp lives on
        // the contact. It must round-trip, and a book merged from an older
        // source (an import, a re-paste) must not erase it.
        let dir = std::env::temp_dir().join(format!("axe_mirror_lj_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("contacts.json");
        let visited = Contact {
            last_joined: Some(1_700_000_500),
            ..c(1, Tier::Kith, AddedVia::Invite, 1_700_000_000)
        };
        save_mirror(&path, std::slice::from_ref(&visited)).unwrap();
        let mut book = load_mirror(&path);
        assert_eq!(book[0].last_joined, Some(1_700_000_500));

        upsert(&mut book, c(1, Tier::Kith, AddedVia::Paste, 1_700_000_000));
        assert_eq!(
            book[0].last_joined,
            Some(1_700_000_500),
            "a row with no visit must not wipe the one we have"
        );
        upsert(
            &mut book,
            Contact { last_joined: Some(1_700_009_000), ..c(1, Tier::Kith, AddedVia::Invite, 1) },
        );
        assert_eq!(book[0].last_joined, Some(1_700_009_000), "the later visit wins");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mirror_written_before_last_joined_existed_still_loads() {
        // The file format is append-only: an older build's file must degrade,
        // never refuse (the `MIRROR_VERSION` contract).
        let dir = std::env::temp_dir().join(format!("axe_mirror_old_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("contacts.json");
        std::fs::write(
            &path,
            br#"{"v":1,"contacts":[{"pubkey":"0101010101010101010101010101010101010101010101010101010101010101","display_name":"P1","tier":"kith","is_child":false,"runtime_pubkey":null,"added_via":"paste","added_at":7}]}"#,
        )
        .unwrap();
        let book = load_mirror(&path);
        assert_eq!(book.len(), 1);
        assert_eq!(book[0].last_joined, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_corrupt_mirror_reads_as_empty() {
        let dir = std::env::temp_dir().join(format!("axe_mirror_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_mirror(&dir.join("contacts.json")).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("contacts.json"), b"{not json").unwrap();
        assert!(load_mirror(&dir.join("contacts.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_inserts_then_updates_without_duplicating() {
        let mut book = vec![c(1, Tier::Ken, AddedVia::Kenspeckle, 100)];
        upsert(&mut book, c(2, Tier::Kith, AddedVia::Invite, 200));
        assert_eq!(book.len(), 2);
        // Re-adding #1 at a closer tier upgrades it and records the runtime key,
        // but keeps the ORIGINAL added_at (when you first met them).
        upsert(
            &mut book,
            Contact {
                runtime_pubkey: Some([9u8; 32]),
                ..c(1, Tier::Kith, AddedVia::Invite, 300)
            },
        );
        assert_eq!(book.len(), 2, "same pubkey must not duplicate");
        let one = find(&book, &[1u8; 32]).unwrap();
        assert_eq!(one.tier, Tier::Kith);
        assert_eq!(one.runtime_pubkey, Some([9u8; 32]));
        assert_eq!(one.added_at, 100, "added_at is when you first met them");
    }

    #[test]
    fn upsert_never_loosens_a_tier() {
        // Being handed an invite must not demote somebody you already call Kin.
        let mut book = vec![c(1, Tier::Kin, AddedVia::Kenspeckle, 100)];
        upsert(&mut book, c(1, Tier::Ken, AddedVia::Invite, 200));
        assert_eq!(find(&book, &[1u8; 32]).unwrap().tier, Tier::Kin);
    }

    #[test]
    fn merge_prefers_the_mirror_for_a_pubkey_in_both() {
        let kenspeckle = vec![c(1, Tier::Ken, AddedVia::Kenspeckle, 0)];
        let mirror = vec![Contact {
            runtime_pubkey: Some([3u8; 32]),
            ..c(1, Tier::Kith, AddedVia::Invite, 500)
        }];
        let merged = merge_books(kenspeckle, mirror);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].tier, Tier::Kith, "the closer tier wins");
        assert_eq!(merged[0].runtime_pubkey, Some([3u8; 32]));
    }

    #[test]
    fn merge_is_the_union_of_both_books() {
        let merged = merge_books(
            vec![c(1, Tier::Kin, AddedVia::Kenspeckle, 0)],
            vec![c(2, Tier::Kith, AddedVia::Invite, 1)],
        );
        assert_eq!(merged.len(), 2);
        assert!(find(&merged, &[1u8; 32]).is_some());
        assert!(find(&merged, &[2u8; 32]).is_some());
    }

    #[test]
    fn tier_of_falls_back_to_stranger() {
        let book = vec![c(1, Tier::Kin, AddedVia::Kenspeckle, 0)];
        assert_eq!(tier_of(&book, &[1u8; 32]), Tier::Kin);
        assert_eq!(tier_of(&book, &[9u8; 32]), Tier::Stranger);
    }

    #[test]
    fn kenspeckle_entries_are_tagged_as_such() {
        let blob = include_bytes!("../assets/test/kenspeckle-export.v1.bin");
        let contacts = parse_kenspeckle_export(blob, &fixture_key()).unwrap();
        assert!(
            contacts.iter().all(|c| c.added_via == AddedVia::Kenspeckle),
            "an imported entry must say where it came from"
        );
        assert!(
            contacts.iter().all(|c| c.runtime_pubkey.is_none()),
            "Kenspeckle carries no runtime key — only a rendezvous can supply one"
        );
    }

    fn row(n: u8, tier: Tier, via: AddedVia) -> Contact {
        Contact {
            pubkey: [n; 32],
            display_name: None,
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: via,
            added_at: 5,
            last_joined: None,
        }
    }

    #[test]
    fn writing_the_mirror_never_copies_signet_rows_into_it() {
        let dir = std::env::temp_dir().join(format!("axe_mirror_signet_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("contacts.json");
        save_mirror(&path, &[row(1, Tier::Kith, AddedVia::Paste)]).unwrap();
        // The assembled book holds a Signet Kin; a pasted friend is recorded.
        let signet = SignetPart { contacts: vec![row(2, Tier::Kin, AddedVia::Import)], blocked: vec![] };
        let book = assemble_book(vec![], load_mirror(&path), Some(signet));
        assert_eq!(book.len(), 2);
        record_in_mirror(&path, row(3, Tier::Kith, AddedVia::Paste)).unwrap();
        let mirror = load_mirror(&path);
        assert_eq!(mirror.len(), 2);
        assert!(find(&mirror, &[2; 32]).is_none(), "Signet row leaked into the mirror");
        // Visiting a Signet-only contact keeps the visit, not Signet's tier.
        let mut visited = row(2, Tier::Kin, AddedVia::Import);
        visited.last_joined = Some(99);
        record_visit(&path, visited).unwrap();
        let kept = find(&load_mirror(&path), &[2; 32]).cloned().unwrap();
        assert_eq!(kept.tier, Tier::Stranger);
        assert_eq!(kept.last_joined, Some(99));
        // A visit to someone already in the mirror is folded in as usual.
        let mut again = row(1, Tier::Kith, AddedVia::Paste);
        again.last_joined = Some(100);
        record_visit(&path, again).unwrap();
        assert_eq!(find(&load_mirror(&path), &[1; 32]).unwrap().tier, Tier::Kith);
    }
}
