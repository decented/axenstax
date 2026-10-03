//! The two files Signet contacts sync keeps (spec §4):
//!
//! - `profile/signet-contacts-grant.json` (0600) — the accepted grant and the
//!   per-grant app secret (D5). Written only after the player pressed Continue
//!   on the code screen (D6).
//! - `profile/signet-contacts.json` (0600) — the last accepted projection,
//!   already parsed and sanitised, plus `fetched_at` and the first time each
//!   pubkey was seen (so `added_at` survives a wholesale replacement, D9).
//!
//! Both formats are append-only (new fields `#[serde(default)]`). A file that
//! does not parse is quarantined the way saves are (`save::quarantine_corrupt`)
//! and treated as absent — never a panic, never silently overwritten.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nostr::{Keys, SecretKey};
use serde::{Deserialize, Serialize};

use crate::signet::contacts_wire::pairing::is_valid_contacts_relay_url;
use crate::signet::contacts_wire::{Ack, Capability, Projection};

pub const GRANT_VERSION: u32 = 1;
pub const SNAPSHOT_VERSION: u32 = 1;

/// Where the two files live.
#[derive(Clone, Debug)]
pub struct StorePaths {
    pub grant: PathBuf,
    pub snapshot: PathBuf,
}

impl StorePaths {
    pub fn in_profile(dir: &Path) -> Self {
        Self {
            grant: dir.join("signet-contacts-grant.json"),
            snapshot: dir.join("signet-contacts.json"),
        }
    }

    /// The real profile directory.
    pub fn default_paths() -> Self {
        Self::in_profile(&crate::data_dir::profile_dir())
    }
}

/// An accepted grant. `Debug` is written by hand so the app secret can never
/// reach a log line.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub v: u32,
    pub app_secret_hex: String,
    pub grant_id: String,
    pub rail_pubkey: String,
    pub projection_tag: String,
    /// The ack's relay — every fetch goes here (D4), never the pairing relay
    /// by assumption.
    pub relay: String,
    pub granted_capabilities: Vec<Capability>,
    pub max_staleness_seconds: u64,
    pub paired_at: u64,
}

impl std::fmt::Debug for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grant")
            .field("app_secret_hex", &"<redacted>")
            .field("grant_id", &self.grant_id)
            .field("rail_pubkey", &self.rail_pubkey)
            .field("relay", &self.relay)
            .field("granted_capabilities", &self.granted_capabilities)
            .field("max_staleness_seconds", &self.max_staleness_seconds)
            .field("paired_at", &self.paired_at)
            .finish()
    }
}

impl Grant {
    /// The grant an accepted ack becomes once the player presses Continue.
    pub fn from_ack(ack: &Ack, app_keys: &Keys, now: u64) -> Self {
        Self {
            v: GRANT_VERSION,
            app_secret_hex: app_keys.secret_key().to_secret_hex(),
            grant_id: ack.grant_id.clone(),
            rail_pubkey: ack.rail_pubkey.clone(),
            projection_tag: ack.projection_tag.clone(),
            relay: ack.relay.clone(),
            granted_capabilities: ack.granted_capabilities.clone(),
            max_staleness_seconds: ack.max_staleness_seconds,
            paired_at: now,
        }
    }

    pub fn app_keys(&self) -> Option<Keys> {
        SecretKey::from_hex(&self.app_secret_hex).ok().map(Keys::new)
    }

    /// Field-level sanity a hand-edited or damaged file can fail.
    fn is_well_formed(&self) -> bool {
        let hex = |s: &str, n: usize| {
            s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        self.v >= 1
            && self.app_keys().is_some()
            && hex(&self.grant_id, 32)
            && hex(&self.rail_pubkey, 64)
            && is_valid_contacts_relay_url(&self.relay)
            && !self.granted_capabilities.is_empty()
    }
}

/// The last accepted projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub projection: Projection,
    /// When this projection was accepted (unix seconds).
    pub fetched_at: u64,
    /// Hex pubkey → unix seconds it first appeared in any accepted
    /// projection. Carried forward across replacements.
    #[serde(default)]
    pub first_seen: BTreeMap<String, u64>,
}

impl Snapshot {
    /// A snapshot for a newly accepted `projection`, carrying `first_seen`
    /// forward from the one it replaces. Pubkeys that left the projection are
    /// dropped (someone removed and re-added is met again).
    pub fn replacing(old: Option<&Snapshot>, projection: Projection, now: u64) -> Self {
        let mut first_seen = BTreeMap::new();
        for c in &projection.contacts {
            for id in c.identities.iter().flatten() {
                let at = old
                    .and_then(|o| o.first_seen.get(&id.pubkey).copied())
                    .unwrap_or(now);
                first_seen.insert(id.pubkey.clone(), at);
            }
        }
        Self { v: SNAPSHOT_VERSION, projection, fetched_at: now, first_seen }
    }
}

/// What a load found.
#[derive(Debug, PartialEq, Eq)]
pub enum Loaded<T> {
    Absent,
    Ok(T),
    /// The file did not parse; it has been moved aside (or could not be, and
    /// is ignored). Treated as absent by every caller.
    Quarantined,
}

impl<T> Loaded<T> {
    pub fn ok(self) -> Option<T> {
        match self {
            Loaded::Ok(t) => Some(t),
            _ => None,
        }
    }
}

fn load_json<T: serde::de::DeserializeOwned>(path: &Path, valid: impl Fn(&T) -> bool) -> Loaded<T> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::Absent,
        Err(e) => {
            log::warn!("[signet-contacts] could not read {}: {e}", path.display());
            return Loaded::Absent;
        }
    };
    match serde_json::from_slice::<T>(&bytes) {
        Ok(t) if valid(&t) => Loaded::Ok(t),
        _ => {
            if let Err(e) = crate::save::quarantine_corrupt(path) {
                log::warn!("[signet-contacts] {e}");
            }
            Loaded::Quarantined
        }
    }
}

pub fn load_grant(path: &Path) -> Loaded<Grant> {
    load_json(path, Grant::is_well_formed)
}

pub fn load_snapshot(path: &Path) -> Loaded<Snapshot> {
    load_json(path, |s: &Snapshot| s.v >= 1)
}

pub fn save_grant(path: &Path, grant: &Grant) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(grant).map_err(|e| format!("serialise grant: {e}"))?;
    write_atomic_0600(path, &json)
}

pub fn save_snapshot(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let json =
        serde_json::to_vec_pretty(snapshot).map_err(|e| format!("serialise snapshot: {e}"))?;
    write_atomic_0600(path, &json)
}

/// Write to a 0600 temp sibling, then rename over the target, so a crash
/// mid-write leaves the old file (or none), never a torn one.
fn write_atomic_0600(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    crate::runtime_identity::write_0600(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))
}

fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove {}: {e}", path.display())),
    }
}

/// Remove a snapshot that belongs to an older grant. If it cannot be removed
/// it is moved aside (`save::quarantine_corrupt`); if even that fails it is
/// logged loudly and left — `book_part_at` ignores a snapshot whose
/// `grant_id` is not the current grant's, so a leftover never counts.
pub fn discard_snapshot(path: &Path) {
    if let Err(e) = remove(path) {
        log::warn!("[signet-contacts] could not remove the old snapshot ({e}); moving it aside");
        if let Err(e) = crate::save::quarantine_corrupt(path) {
            log::error!(
                "[signet-contacts] could not move the old snapshot aside either ({e}); \
                 it is ignored because its grant_id no longer matches"
            );
        }
    }
}

/// Disconnect / revoked: drop the snapshot first (so its contacts stop
/// counting even if the grant removal then fails), then the grant.
pub fn delete_all(paths: &StorePaths) -> Result<(), String> {
    remove(&paths.snapshot)?;
    remove(&paths.grant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signet::contacts_wire::projection::Frontier;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "axenstax-signet-store-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn grant() -> Grant {
        Grant {
            v: GRANT_VERSION,
            app_secret_hex: Keys::generate().secret_key().to_secret_hex(),
            grant_id: "a".repeat(32),
            rail_pubkey: "b".repeat(64),
            projection_tag: "c".repeat(32),
            relay: "wss://relay.example.com".into(),
            granted_capabilities: vec![Capability::ReadDirectory],
            max_staleness_seconds: 21_600,
            paired_at: 7,
        }
    }

    fn projection() -> Projection {
        Projection {
            v: 2,
            grant_id: "a".repeat(32),
            scopes: vec![Capability::ReadDirectory],
            frontier: Frontier { max_clock: 1, op_count: 1, published_at: 1, device_id: "d".repeat(32) },
            issued_at: 1,
            expires_at: 2,
            contacts: Vec::new(),
            revoked: false,
            truncated: false,
        }
    }

    #[test]
    fn grant_and_snapshot_round_trip() {
        let dir = tmp("roundtrip");
        let paths = StorePaths::in_profile(&dir);
        assert_eq!(load_grant(&paths.grant), Loaded::Absent);
        let g = grant();
        save_grant(&paths.grant, &g).unwrap();
        assert_eq!(load_grant(&paths.grant), Loaded::Ok(g));
        let s = Snapshot::replacing(None, projection(), 99);
        save_snapshot(&paths.snapshot, &s).unwrap();
        assert_eq!(load_snapshot(&paths.snapshot), Loaded::Ok(s));
        delete_all(&paths).unwrap();
        assert_eq!(load_grant(&paths.grant), Loaded::Absent);
        assert_eq!(load_snapshot(&paths.snapshot), Loaded::Absent);
    }

    #[cfg(unix)]
    #[test]
    fn the_grant_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("mode");
        let paths = StorePaths::in_profile(&dir);
        save_grant(&paths.grant, &grant()).unwrap();
        let mode = std::fs::metadata(&paths.grant).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_corrupt_file_is_quarantined_not_a_panic() {
        let dir = tmp("corrupt");
        let paths = StorePaths::in_profile(&dir);
        std::fs::write(&paths.grant, b"{not json").unwrap();
        std::fs::write(&paths.snapshot, b"[]").unwrap();
        assert_eq!(load_grant(&paths.grant), Loaded::Quarantined);
        assert_eq!(load_snapshot(&paths.snapshot), Loaded::Quarantined);
        // Moved aside, kept for recovery, and no longer in the way.
        assert!(!paths.grant.exists() && !paths.snapshot.exists());
        let kept: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn a_grant_with_a_bad_field_is_quarantined() {
        let dir = tmp("badfield");
        let paths = StorePaths::in_profile(&dir);
        let mut g = grant();
        g.relay = "http://nope".into();
        std::fs::write(&paths.grant, serde_json::to_vec(&g).unwrap()).unwrap();
        assert_eq!(load_grant(&paths.grant), Loaded::Quarantined);
    }

    #[test]
    fn debug_never_prints_the_app_secret() {
        let g = grant();
        let shown = format!("{g:?}");
        assert!(!shown.contains(&g.app_secret_hex));
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn first_seen_is_carried_across_a_replacement() {
        use crate::signet::contacts_wire::projection::{Identity, ProjectedContact};
        let contact = |pk: &str| ProjectedContact {
            contact_id: "e".repeat(32),
            identities: Some(vec![Identity { pubkey: pk.into(), verification: None }]),
            display_name: None,
            effective_tier: None,
            tier_source: None,
            roles: None,
            contact_methods: None,
            blocked: None,
            checks: None,
        };
        let (a, b) = ("1".repeat(64), "2".repeat(64));
        let mut p1 = projection();
        p1.contacts = vec![contact(&a)];
        let s1 = Snapshot::replacing(None, p1, 100);
        let mut p2 = projection();
        p2.contacts = vec![contact(&a), contact(&b)];
        let s2 = Snapshot::replacing(Some(&s1), p2, 500);
        assert_eq!(s2.first_seen.get(&a), Some(&100));
        assert_eq!(s2.first_seen.get(&b), Some(&500));
        assert_eq!(s2.fetched_at, 500);
    }
}
