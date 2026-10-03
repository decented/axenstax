use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct HelpCommand;

impl Command for HelpCommand {
    fn name(&self) -> &'static str {
        "help"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["?"]
    }
    fn help(&self) -> &'static str {
        "List commands or show help for one"
    }
    fn usage(&self) -> &'static str {
        "/help [command]"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        if let Some(target) = args.first() {
            // Strip a leading / so `/help /time` and `/help time` both work.
            let key = target.trim_start_matches('/').to_lowercase();
            match ctx.registry.lookup(&key) {
                Some(cmd) => {
                    ctx.info(format!("/{} — {}", cmd.name(), cmd.help()));
                    ctx.info(format!("usage: {}", cmd.usage()));
                    if !cmd.aliases().is_empty() {
                        ctx.info(format!("aliases: {}", cmd.aliases().join(", ")));
                    }
                    CommandResult::Success
                }
                None => {
                    let msg = format!("unknown command: /{key}");
                    ctx.error(msg.clone());
                    CommandResult::Error(msg)
                }
            }
        } else {
            ctx.info("Available commands:".to_string());
            // List commands the player has permission to run.
            for cmd in ctx.registry.iter() {
                if ctx.op_level >= cmd.min_op_level() {
                    ctx.info(format!("  /{:<10}  {}", cmd.name(), cmd.help()));
                }
            }
            ctx.info("type /help <command> for usage details".to_string());
            CommandResult::Success
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(
        registry: &CommandRegistry,
        op: OpLevel,
        args: &[&str],
    ) -> (CommandResult, Vec<String>) {
        let cmd = HelpCommand;
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let mut ctx = CommandContext {
            world: &mut world,
            world_time: &mut t,
            world_time_step: &mut s,
            is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
            seed: 0,
            world_name: "test",
            players: &mut players,
            player_idx: 0,
            op_level: op,
            current_tick: 0,
            log: &mut log,
            registry,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let r = cmd.execute(&mut ctx, &owned);
        let lines: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, lines)
    }

    /// Help only sees commands the player has permission to run, so we need
    /// at least one command in the registry to test against.
    fn registry_with_builtins() -> CommandRegistry {
        let mut r = CommandRegistry::new();
        crate::commands::builtins::register_all(&mut r);
        r
    }

    #[test]
    fn no_args_lists_all_visible_commands() {
        let registry = registry_with_builtins();
        let (r, lines) = run(&registry, OpLevel::Op, &[]);
        assert_eq!(r, CommandResult::Success);
        // Header + at least the 7 built-ins as Op.
        assert!(lines.iter().any(|l| l.contains("Available commands:")));
        for name in ["help", "time", "gamemode", "tp", "seed", "clear", "give"] {
            assert!(
                lines.iter().any(|l| l.contains(&format!("/{}", name))),
                "expected /{name} in help output, got {lines:?}"
            );
        }
    }

    #[test]
    fn no_args_filters_by_op_level() {
        // Non-op players can only see commands with min_op_level = None.
        // Today that's /help + /seed + the debug/read-only commands
        // (/biome, /biomes, /mobs, /tools, /armour, /reach, /timestep,
        // /recipes, /complexitytier, /tradevalue).
        let registry = registry_with_builtins();
        let (r, lines) = run(&registry, OpLevel::None, &[]);
        assert_eq!(r, CommandResult::Success);
        assert!(lines.iter().any(|l| l.contains("/help")));
        assert!(lines.iter().any(|l| l.contains("/seed")));
        // Op-only commands shouldn't appear. Match with a trailing
        // word boundary so /time doesn't match /timestep — split each
        // help line by ASCII whitespace and check exact-name.
        let visible_cmds: Vec<&str> = lines.iter()
            .flat_map(|l| l.split_ascii_whitespace())
            .filter(|tok| tok.starts_with('/'))
            .map(|tok| tok.trim_start_matches('/'))
            .collect();
        for name in ["time", "gamemode", "tp", "clear", "give"] {
            assert!(
                !visible_cmds.contains(&name),
                "non-op player saw /{name} in help output: {lines:?}"
            );
        }
    }

    #[test]
    fn looks_up_specific_command() {
        let registry = registry_with_builtins();
        let (r, lines) = run(&registry, OpLevel::Op, &["time"]);
        assert_eq!(r, CommandResult::Success);
        assert!(lines.iter().any(|l| l.contains("/time")));
        assert!(lines.iter().any(|l| l.contains("usage:")));
    }

    #[test]
    fn slash_prefix_stripped_from_lookup_arg() {
        // /help /time and /help time should both work.
        let registry = registry_with_builtins();
        let (r, lines) = run(&registry, OpLevel::Op, &["/time"]);
        assert_eq!(r, CommandResult::Success);
        assert!(lines.iter().any(|l| l.contains("usage:")));
    }

    #[test]
    fn unknown_command_errors() {
        let registry = registry_with_builtins();
        let (r, lines) = run(&registry, OpLevel::Op, &["banana"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(lines.iter().any(|l| l.contains("unknown command")));
    }

    #[test]
    fn is_not_a_cheat() {
        assert!(!HelpCommand.is_cheat());
        assert_eq!(HelpCommand.min_op_level(), OpLevel::None);
    }

    #[test]
    fn shows_aliases_for_aliased_command() {
        let registry = registry_with_builtins();
        let (r, lines) = run(&registry, OpLevel::Op, &["gamemode"]);
        assert_eq!(r, CommandResult::Success);
        assert!(lines.iter().any(|l| l.contains("aliases")));
    }
}
