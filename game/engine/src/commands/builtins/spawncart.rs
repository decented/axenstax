//! `/spawncart` — Rail freight (Phase 1) debug command. Drops a cart on the
//! TRACK cell the player is aiming at.
//!
//! The command itself can't raycast (the `CommandContext` carries no ray /
//! look info), so it returns `CommandResult::SpawnCart` and the game loop does
//! the eye/look raycast + track-cell check before spawning. This mirrors how
//! `/spawn` returns `SpawnMob` for the loop to act on.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct SpawnCartCommand;

impl Command for SpawnCartCommand {
    fn name(&self) -> &'static str {
        "spawncart"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["cart"]
    }
    fn help(&self) -> &'static str {
        "Drop a cart on the rail track you're looking at"
    }
    fn usage(&self) -> &'static str {
        "/spawncart"
    }
    fn is_cheat(&self) -> bool {
        true
    }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        // The game loop raycasts + validates the targeted block is TRACK; it
        // echoes the success/failure line itself once it knows the cell. Here
        // we just request the side-effect.
        *ctx.cheats_used_marker = true;
        *ctx.pure_survival_broken_marker = true;
        CommandResult::SpawnCart
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(args: &[&str]) -> (CommandResult, bool, bool) {
        let cmd = SpawnCartCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut pm = crate::play_mode::PlayMode::Survival;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(10.0, 70.0, 5.0), 0.5)];
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
            play_mode: &mut pm,
            seed: 42,
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
        let r = cmd.execute(&mut ctx, &owned);
        (r, ch, ps)
    }

    #[test]
    fn returns_spawn_cart_side_effect() {
        let (r, ch, ps) = run(&[]);
        assert_eq!(r, CommandResult::SpawnCart);
        assert!(ch, "spawncart marks cheats used");
        assert!(ps, "spawncart breaks pure-survival");
    }

    #[test]
    fn is_a_cheat() {
        assert!(SpawnCartCommand.is_cheat());
    }
}
