//! Tab completion for chat commands (Minecraft style). Pure logic: given the
//! line typed so far it says what could finish the last word. `chat_ui` only
//! draws the candidates and applies the one the player picks.
//!
//! * Only a line that starts with `/` is completed; plain chat gets nothing.
//! * The word being completed is the last whitespace-separated token. A line
//!   that ends in a space is completing the next, empty, word.
//! * Word 0 (right after the `/`) completes command names and aliases the
//!   player may run at their current op level. Later words ask the command
//!   itself, through `Command::complete`.
//! * No type here is native-only, so the module compiles on wasm and Android.

use super::registry::{CommandRegistry, OpLevel};

/// What a completer may need to know about the game. Kept to plain data so it
/// compiles on every target.
pub struct CompletionContext<'a> {
    /// The player's current op level (joined worlds are `OpLevel::None`).
    pub op_level: OpLevel,
    /// Every registered command (for `/help <command>` and word 0).
    pub registry: &'a CommandRegistry,
    /// Handles of the players in a joined world; empty in single-player.
    // BRIDGE: no built-in command takes a player name yet (`/tp` is coordinates
    // only) — replace this with a real read when one does.
    #[allow(dead_code)]
    pub roster_names: &'a [String],
}

/// The result of completing one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Byte index into the line where the word being completed starts. A
    /// picked candidate replaces `line[replace_from..]`.
    pub replace_from: usize,
    /// What could finish the word, best match first. Empty when nothing can.
    pub candidates: Vec<String>,
    /// The command's usage line, set only when there are no candidates and the
    /// command is recognised and allowed.
    pub usage_hint: Option<String>,
}

/// Rank `values` against what has been typed of the word, for argument values.
/// Case-insensitive. First the values that start with `partial`, then the
/// values where any `_`-separated word starts with it (so `torch` also offers
/// `redstone_torch`); each group alphabetical; no duplicates.
pub fn matching<I, S>(values: I, partial: &str) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let want = partial.to_lowercase();
    let mut prefix: Vec<String> = Vec::new();
    let mut word: Vec<String> = Vec::new();
    for v in values {
        let v = v.as_ref();
        let lower = v.to_lowercase();
        if lower.starts_with(&want) {
            prefix.push(v.to_string());
        } else if lower.split('_').any(|w| w.starts_with(&want)) {
            word.push(v.to_string());
        }
    }
    for group in [&mut prefix, &mut word] {
        group.sort();
        group.dedup();
    }
    prefix.extend(word);
    prefix
}

/// Complete `line`. `None` when the line is not a command (does not start with
/// `/`) or the cursor sits inside an open quote.
pub fn complete(line: &str, ctx: &CompletionContext) -> Option<Completion> {
    let body = line.strip_prefix('/')?;
    // An odd number of quotes means the last word is inside a quoted argument
    // (a name with spaces); the word rules below would be wrong there.
    if line.matches('"').count() % 2 == 1 {
        return None;
    }
    // The last word starts after the last whitespace character.
    let replace_from = match body.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
        Some((i, c)) => 1 + i + c.len_utf8(),
        None => 1,
    };
    let partial = &line[replace_from..];
    let before: Vec<&str> = line[1..replace_from].split_whitespace().collect();

    let Some((name, args_before)) = before.split_first() else {
        return Some(Completion {
            replace_from,
            candidates: command_names(partial, ctx),
            usage_hint: None,
        });
    };

    let command = ctx.registry.lookup(&name.to_lowercase());
    // A command the player may not run is not revealed at all.
    let command = command.filter(|c| ctx.op_level >= c.min_op_level());
    let Some(command) = command else {
        return Some(Completion { replace_from, candidates: Vec::new(), usage_hint: None });
    };
    let mut candidates = command.complete(args_before, partial, ctx);
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert(c.clone()));
    let usage_hint = candidates.is_empty().then(|| command.usage().to_string());
    Some(Completion { replace_from, candidates, usage_hint })
}

/// Command names and aliases the player may run, prefix-matched on what they
/// have typed after the `/`; alphabetical, no duplicates.
fn command_names(partial: &str, ctx: &CompletionContext) -> Vec<String> {
    let want = partial.to_lowercase();
    let mut out: Vec<String> = Vec::new();
    for cmd in ctx.registry.iter() {
        if ctx.op_level < cmd.min_op_level() {
            continue;
        }
        for name in std::iter::once(cmd.name()).chain(cmd.aliases().iter().copied()) {
            if name.to_lowercase().starts_with(&want) {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
#[path = "complete_tests.rs"]
mod tests;
