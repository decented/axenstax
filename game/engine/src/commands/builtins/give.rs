use crate::commands::complete::{matching, CompletionContext};
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;
use crate::crafting::Tool;
use crate::item::ItemStack;

// The name tables live in `give_names`: the resolver below and Tab completion
// both read them, so a name cannot be accepted but not suggested.
pub use super::give_names::{armour_set_names, block_names, item_names};
use super::give_names::{
    armour_by_name, armour_set, block_by_name, bow_by_name, closest_name, is_near_typo, material_by_name, tool_by_name,
    ArmourWant,
};

pub struct GiveCommand;

/// Resolve a player-typed item name to a constructed `ItemStack`.
/// Returns `Err(message)` for unknown items.
///
/// `pub(crate)` so the scenario runner (`scenario.rs` + the `/scenario`
/// handler) can provision a `ScenarioDef.kit` through the same vocabulary the
/// `/give` command uses — one resolver, no duplicate item tables.
pub(crate) fn resolve_item(name: &str, count: u8) -> Result<ItemStack, String> {
    match resolve_known(name, count)? {
        Some(stack) => Ok(stack),
        None => Err(unknown_item(name, &normalise(name))),
    }
}

/// [`resolve_item`] without the unknown-name message: `Ok(None)` for a name nothing names. The
/// message reads the suggestion list, and the suggestion list is built by resolving names, so the
/// list's own builder must use this (a name that fails there would otherwise wait on the list it
/// is building).
pub(super) fn resolve_known(name: &str, count: u8) -> Result<Option<ItemStack>, String> {
    let key = normalise(name);
    if let Some(block_id) = block_by_name(&key) {
        return Ok(Some(ItemStack::new_block(block_id, count.max(1))));
    }
    if let Some((tt, tm)) = tool_by_name(&key) {
        // Tools don't stack; ignore count.
        return Ok(Some(ItemStack::new_tool(Tool::new(tt, tm))));
    }
    if let Some((tt, tm)) = bow_by_name(&key) {
        return Ok(Some(ItemStack::new_tool(Tool::new(tt, tm))));
    }
    if let Some(mat) = material_by_name(&key) {
        return Ok(Some(ItemStack::new_material(mat, count.max(1))));
    }
    // Armour: one piece (`satori_helmet`, `iron boots`). Armour never stacks, so the count is
    // ignored here; `resolve_items` is the one that hands out several pieces or a whole set.
    match armour_by_name(&key) {
        Some(ArmourWant::Piece(slot, material)) => return Ok(Some(ItemStack::new_armour(slot, material))),
        Some(ArmourWant::Set(_)) => {
            return Err(format!("'{name}' is a whole armour set, not one item; /give hands it out piece by piece"));
        }
        None => {}
    }
    // Spec 24 — debug Plan item. 3×3 stone footprint, default name +
    // CC-BY-SA licence. Useful for testing Inspect / place / animated
    // build paths once those phases ship. Plans don't stack, so count
    // is ignored.
    if matches!(key.as_str(), "debug_plan" | "plan" | "test_plan") {
        return Ok(Some(crate::item::ItemStack {
            item: crate::item::Item::Plan(crate::plan::PlanData::debug_3x3_stone()),
            count: 1,
        }));
    }
    // Wind, Copper & Electricity wave (2026-09-07) — the little-hut plan the
    // "Follow the Plan" build-along Trial provisions. Plans don't stack, so
    // count is ignored here too.
    if matches!(key.as_str(), "hut_plan" | "house_plan" | "small_house") {
        return Ok(Some(crate::item::ItemStack {
            item: crate::item::Item::Plan(crate::plan::PlanData::small_hut()),
            count: 1,
        }));
    }
    // Last resort before the typo rule: any registered block or material by its display name
    // or registry name (`Spruce Planks`, `genesis:spruce_planks`).
    match super::give_names::registry_item_by_name(&key) {
        Some(crate::item::Item::Block(id)) => return Ok(Some(ItemStack::new_block(id, count.max(1)))),
        Some(crate::item::Item::Material(mat)) => return Ok(Some(ItemStack::new_material(mat, count.max(1)))),
        _ => {}
    }
    Ok(None)
}

/// A player-typed name as a table key: lower case, words joined by `_` whether they were
/// typed with hyphens, underscores or spaces.
fn normalise(name: &str) -> String {
    name.trim().to_lowercase().replace(['-', ' '], "_")
}

/// The error for a name that is nothing `/give` knows, with the nearest real name if there is one.
fn unknown_item(name: &str, key: &str) -> String {
    match closest_name(key) {
        Some(near) => format!("unknown item: '{name}'. Did you mean '{near}'?"),
        None => format!(
            "unknown item: '{name}' (try /give stone, /give wooden_pickaxe, /give bow, /give arrow, /give coal, /give satori armour, etc.)"
        ),
    }
}

/// What `/give <name> [count]` hands over: one stack for most items, `count` pieces for armour
/// (it never stacks), and every piece of a tier for a set name (`satori armour`, `iron armor`),
/// `count` sets of it.
pub(crate) fn resolve_items(name: &str, count: u8) -> Result<Vec<ItemStack>, String> {
    let key = normalise(name);
    let copies = usize::from(count.max(1));
    match armour_by_name(&key) {
        Some(ArmourWant::Set(material)) => {
            let pieces = armour_set(material);
            Ok((0..copies)
                .flat_map(|_| pieces.iter().map(|&(slot, mat)| ItemStack::new_armour(slot, mat)))
                .collect())
        }
        Some(ArmourWant::Piece(slot, material)) => {
            Ok((0..copies).map(|_| ItemStack::new_armour(slot, material)).collect())
        }
        None => resolve_item(name, count).map(|stack| vec![stack]),
    }
}

/// A count word: a whole number with an optional leading `+`, as `parse::<u32>()` takes it.
fn is_count_word(word: &str) -> bool {
    let digits = word.strip_prefix('+').unwrap_or(word);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// The item words and the optional count of `/give` arguments. The complete words are tried as
/// a name first, so a name that ends in a number (`Wheat Stage 2`) is that item; only when they
/// name nothing is a trailing whole number (`5` or `+5`) after at least one item word the count
/// (`oak planks 64`, `satori helmet 2`). Every other word belongs to the item name, so a name may
/// be several words.
fn split_args(args: &[String]) -> (String, Option<&str>) {
    let whole = args.join("_");
    match args.split_last() {
        Some((last, words)) if !words.is_empty() && is_count_word(last) && !names_something(&whole) => {
            (words.join("_"), Some(last.as_str()))
        }
        _ => (whole, None),
    }
}

/// Whether `name` resolves to an item (or an armour set) as typed.
fn names_something(name: &str) -> bool {
    armour_by_name(&normalise(name)).is_some() || matches!(resolve_known(name, 1), Ok(Some(_)))
}

impl Command for GiveCommand {
    fn name(&self) -> &'static str {
        "give"
    }
    fn help(&self) -> &'static str {
        "Add an item to your inventory"
    }
    fn usage(&self) -> &'static str {
        "/give <item> [count]   e.g. /give stone 64, /give wooden_pickaxe"
    }
    fn complete(&self, args_before: &[&str], partial: &str, _ctx: &CompletionContext) -> Vec<String> {
        // Only the item name has a closed set; the count is free text. A name may be several
        // words (`satori helmet`), so after the words already typed offer the next word of
        // every name that starts with them.
        let sets = armour_set_names();
        let names = item_names().iter().chain(sets.iter());
        if args_before.is_empty() {
            return matching(names, partial);
        }
        if args_before.iter().any(|w| is_count_word(w)) {
            return Vec::new();
        }
        let prefix = format!("{}_", args_before.join("_").to_lowercase().replace('-', "_"));
        let next_words = names.filter_map(|n| n.strip_prefix(prefix.as_str())).map(|rest| rest.split('_').next().unwrap_or(rest));
        matching(next_words, partial)
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        if args.is_empty() {
            let msg = "usage: /give <item> [count]".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        }
        let (item_name, count_word) = split_args(args);
        let (count, capped_from): (u8, Option<u32>) = match count_word {
            Some(s) => match s.parse::<u32>() {
                Ok(n) if n >= 1 => {
                    if n > 64 {
                        (64, Some(n))
                    } else {
                        (n as u8, None)
                    }
                }
                Ok(_) => {
                    let msg = "count must be ≥ 1".to_string();
                    ctx.error(msg.clone());
                    return CommandResult::Error(msg);
                }
                Err(_) => {
                    let msg = format!("can't parse '{s}' as a count");
                    ctx.error(msg.clone());
                    return CommandResult::Error(msg);
                }
            },
            None => (1, None),
        };
        let stacks = match resolve_items(&item_name, count) {
            Ok(s) => s,
            Err(e) => {
                // `/give stone many`: the item is fine and the last word was meant as a count.
                // Only when the whole name is nothing like a real one: `satori helmt` is a typo
                // for a name (`satori` alone is the gem), so it gets the nearest-match message.
                let msg = match args.split_last() {
                    Some((last, words))
                        if !words.is_empty()
                            && count_word.is_none()
                            && !is_near_typo(&normalise(&item_name))
                            && resolve_item(&words.join("_"), 1).is_ok() =>
                    {
                        format!("can't parse '{last}' as a count")
                    }
                    _ => e,
                };
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let Some(slot) = ctx.players.get_mut(ctx.player_idx) else {
            let msg = "no player slot".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        let label = given_label(&stacks, &item_name);
        // Count the units the bag accepted, a partial stack included: `add_item` hands back
        // exactly what didn't fit.
        let total: u32 = stacks.iter().map(|s| u32::from(s.count)).sum();
        let mut placed: u32 = 0;
        let mut full = false;
        for stack in stacks {
            let count = u32::from(stack.count);
            if let Some(rest) = slot.inventory.add_item(stack) {
                placed += count - u32::from(rest.count).min(count);
                full = true;
                break;
            }
            placed += count;
        }
        if placed > 0 {
            // /give breaks pure-survival, consistent with /clear and /gamemode
            // (engine audit 2026-06-04, F: /give marked cheats_used but not the
            // pure-survival ledger, so `/give diamond_block 64` still read as a
            // pure survival run). Owner decision P0.3.
            *ctx.cheats_used_marker = true;
            *ctx.pure_survival_broken_marker = true;
        }
        if !full {
            let msg = match capped_from {
                Some(n) => format!("Gave {} (capped from {n})", label.with_count(count)),
                None => format!("Gave {}", label.with_count(count)),
            };
            ctx.success(msg);
            CommandResult::Success
        } else {
            let msg = if placed == 0 {
                "inventory full".to_string()
            } else if label.counted_in_name {
                format!("inventory full: gave {placed} of {total} pieces of {}", label.name)
            } else {
                format!("inventory full: gave {placed} of {total} × {}", label.name)
            };
            ctx.error(msg.clone());
            CommandResult::Error(msg)
        }
    }
}

/// How a hand-over reads back to the player: armour by its real name, never by what was typed.
struct GivenLabel {
    name: String,
    /// Armour is counted in pieces/sets already named in `name`; other items as `N × name`.
    counted_in_name: bool,
}

impl GivenLabel {
    fn with_count(&self, count: u8) -> String {
        if self.counted_in_name {
            self.name.clone()
        } else {
            format!("{count} × {}", self.name)
        }
    }
}

fn given_label(stacks: &[ItemStack], typed: &str) -> GivenLabel {
    let armour_name = |s: &ItemStack| match &s.item {
        crate::item::Item::Armour(a) => Some(crate::armour::armour_label(a.slot, a.material)),
        _ => None,
    };
    let first = stacks.first().and_then(armour_name);
    match first {
        Some(label) if stacks.len() == 1 => GivenLabel { name: label.to_string(), counted_in_name: true },
        Some(label) if stacks.iter().all(|s| armour_name(s) == Some(label)) => {
            GivenLabel { name: format!("{} × {label}", stacks.len()), counted_in_name: true }
        }
        Some(label) => {
            let tier = label.split(' ').next().unwrap_or(label);
            GivenLabel { name: format!("{} pieces of {tier} armour", stacks.len()), counted_in_name: true }
        }
        None => GivenLabel { name: typed.to_string(), counted_in_name: false },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::crafting::{ToolMaterial, ToolType};
    use crate::world::World;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;

    fn run(args: &[&str]) -> (CommandResult, Vec<PlayerSlot>, Vec<String>) {
        let cmd = GiveCommand;
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
        let lines: Vec<String> = log.iter().map(|l| l.text.clone()).collect();
        (r, players, lines)
    }

    /// Like `run`, but surfaces the (cheats_used, pure_survival_broken) markers.
    fn run_markers(args: &[&str]) -> (CommandResult, bool, bool) {
        let cmd = GiveCommand;
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

    /// The whole `/give` line through the dispatcher (the command entry point), on `players`;
    /// returns the result, the chat lines and the (cheats_used, pure_survival_broken) markers.
    fn dispatch_line(players: &mut Vec<PlayerSlot>, line: &str) -> (CommandResult, Vec<String>, bool, bool) {
        let mut world = World::new();
        let mut t = 0u32;
        let mut s = 4u32;
        let mut creative = false;
        let mut log = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let mut reg = CommandRegistry::new();
        reg.register(Box::new(GiveCommand));
        let r = {
            let mut ctx = CommandContext {
                world: &mut world,
                world_time: &mut t,
                world_time_step: &mut s,
                is_creative: &mut creative, play_mode: &mut { use crate::play_mode::PlayMode; PlayMode::Survival },
                seed: 42,
                world_name: "test",
                players,
                player_idx: 0,
                op_level: OpLevel::Op,
                current_tick: 0,
                log: &mut log,
                registry: &reg,
                cheats_used_marker: &mut ch,
                ever_creative_marker: &mut ev,
                pure_survival_broken_marker: &mut ps,
            };
            crate::commands::dispatch::dispatch(line, &mut ctx, &reg)
        };
        (r, log.iter().map(|l| l.text.clone()).collect(), ch, ps)
    }

    /// pt1124 review 6 — a bag with room for one more Cobbled Deepslate takes one of two: that
    /// one unit is a real hand-over, so both ledger flags are set and the message says 1 of 2.
    #[test]
    fn a_partial_give_flags_the_ledger_and_reports_what_was_accepted() {
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        for i in 0..36 {
            players[0].inventory.set_slot(i, Some(ItemStack::new_block(block::DIRT, 64)));
        }
        players[0].inventory.set_slot(5, Some(ItemStack::new_block(block::COBBLED_DEEPSLATE, 63)));
        let (r, lines, cheats_used, pure_broken) = dispatch_line(&mut players, "/give cobbled deepslate 2");
        assert!(matches!(r, CommandResult::Error(_)), "not everything fitted");
        assert_eq!(players[0].inventory.slot(5).unwrap().count, 64, "one unit accepted");
        assert!(cheats_used && pure_broken, "an accepted unit marks the ledger");
        assert!(lines.iter().any(|l| l.contains("gave 1 of 2")), "{lines:?}");
        // Nothing fits at all: no flags.
        let (r, lines, cheats_used, pure_broken) = dispatch_line(&mut players, "/give cobbled deepslate 2");
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!cheats_used && !pure_broken, "nothing given, ledger clean");
        assert!(lines.iter().any(|l| l == "inventory full"), "{lines:?}");
    }

    /// pt1124 review 15 — a display name ending in a number is that item: the complete name is
    /// tried first, and only a name that resolves to nothing reads its last number as a count.
    #[test]
    fn a_name_ending_in_a_number_is_tried_whole_before_the_count() {
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        let (r, lines, ..) = dispatch_line(&mut players, "/give Wheat Stage 2");
        assert_eq!(r, CommandResult::Success, "{lines:?}");
        let got = players[0].inventory.slot(0).unwrap();
        assert_eq!((got.item.clone(), got.count), (crate::item::Item::Block(block::WHEAT_STAGE_2), 1));
        let (r, ..) = dispatch_line(&mut players, "/give stone 5");
        assert_eq!(r, CommandResult::Success);
        let stone = players[0].inventory.slots_iter().flatten().find(|s| s.item == crate::item::Item::Block(block::STONE)).unwrap();
        assert_eq!(stone.count, 5, "a trailing number after a name that resolves alone is still the count");
        let (r, ..) = dispatch_line(&mut players, "/give oak planks 3");
        assert_eq!(r, CommandResult::Success);
    }

    /// pt1124 review 16 — the economy blocks are given as their item form whether the name is
    /// typed bare or with the registry namespace, so the two stack together.
    #[test]
    fn a_qualified_economy_block_name_gives_the_same_item_form() {
        for local in ["bounty_board", "tip_jar", "repair_bench", "plot_marker", "market_bell", "auction_block", "bazaar_block"] {
            let bare = resolve_item(local, 1).unwrap().item;
            let qualified = resolve_item(&format!("genesis:{local}"), 1).unwrap().item;
            assert!(matches!(bare, crate::item::Item::Material(_)), "{local} gives the item form, got {bare:?}");
            assert_eq!(qualified, bare, "genesis:{local}");
        }
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        dispatch_line(&mut players, "/give tip_jar");
        dispatch_line(&mut players, "/give genesis:tip_jar");
        assert_eq!(players[0].inventory.slot(0).unwrap().count, 2, "one stack");
        assert!(players[0].inventory.slot(1).is_none());
    }

    #[test]
    fn gives_stone() {
        let (r, players, _) = run(&["stone", "5"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        assert_eq!(slot.count, 5);
    }

    #[test]
    fn give_breaks_pure_survival_like_clear() {
        // Engine audit F / owner decision P0.3 — /give must flag the integrity
        // ledger (cheats_used + pure_survival_broken), not just cheats_used.
        let (r, cheats_used, pure_survival_broken) = run_markers(&["diamond_block", "64"]);
        assert_eq!(r, CommandResult::Success);
        assert!(cheats_used, "/give marks cheats_used");
        assert!(pure_survival_broken, "/give breaks pure-survival, like /clear");
    }

    #[test]
    fn give_failure_does_not_flag_the_ledger() {
        // An invalid item must not mark the ledger (no items actually granted).
        let (r, cheats_used, pure_survival_broken) = run_markers(&["banana"]);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(!cheats_used && !pure_survival_broken, "failed /give leaves the ledger clean");
    }

    #[test]
    fn defaults_count_to_one() {
        let (r, players, _) = run(&["dirt"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].inventory.slot(0).unwrap().count, 1);
    }

    #[test]
    fn caps_count_at_stack_size() {
        let (r, players, lines) = run(&["stone", "999"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].inventory.slot(0).unwrap().count, 64);
        // The success message must tell the user we capped — silent capping
        // would leave them confused about why /give stone 999 only gave 64.
        assert!(
            lines.iter().any(|l| l.contains("capped from 999")),
            "expected capped message, got: {lines:?}"
        );
    }

    #[test]
    fn rejects_unknown_item() {
        let (r, _, _) = run(&["banana"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn gives_tool() {
        let (r, players, _) = run(&["wooden_pickaxe"]);
        assert_eq!(r, CommandResult::Success);
        // Tools land somewhere in inventory; verify at least one slot has a tool.
        let has_tool = (0..36).any(|i| {
            players[0].inventory.slot(i)
                .map(|s| matches!(s.item, crate::item::Item::Tool(_)))
                .unwrap_or(false)
        });
        assert!(has_tool, "tool not added to inventory");
    }

    #[test]
    fn a_plus_sign_count_gives_that_many() {
        let (r, players, _) = run(&["stone", "+5"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].inventory.slot(0).unwrap().count, 5);
        let (r, players, _) = run(&["oak", "planks", "+3"]);
        assert_eq!(r, CommandResult::Success);
        assert_eq!(players[0].inventory.slot(0).unwrap().count, 3);
    }

    #[test]
    fn a_bare_plus_or_minus_is_not_a_count() {
        assert!(is_count_word("+5") && is_count_word("5"));
        assert!(!is_count_word("+") && !is_count_word("") && !is_count_word("-5") && !is_count_word("5x"));
    }

    #[test]
    fn rejects_zero_count() {
        let (r, _, _) = run(&["stone", "0"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn rejects_garbage_count() {
        let (r, _, _) = run(&["stone", "many"]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    #[test]
    fn handles_aliases() {
        let (r, players, _) = run(&["log", "1"]);
        assert_eq!(r, CommandResult::Success);
        // Should give oak_log
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::OAK_LOG),
            _ => panic!("expected block"),
        }
    }

    #[test]
    fn no_args_errors() {
        let (r, _, _) = run(&[]);
        assert!(matches!(r, CommandResult::Error(_)));
    }

    // --- Farming Tier 1 (Wave 26) /give parsing ---

    #[test]
    fn give_tilled_soil_works() {
        let (r, players, _) = run(&["tilled_soil", "4"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::TILLED_SOIL),
            _ => panic!("expected tilled-soil block"),
        }
        assert_eq!(slot.count, 4);
    }

    #[test]
    fn give_seeds_works() {
        let (r, players, _) = run(&["wheat_seeds", "16"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::WheatSeeds));
            }
            _ => panic!("expected wheat seeds"),
        }
        assert_eq!(slot.count, 16);
    }

    #[test]
    fn give_raw_fish_and_berries_work() {
        // Task 19 — "A Friend in Need" trial's kit needs these to resolve
        // (cat's tame food is raw fish, fox's is berries).
        let (r, players, _) = run(&["raw_fish", "6"]);
        assert_eq!(r, CommandResult::Success);
        match &players[0].inventory.slot(0).unwrap().item {
            crate::item::Item::Material(m) => assert!(matches!(m, crate::item::MaterialId::RawFish)),
            _ => panic!("expected raw fish"),
        }
        let (r, players, _) = run(&["berries", "8"]);
        assert_eq!(r, CommandResult::Success);
        match &players[0].inventory.slot(0).unwrap().item {
            crate::item::Item::Material(m) => assert!(matches!(m, crate::item::MaterialId::Berries)),
            _ => panic!("expected berries"),
        }
    }

    #[test]
    fn give_bread_works() {
        let (r, players, _) = run(&["bread", "3"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::Bread));
            }
            _ => panic!("expected bread"),
        }
    }

    #[test]
    fn give_satori_hoe_works() {
        let (r, players, _) = run(&["satori_hoe"]);
        assert_eq!(r, CommandResult::Success);
        let has_hoe = (0..36).any(|i| {
            players[0].inventory.slot(i).map(|s| match &s.item {
                crate::item::Item::Tool(t) => {
                    t.tool_type == crate::crafting::ToolType::Hoe
                        && t.material == crate::crafting::ToolMaterial::Satori
                }
                _ => false,
            }).unwrap_or(false)
        });
        assert!(has_hoe, "Satori Hoe not in inventory");
    }

    #[test]
    fn give_all_five_hoe_tiers_works() {
        for tier_name in ["wooden_hoe", "stone_hoe", "iron_hoe", "diamond_hoe", "satori_hoe"] {
            let (r, _, _) = run(&[tier_name]);
            assert_eq!(r, CommandResult::Success, "failed to give {tier_name}");
        }
    }

    #[test]
    fn give_mature_crop_block_works() {
        let (r, players, _) = run(&["wheat_stage_3", "1"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::WHEAT_STAGE_3),
            _ => panic!("expected wheat_stage_3 block"),
        }
    }

    // --- Wave 27 campfire / flint / flint-and-steel ---

    #[test]
    fn give_campfire_works_and_is_unlit_by_default() {
        // `/give campfire` must produce the UNLIT variant (player must
        // fuel + ignite). `/give campfire_lit` is the test-only variant.
        let (r, players, _) = run(&["campfire", "2"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::CAMPFIRE_UNLIT),
            _ => panic!("expected unlit campfire"),
        }
        assert_eq!(slot.count, 2);
    }

    #[test]
    fn give_campfire_lit_alias_returns_lit_block() {
        let (r, players, _) = run(&["campfire_lit"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::CAMPFIRE),
            _ => panic!("expected lit campfire"),
        }
    }

    #[test]
    fn give_flint_works() {
        let (r, players, _) = run(&["flint", "5"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Material(m) => {
                assert!(matches!(m, crate::item::MaterialId::Flint));
            }
            _ => panic!("expected flint material"),
        }
        assert_eq!(slot.count, 5);
    }

    #[test]
    fn give_flint_and_steel_works() {
        // Multi-word tool name; tested via the bow_by_name parser path.
        let (r, players, _) = run(&["flint_and_steel"]);
        assert_eq!(r, CommandResult::Success);
        let has_fas = (0..36).any(|i| {
            players[0].inventory.slot(i).map(|s| match &s.item {
                crate::item::Item::Tool(t) => {
                    t.tool_type == crate::crafting::ToolType::FlintAndSteel
                        && t.durability == crate::crafting::FLINT_AND_STEEL_DURABILITY
                }
                _ => false,
            }).unwrap_or(false)
        });
        assert!(has_fas, "Flint and Steel not in inventory at full durability");
    }

    #[test]
    fn give_leaves_works() {
        let (r, players, _) = run(&["leaves", "10"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        match &slot.item {
            crate::item::Item::Block(id) => assert_eq!(*id, block::OAK_LEAVES),
            _ => panic!("expected oak leaves block"),
        }
        assert_eq!(slot.count, 10);
    }

    // --- Spec 23 — Papyrus Reed / Sheet aliases ---

    #[test]
    fn give_papyrus_reed_works() {
        for alias in &["papyrus_reed", "papyrus", "reed"] {
            let (r, players, _) = run(&[alias, "3"]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Material(crate::item::MaterialId::PapyrusReed) => {}
                _ => panic!("alias '{alias}' did not resolve to PapyrusReed"),
            }
            assert_eq!(slot.count, 3);
        }
    }

    #[test]
    fn give_papyrus_sheet_works() {
        // Canonical + every alias including `paper` (since PapyrusSheet
        // is the first paper material in the engine).
        for alias in &["papyrus_sheet", "papyrussheet", "papyrus_paper", "paper"] {
            let (r, players, _) = run(&[alias, "5"]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Material(crate::item::MaterialId::PapyrusSheet) => {}
                _ => panic!("alias '{alias}' did not resolve to PapyrusSheet"),
            }
            assert_eq!(slot.count, 5);
        }
    }

    #[test]
    fn give_papyrus_stage_blocks_work() {
        for (alias, expected) in [
            ("papyrus_stage_0", block::PAPYRUS_STAGE_0),
            ("papyrus_sprout", block::PAPYRUS_STAGE_0),
            ("papyrus_stage_1", block::PAPYRUS_STAGE_1),
            ("papyrus_stage_2", block::PAPYRUS_STAGE_2),
            ("papyrus_stage_3", block::PAPYRUS_STAGE_3),
            ("papyrus_mature", block::PAPYRUS_STAGE_3),
        ] {
            let (r, players, _) = run(&[alias]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Block(id) => assert_eq!(*id, expected, "alias '{alias}'"),
                _ => panic!("alias '{alias}' did not resolve to a block"),
            }
        }
    }

    #[test]
    fn gives_coal_material() {
        let (r, players, _) = run(&["coal", "10"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        assert_eq!(slot.count, 10);
        match &slot.item {
            crate::item::Item::Material(crate::item::MaterialId::Coal) => {}
            _ => panic!("expected Material(Coal)"),
        }
    }

    #[test]
    fn gives_diamond_material_default_count() {
        let (r, players, _) = run(&["diamond"]);
        assert_eq!(r, CommandResult::Success);
        let slot = players[0].inventory.slot(0).unwrap();
        assert_eq!(slot.count, 1);
        match &slot.item {
            crate::item::Item::Material(crate::item::MaterialId::Diamond) => {}
            _ => panic!("expected Material(Diamond)"),
        }
    }

    #[test]
    fn historical_naming_pass_give_aliases() {
        // Old aliases still work.
        assert_eq!(block_by_name("blueprint_paper"), Some(block::BLUEPRINT_PAPER));
        assert_eq!(block_by_name("vendor_block"), Some(block::VENDOR_BLOCK));
        assert_eq!(block_by_name("hay_bale"), Some(block::HAY_BALE));
        // New historical aliases also work.
        assert_eq!(block_by_name("plan_scroll"), Some(block::BLUEPRINT_PAPER));
        assert_eq!(block_by_name("market_stall"), Some(block::VENDOR_BLOCK));
        assert_eq!(block_by_name("hay_rick"), Some(block::HAY_BALE));
        assert_eq!(block_by_name("bone_cairn"), Some(block::BONE_BLOCK));
        assert_eq!(block_by_name("amethyst_cluster"), Some(block::AMETHYST_BLOCK));
        assert_eq!(block_by_name("sugar_cane"), Some(block::SUGARCANE));
        assert_eq!(block_by_name("foundation_stone"), Some(block::CONSTRUCTION_ANCHOR));
        assert_eq!(block_by_name("masons_mark"), Some(block::ARCHITECT_PLAQUE));
        assert_eq!(block_by_name("drafting_bench"), Some(block::DRAFTING_TABLE));
        // Material aliases (old + new).
        assert_eq!(material_by_name("bonemeal"), Some(crate::item::MaterialId::Bonemeal));
        assert_eq!(material_by_name("bone_meal"), Some(crate::item::MaterialId::Bonemeal));
        assert_eq!(material_by_name("honey_bottle"), Some(crate::item::MaterialId::HoneyBottle));
        assert_eq!(material_by_name("honey_jar"), Some(crate::item::MaterialId::HoneyBottle));
    }

    #[test]
    fn trials_kit_items_added_to_give_resolve() {
        // Items added so Trials challenge kits (Smith / Engineer / Angler /
        // Farmhand) can hand them out. A regression here ships a broken kit.
        assert_eq!(block_by_name("furnace"), Some(block::FURNACE));
        assert_eq!(block_by_name("furnace_lit"), Some(block::FURNACE_LIT));
        assert_eq!(block_by_name("piston"), Some(block::PISTON));
        assert_eq!(block_by_name("sticky_piston"), Some(block::STICKY_PISTON));
        assert_eq!(bow_by_name("fishing_rod"), Some((ToolType::FishingRod, ToolMaterial::Wood)));
        assert_eq!(bow_by_name("shears"), Some((ToolType::Shears, ToolMaterial::Iron)));
        // End-to-end through the public resolver too.
        assert!(resolve_item("furnace", 1).is_ok());
        assert!(resolve_item("fishing_rod", 1).is_ok());
        assert!(resolve_item("shears", 1).is_ok());
    }

    #[test]
    fn material_aliases_work() {
        // raw_iron / iron / rawiron should all resolve to RawIron.
        for alias in &["iron", "raw_iron", "rawiron"] {
            let (r, players, _) = run(&[alias, "1"]);
            assert_eq!(r, CommandResult::Success, "alias '{alias}' failed");
            let slot = players[0].inventory.slot(0).unwrap();
            match &slot.item {
                crate::item::Item::Material(crate::item::MaterialId::RawIron) => {}
                _ => panic!("alias '{alias}' did not resolve to RawIron"),
            }
        }
    }

    #[test]
    fn copper_aliases_work() {
        // Wind, Copper & Electricity wave (2026-09-07) — `copper`/`copper_ore`
        // give the ore BLOCK (block_by_name wins over material_by_name),
        // `raw_copper`/`rawcopper` give the smeltable material.
        for alias in &["copper", "copper_ore"] {
            assert_eq!(block_by_name(alias), Some(block::COPPER_ORE), "alias '{alias}'");
        }
        for alias in &["raw_copper", "rawcopper"] {
            assert_eq!(
                material_by_name(alias),
                Some(crate::item::MaterialId::Copper),
                "alias '{alias}'"
            );
        }
        assert_eq!(
            material_by_name("copper_ingot"),
            Some(crate::item::MaterialId::CopperIngot)
        );
        assert_eq!(
            material_by_name("copperingot"),
            Some(crate::item::MaterialId::CopperIngot)
        );
        // End-to-end through the public resolver too.
        for alias in &["copper", "copper_ore", "raw_copper", "copper_ingot"] {
            assert!(resolve_item(alias, 1).is_ok(), "resolve_item('{alias}') failed");
        }
    }

    #[test]
    fn cart_item_aliases_resolve() {
        // Craftable Armoured Carts (CA2) — `/give iron_cart` etc. so testers
        // can spawn cart items before the survival recipes are unlocked.
        use crate::item::MaterialId as M;
        assert_eq!(material_by_name("wood_cart"), Some(M::WoodCart));
        assert_eq!(material_by_name("woodcart"), Some(M::WoodCart));
        assert_eq!(material_by_name("cart"), Some(M::WoodCart));
        assert_eq!(material_by_name("iron_cart"), Some(M::IronCart));
        assert_eq!(material_by_name("ironcart"), Some(M::IronCart));
        assert_eq!(material_by_name("diamond_cart"), Some(M::DiamondCart));
        assert_eq!(material_by_name("diamondcart"), Some(M::DiamondCart));
    }

    #[test]
    fn magnesium_family_aliases_resolve() {
        // Spec 37 — Magnesium mineral + its products had zero /give aliases,
        // making them impossible to test-spawn (docs/superpowers/specs/2026-07-09).
        use crate::item::MaterialId as M;
        assert_eq!(material_by_name("magnesium"), Some(M::Magnesium));
        assert_eq!(material_by_name("fertiliser"), Some(M::Fertiliser));
        assert_eq!(material_by_name("fertilizer"), Some(M::Fertiliser));
        assert_eq!(material_by_name("sparkler"), Some(M::Sparkler));
        assert_eq!(material_by_name("flare"), Some(M::Flare));
        assert_eq!(material_by_name("magnesium_firestarter"), Some(M::MagnesiumFirestarter));
        assert_eq!(material_by_name("firestarter"), Some(M::MagnesiumFirestarter));
        assert_eq!(material_by_name("fire_starter"), Some(M::MagnesiumFirestarter));
    }
}
