//! Keeps: the `axenstax://keep/<host key>/<secret>` invite link, the return pass a
//! Keep gives a player who joined its world by room code, and **My Keeps**, the list
//! of Keeps the player has been let into (saved in `keeps.json` beside
//! `my_servers.json`).
//!
//! The game-neutral parts (the link format, finding a link in pasted text, the saved
//! list's file rules and the rule for believing a pass) are the shared `fray-keep`
//! crate's. What is here is AxeNStax's own: the `axenstax://` routing (a Keep link is
//! a third use of the scheme beside `axenstax://invite/2` and the server connect
//! strings), the pass the client holds until the game saves it, and the words.
//!
//! Native only, like `invite.rs` and `fray_play.rs`: the web build is the anonymous
//! local taster and carries no Keeps. `tools/smoke/forbidden-symbol.mjs` greps its
//! bundle for `axenstax://keep` to prove it.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use decented_net::FailReason;
pub use fray_keep::{Invite, Keep, Keeps};

use crate::protocol::{self, KeepPass};

/// Where a Keep's invite link starts.
pub const LINK_PREFIX: &str = "axenstax://keep/";

/// The saved Keeps' file, in the data root beside `my_servers.json`.
pub const FILE: &str = "keeps.json";

/// Most passes one connection to a Keep may have accepted, for the life of the
/// connection. A Keep sends one; a host reached by room code could send a stream, each
/// a new secret, and every accepted pass is a write of `keeps.json` on the game thread.
pub const MAX_PASSES_PER_CONNECTION: usize = 4;

/// What Join Game says to a signed-out player who pasted a Keep link. A Keep's world
/// refuses guests, so nothing is dialled.
pub const SIGN_IN_FIRST: &str = "Sign in first to join a Keep.";

/// A text that starts like a Keep link but is not a whole one.
pub const NOT_A_KEEP_LINK: &str = "That isn't a Keep invite link.";

/// A Keep that is not found, never answers or turns the joiner away looks the same
/// from here (it says nothing to an invite it does not honour), so they share this.
pub const UNREACHABLE: &str = "Couldn't reach that Keep. The invite may have been used, expired or revoked, or the Keep is offline.";

/// What Add a friend says to a pasted Keep link.
pub const NOT_A_FRIEND_LINK: &str = "That's a Keep invite link. Paste it into Join Game instead.";

/// The question before a Keep that is not in My Keeps is dialled for the first time.
pub const CONFIRM_TITLE: &str = "Join this Keep?";

/// What the confirm says under its title.
pub const CONFIRM_BODY: &str = "Only join a Keep from someone you trust. It will see who you are.";

/// The second tap of Forget.
pub const FORGET_CONFIRM: &str = "Forget this Keep? You'll need a new invite to return.";

/// The heading of the block in the friends column (hidden while the list is empty).
pub const MY_KEEPS: &str = "My Keeps";

/// How a Keep is named before it has told us its own name: the crate's spelling,
/// `Keep ab12cd`, the same one My Keeps shows. The confirm, the joining notice and
/// the failure sentences all use it.
pub fn label(host_key: &[u8; 32]) -> String {
    fray_keep::display_name(None, host_key)
}

/// The saved-Keeps file.
pub fn path() -> PathBuf {
    crate::data_dir::data_root().join(FILE)
}

/// The saved Keeps, newest first. A file that cannot be read gives an empty list (and
/// the log says why); nothing is ever saved over it.
pub fn load_list(path: &Path) -> Vec<Keep> {
    match Keeps::load(path) {
        Ok(keeps) => keeps.list().to_vec(),
        Err(e) => {
            log::warn!("my keeps: could not read the saved Keeps: {e}");
            Vec::new()
        }
    }
}

/// Is the player signed in for a Keep? A Keep's world refuses guests, so this needs a stored
/// Signet session as well as the cached npub: the cache alone can outlive a session that
/// is gone or damaged, and a link would then be dialled only to be turned away after the
/// Keep has seen the secret. A session file that cannot be read counts as no session.
pub fn signed_in_with(identity: &Path, session: &Path) -> bool {
    use crate::signet::native_signer::{load_identity_from, load_session_from};
    load_identity_from(identity).npub().is_some() && matches!(load_session_from(session), Ok(Some(_)))
}

/// [`signed_in_with`] over the player's own files.
pub fn signed_in() -> bool {
    use crate::signet::native_signer::{identity_path, session_path};
    signed_in_with(&identity_path(), &session_path())
}

/// Seconds since the Unix epoch.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ─── What a Join Game text asks for ─────────────────────────────────────────

/// Does `text` start like a Keep link (`axenstax://keep/`, or the prefix cut short as
/// `axenstax://keep`, in any case)? Such a text is never a server address.
pub fn is_keep_text(text: &str) -> bool {
    first_keep_word(text).is_some()
}

/// The word of `text` that is, or begins, a Keep link; `None` for a ws/wss address or
/// an `axenstax://` string of another kind (`invite/2`, an npub, a host and port),
/// which keep their own paths whatever else the text says.
fn first_keep_word(text: &str) -> Option<&str> {
    let t = text.trim();
    let starts = |p: &str| t.get(..p.len()).is_some_and(|head| head.eq_ignore_ascii_case(p));
    if starts("ws://") || starts("wss://") {
        return None;
    }
    if starts("axenstax://") {
        // The text opens with the scheme: it is a Keep link only if its first word is one.
        let first = t.split_whitespace().next().unwrap_or("");
        let first = first.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        return fray_keep::is_link_word(LINK_PREFIX, first).then(|| fray_keep::invite_in_text(LINK_PREFIX, t)).flatten();
    }
    fray_keep::invite_in_text(LINK_PREFIX, t)
}

/// What Join Game does with the text it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinRoute {
    /// Not a Keep link: the room code, server and friend-invite paths take it as before.
    NotKeep,
    /// A Keep link that is not a whole one.
    BadLink,
    /// A Keep link, but the player is signed out: say so, dial nothing.
    SignInFirst,
    /// A Keep that is not in My Keeps: ask before dialling.
    Confirm(Invite),
    /// A Keep already in My Keeps: dial it.
    Join(Invite),
}

/// Route a Join Game text. `signed_in`: the player has a Signet session (a Keep's world
/// refuses guests). `saved`: the Keep is already in My Keeps.
pub fn route_join(text: &str, signed_in: bool, saved: impl Fn(&[u8; 32]) -> bool) -> JoinRoute {
    let Some(word) = first_keep_word(text) else { return JoinRoute::NotKeep };
    let Some(invite) = fray_keep::parse_invite_link(LINK_PREFIX, word) else { return JoinRoute::BadLink };
    if !signed_in {
        JoinRoute::SignInFirst
    } else if saved(&invite.host_key) {
        JoinRoute::Join(invite)
    } else {
        JoinRoute::Confirm(invite)
    }
}

/// How an invite join failed, in the player's words: a Keep that is not found, never
/// answers or does not let the joiner in are one sentence ([`UNREACHABLE`]); anything
/// else (no relay, a different version, a host that is not the Keep named) keeps its
/// own text.
pub fn invite_failure_text(reason: &FailReason, label: &str) -> String {
    match reason {
        FailReason::NoAnswer | FailReason::NotAdmitted | FailReason::NoLinkAnswer => UNREACHABLE.to_string(),
        other => crate::lan_host::join_failure_notice(label, &other.to_string()),
    }
}

// ─── The return pass ────────────────────────────────────────────────────────

/// What became of a pass the Keep sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Held, to be saved.
    Kept,
    /// Already held.
    Repeated,
    /// This connection has had four passes accepted already: dropped.
    Full,
    /// Names a Keep other than the one the link proved (or none was proved): dropped.
    OtherHost,
    /// Not a whole pass.
    Unreadable,
}

/// Passes a Keep sent this connection. One inbox lives as long as its connection: at
/// most [`MAX_PASSES_PER_CONNECTION`] passes are accepted in all, whatever the game
/// does between frames (taking the held ones does not reopen the count), so a host that
/// streams passes costs at most that many saves of `keeps.json`.
#[derive(Default)]
pub struct PassInbox {
    /// Accepted and not yet taken, to be saved.
    held: Vec<KeepPass>,
    /// Every pass accepted on this connection, taken or not. Never emptied.
    accepted: Vec<KeepPass>,
    /// The drop after the cap has been logged (once: a stream must not flood the log).
    full_logged: bool,
}

impl PassInbox {
    /// Take in one tag-78 payload. A pass is believed only when it names the host key the
    /// transport proved ([`fray_keep::accept_pass`]); otherwise any room could fill My
    /// Keeps with Keeps it does not run. The log names a key by its first six hex
    /// characters and never shows the secret.
    pub fn offer(&mut self, proven: Option<[u8; 32]>, payload: &[u8]) -> Offer {
        let Ok(pass) = protocol::safe_deserialize::<KeepPass>(payload) else {
            log::warn!("[client] a Keep pass that could not be read was dropped");
            return Offer::Unreadable;
        };
        if !fray_keep::accept_pass(proven, &pass.host_key) {
            log::warn!("[client] a Keep pass naming Keep {} was ignored: it is not the host this link proved", fray_keep::describe(&pass.host_key));
            return Offer::OtherHost;
        }
        if self.accepted.contains(&pass) {
            Offer::Repeated
        } else if self.accepted.len() >= MAX_PASSES_PER_CONNECTION {
            if !self.full_logged {
                self.full_logged = true;
                log::warn!(
                    "[client] Keep {} sent more than {MAX_PASSES_PER_CONNECTION} return passes on one connection; the rest are dropped",
                    fray_keep::describe(&pass.host_key)
                );
            }
            Offer::Full
        } else {
            log::info!("[client] a return pass from Keep {} was received", fray_keep::describe(&pass.host_key));
            self.accepted.push(pass.clone());
            self.held.push(pass);
            Offer::Kept
        }
    }

    /// Hand over the held passes (the game saves them), leaving none.
    pub fn take(&mut self) -> Vec<KeepPass> {
        std::mem::take(&mut self.held)
    }
}

// ─── Saving ─────────────────────────────────────────────────────────────────

/// Save the Keeps `passes` name in the file at `path`, with the host's name. Returns how
/// many changed the file. A Keep that cannot be saved is logged and does not stop the game.
pub fn save_passes(path: &Path, passes: &[KeepPass], host_name: Option<&str>, now: u64) -> usize {
    let mut saved = 0;
    if passes.is_empty() {
        return saved;
    }
    let mut keeps = match Keeps::load(path) {
        Ok(keeps) => keeps,
        Err(e) => {
            log::warn!("my keeps: could not read the saved Keeps, so the pass was not saved: {e}");
            return saved;
        }
    };
    for pass in passes {
        saved += usize::from(record(&mut keeps, &pass.host_key, &pass.secret, host_name, now));
    }
    saved
}

/// Save a Keep the player was let into by its invite (a link, or a saved Keep tapped in
/// My Keeps) once seated. Returns whether the file changed.
pub fn save_invite_join(path: &Path, invite: &Invite, host_name: Option<&str>, now: u64) -> bool {
    match Keeps::load(path) {
        Ok(mut keeps) => record(&mut keeps, &invite.host_key, &invite.secret, host_name, now),
        Err(e) => {
            log::warn!("my keeps: could not read the saved Keeps, so the Keep was not saved: {e}");
            false
        }
    }
}

fn record(keeps: &mut Keeps, host_key: &[u8; 32], secret: &[u8; 32], host_name: Option<&str>, now: u64) -> bool {
    match keeps.record_join(host_key, secret, host_name, now) {
        Ok(true) => {
            log::info!("my keeps: saved Keep {}", fray_keep::describe(host_key));
            true
        }
        Ok(false) => false,
        Err(e) => {
            log::warn!("my keeps: could not save Keep {}: {e}", fray_keep::describe(host_key));
            false
        }
    }
}

/// One frame of My Keeps' learning from a connection: save the return passes the Keep sent,
/// and, for an invite join (`pending`), the Keep itself once the player is seated. `pending`
/// is cleared when the join is seated or has failed.
pub fn settle(client: &mut crate::remote_client::RemoteClient, pending: &mut Option<Invite>, path: &Path, now: u64) {
    use crate::remote_client::ConnectionState;
    let proven = client.proven_host();
    let host_name = proven.as_ref().map(|host| host.name.as_str());
    let passes = client.take_keep_passes();
    save_passes(path, &passes, host_name, now);
    let Some(invite) = *pending else { return };
    if client.is_connected() {
        // Seated. Fray refused any host but the Keep named, so this holds; the check keeps a
        // Keep from being saved under a name the link did not prove.
        if proven.as_ref().is_some_and(|host| host.key == invite.host_key) {
            save_invite_join(path, &invite, host_name, now);
        }
        *pending = None;
    } else if matches!(client.state, ConnectionState::Failed(_) | ConnectionState::Disconnected) {
        *pending = None;
    }
}

/// Take a Keep out of the saved list. `Ok(true)` if it was there.
pub fn forget(path: &Path, host_key: &[u8; 32]) -> bool {
    match Keeps::load(path).and_then(|mut keeps| keeps.forget(host_key)) {
        Ok(was_there) => was_there,
        Err(e) => {
            log::warn!("my keeps: could not forget Keep {}: {e}", fray_keep::describe(host_key));
            false
        }
    }
}

// ─── The My Keeps block ─────────────────────────────────────────────────────

/// One row of the block: what to show and the key it stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub host_key: String,
    pub name: String,
    pub when: String,
}

/// The rows for `list` as of `now`, in the list's order (newest play first).
pub fn rows(list: &[Keep], now: u64) -> Vec<Row> {
    list.iter().map(|k| Row { host_key: k.host_key.clone(), name: k.name.clone(), when: fray_keep::ago(k.last_joined, now) }).collect()
}

/// Forget needs a second tap on the same row: the first arms it (and disarms any other),
/// the second, while still armed, confirms. Returns whether this tap confirmed.
pub fn tap_forget(armed: &mut Option<String>, host_key: &str) -> bool {
    if armed.as_deref() == Some(host_key) {
        *armed = None;
        true
    } else {
        *armed = Some(host_key.to_string());
        false
    }
}

#[cfg(test)]
#[path = "keeps_tests.rs"]
mod tests;
