//! `/online` — show this world's invite link and how reachable it is, or copy
//! the link to the clipboard.
//!
//! Like every other command that needs the `HostedServer` (`/room`,
//! `/spawncart`), this one only decides WHAT to do and returns it as a
//! [`CommandResult`]; the game loop, which owns `self.online_host`, does it.
//! `CommandContext` is deliberately too narrow to reach the server.
//!
//! Registered NATIVE ONLY: there is no online play on the web taster.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct OnlineCommand;

impl OnlineCommand {
    /// The decision, split out so it is testable without a `CommandContext`.
    pub fn result_for(&self, args: &[String]) -> CommandResult {
        match args.first().map(|s| s.to_lowercase()).as_deref() {
            None => CommandResult::OnlineStatus,
            Some("copy") => CommandResult::OnlineCopyInvite,
            Some(other) => CommandResult::Error(format!(
                "unknown /online subcommand '{other}' — usage: /online | /online copy"
            )),
        }
    }
}

impl Command for OnlineCommand {
    fn name(&self) -> &'static str {
        "online"
    }
    fn help(&self) -> &'static str {
        "Show your invite link and whether friends can reach you"
    }
    fn usage(&self) -> &'static str {
        "/online | /online copy"
    }
    fn min_op_level(&self) -> OpLevel {
        // Whoever is hosting may see their own invite; it is theirs.
        OpLevel::Op
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, _ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        self.result_for(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::Command;

    #[test]
    fn the_command_is_named_online_and_is_not_a_cheat() {
        let c = OnlineCommand;
        assert_eq!(c.name(), "online");
        assert!(!c.is_cheat(), "showing your own invite link is not a cheat");
    }

    #[test]
    fn its_usage_names_both_subcommands() {
        let c = OnlineCommand;
        assert!(c.usage().contains("/online"));
        assert!(c.usage().contains("copy"));
    }

    #[test]
    fn no_argument_asks_for_status_and_copy_asks_to_copy() {
        // `execute` is exercised through the enum it returns, which is what the
        // game loop acts on — CommandContext is too wide to reach a
        // HostedServer, so every server-touching command works this way
        // (see /room, /spawncart).
        let c = OnlineCommand;
        assert!(matches!(c.result_for(&[]), CommandResult::OnlineStatus));
        assert!(matches!(
            c.result_for(&["copy".to_string()]),
            CommandResult::OnlineCopyInvite
        ));
    }

    #[test]
    fn an_unknown_subcommand_is_an_error_not_a_silent_status() {
        assert!(matches!(
            OnlineCommand.result_for(&["wat".to_string()]),
            CommandResult::Error(_)
        ));
    }
}
