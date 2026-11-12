use crate::commands::complete::{matching, CompletionContext};
use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::Command;
use crate::crafting::Tool;
use crate::item::ItemStack;

// The name tables live in `give_names`: the resolver below and Tab completion
// both read them, so a name cannot be accepted but not suggested.
pub use super::give_names::{block_names, item_names};
use super::give_names::{block_by_name, bow_by_name, material_by_name, tool_by_name};

pub struct GiveCommand;

/// Resolve a player-typed item name to a constructed `ItemStack`.
/// Returns `Err(message)` for unknown items.
///
/// `pub(crate)` so the scenario runner (`scenario.rs` + the `/scenario`
/// handler) can provision a `ScenarioDef.kit` through the same vocabulary the
/// `/give` command uses — one resolver, no duplicate item tables.
pub(crate) fn resolve_item(name: &str, count: u8) -> Result<ItemStack, String> {
    let key = name.to_lowercase().replace('-', "_");
    if let Some(block_id) = block_by_name(&key) {
        return Ok(ItemStack::new_block(block_id, count.max(1)));
    }
    if let Some((tt, tm)) = tool_by_name(&key) {
        // Tools don't stack; ignore count.
        return Ok(ItemStack::new_tool(Tool::new(tt, tm)));
    }
    if let Some((tt, tm)) = bow_by_name(&key) {
        return Ok(ItemStack::new_tool(Tool::new(tt, tm)));
    }
    if let Some(mat) = material_by_name(&key) {
        return Ok(ItemStack::new_material(mat, count.max(1)));
    }
    // Spec 24 — debug Plan item. 3×3 stone footprint, default name +
    // CC-BY-SA licence. Useful for testing Inspect / place / animated
    // build paths once those phases ship. Plans don't stack, so count
    // is ignored.
    if matches!(key.as_str(), "debug_plan" | "plan" | "test_plan") {
        return Ok(crate::item::ItemStack {
            item: crate::item::Item::Plan(crate::plan::PlanData::debug_3x3_stone()),
            count: 1,
        });
    }
    // Wind, Copper & Electricity wave (2026-09-07) — the little-hut plan the
    // "Follow the Plan" build-along Trial provisions. Plans don't stack, so
    // count is ignored here too.
    if matches!(key.as_str(), "hut_plan" | "house_plan" | "small_house") {
        return Ok(crate::item::ItemStack {
            item: crate::item::Item::Plan(crate::plan::PlanData::small_hut()),
            count: 1,
        });
    }
    Err(format!(
        "unknown item: '{name}' (try /give stone, /give wooden_pickaxe, /give bow, /give arrow, /give coal, etc.)"
    ))
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
        // Only the item name has a closed set; the count is free text.
        match args_before {
            [] => matching(item_names(), partial),
            _ => Vec::new(),
        }
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let item_name = match args.first() {
            Some(s) => s,
            None => {
                let msg = "usage: /give <item> [count]".to_string();
                ctx.error(msg.clone());
                return CommandResult::Error(msg);
            }
        };
        let (count, capped_from): (u8, Option<u32>) = match args.get(1) {
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
        let stack = match resolve_item(item_name, count) {
            Ok(s) => s,
            Err(e) => {
                ctx.error(e.clone());
                return CommandResult::Error(e);
            }
        };
        let Some(slot) = ctx.players.get_mut(ctx.player_idx) else {
            let msg = "no player slot".to_string();
            ctx.error(msg.clone());
            return CommandResult::Error(msg);
        };
        if slot.inventory.add_item(stack).is_none() {
            // /give breaks pure-survival, consistent with /clear and /gamemode
            // (engine audit 2026-06-04, F: /give marked cheats_used but not the
            // pure-survival ledger, so `/give diamond_block 64` still read as a
            // pure survival run). Owner decision P0.3.
            *ctx.cheats_used_marker = true;
            *ctx.pure_survival_broken_marker = true;
            let msg = match capped_from {
                Some(n) => format!("Gave {count} × {item_name} (capped from {n})"),
                None => format!("Gave {count} × {item_name}"),
            };
            ctx.success(msg);
            CommandResult::Success
        } else {
            let msg = "inventory full".to_string();
            ctx.error(msg.clone());
            CommandResult::Error(msg)
        }
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
