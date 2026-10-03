//! `/armour` — list every armour piece + tier with armour points +
//! durability. Read-only.

use crate::armour::{ArmourMaterial, ArmourSlot, armour_label, armour_points, max_durability};
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

const SLOTS: &[ArmourSlot] = &[
    ArmourSlot::Helmet, ArmourSlot::Chestplate,
    ArmourSlot::Leggings, ArmourSlot::Boots,
];

const TIERS: &[ArmourMaterial] = &[
    ArmourMaterial::Leather, ArmourMaterial::Chainmail,
    ArmourMaterial::Iron, ArmourMaterial::Diamond, ArmourMaterial::Satori,
];

pub struct ArmourCommand;

impl Command for ArmourCommand {
    fn name(&self) -> &'static str { "armour" }
    fn aliases(&self) -> &'static [&'static str] { &["armor"] }
    fn help(&self) -> &'static str { "List every armour piece + tier with stats" }
    fn usage(&self) -> &'static str { "/armour" }
    fn min_op_level(&self) -> OpLevel { OpLevel::None }
    fn is_cheat(&self) -> bool { false }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        let mut count = 0;
        for tier in TIERS {
            let mut total_points = 0u16;
            for slot in SLOTS {
                let pts = armour_points(*slot, *tier);
                total_points += pts as u16;
                ctx.success(format!(
                    "  {} — {} pts, durability {}",
                    armour_label(*slot, *tier),
                    pts,
                    max_durability(*slot, *tier),
                ));
                count += 1;
            }
            ctx.success(format!("    (full {tier:?} set: {total_points} pts)"));
        }
        ctx.success(format!("{count} armour pieces listed."));
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
        let cmd = ArmourCommand;
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
    fn lists_armour_piece_set() {
        let (_, texts) = run();
        // 4 slots × 5 tiers = 20 piece lines + 5 set lines + total
        for name in ["Leather Helmet", "Iron Chestplate", "Diamond Leggings", "Satori Boots"] {
            assert!(texts.iter().any(|t| t.contains(name)), "missing {name}");
        }
    }
}
