//! `/timestep` — print the current world-time step (ticks of world
//! time advanced per game tick) + day length. Read-only diagnostic;
//! `/time speed N` sets the value.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct TimestepCommand;

impl Command for TimestepCommand {
    fn name(&self) -> &'static str { "timestep" }
    fn help(&self) -> &'static str {
        "Print the current world-time step + day length"
    }
    fn usage(&self) -> &'static str { "/timestep" }
    fn min_op_level(&self) -> OpLevel { OpLevel::None }
    fn is_cheat(&self) -> bool { false }
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
        let step = *ctx.world_time_step;
        let day_len_secs = if step == 0 {
            0
        } else {
            (24_000 / (step * 20)).max(1)
        };
        ctx.success(format!("world_time_step = {step}x"));
        ctx.success(format!("day length ≈ {day_len_secs}s @ 20 TPS"));
        ctx.success("use /time speed N to change".to_string());
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(step: u32) -> Vec<String> {
        let cmd = TimestepCommand;
        let mut world = World::new();
        let mut t = 0u32; let mut s = step;
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
        let _ = cmd.execute(&mut ctx, &[]);
        log.iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn reports_step_and_day_length() {
        let texts = run(4);
        assert!(texts.iter().any(|t| t.contains("4x")));
        // 24000 / (4*20) = 300s.
        assert!(texts.iter().any(|t| t.contains("300s")), "got {texts:?}");
    }

    #[test]
    fn reports_1x_default() {
        let texts = run(1);
        assert!(texts.iter().any(|t| t.contains("1x")));
        assert!(texts.iter().any(|t| t.contains("1200s")), "1x → 20 min day");
    }

    #[test]
    fn handles_zero_step_safely() {
        let texts = run(0);
        // Shouldn't panic on division-by-zero; reports 0s for day length.
        assert!(texts.iter().any(|t| t.contains("0x")));
    }
}
