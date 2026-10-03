//! `/recipes` — print a curated index of known crafting recipes.
//!
//! Read-only, never marks cheats. Useful for confirming a recipe
//! exists when the explorer doesn't list it (the explorer enumerates
//! ITEMS, not recipes). The list is hand-maintained: a fully dynamic
//! recipe iterator would require enumerating the recipe combinator
//! space which doesn't exist as data — recipes are pattern-matched
//! procedurally in `crafting.rs::match_recipe`.
//!
//! Optional argument filters by substring on the recipe name:
//! `/recipes armour` shows just the armour recipes.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};

pub struct RecipesCommand;

/// One row in the recipe index.
struct RecipeEntry {
    name: &'static str,
    pattern: &'static str,
}

const RECIPES: &[RecipeEntry] = &[
    RecipeEntry { name: "Planks (Oak default)", pattern: "1 any log → 4 planks" },
    RecipeEntry { name: "Planks (per-species)", pattern: "1 species log block → 4 species planks" },
    RecipeEntry { name: "Sticks", pattern: "2 planks vertical → 4 sticks" },
    RecipeEntry { name: "Bone Meal", pattern: "1 bone → 3 bone meal" },
    RecipeEntry { name: "Torch", pattern: "Coal on top of Stick → 4 torches" },
    RecipeEntry { name: "Arrow", pattern: "Stick on top of Feather → 4 arrows" },
    RecipeEntry { name: "Blueprint Paper", pattern: "Papyrus Sheet / Iron Ingot / Salt (column) → 3 Blueprint Papers" },
    RecipeEntry { name: "Fence Post", pattern: "PSP / PSP (2×3: plank-stick-plank, twice — any wood species; output matches plank) → 3 Fence Posts" },
    RecipeEntry { name: "Bunting", pattern: "Dye + String + Dye (1×3 horizontal, same dye both sides) → 4 Bunting of that dye's colour" },
    RecipeEntry { name: "Paper Lantern", pattern: "Papyrus Sheet + Stick + Dye (1×3 horizontal, left to right) → 1 Paper Lantern of that dye's colour (emits light)" },
    RecipeEntry { name: "Kite", pattern: "Cloth + String + Dye (1×3 horizontal, left to right) → 1 Kite of that dye's colour" },
    RecipeEntry { name: "Banner", pattern: "Dye / Cloth / Stick (3-tall vertical column) → 1 Banner of that dye's colour" },
    RecipeEntry { name: "Sail", pattern: "Dye / Canvas / Stick (3-tall vertical column) → 1 Sail of that dye's colour" },
    RecipeEntry { name: "Tent", pattern: "CCC / S.S (2×3: Canvas roof + Stick corner stays, empty centre-bottom) → 1 Tent" },
    RecipeEntry { name: "Flint and Steel", pattern: "Flint + Iron Ingot → 1 Flint and Steel" },
    RecipeEntry { name: "Bronze Ingot", pattern: "Copper Ingot + Tin Ingot → 1 Bronze Ingot" },
    RecipeEntry { name: "Shears", pattern: "2 Iron Ingots vertical → 1 Shears" },
    RecipeEntry { name: "Pickaxe", pattern: "3 material top + 2 sticks column → 1 Pickaxe (tier from material)" },
    RecipeEntry { name: "Axe", pattern: "MM/MS/_S (or mirrored) → 1 Axe" },
    RecipeEntry { name: "Sword", pattern: "M/M/S → 1 Sword" },
    RecipeEntry { name: "Shovel", pattern: "M/S/S → 1 Shovel" },
    RecipeEntry { name: "Hoe", pattern: "MM/_S/_S (or mirrored) → 1 Hoe" },
    RecipeEntry { name: "Bow", pattern: "Sticks + String diagonal → 1 Bow" },
    RecipeEntry { name: "Fishing Rod", pattern: "Sticks diagonal + 2 String right column → 1 Fishing Rod" },
    RecipeEntry { name: "Bread", pattern: "3 wheat horizontal → 1 bread" },
    RecipeEntry { name: "Papyrus Sheet", pattern: "3 papyrus reed horizontal → 3 sheets" },
    RecipeEntry { name: "Bed", pattern: "3 wool top + 3 planks bottom → 1 bed" },
    RecipeEntry { name: "Workbench", pattern: "4 planks (2x2) → 1 workbench" },
    RecipeEntry { name: "Drying Rack", pattern: "4 sticks (2x2) → 1 drying rack" },
    RecipeEntry { name: "Glass (alpha)", pattern: "4 sand (2x2) → 4 glass" },
    RecipeEntry { name: "Drafting Bench", pattern: "Paper top-left + 3 oak planks (2x2) → 1 drafting bench" },
    RecipeEntry { name: "Amethyst Cluster", pattern: "4 amethyst (2x2) → 1 amethyst cluster (round-trip)" },
    RecipeEntry { name: "Helmet (armour)", pattern: "MMM/M.M → 1 helmet (Leather/Iron/Diamond/Satori)" },
    RecipeEntry { name: "Chestplate (armour)", pattern: "M.M/MMM/MMM → 1 chestplate" },
    RecipeEntry { name: "Leggings (armour)", pattern: "MMM/M.M/M.M → 1 leggings" },
    RecipeEntry { name: "Boots (armour)", pattern: "M.M/M.M → 1 boots" },
    RecipeEntry { name: "Coal Block", pattern: "9 coal (3x3) → 1 coal block (round-trip)" },
    RecipeEntry { name: "Iron Block", pattern: "9 raw iron (3x3) → 1 iron block (round-trip)" },
    RecipeEntry { name: "Diamond Block", pattern: "9 diamond (3x3) → 1 diamond block (round-trip)" },
    RecipeEntry { name: "Satori Block", pattern: "9 Satori (3x3) → 1 Satori block (round-trip)" },
    RecipeEntry { name: "Bone Cairn", pattern: "9 bone (3x3) → 1 bone cairn (round-trip)" },
    RecipeEntry { name: "Hay Rick", pattern: "9 wheat (3x3) → 1 hay rick (round-trip)" },
    RecipeEntry { name: "Furnace", pattern: "8 cobblestone ring (3x3, empty centre) → 1 furnace" },
    RecipeEntry { name: "Market Stall", pattern: "8 oak planks ringing 1 iron ingot (3x3) → 1 market stall" },
    RecipeEntry { name: "Village Bell", pattern: "Iron Ingot / Stick / Oak Plank column → 1 Village Bell" },
];

const FURNACE_SMELTS: &[RecipeEntry] = &[
    RecipeEntry { name: "Iron Ingot", pattern: "Raw Iron → Iron Ingot (furnace)" },
    RecipeEntry { name: "Copper Ingot", pattern: "Copper → Copper Ingot (furnace)" },
    RecipeEntry { name: "Tin Ingot", pattern: "Tin → Tin Ingot (furnace)" },
    RecipeEntry { name: "Cooked meats", pattern: "Raw Beef/Pork/Chicken/Mutton → Cooked variant (furnace OR campfire)" },
];

impl Command for RecipesCommand {
    fn name(&self) -> &'static str {
        "recipes"
    }
    fn help(&self) -> &'static str {
        "List known crafting recipes (optional substring filter)"
    }
    fn usage(&self) -> &'static str {
        "/recipes [filter]"
    }
    fn min_op_level(&self) -> OpLevel {
        OpLevel::None
    }
    fn is_cheat(&self) -> bool {
        false
    }
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let filter = args
            .first()
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        let mut shown = 0usize;
        ctx.success("--- Crafting grid recipes ---".to_string());
        for r in RECIPES {
            if filter.is_empty()
                || r.name.to_lowercase().contains(&filter)
                || r.pattern.to_lowercase().contains(&filter)
            {
                ctx.success(format!("  {}: {}", r.name, r.pattern));
                shown += 1;
            }
        }
        ctx.success("--- Furnace smelting ---".to_string());
        for r in FURNACE_SMELTS {
            if filter.is_empty()
                || r.name.to_lowercase().contains(&filter)
                || r.pattern.to_lowercase().contains(&filter)
            {
                ctx.success(format!("  {}: {}", r.name, r.pattern));
                shown += 1;
            }
        }
        if shown == 0 {
            ctx.error(format!("No recipes match '{filter}'."));
        } else {
            ctx.success(format!("({shown} recipe{} shown)", if shown == 1 { "" } else { "s" }));
        }
        CommandResult::Silent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry::CommandRegistry;
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    fn run(args: &[&str]) -> Vec<String> {
        let cmd = RecipesCommand;
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
            op_level: OpLevel::None,
            current_tick: 0,
            log: &mut log,
            registry: &reg,
            cheats_used_marker: &mut ch,
            ever_creative_marker: &mut ev,
            pure_survival_broken_marker: &mut ps,
        };
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        cmd.execute(&mut ctx, &owned);
        log.iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn lists_recipes_with_no_filter() {
        let lines = run(&[]);
        assert!(lines.iter().any(|l| l.contains("Crafting grid")));
        assert!(lines.iter().any(|l| l.contains("Furnace smelting")));
        // Should have at least 30 recipes shown.
        let recipe_count: usize = lines
            .iter()
            .filter(|l| l.starts_with("  "))
            .count();
        assert!(recipe_count >= 30, "expected ≥30 recipes, got {recipe_count}");
    }

    #[test]
    fn filter_narrows_to_matching_recipes() {
        let lines = run(&["armour"]);
        // All "  " lines should mention armour pieces.
        let recipe_lines: Vec<_> = lines.iter().filter(|l| l.starts_with("  ")).collect();
        for l in &recipe_lines {
            assert!(l.to_lowercase().contains("armour")
                    || l.contains("Helmet")
                    || l.contains("Chestplate")
                    || l.contains("Leggings")
                    || l.contains("Boots"));
        }
        assert!(!recipe_lines.is_empty());
    }

    #[test]
    fn unknown_filter_returns_error_line() {
        let lines = run(&["xyzzy_unknown"]);
        assert!(lines.iter().any(|l| l.contains("No recipes match")));
    }

    #[test]
    fn case_insensitive_filter() {
        let upper = run(&["FURNACE"]);
        let lower = run(&["furnace"]);
        let count = |v: &[String]| v.iter().filter(|l| l.starts_with("  ")).count();
        assert_eq!(count(&upper), count(&lower));
    }

    #[test]
    fn is_not_a_cheat() {
        assert!(!RecipesCommand.is_cheat());
        assert_eq!(RecipesCommand.min_op_level(), OpLevel::None);
    }
}
