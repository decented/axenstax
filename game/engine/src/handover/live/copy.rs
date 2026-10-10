//! Every player-facing line of the live world handover, in one place so the
//! wording can be reviewed as a whole. PROPOSED copy: not yet reviewed by the
//! owner. Plain UK English, npub only (never hex), and nothing beyond what is
//! built: this version stops once the recipient has stored its copy, so no
//! line says the recipient becomes the host or that the world opens there.

/// The host action, one per signed-in player who is here now.
pub fn hand_to(label: &str) -> String {
    format!("Hand this world to {label}\u{2026}")
}

pub const CONSENT_TITLE: &str = "Take over this world?";

/// The consent screen, line by line: the world, who offers it, what moves,
/// what may not, and where the copy is kept.
pub fn consent_lines(world: &str, source_npub: &str) -> [String; 5] {
    [
        format!("{source_npub} wants to hand the world \u{201c}{world}\u{201d} over to you."),
        "If you accept, a copy of the world is stored on this machine. Play pauses for everyone while it is copied."
            .to_string(),
        "Blocks, buildings, containers, carts, tamed animals and your own inventory come across. Loose dropped items, \
         arrows and other things in flight, and wild animals and monsters may not."
            .to_string(),
        "Nobody's sign-in, contacts, wallet or private Plans are copied.".to_string(),
        // Temporary until activation ships: say what this version does.
        "This version stores the copy but can't open it as your world yet.".to_string(),
    ]
}

pub const ACCEPT: &str = "Accept and store the copy";
pub const DECLINE: &str = "Decline";
pub const CANCEL: &str = "Cancel handover";

pub fn source_asking(world: &str, npub: &str) -> String {
    format!("Asking {npub} to accept \u{201c}{world}\u{201d}\u{2026}")
}
pub fn source_copying(world: &str, npub: &str) -> String {
    format!("Play is paused while \u{201c}{world}\u{201d} is copied to {npub}.")
}
pub fn source_stored(world: &str, npub: &str) -> String {
    format!(
        "{npub} has stored a copy of \u{201c}{world}\u{201d}. Finishing a handover isn't available in this version \
         yet: cancel to carry on playing."
    )
}
pub fn recipient_copying(world: &str, npub: &str) -> String {
    format!("Copying \u{201c}{world}\u{201d} from {npub}\u{2026} Play is paused for everyone.")
}
pub fn recipient_stored(world: &str) -> String {
    format!(
        "A copy of \u{201c}{world}\u{201d} is stored on this machine. Finishing the handover isn't available in this \
         version yet."
    )
}

pub const PAUSED_BANNER: &str = "The host has paused play to hand this world over.";
pub const RESUMED: &str = "Play has resumed.";
pub const CANCELLED: &str = "Handover cancelled. Play has resumed.";
pub const DECLINED: &str = "You declined the handover.";
pub const LEFT: &str = "Leaving the world cancelled the handover.";
/// Shown as "Couldn't save: <this>. Your last save is safe."
pub const SAVE_PAUSED: &str = "play is paused for a handover; cancel it first";
pub const COMMAND_PAUSED: &str = "Commands wait while play is paused for a handover.";
pub const NEEDS_ONLINE: &str =
    "Handing a world over needs it hosted online, so there is an invitation to carry over. Use Host online first.";
pub const BUSY: &str = "A handover is already in progress.";
pub const UNVERIFIED_HOST: &str =
    "This world's host offered to hand it over, but this connection didn't verify who the host is, so it was declined.";
pub const NOT_READY_HERE: &str = "This machine isn't signed in for online play, so the handover was declined.";
/// What the host is shown when the selected player says no, by reason.
pub fn host_declined(reason: super::wire::DeclineReason) -> &'static str {
    use super::wire::DeclineReason as R;
    match reason {
        R::Player => "The other player declined the handover. Play has resumed.",
        R::Busy => "The other player is already in another handover. Play has resumed.",
        R::UnverifiedHost => {
            "The other player can't check who is hosting \u{2014} they need to join through the online invite to \
             receive this world. Play has resumed."
        }
        R::NotReady => "The other player isn't signed in for online play, so they can't receive this world.",
        R::Invalid => "The other player couldn't verify this offer against the host they joined, so they declined it.",
    }
}
pub const RESUME_FAILED: &str = "The handover was cancelled, but the connection couldn't carry on. Join the world again.";
