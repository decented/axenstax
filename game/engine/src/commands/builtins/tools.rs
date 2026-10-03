//! `/tools` — list every tool type × tier combo + its stats. Read-only.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};
use crate::crafting::{Tool, ToolMaterial, ToolType};

pub struct ToolsCommand;

const TIERS: &[ToolMaterial] = &[
    ToolMaterial::Wood, ToolMaterial::Stone, ToolMaterial::Iron,
    ToolMaterial::Diamond, ToolMaterial::Satori,
];

const TYPES: &[ToolType] = &[
    ToolType::Pickaxe, ToolType::Axe, ToolType::Sword, ToolType::Shovel,
    ToolType::Hoe, ToolType::Bow, ToolType::Shears,
    ToolType::FishingRod, ToolType::FlintAndSteel,
];

impl Command for ToolsCommand {
    fn name(&self) -> &'static str { "tools" }
    fn help(&self) -> &'static str { "List every tool tier + stats" }
    fn usage(&self) -> &'static str { "/tools" }
    fn min_op_level(&self) -> OpLevel { OpLevel::None }
    fn is_cheat(&self) -> bool { false }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        let mut count = 0;
        for t in TYPES {
            for m in TIERS {
                // Single-tier tools (Shears / FishingRod / FlintAndSteel)
                // only have a meaningful entry at one tier; skip the
                // other 4 rows so the listing isn't cluttered.
                if matches!(t, ToolType::Shears | ToolType::FishingRod | ToolType::FlintAndSteel)
                    && *m != ToolMaterial::Iron
                {
                    continue;
                }
                let tool = Tool::new(*t, *m);
                ctx.success(format!(
                    "  {} — atk {:.1}, mine_speed {:.1}, durability {}",
                    tool.name(),
                    tool.attack_damage(),
                    tool.mining_speed(),
                    tool.durability,
                ));
                count += 1;
            }
        }
        ctx.success(format!("{count} tools listed."));
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run() -> (CommandResult, Vec<String>) {
        let cmd = ToolsCommand;
        let mut world = World::new();
        let mut t = 0u32; let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 0.5)];
        let mut log = Vec::new();
        let mut ch = false; let mut ev = false; let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world, world_time: &mut t, world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival }, seed: 42, world_name: "test",
            players: &mut players, player_idx: 0, op_level: OpLevel::Op,
            current_tick: 0, log: &mut log, registry: &reg,
            cheats_used_marker: &mut ch, ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let r = cmd.execute(&mut ctx, &[]);
        let texts: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, texts)
    }

    #[test]
    fn lists_every_tier_of_pickaxe() {
        let (_, texts) = run();
        for name in ["Wooden Pickaxe", "Stone Pickaxe", "Iron Pickaxe",
                     "Diamond Pickaxe", "Satori Pickaxe"] {
            assert!(texts.iter().any(|t| t.contains(name)), "missing {name}");
        }
    }

    #[test]
    fn includes_per_tier_bows() {
        let (_, texts) = run();
        for name in ["Stone Bow", "Iron Bow", "Diamond Bow", "Satori Bow"] {
            assert!(texts.iter().any(|t| t.contains(name)), "missing {name}");
        }
    }
}
