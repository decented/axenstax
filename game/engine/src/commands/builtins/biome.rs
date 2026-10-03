//! `/biome` — print the biome at a given (x, z) position, or at the
//! player's current position when called with no arguments.
//!
//! Uses the Spec 28a Whittaker classifier (`assign_biome_whittaker`).
//! Read-only debug — does NOT mark cheats / break pure-survival.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct BiomeCommand;

impl Command for BiomeCommand {
    fn name(&self) -> &'static str {
        "biome"
    }
    fn help(&self) -> &'static str {
        "Print the biome at (x, z) or at the player's position"
    }
    fn usage(&self) -> &'static str {
        "/biome [x] [z]"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let (x, z) = match (args.first(), args.get(1)) {
            (Some(xs), Some(zs)) => match (xs.parse::<i32>(), zs.parse::<i32>()) {
                (Ok(x), Ok(z)) => (x, z),
                _ => {
                    let msg = "usage: /biome [x] [z] — coordinates must be integers".to_string();
                    ctx.error(msg.clone());
                    return CommandResult::Error(msg);
                }
            },
            _ => {
                let Some(slot) = ctx.players.get(ctx.player_idx) else {
                    let msg = "no player slot".to_string();
                    ctx.error(msg.clone());
                    return CommandResult::Error(msg);
                };
                (slot.player.pos.x as i32, slot.player.pos.z as i32)
            }
        };
        let biome = crate::biome::assign_biome_whittaker(x, z, ctx.seed);
        ctx.success(format!("Biome at ({x}, {z}): {biome:?}"));
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
        let cmd = BiomeCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(100.0, 70.0, 200.0), 0.5)];
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
    fn at_player_position_returns_a_biome() {
        let (r, texts) = run(&[]);
        assert_eq!(r, CommandResult::Silent);
        assert!(texts.iter().any(|t| t.contains("Biome at (100, 200)")),
            "expected biome at player pos, got {texts:?}");
    }

    #[test]
    fn at_explicit_coords_returns_biome_at_those_coords() {
        let (r, texts) = run(&["1000", "2000"]);
        assert_eq!(r, CommandResult::Silent);
        assert!(texts.iter().any(|t| t.contains("Biome at (1000, 2000)")));
    }

    #[test]
    fn non_integer_args_errors() {
        let (r, _) = run(&["foo", "bar"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn is_not_a_cheat() {
        assert!(!BiomeCommand.is_cheat());
        assert_eq!(BiomeCommand.min_op_level(), OpLevel::None);
    }
}
