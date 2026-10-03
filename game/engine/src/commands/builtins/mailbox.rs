//! `/mailbox` — "Your reports": each report this install sent, with its status
//! (Sent / Received / Fixed in vX / Won't fix). Nobody is ever messaged; the
//! status comes from a public, anonymous board only this game can read the
//! meaning of. Spec: docs/foundations/2026-10-01-feedback-status-board.md.
//! NATIVE ONLY: not compiled on wasm32 (the browser build has no feedback
//! channel — removed 2026-10-01).

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct MailboxCommand;

impl Command for MailboxCommand {
    fn name(&self) -> &'static str {
        "mailbox"
    }
    fn help(&self) -> &'static str {
        "See the status of your bug reports and ideas"
    }
    fn usage(&self) -> &'static str {
        "/mailbox"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        // Ask the worker to re-read the public board (debounced to once a
        // minute); this view shows the last status it stored.
        crate::native_mailbox::request_refresh();
        let lines = crate::native_mailbox::ticket_lines();
        if lines.is_empty() {
            ctx.success("You haven't sent any reports yet — try /bug or /idea.");
        } else {
            ctx.success("Your reports:");
            for l in lines {
                ctx.success(format!("  {l}"));
            }
        }
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    // Mirrors feedback.rs's CommandContext setup.
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
            is_creative: &mut creative,
            play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
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
        // No cheat marker should ever be set by mailbox.
        assert!(!ch, "mailbox commands must never set the cheat marker");
        (r, log.iter().map(|l| l.text.clone()).collect())
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn mailbox_lists_your_reports_with_their_status() {
        use crate::native_mailbox::{board, tickets};
        let dir = std::env::temp_dir().join(format!("axemb-mbcmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::native_mailbox::reset_for_test(&dir);
        let paths = crate::native_mailbox::Paths::in_profile(&dir);
        tickets::record_sent(&paths.tickets, "t1", "bug", "doors too tall", 1000, 1000).unwrap();
        let board = board::Board {
            entries: [(board::ticket_key("t1"), board::Status::Fixed(Some("0.2.28".into())))].into(),
        };
        tickets::apply_board(&paths.tickets, &board, 2000).unwrap();
        let (r, log) = run(&MailboxCommand, &[]);
        assert_eq!(r, CommandResult::Silent);
        assert!(
            log.iter().any(|l| l.contains("doors too tall") && l.contains("Fixed in v0.2.28")),
            "got {log:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn empty_mailbox_says_so() {
        let dir = std::env::temp_dir().join(format!("axemb-mbempty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _guard = crate::native_mailbox::reset_for_test(&dir);
        let (_, log) = run(&MailboxCommand, &[]);
        assert!(log.iter().any(|l| l.contains("haven't sent any reports")), "got {log:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_is_player_safe_non_cheat() {
        assert_eq!(MailboxCommand.name(), "mailbox");
        assert!(!MailboxCommand.is_cheat());
        assert_eq!(MailboxCommand.min_op_level(), OpLevel::None);
    }
}
