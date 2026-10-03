//! `/reach` — print the engine's reach constants. Block placement /
//! breaking uses `REACH_DISTANCE` (5 blocks); melee combat uses
//! `ATTACK_REACH` (3 blocks). Read-only.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

/// Mirrored from `main.rs::REACH_DISTANCE` (5.0) + `combat.rs::ATTACK_REACH` (3.0).
/// Lifted here so adding a `/reach` command doesn't require exposing
/// the constants — the values move in lock-step.
const REACH_DISTANCE: f32 = 5.0;
const ATTACK_REACH: f32 = 3.0;

pub struct ReachCommand;

impl Command for ReachCommand {
    fn name(&self) -> &'static str { "reach" }
    fn help(&self) -> &'static str { "Print the player's current reach + attack reach" }
    fn usage(&self) -> &'static str { "/reach" }
    fn min_op_level(&self) -> OpLevel { OpLevel::None }
    fn is_cheat(&self) -> bool { false }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        // Task 13 added a per-tool modifier (the Reach Claw, +2.0 while held
        // in the active hotbar slot) — read the player's actual held item so
        // this stays true instead of hardcoding the old "flat constants"
        // claim, which the Reach Claw now falsifies.
        let player = &ctx.players[ctx.player_idx];
        let held_item = player
            .inventory
            .slot(player.hotbar_slot)
            .map(|stack| &stack.item);
        let bonus = held_item.map_or(0.0, |item| {
            crate::game_loop::reach_bonus_for_item_ref(crate::inventory::item_to_ref(item))
        });
        let effective_reach = REACH_DISTANCE + bonus;
        if bonus > 0.0 {
            ctx.success(format!(
                "Block reach: {:.1} blocks (base {:.1} + {:.1} from a held Reach Claw)",
                effective_reach, REACH_DISTANCE, bonus
            ));
        } else {
            ctx.success(format!("Block reach: {:.1} blocks", effective_reach));
        }
        ctx.success(format!("Attack reach: {:.1} blocks", ATTACK_REACH));
        ctx.success(
            "Some tools grant extra block reach (e.g. the Reach Claw, +2.0 while held)."
                .to_string(),
        );
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run_with_players(mut players: Vec<PlayerSlot>) -> Vec<String> {
        let cmd = ReachCommand;
        let mut world = World::new();
        let mut t = 0u32; let mut s = 4u32;
        let mut creative = false;
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
        let _ = cmd.execute(&mut ctx, &[]);
        log.iter().map(|l| l.text.clone()).collect()
    }

    fn run() -> Vec<String> {
        run_with_players(vec![PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 0.5)])
    }

    #[test]
    fn reports_block_and_attack_reach() {
        let texts = run();
        assert!(texts.iter().any(|t| t.contains("Block reach")));
        assert!(texts.iter().any(|t| t.contains("Attack reach")));
    }

    #[test]
    fn empty_handed_reports_base_reach_and_no_stale_no_modifier_claim() {
        let texts = run();
        assert!(texts.iter().any(|t| t.contains("Block reach: 5.0 blocks")));
        // Review fix: the old copy claimed "no per-tool reach modifier in
        // alpha" — false since Task 13 shipped the Reach Claw. Must be gone.
        assert!(
            !texts.iter().any(|t| t.contains("no per-tool reach modifier")),
            "stale false claim must not survive: {texts:?}"
        );
    }

    #[test]
    fn holding_a_reach_claw_reports_the_boosted_effective_reach() {
        use crate::item::{ItemStack, MaterialId};
        let mut player = PlayerSlot::new(0, glam::Vec3::new(0.0, 70.0, 0.0), 0.5);
        player.hotbar_slot = 0;
        player
            .inventory
            .set_slot(0, Some(ItemStack::new_material(MaterialId::ReachClaw, 1)));
        let texts = run_with_players(vec![player]);
        assert!(
            texts.iter().any(|t| t.contains("Block reach: 7.0 blocks")),
            "expected the +2.0 Reach Claw bonus applied on top of the 5.0 base: {texts:?}"
        );
        assert!(texts.iter().any(|t| t.contains("Reach Claw")));
    }
}
