use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct HealCommand;

impl Command for HealCommand {
    fn name(&self) -> &'static str {
        "heal"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
    fn help(&self) -> &'static str {
        "Restore the player to full health"
    }
    fn usage(&self) -> &'static str {
        "/heal"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        if !args.is_empty() {
            let msg = "usage: /heal".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
        let Some(slot) = ctx.players.get_mut(ctx.player_idx) else {
            let msg = "no player slot to heal".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        if slot.combat.dead {
            let msg = "can't heal — you're dead. Wait for respawn or click Respawn.".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
        if slot.combat.health >= slot.combat.max_health {
            ctx.info("already at full health".to_string());
            return CommandResult::Silent;
        }
        let restored = slot.combat.heal(slot.combat.max_health);
        // /heal bypasses normal play (you have to find food) — flag as cheat.
        *ctx.cheats_used_marker = true;
        *ctx.pure_survival_broken_marker = true;
        ctx.success(format!("Restored {restored:.1} HP"));
        CommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str]) -> (CommandResult, Vec<PlayerSlot>, bool, bool) {
        let cmd = HealCommand;
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
        (r, players, ch, ps)
    }

    #[test]
    fn heal_restores_to_max() {
        let cmd = HealCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        // Damage the player first.
        players[0].combat.take_damage(8.0);
        assert_eq!(players[0].combat.health, 12.0);
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
        assert_eq!(players[0].combat.health, players[0].combat.max_health);
        assert!(ch, "heal must mark cheats_used");
        assert!(ps, "heal must break pure-survival");
    }

    #[test]
    fn heal_at_full_is_silent_no_cheat() {
        let (r, _players, ch, ps) = run(&[]);
        assert_eq!(r, CommandResult::Silent);
        assert!(!ch, "no-op heal must not mark cheats_used");
        assert!(!ps, "no-op heal must not break pure-survival");
    }

    #[test]
    fn heal_dead_player_errors() {
        let cmd = HealCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        players[0].combat.take_damage(25.0);
        assert!(players[0].combat.dead);
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
        assert!(matches!(cmd.execute(&mut ctx, &[]), CommandResult::Error(_)));
    }

    #[test]
    fn heal_rejects_args() {
        assert!(matches!(run(&["50"]).0, CommandResult::Error(_)));
    }
}
