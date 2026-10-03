//! `/complexitytier` and `/tradevalue` — print the Spec T1.5 Phase 11
//! economy annotations for any item currently held in the player's
//! selected hotbar slot.
//!
//! Read-only debug, never marks cheats. Surfaces the data the future
//! Vendor Block pricing + server economy modes will consume.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct ComplexityTierCommand;

impl Command for ComplexityTierCommand {
    fn name(&self) -> &'static str {
        "complexitytier"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["ctier"]
    }
    fn help(&self) -> &'static str {
        "Print the complexity tier of the held item"
    }
    fn usage(&self) -> &'static str {
        "/complexitytier"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        let Some(slot) = ctx.players.get(ctx.player_idx) else {
            let msg = "no player slot".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        let held = slot.inventory.hotbar_slot(slot.hotbar_slot);
        match held {
            Some(stack) => {
                let tier = stack.item.complexity_tier();
                // Construct a fresh BlockRegistry — it's static-data
                // costed once per command call, fine for debug surface.
                let registry = crate::block::BlockRegistry::new();
                let name = stack.item.name(&registry);
                ctx.success(format!("{name}: complexity tier {tier}"));
            }
            None => {
                ctx.success("Hold an item to see its complexity tier.".to_string());
            }
        }
        CommandResult::Silent
    }
}

pub struct TradeValueCommand;

impl Command for TradeValueCommand {
    fn name(&self) -> &'static str {
        "tradevalue"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["tvalue"]
    }
    fn help(&self) -> &'static str {
        "Print the default trade value of the held item"
    }
    fn usage(&self) -> &'static str {
        "/tradevalue"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        let Some(slot) = ctx.players.get(ctx.player_idx) else {
            let msg = "no player slot".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        let held = slot.inventory.hotbar_slot(slot.hotbar_slot);
        match held {
            Some(stack) => {
                let value = stack.item.trade_value();
                // Construct a fresh BlockRegistry — it's static-data
                // costed once per command call, fine for debug surface.
                let registry = crate::block::BlockRegistry::new();
                let name = stack.item.name(&registry);
                match value {
                    Some(v) => ctx.success(format!("{name}: trade value {v}")),
                    None => ctx.success(format!("{name}: not tradeable")),
                }
            }
            None => {
                ctx.success("Hold an item to see its trade value.".to_string());
            }
        }
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::item::{ItemStack, MaterialId};
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run_ctier(setup_inv: impl FnOnce(&mut PlayerSlot)) -> (CommandResult, Vec<String>) {
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = true;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        setup_inv(&mut players[0]);
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let r = {
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
            ComplexityTierCommand.execute(&mut ctx, &[])
        };
        let texts: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, texts)
    }

    #[test]
    fn complexity_tier_for_cake_is_five() {
        let (_, texts) = run_ctier(|p| {
            p.inventory.add_item(ItemStack::new_material(MaterialId::Cake, 1));
            p.hotbar_slot = 0;
        });
        assert!(texts.iter().any(|t| t.contains("Cake") && t.contains("tier 5")),
            "expected cake tier 5, got {texts:?}");
    }

    #[test]
    fn complexity_tier_for_empty_hand_returns_hint() {
        let (_, texts) = run_ctier(|_p| {});
        assert!(texts.iter().any(|t| t.contains("Hold an item")));
    }

    #[test]
    fn trade_value_for_wheat_is_one() {
        // Tier 0 raw → trade value 1.
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = true;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        players[0].inventory.add_item(ItemStack::new_material(MaterialId::Wheat, 1));
        players[0].hotbar_slot = 0;
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
        TradeValueCommand.execute(&mut ctx, &[]);
        let texts: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        assert!(texts.iter().any(|t| t.contains("trade value 1")),
            "expected wheat trade value 1, got {texts:?}");
    }

    #[test]
    fn neither_command_is_a_cheat() {
        assert!(!ComplexityTierCommand.is_cheat());
        assert!(!TradeValueCommand.is_cheat());
    }
}
