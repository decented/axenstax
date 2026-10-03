//! Guardian copy — a child's own conversation, sent to their guardian's key.
//!
//! What this is, precisely: when a guardian has turned it on, **the child's own
//! client** sends both directions of **the child's own conversation** to the
//! guardian's key as NIP-17, over the family's own relays.
//!
//! Three boundaries, all deliberate:
//!
//! - **Child → guardian only.** No room, no keeper, nothing AxeNStax operates in
//!   the middle. Nothing is logged anywhere we run (world-chat spec §3.4, and
//!   red line 3 in `CLAUDE.md`).
//! - **The child's own conversation, not the world's.** What they said, and what
//!   was said to them. A copy is a record of the child's exposure — not
//!   surveillance of everyone else who happens to be in the world. Other
//!   players have not consented to anything and must not be swept up.
//! - **The child always knows.** A persistent, non-dismissable indicator while
//!   copy is on. A child who does not know they are being copied is being
//!   surveilled; a child who knows is being parented. This is a requirement, not
//!   a nicety, and it is also what makes the feature defensible.
//!
//! The sending half is native-only (it needs the mailbox's NIP-17 primitive and
//! a tokio runtime). Everything here that shapes a payload is pure and compiled
//! everywhere, so it is tested on both targets.

// BRIDGE: blanket module allow — the payload shaping and batching are written
// and tested, but the send path (NIP-17 via native_mailbox::wire::wrap_report)
// and the HUD indicator are NOT wired up in this build. Nothing calls this yet.
// Remove the blanket allow when Phase 5 wires it; leaving it after that would
// hide genuinely dead code.
#![allow(dead_code)]

/// Which way a copied line went, from the child's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The child said it.
    Said,
    /// Somebody said it to the child.
    Heard,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Said => "said",
            Direction::Heard => "heard",
        }
    }
}

/// One line of the child's conversation, as it will be copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedLine {
    pub direction: Direction,
    /// The other party's npub-able pubkey, hex. For `Said`, who it went to; for
    /// `Heard`, who it came from.
    pub counterpart: String,
    /// Their display name at the time, for a guardian's readability. Never
    /// trusted for identity — the pubkey is the identity.
    pub counterpart_name: String,
    pub text: String,
    /// Server tick, so a guardian sees ordering without us shipping a clock.
    pub at_tick: u64,
}

/// How many lines to gather before sending one batch.
///
/// Batching is not an optimisation for its own sake: one relay event per chat
/// line would make a child's typing rhythm visible to anyone watching relay
/// traffic, and would put a burst of events on somebody else's relay every time
/// a ten-year-old gets excited.
pub const BATCH_LINES: usize = 20;

/// A pending batch. Flushed when it fills, and on leaving the world — a copy
/// that only arrives when the buffer happens to fill is a copy with gaps.
#[derive(Clone, Debug, Default)]
pub struct CopyBuffer {
    lines: Vec<CopiedLine>,
}

impl CopyBuffer {
    pub fn new() -> Self {
        CopyBuffer { lines: Vec::new() }
    }

    /// Add a line. Returns the batch to send if this filled it.
    pub fn push(&mut self, line: CopiedLine) -> Option<Vec<CopiedLine>> {
        self.lines.push(line);
        if self.lines.len() >= BATCH_LINES {
            Some(std::mem::take(&mut self.lines))
        } else {
            None
        }
    }

    /// Take whatever is pending. Call on world exit.
    pub fn flush(&mut self) -> Vec<CopiedLine> {
        std::mem::take(&mut self.lines)
    }

    pub fn pending(&self) -> usize {
        self.lines.len()
    }
}

/// The body of one copy batch: plain, readable text, because a guardian is the
/// reader and they should not need a tool to make sense of it.
///
/// Deliberately not JSON. A parent opening this in whatever NIP-17 client they
/// use should be able to read it directly; a machine-readable format would
/// serve us and not them.
pub fn build_copy_body(world: &str, lines: &[CopiedLine]) -> String {
    let mut out = String::new();
    out.push_str(&format!("World chat copy — {world}\n\n"));
    for line in lines {
        out.push_str(&format!(
            "[{}] {} {}: {}\n",
            line.at_tick,
            line.direction.as_str(),
            line.counterpart_name,
            line.text
        ));
    }
    out
}

/// The indicator text the child sees, permanently, while copy is on.
///
/// Worded to a child, not to a lawyer: it says what happens and who sees it, in
/// words a ten-year-old reads without help, and it does not apologise for it.
pub const COPY_INDICATOR: &str = "Your grown-up sees this chat";

#[cfg(test)]
mod tests {
    use super::*;

    fn line(direction: Direction, text: &str, tick: u64) -> CopiedLine {
        CopiedLine {
            direction,
            counterpart: "a".repeat(64),
            counterpart_name: "Pal".to_string(),
            text: text.to_string(),
            at_tick: tick,
        }
    }

    #[test]
    fn a_batch_flushes_when_it_fills() {
        let mut buf = CopyBuffer::new();
        for i in 0..(BATCH_LINES - 1) {
            assert!(buf.push(line(Direction::Said, "hi", i as u64)).is_none());
        }
        let batch = buf
            .push(line(Direction::Said, "last", 99))
            .expect("the batch should flush when full");
        assert_eq!(batch.len(), BATCH_LINES);
        assert_eq!(buf.pending(), 0, "buffer is emptied by a flush");
    }

    /// A copy that only arrives when the buffer happens to fill is a copy with
    /// gaps — the common case is a child playing for ten minutes and saying
    /// three things.
    #[test]
    fn a_partial_batch_is_still_flushed_on_exit() {
        let mut buf = CopyBuffer::new();
        buf.push(line(Direction::Said, "bye", 1));
        buf.push(line(Direction::Heard, "see you", 2));
        let batch = buf.flush();
        assert_eq!(batch.len(), 2);
        assert_eq!(buf.pending(), 0);
        assert!(buf.flush().is_empty(), "flushing twice sends nothing twice");
    }

    /// Both directions are copied — what the child said and what was said to
    /// them. A copy of only one half would misrepresent a conversation.
    #[test]
    fn the_body_carries_both_directions_readably() {
        let body = build_copy_body(
            "Home",
            &[
                line(Direction::Heard, "want to build?", 10),
                line(Direction::Said, "yes!", 12),
            ],
        );
        assert!(body.contains("World chat copy — Home"));
        assert!(body.contains("heard Pal: want to build?"));
        assert!(body.contains("said Pal: yes!"));
    }

    #[test]
    fn an_empty_batch_still_names_the_world() {
        let body = build_copy_body("Home", &[]);
        assert!(body.contains("Home"));
    }

    /// The indicator has to be readable by the child it is about.
    #[test]
    fn the_indicator_is_plain_and_says_who_sees_it() {
        assert!(COPY_INDICATOR.to_lowercase().contains("grown-up"));
        assert!(COPY_INDICATOR.len() < 48, "must fit a HUD line");
    }
}
