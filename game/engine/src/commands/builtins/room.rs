//! `/room` — attach the world's chat to a KithMoot room so it reaches a
//! parent's phone (world-chat spec §4.5). Registered NATIVE ONLY (`mod.rs`):
//! there is no room plug and no chat at all on the web taster (spec §0/§6),
//! and this command's own usage text names `/room invite` literally, which
//! `tools/smoke/forbidden-symbol.mjs` greps the web bundle for.
//!
//! The room itself lives on `HostedServer`, which `CommandContext` is
//! deliberately too narrow to reach (see its doc comment). So — same as every
//! other server-reaching command (`SpawnCart`, `BeaconFollow`, …) — this
//! command only decides WHICH thing to do and returns it as a
//! [`CommandResult`] side effect; the game loop, which owns `self.hosted_server`,
//! applies it.
//!
//! `/room rotate` is deliberately NOT a subcommand here: rotation is a keeper
//! operation (only the process that CREATED the room can rekey it), and this
//! game always joins as a member, never the keeper. Asking for it says so
//! plainly rather than pretending.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct RoomCommand;

impl Command for RoomCommand {
    fn name(&self) -> &'static str {
        "room"
    }
    fn help(&self) -> &'static str {
        "Attach world chat to a room so it reaches a phone"
    }
    fn usage(&self) -> &'static str {
        "/room | /room join <link> | /room leave | /room invite"
    }
    fn min_op_level(&self) -> OpLevel {
        // Attaching/detaching the room is a hosting decision, not something
        // any player should be able to twiddle — same bar as /worldedit.
        OpLevel::Op
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        match args.first().map(|s| s.to_lowercase()).as_deref() {
            None => CommandResult::RoomStatus,
            Some("join") => match args.get(1) {
                Some(link) if !link.trim().is_empty() => CommandResult::RoomJoin(link.clone()),
                _ => {
                    ctx.error("usage: /room join <link>");
                    CommandResult::Silent
                }
            },
            Some("leave") => CommandResult::RoomLeave,
            Some("invite") => CommandResult::RoomInvite,
            Some("rotate") => {
                ctx.error(
                    "/room rotate isn't something this game can do — rotating a room is a \
                     keeper operation, and this game joins a room as a member, not as its \
                     keeper. Whoever created the room can rotate it from there.",
                );
                CommandResult::Silent
            }
            Some(other) => {
                ctx.error(format!(
                    "unknown /room subcommand '{other}' — usage: /room | /room join <link> | \
                     /room leave | /room invite"
                ));
                CommandResult::Silent
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(args: &[&str]) -> (CommandResult, Vec<String>) {
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut play_mode = crate::play_mode::PlayMode::Survival;
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
            play_mode: &mut play_mode,
            seed: 12345,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::Op,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let r = RoomCommand.execute(&mut ctx, &owned);
        assert!(!ch, "/room must never set the cheat marker");
        (r, log.iter().map(|l| l.text.clone()).collect())
    }

    #[test]
    fn metadata_is_not_a_cheat_and_needs_op() {
        assert_eq!(RoomCommand.name(), "room");
        assert!(!RoomCommand.is_cheat());
        assert_eq!(RoomCommand.min_op_level(), OpLevel::Op);
    }

    #[test]
    fn no_args_asks_for_status() {
        let (r, _) = run(&[]);
        assert_eq!(r, CommandResult::RoomStatus);
    }

    #[test]
    fn join_with_a_link_returns_room_join() {
        let (r, _) = run(&["join", "https://example.org/j/#owned-by-members"]);
        assert_eq!(
            r,
            CommandResult::RoomJoin("https://example.org/j/#owned-by-members".to_string())
        );
    }

    #[test]
    fn join_without_a_link_errors_locally() {
        let (r, log) = run(&["join"]);
        assert_eq!(r, CommandResult::Silent);
        assert!(log.iter().any(|l| l.contains("usage: /room join")));
    }

    #[test]
    fn leave_returns_room_leave() {
        let (r, _) = run(&["leave"]);
        assert_eq!(r, CommandResult::RoomLeave);
    }

    #[test]
    fn invite_returns_room_invite() {
        let (r, _) = run(&["invite"]);
        assert_eq!(r, CommandResult::RoomInvite);
    }

    /// Rotation is a keeper-only operation; the game joins as a member and
    /// must say so plainly rather than pretending it can rekey the room.
    #[test]
    fn rotate_says_plainly_that_it_cannot_do_that() {
        let (r, log) = run(&["rotate"]);
        assert_eq!(r, CommandResult::Silent);
        assert!(log.iter().any(|l| l.contains("keeper operation")));
    }

    #[test]
    fn unknown_subcommand_errors_locally() {
        let (r, log) = run(&["nonsense"]);
        assert_eq!(r, CommandResult::Silent);
        assert!(log.iter().any(|l| l.contains("unknown /room subcommand")));
    }
}
