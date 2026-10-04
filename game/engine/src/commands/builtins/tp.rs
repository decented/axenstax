use glam::Vec3;

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct TpCommand;

const MAX_COORD: f32 = 1_000_000.0;

impl Command for TpCommand {
    fn name(&self) -> &'static str {
        "tp"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["teleport"]
    }
    fn help(&self) -> &'static str {
        "Teleport to coordinates"
    }
    fn usage(&self) -> &'static str {
        "/tp <x> <y> <z>"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        if args.len() != 3 {
            let msg = "usage: /tp <x> <y> <z>".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
        let parsed: Result<Vec<f32>, _> = args.iter().map(|s| s.parse::<f32>()).collect();
        let coords = match parsed {
            Ok(v) => v,
            Err(_) => {
                let msg = "all three coordinates must be numbers".to_string();
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        if coords.iter().any(|c| !c.is_finite()) {
            let msg = "coordinates must be finite (no NaN/inf)".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
        let clamp = |v: f32| v.clamp(-MAX_COORD, MAX_COORD);
        let target = Vec3::new(clamp(coords[0]), clamp(coords[1]), clamp(coords[2]));

        if let Some(slot) = ctx.players.get_mut(ctx.player_idx) {
            slot.player.pos = target;
            slot.player.velocity = Vec3::ZERO;
            // W2 — a teleport is not a fall: drop any distance banked before it.
            slot.player.reset_fall();
            ctx.success(format!(
                "Teleported to ({:.1}, {:.1}, {:.1})",
                target.x, target.y, target.z
            ));
            CommandResult::Success
        } else {
            let msg = "no player slot to teleport".to_string();
            ctx.error(msg.clone());
            CommandResult::Error(msg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str]) -> (CommandResult, Vec<PlayerSlot>) {
        let cmd = TpCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, Vec3::new(10.0, 70.0, 10.0), 0.5)];
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
        (r, players)
    }

    #[test]
    fn teleports_to_coords() {
        let (r, players) = run(&["100", "80", "-50"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].player.pos, Vec3::new(100.0, 80.0, -50.0));
    }

    #[test]
    fn parses_floats() {
        let (r, players) = run(&["100.5", "80.25", "-50.75"]);
        assert_eq!(r, CommandResult::Success);
        assert!((players[0].player.pos.x - 100.5).abs() < 0.001);
    }

    #[test]
    fn rejects_wrong_arg_count() {
        assert!(matches!(run(&[]).0, CommandResult::Error(_)));
        assert!(matches!(run(&["1"]).0, CommandResult::Error(_)));
        assert!(matches!(run(&["1", "2"]).0, CommandResult::Error(_)));
        assert!(matches!(run(&["1", "2", "3", "4"]).0, CommandResult::Error(_)));
    }

    #[test]
    fn rejects_garbage_coords() {
        let (r, _) = run(&["banana", "80", "0"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn rejects_nan() {
        let (r, _) = run(&["NaN", "80", "0"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn clamps_extreme_coords() {
        let (r, players) = run(&["1e9", "80", "0"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].player.pos.x, MAX_COORD);
    }

    #[test]
    fn zeros_velocity() {
        let (r, players) = run(&["0", "80", "0"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].player.velocity, Vec3::ZERO);
    }
}
