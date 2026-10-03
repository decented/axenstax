//! `/mobs` — list all mob types known to the engine (data-driven from
//! the `data/mobs/*.toml` registry). Read-only debug — not a cheat.
//!
//! Reports each mob's name + category (passive/hostile) + HP + size,
//! one line per kind. Useful for verifying that a new chunk's species
//! is wired all the way through `MobType` and the TOML loader.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};
use crate::mob::{MOBS, MobCategory};

pub struct MobsCommand;

impl Command for MobsCommand {
    fn name(&self) -> &'static str { "mobs" }
    fn help(&self) -> &'static str {
        "List every mob type the engine knows about + their stats"
    }
    fn usage(&self) -> &'static str { "/mobs" }
    fn min_op_level(&self) -> OpLevel { OpLevel::None }
    fn is_cheat(&self) -> bool { false }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        ctx.success(format!("{} mob types:", MOBS.len()));
        for def in MOBS.iter() {
            let category = match def.category {
                MobCategory::Passive => "passive",
                MobCategory::Hostile => "hostile",
            };
            ctx.success(format!(
                "  {} — {category}, HP {}, {:.1}×{:.1} blocks",
                def.name, def.health, def.width, def.height,
            ));
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

    fn run(args: &[&str]) -> (CommandResult, Vec<String>) {
        let cmd = MobsCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 0.5)];
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
        let texts: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, texts)
    }

    #[test]
    fn lists_every_mob_in_the_registry() {
        let (r, texts) = run(&[]);
        assert_eq!(r, CommandResult::Silent);
        // First line says "N mob types:"; subsequent lines are per-mob.
        assert!(texts[0].contains("mob types"));
        for kind in ["Cow", "Brigand", "Wolf", "Horse", "Rabbit", "Goat", "Bee", "Squid"] {
            assert!(texts.iter().any(|t| t.contains(kind)), "expected {kind} in /mobs output");
        }
    }

    #[test]
    fn not_a_cheat() {
        assert!(!MobsCommand.is_cheat());
    }
}
