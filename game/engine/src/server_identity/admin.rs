#![cfg(not(target_arch = "wasm32"))]
//! Operator admin commands (Track 5).
//!
//! The operator changes a server's live policy (allowlist, sign-in requirement)
//! by signing a command event with the **same Heartwood-held key** that authored
//! the server's attestation. The server verifies the command against its known
//! operator pubkey and applies it — authenticated remote control with no shared
//! password and no box access. The transport that carries the command to the
//! server (a relay subscription) is the owner boundary; this module is the
//! verify + apply core, fully unit-tested.
//!
//! **Audience + replay (Spec 08 §9.0.1).** Every command carries a
//! `["server", <server runtime npub>]` tag naming the one server it is for, so
//! an operator running several servers can't have a command for one replayed
//! to another; the server rejects a missing or foreign tag. Accepted command
//! ids are remembered for the freshness window ([`AdminReplayGuard`]), so the
//! same signed event is applied at most once.

use std::collections::HashMap;
use std::path::Path;

use nostr::{Event, EventBuilder, Kind, NostrSigner, PublicKey, Tag, Timestamp, ToBech32};

/// Admin command event kind (regular event; provisional, like the other
/// server-identity kinds — to be registered in `forgesworn/nips`).
pub const ADMIN_CMD_KIND: u16 = 27422;

/// Default freshness window for an admin command (seconds either side of now).
pub const DEFAULT_ADMIN_SKEW_SECS: u64 = 300;

/// A verified operator instruction to mutate the access policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminCommand {
    WhitelistAdd([u8; 32]),
    WhitelistRemove([u8; 32]),
    BlacklistAdd([u8; 32]),
    BlacklistRemove([u8; 32]),
    /// Immediately disconnect a connected player by npub (Spec B §5).
    KickPlayer([u8; 32]),
    SetRequireSignin(bool),
    // Console settings (Spec B §5) — descriptor / capacity / announce / privacy.
    SetMaxPlayers(u16),
    SetAnnounce(bool),
    SetServerName(String),
    SetAbout(String),
    SetRegion(String),
    /// Privacy level + retention days (values defined by Spec C).
    SetPrivacy(String, u32),
    // Erasure (Spec C §6 — data-subject rights).
    /// Erase all of one player's session records by npub.
    ForgetPlayer([u8; 32]),
    /// Erase the whole session log.
    PurgeAllHistory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminError {
    /// Event signature/id invalid.
    BadSignature,
    /// Signed by someone other than this server's operator.
    NotOperator,
    /// `created_at` outside the freshness window (replay / clock skew).
    Stale,
    /// Wrong kind, missing `cmd` tag, or unrecognised command.
    Malformed,
    /// Missing `server` tag, or it names a different server (audience binding).
    WrongServer,
    /// This exact signed command was already accepted (replay).
    Replayed,
    /// The shared replay record couldn't be locked, read or written. Fails
    /// closed: an unrecorded command could be replayed.
    ReplayGuardIo(String),
}

impl AdminCommand {
    /// Apply this command to a live access policy (in-memory whitelist /
    /// require-signin / blocklist vectors). FLAGGED: no production caller —
    /// the two real wiring sites, `server_main::apply_admin_command_to_files`
    /// (writes the on-disk policy files) and `console_settings::ConsoleSettings::apply`
    /// (in-memory server descriptor), each re-match `AdminCommand` and implement
    /// their own effects rather than calling this. A third parallel
    /// implementation that could silently drift from the other two if the
    /// enum grows a new variant. Exercised only by the tests below.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn apply_to(
        &self,
        whitelist: &mut Vec<[u8; 32]>,
        require_signin: &mut bool,
        blocklist: &mut Vec<[u8; 32]>,
    ) {
        match self {
            AdminCommand::WhitelistAdd(pk) => {
                if !whitelist.contains(pk) {
                    whitelist.push(*pk);
                }
            }
            AdminCommand::WhitelistRemove(pk) => whitelist.retain(|x| x != pk),
            AdminCommand::BlacklistAdd(pk) => {
                if !blocklist.contains(pk) {
                    blocklist.push(*pk);
                }
            }
            AdminCommand::BlacklistRemove(pk) => blocklist.retain(|x| x != pk),
            AdminCommand::SetRequireSignin(b) => *require_signin = *b,
            // Console settings are persisted to console.json, not the access
            // policy — `apply_to` (the in-memory policy) ignores them.
            _ => {}
        }
    }
}

/// Parse the `["cmd", verb, arg]` tag into a command.
fn parse_cmd(event: &Event) -> Option<AdminCommand> {
    let tag = event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == "cmd").then_some(s)
    })?;
    match tag.get(1).map(String::as_str)? {
        "whitelist-add" => PublicKey::parse(tag.get(2)?)
            .ok()
            .map(|pk| AdminCommand::WhitelistAdd(pk.to_bytes())),
        "whitelist-remove" => PublicKey::parse(tag.get(2)?)
            .ok()
            .map(|pk| AdminCommand::WhitelistRemove(pk.to_bytes())),
        "blacklist-add" => PublicKey::parse(tag.get(2)?)
            .ok()
            .map(|pk| AdminCommand::BlacklistAdd(pk.to_bytes())),
        "blacklist-remove" => PublicKey::parse(tag.get(2)?)
            .ok()
            .map(|pk| AdminCommand::BlacklistRemove(pk.to_bytes())),
        "kick" => PublicKey::parse(tag.get(2)?)
            .ok()
            .map(|pk| AdminCommand::KickPlayer(pk.to_bytes())),
        "require-signin" => match tag.get(2)?.as_str() {
            "true" => Some(AdminCommand::SetRequireSignin(true)),
            "false" => Some(AdminCommand::SetRequireSignin(false)),
            _ => None,
        },
        "max-players" => tag.get(2)?.parse::<u16>().ok().map(AdminCommand::SetMaxPlayers),
        "announce" => match tag.get(2)?.as_str() {
            "true" => Some(AdminCommand::SetAnnounce(true)),
            "false" => Some(AdminCommand::SetAnnounce(false)),
            _ => None,
        },
        "server-name" => Some(AdminCommand::SetServerName(tag.get(2)?.clone())),
        "about" => Some(AdminCommand::SetAbout(tag.get(2)?.clone())),
        "region" => Some(AdminCommand::SetRegion(tag.get(2)?.clone())),
        // Privacy arg is "level:days" (e.g. "sessions:30", "none:0").
        "privacy" => {
            let (level, days) = tag.get(2)?.split_once(':')?;
            Some(AdminCommand::SetPrivacy(level.to_string(), days.parse::<u32>().ok()?))
        }
        "forget" => PublicKey::parse(tag.get(2)?)
            .ok()
            .map(|pk| AdminCommand::ForgetPlayer(pk.to_bytes())),
        "purge-history" => Some(AdminCommand::PurgeAllHistory),
        _ => None,
    }
}

/// Sign an admin command with the operator's signer (their Heartwood bunker; a
/// local `Keys` in tests). Mirrors how the attestation is signed. The verb/arg
/// shape is validated up-front so the operator gets immediate feedback instead of
/// the server silently dropping a malformed command.
///
/// `server` is the target server's runtime pubkey; it is bound into the event
/// as `["server", <npub>]` so the command is valid on that server only.
pub async fn sign_admin_command<S: NostrSigner>(
    signer: &S,
    verb: &str,
    arg: &str,
    server: &PublicKey,
) -> Result<Event, String> {
    let recognised = (matches!(
        verb,
        "whitelist-add" | "whitelist-remove" | "blacklist-add" | "blacklist-remove" | "kick" | "forget"
    ) && PublicKey::parse(arg).is_ok())
        || (matches!(verb, "require-signin" | "announce") && matches!(arg, "true" | "false"))
        || (verb == "max-players" && arg.parse::<u16>().is_ok())
        || (matches!(verb, "server-name" | "about" | "region") && !arg.is_empty())
        || (verb == "privacy" && arg.split_once(':').is_some_and(|(_, d)| d.parse::<u32>().is_ok()))
        || verb == "purge-history";
    if !recognised {
        return Err(format!("unrecognised admin command: '{verb} {arg}'"));
    }
    let tag = Tag::parse(["cmd", verb, arg]).map_err(|e| e.to_string())?;
    let server_npub = server.to_bech32().map_err(|e| e.to_string())?;
    let audience = Tag::parse(["server", &server_npub]).map_err(|e| e.to_string())?;
    EventBuilder::new(Kind::Custom(ADMIN_CMD_KIND), "")
        .tags([tag, audience])
        .sign(signer)
        .await
        .map_err(|e| e.to_string())
}

/// The server a command is addressed to (its `["server", …]` tag), if present
/// and a valid npub/hex pubkey.
fn audience(event: &Event) -> Option<PublicKey> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == "server").then(|| PublicKey::parse(&s[1]).ok())?
    })
}

/// Verify an operator admin command: valid signature, authored by `operator`,
/// fresh (`|created_at - now| <= max_skew_secs`), addressed to `server` (this
/// server's runtime pubkey) by its `server` tag, and a recognised command.
/// Replay dedupe is the caller's [`AdminReplayGuard`].
pub fn verify_admin_command(
    event: &Event,
    operator: &PublicKey,
    server: &PublicKey,
    now: Timestamp,
    max_skew_secs: u64,
) -> Result<AdminCommand, AdminError> {
    if event.verify().is_err() {
        return Err(AdminError::BadSignature);
    }
    if event.kind != Kind::Custom(ADMIN_CMD_KIND) {
        return Err(AdminError::Malformed);
    }
    if &event.pubkey != operator {
        return Err(AdminError::NotOperator);
    }
    if event.created_at.as_secs().abs_diff(now.as_secs()) > max_skew_secs {
        return Err(AdminError::Stale);
    }
    if audience(event).as_ref() != Some(server) {
        return Err(AdminError::WrongServer);
    }
    parse_cmd(event).ok_or(AdminError::Malformed)
}

/// Remembers the ids of accepted admin commands for the freshness window, so
/// one signed command is applied at most once. An id can be forgotten once its
/// `created_at + max_skew` has passed: from then on the freshness check alone
/// rejects it. Persisted to a small file so the one-shot `--admin` CLI and the
/// long-running relay listener share one memory across processes and restarts:
/// [`AdminReplayGuard::check_and_record`] reloads, merges, prunes and writes
/// that file under an exclusive lock on every command, so neither process
/// overwrites ids the other recorded.
#[derive(Debug, Default)]
pub struct AdminReplayGuard {
    /// event id (hex) → unix second after which the freshness check covers it.
    seen: HashMap<String, u64>,
}

/// File (in the server identity dir) holding the replay guard's memory.
pub const ADMIN_SEEN_FILE: &str = "admin-seen.json";

impl AdminReplayGuard {
    /// Record `event` as accepted, or refuse it as a replay. Call only after
    /// [`verify_admin_command`] succeeded. Prunes ids past their window.
    pub fn admit(&mut self, event: &Event, now: Timestamp, max_skew_secs: u64) -> Result<(), AdminError> {
        let now = now.as_secs();
        self.seen.retain(|_, until| *until >= now);
        let id = event.id.to_hex();
        if self.seen.contains_key(&id) {
            return Err(AdminError::Replayed);
        }
        self.seen.insert(id, event.created_at.as_secs().saturating_add(max_skew_secs));
        Ok(())
    }

    /// Read the on-disk record in `dir`. A missing file is an empty guard; an
    /// unparseable one is treated as empty too (a corrupt record must not lock
    /// the operator out — the freshness window still bounds any replay).
    fn load(dir: &Path) -> Self {
        let seen = std::fs::read_to_string(dir.join(ADMIN_SEEN_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { seen }
    }

    /// Write the record atomically: a 0600 temp file renamed over the real one.
    fn save(&self, dir: &Path) -> Result<(), String> {
        let json = serde_json::to_string(&self.seen).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("{ADMIN_SEEN_FILE}.tmp"));
        crate::server_identity::store::write_0600(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, dir.join(ADMIN_SEEN_FILE)).map_err(|e| e.to_string())
    }

    /// Check-and-record `event` against the shared record in `dir`, all under
    /// an exclusive lock on the sibling `admin-seen.json.lock` (std
    /// `File::lock` — released on drop, or by the OS if the process dies):
    /// reload from disk, prune expired ids, refuse a repeat, record, write
    /// atomically. Any I/O failure fails closed (`ReplayGuardIo`).
    pub fn check_and_record(
        dir: &Path,
        event: &Event,
        now: Timestamp,
        max_skew_secs: u64,
    ) -> Result<(), AdminError> {
        let io = |e: String| AdminError::ReplayGuardIo(e);
        std::fs::create_dir_all(dir).map_err(|e| io(e.to_string()))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("{ADMIN_SEEN_FILE}.lock")))
            .map_err(|e| io(e.to_string()))?;
        lock.lock().map_err(|e| io(e.to_string()))?;
        let mut guard = Self::load(dir);
        guard.admit(event, now, max_skew_secs)?;
        guard.save(dir).map_err(io)
        // `lock` drops here, after the rename.
    }
}

/// Verify `event` and, if valid, check-and-record it in the replay record
/// shared on disk in `dir` ([`AdminReplayGuard::check_and_record`]). The entry
/// point for the server's apply paths (`--admin` and the relay listener).
pub fn accept_admin_command_in(
    dir: &Path,
    event: &Event,
    operator: &PublicKey,
    server: &PublicKey,
    now: Timestamp,
    max_skew_secs: u64,
) -> Result<AdminCommand, AdminError> {
    let cmd = verify_admin_command(event, operator, server, now, max_skew_secs)?;
    AdminReplayGuard::check_and_record(dir, event, now, max_skew_secs)?;
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag};

    /// The server every test command is addressed to.
    fn srv() -> PublicKey {
        Keys::new(nostr::SecretKey::from_slice(&[0x5e; 32]).unwrap()).public_key()
    }

    async fn signed_cmd_to(
        op: &Keys,
        verb: &str,
        arg: &str,
        created_at: u64,
        server: Option<&str>,
    ) -> Event {
        let mut tags = vec![Tag::parse(["cmd", verb, arg]).unwrap()];
        if let Some(s) = server {
            tags.push(Tag::parse(["server", s]).unwrap());
        }
        EventBuilder::new(Kind::Custom(ADMIN_CMD_KIND), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign(op)
            .await
            .unwrap()
    }

    async fn signed_cmd(op: &Keys, verb: &str, arg: &str, created_at: u64) -> Event {
        signed_cmd_to(op, verb, arg, created_at, Some(&srv().to_bech32().unwrap())).await
    }

    #[tokio::test]
    async fn command_without_a_server_tag_is_rejected() {
        let op = Keys::generate();
        let ev = signed_cmd_to(&op, "require-signin", "true", 1_000, None).await;
        assert_eq!(
            verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300),
            Err(AdminError::WrongServer)
        );
    }

    #[tokio::test]
    async fn command_for_another_server_is_rejected() {
        let op = Keys::generate();
        let other = Keys::generate().public_key().to_bech32().unwrap();
        let ev = signed_cmd_to(&op, "require-signin", "true", 1_000, Some(&other)).await;
        assert_eq!(
            verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300),
            Err(AdminError::WrongServer)
        );
    }

    #[tokio::test]
    async fn replayed_command_is_rejected_and_a_valid_one_accepted() {
        let op = Keys::generate();
        let ev = signed_cmd(&op, "require-signin", "true", 1_000).await;
        let dir = std::env::temp_dir().join(format!("axe_admin_replay_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = Timestamp::from(1_010);
        assert_eq!(
            accept_admin_command_in(&dir, &ev, &op.public_key(), &srv(), now, 300),
            Ok(AdminCommand::SetRequireSignin(true))
        );
        assert_eq!(
            accept_admin_command_in(&dir, &ev, &op.public_key(), &srv(), now, 300),
            Err(AdminError::Replayed)
        );
        // A distinct command (new id) is still accepted.
        let ev2 = signed_cmd(&op, "require-signin", "false", 1_001).await;
        assert_eq!(
            accept_admin_command_in(&dir, &ev2, &op.public_key(), &srv(), now, 300),
            Ok(AdminCommand::SetRequireSignin(false))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replay_guard_persists_across_loads_and_prunes_expired_ids() {
        let dir = std::env::temp_dir().join(format!("axe_admin_seen_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let ev = signed_cmd(&op, "announce", "true", 1_000).await;
        AdminReplayGuard::check_and_record(&dir, &ev, Timestamp::from(1_000), 300).unwrap();
        assert_eq!(
            AdminReplayGuard::check_and_record(&dir, &ev, Timestamp::from(1_100), 300),
            Err(AdminError::Replayed)
        );
        // Past created_at + skew the id is pruned (freshness rejects it anyway).
        let other = signed_cmd(&op, "announce", "false", 2_000).await;
        AdminReplayGuard::check_and_record(&dir, &other, Timestamp::from(2_000), 300).unwrap();
        assert!(!AdminReplayGuard::load(&dir).seen.contains_key(&ev.id.to_hex()), "expired id pruned");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review WSOP: `--admin` and the relay listener each held their own
    /// in-memory copy and overwrote each other's saves. Two independent
    /// "guards" (as two processes would) on one file: an id recorded via A is
    /// rejected via B, and neither drops the other's records.
    #[tokio::test]
    async fn two_guards_on_one_file_share_recorded_ids() {
        let dir = std::env::temp_dir().join(format!("axe_admin_seen2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let op = Keys::generate();
        let (op_pk, now) = (op.public_key(), Timestamp::from(1_000));
        let a_cmd = signed_cmd(&op, "announce", "true", 1_000).await;
        let b_cmd = signed_cmd(&op, "announce", "false", 1_000).await;
        let a = |ev: &Event| accept_admin_command_in(&dir, ev, &op_pk, &srv(), now, 300);
        let b = |ev: &Event| accept_admin_command_in(&dir, ev, &op_pk, &srv(), now, 300);
        assert!(a(&a_cmd).is_ok(), "A records its command");
        assert!(b(&b_cmd).is_ok(), "B records a different command");
        assert_eq!(b(&a_cmd), Err(AdminError::Replayed), "B rejects the id A recorded");
        assert_eq!(a(&b_cmd), Err(AdminError::Replayed), "A's later write kept B's id");
        // Concurrent check-and-record of one id: exactly one thread admits it.
        let c_cmd = signed_cmd(&op, "announce", "true", 1_001).await;
        let wins: usize = std::thread::scope(|sc| {
            let hs: Vec<_> = (0..8)
                .map(|_| sc.spawn(|| AdminReplayGuard::check_and_record(&dir, &c_cmd, now, 300).is_ok()))
                .collect();
            hs.into_iter().map(|h| h.join().unwrap() as usize).sum()
        });
        assert_eq!(wins, 1, "the lock makes check-and-record atomic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn valid_whitelist_add_verifies_and_applies() {
        let op = Keys::generate();
        let target = Keys::generate().public_key();
        let ev = signed_cmd(&op, "whitelist-add", &target.to_hex(), 1_000).await;
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_010), 300).unwrap();
        assert_eq!(cmd, AdminCommand::WhitelistAdd(target.to_bytes()));

        let mut wl = Vec::new();
        let mut require = false;
        cmd.apply_to(&mut wl, &mut require, &mut Vec::new());
        assert_eq!(wl, vec![target.to_bytes()]);
        // Idempotent.
        cmd.apply_to(&mut wl, &mut require, &mut Vec::new());
        assert_eq!(wl.len(), 1);
    }

    #[tokio::test]
    async fn require_signin_command_sets_flag() {
        let op = Keys::generate();
        let ev = signed_cmd(&op, "require-signin", "true", 1_000).await;
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300).unwrap();
        let mut wl = Vec::new();
        let mut require = false;
        cmd.apply_to(&mut wl, &mut require, &mut Vec::new());
        assert!(require);
    }

    #[tokio::test]
    async fn remove_command_drops_entry() {
        let op = Keys::generate();
        let target = Keys::generate().public_key();
        let ev = signed_cmd(&op, "whitelist-remove", &target.to_hex(), 1_000).await;
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300).unwrap();
        let mut wl = vec![target.to_bytes(), [9u8; 32]];
        let mut require = true;
        cmd.apply_to(&mut wl, &mut require, &mut Vec::new());
        assert_eq!(wl, vec![[9u8; 32]]);
    }

    #[tokio::test]
    async fn blacklist_add_then_remove_applies() {
        let op = Keys::generate();
        let target = Keys::generate().public_key();

        let ev = signed_cmd(&op, "blacklist-add", &target.to_hex(), 1_000).await;
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300).unwrap();
        assert_eq!(cmd, AdminCommand::BlacklistAdd(target.to_bytes()));
        let mut bl = Vec::new();
        cmd.apply_to(&mut Vec::new(), &mut false, &mut bl);
        assert_eq!(bl, vec![target.to_bytes()]);
        // Idempotent add.
        cmd.apply_to(&mut Vec::new(), &mut false, &mut bl);
        assert_eq!(bl.len(), 1);

        let ev = signed_cmd(&op, "blacklist-remove", &target.to_hex(), 1_000).await;
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300).unwrap();
        cmd.apply_to(&mut Vec::new(), &mut false, &mut bl);
        assert!(bl.is_empty());
    }

    #[tokio::test]
    async fn settings_commands_parse_and_verify() {
        let op = Keys::generate();
        let cases = [
            ("max-players", "20", AdminCommand::SetMaxPlayers(20)),
            ("announce", "true", AdminCommand::SetAnnounce(true)),
            (
                "server-name",
                "Cool SMP",
                AdminCommand::SetServerName("Cool SMP".into()),
            ),
            ("about", "no griefing", AdminCommand::SetAbout("no griefing".into())),
            ("region", "eu-west", AdminCommand::SetRegion("eu-west".into())),
            (
                "privacy",
                "sessions:30",
                AdminCommand::SetPrivacy("sessions".into(), 30),
            ),
        ];
        for (verb, arg, expected) in cases {
            let ev = signed_cmd(&op, verb, arg, 1_000).await;
            let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300)
                .unwrap_or_else(|e| panic!("verb {verb}: {e:?}"));
            assert_eq!(cmd, expected, "verb {verb}");
        }
    }

    #[tokio::test]
    async fn erasure_commands_parse_and_verify() {
        let op = Keys::generate();
        let target = Keys::generate().public_key();
        let ev = signed_cmd(&op, "forget", &target.to_hex(), 1_000).await;
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300).unwrap();
        assert_eq!(cmd, AdminCommand::ForgetPlayer(target.to_bytes()));

        let ev = signed_cmd(&op, "purge-history", "", 1_000).await;
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300).unwrap();
        assert_eq!(cmd, AdminCommand::PurgeAllHistory);
    }

    #[tokio::test]
    async fn command_from_non_operator_is_rejected() {
        let op = Keys::generate();
        let imposter = Keys::generate();
        let ev = signed_cmd(&imposter, "require-signin", "true", 1_000).await;
        assert_eq!(
            verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300),
            Err(AdminError::NotOperator)
        );
    }

    #[tokio::test]
    async fn tampered_command_is_rejected() {
        let op = Keys::generate();
        let mut ev = signed_cmd(&op, "require-signin", "true", 1_000).await;
        ev.content = "tampered".to_string();
        assert_eq!(
            verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300),
            Err(AdminError::BadSignature)
        );
    }

    #[tokio::test]
    async fn stale_command_is_rejected() {
        let op = Keys::generate();
        let ev = signed_cmd(&op, "require-signin", "true", 1_000).await;
        // now is 10000s later, far outside a 300s window.
        assert_eq!(
            verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(11_000), 300),
            Err(AdminError::Stale)
        );
    }

    #[tokio::test]
    async fn signed_command_round_trips_through_verify() {
        // sign_admin_command must produce an event verify_admin_command accepts.
        let op = Keys::generate();
        let target = Keys::generate().public_key();
        let ev = sign_admin_command(&op, "whitelist-add", &target.to_hex(), &srv())
            .await
            .unwrap();
        let cmd = verify_admin_command(&ev, &op.public_key(), &srv(), ev.created_at, 300)
            .unwrap();
        assert_eq!(cmd, AdminCommand::WhitelistAdd(target.to_bytes()));
    }

    #[tokio::test]
    async fn sign_admin_command_rejects_unknown_verb() {
        let op = Keys::generate();
        assert!(sign_admin_command(&op, "self-destruct", "now", &srv()).await.is_err());
    }

    #[tokio::test]
    async fn unrecognised_command_is_malformed() {
        let op = Keys::generate();
        let ev = signed_cmd(&op, "self-destruct", "now", 1_000).await;
        assert_eq!(
            verify_admin_command(&ev, &op.public_key(), &srv(), Timestamp::from(1_000), 300),
            Err(AdminError::Malformed)
        );
    }
}
