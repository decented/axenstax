//! A link the player tapped outside the game: `axenstax://keep/...` in a message, a browser or
//! My Signet, opening the game on Android (`tools/packaging/android`: a VIEW filter for the
//! scheme and `AxenstaxActivity`, which keeps the newest intent).
//!
//! `android_intent.rs` reads the intent over JNI every [`LINK_POLL`] and consumes it
//! (`setData(null)`, so a link fires once). The text waits here until the game can act on it:
//! **only at the menu**, with no join in flight and no other dialog open. A cold start
//! (still on the splash) and a link tapped mid-game (Playing, Paused, loading) are the same
//! case: held, then handled the first time the player is back at the menu. A player in a
//! world is never dropped out of it. Several held links come out in the order they arrived,
//! one at a time. The link last handed over is remembered, and the same text arriving
//! again (a second tap) is dropped, so it cannot bring the confirm back, or dial a Keep the
//! player has just saved, later.
//!
//! What a held link does is what pasting it does:
//! - a Keep link goes to Join Game's own arm (`keeps::route_join`: sign in first, a first-time
//!   Keep asks, a saved Keep dials);
//! - `axenstax://invite/...` (a friend's invite) opens Add a friend with the text in it, and
//!   any other `axenstax://` text (a server address, an npub) opens Join Game with the text in
//!   it, so the player taps Add or Join. A link from another app never dials a friend or a
//!   server on its own: the Keep confirm's reason applies to them too.
//!
//! Native only. The reader is Android's; everywhere else [`IntentRead::Unavailable`] is all
//! there is and nothing is ever held. Desktop players paste.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::menu::{MenuAction, MenuDialog};

/// How often the intent is read while frames run (the activity may be handed a new intent at
/// any time, and `onNewIntent` cannot call into the engine).
pub const LINK_POLL: Duration = Duration::from_millis(250);

/// Links held at once. The oldest is dropped past this.
pub const MAX_HELD: usize = 4;

/// Longest link taken. A Keep link is under 150 characters; anything longer is not one of ours.
pub const MAX_LINK_CHARS: usize = 2048;

/// What one read of the Activity's intent found. (No `Debug`: a Keep link carries a secret.)
// Only Android's reader finds a link; the unit tests build the others.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub enum IntentRead {
    /// It could not be asked (not Android, or the JVM or Activity is not ready).
    Unavailable,
    /// No link in the intent.
    NoLink,
    /// A link, and whether `setData(null)` took it out of the intent.
    Link { text: String, consumed: bool },
}

/// Where a held link goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchRoute {
    /// A Keep invite: Join Game's arm.
    Keep,
    /// `axenstax://invite/...`, a friend's invite: Add a friend, text filled in.
    FriendInvite,
    /// Any other `axenstax://` text (a server address or an npub): Join Game, text filled in.
    Server,
}

/// Where `text` goes, or `None` when it is not an `axenstax:` link (the manifest only claims that
/// scheme, so this is a guard, not a filter the player sees).
pub fn route(text: &str) -> Option<LaunchRoute> {
    let t = text.trim();
    let starts = |p: &str| t.get(..p.len()).is_some_and(|head| head.eq_ignore_ascii_case(p));
    if !starts("axenstax:") {
        return None;
    }
    // A Keep link first, as everywhere else an `axenstax://` text is read.
    if crate::keeps::is_keep_text(t) {
        Some(LaunchRoute::Keep)
    } else if starts("axenstax://invite/") {
        Some(LaunchRoute::FriendInvite)
    } else {
        Some(LaunchRoute::Server)
    }
}

/// Can a link replace this dialog? The two paste boxes can (the link is what the player would
/// have pasted there); anything else (Create World, Delete, Sign in, the Keep confirm) is the
/// player's business first.
pub fn dialog_takes_link(dialog: &MenuDialog) -> bool {
    matches!(dialog, MenuDialog::None | MenuDialog::JoinDirect { .. } | MenuDialog::AddFriend { .. })
}

/// What else the menu frame is in the middle of. A link waits while any of it is going on: each
/// is a hop or a join that the frame's own actions finish (a Workshop or Trial hop fires on the
/// first lobby frame and would drop the link or be cancelled by it), or one a second join would
/// cut across.
#[derive(Clone, Copy, Default)]
pub struct Busy {
    /// A Join Game through Fray is finding its host.
    pub fray_join: bool,
    /// A seat on a host (`remote_client`).
    pub seated: bool,
    /// An Online-by-contact join or host is being set up.
    pub online: bool,
    /// A world hop is queued: the Workshop (`pending_workshop_for_skin`), a Trial launched from the
    /// lobby (`pending_menu_action`) or an online world about to load (`pending_online_world`).
    pub world_hop: bool,
    /// A world handover is in progress (its consent window is on screen, or the move is under way).
    pub handover: bool,
}

impl Busy {
    pub fn any(self) -> bool {
        self.fray_join || self.seated || self.online || self.world_hop || self.handover
    }
}

/// The links taken from the intent and not yet handled.
#[derive(Default)]
pub struct LaunchLinks {
    held: VecDeque<String>,
    /// The link last handed over. Not held again while it stays the last.
    last_offered: Option<String>,
    /// A link `setData(null)` could not remove: not taken again while it stays the same.
    stuck: Option<String>,
    last_poll: Option<Instant>,
}

impl LaunchLinks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the intent if [`LINK_POLL`] has passed since the last read, and hold what it gives.
    pub fn poll(&mut self, now: Instant, read: impl FnOnce() -> IntentRead) {
        if self.last_poll.is_some_and(|t| now.saturating_duration_since(t) < LINK_POLL) {
            return;
        }
        self.last_poll = Some(now);
        match read() {
            IntentRead::Unavailable => {}
            IntentRead::NoLink => self.stuck = None,
            IntentRead::Link { text, consumed } => {
                if !consumed {
                    if self.stuck.as_ref() == Some(&text) {
                        return;
                    }
                    self.stuck = Some(text.clone());
                }
                self.hold(text);
            }
        }
    }

    /// [`poll`](Self::poll) over this device's intent: Android's JNI read; nothing elsewhere.
    pub fn poll_device(&mut self, now: Instant) {
        #[cfg(target_os = "android")]
        self.poll(now, crate::android_intent::read_launch_link);
        #[cfg(not(target_os = "android"))]
        self.poll(now, || IntentRead::Unavailable);
    }

    fn hold(&mut self, text: String) {
        if text.chars().count() > MAX_LINK_CHARS || route(&text).is_none() {
            log::info!("launch link: dropped (not an axenstax: link, or too long)");
            return;
        }
        if self.last_offered.as_ref() == Some(&text) {
            log::info!("launch link: dropped (the one just handled)");
            return;
        }
        // Only the length is logged: a Keep link carries a secret.
        log::info!("launch link: held ({} chars)", text.len());
        self.held.retain(|x| x != &text);
        self.held.push_back(text);
        while self.held.len() > MAX_HELD {
            self.held.pop_front();
        }
    }

    /// Is a link waiting?
    #[cfg(test)]
    pub fn has_held(&self) -> bool {
        !self.held.is_empty()
    }

    /// The menu frame's turn: if a link is held and the menu can take it (`idle`: at the menu with
    /// no join in flight, and the dialog is [`dialog_takes_link`]), hand it over as Join Game would
    /// have it. A Keep link comes back as `JoinGame(text)` for the game loop's own arm; the others
    /// open their paste box and return `MenuAction::None`. Otherwise it stays held.
    pub fn offer(&mut self, dialog: &mut MenuDialog, idle: bool) -> MenuAction {
        if !idle || !dialog_takes_link(dialog) {
            return MenuAction::None;
        }
        let Some(text) = self.held.pop_front() else { return MenuAction::None };
        self.last_offered = Some(text.clone());
        match route(&text) {
            Some(LaunchRoute::Keep) => MenuAction::JoinGame(text),
            Some(LaunchRoute::FriendInvite) => {
                *dialog = MenuDialog::AddFriend { input: text, error: None };
                MenuAction::None
            }
            Some(LaunchRoute::Server) => {
                *dialog = MenuDialog::JoinDirect { address: text };
                MenuAction::None
            }
            None => MenuAction::None,
        }
    }
}

#[cfg(test)]
#[path = "launch_links_tests.rs"]
mod tests;
