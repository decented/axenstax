use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct KillCommand;

impl Command for KillCommand {
    fn name(&self) -> &'static str {
        "kill"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["suicide"]
    }
    fn help(&self) -> &'static str {
        "Instantly kill yourself (triggers death+respawn loop)"
    }
    fn usage(&self) -> &'static str {
        "/kill"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        if !args.is_empty() {
            let msg = "usage: /kill".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
        let Some(slot) = ctx.players.get_mut(ctx.player_idx) else {
            let msg = "no player slot to kill".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        if slot.combat.dead {
            ctx.info("already dead".to_string());
            return CommandResult::Silent;
        }
        // Funnel through take_damage so just_died fires + the rest of the
        // death loop (inventory drop, respawn timer) runs normally.
        slot.combat.take_damage(slot.combat.max_health + 1.0);
        // /kill is a deliberate cheat for testing the death loop.
        *ctx.cheats_used_marker = true;
        *ctx.pure_survival_broken_marker = true;
        ctx.success("You died.".to_string());
        CommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    #[test]
    fn kill_drops_health_to_zero_and_flags_dead() {
        let cmd = KillCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world, world_time: &mut t, world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival }, seed: 42, world_name: "test",
            players: &mut players, player_idx: 0, op_level: OpLevel::Op,
            current_tick: 0, log: &mut log, registry: &reg,
            cheats_used_marker: &mut ch, ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        assert_eq!(cmd.execute(&mut ctx, &[]), CommandResult::Success);
        assert_eq!(players[0].combat.health, 0.0);
        assert!(players[0].combat.dead);
        assert!(players[0].combat.just_died, "must fire just_died for the inventory-drop handler");
        assert!(ch, "must mark cheats_used");
        assert!(ps, "must break pure-survival");
    }

    #[test]
    fn kill_already_dead_is_silent() {
        let cmd = KillCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        players[0].combat.take_damage(25.0);
        // Consume just_died so we can verify /kill doesn't re-arm it.
        players[0].combat.just_died = false;
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world, world_time: &mut t, world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival }, seed: 42, world_name: "test",
            players: &mut players, player_idx: 0, op_level: OpLevel::Op,
            current_tick: 0, log: &mut log, registry: &reg,
            cheats_used_marker: &mut ch, ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        assert_eq!(cmd.execute(&mut ctx, &[]), CommandResult::Silent);
        assert!(!ch, "no-op /kill must not mark cheats_used");
        assert!(!players[0].combat.just_died, "must not re-arm just_died");
    }

    #[test]
    fn kill_rejects_args() {
        let cmd = KillCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world, world_time: &mut t, world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival }, seed: 42, world_name: "test",
            players: &mut players, player_idx: 0, op_level: OpLevel::Op,
            current_tick: 0, log: &mut log, registry: &reg,
            cheats_used_marker: &mut ch, ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        assert!(matches!(cmd.execute(&mut ctx, &["self".to_string()]), CommandResult::Error(_)));
    }
}
