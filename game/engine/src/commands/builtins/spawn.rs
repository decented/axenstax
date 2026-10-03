use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;
use crate::mob::MobType;

pub struct SpawnCommand;

fn mob_by_name(key: &str) -> Option<MobType> {
    match key {
        "cow" => Some(MobType::Cow),
        "pig" => Some(MobType::Pig),
        "sheep" => Some(MobType::Sheep),
        "chicken" => Some(MobType::Chicken),
        // HP-3 brigand-family hostiles — the live hostile roster.
        "brigand" => Some(MobType::Brigand),
        "marauder" => Some(MobType::Marauder),
        "berserker" => Some(MobType::Berserker),
        // Spec 28d.wolves — debug-spawnable. Live behaviour is the
        // untamed wander path until bone-taming + ECS spawning land.
        "wolf" => Some(MobType::Wolf),
        // Spec 28d chunk 3 — debug-spawnable. Live AI is the pure
        // tick functions in `horse_ai.rs` / `rabbit_ai.rs`; wander
        // path until ECS wire-up.
        "horse" => Some(MobType::Horse),
        "rabbit" => Some(MobType::Rabbit),
        // Spec 28d chunk 4 — passive-with-charge in `goat_ai.rs`.
        "goat" => Some(MobType::Goat),
        // Spec 28d chunk 5 — flying neutral; see `bee_ai.rs`.
        "bee" => Some(MobType::Bee),
        // Spec 28d chunk 6 — aquatic passive; see `squid_ai.rs`.
        "squid" => Some(MobType::Squid),
        _ => None,
    }
}

impl Command for SpawnCommand {
    fn name(&self) -> &'static str {
        "spawn"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["summon"]
    }
    fn help(&self) -> &'static str {
        "Spawn one mob of the named kind 2 blocks east of you"
    }
    fn usage(&self) -> &'static str {
        "/spawn <cow|pig|sheep|chicken|brigand|marauder|berserker|wolf|horse|rabbit|goat|bee|squid>"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let name = match args.first() {
            Some(s) => s.to_lowercase(),
            None => {
                let msg = "usage: /spawn <mob_id>".to_string();
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let kind = match mob_by_name(&name) {
            Some(k) => k,
            None => {
                let msg = format!(
                    "unknown mob: '{name}' (try cow, pig, sheep, chicken, brigand, marauder, berserker, wolf, horse, rabbit, goat, bee, squid)"
                );
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let Some(slot) = ctx.players.get(ctx.player_idx) else {
            let msg = "no player slot to spawn near".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        // Spawn 2 blocks east of the player at their feet height. Far enough
        // not to overlap the player hitbox; close enough to see.
        let pos = slot.player.pos + glam::Vec3::new(2.0, 0.0, 0.0);
        *ctx.cheats_used_marker = true;
        *ctx.pure_survival_broken_marker = true;
        ctx.success(format!("Spawned {name} at ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z));
        CommandResult::SpawnMob(kind, pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str]) -> (CommandResult, bool, bool) {
        let cmd = SpawnCommand;
        let mut world = World::new();
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
        (r, ch, ps)
    }

    #[test]
    fn spawns_named_mob_offset_from_player() {
        let (r, ch, ps) = run(&["brigand"]);
        match r {
            CommandResult::SpawnMob(kind, pos) => {
                assert_eq!(kind, MobType::Brigand);
                // Player is at (10, 70, 5); spawn is +2.0 in X.
                assert!((pos.x - 12.0).abs() < 1e-3);
                assert!((pos.y - 70.0).abs() < 1e-3);
                assert!((pos.z - 5.0).abs() < 1e-3);
            }
            other => panic!("expected SpawnMob, got {other:?}"),
        }
        assert!(ch);
        assert!(ps);
    }

    #[test]
    fn rejects_unknown_mob() {
        let (r, ch, _) = run(&["banana"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!ch, "unknown-mob /spawn must not mark cheats");
    }

    #[test]
    fn no_args_errors() {
        let (r, _, _) = run(&[]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn case_insensitive_alias_works() {
        let (r, _, _) = run(&["MaRaUdEr"]);
        match r {
            CommandResult::SpawnMob(kind, _) => assert_eq!(kind, MobType::Marauder),
            other => panic!("expected SpawnMob, got {other:?}"),
        }
    }
}
