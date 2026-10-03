//! `/place <mob>` — #129. Place a PERMANENT authored animal that saves with the
//! world and respawns on load (e.g. a donkey pinned by its statue). Unlike
//! `/spawn` (a transient debug spawn), the placed mob carries the `Authored`
//! ECS marker, so it persists in the `.axeworld` via the `saved_mobs` list and
//! re-appears every time the world loads — and, having no `Scattered` marker,
//! it survives chunk unload during a session.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;
use crate::mob::MobType;

pub struct PlaceCommand;

impl Command for PlaceCommand {
    fn name(&self) -> &'static str {
        "place"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["placemob", "anchor"]
    }
    fn help(&self) -> &'static str {
        "Place a permanent animal that saves with the world (e.g. /place donkey)"
    }
    fn usage(&self) -> &'static str {
        "/place <cow|pig|sheep|chicken|horse|donkey|mule|rabbit|fox|cat|parrot|...>"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let name = match args.first() {
            Some(s) => s.to_lowercase(),
            None => {
                let msg = "usage: /place <mob_id>  (e.g. /place donkey)".to_string();
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let kind = match MobType::from_name(&name) {
            Some(k) => k,
            None => {
                let msg = format!(
                    "unknown mob: '{name}' — try cow, pig, sheep, chicken, horse, donkey, mule, rabbit, fox, cat, parrot"
                );
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let Some(slot) = ctx.players.get(ctx.player_idx) else {
            let msg = "no player slot to place near".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        // Place 2 blocks east of the player (same offset as /spawn) — far enough
        // not to overlap the player hitbox, close enough to see. Stand where you
        // want it and it lands beside you; it's saved at this spot.
        let base = slot.player.pos + glam::Vec3::new(2.0, 0.0, 0.0);
        // Snap to the REAL surface (like scattered animals, #129) so an animal
        // placed on a slope / ledge / shallow water isn't pinned floating or
        // half-buried at the player's eye-level Y every time the world reloads.
        // `real_surface_y` returns 0 for an all-air column — fall back to the
        // player's own Y there (e.g. placing out over a void).
        let (wx, wz) = (base.x.floor() as i32, base.z.floor() as i32);
        let sy = crate::entity::real_surface_y(ctx.world, wx, wz);
        let pos = if ctx.world.get_block(wx, sy, wz) == crate::block::AIR {
            base
        } else {
            glam::Vec3::new(base.x, (sy + 1) as f32, base.z)
        };
        ctx.success(format!(
            "Placed {name} (saved with the world) at ({:.1}, {:.1}, {:.1})",
            pos.x, pos.y, pos.z
        ));
        CommandResult::PlaceAuthoredMob(kind, pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(args: &[&str]) -> CommandResult {
        run_in(World::new(), args)
    }

    fn run_in(mut world: World, args: &[&str]) -> CommandResult {
        let cmd = PlaceCommand;
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
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
            play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
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
        cmd.execute(&mut ctx, &owned)
    }

    #[test]
    fn places_donkey_as_authored_mob_offset_from_player() {
        // #129 — the headline: pin a donkey. Returns PlaceAuthoredMob so the
        // game loop spawns it with the Authored marker (persisted).
        match run(&["donkey"]) {
            CommandResult::PlaceAuthoredMob(kind, pos) => {
                assert_eq!(kind, MobType::Donkey);
                assert!((pos.x - 12.0).abs() < 1e-3); // player.x 10 + 2
                assert!((pos.y - 70.0).abs() < 1e-3);
                assert!((pos.z - 5.0).abs() < 1e-3);
            }
            other => panic!("expected PlaceAuthoredMob, got {other:?}"),
        }
    }

    #[test]
    fn snaps_placed_mob_to_the_real_surface_not_the_players_y() {
        // Regression (#129): the donkey used to save at the player's eye-level Y,
        // so on a slope/ledge it loaded floating or buried. Place ground well
        // below the player at the target column and assert it snaps to surface+1.
        let mut world = World::new();
        world.set_block(12, 64, 5, crate::block::GRASS); // 2 east of the player (10,70,5)
        match run_in(world, &["donkey"]) {
            CommandResult::PlaceAuthoredMob(_, pos) => {
                assert!((pos.x - 12.0).abs() < 1e-3);
                assert!((pos.z - 5.0).abs() < 1e-3);
                assert!((pos.y - 65.0).abs() < 1e-3, "stands on the grass (65), got {}", pos.y);
            }
            other => panic!("expected PlaceAuthoredMob, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_mob() {
        assert!(matches!(run(&["banana"]), CommandResult::Error(_)));
    }

    #[test]
    fn no_args_errors() {
        assert!(matches!(run(&[]), CommandResult::Error(_)));
    }
}
