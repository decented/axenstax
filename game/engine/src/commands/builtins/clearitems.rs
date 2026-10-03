use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;

pub struct ClearItemsCommand;

impl Command for ClearItemsCommand {
    fn name(&self) -> &'static str {
        "clearitems"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["clearground"]
    }
    fn help(&self) -> &'static str {
        "Despawn every dropped item entity in the world"
    }
    fn usage(&self) -> &'static str {
        "/clearitems"
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        if !args.is_empty() {
            let msg = "usage: /clearitems".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
        *ctx.cheats_used_marker = true;
        *ctx.pure_survival_broken_marker = true;
        ctx.success("Cleared all dropped items.".to_string());
        CommandResult::ClearItems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str]) -> (CommandResult, bool, bool) {
        let cmd = ClearItemsCommand;
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
        (r, ch, ps)
    }

    #[test]
    fn returns_clearitems_side_effect() {
        let (r, ch, ps) = run(&[]);
        assert_eq!(r, CommandResult::ClearItems);
        assert!(ch);
        assert!(ps);
    }

    #[test]
    fn rejects_args() {
        let (r, ch, _) = run(&["all"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!ch);
    }
}
