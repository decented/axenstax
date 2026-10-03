//! `/bug` and `/idea` — player→maker feedback. NATIVE ONLY: the message is
//! handed to `native_mailbox::enqueue_and_flush`, which queues it to the
//! on-disk outbox and (when a worker is running) flushes it as an anonymous
//! NIP-17 DM sealed with a one-time key (no persona, no account).
//! The browser build has no feedback channel at all (removed 2026-10-01), so
//! this module is not compiled on wasm32 and `/bug` is an unknown command there.
//! See `docs/foundations/2026-10-01-feedback-status-board.md`.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct BugCommand;
pub struct IdeaCommand;

const MAX_LEN: usize = 2000;

fn handle(ctx: &mut CommandContext, args: &[String], kind: &'static str) -> CommandResult {
    let joined = args.join(" ");
    let msg = joined.trim();

    if msg.is_empty() {
        // No-arg: show the usage hint (there is no composer UI to open).
        ctx.success(format!(
            "Type /{kind} <{}> — it goes straight to the makers.",
            if kind == "bug" { "what went wrong" } else { "your idea" }
        ));
        return CommandResult::Silent;
    }

    if msg.chars().count() > MAX_LEN {
        ctx.success("Message too long (max 2000 characters).");
        return CommandResult::Silent;
    }

    // Enqueue locally; flushed as a NIP-17 DM by the native mailbox worker.
    {
        // Errors (most commonly: the global mailbox service hasn't been
        // initialised — e.g. in tests that skip reset_for_test/init) are only
        // logged, never surfaced to the player: the player-facing contract
        // stays "Queued" either way, and the only case this hides is a broken
        // profile dir, where a scary error mid-game helps nobody.
        if let Err(e) = crate::native_mailbox::enqueue_and_flush(kind, msg) {
            log::warn!("[mailbox] enqueue failed: {e}");
        }
    }
    ctx.success(format!("Queued — sending to the makers. ({kind})"));
    // The report has been handed off (native outbox) — tell the game loop so a
    // running Trial can count it. The event is local and says nothing about the
    // report.
    CommandResult::FeedbackQueued
}

impl Command for BugCommand {
    fn name(&self) -> &'static str {
        "bug"
    }
    fn help(&self) -> &'static str {
        "Report a bug to the makers"
    }
    fn usage(&self) -> &'static str {
        "/bug <what went wrong>"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        handle(ctx, args, "bug")
    }
}

impl Command for IdeaCommand {
    fn name(&self) -> &'static str {
        "idea"
    }
    fn help(&self) -> &'static str {
        "Share an idea with the makers"
    }
    fn usage(&self) -> &'static str {
        "/idea <your idea>"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        handle(ctx, args, "idea")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    // Mirrors seed.rs's CommandContext setup.
    fn run(cmd: &dyn Command, args: &[&str]) -> (CommandResult, Vec<String>) {
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
            seed: 12345,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::None,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let r = cmd.execute(&mut ctx, &owned);
        // No cheat marker should ever be set by feedback.
        assert!(!ch, "feedback commands must never set the cheat marker");
        (r, log.iter().map(|l| l.text.clone()).collect())
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn inline_bug_queues_and_signals_the_trial_runner() {
        let dir = std::env::temp_dir().join(format!("axemb-cmd-inline-bug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::native_mailbox::reset_for_test(&dir);
        let (r, log) = run(&BugCommand, &["doors", "too", "tall"]);
        assert_eq!(r, CommandResult::FeedbackQueued);
        assert!(
            log.iter().any(|l| l.to_lowercase().contains("queued")),
            "expected a 'Queued' confirmation, got: {log:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn inline_idea_queues_and_signals_the_trial_runner() {
        let dir = std::env::temp_dir().join(format!("axemb-cmd-inline-idea-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::native_mailbox::reset_for_test(&dir);
        let (r, log) = run(&IdeaCommand, &["add", "rainbow", "sheep"]);
        assert_eq!(r, CommandResult::FeedbackQueued);
        assert!(log.iter().any(|l| l.to_lowercase().contains("queued")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_bug_shows_usage_hint() {
        let (r, log) = run(&BugCommand, &[]);
        assert_eq!(r, CommandResult::Silent);
        assert!(
            log.iter().any(|l| l.contains("/bug <what went wrong>")),
            "empty native /bug shows usage, got: {log:?}"
        );
        assert!(!log.iter().any(|l| l.to_lowercase().contains("web client")));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_inline_bug_lands_in_the_outbox() {
        let dir = std::env::temp_dir().join(format!("axemb-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::native_mailbox::reset_for_test(&dir);
        let (r, log) = run(&BugCommand, &["doors", "too", "tall"]);
        assert_eq!(r, CommandResult::FeedbackQueued);
        assert!(log.iter().any(|l| l.contains("Queued")));
        let q = crate::native_mailbox::outbox::queued(
            &crate::native_mailbox::Paths::in_profile(&dir).outbox,
        );
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].body, "doors too tall");
        assert_eq!(q[0].kind, "bug");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn over_long_message_is_rejected() {
        let long = "x".repeat(MAX_LEN + 1);
        let (r, log) = run(&BugCommand, &[long.as_str()]);
        assert_eq!(r, CommandResult::Silent);
        assert!(
            log.iter().any(|l| l.to_lowercase().contains("too long")),
            "expected a length-cap message, got: {log:?}"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn at_limit_message_is_accepted() {
        let dir = std::env::temp_dir().join(format!("axemb-cmd-atlimit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::native_mailbox::reset_for_test(&dir);
        let at = "y".repeat(MAX_LEN);
        let (r, log) = run(&BugCommand, &[at.as_str()]);
        assert_eq!(r, CommandResult::FeedbackQueued);
        assert!(log.iter().any(|l| l.to_lowercase().contains("queued")));
        assert!(!log.iter().any(|l| l.to_lowercase().contains("too long")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_is_player_safe_non_cheat() {
        assert_eq!(BugCommand.name(), "bug");
        assert_eq!(IdeaCommand.name(), "idea");
        assert!(!BugCommand.is_cheat());
        assert!(!IdeaCommand.is_cheat());
        assert_eq!(BugCommand.min_op_level(), OpLevel::None);
        assert_eq!(IdeaCommand.min_op_level(), OpLevel::None);
    }
}
