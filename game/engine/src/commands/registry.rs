//! Command registry — trait + storage. Plugin-shaped: any type implementing
//! `Command` can be registered. Registration order is preserved for /help
//! listing.

use std::collections::{HashMap, HashSet};

use super::dispatch::{CommandContext, CommandResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpLevel {
    None = 0,
    Op = 1,
}

pub trait Command: Send + Sync {
    fn name(&self) -> &'static str;
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
    /// One-line description shown in `/help` listings.
    fn help(&self) -> &'static str;
    /// Concrete usage line shown in `/help <command>`.
    fn usage(&self) -> &'static str;
    fn min_op_level(&self) -> OpLevel {
        OpLevel::Op
    }
    /// True if a successful execution should set `WorldMeta.cheats_used`.
    fn is_cheat(&self) -> bool {
        true
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult;
}

pub struct CommandRegistry {
    /// Insertion-ordered list of (primary_name, command).
    order: Vec<&'static str>,
    by_name: HashMap<&'static str, Box<dyn Command>>,
    /// Maps alias → primary name. Lookup by alias re-resolves through here.
    alias_to_primary: HashMap<&'static str, &'static str>,
    /// Primary names switched off at runtime (see [`CommandRegistry::set_hidden`]).
    /// A hidden command is registered but behaves exactly as if it were not:
    /// `lookup` misses, `iter` skips it, so dispatch, `/help` and `/help <cmd>`
    /// all answer as for any unknown command.
    hidden: HashSet<&'static str>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            by_name: HashMap::new(),
            alias_to_primary: HashMap::new(),
            hidden: HashSet::new(),
        }
    }

    /// Hide (or un-hide) a registered command by primary name. Hidden commands
    /// are invisible to `lookup` and `iter` — indistinguishable from unknown —
    /// while staying compiled and registered, so un-hiding is instant. Used for
    /// feature-gated commands (the alpha-tester feedback commands).
    pub fn set_hidden(&mut self, name: &'static str, hidden: bool) {
        if hidden {
            self.hidden.insert(name);
        } else {
            self.hidden.remove(name);
        }
    }

    /// Register a command. If the primary name or any alias collides with an
    /// existing entry, the new command wins for that key (later wins).
    pub fn register(&mut self, cmd: Box<dyn Command>) {
        let name = cmd.name();
        let aliases = cmd.aliases();

        if !self.by_name.contains_key(name) {
            self.order.push(name);
        }
        // Move new aliases first; existing aliases pointing at this name stay.
        for &alias in aliases {
            self.alias_to_primary.insert(alias, name);
        }
        self.by_name.insert(name, cmd);
    }

    pub fn lookup(&self, name: &str) -> Option<&dyn Command> {
        // Direct primary-name hit (HashMap key is &'static str; we compare
        // owned strings via the typed key).
        if let Some(boxed) = self.by_name.get(name) {
            return (!self.hidden.contains(boxed.name())).then(|| boxed.as_ref());
        }
        // Alias resolution.
        if let Some(primary) = self.alias_to_primary.get(name) {
            if self.hidden.contains(*primary) {
                return None;
            }
            return self.by_name.get(*primary).map(|b| b.as_ref());
        }
        None
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Command> {
        self.order
            .iter()
            .filter(move |n| !self.hidden.contains(**n))
            .filter_map(move |n| self.by_name.get(*n).map(|b| b.as_ref()))
    }

    // Convenience accessors. Currently only used in tests, but part of the
    // intended public surface for any consumer wanting to introspect the
    // registry.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::dispatch::{CommandContext, CommandResult};

    struct Fake {
        name: &'static str,
        aliases: &'static [&'static str],
        op: OpLevel,
        cheat: bool,
    }
    impl Command for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn aliases(&self) -> &'static [&'static str] {
            self.aliases
        }
        fn help(&self) -> &'static str {
            "fake"
        }
        fn usage(&self) -> &'static str {
            "/fake"
        }
        fn min_op_level(&self) -> OpLevel {
            self.op
        }
        fn is_cheat(&self) -> bool {
            self.cheat
        }
        fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> CommandResult {
            CommandResult::Success
        }
    }

    fn fake(name: &'static str, aliases: &'static [&'static str]) -> Box<dyn Command> {
        Box::new(Fake {
            name,
            aliases,
            op: OpLevel::Op,
            cheat: true,
        })
    }

    #[test]
    fn register_and_lookup_by_primary() {
        let mut r = CommandRegistry::new();
        r.register(fake("alpha", &[]));
        assert!(r.lookup("alpha").is_some());
        assert!(r.lookup("beta").is_none());
    }

    #[test]
    fn aliases_resolve() {
        let mut r = CommandRegistry::new();
        r.register(fake("gamemode", &["gm"]));
        assert_eq!(r.lookup("gm").unwrap().name(), "gamemode");
        assert_eq!(r.lookup("gamemode").unwrap().name(), "gamemode");
    }

    #[test]
    fn iter_preserves_insertion_order() {
        let mut r = CommandRegistry::new();
        r.register(fake("zebra", &[]));
        r.register(fake("alpha", &[]));
        r.register(fake("middle", &[]));
        let names: Vec<_> = r.iter().map(|c| c.name()).collect();
        assert_eq!(names, vec!["zebra", "alpha", "middle"]);
    }

    #[test]
    fn alias_collision_later_wins() {
        let mut r = CommandRegistry::new();
        r.register(fake("first", &["x"]));
        r.register(fake("second", &["x"]));
        // x now points to second.
        assert_eq!(r.lookup("x").unwrap().name(), "second");
        // Originals still resolvable by their primary names.
        assert_eq!(r.lookup("first").unwrap().name(), "first");
        assert_eq!(r.lookup("second").unwrap().name(), "second");
    }

    #[test]
    fn re_register_same_primary_replaces_command() {
        let mut r = CommandRegistry::new();
        r.register(fake("dup", &[]));
        let len_after_first = r.len();
        r.register(fake("dup", &[]));
        assert_eq!(r.len(), len_after_first); // no duplicate entry in order
    }

    #[test]
    fn hidden_commands_are_invisible_to_lookup_aliases_and_iter() {
        let mut r = CommandRegistry::new();
        r.register(fake("secret", &["s"]));
        r.register(fake("open", &[]));
        r.set_hidden("secret", true);
        assert!(r.lookup("secret").is_none());
        assert!(r.lookup("s").is_none(), "an alias must not leak a hidden command");
        assert!(r.lookup("open").is_some());
        let names: Vec<_> = r.iter().map(|c| c.name()).collect();
        assert_eq!(names, vec!["open"]);
        // Un-hiding restores it without re-registering.
        r.set_hidden("secret", false);
        assert!(r.lookup("secret").is_some());
        assert!(r.lookup("s").is_some());
        assert_eq!(r.iter().count(), 2);
    }

    #[test]
    fn op_levels_compare() {
        assert!(OpLevel::Op > OpLevel::None);
    }
}
