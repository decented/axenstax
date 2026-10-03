use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct ClearCommand;

impl Command for ClearCommand {
    fn name(&self) -> &'static str {
        "clear"
    }
    fn help(&self) -> &'static str {
        "Empty your inventory"
    }
    fn usage(&self) -> &'static str {
        "/clear"
    }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        let Some(slot) = ctx.players.get_mut(ctx.player_idx) else {
            let msg = "no player slot to clear".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        for i in 0..36 {
            slot.inventory.set_slot(i, None);
        }
        ctx.success("Inventory cleared".to_string());
        CommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::item::ItemStack;
    use crate::player_slot::PlayerSlot;

    #[test]
    fn empties_all_slots() {
        let cmd = ClearCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        // Pre-fill a couple of slots.
        players[0].inventory.set_slot(0, Some(ItemStack::new_block(block::STONE, 5)));
        players[0].inventory.set_slot(20, Some(ItemStack::new_block(block::DIRT, 8)));
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
        let r = cmd.execute(&mut ctx, &[]);
        assert_eq!(r, CommandResult::Success);
        for i in 0..36 {
            assert!(players[0].inventory.slot(i).is_none(), "slot {i} not cleared");
        }
    }
}
