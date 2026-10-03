use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct GamemodeCommand;

impl Command for GamemodeCommand {
    fn name(&self) -> &'static str {
        "gamemode"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["gm"]
    }
    fn help(&self) -> &'static str {
        "Switch play mode: survival, creative, adventure, spectator"
    }
    fn usage(&self) -> &'static str {
        "/gamemode <survival|creative|adventure|spectator> | /gm <s|c|a|sp>"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        use crate::play_mode::PlayMode;
        let target = match args.first() {
            Some(s) => s.to_lowercase(),
            None => {
                let msg = "usage: /gamemode <survival|creative|adventure|spectator>".to_string();
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let mode = match target.as_str() {
            "survival" | "s" | "0" => PlayMode::Survival,
            "creative" | "c" | "1" => PlayMode::Creative,
            "adventure" | "a" | "2" => PlayMode::Adventure,
            "spectator" | "sp" | "spec" | "3" => PlayMode::Spectator,
            other => {
                let msg = format!(
                    "unknown mode: {other} (survival|creative|adventure|spectator)"
                );
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        if *ctx.play_mode == mode {
            ctx.info(format!("already in {}", mode.label().to_lowercase()));
            return CommandResult::Silent;
        }
        // Set the source of truth + keep the is_creative projection in lock-step.
        *ctx.play_mode = mode;
        *ctx.is_creative = mode.is_creative();
        // Flight: Creative+Spectator fly; grounded modes drop the player.
        if let Some(slot) = ctx.players.get_mut(ctx.player_idx) {
            slot.player.flying = mode.flies();
        }
        // Ledger: entering Creative is a one-way "ever creative" / breaks
        // "pure survival". Adventure/Spectator do NOT trip the ledger.
        if mode.is_creative() {
            *ctx.ever_creative_marker = true;
            *ctx.pure_survival_broken_marker = true;
        }
        ctx.success(format!("Switched to {}", mode.label().to_lowercase()));
        CommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play_mode::PlayMode;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str], mode: PlayMode) -> (CommandResult, PlayMode, bool, bool) {
        let cmd = GamemodeCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut pm = mode;
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let mut creative = mode.is_creative();
        let reg = CommandRegistry::new();
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative,
            play_mode: &mut pm,
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
        (r, pm, ev, ps)
    }

    #[test]
    fn switch_to_adventure() {
        let (r, pm, ev, ps) = run(&["adventure"], PlayMode::Survival);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(pm, PlayMode::Adventure);
        assert!(!ev); // entering adventure is NOT "ever creative"
        assert!(!ps);
    }

    #[test]
    fn switch_to_spectator() {
        let (r, pm, _, _) = run(&["spectator"], PlayMode::Survival);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(pm, PlayMode::Spectator);
    }

    #[test]
    fn creative_still_sets_ledger() {
        let (r, pm, ev, ps) = run(&["creative"], PlayMode::Survival);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(pm, PlayMode::Creative);
        assert!(ev);
        assert!(ps);
    }

    #[test]
    fn switch_to_survival_no_ledger() {
        let (r, pm, ev, ps) = run(&["survival"], PlayMode::Creative);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(pm, PlayMode::Survival);
        assert!(!ev);
        assert!(!ps);
    }

    #[test]
    fn already_in_target_is_silent() {
        let (r, pm, _, _) = run(&["survival"], PlayMode::Survival);
        assert_eq!(r, CommandResult::Silent);
        assert_eq!(pm, PlayMode::Survival);
    }

    #[test]
    fn shorthand_a_is_adventure() {
        let (r, pm, _, _) = run(&["a"], PlayMode::Survival);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(pm, PlayMode::Adventure);
    }

    #[test]
    fn shorthand_c_is_creative() {
        let (r, pm, _, _) = run(&["c"], PlayMode::Survival);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(pm, PlayMode::Creative);
    }

    #[test]
    fn no_args_errors() {
        let (r, _, _, _) = run(&[], PlayMode::Survival);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn unknown_mode_errors() {
        let (r, _, _, _) = run(&["banana"], PlayMode::Survival);
        assert!(matches!(r, CommandResult::Error(_)));
    }
}
