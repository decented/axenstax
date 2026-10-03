//! Recipe catalogue — a **data** description of craftable recipes, for the
//! recipe book, search, and "what can I make?" hints.
//!
//! Design (foundations 2026-06-12-recipe-catalogue-and-book): this catalogue
//! does NOT decide what crafts. The authority for that stays
//! [`crate::crafting::match_recipe`]. Each [`RecipeCard`] carries an
//! `example_grid` that the live matcher maps to the card's `output`; the
//! recipe book auto-fills that grid and lets the real matcher produce the
//! item, so the book can never advertise a craft the engine won't honour.
//!
//! The safety net is [`tests::every_card_matches_the_live_matcher`]: every
//! card's grid is fed to `match_recipe` and must equal the card's output.
//! A wrong or stale card fails CI rather than mis-crafting in the wild.
//!
//! Parametric families (tools × materials, armour × materials, bows × tiers)
//! are GENERATED from the same material tables the matcher reads, so they
//! cannot drift out of sync with the material set.

use std::sync::LazyLock;

use crate::block;
use crate::crafting::{CraftSlot, Tool, ToolMaterial, ToolType};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};

/// Where a recipe can be crafted: the 2×2 player grid, or a 3×3 workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CraftStation {
    PlayerGrid,
    Workbench,
}

/// Top-level grouping for the recipe-book tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipeCategory {
    Tools,
    Combat,
    Building,
    Decoration,
    Food,
    Materials,
    Stations,
    Transport,
}

impl RecipeCategory {
    pub fn label(self) -> &'static str {
        match self {
            RecipeCategory::Tools => "Tools",
            RecipeCategory::Combat => "Combat",
            RecipeCategory::Building => "Building",
            RecipeCategory::Decoration => "Decoration",
            RecipeCategory::Food => "Food",
            RecipeCategory::Materials => "Materials",
            RecipeCategory::Stations => "Stations",
            RecipeCategory::Transport => "Transport",
        }
    }

    /// Tab order for the book.
    pub const ALL: [RecipeCategory; 8] = [
        RecipeCategory::Tools,
        RecipeCategory::Combat,
        RecipeCategory::Building,
        RecipeCategory::Stations,
        RecipeCategory::Decoration,
        RecipeCategory::Food,
        RecipeCategory::Materials,
        RecipeCategory::Transport,
    ];
}

/// A "fuzzy" ingredient accepts any member of a set (the matcher's
/// `is_logish_slot` / `is_any_planks` / tier helpers). The catalogue shows a
/// representative item but tags it so affordability checks honour the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuzzyKind {
    Log,
    /// Handled by the matcher (`is_any_planks`) and the recipe-book label
    /// lookup, but no bundled recipe currently sets `fuzzy: Some(Plank)` —
    /// ready for whichever "any plank species" recipe needs it next.
    #[allow(dead_code)]
    Plank,
    Paper,
}

/// One line of a recipe's shapeless bill of materials (display + search +
/// affordability). Not used for matching — that's `example_grid`'s job.
#[derive(Debug, Clone)]
pub struct RecipeIngredient {
    pub item: Item,
    pub count: u8,
    pub fuzzy: Option<FuzzyKind>,
}

impl RecipeIngredient {
    fn fixed(item: Item, count: u8) -> Self {
        Self { item, count, fuzzy: None }
    }
}

/// A browsable recipe. `output` + `example_grid` are the load-bearing
/// fields; everything else is for the book UI.
#[derive(Debug, Clone)]
pub struct RecipeCard {
    /// Display name, e.g. "Iron Pickaxe". Generated for families, authored
    /// for literals. Used for search.
    pub name: String,
    /// What the recipe yields (the exact `ItemStack` the matcher returns).
    pub output: ItemStack,
    /// Shapeless ingredient list for the book.
    pub ingredients: Vec<RecipeIngredient>,
    /// A concrete grid the live matcher maps to `output`. Drives both
    /// click-to-craft auto-fill and the consistency test.
    pub example_grid: [[CraftSlot; 3]; 3],
    pub category: RecipeCategory,
    /// Every card carries which station it needs, but `recipe_book_ui` doesn't
    /// display or filter on it yet — captured for that UI's next pass.
    #[allow(dead_code)]
    pub station: CraftStation,
    /// A short, kid-readable line explaining what a non-obvious item DOES
    /// (e.g. "Right-click crops to make them grow"). `None` for
    /// self-evident blocks/tools — most cards. Rendered under the recipe
    /// row in the book when present. UX polish sweep Task 3.
    pub usage: Option<&'static str>,
}

impl RecipeCard {
    /// Builder-style setter used at the handful of call sites that need a
    /// non-obvious-item usage line, so `card()`/`card_decor()` callers don't
    /// all have to pass an extra argument.
    fn with_usage(mut self, usage: &'static str) -> Self {
        self.usage = Some(usage);
        self
    }
}

// ── Grid helpers ───────────────────────────────────────────────────────────

const EMPTY_GRID: [[CraftSlot; 3]; 3] = [[CraftSlot::Empty; 3]; 3];

/// Build a 3×3 grid from explicit `(row, col, slot)` placements.
fn grid(cells: &[(usize, usize, CraftSlot)]) -> [[CraftSlot; 3]; 3] {
    let mut g = EMPTY_GRID;
    for &(r, c, slot) in cells {
        g[r][c] = slot;
    }
    g
}

/// Does this grid need a 3×3 workbench, or fit the 2×2 player grid?
/// Derived from the bounding box of non-empty cells.
fn station_for(g: &[[CraftSlot; 3]; 3]) -> CraftStation {
    let mut max_r = 0usize;
    let mut max_c = 0usize;
    let mut min_r = 2usize;
    let mut min_c = 2usize;
    let mut any = false;
    for (r, row) in g.iter().enumerate() {
        for (c, slot) in row.iter().enumerate() {
            if *slot != CraftSlot::Empty {
                any = true;
                max_r = max_r.max(r);
                max_c = max_c.max(c);
                min_r = min_r.min(r);
                min_c = min_c.min(c);
            }
        }
    }
    if !any {
        return CraftStation::PlayerGrid;
    }
    let h = max_r - min_r + 1;
    let w = max_c - min_c + 1;
    if h <= 2 && w <= 2 {
        CraftStation::PlayerGrid
    } else {
        CraftStation::Workbench
    }
}

const BLOCK_PLANKS: CraftSlot = CraftSlot::Block(block::OAK_PLANKS);
const STICK: CraftSlot = CraftSlot::Material(MaterialId::Stick);
const STRING: CraftSlot = CraftSlot::Material(MaterialId::String);

/// The grid slot that yields each tool material under the matcher's
/// `material_from_slot`.
fn tool_mat_slot(mat: ToolMaterial) -> CraftSlot {
    match mat {
        ToolMaterial::Wood => BLOCK_PLANKS,
        ToolMaterial::Stone => CraftSlot::Block(block::COBBLESTONE),
        ToolMaterial::Iron => CraftSlot::Material(MaterialId::IronIngot),
        ToolMaterial::Diamond => CraftSlot::Material(MaterialId::Diamond),
        ToolMaterial::Satori => CraftSlot::Material(MaterialId::Satori),
    }
}

fn tool_mat_name(mat: ToolMaterial) -> &'static str {
    match mat {
        ToolMaterial::Wood => "Wooden",
        ToolMaterial::Stone => "Stone",
        ToolMaterial::Iron => "Iron",
        ToolMaterial::Diamond => "Diamond",
        ToolMaterial::Satori => "Satori",
    }
}

/// Representative ingredient item for a tool material (for the bill).
fn tool_mat_item(mat: ToolMaterial) -> Item {
    match tool_mat_slot(mat) {
        CraftSlot::Block(b) => Item::Block(b),
        CraftSlot::Material(m) => Item::Material(m),
        CraftSlot::Empty => Item::Material(MaterialId::Stick),
    }
}

// ── Generated families ──────────────────────────────────────────────────────

const TOOL_MATERIALS: [ToolMaterial; 5] = [
    ToolMaterial::Wood,
    ToolMaterial::Stone,
    ToolMaterial::Iron,
    ToolMaterial::Diamond,
    ToolMaterial::Satori,
];

/// Tools whose grid is `material_from_slot`-parametric: (type, head-count,
/// stick-count, grid-builder taking the material slot).
fn push_tool_cards(out: &mut Vec<RecipeCard>) {
    // Each entry: tool type, display suffix, head count, stick count, grid.
    type Builder = fn(CraftSlot) -> [[CraftSlot; 3]; 3];

    let sword: Builder = |m| grid(&[(0, 0, m), (1, 0, m), (2, 0, STICK)]);
    let shovel: Builder = |m| grid(&[(0, 0, m), (1, 0, STICK), (2, 0, STICK)]);
    let pickaxe: Builder =
        |m| grid(&[(0, 0, m), (0, 1, m), (0, 2, m), (1, 1, STICK), (2, 1, STICK)]);
    let axe: Builder = |m| grid(&[(0, 0, m), (0, 1, m), (1, 0, m), (1, 1, STICK), (2, 1, STICK)]);
    let hoe: Builder = |m| grid(&[(0, 0, m), (0, 1, m), (1, 1, STICK), (2, 1, STICK)]);

    let specs: [(ToolType, &str, u8, u8, Builder); 5] = [
        (ToolType::Sword, "Sword", 2, 1, sword),
        (ToolType::Shovel, "Shovel", 1, 2, shovel),
        (ToolType::Pickaxe, "Pickaxe", 3, 2, pickaxe),
        (ToolType::Axe, "Axe", 3, 2, axe),
        (ToolType::Hoe, "Hoe", 2, 2, hoe),
    ];

    for (ttype, suffix, heads, sticks, build) in specs {
        for mat in TOOL_MATERIALS {
            let g = build(tool_mat_slot(mat));
            let category = if ttype == ToolType::Sword {
                RecipeCategory::Combat
            } else {
                RecipeCategory::Tools
            };
            out.push(RecipeCard {
                name: format!("{} {}", tool_mat_name(mat), suffix),
                output: ItemStack::new_tool(Tool::new(ttype, mat)),
                ingredients: vec![
                    RecipeIngredient::fixed(tool_mat_item(mat), heads),
                    RecipeIngredient::fixed(Item::Material(MaterialId::Stick), sticks),
                ],
                example_grid: g,
                category,
                station: station_for(&g),
                usage: None,
            });
        }
    }
}

/// Bows: one curve pattern, centre cell selects the tier (Empty = Wood).
fn push_bow_cards(out: &mut Vec<RecipeCard>) {
    // tier material → centre slot (matcher's bow_tier_from_slot)
    let centre = |mat: ToolMaterial| match mat {
        ToolMaterial::Wood => CraftSlot::Empty,
        other => tool_mat_slot(other),
    };
    for mat in TOOL_MATERIALS {
        let g = grid(&[
            (0, 1, STICK),
            (0, 2, STRING),
            (1, 0, STICK),
            (1, 1, centre(mat)),
            (1, 2, STRING),
            (2, 1, STICK),
            (2, 2, STRING),
        ]);
        let mut ingredients = vec![
            RecipeIngredient::fixed(Item::Material(MaterialId::Stick), 3),
            RecipeIngredient::fixed(Item::Material(MaterialId::String), 3),
        ];
        if mat != ToolMaterial::Wood {
            ingredients.push(RecipeIngredient::fixed(tool_mat_item(mat), 1));
        }
        out.push(RecipeCard {
            name: format!("{} Bow", tool_mat_name(mat)),
            output: ItemStack::new_tool(Tool::new(ToolType::Bow, mat)),
            ingredients,
            example_grid: g,
            category: RecipeCategory::Combat,
            station: station_for(&g),
            usage: None,
        });
    }
}

/// Armour: per-piece grids, parametric on `armour_material_from_slot`.
fn push_armour_cards(out: &mut Vec<RecipeCard>) {
    use crate::armour::{ArmourMaterial, ArmourSlot};

    let mat_slot = |m: ArmourMaterial| match m {
        ArmourMaterial::Leather => CraftSlot::Material(MaterialId::Leather),
        ArmourMaterial::Iron => CraftSlot::Material(MaterialId::IronIngot),
        ArmourMaterial::Diamond => CraftSlot::Material(MaterialId::Diamond),
        ArmourMaterial::Satori => CraftSlot::Material(MaterialId::Satori),
        ArmourMaterial::Rubber => CraftSlot::Material(MaterialId::Rubber),
        ArmourMaterial::Chainmail => CraftSlot::Empty, // drop-only, never crafted
    };
    let mat_item = |m: ArmourMaterial| match mat_slot(m) {
        CraftSlot::Material(id) => Item::Material(id),
        _ => Item::Material(MaterialId::Leather),
    };
    let mat_name = |m: ArmourMaterial| match m {
        ArmourMaterial::Leather => "Leather",
        ArmourMaterial::Iron => "Iron",
        ArmourMaterial::Diamond => "Diamond",
        ArmourMaterial::Satori => "Satori",
        ArmourMaterial::Rubber => "Rubber",
        ArmourMaterial::Chainmail => "Chainmail",
    };

    // Piece: slot, display, head count, grid-builder.
    let helmet = |m: CraftSlot| grid(&[(0, 0, m), (0, 1, m), (0, 2, m), (1, 0, m), (1, 2, m)]);
    let boots = |m: CraftSlot| grid(&[(0, 0, m), (0, 2, m), (1, 0, m), (1, 2, m)]);
    let chest = |m: CraftSlot| {
        grid(&[
            (0, 0, m), (0, 2, m),
            (1, 0, m), (1, 1, m), (1, 2, m),
            (2, 0, m), (2, 1, m), (2, 2, m),
        ])
    };
    let leggings = |m: CraftSlot| {
        grid(&[
            (0, 0, m), (0, 1, m), (0, 2, m),
            (1, 0, m), (1, 2, m),
            (2, 0, m), (2, 2, m),
        ])
    };

    // Material sets per piece. The matcher actually accepts Rubber for every
    // piece, but Rubber armour is intended boots-only (Spec 28e), so the
    // catalogue advertises Rubber only on Boots. Subset → still consistent.
    let standard = [
        ArmourMaterial::Leather,
        ArmourMaterial::Iron,
        ArmourMaterial::Diamond,
        ArmourMaterial::Satori,
    ];
    let boots_mats = [
        ArmourMaterial::Leather,
        ArmourMaterial::Iron,
        ArmourMaterial::Diamond,
        ArmourMaterial::Satori,
        ArmourMaterial::Rubber,
    ];

    let pieces: [(ArmourSlot, &str, u8, &[ArmourMaterial], fn(CraftSlot) -> [[CraftSlot; 3]; 3]); 4] = [
        (ArmourSlot::Helmet, "Helmet", 5, &standard, helmet),
        (ArmourSlot::Chestplate, "Chestplate", 8, &standard, chest),
        (ArmourSlot::Leggings, "Leggings", 7, &standard, leggings),
        (ArmourSlot::Boots, "Boots", 4, &boots_mats, boots),
    ];

    for (slot, suffix, heads, mats, build) in pieces {
        for &m in mats {
            let g = build(mat_slot(m));
            out.push(RecipeCard {
                name: format!("{} {}", mat_name(m), suffix),
                output: ItemStack::new_armour(slot, m),
                ingredients: vec![RecipeIngredient::fixed(mat_item(m), heads)],
                example_grid: g,
                category: RecipeCategory::Combat,
                station: station_for(&g),
                usage: None,
            });
        }
    }
}

/// The 16 dyes, with display colour names. Order matches the dye ladder.
const DYES: [(MaterialId, &str); 16] = [
    (MaterialId::WhiteDye, "White"),
    (MaterialId::BlackDye, "Black"),
    (MaterialId::RedDye, "Red"),
    (MaterialId::BlueDye, "Blue"),
    (MaterialId::YellowDye, "Yellow"),
    (MaterialId::OrangeDye, "Orange"),
    (MaterialId::GreenDye, "Green"),
    (MaterialId::PurpleDye, "Purple"),
    (MaterialId::PinkDye, "Pink"),
    (MaterialId::LimeDye, "Lime"),
    (MaterialId::LightBlueDye, "Light Blue"),
    (MaterialId::GreyDye, "Grey"),
    (MaterialId::LightGreyDye, "Light Grey"),
    (MaterialId::BrownDye, "Brown"),
    (MaterialId::CyanDye, "Cyan"),
    (MaterialId::MagentaDye, "Magenta"),
];

/// Dye-driven décor: 5 families × 16 dyes, generated by calling the same
/// `*_for_dye` mapping functions the matcher uses, so output + grid come from
/// one source and can't disagree.
fn push_dye_decor_cards(out: &mut Vec<RecipeCard>) {
    let cloth = CraftSlot::Material(MaterialId::Cloth);
    let canvas = CraftSlot::Material(MaterialId::Canvas);
    let papyrus = CraftSlot::Material(MaterialId::PapyrusSheet);

    for (dye, colour) in DYES {
        let d = CraftSlot::Material(dye);

        // Banner — column: Dye / Cloth / Stick → 1 banner.
        if let Some(b) = crate::crafting::banner_for_dye(dye) {
            let g = grid(&[(0, 0, d), (1, 0, cloth), (2, 0, STICK)]);
            out.push(card_decor(
                format!("{} Banner", colour),
                ItemStack::new_block(b, 1),
                vec![mat(dye, 1), mat(MaterialId::Cloth, 1), mat(MaterialId::Stick, 1)],
                g,
            ));
        }
        // Sail — column: Dye / Canvas / Stick → 1 sail.
        if let Some(b) = crate::crafting::sail_for_dye(dye) {
            let g = grid(&[(0, 0, d), (1, 0, canvas), (2, 0, STICK)]);
            out.push(card_decor(
                format!("{} Sail", colour),
                ItemStack::new_block(b, 1),
                vec![mat(dye, 1), mat(MaterialId::Canvas, 1), mat(MaterialId::Stick, 1)],
                g,
            ));
        }
        // Bunting — row: Dye / String / Dye → 4 bunting.
        if let Some(b) = crate::crafting::bunting_for_dye(dye) {
            let g = grid(&[(0, 0, d), (0, 1, STRING), (0, 2, d)]);
            out.push(card_decor(
                format!("{} Bunting", colour),
                ItemStack::new_block(b, 4),
                vec![mat(dye, 2), mat(MaterialId::String, 1)],
                g,
            ));
        }
        // Paper Lantern — row: Papyrus / Stick / Dye → 1 lantern.
        if let Some(b) = crate::crafting::paper_lantern_for_dye(dye) {
            let g = grid(&[(0, 0, papyrus), (0, 1, STICK), (0, 2, d)]);
            out.push(card_decor(
                format!("{} Paper Lantern", colour),
                ItemStack::new_block(b, 1),
                vec![mat(MaterialId::PapyrusSheet, 1), mat(MaterialId::Stick, 1), mat(dye, 1)],
                g,
            ));
        }
        // Kite — row: Cloth / String / Dye → 1 kite.
        if let Some(b) = crate::crafting::kite_for_dye(dye) {
            let g = grid(&[(0, 0, cloth), (0, 1, STRING), (0, 2, d)]);
            out.push(card_decor(
                format!("{} Kite", colour),
                ItemStack::new_block(b, 1),
                vec![mat(MaterialId::Cloth, 1), mat(MaterialId::String, 1), mat(dye, 1)],
                g,
            ));
        }
    }
}

/// Generated loop-friendly families: species planks, storage round-trips,
/// salt-preserved foods, magnesium items, dye mixes, wallpaper. Grids +
/// outputs come from one place each, so transcription error is minimal and
/// the consistency test covers the rest.
fn push_generated_literals(out: &mut Vec<RecipeCard>) {
    use MaterialId as M;

    // — Species log → 4 species planks (1×1) —
    let species: [(block::BlockId, block::BlockId, &str); 5] = [
        (block::BIRCH_LOG, block::BIRCH_PLANKS, "Birch"),
        (block::SPRUCE_LOG, block::SPRUCE_PLANKS, "Spruce"),
        (block::JUNGLE_LOG, block::JUNGLE_PLANKS, "Jungle"),
        (block::ACACIA_LOG, block::ACACIA_PLANKS, "Acacia"),
        (block::DARK_OAK_LOG, block::DARK_OAK_PLANKS, "Dark Oak"),
    ];
    for (log, plank, name) in species {
        out.push(card(
            &format!("{name} Planks"),
            ItemStack::new_block(plank, 4),
            vec![blk(log, 1)],
            grid(&[(0, 0, CraftSlot::Block(log))]),
            RecipeCategory::Building,
        ));
    }

    // — Storage round-trips: 9 material → block (3×3) and block → 9 material (1×1) —
    let storage: [(M, u8, block::BlockId, &str); 7] = [
        (M::Coal, 9, block::COAL_BLOCK, "Coal"),
        (M::RawIron, 9, block::IRON_BLOCK, "Iron"),
        (M::Diamond, 9, block::DIAMOND_BLOCK, "Diamond"),
        (M::Satori, 9, block::SATORI_BLOCK, "Satori"),
        (M::Bone, 9, block::BONE_BLOCK, "Bone"),
        (M::Wheat, 9, block::HAY_BALE, "Hay Bale"),
        (M::Salt, 9, block::SALT_BLOCK, "Salt"),
    ];
    for (mat_id, n, block_id, name) in storage {
        let mut cells = Vec::new();
        for r in 0..3 {
            for c in 0..3 {
                cells.push((r, c, CraftSlot::Material(mat_id)));
            }
        }
        let block_name = if name == "Hay Bale" {
            "Hay Bale".to_string()
        } else {
            format!("{name} Block")
        };
        out.push(card(
            &block_name,
            ItemStack::new_block(block_id, 1),
            vec![mat(mat_id, n)],
            grid(&cells),
            RecipeCategory::Building,
        ));
        out.push(card(
            &format!("{n}× {name} (unpack)"),
            ItemStack::new_material(mat_id, n),
            vec![blk(block_id, 1)],
            grid(&[(0, 0, CraftSlot::Block(block_id))]),
            RecipeCategory::Materials,
        ));
    }
    // Amethyst is a 4-pack round-trip.
    out.push(card(
        "Amethyst Block",
        ItemStack::new_block(block::AMETHYST_BLOCK, 1),
        vec![mat(M::Amethyst, 4)],
        grid(&[
            (0, 0, CraftSlot::Material(M::Amethyst)), (0, 1, CraftSlot::Material(M::Amethyst)),
            (1, 0, CraftSlot::Material(M::Amethyst)), (1, 1, CraftSlot::Material(M::Amethyst)),
        ]),
        RecipeCategory::Building,
    ));
    out.push(card(
        "4× Amethyst (unpack)",
        ItemStack::new_material(M::Amethyst, 4),
        vec![blk(block::AMETHYST_BLOCK, 1)],
        grid(&[(0, 0, CraftSlot::Block(block::AMETHYST_BLOCK))]),
        RecipeCategory::Materials,
    ));

    // — Salt-preserved foods (1×2: salt + food) —
    let cured: [(M, M, &str); 6] = [
        (M::RawBeef, M::SaltCuredBeef, "Salt-Cured Beef"),
        (M::RawPorkchop, M::SaltCuredPorkchop, "Salt-Cured Porkchop"),
        (M::RawMutton, M::SaltCuredMutton, "Salt-Cured Mutton"),
        (M::RawChicken, M::SaltCuredChicken, "Salt-Cured Chicken"),
        (M::RawRabbit, M::SaltCuredRabbit, "Salt-Cured Rabbit"),
        (M::RawNostrichMeat, M::SaltCuredNostrichMeat, "Salt-Cured Nostrich Meat"),
    ];
    let seasoned: [(M, M, &str); 9] = [
        (M::Bread, M::SeasonedBread, "Seasoned Bread"),
        (M::BakedPotato, M::SeasonedBakedPotato, "Seasoned Baked Potato"),
        (M::BakedCarrot, M::SeasonedBakedCarrot, "Seasoned Baked Carrot"),
        (M::BakedCorn, M::SeasonedBakedCorn, "Seasoned Baked Corn"),
        (M::CookedBeef, M::SeasonedCookedBeef, "Seasoned Cooked Beef"),
        (M::CookedPorkchop, M::SeasonedCookedPorkchop, "Seasoned Cooked Porkchop"),
        (M::CookedMutton, M::SeasonedCookedMutton, "Seasoned Cooked Mutton"),
        (M::CookedChicken, M::SeasonedCookedChicken, "Seasoned Cooked Chicken"),
        (M::CookedRabbit, M::SeasonedCookedRabbit, "Seasoned Cooked Rabbit"),
    ];
    for (food, result, name) in cured.into_iter().chain(seasoned) {
        out.push(card(
            name,
            ItemStack::new_material(result, 1),
            vec![mat(M::Salt, 1), mat(food, 1)],
            grid(&[(0, 0, CraftSlot::Material(M::Salt)), (0, 1, CraftSlot::Material(food))]),
            RecipeCategory::Food,
        ));
    }

    // — Magnesium pairings (1×2) —
    let magnesium: [(M, M, &str); 4] = [
        (M::Sulphur, M::Fertiliser, "Fertiliser"),
        (M::Stick, M::Sparkler, "Sparkler"),
        (M::PapyrusSheet, M::Flare, "Flare"),
        (M::IronIngot, M::MagnesiumFirestarter, "Magnesium Firestarter"),
    ];
    for (other, result, name) in magnesium {
        out.push(card(
            name,
            ItemStack::new_material(result, 1),
            vec![mat(M::Magnesium, 1), mat(other, 1)],
            grid(&[(0, 0, CraftSlot::Material(M::Magnesium)), (0, 1, CraftSlot::Material(other))]),
            RecipeCategory::Materials,
        ));
    }

    // — Two-input dye mixes (1×2) —
    let dye_mix: [(M, M, M, &str); 10] = [
        (M::RedDye, M::YellowDye, M::OrangeDye, "Orange Dye"),
        (M::YellowDye, M::BlueDye, M::GreenDye, "Green Dye"),
        (M::BlueDye, M::RedDye, M::PurpleDye, "Purple Dye"),
        (M::RedDye, M::WhiteDye, M::PinkDye, "Pink Dye"),
        (M::GreenDye, M::WhiteDye, M::LimeDye, "Lime Dye"),
        (M::BlueDye, M::WhiteDye, M::LightBlueDye, "Light Blue Dye"),
        (M::WhiteDye, M::BlackDye, M::GreyDye, "Grey Dye"),
        (M::GreyDye, M::WhiteDye, M::LightGreyDye, "Light Grey Dye"),
        (M::GreenDye, M::BlueDye, M::CyanDye, "Cyan Dye"),
        (M::PurpleDye, M::PinkDye, M::MagentaDye, "Magenta Dye"),
    ];
    for (a, b, result, name) in dye_mix {
        out.push(card(
            &format!("{name} (mix)"),
            ItemStack::new_material(result, 2),
            vec![mat(a, 1), mat(b, 1)],
            grid(&[(0, 0, CraftSlot::Material(a)), (0, 1, CraftSlot::Material(b))]),
            RecipeCategory::Materials,
        ));
    }
    // Brown dye — 3 primary dyes in a row.
    out.push(card(
        "Brown Dye (mix)",
        ItemStack::new_material(M::BrownDye, 2),
        vec![mat(M::RedDye, 1), mat(M::YellowDye, 1), mat(M::BlueDye, 1)],
        grid(&[
            (0, 0, CraftSlot::Material(M::RedDye)),
            (0, 1, CraftSlot::Material(M::YellowDye)),
            (0, 2, CraftSlot::Material(M::BlueDye)),
        ]),
        RecipeCategory::Materials,
    ));

    // — Wallpaper: Papyrus + dye → 3 wallpaper (1×2), one per dye —
    for (dye, colour) in DYES {
        if let Some(b) = dye.paint_block() {
            out.push(card(
                &format!("{colour} Wallpaper"),
                ItemStack::new_block(b, 3),
                vec![mat(M::PapyrusSheet, 1), mat(dye, 1)],
                grid(&[
                    (0, 0, CraftSlot::Material(M::PapyrusSheet)),
                    (0, 1, CraftSlot::Material(dye)),
                ]),
                RecipeCategory::Decoration,
            ));
        }
    }
}

/// Decoration-category card helper (dye families are all Decoration).
fn card_decor(
    name: String,
    output: ItemStack,
    ingredients: Vec<RecipeIngredient>,
    example_grid: [[CraftSlot; 3]; 3],
) -> RecipeCard {
    let station = station_for(&example_grid);
    RecipeCard { name, output, ingredients, example_grid, category: RecipeCategory::Decoration, station, usage: None }
}

// ── Authored literal recipes ────────────────────────────────────────────────

/// One authored literal card. `ing` is the shapeless bill.
fn card(
    name: &str,
    output: ItemStack,
    ingredients: Vec<RecipeIngredient>,
    example_grid: [[CraftSlot; 3]; 3],
    category: RecipeCategory,
) -> RecipeCard {
    let station = station_for(&example_grid);
    RecipeCard { name: name.to_string(), output, ingredients, example_grid, category, station, usage: None }
}

fn mat(id: MaterialId, n: u8) -> RecipeIngredient {
    RecipeIngredient::fixed(Item::Material(id), n)
}
fn blk(b: block::BlockId, n: u8) -> RecipeIngredient {
    RecipeIngredient::fixed(Item::Block(b), n)
}

fn push_literal_cards(out: &mut Vec<RecipeCard>) {
    let p = BLOCK_PLANKS;
    let cobble = CraftSlot::Block(block::COBBLESTONE);

    // — Building basics —
    out.push(card(
        "Oak Planks",
        ItemStack::new_block(block::OAK_PLANKS, 4),
        vec![RecipeIngredient { item: Item::Block(block::OAK_LOG), count: 1, fuzzy: Some(FuzzyKind::Log) }],
        grid(&[(0, 0, CraftSlot::Block(block::OAK_LOG))]),
        RecipeCategory::Building,
    ));
    out.push(card(
        "Sticks",
        ItemStack::new_material(MaterialId::Stick, 4),
        vec![blk(block::OAK_PLANKS, 2)],
        grid(&[(0, 0, p), (1, 0, p)]),
        RecipeCategory::Materials,
    ));
    out.push(card(
        "Crafting Table",
        ItemStack::new_block(block::CRAFTING_TABLE, 1),
        vec![blk(block::OAK_PLANKS, 4)],
        grid(&[(0, 0, p), (0, 1, p), (1, 0, p), (1, 1, p)]),
        RecipeCategory::Stations,
    ));
    out.push(card(
        "Glass",
        ItemStack::new_block(block::GLASS, 4),
        vec![blk(block::SAND, 4)],
        grid(&[
            (0, 0, CraftSlot::Block(block::SAND)),
            (0, 1, CraftSlot::Block(block::SAND)),
            (1, 0, CraftSlot::Block(block::SAND)),
            (1, 1, CraftSlot::Block(block::SAND)),
        ]),
        RecipeCategory::Building,
    ));

    // — Lighting / combat consumables —
    out.push(card(
        "Torch",
        ItemStack::new_block(block::TORCH, 4),
        vec![mat(MaterialId::Coal, 1), mat(MaterialId::Stick, 1)],
        grid(&[(0, 0, CraftSlot::Material(MaterialId::Coal)), (1, 0, STICK)]),
        RecipeCategory::Building,
    ));
    out.push(card(
        "Arrow",
        ItemStack::new_material(MaterialId::Arrow, 4),
        vec![mat(MaterialId::Stick, 1), mat(MaterialId::Feather, 1)],
        grid(&[(0, 0, STICK), (1, 0, CraftSlot::Material(MaterialId::Feather))]),
        RecipeCategory::Combat,
    ));

    // — Pets wave Task 7 —
    out.push(card(
        "Recall Whistle",
        ItemStack::new_material(MaterialId::RecallWhistle, 1),
        vec![mat(MaterialId::Bone, 1), mat(MaterialId::String, 1)],
        grid(&[
            (0, 0, CraftSlot::Material(MaterialId::Bone)),
            (1, 0, CraftSlot::Material(MaterialId::String)),
        ]),
        RecipeCategory::Tools,
    ).with_usage("Right-click to call your pets and mounts to you."));

    // — Pets wave Task 9 —
    out.push(card(
        "Cat Treat",
        ItemStack::new_material(MaterialId::CatTreat, 2),
        vec![mat(MaterialId::RawFish, 1), mat(MaterialId::Wheat, 1)],
        grid(&[
            (0, 0, CraftSlot::Material(MaterialId::RawFish)),
            (1, 0, CraftSlot::Material(MaterialId::Wheat)),
        ]),
        RecipeCategory::Materials,
    ).with_usage("Always tames a cat on the first try."));

    // — Stations —
    let furnace = {
        let mut cells = vec![];
        for r in 0..3 {
            for c in 0..3 {
                if !(r == 1 && c == 1) {
                    cells.push((r, c, cobble));
                }
            }
        }
        grid(&cells)
    };
    out.push(card(
        "Furnace",
        ItemStack::new_block(block::FURNACE, 1),
        vec![blk(block::COBBLESTONE, 8)],
        furnace,
        RecipeCategory::Stations,
    ));
    let chest = {
        let mut cells = vec![];
        for r in 0..3 {
            for c in 0..3 {
                if !(r == 1 && c == 1) {
                    cells.push((r, c, p));
                }
            }
        }
        grid(&cells)
    };
    out.push(card(
        "Chest",
        ItemStack::new_block(block::CHEST, 1),
        vec![blk(block::OAK_PLANKS, 8)],
        chest,
        RecipeCategory::Stations,
    ));

    // #15 — tier chests: 8× the tier material ringing a wood chest at centre.
    for (mat_id, out_block, name) in [
        (MaterialId::CopperIngot, block::COPPER_CHEST, "Copper Chest"),
        (MaterialId::IronIngot, block::IRON_CHEST, "Iron Chest"),
        (MaterialId::Diamond, block::DIAMOND_CHEST, "Diamond Chest"),
        (MaterialId::Satori, block::SATORI_CHEST, "Satori Chest"),
    ] {
        let g = {
            let s = CraftSlot::Material(mat_id);
            let mut cells = vec![(1usize, 1usize, CraftSlot::Block(block::CHEST))];
            for r in 0..3 {
                for c in 0..3 {
                    if !(r == 1 && c == 1) {
                        cells.push((r, c, s));
                    }
                }
            }
            grid(&cells)
        };
        out.push(card(
            name,
            ItemStack::new_block(out_block, 1),
            vec![mat(mat_id, 8), blk(block::CHEST, 1)],
            g,
            RecipeCategory::Stations,
        ));
    }

    // — Food —
    out.push(card(
        "Bread",
        ItemStack::new_material(MaterialId::Bread, 1),
        vec![mat(MaterialId::Wheat, 3)],
        grid(&[
            (0, 0, CraftSlot::Material(MaterialId::Wheat)),
            (0, 1, CraftSlot::Material(MaterialId::Wheat)),
            (0, 2, CraftSlot::Material(MaterialId::Wheat)),
        ]),
        RecipeCategory::Food,
    ));

    push_more_literal_cards(out);
}

/// Tool-output card helper.
fn tool_card(
    name: &str,
    tool: crate::crafting::Tool,
    ingredients: Vec<RecipeIngredient>,
    g: [[CraftSlot; 3]; 3],
    category: RecipeCategory,
) -> RecipeCard {
    card(name, ItemStack::new_tool(tool), ingredients, g, category)
}

/// The remaining one-off literal recipes (2×1 / 2×2 / 2×3 / 3×3 stations /
/// 1×1 conversions / 1×3 / 3×1 columns / special tool shapes). Each grid is
/// validated against the live matcher by the consistency test.
fn push_more_literal_cards(out: &mut Vec<RecipeCard>) {
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use MaterialId as M;
    let p = BLOCK_PLANKS;
    let cobble = CraftSlot::Block(block::COBBLESTONE);
    let iron = CraftSlot::Material(M::IronIngot);
    let blk_at = |b: block::BlockId| CraftSlot::Block(b);
    let m_at = |id: M| CraftSlot::Material(id);

    // ── 1×1 conversions ───────────────────────────────────────────────────
    out.push(card("Bone Meal", ItemStack::new_material(M::Bonemeal, 3), vec![mat(M::Bone, 1)],
        grid(&[(0, 0, m_at(M::Bone))]), RecipeCategory::Materials)
        .with_usage("Right-click crops to make them grow."));
    out.push(card("Blue Dye", ItemStack::new_material(M::BlueDye, 1), vec![blk(block::CORNFLOWER, 1)],
        grid(&[(0, 0, blk_at(block::CORNFLOWER))]), RecipeCategory::Materials));
    out.push(card("Red Dye", ItemStack::new_material(M::RedDye, 1), vec![blk(block::FIELD_POPPY, 1)],
        grid(&[(0, 0, blk_at(block::FIELD_POPPY))]), RecipeCategory::Materials));
    out.push(card("Yellow Dye", ItemStack::new_material(M::YellowDye, 1), vec![blk(block::BUTTERCUP, 1)],
        grid(&[(0, 0, blk_at(block::BUTTERCUP))]), RecipeCategory::Materials));
    out.push(card("Black Dye", ItemStack::new_material(M::BlackDye, 1), vec![mat(M::InkSac, 1)],
        grid(&[(0, 0, m_at(M::InkSac))]), RecipeCategory::Materials));
    out.push(card("White Dye", ItemStack::new_material(M::WhiteDye, 1), vec![mat(M::Bonemeal, 1)],
        grid(&[(0, 0, m_at(M::Bonemeal))]), RecipeCategory::Materials));
    out.push(card("String (from Cotton)", ItemStack::new_material(M::String, 2), vec![mat(M::Cotton, 1)],
        grid(&[(0, 0, m_at(M::Cotton))]), RecipeCategory::Materials));
    out.push(card("Rubber Balls", ItemStack::new_material(M::RubberBall, 4), vec![mat(M::Rubber, 1)],
        grid(&[(0, 0, m_at(M::Rubber))]), RecipeCategory::Materials));

    // ── 2×1 vertical ──────────────────────────────────────────────────────
    out.push(card("Purple Banner (Nostrich)", ItemStack::new_material(M::PurpleBanner, 1),
        vec![mat(M::Wool, 1), mat(M::NostrichFeather, 1)],
        grid(&[(0, 0, m_at(M::Wool)), (1, 0, m_at(M::NostrichFeather))]), RecipeCategory::Decoration));
    out.push(tool_card("Flint and Steel", Tool::new(ToolType::FlintAndSteel, ToolMaterial::Iron),
        vec![mat(M::Flint, 1), mat(M::IronIngot, 1)],
        grid(&[(0, 0, m_at(M::Flint)), (1, 0, iron)]), RecipeCategory::Tools));
    out.push(card("Bronze Ingot", ItemStack::new_material(M::BronzeIngot, 1),
        vec![mat(M::CopperIngot, 1), mat(M::TinIngot, 1)],
        grid(&[(0, 0, m_at(M::CopperIngot)), (1, 0, m_at(M::TinIngot))]), RecipeCategory::Materials));
    out.push(card("Salt Lick", ItemStack::new_block(block::SALT_LICK, 1),
        vec![blk(block::SALT_BLOCK, 1), blk(block::COBBLESTONE, 1)],
        grid(&[(0, 0, blk_at(block::SALT_BLOCK)), (1, 0, cobble)]), RecipeCategory::Building)
        .with_usage("Place it — nearby animals heal faster and drop a bit more."));
    out.push(tool_card("Eraser", Tool::new(ToolType::Eraser, ToolMaterial::Wood),
        vec![mat(M::Rubber, 1), mat(M::Stick, 1)],
        grid(&[(0, 0, m_at(M::Rubber)), (1, 0, STICK)]), RecipeCategory::Tools));
    out.push(tool_card("Drafting Stamp", Tool::new(ToolType::DraftingStamp, ToolMaterial::Wood),
        vec![mat(M::IronIngot, 1), blk(block::BLUEPRINT_PAPER, 1)],
        grid(&[(0, 0, iron), (1, 0, blk_at(block::BLUEPRINT_PAPER))]), RecipeCategory::Tools));
    out.push(tool_card("Shears", Tool::new(ToolType::Shears, ToolMaterial::Iron),
        vec![mat(M::IronIngot, 2)],
        grid(&[(0, 0, iron), (1, 0, iron)]), RecipeCategory::Tools)
        .with_usage("Right-click a sheep for wool."));

    // ── 2×2 ───────────────────────────────────────────────────────────────
    out.push(card("Drying Rack", ItemStack::new_block(block::DRYING_RACK, 1), vec![mat(M::Stick, 4)],
        grid(&[(0, 0, STICK), (0, 1, STICK), (1, 0, STICK), (1, 1, STICK)]), RecipeCategory::Stations));
    out.push(card("Drafting Table", ItemStack::new_block(block::DRAFTING_TABLE, 1),
        vec![RecipeIngredient { item: Item::Material(M::PapyrusSheet), count: 1, fuzzy: Some(FuzzyKind::Paper) }, blk(block::OAK_PLANKS, 3)],
        grid(&[(0, 0, m_at(M::PapyrusSheet)), (0, 1, p), (1, 0, p), (1, 1, p)]), RecipeCategory::Stations));
    out.push(card("Cloth", ItemStack::new_material(M::Cloth, 1), vec![mat(M::Cotton, 4)],
        grid(&[(0, 0, m_at(M::Cotton)), (0, 1, m_at(M::Cotton)), (1, 0, m_at(M::Cotton)), (1, 1, m_at(M::Cotton))]),
        RecipeCategory::Materials));
    out.push(card("Canvas", ItemStack::new_material(M::Canvas, 1), vec![mat(M::HempFibre, 4)],
        grid(&[(0, 0, m_at(M::HempFibre)), (0, 1, m_at(M::HempFibre)), (1, 0, m_at(M::HempFibre)), (1, 1, m_at(M::HempFibre))]),
        RecipeCategory::Materials));

    // ── 2×3 ───────────────────────────────────────────────────────────────
    out.push(card("Bed", ItemStack::new_block(block::BED, 1),
        vec![mat(M::Wool, 3), blk(block::OAK_PLANKS, 3)],
        grid(&[(0, 0, m_at(M::Wool)), (0, 1, m_at(M::Wool)), (0, 2, m_at(M::Wool)), (1, 0, p), (1, 1, p), (1, 2, p)]),
        RecipeCategory::Building));
    out.push(card("Pet Bed", ItemStack::new_block(block::PET_BED, 1),
        vec![mat(M::Wool, 2), blk(block::OAK_PLANKS, 3)],
        grid(&[(0, 0, m_at(M::Wool)), (0, 2, m_at(M::Wool)), (1, 0, p), (1, 1, p), (1, 2, p)]),
        RecipeCategory::Building));
    out.push(card("Tent", ItemStack::new_block(block::TENT, 1),
        vec![mat(M::Canvas, 3), mat(M::Stick, 2)],
        grid(&[(0, 0, m_at(M::Canvas)), (0, 1, m_at(M::Canvas)), (0, 2, m_at(M::Canvas)), (1, 0, STICK), (1, 2, STICK)]),
        RecipeCategory::Building));
    out.push(card("Fence Post", ItemStack::new_block(block::OAK_FENCE_POST, 3),
        vec![blk(block::OAK_PLANKS, 4), mat(M::Stick, 2)],
        grid(&[(0, 0, p), (0, 1, STICK), (0, 2, p), (1, 0, p), (1, 1, STICK), (1, 2, p)]),
        RecipeCategory::Building));
    out.push(card("Wood Cart", ItemStack::new_material(M::WoodCart, 1), vec![blk(block::OAK_PLANKS, 5)],
        grid(&[(0, 0, p), (0, 2, p), (1, 0, p), (1, 1, p), (1, 2, p)]), RecipeCategory::Transport));

    // ── 3×3 stations / blocks ─────────────────────────────────────────────
    let ring = |centre: CraftSlot, edge: CraftSlot| {
        let mut cells = Vec::new();
        for r in 0..3 {
            for c in 0..3 {
                cells.push((r, c, if r == 1 && c == 1 { centre } else { edge }));
            }
        }
        grid(&cells)
    };
    out.push(card("Vendor Block", ItemStack::new_block(block::VENDOR_BLOCK, 1),
        vec![blk(block::OAK_PLANKS, 8), mat(M::IronIngot, 1)], ring(iron, p), RecipeCategory::Stations));
    out.push(card("Bazaar Block", ItemStack::new_material(M::BazaarBlockItem, 1),
        vec![blk(block::OAK_PLANKS, 8), mat(M::Diamond, 1)], ring(m_at(M::Diamond), p), RecipeCategory::Stations));
    out.push(card("Bounty Board", ItemStack::new_material(M::BountyBoardItem, 1),
        vec![blk(block::OAK_PLANKS, 8), mat(M::PapyrusSheet, 1)], ring(m_at(M::PapyrusSheet), p), RecipeCategory::Stations));
    out.push(card("Iron Cart", ItemStack::new_material(M::IronCart, 1),
        vec![mat(M::IronIngot, 8), mat(M::WoodCart, 1)], ring(m_at(M::WoodCart), iron), RecipeCategory::Transport));
    out.push(card("Diamond Cart", ItemStack::new_material(M::DiamondCart, 1),
        vec![mat(M::Diamond, 8), mat(M::IronCart, 1)], ring(m_at(M::IronCart), m_at(M::Diamond)), RecipeCategory::Transport));
    out.push(card("Track", ItemStack::new_block(crate::rail::TRACK, 16),
        vec![mat(M::IronIngot, 6), mat(M::Stick, 1)],
        grid(&[(0, 0, iron), (0, 2, iron), (1, 0, iron), (1, 1, STICK), (1, 2, iron), (2, 0, iron), (2, 2, iron)]),
        RecipeCategory::Transport));
    out.push(card("Auction Block", ItemStack::new_material(M::AuctionBlockItem, 1),
        vec![mat(M::IronIngot, 4), blk(block::OAK_PLANKS, 5)],
        grid(&[(0, 0, iron), (0, 1, p), (0, 2, iron), (1, 0, p), (1, 1, p), (1, 2, p), (2, 0, iron), (2, 1, p), (2, 2, iron)]),
        RecipeCategory::Stations));
    out.push(card("Plot Marker", ItemStack::new_material(M::PlotMarkerItem, 1),
        vec![mat(M::IronIngot, 4), blk(block::OAK_PLANKS, 1)],
        grid(&[(0, 0, iron), (0, 2, iron), (1, 1, p), (2, 0, iron), (2, 2, iron)]), RecipeCategory::Stations));
    out.push(card("Repair Bench", ItemStack::new_material(M::RepairBenchItem, 1),
        vec![mat(M::IronIngot, 3), blk(block::STONE, 4)],
        grid(&[(0, 0, iron), (0, 1, iron), (0, 2, iron), (1, 1, blk_at(block::STONE)), (2, 0, blk_at(block::STONE)), (2, 1, blk_at(block::STONE)), (2, 2, blk_at(block::STONE))]),
        RecipeCategory::Stations));
    out.push(card("Tip Jar", ItemStack::new_material(M::TipJarItem, 1),
        vec![blk(block::OAK_PLANKS, 8), mat(M::IronIngot, 2)],
        grid(&[(0, 0, p), (0, 1, iron), (0, 2, p), (1, 0, p), (1, 1, iron), (1, 2, p), (2, 0, p), (2, 1, p), (2, 2, p)]),
        RecipeCategory::Stations));
    out.push(card("Salt Lamp", ItemStack::new_block(block::SALT_LAMP, 1),
        vec![mat(M::Salt, 4), mat(M::Stick, 1)],
        grid(&[(0, 1, m_at(M::Salt)), (1, 0, m_at(M::Salt)), (1, 1, STICK), (1, 2, m_at(M::Salt)), (2, 1, m_at(M::Salt))]),
        RecipeCategory::Building));
    out.push(tool_card("Slingshot", Tool::new(ToolType::Slingshot, ToolMaterial::Wood),
        vec![mat(M::Stick, 3), mat(M::Rubber, 1)],
        grid(&[(0, 0, STICK), (0, 2, STICK), (1, 1, m_at(M::Rubber)), (2, 1, STICK)]), RecipeCategory::Combat));

    // ── 1×3 horizontal ────────────────────────────────────────────────────
    out.push(card("Papyrus Sheet", ItemStack::new_material(M::PapyrusSheet, 3), vec![mat(M::PapyrusReed, 3)],
        grid(&[(0, 0, m_at(M::PapyrusReed)), (0, 1, m_at(M::PapyrusReed)), (0, 2, m_at(M::PapyrusReed))]), RecipeCategory::Materials));
    out.push(card("Copper Cable", ItemStack::new_material(M::CopperCable, 2),
        vec![mat(M::CopperIngot, 2), mat(M::Rubber, 1)],
        grid(&[(0, 0, m_at(M::CopperIngot)), (0, 1, m_at(M::Rubber)), (0, 2, m_at(M::CopperIngot))]), RecipeCategory::Materials));
    // ── Spec 48 (Electricity) — power blocks (catalogue ↔ matcher; shapes
    //    are the spec's proposed set, owner/Axolittle to confirm feel) ──
    out.push(card("Cable", ItemStack::new_block(block::CABLE, 3),
        vec![mat(M::Rubber, 2), mat(M::CopperIngot, 1)],
        grid(&[(0, 0, m_at(M::Rubber)), (0, 1, m_at(M::CopperIngot)), (0, 2, m_at(M::Rubber))]), RecipeCategory::Building));
    out.push(card("Electric Lamp", ItemStack::new_block(block::ELECTRIC_LAMP, 1),
        vec![blk(block::GLASS, 2), mat(M::CopperIngot, 1)],
        grid(&[(0, 0, blk_at(block::GLASS)), (0, 1, m_at(M::CopperIngot)), (0, 2, blk_at(block::GLASS))]), RecipeCategory::Building));
    out.push(card("Logic Gate", ItemStack::new_block(block::LOGIC_GATE, 1),
        vec![mat(M::IronIngot, 2), mat(M::CopperCable, 1)],
        grid(&[(0, 0, iron), (0, 1, m_at(M::CopperCable)), (0, 2, iron)]), RecipeCategory::Stations));
    // Spec 48 Phase 2 — sensors.
    out.push(card("Mirror", ItemStack::new_block(block::MIRROR, 1),
        vec![mat(M::IronIngot, 2), blk(block::GLASS, 1)],
        grid(&[(0, 0, iron), (0, 1, blk_at(block::GLASS)), (0, 2, iron)]), RecipeCategory::Building));
    out.push(card("Beam Sensor", ItemStack::new_block(block::BEAM_SENSOR, 1),
        vec![blk(block::GLASS, 1), mat(M::CopperIngot, 1), mat(M::IronIngot, 1)],
        grid(&[(0, 0, blk_at(block::GLASS)), (0, 1, m_at(M::CopperIngot)), (0, 2, iron)]), RecipeCategory::Stations));
    out.push(card("Motion Sensor", ItemStack::new_block(block::MOTION_SENSOR, 1),
        vec![mat(M::CopperIngot, 2), blk(block::GLASS, 1)],
        grid(&[(0, 0, m_at(M::CopperIngot)), (0, 1, blk_at(block::GLASS)), (0, 2, m_at(M::CopperIngot))]), RecipeCategory::Stations));
    // Spec 49 (Explosives) — the Plunger Detonator's recipe was specified
    // in docs/foundations/2026-06-20-explosives-blasting-keg.md ("Recipe:
    // Iron Ingot + Cable + Planks — a boxed switch") but was never actually
    // wired into match_recipe, leaving the block craftable only via
    // creative/`/give` (wiki audit 2026-07-09, finding #1).
    out.push(card("Plunger Detonator", ItemStack::new_block(block::PLUNGER_DETONATOR, 1),
        vec![mat(M::IronIngot, 1), mat(M::CopperCable, 1), blk(block::OAK_PLANKS, 1)],
        grid(&[(0, 0, iron), (0, 1, m_at(M::CopperCable)), (0, 2, blk_at(block::OAK_PLANKS))]), RecipeCategory::Stations));
    out.push(card("Button", ItemStack::new_block(block::BUTTON, 1),
        vec![blk(block::STONE, 1)],
        grid(&[(0, 0, blk_at(block::STONE))]), RecipeCategory::Building));
    out.push(card("Pressure Plate", ItemStack::new_block(block::PRESSURE_PLATE, 1),
        vec![blk(block::STONE, 2)],
        grid(&[(0, 0, blk_at(block::STONE)), (0, 1, blk_at(block::STONE))]), RecipeCategory::Building));
    out.push(card("Lever", ItemStack::new_block(block::LEVER, 1),
        vec![mat(M::Stick, 1), blk(block::COBBLESTONE, 1)],
        grid(&[(0, 0, STICK), (1, 0, blk_at(block::COBBLESTONE))]), RecipeCategory::Building));
    out.push(card("Hand Crank", ItemStack::new_block(block::HAND_CRANK, 1),
        vec![mat(M::Stick, 1), mat(M::CopperIngot, 1), blk(block::OAK_PLANKS, 1)],
        grid(&[(0, 0, STICK), (1, 0, m_at(M::CopperIngot)), (2, 0, p)]), RecipeCategory::Stations));
    out.push(card("Battery", ItemStack::new_block(block::BATTERY, 1),
        vec![mat(M::CopperIngot, 2), mat(M::Coal, 1)],
        grid(&[(0, 0, m_at(M::CopperIngot)), (1, 0, m_at(M::Coal)), (2, 0, m_at(M::CopperIngot))]), RecipeCategory::Stations));
    // Dispenser / Dropper (2026-07-04 gap-fill wave) — cobble ring, cable at
    // the bottom (piston convention); the arrow centre makes the shooter.
    out.push(card("Dispenser", ItemStack::new_block(block::DISPENSER, 1),
        vec![blk(block::COBBLESTONE, 7), mat(M::Arrow, 1), blk(block::CABLE, 1)],
        grid(&[
            (0, 0, blk_at(block::COBBLESTONE)), (0, 1, blk_at(block::COBBLESTONE)), (0, 2, blk_at(block::COBBLESTONE)),
            (1, 0, blk_at(block::COBBLESTONE)), (1, 1, m_at(M::Arrow)), (1, 2, blk_at(block::COBBLESTONE)),
            (2, 0, blk_at(block::COBBLESTONE)), (2, 1, blk_at(block::CABLE)), (2, 2, blk_at(block::COBBLESTONE)),
        ]), RecipeCategory::Stations));
    out.push(card("Dropper", ItemStack::new_block(block::DROPPER, 1),
        vec![blk(block::COBBLESTONE, 7), blk(block::CABLE, 1)],
        grid(&[
            (0, 0, blk_at(block::COBBLESTONE)), (0, 1, blk_at(block::COBBLESTONE)), (0, 2, blk_at(block::COBBLESTONE)),
            (1, 0, blk_at(block::COBBLESTONE)), (1, 2, blk_at(block::COBBLESTONE)),
            (2, 0, blk_at(block::COBBLESTONE)), (2, 1, blk_at(block::CABLE)), (2, 2, blk_at(block::COBBLESTONE)),
        ]), RecipeCategory::Stations));
    out.push(card("Steam Generator", ItemStack::new_block(block::STEAM_GENERATOR, 1),
        vec![mat(M::IronIngot, 8), mat(M::CopperIngot, 1)],
        ring(m_at(M::CopperIngot), iron), RecipeCategory::Stations));
    // Spec 48 Phase 4 — Water Wheel: eight plank paddles ringing a copper axle.
    // It only turns in *flowing* water, so a pond powers nothing — cut a channel.
    out.push(card("Water Wheel", ItemStack::new_block(block::WATER_WHEEL, 1),
        vec![blk(block::OAK_PLANKS, 8), mat(M::CopperIngot, 1)],
        ring(m_at(M::CopperIngot), p), RecipeCategory::Stations));
    // Wind, Copper & Electricity wave §2.2 — Windmill: canvas sails on a
    // plank-and-copper hub over a stick-and-iron trestle. The Water Wheel's dry
    // twin — it turns on the wind, so it wants open sky and high ground.
    out.push(card("Windmill", ItemStack::new_block(block::WINDMILL, 1),
        vec![mat(M::Canvas, 3), blk(block::OAK_PLANKS, 2), mat(M::CopperIngot, 1),
             mat(M::Stick, 2), mat(M::IronIngot, 1)],
        grid(&[
            (0, 0, m_at(M::Canvas)), (0, 1, m_at(M::Canvas)), (0, 2, m_at(M::Canvas)),
            (1, 0, p), (1, 1, m_at(M::CopperIngot)), (1, 2, p),
            (2, 0, m_at(M::Stick)), (2, 1, m_at(M::IronIngot)), (2, 2, m_at(M::Stick)),
        ]), RecipeCategory::Stations)
        .with_usage("Build it high, with open sky above — it only turns in the wind."));
    out.push(card("Nostrich Omelette", ItemStack::new_material(M::NostrichOmelette, 1),
        vec![mat(M::NostrichEgg, 1), mat(M::Flour, 1), mat(M::Wheat, 1)],
        grid(&[(0, 0, m_at(M::NostrichEgg)), (0, 1, m_at(M::Flour)), (0, 2, m_at(M::Wheat))]), RecipeCategory::Food));
    // Spec 49 (Explosives) — Black Powder (Sulphur + Coal + Saltpetre, any order)
    // and the Blasting Keg (8 planks ringing 1 Black Powder; never the red cube).
    out.push(card("Black Powder", ItemStack::new_material(M::BlackPowder, 3),
        vec![mat(M::Sulphur, 1), mat(M::Coal, 1), mat(M::Saltpetre, 1)],
        grid(&[(0, 0, m_at(M::Sulphur)), (0, 1, m_at(M::Coal)), (0, 2, m_at(M::Saltpetre))]),
        RecipeCategory::Materials));
    out.push(card("Blasting Keg", ItemStack::new_block(block::BLASTING_KEG, 1),
        vec![blk(block::OAK_PLANKS, 8), mat(M::BlackPowder, 1)],
        ring(m_at(M::BlackPowder), p), RecipeCategory::Combat));

    // ── 3×1 vertical columns ──────────────────────────────────────────────
    out.push(card("Village Bell", ItemStack::new_block(block::VILLAGE_BELL, 1),
        vec![mat(M::IronIngot, 1), mat(M::Stick, 1), blk(block::OAK_PLANKS, 1)],
        grid(&[(0, 0, iron), (1, 0, STICK), (2, 0, p)]), RecipeCategory::Stations));
    out.push(card("Market Bell", ItemStack::new_material(M::MarketBellItem, 1),
        vec![mat(M::IronIngot, 2), blk(block::OAK_PLANKS, 1)],
        grid(&[(0, 0, iron), (1, 0, iron), (2, 0, p)]), RecipeCategory::Stations));
    out.push(card("Nostrich Arrow", ItemStack::new_material(M::NostrichArrow, 6),
        vec![mat(M::Stick, 1), mat(M::Flint, 1), mat(M::NostrichFeather, 1)],
        grid(&[(0, 0, STICK), (1, 0, m_at(M::Flint)), (2, 0, m_at(M::NostrichFeather))]), RecipeCategory::Combat));
    out.push(card("Trophy Wall", ItemStack::new_block(block::TROPHY_WALL, 1),
        vec![mat(M::BrigandChieftainTrophy, 1), blk(block::OAK_PLANKS, 2)],
        grid(&[(0, 0, m_at(M::BrigandChieftainTrophy)), (1, 0, p), (2, 0, p)]), RecipeCategory::Decoration));
    out.push(card("Rope", ItemStack::new_material(M::Rope, 1), vec![mat(M::HempFibre, 3)],
        grid(&[(0, 0, m_at(M::HempFibre)), (1, 0, m_at(M::HempFibre)), (2, 0, m_at(M::HempFibre))]), RecipeCategory::Materials));
    out.push(card("Lead", ItemStack::new_material(M::Lead, 1),
        vec![mat(M::Rope, 1), mat(M::String, 2)],
        grid(&[(0, 0, m_at(M::Rope)), (1, 0, m_at(M::String)), (2, 0, m_at(M::String))]), RecipeCategory::Materials)
        .with_usage("Right-click an animal to walk it on a lead."));
    out.push(card("Blueprint Paper", ItemStack::new_block(block::BLUEPRINT_PAPER, 3),
        vec![mat(M::PapyrusSheet, 1), mat(M::IronIngot, 1), mat(M::Salt, 1)],
        grid(&[(0, 0, m_at(M::PapyrusSheet)), (1, 0, iron), (2, 0, m_at(M::Salt))]), RecipeCategory::Materials));
    // Pets wave Task 13 — Reach Claw. Crab Claw on top, Stick middle +
    // bottom → 1 Reach Claw (+2 block/entity reach while held).
    out.push(card("Reach Claw", ItemStack::new_material(M::ReachClaw, 1),
        vec![mat(M::CrabClaw, 1), mat(M::Stick, 2)],
        grid(&[(0, 0, m_at(M::CrabClaw)), (1, 0, STICK), (2, 0, STICK)]), RecipeCategory::Tools)
        .with_usage("Reach 2 blocks further while held."));

    // ── Special tool shapes ───────────────────────────────────────────────
    out.push(tool_card("Fishing Rod", Tool::new(ToolType::FishingRod, ToolMaterial::Wood),
        vec![mat(M::Stick, 3), mat(M::String, 2)],
        grid(&[(0, 2, STICK), (1, 1, STICK), (1, 2, STRING), (2, 0, STICK), (2, 2, STRING)]), RecipeCategory::Tools));
    out.push(card("Campfire", ItemStack::new_block(block::CAMPFIRE_UNLIT, 1),
        vec![mat(M::Stick, 6), RecipeIngredient { item: Item::Material(M::GreenLog), count: 3, fuzzy: Some(FuzzyKind::Log) }],
        grid(&[(0, 0, STICK), (0, 1, STICK), (0, 2, STICK), (1, 0, m_at(M::GreenLog)), (1, 1, m_at(M::GreenLog)), (1, 2, m_at(M::GreenLog)), (2, 0, STICK), (2, 1, STICK), (2, 2, STICK)]),
        RecipeCategory::Stations));
}

/// Task 14 (pets-debt-water-wave, Campaign D) — recipe catalogue gap-close.
/// 13 named recipes + 5 non-oak fence-post species (the same species set
/// `push_generated_literals` already uses for plank generation — Rubber is
/// excluded there because no matcher arm turns a Rubber Log into Rubber
/// Planks, so a Rubber Fence Post card would advertise an unreachable
/// recipe; mirrored here for the same reason) were working matcher logic
/// in `crafting.rs` with no catalogue card, so the recipe book couldn't
/// show them even though they craft correctly on the real grid. Every grid
/// below is copied verbatim from the cited matcher arm; `card()` derives
/// the station from the grid's bounding box, and
/// `every_card_matches_the_live_matcher` proves each one against
/// `match_recipe`.
fn push_gap_close_cards(out: &mut Vec<RecipeCard>) {
    use MaterialId as M;
    let p = BLOCK_PLANKS;
    let cobble = CraftSlot::Block(block::COBBLESTONE);
    let iron = CraftSlot::Material(M::IronIngot);
    let blk_at = |b: block::BlockId| CraftSlot::Block(b);
    let m_at = |id: M| CraftSlot::Material(id);

    // crafting.rs:834 — Sticky Piston: Rubber atop a Piston (2×1 vertical).
    out.push(card(
        "Sticky Piston",
        ItemStack::new_block(block::STICKY_PISTON, 1),
        vec![mat(M::Rubber, 1), blk(block::PISTON, 1)],
        grid(&[(0, 0, m_at(M::Rubber)), (1, 0, blk_at(block::PISTON))]),
        RecipeCategory::Stations,
    ));

    // crafting.rs:993 — Oak Fence Gate: SPS / SPS (sticks flank a plank rail).
    out.push(card(
        "Oak Fence Gate",
        ItemStack::new_block(block::OAK_FENCE_GATE, 1),
        vec![mat(M::Stick, 4), blk(block::OAK_PLANKS, 2)],
        grid(&[
            (0, 0, STICK), (0, 1, p), (0, 2, STICK),
            (1, 0, STICK), (1, 1, p), (1, 2, STICK),
        ]),
        RecipeCategory::Building,
    ));

    // crafting.rs:998 — Oak Trapdoor: PPP / PPP (6 planks) → 2 trapdoors.
    out.push(card(
        "Oak Trapdoor",
        ItemStack::new_block(block::OAK_TRAPDOOR, 2),
        vec![blk(block::OAK_PLANKS, 6)],
        grid(&[
            (0, 0, p), (0, 1, p), (0, 2, p),
            (1, 0, p), (1, 1, p), (1, 2, p),
        ]),
        RecipeCategory::Building,
    ));

    // crafting.rs:1006 — Glass Pane: GGG / GGG (6 glass) → 16 panes.
    let glass = blk_at(block::GLASS);
    out.push(card(
        "Glass Pane",
        ItemStack::new_block(block::GLASS_PANE, 16),
        vec![blk(block::GLASS, 6)],
        grid(&[
            (0, 0, glass), (0, 1, glass), (0, 2, glass),
            (1, 0, glass), (1, 1, glass), (1, 2, glass),
        ]),
        RecipeCategory::Building,
    ));

    // crafting.rs:1013 — Iron Bars: III / III (6 iron ingots) → 16 bars.
    out.push(card(
        "Iron Bars",
        ItemStack::new_block(block::IRON_BARS, 16),
        vec![mat(M::IronIngot, 6)],
        grid(&[
            (0, 0, iron), (0, 1, iron), (0, 2, iron),
            (1, 0, iron), (1, 1, iron), (1, 2, iron),
        ]),
        RecipeCategory::Building,
    ));

    // crafting.rs:1192 — Stone Stairs: a 6-stone staircase (left-handed
    // orientation; the matcher also accepts the mirrored right-hand grid,
    // but one example grid is enough to prove the card) → 4 stairs.
    let stone = blk_at(block::STONE);
    out.push(card(
        "Stone Stairs",
        ItemStack::new_block(block::STONE_STAIRS, 4),
        vec![blk(block::STONE, 6)],
        grid(&[
            (0, 0, stone),
            (1, 0, stone), (1, 1, stone),
            (2, 0, stone), (2, 1, stone), (2, 2, stone),
        ]),
        RecipeCategory::Building,
    ));

    // crafting.rs:1225-1238 — Hopper: an iron V around a Chest centre.
    let chest = blk_at(block::CHEST);
    out.push(card(
        "Hopper",
        ItemStack::new_block(block::HOPPER, 1),
        vec![mat(M::IronIngot, 5), blk(block::CHEST, 1)],
        grid(&[
            (0, 0, iron), (0, 2, iron),
            (1, 0, iron), (1, 1, chest), (1, 2, iron),
            (2, 1, iron),
        ]),
        RecipeCategory::Stations,
    ));

    // crafting.rs:1249-1264 — Piston: plank cap, iron-cored cobble body,
    // cable tap at the base (the redstone-dust slot, our cable convention).
    let cable = blk_at(block::CABLE);
    out.push(card(
        "Piston",
        ItemStack::new_block(block::PISTON, 1),
        vec![blk(block::OAK_PLANKS, 3), blk(block::COBBLESTONE, 4), mat(M::IronIngot, 1), blk(block::CABLE, 1)],
        grid(&[
            (0, 0, p), (0, 1, p), (0, 2, p),
            (1, 0, cobble), (1, 1, iron), (1, 2, cobble),
            (2, 0, cobble), (2, 1, cable), (2, 2, cobble),
        ]),
        RecipeCategory::Stations,
    ));

    // crafting.rs:1851-1858 — Oak Door: a 3-tall × 2-wide column of planks
    // → 3 doors.
    out.push(card(
        "Oak Door",
        ItemStack::new_block(block::OAK_DOOR, 3),
        vec![blk(block::OAK_PLANKS, 6)],
        grid(&[
            (0, 0, p), (0, 1, p),
            (1, 0, p), (1, 1, p),
            (2, 0, p), (2, 1, p),
        ]),
        RecipeCategory::Building,
    ));

    // crafting.rs:1864-1875 — Oak Sign: 6 planks over a centred stick foot
    // → 3 signs.
    out.push(card(
        "Oak Sign",
        ItemStack::new_block(block::OAK_SIGN, 3),
        vec![blk(block::OAK_PLANKS, 6), mat(M::Stick, 1)],
        grid(&[
            (0, 0, p), (0, 1, p), (0, 2, p),
            (1, 0, p), (1, 1, p), (1, 2, p),
            (2, 1, STICK),
        ]),
        RecipeCategory::Decoration,
    ));

    // crafting.rs:1878-1891 — Item Frame: an 8-stick ring around a leather
    // centre → 1 frame.
    let leather = m_at(M::Leather);
    out.push(card(
        "Item Frame",
        ItemStack::new_block(block::ITEM_FRAME, 1),
        vec![mat(M::Stick, 8), mat(M::Leather, 1)],
        grid(&[
            (0, 0, STICK), (0, 1, STICK), (0, 2, STICK),
            (1, 0, STICK), (1, 1, leather), (1, 2, STICK),
            (2, 0, STICK), (2, 1, STICK), (2, 2, STICK),
        ]),
        RecipeCategory::Decoration,
    ));

    // crafting.rs:1897-1903 — Cobblestone Wall: a 3×2 block of cobblestone
    // → 6 walls.
    out.push(card(
        "Cobblestone Wall",
        ItemStack::new_block(block::COBBLESTONE_WALL, 6),
        vec![blk(block::COBBLESTONE, 6)],
        grid(&[
            (0, 0, cobble), (0, 1, cobble), (0, 2, cobble),
            (1, 0, cobble), (1, 1, cobble), (1, 2, cobble),
        ]),
        RecipeCategory::Building,
    ));

    // crafting.rs:1915-1918 — Stone Slab: 3 stone horizontal → 6 slabs.
    out.push(card(
        "Stone Slab",
        ItemStack::new_block(block::STONE_SLAB, 6),
        vec![blk(block::STONE, 3)],
        grid(&[(0, 0, stone), (0, 1, stone), (0, 2, stone)]),
        RecipeCategory::Building,
    ));

    // crafting.rs:948-957 (species_for_planks) + block.rs fence_post_for_species
    // — per-species Fence Post, same PSP/PSP shape as the Oak card above
    // (crafting_catalogue.rs "Fence Post"), for every non-oak species that
    // actually has a plank-crafting path (mirrors push_generated_literals'
    // species set exactly — Rubber has no Log → Planks matcher arm, so it's
    // excluded here too).
    let fence_species: [(block::BlockId, block::BlockId, &str); 5] = [
        (block::BIRCH_PLANKS, block::BIRCH_FENCE_POST, "Birch"),
        (block::SPRUCE_PLANKS, block::SPRUCE_FENCE_POST, "Spruce"),
        (block::JUNGLE_PLANKS, block::JUNGLE_FENCE_POST, "Jungle"),
        (block::ACACIA_PLANKS, block::ACACIA_FENCE_POST, "Acacia"),
        (block::DARK_OAK_PLANKS, block::DARK_OAK_FENCE_POST, "Dark Oak"),
    ];
    for (plank, post, name) in fence_species {
        let pl = blk_at(plank);
        out.push(card(
            &format!("{name} Fence Post"),
            ItemStack::new_block(post, 3),
            vec![blk(plank, 4), mat(M::Stick, 2)],
            grid(&[
                (0, 0, pl), (0, 1, STICK), (0, 2, pl),
                (1, 0, pl), (1, 1, STICK), (1, 2, pl),
            ]),
            RecipeCategory::Building,
        ));
    }
}

// ── Catalogue assembly + query API ──────────────────────────────────────────

static CATALOGUE: LazyLock<Vec<RecipeCard>> = LazyLock::new(|| {
    let mut out = Vec::new();
    push_tool_cards(&mut out);
    push_bow_cards(&mut out);
    push_armour_cards(&mut out);
    push_dye_decor_cards(&mut out);
    push_generated_literals(&mut out);
    push_literal_cards(&mut out);
    push_gap_close_cards(&mut out);
    out
});

/// Every recipe card, built once.
pub fn all_cards() -> &'static [RecipeCard] {
    &CATALOGUE
}

/// Cards in a category, in catalogue order.
#[cfg_attr(not(test), allow(dead_code))]
pub fn cards_in(category: RecipeCategory) -> impl Iterator<Item = &'static RecipeCard> {
    CATALOGUE.iter().filter(move |c| c.category == category)
}

/// Case-insensitive name substring search.
#[cfg_attr(not(test), allow(dead_code))]
pub fn search(query: &str) -> Vec<&'static RecipeCard> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    CATALOGUE.iter().filter(|c| c.name.to_lowercase().contains(&q)).collect()
}

/// Does an inventory item satisfy this ingredient (honouring fuzzy sets)?
fn item_satisfies(inv_item: &Item, ing: &RecipeIngredient) -> bool {
    match ing.fuzzy {
        Some(FuzzyKind::Log) => crate::crafting::is_logish_slot(CraftSlot::from_item(inv_item)),
        Some(FuzzyKind::Plank) => {
            matches!(inv_item, Item::Block(b) if block::is_any_planks(*b))
        }
        Some(FuzzyKind::Paper) => {
            crate::crafting::is_paperish_slot(CraftSlot::from_item(inv_item))
        }
        None => match (inv_item, &ing.item) {
            (Item::Block(a), Item::Block(b)) => a == b,
            (Item::Material(a), Item::Material(b)) => a == b,
            _ => false,
        },
    }
}

/// Total count of items in `inv` that satisfy `ing`.
fn available(inv: &Inventory, ing: &RecipeIngredient) -> u32 {
    inv.slots_iter()
        .flatten()
        .filter(|st| item_satisfies(&st.item, ing))
        .map(|st| st.count as u32)
        .sum()
}

/// Cards the player can afford right now, from their inventory. Honours
/// fuzzy ingredient sets (any-log, any-plank, any-paper). Ordering follows
/// the catalogue.
///
/// Approximation (documented): each ingredient is checked independently, so
/// a recipe whose two ingredients would draw from the *same* shared stock
/// could read as affordable when it isn't. No current recipe has two
/// ingredients satisfied by one item, so this is exact in practice; revisit
/// if a same-item-twice recipe is ever added.
#[cfg_attr(not(test), allow(dead_code))]
pub fn craftable_now(inv: &Inventory) -> Vec<&'static RecipeCard> {
    CATALOGUE.iter().filter(|card| can_craft(inv, card)).collect()
}

/// Can the player afford this one card right now? Honours fuzzy sets.
pub fn can_craft(inv: &Inventory, card: &RecipeCard) -> bool {
    card.ingredients.iter().all(|ing| available(inv, ing) >= ing.count as u32)
}

/// Global catalogue index of the FIRST card that produces `item` (same block
/// or material id). Used by the placement-guide drill-down: clicking an
/// ingredient that is itself craftable jumps to its recipe. `None` for raw
/// inputs with no crafting recipe (cobblestone, iron ingot, wool, …).
pub fn recipe_index_for_output(item: &Item) -> Option<usize> {
    CATALOGUE.iter().position(|c| match (&c.output.item, item) {
        (Item::Block(a), Item::Block(b)) => a == b,
        (Item::Material(a), Item::Material(b)) => a == b,
        // Tools: match on type + material (ignore per-instance durability). Lets
        // the Trials recipe-on-right find pickaxe/sword/etc. recipes.
        (Item::Tool(a), Item::Tool(b)) => {
            a.tool_type == b.tool_type && a.material == b.material
        }
        _ => false,
    })
}

/// #46 — reverse "uses" index: item → catalogue indices of every recipe that
/// **consumes** it (JEI's "U / show uses"). Built once from each card's
/// `ingredients`, keyed by [`Item::sort_key`] (a total, hashable key that's
/// 1:1 for the Block/Material variants ingredients are ever made of). The
/// forward `recipe_index_for_output` stays the only output→recipe source — this
/// derives from the same `RecipeCard`s, introducing no second source of truth.
static USES_INDEX: LazyLock<std::collections::HashMap<(u8, u32), Vec<usize>>> = LazyLock::new(|| {
    let mut map: std::collections::HashMap<(u8, u32), Vec<usize>> = std::collections::HashMap::new();
    for (i, card) in CATALOGUE.iter().enumerate() {
        let mut seen: std::collections::HashSet<(u8, u32)> = std::collections::HashSet::new();
        for ing in &card.ingredients {
            let key = ing.item.sort_key();
            // De-dupe within a card so a recipe listing an item twice still
            // appears once in its uses set.
            if seen.insert(key) {
                map.entry(key).or_default().push(i);
            }
        }
    }
    map
});

/// #46 — catalogue indices of every recipe that consumes `item`, in ascending
/// (display) order. Empty when the item is used in nothing.
pub fn recipes_using(item: &Item) -> Vec<usize> {
    USES_INDEX
        .get(&item.sort_key())
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::match_recipe;

    /// THE safety net. Every card's example grid, fed to the live matcher,
    /// must produce exactly the card's stated output. A wrong/stale card
    /// fails here rather than mis-crafting in game.
    #[test]
    fn every_card_matches_the_live_matcher() {
        // ItemStack/Item/Tool don't derive PartialEq (and adding it would
        // ripple through Plan + the whole item graph), so compare via Debug
        // — it captures item kind, material, count, and tool durability,
        // which is exactly the identity we need here.
        let mut failures = Vec::new();
        for c in all_cards() {
            let got = match_recipe(&c.example_grid);
            let want = Some(c.output.clone());
            if format!("{:?}", got) != format!("{:?}", want) {
                failures.push(format!(
                    "card '{}': expected {:?}, matcher gave {:?}",
                    c.name, want, got
                ));
            }
        }
        assert!(failures.is_empty(), "{} card(s) mismatch:\n{}", failures.len(), failures.join("\n"));
    }

    #[test]
    fn recipes_using_matches_a_brute_force_scan() {
        // #46 — the reverse "uses" index must return EXACTLY the cards whose
        // ingredient list contains the item, matching a brute-force scan. Sticks
        // are an ingredient in many recipes, so the set is non-trivial.
        let item = Item::Material(crate::item::MaterialId::Stick);
        let mut brute: Vec<usize> = all_cards()
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.ingredients.iter().any(|ing| match (&ing.item, &item) {
                    (Item::Block(a), Item::Block(b)) => a == b,
                    (Item::Material(a), Item::Material(b)) => a == b,
                    _ => false,
                })
            })
            .map(|(i, _)| i)
            .collect();
        let mut got = recipes_using(&item);
        brute.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, brute, "reverse index != brute-force scan");
        assert!(!got.is_empty(), "sticks should be used somewhere");
    }

    #[test]
    fn recipes_using_unused_item_is_empty() {
        // An item that never appears as an ingredient yields no uses.
        let got = recipes_using(&Item::Material(crate::item::MaterialId::Bone));
        let brute = all_cards().iter().any(|c| {
            c.ingredients.iter().any(|ing| matches!(&ing.item,
                Item::Material(m) if *m == crate::item::MaterialId::Bone))
        });
        assert_eq!(got.is_empty(), !brute, "empty iff brute-force finds none");
    }

    #[test]
    fn catalogue_is_non_trivial() {
        // Floor assertion (spec §4 known-gap mitigation): parametric families
        // (25 tools + 5 bows + 17 armour + 80 dye = 127) + generated literals
        // (species/storage/food/dye-mix/wallpaper) + one-off literals now put
        // the catalogue well over 200. If this drops, a family stopped
        // generating.
        // Exact count locks the catalogue size (295 as of 2026-09-07: 294 prior
        // + 1, the Windmill card, Wind/Copper/Electricity wave §2.2. The 294
        // was 293 + the Water Wheel card, Spec 48 Phase 4; the 293 was 292 +
        // the Plunger Detonator card added when the 2026-07-09 wiki audit
        // caught its spec'd recipe never having been wired into match_recipe).
        // Bump deliberately when adding recipes — a silent drop means a family
        // stopped generating.
        assert_eq!(all_cards().len(), 295, "catalogue size changed to {}", all_cards().len());
    }

    /// UX polish sweep Task 3 — the non-obvious craftables (a kid can't
    /// guess what they DO from the name + ingredient grid alone) must carry
    /// a `usage` line. Buckets aren't in this list: they have no catalogue
    /// card (fill/pour is a right-click interaction, not a crafting recipe
    /// — see `bucket.rs`), so there's nothing here to annotate.
    #[test]
    fn non_obvious_items_carry_a_usage_line() {
        for name in [
            "Reach Claw",
            "Cat Treat",
            "Recall Whistle",
            "Salt Lick",
            "Bone Meal",
            "Lead",
            "Shears",
        ] {
            let card = all_cards().iter().find(|c| c.name == name)
                .unwrap_or_else(|| panic!("expected a '{name}' card in the catalogue"));
            assert!(
                card.usage.is_some_and(|u| !u.trim().is_empty()),
                "'{name}' should carry a non-empty usage line"
            );
        }
    }

    #[test]
    fn all_five_tiers_of_pickaxe_are_present() {
        for tier in ["Wooden", "Stone", "Iron", "Diamond", "Satori"] {
            let name = format!("{} Pickaxe", tier);
            assert!(all_cards().iter().any(|c| c.name == name), "missing {name}");
        }
    }

    #[test]
    fn search_is_case_insensitive_and_substring() {
        assert!(search("pickaxe").iter().any(|c| c.name == "Iron Pickaxe"));
        assert!(search("IRON").iter().any(|c| c.name == "Iron Pickaxe"));
        assert!(search("").is_empty());
    }

    #[test]
    fn cards_in_category_are_filtered() {
        assert!(cards_in(RecipeCategory::Combat).all(|c| c.category == RecipeCategory::Combat));
        assert!(cards_in(RecipeCategory::Tools).any(|c| c.name == "Iron Pickaxe"));
    }

    #[test]
    fn all_sixteen_dyes_get_a_banner_card() {
        for (_, colour) in DYES {
            let name = format!("{} Banner", colour);
            assert!(all_cards().iter().any(|c| c.name == name), "missing {name}");
        }
    }

    #[test]
    fn craftable_now_respects_inventory_and_fuzzy_logs() {
        let mut inv = Inventory::new();
        // Empty inventory crafts nothing.
        assert!(craftable_now(&inv).is_empty());

        // An oak log satisfies the fuzzy "Any log" ingredient of Oak Planks.
        inv.set_slot(0, Some(ItemStack::new_block(block::OAK_LOG, 1)));
        let names: Vec<_> = craftable_now(&inv).iter().map(|c| c.name.clone()).collect();
        assert!(names.contains(&"Oak Planks".to_string()), "log should afford planks: {names:?}");

        // Two planks afford Sticks (count threshold honoured: one plank doesn't).
        let mut inv1 = Inventory::new();
        inv1.set_slot(0, Some(ItemStack::new_block(block::OAK_PLANKS, 1)));
        assert!(!craftable_now(&inv1).iter().any(|c| c.name == "Sticks"), "1 plank can't make sticks");
        inv1.set_slot(0, Some(ItemStack::new_block(block::OAK_PLANKS, 2)));
        assert!(craftable_now(&inv1).iter().any(|c| c.name == "Sticks"), "2 planks make sticks");
    }

    #[test]
    fn dye_decor_families_are_complete() {
        // 5 families × 16 dyes = 80 dye cards.
        let dye_cards = all_cards()
            .iter()
            .filter(|c| {
                ["Banner", "Sail", "Bunting", "Paper Lantern", "Kite"]
                    .iter()
                    .any(|fam| c.name.ends_with(fam))
            })
            .count();
        assert_eq!(dye_cards, 80, "expected 5×16 dye-décor cards, got {dye_cards}");
    }

    #[test]
    fn recipe_index_for_output_finds_craftable_ingredients() {
        // Sticks and oak planks are craftable → drill-down targets exist.
        let stick = recipe_index_for_output(&Item::Material(MaterialId::Stick));
        assert!(stick.is_some(), "Stick should have a recipe");
        assert_eq!(all_cards()[stick.unwrap()].name, "Sticks");

        let planks = recipe_index_for_output(&Item::Block(block::OAK_PLANKS));
        assert!(planks.is_some(), "Oak planks should have a recipe");

        // Cobblestone is mined, not crafted → no drill target.
        assert!(
            recipe_index_for_output(&Item::Block(block::COBBLESTONE)).is_none(),
            "cobblestone has no crafting recipe"
        );
    }
}
