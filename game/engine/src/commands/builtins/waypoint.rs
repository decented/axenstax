//! `/waypoint` (#6) — manage map waypoints: pins shown on the minimap + the
//! full-screen map. Subcommands:
//!   - `add <name>`    — drop a pin at your feet
//!   - `list`          — list every waypoint (pins + death markers)
//!   - `remove <name>` — delete a pin by name
//!   - `tp <name>`     — teleport to a waypoint (**creative only**)
//!
//! Navigation is not a cheat, so this is available to all players (`OpLevel::
//! None`); only the creative-gated teleport changes the world state.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};
use crate::waypoint;
use glam::Vec3;

pub struct WaypointCommand;

impl Command for WaypointCommand {
    fn name(&self) -> &'static str {
        "waypoint"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["wp"]
    }
    fn help(&self) -> &'static str {
        "Manage map waypoints (pins on the minimap + map)"
    }
    fn usage(&self) -> &'static str {
        "/waypoint add <name> | list | remove <name> | tp <name>"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let usage = "usage: /waypoint add <name> | list | remove <name> | tp <name>";
        match args.first().map(|s| s.to_lowercase()).as_deref() {
            Some("add") => {
                let name = args[1..].join(" ");
                if name.trim().is_empty() {
                    let m = "usage: /waypoint add <name>".to_string();
                    ctx.error(m.clone());
                    return CommandResult::Error(m);
                }
                let pos = match ctx.players.get(ctx.player_idx) {
                    Some(s) => s.player.pos,
                    None => {
                        let m = "no player".to_string();
                        ctx.error(m.clone());
                        return CommandResult::Error(m);
                    }
                };
                let p = [pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32];
                waypoint::add(
                    &mut ctx.world.waypoints,
                    name.clone(),
                    p,
                    waypoint::MANUAL_COLOUR,
                    waypoint::WaypointKind::Manual,
                );
                ctx.success(format!("Waypoint '{name}' set at ({}, {}, {})", p[0], p[1], p[2]));
                CommandResult::Success
            }
            Some("list") => {
                if ctx.world.waypoints.is_empty() {
                    ctx.success("No waypoints set.".to_string());
                    return CommandResult::Success;
                }
                let lines: Vec<String> = ctx
                    .world
                    .waypoints
                    .iter()
                    .map(|w| {
                        let kind = match w.kind {
                            waypoint::WaypointKind::Manual => "pin",
                            waypoint::WaypointKind::Death => "death",
                        };
                        format!("• {} [{}] ({}, {}, {})", w.name, kind, w.pos[0], w.pos[1], w.pos[2])
                    })
                    .collect();
                for l in lines {
                    ctx.success(l);
                }
                CommandResult::Success
            }
            Some("remove") | Some("rm") | Some("del") => {
                let name = args[1..].join(" ");
                if waypoint::remove_by_name(&mut ctx.world.waypoints, &name) {
                    ctx.success(format!("Removed waypoint '{name}'"));
                    CommandResult::Success
                } else {
                    let m = format!("No waypoint named '{name}'");
                    ctx.error(m.clone());
                    CommandResult::Error(m)
                }
            }
            Some("tp") | Some("goto") => {
                if !*ctx.is_creative {
                    let m = "Waypoint teleport is creative-only.".to_string();
                    ctx.error(m.clone());
                    return CommandResult::Error(m);
                }
                let name = args[1..].join(" ");
                let target = match waypoint::find_by_name(&ctx.world.waypoints, &name) {
                    Some(w) => {
                        Vec3::new(w.pos[0] as f32 + 0.5, w.pos[1] as f32, w.pos[2] as f32 + 0.5)
                    }
                    None => {
                        let m = format!("No waypoint named '{name}'");
                        ctx.error(m.clone());
                        return CommandResult::Error(m);
                    }
                };
                if let Some(slot) = ctx.players.get_mut(ctx.player_idx) {
                    slot.player.pos = target;
                    slot.player.velocity = Vec3::ZERO;
                    ctx.success(format!("Teleported to '{name}'"));
                    CommandResult::Success
                } else {
                    let m = "no player".to_string();
                    ctx.error(m.clone());
                    CommandResult::Error(m)
                }
            }
            _ => {
                let m = usage.to_string();
                ctx.error(m.clone());
                CommandResult::Error(m)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::play_mode::PlayMode;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(world: &mut World, creative: bool, args: &[&str]) -> (CommandResult, Vec<PlayerSlot>) {
        let cmd = WaypointCommand;
        let mut t = 0u32;
        let mut s = 4u32;
        let mut cr = creative;
        let mut mode = PlayMode::Survival;
        let mut players = vec![PlayerSlot::new(0, Vec3::new(12.0, 70.0, -5.0), 0.5)];
        let mut log = Vec::new();
        let (mut ch, mut ev, mut ps) = (false, false, false);
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut cr,
            play_mode: &mut mode,
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
    fn add_pins_at_the_players_floored_position() {
        let mut world = World::new();
        let (r, _) = run(&mut world, false, &["add", "Base", "Camp"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(world.waypoints.len(), 1);
        assert_eq!(world.waypoints[0].name, "Base Camp");
        assert_eq!(world.waypoints[0].pos, [12, 70, -5]);
        assert_eq!(world.waypoints[0].kind, waypoint::WaypointKind::Manual);
    }

    #[test]
    fn add_without_a_name_errors() {
        let mut world = World::new();
        assert!(matches!(run(&mut world, false, &["add"]).0, CommandResult::Error(_)));
        assert!(world.waypoints.is_empty());
    }

    #[test]
    fn remove_deletes_by_name() {
        let mut world = World::new();
        run(&mut world, false, &["add", "Mine"]);
        let (r, _) = run(&mut world, false, &["remove", "mine"]); // case-insensitive
        assert_eq!(r, CommandResult::Success);
        assert!(world.waypoints.is_empty());
    }

    #[test]
    fn tp_is_creative_only() {
        let mut world = World::new();
        run(&mut world, false, &["add", "Home"]);
        // Survival: refused, position unchanged.
        let (r, players) = run(&mut world, false, &["tp", "Home"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert_eq!(players[0].player.pos, Vec3::new(12.0, 70.0, -5.0));
    }

    #[test]
    fn tp_in_creative_moves_to_the_waypoint() {
        let mut world = World::new();
        // Pin a spot, then teleport there in creative.
        waypoint::add(
            &mut world.waypoints,
            "Far".into(),
            [100, 80, -40],
            waypoint::MANUAL_COLOUR,
            waypoint::WaypointKind::Manual,
        );
        let (r, players) = run(&mut world, true, &["tp", "Far"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].player.pos, Vec3::new(100.5, 80.0, -39.5));
    }

    #[test]
    fn unknown_subcommand_errors() {
        let mut world = World::new();
        assert!(matches!(run(&mut world, false, &["wat"]).0, CommandResult::Error(_)));
    }
}
