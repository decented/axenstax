use glam::Vec3;

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct SpawnpointCommand;

const MAX_COORD: f32 = 1_000_000.0;

impl Command for SpawnpointCommand {
    fn name(&self) -> &'static str {
        "spawnpoint"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["sp"]
    }
    fn help(&self) -> &'static str {
        "Set respawn point — current position, or explicit x/y/z"
    }
    fn usage(&self) -> &'static str {
        "/spawnpoint [<x> <y> <z>]"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let target = match args.len() {
            0 => {
                // No args: use the player's current position.
                let Some(slot) = ctx.players.get(ctx.player_idx) else {
                    let msg = "no player slot to set spawn for".to_string();
                    ctx.error(msg.clone());
                    return CommandResult::Error(msg);
                };
                slot.player.pos
            }
            3 => {
                let parsed: Result<Vec<f32>, _> =
                    args.iter().map(|s| s.parse::<f32>()).collect();
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
                Vec3::new(clamp(coords[0]), clamp(coords[1]), clamp(coords[2]))
            }
            _ => {
                let msg = "usage: /spawnpoint [<x> <y> <z>]".to_string();
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };

        let Some(slot) = ctx.players.get_mut(ctx.player_idx) else {
            let msg = "no player slot to set spawn for".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        slot.spawn_pos = target;
        ctx.success(format!(
            "Spawn point set to ({:.1}, {:.1}, {:.1})",
            target.x, target.y, target.z
        ));
        CommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str]) -> (CommandResult, Vec<PlayerSlot>) {
        let cmd = SpawnpointCommand;
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
    fn no_args_sets_to_current_position() {
        let (r, players) = run(&[]);
        assert_eq!(r, CommandResult::Success);
        // Player was spawned at (10, 70, 10). Spawn_pos should now match.
        assert_eq!(players[0].spawn_pos, Vec3::new(10.0, 70.0, 10.0));
    }

    #[test]
    fn explicit_coords_set_spawn() {
        let (r, players) = run(&["100", "65", "-50"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].spawn_pos, Vec3::new(100.0, 65.0, -50.0));
    }

    #[test]
    fn one_or_two_args_rejected() {
        assert!(matches!(run(&["1"]).0, CommandResult::Error(_)));
        assert!(matches!(run(&["1", "2"]).0, CommandResult::Error(_)));
    }

    #[test]
    fn four_args_rejected() {
        assert!(matches!(run(&["1", "2", "3", "4"]).0, CommandResult::Error(_)));
    }

    #[test]
    fn rejects_garbage_coords() {
        assert!(matches!(run(&["banana", "70", "0"]).0, CommandResult::Error(_)));
    }

    #[test]
    fn rejects_nan() {
        assert!(matches!(run(&["NaN", "70", "0"]).0, CommandResult::Error(_)));
    }

    #[test]
    fn clamps_extreme_coords() {
        let (r, players) = run(&["1e9", "70", "0"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].spawn_pos.x, MAX_COORD);
    }

    #[test]
    fn does_not_teleport_player() {
        let (r, players) = run(&["100", "65", "-50"]);
        assert_eq!(r, CommandResult::Success);
        // Player's position should be unchanged; only spawn_pos moved.
        assert_eq!(players[0].player.pos, Vec3::new(10.0, 70.0, 10.0));
    }
}
