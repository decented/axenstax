//! Spec 28f — Inventory Explorer with Search.
//!
//! Full-screen pane separate from the hotbar + crafting inventory that
//! enumerates every block, tool, and material registered in the engine.
//! Filter by text (case-insensitive substring on the display name) +
//! by category (All / Blocks / Tools / Materials). In creative, click an
//! entry to drop one onto the hotbar.
//!
//! As Spec 28a-e content sub-foundations land, the explorer picks the
//! new content up automatically because it enumerates the registry +
//! the MaterialId / ToolType / ToolMaterial enums at runtime.
//!
//! Opens via the **B key**. Esc closes. Cursor releases while open;
//! gameplay input (movement / break / place) is suppressed.

use crate::block::{BlockId, BlockRegistry, AIR};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack, MaterialId};

/// Top-level category for the radio filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerCategory {
    All,
    Blocks,
    Tools,
    Materials,
}

impl ExplorerCategory {
    pub fn label(self) -> &'static str {
        match self {
            ExplorerCategory::All => "All",
            ExplorerCategory::Blocks => "Blocks",
            ExplorerCategory::Tools => "Tools",
            ExplorerCategory::Materials => "Materials",
        }
    }
}

/// One row in the explorer.
#[derive(Clone, Debug)]
pub struct ExplorerEntry {
    /// Pre-computed UK-English display label (matches what `Item::name`
    /// produces against the live registry).
    pub label: String,
    /// Derived from `item` variant — cached so the filter doesn't have
    /// to re-match on every keystroke.
    pub category: ExplorerCategory,
    /// The concrete item the explorer will dispense in creative mode.
    pub item: Item,
}

/// Per-player UI state. Lives on `PlayerSlot` while the explorer is
/// open; cleared on close so the next open starts fresh.
#[derive(Clone, Debug, Default)]
pub struct ExplorerState {
    pub query: String,
    pub category: Option<ExplorerCategory>,
    /// Set on the frame the explorer opens so the search field can grab
    /// focus exactly once (egui memorises focus across frames otherwise).
    pub focus_search_this_frame: bool,
}

impl ExplorerState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            category: None,
            focus_search_this_frame: true,
        }
    }
}

/// What the egui draw call wants the caller to do this frame.
#[derive(Clone, Debug)]
pub enum ExplorerOutcome {
    /// Dialog still rendering, no terminal action.
    InProgress,
    /// User pressed Close / Esc. Caller clears `PlayerSlot.explorer_state`.
    Closed,
    /// User clicked an entry in creative mode. Caller pushes a stack
    /// of this item into the player's inventory.
    GiveToInventory(ItemStack),
}

// ============================================================================
// §2 — Enumeration
// ============================================================================

/// Every `MaterialId` variant in alpha order. Locked by a test against
/// a manual count — when a new variant is added to `MaterialId`, append
/// here AND bump the test's expected count, otherwise the test fails
/// loudly. There's no derive(IntoEnumIterator) on the project because
/// we'd rather pull strum than add a proc-macro dep for one enum.
pub const ALL_MATERIAL_IDS: &[MaterialId] = &[
    MaterialId::Stick,
    MaterialId::Leather,
    MaterialId::Feather,
    MaterialId::Wool,
    MaterialId::Bone,
    MaterialId::RawBeef,
    MaterialId::RawPorkchop,
    MaterialId::RawChicken,
    MaterialId::RawMutton,
    MaterialId::String,
    MaterialId::Bonemeal,
    MaterialId::Coal,
    MaterialId::RawIron,
    MaterialId::Diamond,
    MaterialId::IronIngot,
    MaterialId::CookedBeef,
    MaterialId::CookedPorkchop,
    MaterialId::CookedChicken,
    MaterialId::CookedMutton,
    MaterialId::Arrow,
    MaterialId::Satori,
    MaterialId::WheatSeeds,
    MaterialId::Wheat,
    MaterialId::Bread,
    MaterialId::Carrot,
    MaterialId::Potato,
    MaterialId::Flint,
    MaterialId::CornSeeds,
    MaterialId::Corn,
    MaterialId::BakedCorn,
    MaterialId::BakedPotato,
    MaterialId::BakedCarrot,
    MaterialId::GreenLog,
    MaterialId::SeasonedLog,
    MaterialId::KilnDriedLog,
    MaterialId::PapyrusReed,
    MaterialId::PapyrusSheet,
    // Spec 28c Materials Expansion (2026-05-20).
    MaterialId::Copper,
    MaterialId::Tin,
    MaterialId::Sulphur,
    MaterialId::Amethyst,
    MaterialId::CopperIngot,
    MaterialId::TinIngot,
    MaterialId::BronzeIngot,
    MaterialId::Sugar,
    // Spec T1.5 Processed Economy Base (data layer 2026-05-20).
    MaterialId::Bucket,
    MaterialId::MilkBucket,
    MaterialId::WaterBucket,
    MaterialId::LavaBucket,
    MaterialId::Egg,
    MaterialId::Flour,
    MaterialId::Dough,
    MaterialId::Cream,
    MaterialId::Butter,
    MaterialId::Cheese,
    MaterialId::SweetBread,
    MaterialId::Cake,
    MaterialId::PumpkinPie,
    MaterialId::BerryPie,
    MaterialId::Cookie,
    MaterialId::Pancakes,
    MaterialId::LoadedBakedPotato,
    MaterialId::Stew,
    MaterialId::BeetrootSoup,
    MaterialId::Bowl,
    MaterialId::PumpkinFood,
    MaterialId::SugarBeet,
    MaterialId::SugarBeetSeeds,
    MaterialId::Beetroot,
    MaterialId::BeetrootSeeds,
    MaterialId::Berries,
    // Spec 28b Wood Species saplings.
    MaterialId::OakSapling,
    MaterialId::BirchSapling,
    MaterialId::SpruceSapling,
    MaterialId::JungleSapling,
    MaterialId::AcaciaSapling,
    MaterialId::DarkOakSapling,
    // Spec 28c Phase 5 mob-drop materials (no live consumers yet).
    MaterialId::Honeycomb,
    MaterialId::Honey,
    MaterialId::InkSac,
    MaterialId::GlowBerry,
    // Spec 28d.rabbit + chunk 5 (Bee). Drops wired from `mob::drops_for`;
    // furnace cooks RawRabbit → CookedRabbit.
    MaterialId::RawRabbit,
    MaterialId::CookedRabbit,
    MaterialId::RabbitHide,
    MaterialId::HoneyBottle,
    MaterialId::BeeStinger,
    // Spec 28d.nostrich — purple-ostrich mob drops + premium recipe
    // outputs. Trade-value tier ladder lives on each Item::name arm.
    MaterialId::NostrichFeather,
    MaterialId::NostrichEgg,
    MaterialId::RawNostrichMeat,
    MaterialId::RoyalPavlova,
    MaterialId::NostrichOmelette,
    MaterialId::NostrichCustard,
    MaterialId::NostrichArrow,
    MaterialId::PurpleBanner,
    // Spec HP-1 (2026-05-22). Brigand Chieftain Trophy reserved for
    // Sub 3's Brigand Hideout drop; the trophy slot post-cutover.
    MaterialId::BrigandChieftainTrophy,
    // Salt feature (2026-05-23) — mined material + cured/seasoned ladder.
    MaterialId::Salt,
    MaterialId::SaltCuredBeef,
    MaterialId::SaltCuredPorkchop,
    MaterialId::SaltCuredMutton,
    MaterialId::SaltCuredChicken,
    MaterialId::SaltCuredRabbit,
    MaterialId::SaltCuredNostrichMeat,
    MaterialId::SeasonedBread,
    MaterialId::SeasonedBakedPotato,
    MaterialId::SeasonedBakedCarrot,
    MaterialId::SeasonedBakedCorn,
    MaterialId::SeasonedCookedBeef,
    MaterialId::SeasonedCookedPorkchop,
    MaterialId::SeasonedCookedMutton,
    MaterialId::SeasonedCookedChicken,
    MaterialId::SeasonedCookedRabbit,
    // Rubber feature (2026-05-23).
    MaterialId::Rubber,
    MaterialId::RubberSapling,
    MaterialId::RubberBall,
    MaterialId::CopperCable,
    // Economy-block placeable items (Specs 33-39).
    MaterialId::BountyBoardItem,
    MaterialId::TipJarItem,
    MaterialId::RepairBenchItem,
    MaterialId::PlotMarkerItem,
    MaterialId::MarketBellItem,
    MaterialId::AuctionBlockItem,
    MaterialId::BazaarBlockItem,
    // Dyes (Spec 35) + fibre/cordage (Spec 36), 2026-05-27.
    MaterialId::BlueDye,
    MaterialId::RedDye,
    MaterialId::YellowDye,
    MaterialId::Cotton,
    MaterialId::HempFibre,
    MaterialId::Rope,
    // Magnesium (Spec 37), 2026-05-27.
    MaterialId::Magnesium,
    MaterialId::Fertiliser,
    MaterialId::Sparkler,
    MaterialId::Flare,
    MaterialId::MagnesiumFirestarter,
    // Dye Phase 2 (Spec 35), 2026-05-27.
    MaterialId::BlackDye,
    MaterialId::WhiteDye,
    MaterialId::OrangeDye,
    MaterialId::GreenDye,
    MaterialId::PurpleDye,
    MaterialId::PinkDye,
    MaterialId::LimeDye,
    MaterialId::LightBlueDye,
    MaterialId::GreyDye,
    MaterialId::LightGreyDye,
    // Fibre Phase 2 seeds (Spec 36), 2026-05-27.
    MaterialId::CottonSeeds,
    MaterialId::HempSeeds,
    // Dye Phase 2 completion (Spec 35, 2026-05-28) — the three 3-input mixes.
    MaterialId::BrownDye,
    MaterialId::CyanDye,
    MaterialId::MagentaDye,
    // Spec 36 Phase 2 (2026-05-28) — Rope's consumer + the textile/sailcloth pair.
    MaterialId::Lead,
    MaterialId::Cloth,
    MaterialId::Canvas,
    // Spec 35 farmable-flower seeds (2026-05-28).
    MaterialId::CornflowerSeeds,
    MaterialId::FieldPoppySeeds,
    MaterialId::ButtercupSeeds,
    // Spec 40 (The Workshop) — the Bellows authoring tool. MUST stay listed:
    // without it the Bellows is unobtainable from the creative inventory once
    // removed from the hotbar (2026-06-18 regression fix).
    MaterialId::Bellows,
    // Craftable Armoured Carts (CA2) — the three cart-vehicle item tiers.
    MaterialId::WoodCart,
    MaterialId::IronCart,
    MaterialId::DiamondCart,
    // Spec 49 (Explosives).
    MaterialId::Saltpetre,
    MaterialId::BlackPowder,
    MaterialId::Compost,
];

/// (ToolType, ToolMaterial) pairs that are actually valid in the engine.
/// Pickaxe/Axe/Sword/Shovel/Hoe span all five tiers. Bow is wood-only by
/// design (Wave 23 — per-tier bows are reserved). FlintAndSteel is
/// single-tier Iron by recipe (Wave 27).
pub fn all_tool_combos() -> Vec<(ToolType, ToolMaterial)> {
    let tiered = [
        ToolType::Pickaxe,
        ToolType::Axe,
        ToolType::Sword,
        ToolType::Shovel,
        ToolType::Hoe,
    ];
    let materials = [
        ToolMaterial::Wood,
        ToolMaterial::Stone,
        ToolMaterial::Iron,
        ToolMaterial::Diamond,
        ToolMaterial::Satori,
    ];
    let mut out = Vec::with_capacity(tiered.len() * materials.len() + 2);
    for tt in tiered {
        for &mat in &materials {
            out.push((tt, mat));
        }
    }
    out.push((ToolType::Bow, ToolMaterial::Wood));
    out.push((ToolType::FlintAndSteel, ToolMaterial::Iron));
    // Spec 28e — single-tier utility tools. Shears uses Iron material
    // by convention (durability lives in SHEARS_DURABILITY constant);
    // Fishing Rod uses Wood (recipe is sticks + string).
    out.push((ToolType::Shears, ToolMaterial::Iron));
    out.push((ToolType::FishingRod, ToolMaterial::Wood));
    // Single Wood-tier utility tools (Rubber + Blueprint). These were absent
    // from the explorer, so — like the Bellows — they were unobtainable from
    // the creative inventory once removed from the hotbar (2026-06-18 fix).
    out.push((ToolType::Slingshot, ToolMaterial::Wood));
    out.push((ToolType::Eraser, ToolMaterial::Wood));
    out.push((ToolType::DraftingStamp, ToolMaterial::Wood));
    out
}

/// Enumerate every item that the explorer can show: every block (minus
/// AIR), every MaterialId, every valid (ToolType, ToolMaterial) combo.
/// Pure — no UI, no IO. Result ordered Blocks → Materials → Tools so the
/// list reads sensibly when the All category is selected.
pub fn enumerate_all_items(registry: &BlockRegistry) -> Vec<ExplorerEntry> {
    let mut entries: Vec<ExplorerEntry> = Vec::new();

    // Blocks. Iterate by id rather than slice so a future sparse registry
    // (skipped ids) still produces the right block id in the entry.
    for id in 0u16..registry.len() as u16 {
        if id == AIR {
            continue;
        }
        let item = Item::Block(id as BlockId);
        let label = item.name(registry);
        entries.push(ExplorerEntry {
            label,
            category: ExplorerCategory::Blocks,
            item,
        });
    }

    // Materials.
    for &mid in ALL_MATERIAL_IDS {
        let item = Item::Material(mid);
        let label = item.name(registry);
        entries.push(ExplorerEntry {
            label,
            category: ExplorerCategory::Materials,
            item,
        });
    }

    // Tools — Tool::new dispenses a fully-durable instance, which is what
    // an in-explorer "give" should hand out.
    for (tt, mat) in all_tool_combos() {
        let tool = Tool::new(tt, mat);
        let item = Item::Tool(tool);
        let label = item.name(registry);
        entries.push(ExplorerEntry {
            label,
            category: ExplorerCategory::Tools,
            item,
        });
    }

    // Spec 28e — armour pieces. ArmourItem::new dispenses a fully-
    // durable instance. Categorised as Tools because they live in
    // the same broad "equipment" surface and the explorer only has
    // four category buckets on alpha.
    for (slot, mat) in crate::armour::all_armour_combos() {
        let item = Item::Armour(crate::armour::ArmourItem::new(slot, mat));
        let label = item.name(registry);
        entries.push(ExplorerEntry {
            label,
            category: ExplorerCategory::Tools,
            item,
        });
    }

    entries
}

// ============================================================================
// §3 — Filter
// ============================================================================

/// Apply search + category filter. Pure; `query` is matched
/// case-insensitively as a substring of the display label. `category =
/// None` and `category = Some(All)` both mean "no category filter".
pub fn filter_entries<'a>(
    entries: &'a [ExplorerEntry],
    query: &str,
    category: Option<ExplorerCategory>,
) -> Vec<&'a ExplorerEntry> {
    let needle = query.trim().to_lowercase();
    entries
        .iter()
        .filter(|e| match category {
            None | Some(ExplorerCategory::All) => true,
            Some(c) => e.category == c,
        })
        .filter(|e| needle.is_empty() || e.label.to_lowercase().contains(&needle))
        .collect()
}

// ============================================================================
// §4 — egui draw
// ============================================================================

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 195, 100);
const SECTION_COLOR: egui::Color32 = egui::Color32::LIGHT_GRAY;

/// Render the explorer modal and return the outcome. `is_creative`
/// controls whether the entry buttons are clickable (creative spawns
/// one of that item; survival shows the count as a tooltip line).
pub fn draw_inventory_explorer(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    is_creative: bool,
    inventory: &crate::inventory::Inventory,
    _registry: &BlockRegistry,
    state: &mut ExplorerState,
    entries: &[ExplorerEntry],
) -> ExplorerOutcome {
    let mut outcome = ExplorerOutcome::InProgress;

    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("explorer_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    // Esc is handled centrally in `main.rs` (EscAction::CloseExplorer) so it
    // closes this overlay FIRST, leaving an open inventory behind it intact —
    // one panel per Esc. Handling it here too would close both at once.

    let panel_w = 560.0;
    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + 60.0,
    );

    egui::Area::new(egui::Id::new(("explorer_panel", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);

            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Inventory Explorer")
                            .size(22.0)
                            .color(TITLE_COLOR)
                            .strong(),
                    );
                });
                ui.add_space(6.0);

                // Search row.
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Search:").color(SECTION_COLOR));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut state.query)
                            .desired_width(panel_w - 110.0)
                            .hint_text("type to filter"),
                    );
                    if state.focus_search_this_frame {
                        resp.request_focus();
                        state.focus_search_this_frame = false;
                    }
                    if ui.small_button("Clear").clicked() {
                        state.query.clear();
                    }
                });

                ui.add_space(4.0);

                // Category radio row.
                ui.horizontal(|ui| {
                    for cat in [
                        ExplorerCategory::All,
                        ExplorerCategory::Blocks,
                        ExplorerCategory::Tools,
                        ExplorerCategory::Materials,
                    ] {
                        let is_active = match (state.category, cat) {
                            (None, ExplorerCategory::All) => true,
                            (Some(c), other) => c == other,
                            _ => false,
                        };
                        if ui
                            .selectable_label(is_active, cat.label())
                            .clicked()
                        {
                            state.category = if cat == ExplorerCategory::All {
                                None
                            } else {
                                Some(cat)
                            };
                        }
                    }
                });

                ui.separator();

                // Filtered list.
                let filtered = filter_entries(entries, &state.query, state.category);
                ui.label(
                    egui::RichText::new(format!(
                        "{} of {} entries",
                        filtered.len(),
                        entries.len()
                    ))
                    .color(SECTION_COLOR),
                );
                ui.add_space(2.0);

                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for entry in &filtered {
                            let count = if !is_creative {
                                Some(count_in_inventory(inventory, &entry.item))
                            } else {
                                None
                            };
                            let label = match count {
                                Some(n) if n > 0 => format!("{}  ×{}", entry.label, n),
                                _ => entry.label.clone(),
                            };
                            let cat_tag = match entry.category {
                                ExplorerCategory::Blocks => "[B]",
                                ExplorerCategory::Tools => "[T]",
                                ExplorerCategory::Materials => "[M]",
                                ExplorerCategory::All => "[?]",
                            };
                            let row = format!("{cat_tag} {label}");
                            // Creative: clickable button that gives one of the item.
                            // Survival: read-only label (count already shown inline).
                            if is_creative {
                                let resp = ui.button(row);
                                if resp.clicked() {
                                    outcome = ExplorerOutcome::GiveToInventory(stack_for(
                                        &entry.item,
                                    ));
                                }
                            } else {
                                ui.label(row);
                            }
                        }
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Close (Esc)").clicked() {
                        outcome = ExplorerOutcome::Closed;
                    }
                    ui.label(
                        egui::RichText::new(if is_creative {
                            "Creative: click an entry to add 1 to your inventory."
                        } else {
                            "Survival: ×N is what you currently hold."
                        })
                        .color(SECTION_COLOR)
                        .small(),
                    );
                });
            });
        });

    outcome
}

/// Default "one of" stack for a given item — blocks/materials get a
/// stack of one (creative players can fan it out with shift-click later),
/// tools always come as a single durable instance.
fn stack_for(item: &Item) -> ItemStack {
    match item {
        Item::Block(id) => ItemStack::new_block(*id, 1),
        Item::Material(m) => ItemStack::new_material(*m, 1),
        Item::Tool(t) => ItemStack::new_tool(*t),
        Item::Plan(_) => ItemStack::empty(),
        Item::Armour(a) => ItemStack::new_armour(a.slot, a.material),
    }
}

/// Count how many of the given item the player has across the inventory.
/// Used by the survival "×N" overlay. Tools count by (ToolType,
/// ToolMaterial) regardless of remaining durability — what we're saying
/// is "you have a wooden pickaxe", not "you have a fully-durable wooden
/// pickaxe".
fn count_in_inventory(inventory: &crate::inventory::Inventory, target: &Item) -> u32 {
    let mut n = 0u32;
    for slot in inventory.slots_iter() {
        let Some(stack) = slot else { continue };
        let hit = match (&stack.item, target) {
            (Item::Block(a), Item::Block(b)) => a == b,
            (Item::Material(a), Item::Material(b)) => a == b,
            (Item::Tool(a), Item::Tool(b)) => {
                a.tool_type == b.tool_type && a.material == b.material
            }
            (Item::Armour(a), Item::Armour(b)) => {
                a.slot == b.slot && a.material == b.material
            }
            _ => false,
        };
        if hit {
            n += stack.count as u32;
        }
    }
    n
}

// ============================================================================
// §7 — Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_id_table_matches_enum_size() {
        // If you add a MaterialId variant, append it to ALL_MATERIAL_IDS
        // and bump this count. This is the explorer's "you forgot to
        // register a new material" alarm. Current count: 120 variants
        // (Stick through BazaarBlockItem). Phase D (2026-05-24) removed the
        // 5 retired fantasy drop items and brought the table back in sync
        // with the full enum — the list had drifted stale (was 98 against a 125-variant
        // enum), so the Salt/Seasoned/Rubber/economy-block materials are
        // now enumerated too. 2026-06-18: +13 — the variants declared after
        // HempSeeds (Brown/Cyan/Magenta dyes, Lead/Cloth/Canvas, the three
        // flower seeds, the Bellows, and the three carts) had drifted off.
        // 2026-06-22: +2 — WaterBucket + LavaBucket (Buckets MC-parity).
        assert_eq!(ALL_MATERIAL_IDS.len(), 161);
    }

    #[test]
    fn enumerate_includes_all_three_kinds() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        assert!(entries
            .iter()
            .any(|e| e.category == ExplorerCategory::Blocks));
        assert!(entries
            .iter()
            .any(|e| e.category == ExplorerCategory::Tools));
        assert!(entries
            .iter()
            .any(|e| e.category == ExplorerCategory::Materials));
    }

    #[test]
    fn enumerate_skips_air() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        assert!(entries
            .iter()
            .all(|e| !matches!(&e.item, Item::Block(b) if *b == AIR)));
        // "Air" shouldn't appear by display label either.
        assert!(entries.iter().all(|e| e.label != "Air"));
    }

    #[test]
    fn enumerate_block_count_matches_registry_minus_air() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        let block_count = entries
            .iter()
            .filter(|e| e.category == ExplorerCategory::Blocks)
            .count();
        assert_eq!(block_count, registry.len() - 1);
    }

    #[test]
    fn enumerate_material_count_matches_table() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        let material_count = entries
            .iter()
            .filter(|e| e.category == ExplorerCategory::Materials)
            .count();
        assert_eq!(material_count, ALL_MATERIAL_IDS.len());
    }

    #[test]
    fn brigand_chieftain_trophy_appears_in_inventory_explorer() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        assert!(entries
            .iter()
            .any(|e| matches!(&e.item, Item::Material(MaterialId::BrigandChieftainTrophy))));
    }

    #[test]
    fn bellows_and_carts_appear_in_inventory_explorer() {
        // Regression (2026-06-18): the Bellows (and the carts + the dyes/seeds
        // declared after HempSeeds) had drifted off ALL_MATERIAL_IDS, so once a
        // player removed the Bellows from their hotbar it was unobtainable from
        // the creative inventory. Every authoring tool must be re-gettable.
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        for m in [
            MaterialId::Bellows,
            MaterialId::WoodCart,
            MaterialId::IronCart,
            MaterialId::DiamondCart,
            MaterialId::BrownDye,
            MaterialId::CyanDye,
            MaterialId::MagentaDye,
        ] {
            assert!(
                entries
                    .iter()
                    .any(|e| matches!(&e.item, Item::Material(x) if *x == m)),
                "{m:?} should be in the creative inventory explorer"
            );
        }
    }

    #[test]
    fn workshop_and_blueprint_tools_appear_in_inventory_explorer() {
        // Eraser/Slingshot/Drafting Stamp were missing from all_tool_combos(),
        // so they too were unobtainable once removed from the hotbar (2026-06-18).
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        for tt in [
            ToolType::Eraser,
            ToolType::Slingshot,
            ToolType::DraftingStamp,
        ] {
            assert!(
                entries
                    .iter()
                    .any(|e| matches!(&e.item, Item::Tool(t) if t.tool_type == tt)),
                "{tt:?} should be in the creative inventory explorer"
            );
        }
    }

    #[test]
    fn tool_combos_cover_all_tiers_plus_bow_plus_flint_and_steel() {
        let combos = all_tool_combos();
        // 5 tiered tool types × 5 tiers + Bow(Wood) + FlintAndSteel(Iron)
        // + Shears(Iron) + FishingRod(Wood) + Slingshot/Eraser/DraftingStamp
        // (Wood) = 32 (2026-06-18: +3 single-tier utility tools).
        assert_eq!(combos.len(), 32);
        assert!(combos.contains(&(ToolType::Pickaxe, ToolMaterial::Satori)));
        assert!(combos.contains(&(ToolType::Bow, ToolMaterial::Wood)));
        assert!(combos.contains(&(ToolType::FlintAndSteel, ToolMaterial::Iron)));
        assert!(combos.contains(&(ToolType::Shears, ToolMaterial::Iron)));
        assert!(combos.contains(&(ToolType::FishingRod, ToolMaterial::Wood)));
        assert!(combos.contains(&(ToolType::Slingshot, ToolMaterial::Wood)));
        assert!(combos.contains(&(ToolType::Eraser, ToolMaterial::Wood)));
        assert!(combos.contains(&(ToolType::DraftingStamp, ToolMaterial::Wood)));
        // Single-tier tools should not appear in other tiers.
        assert!(!combos.contains(&(ToolType::Bow, ToolMaterial::Iron)));
        assert!(!combos.contains(&(ToolType::FlintAndSteel, ToolMaterial::Diamond)));
        assert!(!combos.contains(&(ToolType::Shears, ToolMaterial::Wood)));
        assert!(!combos.contains(&(ToolType::FishingRod, ToolMaterial::Iron)));
    }

    #[test]
    fn filter_empty_query_returns_all_in_category() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        let all = filter_entries(&entries, "", None);
        assert_eq!(all.len(), entries.len());
    }

    #[test]
    fn filter_search_is_case_insensitive() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        let lo = filter_entries(&entries, "iron", None);
        let up = filter_entries(&entries, "IRON", None);
        let mixed = filter_entries(&entries, "iRoN", None);
        assert_eq!(lo.len(), up.len());
        assert_eq!(lo.len(), mixed.len());
        assert!(lo.iter().any(|e| e.label.contains("Iron")));
    }

    #[test]
    fn filter_category_restricts_correctly() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        let only_tools =
            filter_entries(&entries, "", Some(ExplorerCategory::Tools));
        assert!(only_tools
            .iter()
            .all(|e| e.category == ExplorerCategory::Tools));
        // Tools category includes both tool combos AND armour combos
        // since armour pieces are equipment-shape and live in the same
        // bucket on alpha (4 explorer categories, not 5).
        let expected = all_tool_combos().len() + crate::armour::all_armour_combos().len();
        assert_eq!(only_tools.len(), expected);
    }

    #[test]
    fn filter_category_all_is_identical_to_none() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        let none = filter_entries(&entries, "", None);
        let all =
            filter_entries(&entries, "", Some(ExplorerCategory::All));
        assert_eq!(none.len(), all.len());
    }

    #[test]
    fn filter_combines_query_and_category() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        // "iron" inside Tools category should match every iron tool tier:
        // Iron Pickaxe/Axe/Sword/Shovel/Hoe (5) + the 4 iron armour
        // pieces (Helmet/Chestplate/Leggings/Boots) = 9 entries.
        // FlintAndSteel renders as "Flint and Steel" (no "iron" in the
        // display label), so it doesn't match here — by design.
        let hits = filter_entries(&entries, "iron", Some(ExplorerCategory::Tools));
        assert_eq!(hits.len(), 9);
        assert!(hits.iter().all(|e| e.label.starts_with("Iron ")));
    }

    #[test]
    fn filter_whitespace_trim() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        let trimmed = filter_entries(&entries, "  diamond  ", None);
        let plain = filter_entries(&entries, "diamond", None);
        assert_eq!(trimmed.len(), plain.len());
    }

    #[test]
    fn enumerate_label_for_iron_pickaxe_is_uk_titled() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        assert!(entries.iter().any(|e| e.label == "Iron Pickaxe"));
        assert!(entries.iter().any(|e| e.label == "Satori Pickaxe"));
    }

    #[test]
    fn spec_28c_new_materials_are_enumerated() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        for label in [
            "Copper", "Tin", "Sulphur", "Amethyst", "Copper Ingot",
            "Tin Ingot", "Bronze Ingot", "Sugar",
        ] {
            assert!(
                entries.iter().any(|e| e.label == label),
                "Spec 28c material missing from explorer: {label}"
            );
        }
    }

    #[test]
    fn spec_t1_5_new_materials_are_enumerated() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        for label in [
            "Bucket", "Milk Bucket", "Egg", "Flour", "Dough", "Cream",
            "Butter", "Cheese", "Sweet Bread", "Cake", "Pumpkin Pie",
            "Berry Pie", "Cookie", "Pancakes", "Loaded Baked Potato",
            "Stew", "Beetroot Soup", "Bowl", "Sugar Beet", "Sugar Beet Seeds",
            "Beetroot", "Beetroot Seeds", "Berries",
        ] {
            assert!(
                entries.iter().any(|e| e.label == label),
                "T1.5 material missing from explorer: {label}"
            );
        }
    }

    #[test]
    fn spec_t1_5_value_ladder_monotonic_per_tier() {
        // Value ladder invariant: tier N+1 trade_value > tier N.
        let cheap = crate::item::Item::Material(crate::item::MaterialId::Wheat);     // T0
        let basic = crate::item::Item::Material(crate::item::MaterialId::Bread);     // T1
        let proc = crate::item::Item::Material(crate::item::MaterialId::Cheese);    // T2
        let baked = crate::item::Item::Material(crate::item::MaterialId::Cookie);   // T3
        let meal = crate::item::Item::Material(crate::item::MaterialId::Stew);      // T4
        let master = crate::item::Item::Material(crate::item::MaterialId::Cake);   // T5
        let ladder: Vec<u64> = [&cheap, &basic, &proc, &baked, &meal, &master]
            .iter()
            .map(|i| i.trade_value().unwrap())
            .collect();
        for w in ladder.windows(2) {
            assert!(w[1] > w[0], "value ladder not strictly increasing: {ladder:?}");
        }
    }

    #[test]
    fn spec_t1_5_complexity_tiers_correct() {
        use crate::item::{Item, MaterialId};
        assert_eq!(Item::Material(MaterialId::Wheat).complexity_tier(), 0);
        assert_eq!(Item::Material(MaterialId::Bread).complexity_tier(), 1);
        assert_eq!(Item::Material(MaterialId::Cheese).complexity_tier(), 2);
        assert_eq!(Item::Material(MaterialId::Cookie).complexity_tier(), 3);
        assert_eq!(Item::Material(MaterialId::Stew).complexity_tier(), 4);
        assert_eq!(Item::Material(MaterialId::Cake).complexity_tier(), 5);
    }

    #[test]
    fn spec_t1_5_air_water_have_no_trade_value() {
        use crate::item::Item;
        assert_eq!(Item::Block(crate::block::AIR).trade_value(), None);
        assert_eq!(Item::Block(crate::block::WATER).trade_value(), None);
    }

    #[test]
    fn spec_t1_5_processed_foods_are_edible() {
        use crate::item::{Item, MaterialId};
        for m in [
            MaterialId::Cheese, MaterialId::Butter, MaterialId::Cake,
            MaterialId::Cookie, MaterialId::Pancakes, MaterialId::Stew,
            MaterialId::SweetBread, MaterialId::PumpkinPie, MaterialId::BerryPie,
            MaterialId::BeetrootSoup, MaterialId::LoadedBakedPotato,
        ] {
            assert!(Item::Material(m).is_food(), "{m:?} should be food");
        }
    }

    #[test]
    fn spec_28c_new_blocks_are_enumerated() {
        let registry = BlockRegistry::new();
        let entries = enumerate_all_items(&registry);
        for label in [
            "Limestone", "Marble", "Granite", "Slate", "Copper Ore",
            "Bone Cairn", "Hay Rick", "Amethyst Cluster",
        ] {
            assert!(
                entries.iter().any(|e| e.label == label),
                "Spec 28c block missing from explorer: {label}"
            );
        }
    }

    #[test]
    fn stack_for_block_is_one_count() {
        let stack = stack_for(&Item::Block(crate::block::STONE));
        assert_eq!(stack.count, 1);
        assert!(matches!(stack.item, Item::Block(_)));
    }

    #[test]
    fn stack_for_material_is_one_count() {
        let stack = stack_for(&Item::Material(MaterialId::Stick));
        assert_eq!(stack.count, 1);
    }

    #[test]
    fn stack_for_plan_is_empty_placeholder() {
        // Plans are per-instance; the explorer can't dispense them.
        // Return an empty stack rather than panic.
        let plan = crate::plan::PlanData::debug_3x3_stone();
        let stack = stack_for(&Item::Plan(plan));
        assert_eq!(stack.count, 0);
    }
}
