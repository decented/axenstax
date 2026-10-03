use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct SeedCommand;

impl Command for SeedCommand {
    fn name(&self) -> &'static str {
        "seed"
    }
    fn help(&self) -> &'static str {
        "Show the world seed"
    }
    fn usage(&self) -> &'static str {
        "/seed"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        ctx.success(format!("Seed: {}", ctx.seed));
        // Read-only; classify as Silent so dispatch's cheat path stays clean
        // even though is_cheat() is already false.
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    #[test]
    fn returns_silent_with_seed_in_log() {
        let cmd = SeedCommand;
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
            seed: 12345,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: OpLevel::None,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let r = cmd.execute(&mut ctx, &[]);
        assert_eq!(r, CommandResult::Silent);
        assert!(
            log.iter().any(|l| l.text.contains("12345")),
            "expected seed in log, got: {:?}",
            log.iter().map(|l| l.text.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn is_not_a_cheat() {
        // Read-only metadata; survival players can run it without tainting
        // the World Integrity Ledger.
        assert!(!SeedCommand.is_cheat());
        assert_eq!(SeedCommand.min_op_level(), OpLevel::None);
    }
}
