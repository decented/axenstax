//! Every player-facing line of the live world handover, in one place so the
//! wording can be reviewed as a whole. PROPOSED copy: not yet reviewed by the
//! owner. Plain UK English, npub only (never hex), and nothing beyond what is
//! built. CP18: once the import is Activated the world moves into the
//! recipient's world list (`promote`), where it opens and hosts like any
//! other world.

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
        // CP18: once the handover finishes, the world is moved into the
        // world list (`promote::promote_activated_imports`).
        "Once the handover finishes, the world appears in your world list and you can play or host it.".to_string(),
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
/// In place of a HUD dialog (chest, crafting, bounty board, …) while play is
/// paused for a handover: nothing in it can be used until play resumes.
pub const DIALOGS_PAUSED: &str =
    "Play is paused for a world handover. This can't be used until play resumes.";
/// A pause-menu world setting (difficulty, creative, Stash, Try again)
/// pressed while play is paused for a handover.
pub const SETTINGS_PAUSED: &str = "World settings wait while play is paused for a handover.";
pub const COMMAND_PAUSED: &str = "Commands wait while play is paused for a handover.";
pub const NEEDS_ONLINE: &str =
    "Handing a world over needs it hosted online, so there is an invitation to carry over. Use Host online first.";
pub const BUSY: &str = "A handover is already in progress.";
/// L3: a `--no-lend` host keeps a second copy of the world for its server.
pub const NO_LEND: &str =
    "This world is running with a second server copy (--no-lend), so it can't be handed over. Restart it normally first.";
/// L6: the row the host clicked no longer names the same signed-in seat.
pub const SEAT_CHANGED: &str = "That player's seat changed before you clicked, so nothing was started. Choose again.";
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

/// P4: the whole handover ran past its 15-minute limit.
pub const TOO_LONG: &str = "The handover took longer than 15 minutes, so it was cancelled.";
pub const TOO_LONG_SOURCE: &str = "The handover took longer than 15 minutes, so it was cancelled. Play has resumed.";
/// D7 open guard (PROPOSED copy, not owner-reviewed).
pub const RETIRED: &str = "This world was handed over to another player, so it can't be opened or saved here. Its retained copy is a separate world.";
/// CP10: an imported world whose handover hasn't reached Activated.
/// CP11: the world list's line for an import still awaiting its Committed.
pub fn awaiting_handover(npub: &str) -> String {
    format!("Waiting for {npub} to finish handing this world over")
}
/// CP13: the world list's line for a retired world whose handover record
/// this install's ledger doesn't list (planted, or from another computer).
/// It is ignored: no network action, and the world stays closed.
pub fn unlisted_world(name: &str) -> String {
    format!("\"{name}\" holds a handover record this computer didn't make. It is ignored, and the world stays closed.")
}
/// CP14b (CP13b review LOW-5): an unlisted ABORT record: the open guard
/// ignores abort records, so the world isn't closed by it.
pub const UNLISTED_ABORT: &str = "A handover note on this computer can't be checked. It is ignored; the world is unaffected.";
/// CP13: the same for an incoming world's receipt.
pub const UNLISTED_IMPORT: &str = "An incoming world holds a handover record this computer didn't make. It is ignored, and the world stays closed.";
/// CP13 (D2): the world list's line for a quarantined import: it is never
/// opened.
/// CP13b (D2): a readback found both the sender's Committed and its
/// Aborted for one handover; Committed won, so the sender may still hold it.
pub const WORLD_IN_TWO_PLACES: &str = "This world may now exist in two places.";
/// CP13b (D2): the source's abort record couldn't be saved; play resumed.
/// CP14b (CP13b review LOW-3): a recipient past its ack can't cancel, so the
/// line says what is true.
/// CP15: the pending abort record written before the bearer left is still
/// on disk, so the next start of this game publishes the cancellation.
/// CP16 (CP14 review LOW-1): said only when no Aborted went to the other
/// player (no link, or the send failed); `ABORT_UNRECORDED_SENT` otherwise.
/// CP16 (CP15 review MEDIUM-1): no promise of the next start: the room for
/// the record is checked before the bearer leaves, but a save that keeps
/// failing (a full disk) fails at every start too.
pub const ABORT_UNRECORDED: &str =
    "The cancellation couldn't be saved. The other player's game will keep waiting for this handover until this game manages to save it; it tries again each time it starts.";
/// CP16 (CP14 review LOW-1): the abort record couldn't be saved, but the
/// signed cancellation was sent over the link. A recipient that got it
/// stops waiting; only one that didn't keeps waiting.
pub const ABORT_UNRECORDED_SENT: &str =
    "The cancellation couldn't be saved here, but it was sent. If it didn't reach the other player, their game will keep waiting for this handover until this game manages to save it; it tries again each time it starts.";
/// CP16 (CP15 review MEDIUM-1): this world's `handover/` has no room for
/// every record a handover may need (checked before the bearer leaves).
pub const WORLD_RECORDS_FULL: &str =
    "This world has no room for another handover record, so the handover stopped. Nothing was handed over. Play has resumed.";
/// CP15: the pending abort record couldn't be saved before the bearer could
/// leave, so nothing was handed over.
pub const PENDING_ABORT_UNWRITTEN: &str =
    "The handover couldn't be saved on this device, so it stopped. Nothing was handed over. Play has resumed.";
/// CP13b (CP13 review MEDIUM-1): the ledger can't take this handover.
pub const LEDGER_FULL: &str = "This device's handover record is full or can't be read, so the handover stopped. Nothing was handed over.";
/// CP13b (CP13 review LOW-1): a recipient's Cancel after its ack.
pub fn waiting_after_ack(npub: &str) -> String {
    format!("Waiting for {npub} to finish. You can leave; the game will keep checking.")
}
pub const HANDOVER_UNFINISHED: &str = "This handover didn't finish. The world stays with the sender.";
pub const NOT_ACTIVATED: &str = "This world is still arriving from a handover, so it can't be opened or saved yet.";
pub const AUTHORITY_UNREADABLE: &str = "This world's handover record can't be read, so it won't be opened or saved.";
/// CP17 (CP15 review LOW-3): another copy of the game on this computer is
/// in the middle of a handover of this world (it holds the world's
/// handover lock), so this one won't open, host or hand it over.
pub const HANDOVER_ELSEWHERE: &str =
    "Another copy of the game on this computer is handing this world over right now. Try again once that has finished.";

/// CP17 review MEDIUM-1: the world's folder gives no file lock (e.g. some
/// network drives), so a handover can't start; play and saving go on.
pub const NO_FILE_LOCKING: &str =
    "This world's folder doesn't support the file locking a handover needs, so it can't be handed over from here. You can still play it.";

/// CP19 (CP18 review MEDIUM-2): the recipient's import was removed (for
/// example by another copy of the game) before the handover finished; it
/// refuses before acknowledging, so the host keeps its world.
pub const IMPORT_VANISHED: &str =
    "The handed-over world's files went missing on this device before the handover finished, so it was cancelled. The host keeps the world.";

// Slice A (CP7): bearer, Prepared, retirement.
pub const NO_LIVE_INVITE: &str =
    "This world has no live online invitation, so it can't be handed over. Host it online again for a fresh invitation.";
pub const NEVER_SAVED: &str = "Save this world once before handing it over. Play has resumed.";
pub const PREPARED_UNSTORED: &str =
    "The handover record couldn't be stored on the invitation's relays, so the handover was cancelled. Play has resumed.";
pub const RETIREMENT_UNCERTAIN: &str = "This world's handover record may only be partly saved, so the world stays paused and \
     won't reopen here. Its handover record will be checked again.";
pub const WRONG_BEARER: &str =
    "The host's invitation didn't match its handover record, so the handover was stopped. Nothing was activated.";
pub const INVITE_REISSUED: &str =
    "The handover had already shared this world's invitation, so a new one has been made: old invite links no longer let anyone in.";
pub const QUARANTINED: &str = "The handover stopped before it finished. The copy is kept on this machine, untouched, and \
     won't open as a world unless the handover completes.";
pub fn source_finishing(world: &str, npub: &str) -> String {
    format!("Handing \u{201c}{world}\u{201d} to {npub}\u{2026} Keep the game open.")
}
pub fn source_retired(world: &str, npub: &str) -> String {
    format!("\u{201c}{world}\u{201d} now belongs to {npub}. Sending them the final record\u{2026}")
}
pub fn recipient_finishing(world: &str) -> String {
    format!("Finishing the handover of \u{201c}{world}\u{201d}\u{2026} Keep the game open.")
}

/// CP18: an Activated import that can't be moved into the worlds folder
/// because that folder is on a different drive (no copy path).
/// CP20 (CP18 review LOW-6, DECIDED): names no environment variable.
pub fn import_on_other_drive(world: &str) -> String {
    format!(
        "\u{201c}{world}\u{201d} was handed over to you, but it can't be moved into your world list because your worlds folder \
         is on a different drive from the game's data folder. Move one of them so both are on the same drive, then start \
         the game again."
    )
}
/// CP20 (CP18 review LOW-2): an Activated import the open guard refuses
/// (its handover record is damaged), so it is never moved in.
pub fn import_record_damaged(world: &str) -> String {
    format!("\u{201c}{world}\u{201d} was handed over to you, but it can't be opened because its handover record is damaged.")
}
/// CP20 (CP18 review LOW-2): moving an Activated import into the worlds
/// folder failed for another reason; it is tried again.
pub fn import_not_moved_yet(world: &str) -> String {
    format!(
        "\u{201c}{world}\u{201d} was handed over to you, but it couldn't be moved into your world list yet. The game will try again when you come back to the world list."
    )
}
/// CP10: the recipient's world is activated here: the handover finished.
pub fn recipient_activated(world: &str) -> String {
    format!("\"{world}\" is yours now: the handover finished and the world is stored on this device.")
}
