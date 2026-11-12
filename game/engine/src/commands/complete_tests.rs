use super::*;
use crate::commands::test_support::{full_registry, run_named};

fn at(reg: &CommandRegistry, op: OpLevel, line: &str) -> Completion {
    let ctx = CompletionContext { op_level: op, registry: reg, roster_names: &[] };
    complete(line, &ctx).unwrap_or_else(|| panic!("'{line}' got no completion"))
}

fn names(c: &Completion) -> Vec<&str> {
    c.candidates.iter().map(String::as_str).collect()
}

#[test]
fn plain_text_gets_no_completion() {
    let reg = full_registry();
    let ctx = CompletionContext { op_level: OpLevel::Op, registry: &reg, roster_names: &[] };
    assert_eq!(complete("hello there", &ctx), None);
    assert_eq!(complete("", &ctx), None);
    assert_eq!(complete("give to", &ctx), None);
}

#[test]
fn a_command_prefix_completes_names_and_aliases() {
    let reg = full_registry();
    let c = at(&reg, OpLevel::Op, "/gi");
    assert!(c.candidates.contains(&"give".to_string()), "{:?}", c.candidates);
    assert_eq!(c.replace_from, 1);
    // An alias completes too, and the match ignores case.
    let c = at(&reg, OpLevel::Op, "/G");
    assert!(names(&c).contains(&"gm") && names(&c).contains(&"gamemode") && names(&c).contains(&"give"));
    assert_eq!(at(&reg, OpLevel::Op, "/GI").candidates, vec!["give"]);
    // Alphabetical, no duplicates.
    let mut sorted = c.candidates.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(c.candidates, sorted);
}

#[test]
fn a_bare_slash_lists_every_command_the_player_may_run() {
    let reg = full_registry();
    let op = at(&reg, OpLevel::Op, "/");
    let none = at(&reg, OpLevel::None, "/");
    assert!(names(&op).contains(&"give") && names(&op).contains(&"help"));
    assert!(names(&none).contains(&"help") && names(&none).contains(&"waypoint"));
    assert!(!names(&none).contains(&"give") && !names(&none).contains(&"gm"));
    assert!(none.candidates.len() < op.candidates.len());
}

#[test]
fn an_op_gated_command_is_hidden_at_op_level_none() {
    let reg = full_registry();
    assert!(!names(&at(&reg, OpLevel::None, "/gi")).contains(&"give"));
    // Neither its name nor its arguments leak: no candidates, no usage hint.
    let c = at(&reg, OpLevel::None, "/give tor");
    assert!(c.candidates.is_empty());
    assert_eq!(c.usage_hint, None);
    let c = at(&reg, OpLevel::None, "/gm ");
    assert!(c.candidates.is_empty() && c.usage_hint.is_none());
}

#[test]
fn give_completes_item_names_best_match_first() {
    let reg = full_registry();
    let c = at(&reg, OpLevel::Op, "/give tor");
    assert_eq!(c.candidates[0], "torch");
    assert_eq!(c.replace_from, "/give ".len());
    assert!(c.candidates.iter().all(|n| n.contains("tor")), "{:?}", c.candidates);
    // A trailing space completes the next, empty, word: everything.
    let all = at(&reg, OpLevel::Op, "/give ");
    assert_eq!(all.candidates.len(), crate::commands::builtins::give::item_names().len());
    assert_eq!(all.replace_from, 6);
    // The count is free text: no candidates, so the usage line is the hint.
    let count = at(&reg, OpLevel::Op, "/give torch ");
    assert!(count.candidates.is_empty());
    assert!(count.usage_hint.unwrap().starts_with("/give <item>"));
}

#[test]
fn gm_space_lists_the_modes() {
    let reg = full_registry();
    let c = at(&reg, OpLevel::Op, "/gm ");
    assert_eq!(names(&c), ["adventure", "creative", "spectator", "survival"]);
    assert_eq!(c.replace_from, 4);
    assert_eq!(names(&at(&reg, OpLevel::Op, "/gamemode su")), ["survival"]);
}

#[test]
fn time_completes_subcommands_then_values() {
    let reg = full_registry();
    assert_eq!(names(&at(&reg, OpLevel::Op, "/time ")), ["get", "set", "speed"]);
    assert_eq!(names(&at(&reg, OpLevel::Op, "/time set mid")), ["midnight"]);
    let c = at(&reg, OpLevel::Op, "/time set ");
    for kw in ["day", "noon", "night", "midnight"] {
        assert!(c.candidates.contains(&kw.to_string()), "{kw}");
    }
}

#[test]
fn word_start_matches_come_after_prefix_matches() {
    assert_eq!(
        matching(["redstone_torch", "torch", "torchflower", "stone"], "torch"),
        ["torch", "torchflower", "redstone_torch"]
    );
    // Case-insensitive, deduped, alphabetical within each group.
    assert_eq!(matching(["Zeta", "alpha", "alpha", "ALPHA2"], "AL"), ["ALPHA2", "alpha"]);
    assert_eq!(matching(["b", "a"], ""), ["a", "b"]);
    assert!(matching(["stone"], "x").is_empty());
}

#[test]
fn the_last_word_is_what_gets_replaced() {
    let reg = full_registry();
    // Several spaces, and a word that is not the first argument.
    let c = at(&reg, OpLevel::Op, "/we   set   sto");
    assert_eq!(c.replace_from, "/we   set   ".len());
    assert!(c.candidates.iter().any(|n| n == "stone"));
    // Unknown command: nothing to offer and nothing to hint.
    let c = at(&reg, OpLevel::Op, "/zzzz foo");
    assert!(c.candidates.is_empty() && c.usage_hint.is_none());
    // Inside an open quote the word rules do not apply.
    let ctx = CompletionContext { op_level: OpLevel::Op, registry: &reg, roster_names: &[] };
    assert_eq!(complete("/waypoint add \"my ba", &ctx), None);
}

/// Where a completer is asked: the command and the words before the one being
/// completed. Every command with a closed set of arguments has a row here.
const PROBES: &[(&str, &[&str])] = &[
    ("gamemode", &[]),
    ("time", &[]),
    ("time", &["set"]),
    ("keepinventory", &[]),
    ("give", &[]),
    ("spawn", &[]),
    ("place", &[]),
    ("scenario", &[]),
    ("trial", &[]),
    ("help", &[]),
    ("waypoint", &[]),
    ("we", &[]),
    ("we", &["set"]),
    ("we", &["replace"]),
    ("we", &["replace", "stone"]),
    ("we", &["walls"]),
    ("we", &["stack", "3"]),
    ("buildguide", &[]),
    ("buildguide", &["mode"]),
    ("ws", &[]),
    ("ws", &["play"]),
    ("exhibit", &[]),
    ("exhibit", &["place", "pic"]),
    #[cfg(not(target_arch = "wasm32"))]
    ("room", &[]),
    #[cfg(not(target_arch = "wasm32"))]
    ("online", &[]),
];

/// Does this output say the command did not recognise a word?
fn says_unrecognised(command: &str, log: &[String]) -> bool {
    log.iter().any(|l| {
        let l = l.to_lowercase();
        l.contains("unknown")
            || l.contains("can't parse")
            || l.contains("not a placeable")
            || l.contains("(got ")
            // The catch-all arms of these two print their whole usage.
            || (command == "we" && l == "usage: /we pos1|pos2|set|replace|walls|copy|paste|stack|size|clear")
            || (command == "waypoint"
                && l == "usage: /waypoint add <name> | list | remove <name> | tp <name>")
    })
}

#[test]
fn every_probe_offers_something_and_every_suggestion_is_accepted_by_its_command() {
    let reg = full_registry();
    let ctx = CompletionContext { op_level: OpLevel::Op, registry: &reg, roster_names: &[] };
    for (name, before) in PROBES {
        let cmd = reg.lookup(name).unwrap_or_else(|| panic!("no command {name}"));
        let all = cmd.complete(before, "", &ctx);
        assert!(!all.is_empty(), "/{name} {before:?} offers nothing");
        // Big lists (items, blocks) are covered in full by the give tests;
        // sample them here so this stays quick.
        let step = (all.len() / 40).max(1);
        for cand in all.iter().step_by(step) {
            let mut args: Vec<&str> = before.to_vec();
            args.push(cand);
            let (_, log) = run_named(name, &args, OpLevel::Op);
            assert!(
                !says_unrecognised(name, &log),
                "/{name} {args:?} was suggested but its own command rejects it: {log:?}"
            );
        }
    }
}

#[test]
fn the_filtering_completers_agree_with_the_ranking_rule() {
    let reg = full_registry();
    let ctx = CompletionContext { op_level: OpLevel::Op, registry: &reg, roster_names: &[] };
    for (name, before) in PROBES {
        let cmd = reg.lookup(name).unwrap();
        let all = cmd.complete(before, "", &ctx);
        for partial in ["s", "t", "mo"] {
            let got = cmd.complete(before, partial, &ctx);
            assert_eq!(got, matching(&all, partial), "/{name} {before:?} + '{partial}'");
        }
    }
}
