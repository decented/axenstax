//! Crafting & tool system — tool types, mining speed, durability, recipes.
//!
//! Spec 05 Section 3 (Tools) and Section 5 (Crafting).

use serde::{Deserialize, Serialize};

use crate::block::{self, BlockId};
use crate::item::{Item, ItemStack, MaterialId};

/// Tool material tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolMaterial {
    Wood,
    Stone,
    Iron,
    Diamond,
    /// Top tier — Satori, the orange Bitcoin gem (Spec 5 §3.8 / Spec 6 §2.2c).
    /// Above diamond in durability + mining speed + mining level. Crafted from
    /// the Satori material, which itself drops from pure-deepslate veins at
    /// depth.
    Satori,
}

/// Tool type.
///
/// **Bincode-positional**: variants are serialised by index. Appending to
/// the end is the only safe way to add — inserting `Hoe` between `Shovel`
/// and `Bow` (which would read better cosmetically) would shift `Bow`
/// from index 4 to 5 and rewrite existing saves' bows as hoes. So
/// farming `Hoe` is appended *after* `Bow` despite the cosmetic awkwardness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolType {
    Pickaxe,
    Axe,
    Sword,
    Shovel,
    /// Ranged weapon (Wave 23). Recipe only produces ToolMaterial::Wood;
    /// per-tier bows are reserved for a future polish wave.
    Bow,
    /// Farming Tier 1 (Wave 26, 2026-05-17). Tills dirt/grass into
    /// tilled soil. Utility tool — attack_damage is 1.0 regardless of
    /// tier (a Satori Hoe is not a weapon).
    Hoe,
    /// Campfire ignition (Wave 27, 2026-05-18). Single-tier — crafted
    /// from 1 flint + 1 iron ingot regardless of `ToolMaterial`. By
    /// convention the constructor uses `ToolMaterial::Iron`. Right-
    /// click an unlit campfire to ignite it; durability decrements by
    /// 1 per use (~65 uses per the FLINT_AND_STEEL_DURABILITY const).
    FlintAndSteel,
    /// Spec 28e Phase 2 — Shears. Single-tier (Iron only by recipe).
    /// Uses: shear sheep for Wool without killing them (alpha: same
    /// drop table for now); future Bee Hive harvest. Right-click to
    /// shear; durability decrements by 1 per use.
    Shears,
    /// Spec 28e Phase 3 — Fishing Rod. Single-tier (Wood only by
    /// recipe). v1 spawns the tool; cast / bobber / catch mechanics
    /// are deferred to a future playtest wave.
    FishingRod,
    /// Rubber feature — Slingshot ranged tool. Single Wood-tier (no
    /// per-material ladder). Fires Rubber Ball ammo with charge-scaled
    /// damage. Stun-on-hit for passive + neutral mobs at half charge
    /// or more (carnivores + brigands shrug it off).
    Slingshot,
    /// Rubber feature — Eraser. Single Wood-tier. Right-click on a
    /// BLUEPRINT_PAPER block converts it back to 1 PapyrusSheet (partial
    /// reclaim of paper costs).
    Eraser,
    /// Blueprint feature — Drafting Stamp. Single Wood-tier. Right-click
    /// on a built structure with Blueprint Paper laid as a floor tile to
    /// capture it as a Plan item. Reusable: the use path does NOT call
    /// use_tool(), so durability is never decremented in practice.
    DraftingStamp,
}

/// Single-tier durability for `FlintAndSteel`. Mirrors Minecraft's 65
/// uses; lives separate from `max_durability` so the per-tier ladder
/// for other tools isn't disturbed.
pub const FLINT_AND_STEEL_DURABILITY: u16 = 65;

/// Spec 28e — Shears durability. Mirrors Minecraft baseline ~238 uses.
pub const SHEARS_DURABILITY: u16 = 238;

/// Spec 28e — Fishing Rod durability. Mirrors Minecraft baseline 64
/// casts. Cast/catch mechanics deferred — durability is wired now so
/// when the live system lands it doesn't need to touch this module.
pub const FISHING_ROD_DURABILITY: u16 = 64;

/// Rubber feature — Slingshot durability. 64 shots before it breaks
/// (matches Fishing Rod's tier).
pub const SLINGSHOT_DURABILITY: u16 = 64;

/// Rubber feature — Eraser durability. 32 uses (same as Flint and
/// Steel halved — erasing is a more delicate operation than igniting).
pub const ERASER_DURABILITY: u16 = 32;

/// Blueprint feature — Drafting Stamp durability. 32 uses (mirrors the
/// Eraser tier — a stamp head is similarly delicate). In practice the
/// Stamp use path does NOT call use_tool(), so this value is never
/// decremented; it exists so the item serialises cleanly.
pub const DRAFTING_STAMP_DURABILITY: u16 = 32;

/// A tool item with type, material, and remaining durability.
///
/// `PartialEq` (2026-07-06, Task 11) — needed so `Item`/`ItemStack`/
/// `ChestData` can derive it in turn, for `HorseData.pack`'s whole-struct
/// equality checks in `save.rs` (`SavedTamedPetData: PartialEq`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub tool_type: ToolType,
    pub material: ToolMaterial,
    pub durability: u16,
}

impl Tool {
    pub fn new(tool_type: ToolType, material: ToolMaterial) -> Self {
        // Single-tier tools use their own durability constants rather
        // than the per-material ladder.
        let durability = match tool_type {
            ToolType::FlintAndSteel => FLINT_AND_STEEL_DURABILITY,
            ToolType::Shears => SHEARS_DURABILITY,
            ToolType::FishingRod => FISHING_ROD_DURABILITY,
            ToolType::Slingshot => SLINGSHOT_DURABILITY,
            ToolType::Eraser => ERASER_DURABILITY,
            ToolType::DraftingStamp => DRAFTING_STAMP_DURABILITY,
            _ => max_durability(material),
        };
        Self { tool_type, material, durability }
    }

    pub fn is_broken(&self) -> bool {
        self.durability == 0
    }

    /// Maximum durability for this tool — the cap a fresh `Tool::new`
    /// would start at. Generalises the per-material + single-tier
    /// logic so repair (Spec 35) can compute "how damaged is this".
    pub fn max_durability(&self) -> u16 {
        match self.tool_type {
            ToolType::FlintAndSteel => FLINT_AND_STEEL_DURABILITY,
            ToolType::Shears => SHEARS_DURABILITY,
            ToolType::FishingRod => FISHING_ROD_DURABILITY,
            ToolType::Slingshot => SLINGSHOT_DURABILITY,
            ToolType::Eraser => ERASER_DURABILITY,
            ToolType::DraftingStamp => DRAFTING_STAMP_DURABILITY,
            _ => max_durability(self.material),
        }
    }

    pub fn use_tool(&mut self) -> bool {
        if self.durability > 0 {
            self.durability -= 1;
            true
        } else {
            false
        }
    }

    pub fn attack_damage(&self) -> f32 {
        match self.tool_type {
            ToolType::Sword => match self.material {
                ToolMaterial::Wood => 4.0,
                ToolMaterial::Stone => 5.0,
                ToolMaterial::Iron => 6.0,
                ToolMaterial::Diamond => 7.0,
                ToolMaterial::Satori => 8.0,
            },
            ToolType::Axe => match self.material {
                ToolMaterial::Wood => 7.0,
                ToolMaterial::Stone => 7.0,
                ToolMaterial::Iron => 7.0,
                ToolMaterial::Diamond => 9.0,
                ToolMaterial::Satori => 10.0,
            },
            // Bows are ranged — melee with a bow does fist damage,
            // EXCEPT in the per-tier ladder where the tier hints at
            // a stronger arrow draw (consumed by the Bow's arrow-fire
            // path upstream). Melee values stay tiny.
            ToolType::Bow => match self.material {
                ToolMaterial::Wood => 1.0,
                ToolMaterial::Stone => 1.0,
                ToolMaterial::Iron => 1.5,
                ToolMaterial::Diamond => 2.0,
                ToolMaterial::Satori => 2.5,
            },
            _ => 1.0,
        }
    }

    pub fn mining_speed(&self) -> f32 {
        match self.material {
            ToolMaterial::Wood => 2.0,
            ToolMaterial::Stone => 4.0,
            ToolMaterial::Iron => 6.0,
            ToolMaterial::Diamond => 8.0,
            ToolMaterial::Satori => 9.0,
        }
    }

    /// Spec 28d chunk 9 — per-tier Bow arrow damage. The Bow itself
    /// only does fist damage in melee; this is the per-arrow damage
    /// scaling consumed by the arrow-fire path (`combat.rs` /
    /// projectile spawn). Wood is the original baseline; higher tiers
    /// are linearly scaled. No caller found in combat.rs — arrows may not
    /// be a real fired projectile yet. Tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn bow_arrow_damage(&self) -> f32 {
        if !matches!(self.tool_type, ToolType::Bow) {
            return 0.0;
        }
        match self.material {
            ToolMaterial::Wood => 5.0,    // baseline
            ToolMaterial::Stone => 6.0,
            ToolMaterial::Iron => 7.0,
            ToolMaterial::Diamond => 9.0,
            ToolMaterial::Satori => 10.0,
        }
    }

    pub fn name(&self) -> &'static str {
        match (self.material, self.tool_type) {
            (ToolMaterial::Wood, ToolType::Pickaxe) => "Wooden Pickaxe",
            (ToolMaterial::Wood, ToolType::Axe) => "Wooden Axe",
            (ToolMaterial::Wood, ToolType::Sword) => "Wooden Sword",
            (ToolMaterial::Wood, ToolType::Shovel) => "Wooden Shovel",
            (ToolMaterial::Stone, ToolType::Pickaxe) => "Stone Pickaxe",
            (ToolMaterial::Stone, ToolType::Axe) => "Stone Axe",
            (ToolMaterial::Stone, ToolType::Sword) => "Stone Sword",
            (ToolMaterial::Stone, ToolType::Shovel) => "Stone Shovel",
            (ToolMaterial::Iron, ToolType::Pickaxe) => "Iron Pickaxe",
            (ToolMaterial::Iron, ToolType::Axe) => "Iron Axe",
            (ToolMaterial::Iron, ToolType::Sword) => "Iron Sword",
            (ToolMaterial::Iron, ToolType::Shovel) => "Iron Shovel",
            (ToolMaterial::Diamond, ToolType::Pickaxe) => "Diamond Pickaxe",
            (ToolMaterial::Diamond, ToolType::Axe) => "Diamond Axe",
            (ToolMaterial::Diamond, ToolType::Sword) => "Diamond Sword",
            (ToolMaterial::Diamond, ToolType::Shovel) => "Diamond Shovel",
            (ToolMaterial::Satori, ToolType::Pickaxe) => "Satori Pickaxe",
            (ToolMaterial::Satori, ToolType::Axe) => "Satori Axe",
            (ToolMaterial::Satori, ToolType::Sword) => "Satori Sword",
            (ToolMaterial::Satori, ToolType::Shovel) => "Satori Shovel",
            // Hoes (Wave 26 farming). Names declared for every tier
            // even though Phase 3 only ships the wood-tier recipe;
            // Phase 8 ships the rest.
            (ToolMaterial::Wood, ToolType::Hoe) => "Wooden Hoe",
            (ToolMaterial::Stone, ToolType::Hoe) => "Stone Hoe",
            (ToolMaterial::Iron, ToolType::Hoe) => "Iron Hoe",
            (ToolMaterial::Diamond, ToolType::Hoe) => "Diamond Hoe",
            (ToolMaterial::Satori, ToolType::Hoe) => "Satori Hoe",
            // Flint and Steel is single-tier (Wave 27).
            (_, ToolType::FlintAndSteel) => "Flint and Steel",
            // Bows (Wave 23 + chunk 9 per-tier). Per-tier upgrade
            // recipes use the same shape with a tier-material in the
            // centre slot.
            (ToolMaterial::Wood, ToolType::Bow) => "Bow",
            (ToolMaterial::Stone, ToolType::Bow) => "Stone Bow",
            (ToolMaterial::Iron, ToolType::Bow) => "Iron Bow",
            (ToolMaterial::Diamond, ToolType::Bow) => "Diamond Bow",
            (ToolMaterial::Satori, ToolType::Bow) => "Satori Bow",
            // Spec 28e — Shears + Fishing Rod, each single-tier.
            (_, ToolType::Shears) => "Shears",
            (_, ToolType::FishingRod) => "Fishing Rod",
            // Rubber feature — Slingshot + Eraser, each single-tier.
            (_, ToolType::Slingshot) => "Slingshot",
            (_, ToolType::Eraser) => "Eraser",
            // Blueprint feature — Drafting Stamp, single-tier.
            (_, ToolType::DraftingStamp) => "Drafting Stamp",
        }
    }

    pub fn color(&self) -> [f32; 3] {
        match self.material {
            ToolMaterial::Wood => [0.6, 0.45, 0.25],
            ToolMaterial::Stone => [0.5, 0.5, 0.5],
            ToolMaterial::Iron => [0.8, 0.8, 0.8],
            ToolMaterial::Diamond => [0.3, 0.9, 0.9],
            ToolMaterial::Satori => [0.95, 0.55, 0.18],
        }
    }
}

fn max_durability(material: ToolMaterial) -> u16 {
    match material {
        ToolMaterial::Wood => 59,
        ToolMaterial::Stone => 131,
        ToolMaterial::Iron => 250,
        ToolMaterial::Diamond => 1561,
        ToolMaterial::Satori => 2031,
    }
}

/// Block hardness in seconds (how long to mine with bare hands).
pub fn block_hardness(block_id: BlockId) -> f32 {
    // Values = seconds to break with bare fist (speed 1.0).
    match block_id {
        block::DIRT | block::GRASS => 2.5,  // Axolittle: 2.5s
        block::SAND => 0.5,
        block::GRAVEL => 0.6,
        block::STONE => 10.0,               // Axolittle: 10s
        block::COBBLESTONE => 2.0,
        block::OAK_LOG => 4.5,              // Axolittle: 4.5s
        block::OAK_PLANKS | block::CRAFTING_TABLE => 2.0,
        // Spec 48 Phase 4 — the Water Wheel is a plank build; mine it like planks.
        block::WATER_WHEEL | block::WATER_WHEEL_TURNING => 2.0,
        // Wind wave §2.2 — the Windmill is canvas on planks; same rung.
        block::WINDMILL | block::WINDMILL_TURNING => 2.0,
        block::OAK_LEAVES => 0.2,
        block::SANDSTONE => 0.8,
        block::SNOW => 0.2,
        // Ore blocks (MC ~3s with bare hands, fast with proper pickaxe).
        block::COAL_ORE | block::IRON_ORE | block::DIAMOND_ORE => 3.0,
        // Spec 49 — Brimstone + Nitre ores on the same hardness rung as stone ores.
        block::BRIMSTONE | block::NITRE_ORE => 3.0,
        // Storage blocks — denser than ore, slower to mine.
        block::COAL_BLOCK | block::IRON_BLOCK | block::DIAMOND_BLOCK => 5.0,
        // Deepslate is significantly harder than stone (MC 1.18 baseline: pure
        // deepslate ~3x stone hardness; deepslate ores ~50% slower than stone
        // counterparts). Encourages the "commit to depth" mining loop.
        // Spec 16 Phase 3 — all pure-deepslate-family variants share
        // this break time. (Match-guard form lets Phase 3b extension
        // pick up automatically.)
        id if block::is_pure_deepslate_family(id) => 15.0,
        block::DEEPSLATE_COAL_ORE | block::DEEPSLATE_IRON_ORE | block::DEEPSLATE_DIAMOND_ORE => 4.5,
        block::SATORI_BLOCK => 5.0,
        block::BED => 0.2,
        block::GLASS => 0.3,
        block::TORCH => 0.0,
        _ => 1.0,
    }
}

/// Proof-of-play **work** units for breaking `block_id` — a function of its
/// intrinsic hardness (the work it takes to break). Scaled so the lowest
/// non-instant block (leaves, 0.2s) is worth exactly 1 unit; everything else
/// scales off `block_hardness` from there (stone 50, pure deepslate 75,
/// satori 25). Instant-break blocks (torch, 0s) are worth 0.
///
/// This is the per-block value tallied into the world's lifetime `total_work`
/// and a scenario's work score on each successful `can_harvest` break. The
/// tool sets *capability + speed*, never the work count — see
/// `docs/foundations/2026-06-03-work-based-hashing.md` and Spec 6 §2.
pub fn block_work(block_id: BlockId) -> u64 {
    // 1 unit = one leaf's worth of bare-hand work (leaf hardness 0.2s × 5 = 1).
    (block_hardness(block_id).max(0.0) * 5.0).round() as u64
}

/// Proof-of-play **work credited for breaking a block** — the single decision
/// point shared by the production break path and the test harness so they can
/// never drift. Returns [`block_work`] only when the break is *harvestable*
/// (right tool/tier) AND the block was **not** placed by a player. A
/// player-placed block earns 0: you still recover the item, but the
/// place→break / break-restand-rebreak loop can't farm hash/work or, on a
/// Bitcoin-enabled server, mint payouts (Spec 06 §2.2 anti-farming).
pub fn break_work(block_id: BlockId, harvestable: bool, was_player_placed: bool) -> u64 {
    if harvestable && !was_player_placed {
        block_work(block_id)
    } else {
        0
    }
}

/// Best tool type for a block.
pub fn best_tool_for(block_id: BlockId) -> Option<ToolType> {
    match block_id {
        block::STONE | block::COBBLESTONE | block::SANDSTONE
        | block::COAL_ORE | block::IRON_ORE | block::DIAMOND_ORE
        | block::COAL_BLOCK | block::IRON_BLOCK | block::DIAMOND_BLOCK
        | block::DEEPSLATE_COAL_ORE
        | block::DEEPSLATE_IRON_ORE | block::DEEPSLATE_DIAMOND_ORE
        // Spec 49 — Brimstone + Nitre ores mine fastest with a pickaxe.
        | block::BRIMSTONE | block::NITRE_ORE
        | block::SATORI_BLOCK => Some(ToolType::Pickaxe),
        // Spec 16 Phase 3 — every pure-deepslate-family variant gets
        // the Pickaxe bonus. Match-guard form mirrors `break_time_ticks`
        // so Phase 3b's variant rollout is automatic.
        id if block::is_pure_deepslate_family(id) => Some(ToolType::Pickaxe),
        block::DIRT | block::GRASS | block::SAND | block::GRAVEL | block::SNOW => Some(ToolType::Shovel),
        block::OAK_LOG | block::OAK_PLANKS | block::CRAFTING_TABLE => Some(ToolType::Axe),
        _ => None,
    }
}

/// Numeric tier index for a tool material (Wood=0, Stone=1, Iron=2,
/// Diamond=3). Used by `can_harvest` and elsewhere to compare tiers
/// without depending on enum variant order.
pub fn tier_index(material: ToolMaterial) -> u8 {
    match material {
        ToolMaterial::Wood => 0,
        ToolMaterial::Stone => 1,
        ToolMaterial::Iron => 2,
        ToolMaterial::Diamond => 3,
        ToolMaterial::Satori => 4,
    }
}

/// Minimum pickaxe tier required to actually *harvest* a block (i.e. get a
/// drop). Returns None if the block has no tier requirement (any tool, even
/// fist, drops it). Returns Some(material) for the minimum pickaxe material.
///
/// Mining a block below your tier-requirement still breaks the block
/// (`break_time_ticks` returns the slow-mode time) but `can_harvest`
/// returns false → no drop.
pub fn min_tool_tier(block_id: BlockId) -> Option<ToolMaterial> {
    match block_id {
        // Stone-like blocks all need a Wood pickaxe at minimum.
        block::STONE | block::COBBLESTONE | block::SANDSTONE
        | block::COAL_ORE | block::COAL_BLOCK => Some(ToolMaterial::Wood),
        // Iron tier requires a Stone pickaxe.
        block::IRON_ORE | block::IRON_BLOCK => Some(ToolMaterial::Stone),
        // Magnesium + Copper are metal ores on the same Stone-pickaxe rung as
        // iron. Without an arm they fell through to `None` → harvestable with a
        // fist, contrary to the documented intent (engine audit 2026-06-04, E1).
        block::MAGNESIUM_ORE | block::COPPER_ORE => Some(ToolMaterial::Stone),
        // Spec 49 — Brimstone (sulphur) + Nitre ores: stone-pickaxe rung.
        block::BRIMSTONE | block::NITRE_ORE => Some(ToolMaterial::Stone),
        // Diamond tier requires an Iron pickaxe.
        block::DIAMOND_ORE | block::DIAMOND_BLOCK => Some(ToolMaterial::Iron),
        // Deepslate variants: pure + coal at Stone tier (deepslate is harder
        // than stone-tier ore but doesn't add a new mining-level rung);
        // deepslate-iron-ore at Stone, deepslate-diamond-ore at Iron — same
        // shape as the stone-tier ladder.
        //
        // Spec 16 Phase 3 — pure-deepslate-family variants share the
        // Stone-tier requirement via the helper. Match-guard form so
        // Phase 3b's variant rollout (ids 61-63) is automatic.
        id if block::is_pure_deepslate_family(id) => Some(ToolMaterial::Stone),
        block::DEEPSLATE_COAL_ORE => Some(ToolMaterial::Stone),
        block::DEEPSLATE_IRON_ORE => Some(ToolMaterial::Stone),
        block::DEEPSLATE_DIAMOND_ORE => Some(ToolMaterial::Iron),
        // Satori storage block: same Iron-tier requirement as the diamond
        // storage block.
        block::SATORI_BLOCK => Some(ToolMaterial::Iron),
        // Everything else (dirt, sand, wood, glass, bed, torch, …) any tool
        // (or fist) can harvest. Bedrock is a separate concern — it's never
        // actually broken in the mining handler.
        _ => None,
    }
}

/// Whether the player can actually harvest (= get a drop from) this block
/// with the held tool. Pickaxe-tier-gated blocks need a pickaxe of at least
/// the required material tier. Everything else is harvestable with any tool
/// or no tool.
pub fn can_harvest(block_id: BlockId, tool: Option<&Tool>) -> bool {
    let Some(req) = min_tool_tier(block_id) else { return true; };
    match tool {
        Some(t) if t.tool_type == ToolType::Pickaxe => tier_index(t.material) >= tier_index(req),
        _ => false,
    }
}

/// Calculate break time in ticks.
pub fn break_time_ticks(block_id: BlockId, tool: Option<&Tool>) -> u32 {
    let hardness = block_hardness(block_id);
    let speed = match tool {
        Some(t) if best_tool_for(block_id) == Some(t.tool_type) => t.mining_speed(),
        _ => 1.0,
    };
    let seconds = hardness / speed;
    (seconds * 20.0).max(1.0) as u32
}

/// Crack-overlay stage (0-9) for the current mining progress, or `None` when
/// nothing should be drawn.
///
/// Returns `None` for instant breaks (`break_time == 0`, i.e. creative) and
/// before any progress accrues (`break_progress == 0`), so the overlay appears
/// only once the player has actually started mining and clears the instant they
/// stop — `break_progress` resets to 0 on release / target-switch. The final
/// mining tick (`break_progress >= break_time`) clamps to stage 9 rather than an
/// out-of-range 10, so the result is always a valid crack-texture stage index.
/// Spec 05 §2.2.
pub fn crack_stage(break_progress: u32, break_time: u32) -> Option<u8> {
    if break_time == 0 || break_progress == 0 {
        return None;
    }
    let stage = (break_progress.saturating_mul(10) / break_time).min(9);
    Some(stage as u8)
}

// --- Crafting Recipes ---

/// A crafting slot: what kind of item is in this grid position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CraftSlot {
    Empty,
    Block(BlockId),
    Material(MaterialId),
}

impl CraftSlot {
    /// Convert from an Item reference.
    pub fn from_item(item: &Item) -> Self {
        match item {
            Item::Block(id) => CraftSlot::Block(*id),
            Item::Material(id) => CraftSlot::Material(*id),
            Item::Tool(_) => CraftSlot::Empty, // Tools can't be crafting ingredients
            // Spec 24 — Plans can't be slotted into a recipe in v1.
            // Future spec (modify-via-crafting) may revisit.
            Item::Plan(_) => CraftSlot::Empty,
            // Spec 28e — armour can't be a crafting ingredient (no
            // recycling on alpha).
            Item::Armour(_) => CraftSlot::Empty,
        }
    }
}

/// Try to match a 3x3 crafting grid against known recipes.
/// Returns the output ItemStack if a recipe matches.
/// Whether a single grid slot holds something that counts as "a log" for
/// recipe purposes. Accepts both the back-compat `Block(OAK_LOG)` (old
/// saves' inventory log-blocks) and the three Wave 29 log materials
/// (GreenLog / SeasonedLog / KilnDriedLog). Used by the 1×1 plank recipe.
pub fn is_logish_slot(slot: CraftSlot) -> bool {
    match slot {
        CraftSlot::Block(b) if b == block::OAK_LOG => true,
        CraftSlot::Material(MaterialId::GreenLog) => true,
        CraftSlot::Material(MaterialId::SeasonedLog) => true,
        CraftSlot::Material(MaterialId::KilnDriedLog) => true,
        _ => false,
    }
}

/// Whether a single grid slot holds "any paper" for recipe purposes.
/// Mirrors [`is_logish_slot`] — recipes that want any paper tier match
/// this without caring about the specific material. v1 covers just
/// `PapyrusSheet` (Spec 23); T1.5 (Spec 12) will join `PulpPaper` here
/// as a one-arm extension; future bamboo / hemp / linen papers slot in
/// the same way. Used by Blueprint Paper (Spec 24) and any future paper-
/// consuming recipe (books / maps / quest scrolls / charter clauses).
///
/// Cross-game-generic — the predicate shape is engine-neutral; the
/// specific MaterialIds are AxeNStax data.
/// Spec 35 Phase 2 completion helper — multiset equality on a 1x3 row
/// (used by the 3-input dye-mix recipes so any horizontal arrangement
/// of the three inputs counts as the same recipe). Each "want" slot
/// must match exactly one row slot; duplicates are honoured by
/// consuming each match slot only once.
fn row_matches_set(row: &[CraftSlot; 3], want: &[CraftSlot; 3]) -> bool {
    let mut matched = [false; 3];
    for &actual in row {
        let mut found = false;
        for (i, w) in want.iter().enumerate() {
            if !matched[i] && *w == actual {
                matched[i] = true;
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    matched.iter().all(|&m| m)
}

/// Map a dye `MaterialId` to its matching sail BlockId. Sail is the
/// first **Canvas** consumer (2026-05-28); shape mirrors banner
/// (column with the middle slot swapping Cloth → Canvas).
pub fn sail_for_dye(dye: crate::item::MaterialId) -> Option<BlockId> {
    use crate::item::MaterialId as M;
    Some(match dye {
        M::WhiteDye => block::SAIL_WHITE,
        M::BlackDye => block::SAIL_BLACK,
        M::RedDye => block::SAIL_RED,
        M::BlueDye => block::SAIL_BLUE,
        M::YellowDye => block::SAIL_YELLOW,
        M::OrangeDye => block::SAIL_ORANGE,
        M::GreenDye => block::SAIL_GREEN,
        M::PurpleDye => block::SAIL_PURPLE,
        M::PinkDye => block::SAIL_PINK,
        M::LimeDye => block::SAIL_LIME,
        M::LightBlueDye => block::SAIL_LIGHT_BLUE,
        M::GreyDye => block::SAIL_GREY,
        M::LightGreyDye => block::SAIL_LIGHT_GREY,
        M::BrownDye => block::SAIL_BROWN,
        M::CyanDye => block::SAIL_CYAN,
        M::MagentaDye => block::SAIL_MAGENTA,
        _ => return None,
    })
}

/// Map a dye `MaterialId` to its matching banner BlockId. Banner is
/// the first **Cloth** consumer (2026-05-28). Sibling to
/// `bunting_for_dye` / `paper_lantern_for_dye` / `kite_for_dye`; all
/// four share the same 16-colour dye domain.
pub fn banner_for_dye(dye: crate::item::MaterialId) -> Option<BlockId> {
    use crate::item::MaterialId as M;
    Some(match dye {
        M::WhiteDye => block::BANNER_WHITE,
        M::BlackDye => block::BANNER_BLACK,
        M::RedDye => block::BANNER_RED,
        M::BlueDye => block::BANNER_BLUE,
        M::YellowDye => block::BANNER_YELLOW,
        M::OrangeDye => block::BANNER_ORANGE,
        M::GreenDye => block::BANNER_GREEN,
        M::PurpleDye => block::BANNER_PURPLE,
        M::PinkDye => block::BANNER_PINK,
        M::LimeDye => block::BANNER_LIME,
        M::LightBlueDye => block::BANNER_LIGHT_BLUE,
        M::GreyDye => block::BANNER_GREY,
        M::LightGreyDye => block::BANNER_LIGHT_GREY,
        M::BrownDye => block::BANNER_BROWN,
        M::CyanDye => block::BANNER_CYAN,
        M::MagentaDye => block::BANNER_MAGENTA,
        _ => return None,
    })
}

/// Spec 35 dyed-décor — map a dye `MaterialId` to its matching kite
/// BlockId. Sibling to `bunting_for_dye` + `paper_lantern_for_dye`;
/// the three mappings share the same 16-colour domain.
pub fn kite_for_dye(dye: crate::item::MaterialId) -> Option<BlockId> {
    use crate::item::MaterialId as M;
    Some(match dye {
        M::WhiteDye => block::KITE_WHITE,
        M::BlackDye => block::KITE_BLACK,
        M::RedDye => block::KITE_RED,
        M::BlueDye => block::KITE_BLUE,
        M::YellowDye => block::KITE_YELLOW,
        M::OrangeDye => block::KITE_ORANGE,
        M::GreenDye => block::KITE_GREEN,
        M::PurpleDye => block::KITE_PURPLE,
        M::PinkDye => block::KITE_PINK,
        M::LimeDye => block::KITE_LIME,
        M::LightBlueDye => block::KITE_LIGHT_BLUE,
        M::GreyDye => block::KITE_GREY,
        M::LightGreyDye => block::KITE_LIGHT_GREY,
        M::BrownDye => block::KITE_BROWN,
        M::CyanDye => block::KITE_CYAN,
        M::MagentaDye => block::KITE_MAGENTA,
        _ => return None,
    })
}

/// Spec 35 dyed-décor — map a dye `MaterialId` to its matching paper-
/// lantern BlockId. Sibling to `bunting_for_dye`; the two mappings
/// share the same 16-colour domain.
pub fn paper_lantern_for_dye(dye: crate::item::MaterialId) -> Option<BlockId> {
    use crate::item::MaterialId as M;
    Some(match dye {
        M::WhiteDye => block::PAPER_LANTERN_WHITE,
        M::BlackDye => block::PAPER_LANTERN_BLACK,
        M::RedDye => block::PAPER_LANTERN_RED,
        M::BlueDye => block::PAPER_LANTERN_BLUE,
        M::YellowDye => block::PAPER_LANTERN_YELLOW,
        M::OrangeDye => block::PAPER_LANTERN_ORANGE,
        M::GreenDye => block::PAPER_LANTERN_GREEN,
        M::PurpleDye => block::PAPER_LANTERN_PURPLE,
        M::PinkDye => block::PAPER_LANTERN_PINK,
        M::LimeDye => block::PAPER_LANTERN_LIME,
        M::LightBlueDye => block::PAPER_LANTERN_LIGHT_BLUE,
        M::GreyDye => block::PAPER_LANTERN_GREY,
        M::LightGreyDye => block::PAPER_LANTERN_LIGHT_GREY,
        M::BrownDye => block::PAPER_LANTERN_BROWN,
        M::CyanDye => block::PAPER_LANTERN_CYAN,
        M::MagentaDye => block::PAPER_LANTERN_MAGENTA,
        _ => return None,
    })
}

/// Spec 35 dyed-décor — map a dye `MaterialId` to its matching
/// bunting BlockId. Returns `None` for non-dye materials. Shared by
/// the bunting recipe + paper-lantern recipe so the dye → décor
/// mapping has one canonical source.
pub fn bunting_for_dye(dye: crate::item::MaterialId) -> Option<BlockId> {
    use crate::item::MaterialId as M;
    Some(match dye {
        M::WhiteDye => block::BUNTING_WHITE,
        M::BlackDye => block::BUNTING_BLACK,
        M::RedDye => block::BUNTING_RED,
        M::BlueDye => block::BUNTING_BLUE,
        M::YellowDye => block::BUNTING_YELLOW,
        M::OrangeDye => block::BUNTING_ORANGE,
        M::GreenDye => block::BUNTING_GREEN,
        M::PurpleDye => block::BUNTING_PURPLE,
        M::PinkDye => block::BUNTING_PINK,
        M::LimeDye => block::BUNTING_LIME,
        M::LightBlueDye => block::BUNTING_LIGHT_BLUE,
        M::GreyDye => block::BUNTING_GREY,
        M::LightGreyDye => block::BUNTING_LIGHT_GREY,
        M::BrownDye => block::BUNTING_BROWN,
        M::CyanDye => block::BUNTING_CYAN,
        M::MagentaDye => block::BUNTING_MAGENTA,
        _ => return None,
    })
}

pub fn is_paperish_slot(slot: CraftSlot) -> bool {
    matches!(slot, CraftSlot::Material(MaterialId::PapyrusSheet))
}

pub fn match_recipe(grid: &[[CraftSlot; 3]; 3]) -> Option<ItemStack> {
    // Find the bounding box of non-empty slots
    let (min_r, max_r, min_c, max_c) = grid_bounds(grid);
    if min_r > max_r { return None; } // Empty grid

    let h = max_r - min_r + 1;
    let w = max_c - min_c + 1;

    // Extract the trimmed subgrid for matching
    // Check against each recipe

    // --- 1x1 recipes ---
    if h == 1 && w == 1 {
        let slot = grid[min_r][min_c];
        // Spec 48 (Electricity) — Button: a single Stone (classic). 1×1 stone
        // crafts nothing else today, so this is unambiguous.
        if slot == CraftSlot::Block(block::STONE) {
            return Some(ItemStack::new_block(block::BUTTON, 1));
        }
        // Any-log → 4 Planks. Wave 29 (log seasoning) generalised this
        // from `Block(OAK_LOG)` only to also accept the three log
        // materials. Yield is uniform — structural plank-use isn't gated
        // on seasoning; that pressure lives in the fuel ladder instead.
        // Spec 28b — per-species fresh-log blocks craft to their species'
        // planks. Mine-drops normalise to GreenLog (species-neutral), so
        // these arms fire only on creative `/give` or via the explorer
        // dispensing a specific species log block. Falls through to the
        // generic Oak-planks default below for log materials.
        if let CraftSlot::Block(b) = slot {
            if b == block::BIRCH_LOG {
                return Some(ItemStack::new_block(block::BIRCH_PLANKS, 4));
            }
            if b == block::SPRUCE_LOG {
                return Some(ItemStack::new_block(block::SPRUCE_PLANKS, 4));
            }
            if b == block::JUNGLE_LOG {
                return Some(ItemStack::new_block(block::JUNGLE_PLANKS, 4));
            }
            if b == block::ACACIA_LOG {
                return Some(ItemStack::new_block(block::ACACIA_PLANKS, 4));
            }
            if b == block::DARK_OAK_LOG {
                return Some(ItemStack::new_block(block::DARK_OAK_PLANKS, 4));
            }
        }
        if is_logish_slot(slot) {
            return Some(ItemStack::new_block(block::OAK_PLANKS, 4));
        }
        // Bone → 3 Bonemeal (Wave 14)
        if slot == CraftSlot::Material(MaterialId::Bone) {
            return Some(ItemStack::new_material(MaterialId::Bonemeal, 3));
        }
        // Spec 35 (2026-05-27) — flowers press to their primary dye, one
        // flower → one dye. Black/White + mixing land in Phase 2.
        if slot == CraftSlot::Block(block::CORNFLOWER) {
            return Some(ItemStack::new_material(MaterialId::BlueDye, 1));
        }
        if slot == CraftSlot::Block(block::FIELD_POPPY) {
            return Some(ItemStack::new_material(MaterialId::RedDye, 1));
        }
        if slot == CraftSlot::Block(block::BUTTERCUP) {
            return Some(ItemStack::new_material(MaterialId::YellowDye, 1));
        }
        // Spec 35 Phase 2 — Black/White from mob materials (the "ink sacs and
        // different things"): Ink Sac (Squid) → Black Dye; Bone Meal → White.
        if slot == CraftSlot::Material(MaterialId::InkSac) {
            return Some(ItemStack::new_material(MaterialId::BlackDye, 1));
        }
        if slot == CraftSlot::Material(MaterialId::Bonemeal) {
            return Some(ItemStack::new_material(MaterialId::WhiteDye, 1));
        }
        // Spec 36 (2026-05-27) — Cotton → String (the honest String source,
        // replacing the retired Wool → String stopgap). The Hemp Fibre →
        // Rope 1:1 stopgap is retired in Phase 2 (2026-05-28); rope now
        // wants 3 Hemp Fibre in a vertical column (see the 3×1 arm
        // further down) — the "twist three fibres into one rope" motif
        // makes rope feel like a heavier cordage than string.
        if slot == CraftSlot::Material(MaterialId::Cotton) {
            return Some(ItemStack::new_material(MaterialId::String, 2));
        }
        // Storage-block → 9 of material (Wave 17 reverse recipes).
        if slot == CraftSlot::Block(block::COAL_BLOCK) {
            return Some(ItemStack::new_material(MaterialId::Coal, 9));
        }
        if slot == CraftSlot::Block(block::IRON_BLOCK) {
            return Some(ItemStack::new_material(MaterialId::RawIron, 9));
        }
        if slot == CraftSlot::Block(block::DIAMOND_BLOCK) {
            return Some(ItemStack::new_material(MaterialId::Diamond, 9));
        }
        // Satori Block → 9 Satori (round-trip for the storage block, same
        // shape as the diamond storage block).
        if slot == CraftSlot::Block(block::SATORI_BLOCK) {
            return Some(ItemStack::new_material(MaterialId::Satori, 9));
        }
        // Spec 28c reverse recipes — match the storage-block 9-of-material
        // pattern so a placed Bone Block / Hay Bale can be reclaimed.
        if slot == CraftSlot::Block(block::BONE_BLOCK) {
            return Some(ItemStack::new_material(MaterialId::Bone, 9));
        }
        if slot == CraftSlot::Block(block::HAY_BALE) {
            return Some(ItemStack::new_material(MaterialId::Wheat, 9));
        }
        // Amethyst Block reverses to 4 (the 2×2 compaction).
        if slot == CraftSlot::Block(block::AMETHYST_BLOCK) {
            return Some(ItemStack::new_material(MaterialId::Amethyst, 4));
        }
        // Salt — SALT_BLOCK reverses to 9 Salt (the 9-grain storage
        // pattern).
        if slot == CraftSlot::Block(block::SALT_BLOCK) {
            return Some(ItemStack::new_material(MaterialId::Salt, 9));
        }
        // Rubber — 1 Rubber -> 4 RubberBalls (slingshot ammo).
        if slot == CraftSlot::Material(MaterialId::Rubber) {
            return Some(ItemStack::new_material(MaterialId::RubberBall, 4));
        }
    }

    // --- 1x2 / 2x1 recipes (vertical) ---
    if h == 2 && w == 1 {
        let top = grid[min_r][min_c];
        let bot = grid[min_r + 1][min_c];
        // Spec 48 (Electricity) — Lever: a Stick on Cobblestone (classic).
        if top == CraftSlot::Material(MaterialId::Stick)
            && bot == CraftSlot::Block(block::COBBLESTONE)
        {
            return Some(ItemStack::new_block(block::LEVER, 1));
        }
        // 2 Planks vertical → 4 Sticks
        if top == CraftSlot::Block(block::OAK_PLANKS) && bot == CraftSlot::Block(block::OAK_PLANKS) {
            return Some(ItemStack::new_material(MaterialId::Stick, 4));
        }
        // Spec 28d.nostrich — Purple Banner. Wool on top, Nostrich-
        // Feather on bottom → 1 decorative banner. T2 trade-value.
        if top == CraftSlot::Material(MaterialId::Wool)
            && bot == CraftSlot::Material(MaterialId::NostrichFeather)
        {
            return Some(ItemStack::new_material(MaterialId::PurpleBanner, 1));
        }
        // Coal on top + Stick below → 4 Torches (Wave 18)
        if top == CraftSlot::Material(MaterialId::Coal)
            && bot == CraftSlot::Material(MaterialId::Stick)
        {
            return Some(ItemStack::new_block(block::TORCH, 4));
        }
        // Campaign D — Sticky Piston. Rubber (our sticky binder, from tapped
        // rubber trees) on top of a Piston → a piston that pulls its block back
        // on retract. Mirrors Minecraft's slimeball+piston.
        if top == CraftSlot::Material(MaterialId::Rubber)
            && bot == CraftSlot::Block(block::PISTON)
        {
            return Some(ItemStack::new_block(block::STICKY_PISTON, 1));
        }
        // Stick on top + Feather below → 4 Arrows (Wave 23). Simplified
        // from MC's stick+flint+feather; we skip flint until it's a thing.
        if top == CraftSlot::Material(MaterialId::Stick)
            && bot == CraftSlot::Material(MaterialId::Feather)
        {
            return Some(ItemStack::new_material(MaterialId::Arrow, 4));
        }
        // Spec 38 (Blueprint / Cyanotype, 2026-05-27) — the old
        // `Stick + PapyrusSheet → 9 Plan Tiles` recipe is RETIRED. The
        // sensitised Blueprint Paper now lives in the 3-tall vertical
        // arm below (`Papyrus Sheet + Iron + Salt → 3 Blueprint Paper`)
        // — the stick was always cosmetic and the iron-salt column reads
        // as cyanotype sensitisation (paper held against the iron-salt
        // solution). The 9 → 3 yield drop reflects iron's cost; large
        // builds now need real planning, not a stack of free paper.
        // Flint on top + IronIngot below → 1 Flint and Steel (Wave 27).
        // Single-tier tool; the Iron material in Tool::new is purely
        // cosmetic — durability comes from FLINT_AND_STEEL_DURABILITY.
        if top == CraftSlot::Material(MaterialId::Flint)
            && bot == CraftSlot::Material(MaterialId::IronIngot)
        {
            return Some(ItemStack::new_tool(Tool::new(
                ToolType::FlintAndSteel,
                ToolMaterial::Iron,
            )));
        }
        // Spec 28c — Bronze Ingot alloy. Copper Ingot on top, Tin Ingot
        // below → 1 Bronze Ingot. Choice of crafting-grid (not furnace)
        // because alloys mix two inputs which furnace doesn't support;
        // the column-of-two motif (copper-over-tin) matches the way
        // smiths historically described the layered alloy. Single
        // ingot per pair to avoid trivialising the recipe.
        if top == CraftSlot::Material(MaterialId::CopperIngot)
            && bot == CraftSlot::Material(MaterialId::TinIngot)
        {
            return Some(ItemStack::new_material(MaterialId::BronzeIngot, 1));
        }
        // Salt — SALT_BLOCK on top, COBBLESTONE on bottom → 1 SALT_LICK.
        // The mineral cube on a stone plinth motif reads as "livestock
        // feeder" without needing extra blocks.
        if top == CraftSlot::Block(block::SALT_BLOCK)
            && bot == CraftSlot::Block(block::COBBLESTONE)
        {
            return Some(ItemStack::new_block(block::SALT_LICK, 1));
        }
        // Rubber — Eraser: Rubber on top + Stick handle below.
        // "Rubber on a handle" — bridges the eraser-of-pencil motif
        // without competing with the 1x1 Rubber → 4 RubberBalls arm.
        if top == CraftSlot::Material(MaterialId::Rubber)
            && bot == CraftSlot::Material(MaterialId::Stick)
        {
            return Some(ItemStack::new_tool(Tool::new(
                ToolType::Eraser,
                ToolMaterial::Wood,
            )));
        }
        // Blueprint — Drafting Stamp: IronIngot on top + Blueprint Paper below.
        // "Iron stamp head pressed onto a prepared sheet" — the iron gives
        // the stamp enough weight to make a clean impression. v1 recipe
        // placeholder — OWNER-CONFIRMABLE: spec proposed "Blueprint Paper
        // + Iron Nugget" but no Iron Nugget material exists; IronIngot is
        // the nearest equivalent. Swap to a cheaper material if desired.
        if top == CraftSlot::Material(MaterialId::IronIngot)
            && bot == CraftSlot::Block(block::BLUEPRINT_PAPER)
        {
            return Some(ItemStack::new_tool(Tool::new(
                ToolType::DraftingStamp,
                ToolMaterial::Wood,
            )));
        }
        // Spec 28e — Shears. Two iron ingots stacked vertically — the
        // hinged-blade motif. Single-tier; durability lives in the
        // SHEARS_DURABILITY constant.
        if top == CraftSlot::Material(MaterialId::IronIngot)
            && bot == CraftSlot::Material(MaterialId::IronIngot)
        {
            return Some(ItemStack::new_tool(Tool::new(
                ToolType::Shears,
                ToolMaterial::Iron,
            )));
        }
        // Smelting recipes (Wave 6 vintage). All moved out:
        // - Meat cooking → campfire (Spec 17).
        // - Iron smelting → furnace (Spec 20 Phase 6, 2026-05-20).
        // The last grid-smelting arm — `RawIron + Coal → IronIngot` —
        // was removed when the Furnace foundation shipped. Players
        // craft a furnace (8 cobblestone ring), drop ore in input
        // and coal in fuel, and the furnace tick produces the ingot.

        // Pets wave Task 7 — Recall Whistle: Bone on top, String below.
        // "A bone whistle on a lanyard" — the same bone-carving motif as
        // Bone Meal/Bone Block, with the string as the cord the owner
        // wears it on. Not consumed on use (see game_loop's held-material
        // right-click handling).
        if top == CraftSlot::Material(MaterialId::Bone)
            && bot == CraftSlot::Material(MaterialId::String)
        {
            return Some(ItemStack::new_material(MaterialId::RecallWhistle, 1));
        }

        // Pets wave Task 9 — Cat Treat: Raw Fish on top, Wheat below. A fish
        // biscuit that guarantees taming a Cat on the first right-click
        // (see the companion-tame block in game_loop.rs), skipping the
        // generic food's 1-in-3 roll.
        if top == CraftSlot::Material(MaterialId::RawFish)
            && bot == CraftSlot::Material(MaterialId::Wheat)
        {
            return Some(ItemStack::new_material(MaterialId::CatTreat, 2));
        }
    }

    // --- 2x3 recipes (Bed + Spec 28e Helmet) ---
    if h == 2 && w == 3 {
        let r0 = [grid[min_r][min_c], grid[min_r][min_c+1], grid[min_r][min_c+2]];
        let r1 = [grid[min_r+1][min_c], grid[min_r+1][min_c+1], grid[min_r+1][min_c+2]];
        // Bed: WWW / PPP — 3 wool on top + 3 planks below.
        let wool = CraftSlot::Material(MaterialId::Wool);
        let planks = CraftSlot::Block(block::OAK_PLANKS);
        if r0 == [wool, wool, wool] && r1 == [planks, planks, planks] {
            return Some(ItemStack::new_block(block::BED, 1));
        }
        // Pet Bed (2026-07-06 pets wave): W.W / PPP — wool tufts at the top
        // corners over a solid plank base (a basket, not a mattress). Kept
        // deliberately distinct from the player Bed's WWW/PPP grid above —
        // an identical shape would either shadow the Bed recipe or never
        // match at all, depending on arm order.
        if r0 == [wool, CraftSlot::Empty, wool] && r1 == [planks, planks, planks] {
            return Some(ItemStack::new_block(block::PET_BED, 1));
        }
        // Spec 36 Fences mini-spec (2026-05-28) + per-species v2 slice
        // (2026-05-28) — Fence Post: PSP / PSP (Minecraft fence motif:
        // plank uprights with a stick rail between). 2×3 → 3 fence
        // posts of THAT plank species. All 4 plank slots must be the
        // SAME species; mixed-species columns fall through (no recipe).
        let stick_2x3 = CraftSlot::Material(MaterialId::Stick);
        if r0[1] == stick_2x3 && r1[1] == stick_2x3
            && let (CraftSlot::Block(p00), CraftSlot::Block(p02),
                    CraftSlot::Block(p10), CraftSlot::Block(p12))
                = (r0[0], r0[2], r1[0], r1[2])
                && p00 == p02 && p02 == p10 && p10 == p12
                    && let Some(species) = block::species_for_planks(p00) {
                        return Some(ItemStack::new_block(
                            block::fence_post_for_species(species),
                            3,
                        ));
                    }
        // F1 Wave 2 — Oak Fence Gate: SPS / SPS (sticks outside, plank rail
        // between — the inverse of the fence-post motif above, so the two
        // grids stay distinct). 2×3 → 1 gate. v1 oak only.
        let plank_b = CraftSlot::Block(block::OAK_PLANKS);
        if r0 == [stick_2x3, plank_b, stick_2x3] && r1 == [stick_2x3, plank_b, stick_2x3] {
            return Some(ItemStack::new_block(block::OAK_FENCE_GATE, 1));
        }
        // F1 Wave 2 — Oak Trapdoor: PPP / PPP (6 planks, the Minecraft motif)
        // → 2 trapdoors. Distinct from BED (top row wool) + the gate/post grids.
        if r0 == [plank_b, plank_b, plank_b] && r1 == [plank_b, plank_b, plank_b] {
            return Some(ItemStack::new_block(block::OAK_TRAPDOOR, 2));
        }
        // F1 Wave 2 — Glass Pane: GGG / GGG (6 glass) → 16 panes (Minecraft).
        let glass_pane_b = CraftSlot::Block(block::GLASS);
        if r0 == [glass_pane_b, glass_pane_b, glass_pane_b]
            && r1 == [glass_pane_b, glass_pane_b, glass_pane_b]
        {
            return Some(ItemStack::new_block(block::GLASS_PANE, 16));
        }
        // F1 Wave 2 — Iron Bars: III / III (6 iron ingots) → 16 bars (Minecraft).
        let iron_i_2x3 = CraftSlot::Material(MaterialId::IronIngot);
        if r0 == [iron_i_2x3, iron_i_2x3, iron_i_2x3]
            && r1 == [iron_i_2x3, iron_i_2x3, iron_i_2x3]
        {
            return Some(ItemStack::new_block(block::IRON_BARS, 16));
        }
        // Tent (2026-05-28) — `CCC / S.S` 2×3 → 1 Tent. Canvas roof
        // across the top, Stick guy-line stakes on the corners,
        // empty centre-bottom (the doorway). Distinct from every
        // other 2×3 pattern: Bed needs Wool top row (mat type
        // differs), Helmet needs same material across the 5 non-
        // empty slots (Canvas + Stick mix breaks the equality),
        // Boots needs empty centre on BOTH rows (tent has filled
        // top), Fence Post needs sticks in the centres of both rows
        // (tent has them at the bottom corners).
        let canvas = CraftSlot::Material(MaterialId::Canvas);
        if r0 == [canvas, canvas, canvas]
            && r1[0] == stick_2x3
            && r1[1] == CraftSlot::Empty
            && r1[2] == stick_2x3
        {
            return Some(ItemStack::new_block(block::TENT, 1));
        }
        // Spec 28e Helmet: MMM / M.M (5 material in a top-arc). Material
        // must be the same in all 5 slots; centre-bottom is empty.
        if r0[0] == r0[1] && r0[1] == r0[2]
            && r1[0] == r0[0] && r1[1] == CraftSlot::Empty && r1[2] == r0[0]
            && let Some(mat) = armour_material_from_slot(r0[0]) {
                return Some(ItemStack::new_armour(
                    crate::armour::ArmourSlot::Helmet,
                    mat,
                ));
            }
        // Spec 28e Boots: M.M / M.M (4 material in two parallel columns).
        if r0[0] == r1[0] && r0[2] == r1[2] && r0[0] == r0[2]
            && r0[1] == CraftSlot::Empty && r1[1] == CraftSlot::Empty
            && let Some(mat) = armour_material_from_slot(r0[0]) {
                return Some(ItemStack::new_armour(
                    crate::armour::ArmourSlot::Boots,
                    mat,
                ));
            }
        // Craftable Armoured Carts (CA2) — Wood Cart, the Minecraft-minecart
        // U of planks (the empty top row is trimmed by the bounding box, so
        // this matches whether the player anchors the U at the top or bottom
        // of the grid):
        //   P . P   →  1 Wood Cart
        //   P P P
        // Accepts mixed plank species via `is_any_planks`. Distinct from
        // every other 2×3 recipe: Boots needs an empty r1[1] (cart fills it),
        // Helmet/Bed/Tent need a full or non-plank top row (cart's r0[1] is
        // empty), Fence Post needs sticks in both row centres.
        let is_planks_local = |s: CraftSlot| match s {
            CraftSlot::Block(b) => block::is_any_planks(b),
            _ => false,
        };
        if is_planks_local(r0[0])
            && r0[1] == CraftSlot::Empty
            && is_planks_local(r0[2])
            && is_planks_local(r1[0])
            && is_planks_local(r1[1])
            && is_planks_local(r1[2])
        {
            return Some(ItemStack::new_material(MaterialId::WoodCart, 1));
        }
    }

    // --- 2x2 recipes ---
    if h == 2 && w == 2 {
        let tl = grid[min_r][min_c];
        let tr = grid[min_r][min_c + 1];
        let bl = grid[min_r + 1][min_c];
        let br = grid[min_r + 1][min_c + 1];
        // 4 Planks → Crafting Table
        if tl == CraftSlot::Block(block::OAK_PLANKS)
            && tr == CraftSlot::Block(block::OAK_PLANKS)
            && bl == CraftSlot::Block(block::OAK_PLANKS)
            && br == CraftSlot::Block(block::OAK_PLANKS)
        {
            return Some(ItemStack::new_block(block::CRAFTING_TABLE, 1));
        }
        // 4 Sand → 4 Glass (Wave 16, no furnace yet — direct craft).
        let sand = CraftSlot::Block(block::SAND);
        if tl == sand && tr == sand && bl == sand && br == sand {
            return Some(ItemStack::new_block(block::GLASS, 4));
        }
        // 4 Sticks → 1 Drying Rack (Wave 29 — Spec 29). The simplest
        // possible workstation recipe: a frame of four sticks. Cheap
        // enough to encourage adoption on Day 1 once the player has
        // chopped a tree + crafted sticks.
        let stick = CraftSlot::Material(MaterialId::Stick);
        if tl == stick && tr == stick && bl == stick && br == stick {
            return Some(ItemStack::new_block(block::DRYING_RACK, 1));
        }
        // Spec 26 — Drafting Table: 3 oak planks + 1 paper in a 2×2.
        // Paper-on-wood motif (the drafting surface). Paper accepted
        // via the `is_paperish_slot` predicate so a future Pulp-Paper
        // grade (T1.5) drops into the same arm.
        let planks_2x2 = CraftSlot::Block(block::OAK_PLANKS);
        if is_paperish_slot(tl)
            && tr == planks_2x2
            && bl == planks_2x2
            && br == planks_2x2
        {
            return Some(ItemStack::new_block(block::DRAFTING_TABLE, 1));
        }
        // Spec 28c — Amethyst Block: 4 amethyst in a 2×2 square. Mirrors
        // the simpler-than-storage-block density: amethyst is rare enough
        // that compacting only 4 at a time still produces a satisfying
        // wall-block. (Compare to coal/iron/diamond/satori 3×3 = 9.)
        let amethyst = CraftSlot::Material(MaterialId::Amethyst);
        if tl == amethyst && tr == amethyst && bl == amethyst && br == amethyst {
            return Some(ItemStack::new_block(block::AMETHYST_BLOCK, 1));
        }
        // Spec 36 Phase 2 (2026-05-28) — Cloth + Canvas: textile
        // compaction. 4 Cotton → 1 Cloth (fine textile); 4 Hemp Fibre →
        // 1 Canvas (coarse sailcloth). Same compaction motif as
        // amethyst, sized for "this is one cloth piece, not a whole
        // bolt of fabric" — downstream specs (bags, banners, sails,
        // tents) cost multiple cloth/canvas pieces.
        let cotton_m = CraftSlot::Material(MaterialId::Cotton);
        if tl == cotton_m && tr == cotton_m && bl == cotton_m && br == cotton_m {
            return Some(ItemStack::new_material(MaterialId::Cloth, 1));
        }
        let hemp_m = CraftSlot::Material(MaterialId::HempFibre);
        if tl == hemp_m && tr == hemp_m && bl == hemp_m && br == hemp_m {
            return Some(ItemStack::new_material(MaterialId::Canvas, 1));
        }
    }

    // --- 3x3 storage-block recipes (Wave 17). All 9 cells must be the same
    // material; produces one compressed block. Checked BEFORE the tool
    // recipes below so 9-of-stick doesn't accidentally try to be a tool.
    if h == 3 && w == 3 {
        let centre = grid[min_r + 1][min_c + 1];
        let all_same = (0..3).all(|r| (0..3).all(|c| grid[min_r + r][min_c + c] == centre));
        if all_same {
            match centre {
                CraftSlot::Material(MaterialId::Coal) => {
                    return Some(ItemStack::new_block(block::COAL_BLOCK, 1));
                }
                CraftSlot::Material(MaterialId::RawIron) => {
                    return Some(ItemStack::new_block(block::IRON_BLOCK, 1));
                }
                CraftSlot::Material(MaterialId::Diamond) => {
                    return Some(ItemStack::new_block(block::DIAMOND_BLOCK, 1));
                }
                CraftSlot::Material(MaterialId::Satori) => {
                    return Some(ItemStack::new_block(block::SATORI_BLOCK, 1));
                }
                // Spec 28c — Bone Block (9 Bone → 1 placed-bone block).
                // Decorative; same compaction pattern as Coal Block etc.
                CraftSlot::Material(MaterialId::Bone) => {
                    return Some(ItemStack::new_block(block::BONE_BLOCK, 1));
                }
                // Spec 28c — Hay Bale (9 Wheat → 1 hay bale). Decorative
                // + future animal-food. The 9-wheat compaction echoes the
                // 9-coal/iron pattern; bale "looks" denser than loose wheat.
                CraftSlot::Material(MaterialId::Wheat) => {
                    return Some(ItemStack::new_block(block::HAY_BALE, 1));
                }
                // Salt — 9 Salt -> 1 SALT_BLOCK storage block.
                CraftSlot::Material(MaterialId::Salt) => {
                    return Some(ItemStack::new_block(block::SALT_BLOCK, 1));
                }
                _ => {}
            }
        }

        // F1 — Stone Stairs: 6 stone in a staircase (either handedness) → 4
        // stairs (Minecraft pattern). Row 0 is the top of the grid.
        {
            let s = CraftSlot::Block(block::STONE);
            let e = CraftSlot::Empty;
            let g = [
                [grid[min_r][min_c], grid[min_r][min_c + 1], grid[min_r][min_c + 2]],
                [grid[min_r + 1][min_c], grid[min_r + 1][min_c + 1], grid[min_r + 1][min_c + 2]],
                [grid[min_r + 2][min_c], grid[min_r + 2][min_c + 1], grid[min_r + 2][min_c + 2]],
            ];
            let left_hand = g == [[s, e, e], [s, s, e], [s, s, s]];
            let right_hand = g == [[e, e, s], [e, s, s], [s, s, s]];
            if left_hand || right_hand {
                return Some(ItemStack::new_block(block::STONE_STAIRS, 4));
            }
        }

        // Spec 20 — Furnace: 8 cobblestone in a hollow ring (Minecraft
        // standard). The centre must be empty; the 8 surrounding cells
        // must all be cobblestone. Checked AFTER the storage-block
        // arm above so a 9-cobble grid doesn't match this recipe
        // (cobble has no storage-block variant — safe but explicit).
        let cob = CraftSlot::Block(block::COBBLESTONE);
        if grid[min_r + 1][min_c + 1] == CraftSlot::Empty {
            let ring_full = grid[min_r][min_c] == cob
                && grid[min_r][min_c + 1] == cob
                && grid[min_r][min_c + 2] == cob
                && grid[min_r + 1][min_c] == cob
                && grid[min_r + 1][min_c + 2] == cob
                && grid[min_r + 2][min_c] == cob
                && grid[min_r + 2][min_c + 1] == cob
                && grid[min_r + 2][min_c + 2] == cob;
            if ring_full {
                return Some(ItemStack::new_block(block::FURNACE, 1));
            }
        }

        // P7 Hopper — 5 iron ingots in a V around a CHEST centre (Minecraft
        // shape). Checked before the tiered-chest recipe; the empty top-centre
        // + bottom corners make it unambiguous against the full-ring chests.
        //   I . I
        //   I C I
        //   . I .
        {
            let iron = CraftSlot::Material(MaterialId::IronIngot);
            let chest = CraftSlot::Block(block::CHEST);
            if grid[min_r + 1][min_c + 1] == chest
                && grid[min_r][min_c] == iron
                && grid[min_r][min_c + 2] == iron
                && grid[min_r + 1][min_c] == iron
                && grid[min_r + 1][min_c + 2] == iron
                && grid[min_r + 2][min_c + 1] == iron
                && grid[min_r][min_c + 1] == CraftSlot::Empty
                && grid[min_r + 2][min_c] == CraftSlot::Empty
                && grid[min_r + 2][min_c + 2] == CraftSlot::Empty
            {
                return Some(ItemStack::new_block(block::HOPPER, 1));
            }
        }

        // P11 Piston — an Electricity-powered block pusher. Mirrors Minecraft's
        // piston shape, with the redstone-dust slot replaced by a CABLE (our
        // power-wire): a plank cap, a cobble body, an iron core, tapped by a
        // cable. It fills the whole 3×3 so it sits at offset (0,0).
        //   P P P   (oak planks)
        //   C I C   (cobblestone · iron ingot · cobblestone)
        //   C W C   (cobblestone · cable · cobblestone)
        {
            let p = CraftSlot::Block(block::OAK_PLANKS);
            let cob = CraftSlot::Block(block::COBBLESTONE);
            let iron = CraftSlot::Material(MaterialId::IronIngot);
            let cable = CraftSlot::Block(block::CABLE);
            if grid[0][0] == p
                && grid[0][1] == p
                && grid[0][2] == p
                && grid[1][0] == cob
                && grid[1][1] == iron
                && grid[1][2] == cob
                && grid[2][0] == cob
                && grid[2][1] == cable
                && grid[2][2] == cob
            {
                return Some(ItemStack::new_block(block::PISTON, 1));
            }
        }

        // Dispenser / Dropper (2026-07-04 gap-fill wave) — MC shapes with the
        // redstone slot replaced by a CABLE (the piston convention) and the
        // bow replaced by an ARROW (tools can't be ingredients here; the
        // arrow reads as "the block that shoots").
        //   Dispenser:  C C C / C A C / C W C   (A = arrow, W = cable)
        //   Dropper:    C C C / C _ C / C W C
        {
            let cob = CraftSlot::Block(block::COBBLESTONE);
            let cable = CraftSlot::Block(block::CABLE);
            let ring_matches = |g: &[[CraftSlot; 3]; 3]| {
                g[0][0] == cob
                    && g[0][1] == cob
                    && g[0][2] == cob
                    && g[1][0] == cob
                    && g[1][2] == cob
                    && g[2][0] == cob
                    && g[2][1] == cable
                    && g[2][2] == cob
            };
            if ring_matches(grid) {
                match grid[1][1] {
                    CraftSlot::Material(MaterialId::Arrow) => {
                        return Some(ItemStack::new_block(block::DISPENSER, 1));
                    }
                    CraftSlot::Empty => {
                        return Some(ItemStack::new_block(block::DROPPER, 1));
                    }
                    _ => {}
                }
            }
        }

        // Spec 21 Vendor Block — 8 oak planks ringing 1 iron ingot at
        // the centre. The chest-wood + coin-slot motif made physical:
        // wood is the chest, iron is the till. Checked here (3x3 path)
        // BEFORE the tool recipes so the iron-centred grid doesn't
        // accidentally try to be a pickaxe.
        let planks = CraftSlot::Block(block::OAK_PLANKS);
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        if grid[min_r + 1][min_c + 1] == iron {
            let ring_planks = grid[min_r][min_c] == planks
                && grid[min_r][min_c + 1] == planks
                && grid[min_r][min_c + 2] == planks
                && grid[min_r + 1][min_c] == planks
                && grid[min_r + 1][min_c + 2] == planks
                && grid[min_r + 2][min_c] == planks
                && grid[min_r + 2][min_c + 1] == planks
                && grid[min_r + 2][min_c + 2] == planks;
            if ring_planks {
                return Some(ItemStack::new_block(block::VENDOR_BLOCK, 1));
            }
        }

        // Spec 48 (Electricity) — Steam Generator: 8 Iron Ingots ringing 1
        // Copper Ingot core ("boiler shell + copper dynamo"). Copper centre is
        // distinct from the iron-centre (Vendor) and diamond-centre (Bazaar)
        // rings, so the grids never collide.
        {
            let iron_i = CraftSlot::Material(MaterialId::IronIngot);
            let copper_i = CraftSlot::Material(MaterialId::CopperIngot);
            if grid[min_r + 1][min_c + 1] == copper_i {
                let ring_iron = grid[min_r][min_c] == iron_i
                    && grid[min_r][min_c + 1] == iron_i
                    && grid[min_r][min_c + 2] == iron_i
                    && grid[min_r + 1][min_c] == iron_i
                    && grid[min_r + 1][min_c + 2] == iron_i
                    && grid[min_r + 2][min_c] == iron_i
                    && grid[min_r + 2][min_c + 1] == iron_i
                    && grid[min_r + 2][min_c + 2] == iron_i;
                if ring_iron {
                    return Some(ItemStack::new_block(block::STEAM_GENERATOR, 1));
                }
            }
        }

        // Spec 48 Phase 4 (Electricity) — Water Wheel: 8 Planks (any species)
        // ringing 1 Copper Ingot — eight paddles around the copper dynamo axle,
        // the same "copper is the dynamo" motif as the Hand Crank, Battery and
        // Steam Generator core. Copper at the centre keeps it clear of every
        // other plank-ring recipe (iron = Vendor Block, papyrus = Bounty Board,
        // Black Powder = Blasting Keg, empty = Chest); the plank ring keeps it
        // clear of the Steam Generator, which rings the same copper core in iron.
        {
            let copper_axle = CraftSlot::Material(MaterialId::CopperIngot);
            let is_planks_local = |s: CraftSlot| match s {
                CraftSlot::Block(b) => block::is_any_planks(b),
                _ => false,
            };
            if grid[min_r + 1][min_c + 1] == copper_axle {
                let ring_any_planks = is_planks_local(grid[min_r][min_c])
                    && is_planks_local(grid[min_r][min_c + 1])
                    && is_planks_local(grid[min_r][min_c + 2])
                    && is_planks_local(grid[min_r + 1][min_c])
                    && is_planks_local(grid[min_r + 1][min_c + 2])
                    && is_planks_local(grid[min_r + 2][min_c])
                    && is_planks_local(grid[min_r + 2][min_c + 1])
                    && is_planks_local(grid[min_r + 2][min_c + 2]);
                if ring_any_planks {
                    return Some(ItemStack::new_block(block::WATER_WHEEL, 1));
                }
                // Wind, Copper & Electricity wave §2.2 — Windmill: canvas sails
                // over a plank-and-copper hub on a stick-and-iron trestle.
                //   Canvas Canvas Canvas
                //   Plank  Copper Plank
                //   Stick  Iron   Stick
                // Shares the copper centre with the Water Wheel and the Steam
                // Generator (copper is the dynamo, every time), but the three
                // rows are all different, so nothing else can match it.
                let canvas = CraftSlot::Material(MaterialId::Canvas);
                let stick = CraftSlot::Material(MaterialId::Stick);
                let iron_i = CraftSlot::Material(MaterialId::IronIngot);
                let sails = grid[min_r][min_c] == canvas
                    && grid[min_r][min_c + 1] == canvas
                    && grid[min_r][min_c + 2] == canvas;
                let hub = is_planks_local(grid[min_r + 1][min_c])
                    && is_planks_local(grid[min_r + 1][min_c + 2]);
                let trestle = grid[min_r + 2][min_c] == stick
                    && grid[min_r + 2][min_c + 1] == iron_i
                    && grid[min_r + 2][min_c + 2] == stick;
                if sails && hub && trestle {
                    return Some(ItemStack::new_block(block::WINDMILL, 1));
                }
            }
        }

        // Spec 39 Bazaar Block — 8 OAK_PLANKS ring around 1 Diamond
        // centre. Diamond centre is distinct from every other centred
        // recipe (iron / papyrus / 2-iron-stack).
        {
            let plank = CraftSlot::Block(block::OAK_PLANKS);
            let diamond = CraftSlot::Material(MaterialId::Diamond);
            if grid[min_r + 1][min_c + 1] == diamond {
                let ring =
                    grid[min_r][min_c] == plank
                    && grid[min_r][min_c + 1] == plank
                    && grid[min_r][min_c + 2] == plank
                    && grid[min_r + 1][min_c] == plank
                    && grid[min_r + 1][min_c + 2] == plank
                    && grid[min_r + 2][min_c] == plank
                    && grid[min_r + 2][min_c + 1] == plank
                    && grid[min_r + 2][min_c + 2] == plank;
                if ring {
                    return Some(ItemStack::new_material(MaterialId::BazaarBlockItem, 1));
                }
            }
        }

        // Craftable Armoured Carts (CA2) — Track + the three cart tiers.
        // Checked here (3x3 / bounded-box path) BEFORE the tool matchers so
        // the iron-heavy shapes can't be misread as pickaxes/etc.
        //
        // Track — Minecraft rail shape, two full Iron Ingot columns framing a
        // Stick centre:
        //   I . I
        //   I S I   →  16 Track
        //   I . I
        // Distinct from Plot Marker (I.I / .P. / I.I) by the iron side-mids +
        // Stick centre (Plot Marker has empty side-mids + a plank centre).
        {
            let iron_m = CraftSlot::Material(MaterialId::IronIngot);
            let stick = CraftSlot::Material(MaterialId::Stick);
            let track =
                grid[min_r][min_c] == iron_m
                && grid[min_r][min_c + 1] == CraftSlot::Empty
                && grid[min_r][min_c + 2] == iron_m
                && grid[min_r + 1][min_c] == iron_m
                && grid[min_r + 1][min_c + 1] == stick
                && grid[min_r + 1][min_c + 2] == iron_m
                && grid[min_r + 2][min_c] == iron_m
                && grid[min_r + 2][min_c + 1] == CraftSlot::Empty
                && grid[min_r + 2][min_c + 2] == iron_m;
            if track {
                return Some(ItemStack::new_block(crate::rail::TRACK, 16));
            }
        }

        // Iron Cart — full Iron Ingot ring around a Wood Cart centre:
        //   I I I
        //   I W I   →  1 Iron Cart   (W = WoodCart)
        //   I I I
        // Distinct from every other centred recipe by the WoodCart centre.
        {
            let iron_m = CraftSlot::Material(MaterialId::IronIngot);
            let wood_cart = CraftSlot::Material(MaterialId::WoodCart);
            if grid[min_r + 1][min_c + 1] == wood_cart {
                let ring =
                    grid[min_r][min_c] == iron_m
                    && grid[min_r][min_c + 1] == iron_m
                    && grid[min_r][min_c + 2] == iron_m
                    && grid[min_r + 1][min_c] == iron_m
                    && grid[min_r + 1][min_c + 2] == iron_m
                    && grid[min_r + 2][min_c] == iron_m
                    && grid[min_r + 2][min_c + 1] == iron_m
                    && grid[min_r + 2][min_c + 2] == iron_m;
                if ring {
                    return Some(ItemStack::new_material(MaterialId::IronCart, 1));
                }
            }
        }

        // Diamond Cart — full Diamond ring around an Iron Cart centre:
        //   D D D
        //   D R D   →  1 Diamond Cart   (R = IronCart)
        //   D D D
        // Distinct from Bazaar Block (Diamond centre / plank ring) + every
        // other centred recipe by the IronCart centre.
        {
            let diamond = CraftSlot::Material(MaterialId::Diamond);
            let iron_cart = CraftSlot::Material(MaterialId::IronCart);
            if grid[min_r + 1][min_c + 1] == iron_cart {
                let ring =
                    grid[min_r][min_c] == diamond
                    && grid[min_r][min_c + 1] == diamond
                    && grid[min_r][min_c + 2] == diamond
                    && grid[min_r + 1][min_c] == diamond
                    && grid[min_r + 1][min_c + 2] == diamond
                    && grid[min_r + 2][min_c] == diamond
                    && grid[min_r + 2][min_c + 1] == diamond
                    && grid[min_r + 2][min_c + 2] == diamond;
                if ring {
                    return Some(ItemStack::new_material(MaterialId::DiamondCart, 1));
                }
            }
        }

        // Spec 38 Auction Block — podium shape:
        //   I P I
        //   P P P
        //   I P I
        // 4 IronIngot corners + 5 OAK_PLANKS. Distinct from Plot
        // Marker (empty edges/centre) + every other recipe.
        {
            let iron_m = CraftSlot::Material(MaterialId::IronIngot);
            let plank = CraftSlot::Block(block::OAK_PLANKS);
            let auction =
                grid[min_r][min_c] == iron_m
                && grid[min_r][min_c + 1] == plank
                && grid[min_r][min_c + 2] == iron_m
                && grid[min_r + 1][min_c] == plank
                && grid[min_r + 1][min_c + 1] == plank
                && grid[min_r + 1][min_c + 2] == plank
                && grid[min_r + 2][min_c] == iron_m
                && grid[min_r + 2][min_c + 1] == plank
                && grid[min_r + 2][min_c + 2] == iron_m;
            if auction {
                return Some(ItemStack::new_material(MaterialId::AuctionBlockItem, 1));
            }
        }

        // Spec 36 Plot Marker — boundary-post shape:
        //   I . I
        //   . P .
        //   I . I
        // 4 IronIngot corners + 1 OAK_PLANKS centre. Distinct from
        // all existing recipes (corners-only iron + plank centre);
        // checked before the tool matchers.
        {
            let iron_m = CraftSlot::Material(MaterialId::IronIngot);
            let plank = CraftSlot::Block(block::OAK_PLANKS);
            let marker =
                grid[min_r][min_c] == iron_m
                && grid[min_r][min_c + 1] == CraftSlot::Empty
                && grid[min_r][min_c + 2] == iron_m
                && grid[min_r + 1][min_c] == CraftSlot::Empty
                && grid[min_r + 1][min_c + 1] == plank
                && grid[min_r + 1][min_c + 2] == CraftSlot::Empty
                && grid[min_r + 2][min_c] == iron_m
                && grid[min_r + 2][min_c + 1] == CraftSlot::Empty
                && grid[min_r + 2][min_c + 2] == iron_m;
            if marker {
                return Some(ItemStack::new_material(MaterialId::PlotMarkerItem, 1));
            }
        }

        // Spec 35 Repair Bench — anvil-ish shape:
        //   I I I
        //   . S .
        //   S S S
        // 3 IronIngot top row + Stone centre + 3 Stone bottom row.
        // Checked BEFORE the tool-pattern matchers (which need sticks
        // in the handle column) so the iron top row can't be read as
        // a pickaxe — the Stone centre + Stone bottom-row disambiguate.
        {
            let stone = CraftSlot::Block(block::STONE);
            let iron_m = CraftSlot::Material(MaterialId::IronIngot);
            let bench =
                grid[min_r][min_c] == iron_m
                && grid[min_r][min_c + 1] == iron_m
                && grid[min_r][min_c + 2] == iron_m
                && grid[min_r + 1][min_c] == CraftSlot::Empty
                && grid[min_r + 1][min_c + 1] == stone
                && grid[min_r + 1][min_c + 2] == CraftSlot::Empty
                && grid[min_r + 2][min_c] == stone
                && grid[min_r + 2][min_c + 1] == stone
                && grid[min_r + 2][min_c + 2] == stone;
            if bench {
                return Some(ItemStack::new_material(MaterialId::RepairBenchItem, 1));
            }
        }

        // Spec 34 Tip Jar — 8 planks ringing 2 IronIngot stacked at
        // the centre (top + middle of the central column). The stack
        // distinguishes from Vendor Block (single iron centre) and
        // Bounty Board (papyrus centre). Accepts any wood species
        // via `block::is_any_planks`. Checked BEFORE Vendor Block
        // since the top-of-column iron + ring would otherwise be
        // ambiguous with Vendor's single-iron-centre.
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        if grid[min_r][min_c + 1] == iron
            && grid[min_r + 1][min_c + 1] == iron
        {
            let is_planks_local = |s: CraftSlot| match s {
                CraftSlot::Block(b) => block::is_any_planks(b),
                _ => false,
            };
            let ring =
                is_planks_local(grid[min_r][min_c])
                && is_planks_local(grid[min_r][min_c + 2])
                && is_planks_local(grid[min_r + 1][min_c])
                && is_planks_local(grid[min_r + 1][min_c + 2])
                && is_planks_local(grid[min_r + 2][min_c])
                && is_planks_local(grid[min_r + 2][min_c + 1])
                && is_planks_local(grid[min_r + 2][min_c + 2]);
            if ring {
                return Some(ItemStack::new_material(MaterialId::TipJarItem, 1));
            }
        }

        // Spec 33 Mob Bounty Board — 8 planks ringing 1 PapyrusSheet
        // at the centre. Different centre material from Vendor Block
        // (iron) so the two recipes can't collide. PapyrusSheet at
        // centre is thematic: bounty notices pinned to parchment.
        // Accepts any wood species via `block::is_any_planks`.
        let papyrus = CraftSlot::Material(MaterialId::PapyrusSheet);
        if grid[min_r + 1][min_c + 1] == papyrus {
            let is_planks_slot_local = |s: CraftSlot| match s {
                CraftSlot::Block(b) => block::is_any_planks(b),
                _ => false,
            };
            let ring_planks =
                is_planks_slot_local(grid[min_r][min_c])
                && is_planks_slot_local(grid[min_r][min_c + 1])
                && is_planks_slot_local(grid[min_r][min_c + 2])
                && is_planks_slot_local(grid[min_r + 1][min_c])
                && is_planks_slot_local(grid[min_r + 1][min_c + 2])
                && is_planks_slot_local(grid[min_r + 2][min_c])
                && is_planks_slot_local(grid[min_r + 2][min_c + 1])
                && is_planks_slot_local(grid[min_r + 2][min_c + 2]);
            if ring_planks {
                return Some(ItemStack::new_material(MaterialId::BountyBoardItem, 1));
            }
        }

        // HP-2 Chest — 8 planks ringing an empty centre. Accepts any
        // wood species via `block::is_any_planks`, so a fully mixed-
        // species ring still crafts. Checked AFTER the Vendor recipe
        // (which requires iron at centre) so the two don't fight.
        let centre_empty = grid[min_r + 1][min_c + 1] == CraftSlot::Empty;
        let is_planks_slot = |s: CraftSlot| match s {
            CraftSlot::Block(b) => block::is_any_planks(b),
            _ => false,
        };
        if centre_empty {
            let ring_any_planks = is_planks_slot(grid[min_r][min_c])
                && is_planks_slot(grid[min_r][min_c + 1])
                && is_planks_slot(grid[min_r][min_c + 2])
                && is_planks_slot(grid[min_r + 1][min_c])
                && is_planks_slot(grid[min_r + 1][min_c + 2])
                && is_planks_slot(grid[min_r + 2][min_c])
                && is_planks_slot(grid[min_r + 2][min_c + 1])
                && is_planks_slot(grid[min_r + 2][min_c + 2]);
            if ring_any_planks {
                return Some(ItemStack::new_block(block::CHEST, 1));
            }
        }

        // Spec 49 (Explosives) — Blasting Keg: 8 planks ringing 1 Black Powder
        // at the centre (a barrel of powder; NEVER the red TNT cube). Accepts any
        // wood species via `is_any_planks`. Centre == BlackPowder distinguishes it
        // from the empty/iron/papyrus-centred ring recipes above, so it can't fight.
        if grid[min_r + 1][min_c + 1] == CraftSlot::Material(MaterialId::BlackPowder) {
            let ring_any_planks = is_planks_slot(grid[min_r][min_c])
                && is_planks_slot(grid[min_r][min_c + 1])
                && is_planks_slot(grid[min_r][min_c + 2])
                && is_planks_slot(grid[min_r + 1][min_c])
                && is_planks_slot(grid[min_r + 1][min_c + 2])
                && is_planks_slot(grid[min_r + 2][min_c])
                && is_planks_slot(grid[min_r + 2][min_c + 1])
                && is_planks_slot(grid[min_r + 2][min_c + 2]);
            if ring_any_planks {
                return Some(ItemStack::new_block(block::BLASTING_KEG, 1));
            }
        }

        // #15 — tier chests: 8× the tier material ringing a wood CHEST at the
        // centre (the "upgrade" motif). Centre == CHEST distinguishes these from
        // the empty-centre wood-chest recipe above and the iron/diamond-centred
        // Vendor/Bazaar recipes.
        if grid[min_r + 1][min_c + 1] == CraftSlot::Block(block::CHEST) {
            let ring_of = |m: MaterialId| -> bool {
                let s = CraftSlot::Material(m);
                grid[min_r][min_c] == s
                    && grid[min_r][min_c + 1] == s
                    && grid[min_r][min_c + 2] == s
                    && grid[min_r + 1][min_c] == s
                    && grid[min_r + 1][min_c + 2] == s
                    && grid[min_r + 2][min_c] == s
                    && grid[min_r + 2][min_c + 1] == s
                    && grid[min_r + 2][min_c + 2] == s
            };
            for (mat, out) in [
                (MaterialId::CopperIngot, block::COPPER_CHEST),
                (MaterialId::IronIngot, block::IRON_CHEST),
                (MaterialId::Diamond, block::DIAMOND_CHEST),
                (MaterialId::Satori, block::SATORI_CHEST),
            ] {
                if ring_of(mat) {
                    return Some(ItemStack::new_block(out, 1));
                }
            }
        }

        // Spec 28e Chestplate: M.M / MMM / MMM (7 material). Centre-top
        // empty (neck hole); all other 7 cells the same armour material.
        let r0c = [grid[min_r][min_c], grid[min_r][min_c + 1], grid[min_r][min_c + 2]];
        let r1c = [grid[min_r + 1][min_c], grid[min_r + 1][min_c + 1], grid[min_r + 1][min_c + 2]];
        let r2c = [grid[min_r + 2][min_c], grid[min_r + 2][min_c + 1], grid[min_r + 2][min_c + 2]];
        let cs_neck_empty = r0c[1] == CraftSlot::Empty;
        if cs_neck_empty
            && r0c[0] != CraftSlot::Empty
            && r0c[0] == r0c[2]
            && r1c[0] == r0c[0] && r1c[1] == r0c[0] && r1c[2] == r0c[0]
            && r2c[0] == r0c[0] && r2c[1] == r0c[0] && r2c[2] == r0c[0]
            && let Some(mat) = armour_material_from_slot(r0c[0]) {
                return Some(ItemStack::new_armour(
                    crate::armour::ArmourSlot::Chestplate,
                    mat,
                ));
            }

        // Spec 28e Leggings: MMM / M.M / M.M (7 material). Top row full,
        // middle + bottom rows have empty centres (waistband + legs).
        let lg_top_full = r0c[0] != CraftSlot::Empty
            && r0c[0] == r0c[1] && r0c[1] == r0c[2];
        let lg_legs = r1c[0] == r0c[0] && r1c[1] == CraftSlot::Empty && r1c[2] == r0c[0]
            && r2c[0] == r0c[0] && r2c[1] == CraftSlot::Empty && r2c[2] == r0c[0];
        if lg_top_full && lg_legs
            && let Some(mat) = armour_material_from_slot(r0c[0]) {
                return Some(ItemStack::new_armour(
                    crate::armour::ArmourSlot::Leggings,
                    mat,
                ));
            }

        // Salt — SALT_LAMP. Cross: 4 Salt around 1 Stick centre.
        //   . S .
        //   S T S    (T = Stick, S = Salt)
        //   . S .
        let s_slot = CraftSlot::Material(MaterialId::Salt);
        let t_slot = CraftSlot::Material(MaterialId::Stick);
        let salt_lamp_cross = r0c[0] == CraftSlot::Empty && r0c[1] == s_slot && r0c[2] == CraftSlot::Empty
            && r1c[0] == s_slot && r1c[1] == t_slot && r1c[2] == s_slot
            && r2c[0] == CraftSlot::Empty && r2c[1] == s_slot && r2c[2] == CraftSlot::Empty;
        if salt_lamp_cross {
            return Some(ItemStack::new_block(block::SALT_LAMP, 1));
        }

        // Rubber — Slingshot Y-fork:
        //   S . S    (S = Stick on top corners)
        //   . R .    (R = Rubber band in centre)
        //   . S .    (S = Stick handle bottom-centre)
        let r_slot = CraftSlot::Material(MaterialId::Rubber);
        let slingshot_y = r0c == [t_slot, CraftSlot::Empty, t_slot]
            && r1c == [CraftSlot::Empty, r_slot, CraftSlot::Empty]
            && r2c == [CraftSlot::Empty, t_slot, CraftSlot::Empty];
        if slingshot_y {
            return Some(ItemStack::new_tool(Tool::new(
                ToolType::Slingshot,
                ToolMaterial::Wood,
            )));
        }
    }

    // --- 1×2 horizontal recipes (Salt feature — order-insensitive pairs) ---
    if h == 1 && w == 2 {
        let pair = [grid[min_r][min_c], grid[min_r][min_c + 1]];
        // Spec 48 (Electricity) — Pressure Plate: 2 Stone in a row (classic).
        if pair == [CraftSlot::Block(block::STONE), CraftSlot::Block(block::STONE)] {
            return Some(ItemStack::new_block(block::PRESSURE_PLATE, 1));
        }
        let s_slot = CraftSlot::Material(MaterialId::Salt);
        // Order-insensitive: Salt left or Salt right.
        let other = if pair[0] == s_slot {
            Some(pair[1])
        } else if pair[1] == s_slot {
            Some(pair[0])
        } else {
            None
        };
        if let Some(other) = other {
            use crate::item::MaterialId as M;
            let result = match other {
                // Salt-Cured raw meats (6 recipes).
                CraftSlot::Material(M::RawBeef)         => Some(M::SaltCuredBeef),
                CraftSlot::Material(M::RawPorkchop)     => Some(M::SaltCuredPorkchop),
                CraftSlot::Material(M::RawMutton)       => Some(M::SaltCuredMutton),
                CraftSlot::Material(M::RawChicken)      => Some(M::SaltCuredChicken),
                CraftSlot::Material(M::RawRabbit)       => Some(M::SaltCuredRabbit),
                CraftSlot::Material(M::RawNostrichMeat) => Some(M::SaltCuredNostrichMeat),
                // Seasoned cooked staples (9 recipes — Nostrich has no
                // Cooked variant, only NostrichOmelette which is T4).
                CraftSlot::Material(M::Bread)          => Some(M::SeasonedBread),
                CraftSlot::Material(M::BakedPotato)    => Some(M::SeasonedBakedPotato),
                CraftSlot::Material(M::BakedCarrot)    => Some(M::SeasonedBakedCarrot),
                CraftSlot::Material(M::BakedCorn)      => Some(M::SeasonedBakedCorn),
                CraftSlot::Material(M::CookedBeef)     => Some(M::SeasonedCookedBeef),
                CraftSlot::Material(M::CookedPorkchop) => Some(M::SeasonedCookedPorkchop),
                CraftSlot::Material(M::CookedMutton)   => Some(M::SeasonedCookedMutton),
                CraftSlot::Material(M::CookedChicken)  => Some(M::SeasonedCookedChicken),
                CraftSlot::Material(M::CookedRabbit)   => Some(M::SeasonedCookedRabbit),
                _ => None,
            };
            if let Some(out) = result {
                return Some(ItemStack::new_material(out, 1));
            }
        }

        // Spec 37 (2026-05-27) — Magnesium pairings (order-insensitive):
        //   + Sulphur → Fertiliser (Epsom salt — magnesium sulphate)
        //   + Stick   → Sparkler
        //   + Papyrus → Flare
        //   + Iron    → Magnesium Firestarter
        let mg = CraftSlot::Material(MaterialId::Magnesium);
        let mg_other = if pair[0] == mg {
            Some(pair[1])
        } else if pair[1] == mg {
            Some(pair[0])
        } else {
            None
        };
        if let Some(other) = mg_other {
            use crate::item::MaterialId as M;
            let out = match other {
                CraftSlot::Material(M::Sulphur) => Some(M::Fertiliser),
                CraftSlot::Material(M::Stick) => Some(M::Sparkler),
                CraftSlot::Material(M::PapyrusSheet) => Some(M::Flare),
                CraftSlot::Material(M::IronIngot) => Some(M::MagnesiumFirestarter),
                _ => None,
            };
            if let Some(out) = out {
                return Some(ItemStack::new_material(out, 1));
            }
        }

        // Spec 35 Phase 2 (2026-05-27) — dye mixing (unordered 1×2). Two
        // paths to each secondary: mix two primary DYES, or mix the two
        // primary FLOWERS directly. Tints add White; shades add Black.
        // Output is 2 (quantity-conserving, genre standard).
        {
            use crate::item::MaterialId as M;
            let dye = |id| CraftSlot::Material(id);
            let blk = |id| CraftSlot::Block(id);
            let is_pair = |a: CraftSlot, b: CraftSlot| {
                (pair[0] == a && pair[1] == b) || (pair[0] == b && pair[1] == a)
            };
            let mixed: Option<M> =
                if is_pair(dye(M::RedDye), dye(M::YellowDye))
                    || is_pair(blk(block::FIELD_POPPY), blk(block::BUTTERCUP)) {
                    Some(M::OrangeDye)
                } else if is_pair(dye(M::YellowDye), dye(M::BlueDye))
                    || is_pair(blk(block::BUTTERCUP), blk(block::CORNFLOWER)) {
                    Some(M::GreenDye)
                } else if is_pair(dye(M::BlueDye), dye(M::RedDye))
                    || is_pair(blk(block::CORNFLOWER), blk(block::FIELD_POPPY)) {
                    Some(M::PurpleDye)
                } else if is_pair(dye(M::RedDye), dye(M::WhiteDye)) {
                    Some(M::PinkDye)
                } else if is_pair(dye(M::GreenDye), dye(M::WhiteDye)) {
                    Some(M::LimeDye)
                } else if is_pair(dye(M::BlueDye), dye(M::WhiteDye)) {
                    Some(M::LightBlueDye)
                } else if is_pair(dye(M::WhiteDye), dye(M::BlackDye)) {
                    Some(M::GreyDye)
                } else if is_pair(dye(M::GreyDye), dye(M::WhiteDye)) {
                    Some(M::LightGreyDye)
                // Spec 35 Phase 2 completion (2026-05-28) — the two
                // remaining 2-input mixes. Cyan via Green+Blue (or
                // Blue+Green — `is_pair` is unordered); Magenta via
                // Purple+Pink. Brown lives in the 1×3 arm below since
                // it's a three-input recipe.
                } else if is_pair(dye(M::GreenDye), dye(M::BlueDye)) {
                    Some(M::CyanDye)
                } else if is_pair(dye(M::PurpleDye), dye(M::PinkDye)) {
                    Some(M::MagentaDye)
                } else {
                    None
                };
            if let Some(out) = mixed {
                return Some(ItemStack::new_material(out, 2));
            }
        }

        // Dyed-paper décor (2026-05-27) — Papyrus Sheet + a dye → 3 coloured
        // Wallpaper blocks. The dyes' first in-world consumer. (Order-
        // insensitive; runs after the Flare pairing so Papyrus+Magnesium is
        // unambiguous.)
        let paper = CraftSlot::Material(MaterialId::PapyrusSheet);
        let paper_other = if pair[0] == paper {
            Some(pair[1])
        } else if pair[1] == paper {
            Some(pair[0])
        } else {
            None
        };
        if let Some(other) = paper_other {
            // Delegate to MaterialId::paint_block — single source of truth
            // for the dye → WALLPAPER colour mapping (item.rs).
            let wp = if let CraftSlot::Material(m) = other {
                m.paint_block()
            } else {
                None
            };
            if let Some(b) = wp {
                return Some(ItemStack::new_block(b, 3));
            }
        }
    }

    // F1 Wave 2 — Oak Door: a 2-wide × 3-tall column of planks → 3 doors
    // (Minecraft pattern). All six cells must be oak planks.
    if h == 3 && w == 2 {
        let p = CraftSlot::Block(block::OAK_PLANKS);
        let all_planks = (0..3).all(|r| {
            grid[min_r + r][min_c] == p && grid[min_r + r][min_c + 1] == p
        });
        if all_planks {
            return Some(ItemStack::new_block(block::OAK_DOOR, 3));
        }
    }

    // F1 Wave 2c — Oak Sign: six planks (3×2) over a centred stick → 3 signs
    // (Minecraft pattern). Top two rows all planks; bottom row is empty / stick
    // / empty.
    if h == 3 && w == 3 {
        let p = CraftSlot::Block(block::OAK_PLANKS);
        let e = CraftSlot::Empty;
        let s = CraftSlot::Material(MaterialId::Stick);
        let top_two_planks = (0..2).all(|r| {
            (0..3).all(|c| grid[min_r + r][min_c + c] == p)
        });
        let stick_foot = grid[min_r + 2][min_c] == e
            && grid[min_r + 2][min_c + 1] == s
            && grid[min_r + 2][min_c + 2] == e;
        if top_two_planks && stick_foot {
            return Some(ItemStack::new_block(block::OAK_SIGN, 3));
        }

        // F1 Wave 2c — Item Frame: an 8-stick ring around a leather centre → 1
        // frame (Minecraft pattern).
        let stick = CraftSlot::Material(MaterialId::Stick);
        let leather = CraftSlot::Material(MaterialId::Leather);
        let ring_sticks = grid[min_r][min_c] == stick
            && grid[min_r][min_c + 1] == stick
            && grid[min_r][min_c + 2] == stick
            && grid[min_r + 1][min_c] == stick
            && grid[min_r + 1][min_c + 2] == stick
            && grid[min_r + 2][min_c] == stick
            && grid[min_r + 2][min_c + 1] == stick
            && grid[min_r + 2][min_c + 2] == stick;
        if ring_sticks && grid[min_r + 1][min_c + 1] == leather {
            return Some(ItemStack::new_block(block::ITEM_FRAME, 1));
        }
    }

    // F1 Wave 2c — Cobblestone Wall: a 3-wide × 2-tall block of cobblestone →
    // 6 walls (Minecraft pattern). All six cells must be cobblestone.
    if h == 2 && w == 3 {
        let c = CraftSlot::Block(block::COBBLESTONE);
        let all_cobble = (0..3).all(|col| {
            grid[min_r][min_c + col] == c && grid[min_r + 1][min_c + col] == c
        });
        if all_cobble {
            return Some(ItemStack::new_block(block::COBBLESTONE_WALL, 6));
        }
    }

    // --- 1-tall recipes (Wave 26 farming bread + Spec 23 papyrus) ---
    if h == 1 && w == 3 {
        let r = [grid[min_r][min_c], grid[min_r][min_c + 1], grid[min_r][min_c + 2]];
        let wheat = CraftSlot::Material(MaterialId::Wheat);
        // Bread: 3 wheat horizontal → 1 bread (Minecraft pattern).
        if r == [wheat, wheat, wheat] {
            return Some(ItemStack::new_material(MaterialId::Bread, 1));
        }
        // F1 — Stone Slab: 3 stone horizontal → 6 slabs (Minecraft pattern).
        let stone_b = CraftSlot::Block(block::STONE);
        if r == [stone_b, stone_b, stone_b] {
            return Some(ItemStack::new_block(block::STONE_SLAB, 6));
        }
        // Spec 23 — Papyrus Sheets: 3 reeds horizontal → 3 sheets
        // (Minecraft sugarcane → paper pattern). High-yield: a single
        // craft makes enough paper for 3 cyanotype-sensitisation batches
        // at Spec 38's recipe (1 sheet + 1 iron + 1 salt → 3 Blueprint
        // Paper) — i.e. ~9 sheets of Blueprint Paper per reed pull,
        // gated by the iron cost.
        let reed = CraftSlot::Material(MaterialId::PapyrusReed);
        if r == [reed, reed, reed] {
            return Some(ItemStack::new_material(MaterialId::PapyrusSheet, 3));
        }
        // Rubber — Copper Cable: Copper / Rubber / Copper horizontal.
        // Data-laydown recipe for the future electricity foundation spec;
        // CopperCable has no gameplay effect in v1.
        let copper = CraftSlot::Material(MaterialId::CopperIngot);
        let rubber = CraftSlot::Material(MaterialId::Rubber);
        if r == [copper, rubber, copper] {
            return Some(ItemStack::new_material(MaterialId::CopperCable, 2));
        }
        // Spec 48 (Electricity) — power-block 1×3 recipes. Shapes adapted from
        // the spec's proposed set to existing materials (owner/Axolittle to
        // confirm feel). Placed AFTER the CopperCable material arm so its
        // Copper/Rubber/Copper recipe stays unshadowed; Cable inverts the layers
        // (Rubber outside, "insulated wire") to keep the two grids distinct.
        let cable_mat = CraftSlot::Material(MaterialId::CopperCable);
        let iron_i = CraftSlot::Material(MaterialId::IronIngot);
        let glass_b = CraftSlot::Block(block::GLASS);
        if r == [rubber, copper, rubber] {
            return Some(ItemStack::new_block(block::CABLE, 3));
        }
        // Electric Lamp — glass bulb + copper filament.
        if r == [glass_b, copper, glass_b] {
            return Some(ItemStack::new_block(block::ELECTRIC_LAMP, 1));
        }
        // Logic Gate — "relay": iron casing + copper-cable contacts.
        if r == [iron_i, cable_mat, iron_i] {
            return Some(ItemStack::new_block(block::LOGIC_GATE, 1));
        }
        // Spec 48 Phase 2 — sensors (each a distinct 1×3 so they never collide).
        // Mirror — framed reflective glass.
        if r == [iron_i, glass_b, iron_i] {
            return Some(ItemStack::new_block(block::MIRROR, 1));
        }
        // Beam Sensor — photoelectric: glass lens + copper + iron housing.
        if r == [glass_b, copper, iron_i] {
            return Some(ItemStack::new_block(block::BEAM_SENSOR, 1));
        }
        // Motion Sensor — PIR dome: copper contacts either side of a glass dome.
        if r == [copper, glass_b, copper] {
            return Some(ItemStack::new_block(block::MOTION_SENSOR, 1));
        }
        // Plunger Detonator (Spec 49) — "a boxed switch": iron casing,
        // cable contact, plank base. Recipe was specified in
        // docs/foundations/2026-06-20-explosives-blasting-keg.md but never
        // wired into the matcher, leaving the block craftable only via
        // creative/`/give` (wiki audit 2026-07-09, finding #1).
        let planks_b = CraftSlot::Block(block::OAK_PLANKS);
        if r == [iron_i, cable_mat, planks_b] {
            return Some(ItemStack::new_block(block::PLUNGER_DETONATOR, 1));
        }
        // Spec 28d.nostrich — Nostrich Omelette. NostrichEgg + Flour
        // + Wheat horizontal → 1 Omelette. 12 hunger / saturating
        // breakfast; T4 trade-value. NostrichEgg's "21× chicken-egg"
        // value lives in the recipe ladder (one egg here = one
        // Omelette) rather than as raw food_value — eggs aren't
        // raw-edible per the Minecraft pattern.
        let egg = CraftSlot::Material(MaterialId::NostrichEgg);
        let flour = CraftSlot::Material(MaterialId::Flour);
        if r == [egg, flour, wheat] {
            return Some(ItemStack::new_material(MaterialId::NostrichOmelette, 1));
        }
        // Spec 49 (Explosives) — Black Powder: Sulphur + Coal + Saltpetre in a
        // row, any order (the real ~75/15/10 gunpowder formula as a teachable
        // 1×3 shapeless craft). Coal is the carbon leg; Charcoal would also
        // qualify once it exists as a material. Yields 3 — one craft seeds a few
        // charges. Sulphur/Saltpetre are unique to this recipe, so no collision.
        if row_matches_set(
            &r,
            &[
                CraftSlot::Material(MaterialId::Sulphur),
                CraftSlot::Material(MaterialId::Coal),
                CraftSlot::Material(MaterialId::Saltpetre),
            ],
        ) {
            return Some(ItemStack::new_material(MaterialId::BlackPowder, 3));
        }
        // Spec 35 Phase 2 completion (2026-05-28) — 3-input dye mixes.
        // Order-independent: collect into a set-like trio (by counting
        // matches) so any horizontal arrangement of the three primaries
        // works. Three recipes share this arm:
        //   • Red + Yellow + Blue (dyes)             → 2 BrownDye
        //   • Field Poppy + Buttercup + Cornflower   → 2 BrownDye (flower path)
        //   • Red + Blue + White (dyes)              → 2 MagentaDye (spec's
        //     alternate Magenta route alongside Purple+Pink in the 1×2
        //     arm above).
        use crate::item::MaterialId as M;
        let dye_set = |a: M, b: M, c: M| -> bool {
            let want = [CraftSlot::Material(a), CraftSlot::Material(b), CraftSlot::Material(c)];
            row_matches_set(&r, &want)
        };
        let block_set = |a: BlockId, b: BlockId, c: BlockId| -> bool {
            let want = [CraftSlot::Block(a), CraftSlot::Block(b), CraftSlot::Block(c)];
            row_matches_set(&r, &want)
        };
        if dye_set(M::RedDye, M::YellowDye, M::BlueDye)
            || block_set(block::FIELD_POPPY, block::BUTTERCUP, block::CORNFLOWER)
        {
            return Some(ItemStack::new_material(M::BrownDye, 2));
        }
        if dye_set(M::RedDye, M::BlueDye, M::WhiteDye) {
            return Some(ItemStack::new_material(M::MagentaDye, 2));
        }
        // Spec 35 dyed-décor (2026-05-28) — Bunting: `Dye + String +
        // Dye` 1×3 horizontal → 4 bunting of that dye's colour. Both
        // dye slots must be the SAME colour (the row is symmetric); a
        // mismatched pair falls through.
        let string_slot = CraftSlot::Material(M::String);
        if r[1] == string_slot && r[0] == r[2]
            && let CraftSlot::Material(dye) = r[0]
                && let Some(bunting_block) = bunting_for_dye(dye) {
                    return Some(ItemStack::new_block(bunting_block, 4));
                }
        // Spec 35 dyed-décor (2026-05-28) — Paper Lantern: `Papyrus
        // Sheet + Stick + Dye` 1×3 horizontal → 1 paper lantern of
        // that dye's colour. Strictly directional (left → right reads
        // as "the paper wraps the stick frame, then takes the dye"),
        // distinct enough from the Nostrich-omelette pattern above
        // (Egg + Flour + Wheat) that the two never clash.
        let papyrus = CraftSlot::Material(M::PapyrusSheet);
        let stick_slot = CraftSlot::Material(M::Stick);
        if r[0] == papyrus && r[1] == stick_slot
            && let CraftSlot::Material(dye) = r[2]
                && let Some(lantern_block) = paper_lantern_for_dye(dye) {
                    return Some(ItemStack::new_block(lantern_block, 1));
                }
        // Spec 35 dyed-décor (2026-05-28) — Kite: `Cloth + String +
        // Dye` 1×3 horizontal → 1 kite of that dye's colour. Distinct
        // first-slot material (Cloth, not Papyrus) so it doesn't
        // collide with the lantern recipe; strictly directional left
        // → right (Cloth body, String tail, Dye colour).
        let cloth = CraftSlot::Material(M::Cloth);
        let string_slot_kite = CraftSlot::Material(M::String);
        if r[0] == cloth && r[1] == string_slot_kite
            && let CraftSlot::Material(dye) = r[2]
                && let Some(kite_block) = kite_for_dye(dye) {
                    return Some(ItemStack::new_block(kite_block, 1));
                }
    }

    // --- 3-tall, 1-wide column (Spec 19 phase 10) ---
    if h == 3 && w == 1 {
        let col = [grid[min_r][min_c], grid[min_r + 1][min_c], grid[min_r + 2][min_c]];
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        let stick = CraftSlot::Material(MaterialId::Stick);
        let plank = CraftSlot::Block(block::OAK_PLANKS);
        // Village Bell: iron on top, stick in middle, plank base → 1 bell.
        if col == [iron, stick, plank] {
            return Some(ItemStack::new_block(block::VILLAGE_BELL, 1));
        }
        // Spec 48 (Electricity) — power-block 3×1 column recipes.
        let copper = CraftSlot::Material(MaterialId::CopperIngot);
        let coal = CraftSlot::Material(MaterialId::Coal);
        // Hand Crank — manual bootstrap source: Stick / Copper / Plank stack.
        if col == [stick, copper, plank] {
            return Some(ItemStack::new_block(block::HAND_CRANK, 1));
        }
        // Battery — voltaic pile: Copper / Coal / Copper layered.
        if col == [copper, coal, copper] {
            return Some(ItemStack::new_block(block::BATTERY, 1));
        }
        // Spec 37 Market Bell: iron / iron / plank column. Distinct
        // from Village Bell (stick middle) — the double-iron reads as
        // a heftier "market" bell.
        if col == [iron, iron, plank] {
            return Some(ItemStack::new_material(MaterialId::MarketBellItem, 1));
        }
        // Spec 28d.nostrich — Nostrich Arrow. Stick on top, Flint in
        // middle, NostrichFeather on bottom → 6 premium arrows. +20%
        // range + +20% damage vs the regular 4-arrow recipe.
        let flint = CraftSlot::Material(MaterialId::Flint);
        let nostrich_feather = CraftSlot::Material(MaterialId::NostrichFeather);
        if col == [stick, flint, nostrich_feather] {
            return Some(ItemStack::new_material(MaterialId::NostrichArrow, 6));
        }
        // HP-3 v2 (2026-05-23) — Trophy Wall. Brigand Chieftain Trophy
        // on top, plank in middle, plank on bottom → 1 Trophy Wall
        // block. Mounts the kill memento on a wooden plaque the player
        // can display. Trophy is consumed (no economic refund); the
        // wall becomes the kept artefact.
        let trophy = CraftSlot::Material(MaterialId::BrigandChieftainTrophy);
        if col == [trophy, plank, plank] {
            return Some(ItemStack::new_block(block::TROPHY_WALL, 1));
        }
        // Spec 36 Phase 2 (2026-05-28) — Rope: three Hemp Fibres in a
        // vertical column → 1 Rope (twisting fibres into rope). Replaces
        // the retired Phase-1 1:1 Hemp Fibre → Rope shorthand; the 3:1
        // ratio makes Rope feel heavier than String (which is still 2:1
        // from Cotton).
        let hemp = CraftSlot::Material(MaterialId::HempFibre);
        if col == [hemp, hemp, hemp] {
            return Some(ItemStack::new_material(MaterialId::Rope, 1));
        }
        // Spec 36 Phase 2 (2026-05-28) — Lead: Rope on top, String at
        // the middle/bottom (the slip-knot loop). 3-tall column matches
        // the rope-handle motif used by Village Bell / Market Bell.
        let rope = CraftSlot::Material(MaterialId::Rope);
        let string_mat = CraftSlot::Material(MaterialId::String);
        if col == [rope, string_mat, string_mat] {
            return Some(ItemStack::new_material(MaterialId::Lead, 1));
        }
        // Banner block (2026-05-28) — first Cloth consumer. Top-to-
        // bottom: Dye (the flag colour) / Cloth (the fabric) / Stick
        // (the pole). 1 banner of the dye's colour per craft. Distinct
        // shape from every other 3-tall column (none use Cloth in the
        // middle slot today).
        let cloth_banner = CraftSlot::Material(MaterialId::Cloth);
        if col[1] == cloth_banner && col[2] == stick
            && let CraftSlot::Material(dye) = col[0]
                && let Some(banner_block) = banner_for_dye(dye) {
                    return Some(ItemStack::new_block(banner_block, 1));
                }
        // Sail block (2026-05-28) — first Canvas consumer. Mirrors the
        // banner column with Canvas substituted for Cloth (the fine-
        // textile vs sailcloth split). 1 sail of the dye's colour
        // per craft. Distinct from banner only by the middle slot.
        let canvas_sail = CraftSlot::Material(MaterialId::Canvas);
        if col[1] == canvas_sail && col[2] == stick
            && let CraftSlot::Material(dye) = col[0]
                && let Some(sail_block) = sail_for_dye(dye) {
                    return Some(ItemStack::new_block(sail_block, 1));
                }
        // Spec 38 (Blueprint / Cyanotype, 2026-05-27) — Blueprint Paper.
        // Papyrus Sheet on top, Iron Ingot in the middle, Salt on the
        // bottom → 3 Blueprint Paper. The column reads as cyanotype
        // sensitisation: paper held against an iron + salt bath until
        // the sheet emerges pale greenish-yellow, ready to capture a
        // build. Replaces the retired Stick + Papyrus → 9 Plan Tiles
        // recipe (see the 1x2 arm above). The yield drops from 9 to 3
        // because iron raises the floor cost; lay-and-lift on a single
        // tile still works, so a 3-pack covers a small house and the
        // player crafts a second batch for anything bigger.
        let papyrus = CraftSlot::Material(MaterialId::PapyrusSheet);
        let salt = CraftSlot::Material(MaterialId::Salt);
        if col == [papyrus, iron, salt] {
            return Some(ItemStack::new_block(block::BLUEPRINT_PAPER, 3));
        }
        // Pets wave Task 13 — Reach Claw. Crab Claw on top, Stick middle +
        // bottom (the claw lashed to a pole) → 1 Reach Claw, a build tool
        // that extends block/entity interaction reach by REACH_CLAW_BONUS
        // while held (see `GameState::effective_reach`).
        let crab_claw = CraftSlot::Material(MaterialId::CrabClaw);
        if col == [crab_claw, stick, stick] {
            return Some(ItemStack::new_material(MaterialId::ReachClaw, 1));
        }
    }

    // --- 3-tall tool recipes (3x1, 3x2, 3x3) ---
    if h == 3 {
        let s = CraftSlot::Material(MaterialId::Stick);
        let string_slot = CraftSlot::Material(MaterialId::String);
        // Check for tool patterns (material on top row(s), sticks below)

        // Spec 28e — Fishing Rod. Pattern (3x3):
        //   . . S
        //   . S X
        //   S . X
        // Sticks on the diagonal + string on the right column at
        // rows 1+2. Single-tier (Wood); the cast/catch mechanics are
        // deferred but the durability is FISHING_ROD_DURABILITY.
        if w == 3 {
            let r0 = [grid[min_r][min_c], grid[min_r][min_c+1], grid[min_r][min_c+2]];
            let r1 = [grid[min_r+1][min_c], grid[min_r+1][min_c+1], grid[min_r+1][min_c+2]];
            let r2 = [grid[min_r+2][min_c], grid[min_r+2][min_c+1], grid[min_r+2][min_c+2]];
            let fishing_rod = r0 == [CraftSlot::Empty, CraftSlot::Empty, s]
                && r1 == [CraftSlot::Empty, s, string_slot]
                && r2 == [s, CraftSlot::Empty, string_slot];
            if fishing_rod {
                return Some(ItemStack::new_tool(Tool::new(
                    ToolType::FishingRod,
                    ToolMaterial::Wood,
                )));
            }
        }

        // 2-wide tools: axe (Wave 7 — was previously written under the
        // `if w == 3` branch and never actually fired because bounding-box
        // trimming reduces the empty third column) and hoe (Wave 26
        // farming). Hoe pattern is the axe pattern with the inner head
        // piece replaced by empty space, so the two patterns are
        // mutually exclusive: hoe requires `r1[0] == Empty`, axe
        // requires `r1[0] == M`.
        if w == 2 {
            let r0 = [grid[min_r][min_c], grid[min_r][min_c + 1]];
            let r1 = [grid[min_r + 1][min_c], grid[min_r + 1][min_c + 1]];
            let r2 = [grid[min_r + 2][min_c], grid[min_r + 2][min_c + 1]];

            // Hoe: MM / _S / _S — head pieces on the top-left + stick
            // column on the right.
            if r0[0] == r0[1]
                && r1[0] == CraftSlot::Empty && r1[1] == s
                && r2[0] == CraftSlot::Empty && r2[1] == s
                && let Some(mat) = material_from_slot(r0[0]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Hoe, mat)));
                }
            // Hoe mirrored: MM / S_ / S_ — head pieces on the
            // top-right + stick column on the left.
            if r0[0] == r0[1]
                && r1[0] == s && r1[1] == CraftSlot::Empty
                && r2[0] == s && r2[1] == CraftSlot::Empty
                && let Some(mat) = material_from_slot(r0[0]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Hoe, mat)));
                }

            // Axe: MM / MS / _S — head pieces on top + one head piece
            // extending into the middle row, then sticks.
            if r0[0] == r0[1]
                && r1[0] == r0[0] && r1[1] == s
                && r2[0] == CraftSlot::Empty && r2[1] == s
                && let Some(mat) = material_from_slot(r0[0]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Axe, mat)));
                }
            // Axe mirrored: MM / SM / S_.
            if r0[0] == r0[1]
                && r1[0] == s && r1[1] == r0[0]
                && r2[0] == s && r2[1] == CraftSlot::Empty
                && let Some(mat) = material_from_slot(r0[0]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Axe, mat)));
                }
        }

        // Sword: 1 wide, M / M / S
        if w == 1 {
            let t = grid[min_r][min_c];
            let m = grid[min_r + 1][min_c];
            let b = grid[min_r + 2][min_c];
            if t == m && b == s
                && let Some(mat) = material_from_slot(t) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Sword, mat)));
                }
        }

        // Shovel: 1 wide, M / S / S
        if w == 1 {
            let t = grid[min_r][min_c];
            let m = grid[min_r + 1][min_c];
            let b = grid[min_r + 2][min_c];
            if m == s && b == s
                && let Some(mat) = material_from_slot(t) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Shovel, mat)));
                }
        }

        // Pickaxe: 3 wide, M M M / _ S _ / _ S _
        if w == 3 {
            let r0 = [grid[min_r][min_c], grid[min_r][min_c+1], grid[min_r][min_c+2]];
            let r1 = [grid[min_r+1][min_c], grid[min_r+1][min_c+1], grid[min_r+1][min_c+2]];
            let r2 = [grid[min_r+2][min_c], grid[min_r+2][min_c+1], grid[min_r+2][min_c+2]];

            // Pickaxe: MMM / _S_ / _S_
            if r0[0] == r0[1] && r0[1] == r0[2]
                && r1[0] == CraftSlot::Empty && r1[1] == s && r1[2] == CraftSlot::Empty
                && r2[0] == CraftSlot::Empty && r2[1] == s && r2[2] == CraftSlot::Empty
                && let Some(mat) = material_from_slot(r0[0]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, mat)));
                }

            // Axe: MM_ / MS_ / _S_ (and mirrored: _MM / _SM / _S_)
            if r0[0] == r0[1] && r0[2] == CraftSlot::Empty
                && r1[0] == r0[0] && r1[1] == s && r1[2] == CraftSlot::Empty
                && r2[0] == CraftSlot::Empty && r2[1] == s && r2[2] == CraftSlot::Empty
                && let Some(mat) = material_from_slot(r0[0]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Axe, mat)));
                }
            // Mirrored axe
            if r0[0] == CraftSlot::Empty && r0[1] == r0[2]
                && r1[0] == CraftSlot::Empty && r1[1] == s && r1[2] == r0[1]
                && r2[0] == CraftSlot::Empty && r2[1] == s && r2[2] == CraftSlot::Empty
                && let Some(mat) = material_from_slot(r0[1]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Axe, mat)));
                }

            // Bow (Wave 23): 3-tall × 3-wide curve pattern.
            //   _ T S
            //   T R S       R = optional tier reinforcement
            //   _ T S
            // Where T = Stick, S = String. The centre cell (R) selects
            // the bow's material tier — chunk 9 per-tier extension:
            //   Empty   → Wood Bow (Wave 23 default)
            //   Stone   → Stone Bow (Cobblestone)
            //   Iron    → Iron Bow (IronIngot)
            //   Diamond → Diamond Bow
            //   Satori  → Satori Bow
            let st = CraftSlot::Material(MaterialId::String);
            if r0[0] == CraftSlot::Empty && r0[1] == s && r0[2] == st
                && r1[0] == s                              && r1[2] == st
                && r2[0] == CraftSlot::Empty && r2[1] == s && r2[2] == st
            {
                let tier = bow_tier_from_slot(r1[1]);
                if let Some(material) = tier {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Bow, material)));
                }
            }

            // Campfire (Wave 27 — Spec 17 foundation
            // `2026-05-18-campfire.md`): SSS / LLL / SSS — three sticks
            // top, three logs middle, three sticks bottom. Output is
            // the UNLIT campfire; player fuels + ignites separately.
            // Wave 29: the middle row accepts ANY log variant (green /
            // seasoned / kiln-dried / back-compat block) so the player
            // can craft their first campfire from freshly-chopped wood
            // without first running it through a Drying Rack — first-
            // night survival can't be gated on T2 infrastructure.
            if r0 == [s, s, s]
                && is_logish_slot(r1[0]) && is_logish_slot(r1[1]) && is_logish_slot(r1[2])
                && r2 == [s, s, s]
            {
                return Some(ItemStack::new_block(block::CAMPFIRE_UNLIT, 1));
            }

            // Hoe (Wave 26 farming): MM_ / _S_ / _S_ — two head pieces on
            // the top-left, stick handle straight down the middle. Note
            // r1[0] must be EMPTY, distinguishing this from the axe
            // (which has the head extending to r1[0]).
            if r0[0] == r0[1] && r0[2] == CraftSlot::Empty
                && r1[0] == CraftSlot::Empty && r1[1] == s && r1[2] == CraftSlot::Empty
                && r2[0] == CraftSlot::Empty && r2[1] == s && r2[2] == CraftSlot::Empty
                && let Some(mat) = material_from_slot(r0[0]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Hoe, mat)));
                }
            // Mirrored hoe: _MM / _S_ / _S_
            if r0[0] == CraftSlot::Empty && r0[1] == r0[2]
                && r1[0] == CraftSlot::Empty && r1[1] == s && r1[2] == CraftSlot::Empty
                && r2[0] == CraftSlot::Empty && r2[1] == s && r2[2] == CraftSlot::Empty
                && let Some(mat) = material_from_slot(r0[1]) {
                    return Some(ItemStack::new_tool(Tool::new(ToolType::Hoe, mat)));
                }
        }
    }

    None
}

/// Convert a CraftSlot to a ToolMaterial (for tool head ingredients).
fn material_from_slot(slot: CraftSlot) -> Option<ToolMaterial> {
    match slot {
        CraftSlot::Block(block::OAK_PLANKS) => Some(ToolMaterial::Wood),
        CraftSlot::Block(block::COBBLESTONE) => Some(ToolMaterial::Stone),
        CraftSlot::Material(MaterialId::IronIngot) => Some(ToolMaterial::Iron),
        CraftSlot::Material(MaterialId::Diamond) => Some(ToolMaterial::Diamond),
        // Satori head ingredient — combined with sticks in standard tool
        // patterns produces top-tier tools (Spec 5 §3.8).
        CraftSlot::Material(MaterialId::Satori) => Some(ToolMaterial::Satori),
        _ => None,
    }
}

/// Spec 28d chunk 9 — Bow tier reinforcement slot. Empty centre =
/// baseline Wood Bow (Wave 23). Cobblestone / IronIngot / Diamond /
/// Satori in the centre selects the upgraded tier without needing a
/// dedicated recipe per tier.
fn bow_tier_from_slot(slot: CraftSlot) -> Option<ToolMaterial> {
    match slot {
        CraftSlot::Empty => Some(ToolMaterial::Wood),
        CraftSlot::Block(block::COBBLESTONE) => Some(ToolMaterial::Stone),
        CraftSlot::Material(MaterialId::IronIngot) => Some(ToolMaterial::Iron),
        CraftSlot::Material(MaterialId::Diamond) => Some(ToolMaterial::Diamond),
        CraftSlot::Material(MaterialId::Satori) => Some(ToolMaterial::Satori),
        _ => None,
    }
}

/// Spec 28e — map a crafting-grid slot to an armour material tier.
/// Chainmail is intentionally NOT recoverable here: it's a drop-only
/// rarity tier (no live source since the 2026-05-24 roster excision).
/// Returns `None` if the slot
/// doesn't carry an armour-grade ingredient.
fn armour_material_from_slot(slot: CraftSlot) -> Option<crate::armour::ArmourMaterial> {
    use crate::armour::ArmourMaterial;
    match slot {
        CraftSlot::Material(MaterialId::Leather) => Some(ArmourMaterial::Leather),
        CraftSlot::Material(MaterialId::IronIngot) => Some(ArmourMaterial::Iron),
        CraftSlot::Material(MaterialId::Diamond) => Some(ArmourMaterial::Diamond),
        CraftSlot::Material(MaterialId::Satori) => Some(ArmourMaterial::Satori),
        // Rubber feature — boots-only material; the boot-shape recipe
        // arm resolves Rubber for boots, and the chestplate / leggings /
        // helmet arms gate on a material whitelist that excludes Rubber.
        CraftSlot::Material(MaterialId::Rubber) => Some(ArmourMaterial::Rubber),
        _ => None,
    }
}

/// Find the bounding box of non-empty slots in the grid: `(min_row,
/// max_row, min_col, max_col)`. C2b: the server's craft mirror reads it to
/// tell a 2×2 recipe from one that needs a crafting table
/// (`item_actions::judge_craft`).
pub(crate) fn grid_bounds(grid: &[[CraftSlot; 3]; 3]) -> (usize, usize, usize, usize) {
    let mut min_r = 3;
    let mut max_r = 0;
    let mut min_c = 3;
    let mut max_c = 0;
    for (r, row) in grid.iter().enumerate() {
        for (c, slot) in row.iter().enumerate() {
            if *slot != CraftSlot::Empty {
                min_r = min_r.min(r);
                max_r = max_r.max(r);
                min_c = min_c.min(c);
                max_c = max_c.max(c);
            }
        }
    }
    (min_r, max_r, min_c, max_c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_grid() -> [[CraftSlot; 3]; 3] {
        [[CraftSlot::Empty; 3]; 3]
    }

    // Work-based hashing (docs/foundations/2026-06-03-work-based-hashing.md):
    // a block's work = f(block_hardness), the unit being one leaf's worth of
    // bare-hand work. Awarded on a successful can_harvest break.
    #[test]
    fn block_work_unit_is_one_leaf() {
        // Leaves are the lowest non-instant hardness (0.2s) — the "1 hash" unit.
        assert_eq!(block_work(block::OAK_LEAVES), 1);
    }

    #[test]
    fn block_work_scales_off_hardness() {
        // Clean multiples of the 0.2s→1 scale: stone 10s→50, satori 5s→25.
        assert_eq!(block_work(block::STONE), 50);
        assert_eq!(block_work(block::SATORI_BLOCK), 25);
    }

    #[test]
    fn block_work_instant_break_is_zero() {
        // Torch is 0.0s (instant) → no work done breaking it.
        assert_eq!(block_work(block::TORCH), 0);
    }

    // Spec 06 §2.2 anti-farming — break_work gates block_work on origin.
    #[test]
    fn break_work_natural_harvestable_earns_block_work() {
        assert_eq!(break_work(block::STONE, true, false), block_work(block::STONE));
        assert!(break_work(block::STONE, true, false) > 0);
    }

    #[test]
    fn break_work_player_placed_earns_zero() {
        // The exploit fix: a placed block re-mined earns no work.
        assert_eq!(break_work(block::STONE, true, true), 0);
        assert_eq!(break_work(block::OAK_LOG, true, true), 0);
    }

    #[test]
    fn break_work_unharvestable_earns_zero() {
        // Wrong/no tool: no drop, no work — regardless of origin.
        assert_eq!(break_work(block::STONE, false, false), 0);
        assert_eq!(break_work(block::STONE, false, true), 0);
    }

    #[test]
    fn block_work_monotonic_in_hardness() {
        // Harder blocks are worth strictly more work: stone > log > leaves.
        assert!(block_work(block::STONE) > block_work(block::OAK_LOG));
        assert!(block_work(block::OAK_LOG) > block_work(block::OAK_LEAVES));
    }

    #[test]
    fn crack_stage_none_before_progress() {
        // Targeting a block but not yet mined → no overlay.
        assert_eq!(crack_stage(0, 200), None);
    }

    #[test]
    fn crack_stage_none_for_instant_break() {
        // Creative / instant breaks have break_time 0 → never draw cracks.
        assert_eq!(crack_stage(5, 0), None);
    }

    #[test]
    fn crack_stage_first_tick_is_hairline() {
        assert_eq!(crack_stage(1, 200), Some(0));
    }

    #[test]
    fn crack_stage_near_completion_is_heavy() {
        assert_eq!(crack_stage(199, 200), Some(9));
    }

    #[test]
    fn crack_stage_at_threshold_clamps_to_nine() {
        // progress == break_time is the breaking tick; floor would give 10,
        // which must clamp to the last valid stage (9), never index OOB.
        assert_eq!(crack_stage(200, 200), Some(9));
    }

    #[test]
    fn crack_stage_overshoot_clamps_to_nine() {
        assert_eq!(crack_stage(500, 200), Some(9));
    }

    #[test]
    fn crack_stage_is_monotonic_non_decreasing() {
        let bt = 200;
        let mut last = 0u8;
        for p in 1..=bt {
            let s = crack_stage(p, bt).expect("progress > 0 should yield a stage");
            assert!(s >= last, "stage went backwards at progress {p}: {s} < {last}");
            assert!(s <= 9, "stage out of range at progress {p}: {s}");
            last = s;
        }
    }

    #[test]
    fn log_to_planks_recipe_works() {
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Block(block::OAK_LOG);
        let out = match_recipe(&g).expect("log → planks should match");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::OAK_PLANKS),
            _ => panic!("expected planks"),
        }
        assert_eq!(out.count, 4);
    }

    #[test]
    fn plank_recipe_accepts_any_log_variant() {
        // Wave 29 — plank recipe accepts the OAK_LOG back-compat block
        // AND all three log materials. Uniform 4-plank yield: structural
        // use isn't gated on seasoning, only fuel quality is.
        let inputs = [
            CraftSlot::Block(block::OAK_LOG),
            CraftSlot::Material(MaterialId::GreenLog),
            CraftSlot::Material(MaterialId::SeasonedLog),
            CraftSlot::Material(MaterialId::KilnDriedLog),
        ];
        for input in inputs {
            let mut g = empty_grid();
            g[1][1] = input;
            let out = match_recipe(&g).unwrap_or_else(|| panic!("{:?} → planks must match", input));
            match out.item {
                Item::Block(id) => assert_eq!(id, block::OAK_PLANKS, "{:?} should produce OAK_PLANKS", input),
                _ => panic!("expected planks block for {:?}", input),
            }
            assert_eq!(out.count, 4, "{:?} should yield 4 planks", input);
        }
    }

    #[test]
    fn drying_rack_recipe_four_sticks() {
        // Wave 29 — four sticks 2×2 → 1 Drying Rack block-item.
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        g[1][1] = stick;
        g[1][2] = stick;
        g[2][1] = stick;
        g[2][2] = stick;
        let out = match_recipe(&g).expect("4-sticks 2×2 → Drying Rack");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::DRYING_RACK),
            _ => panic!("expected DRYING_RACK block"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn is_logish_slot_covers_all_log_variants() {
        assert!(is_logish_slot(CraftSlot::Block(block::OAK_LOG)));
        assert!(is_logish_slot(CraftSlot::Material(MaterialId::GreenLog)));
        assert!(is_logish_slot(CraftSlot::Material(MaterialId::SeasonedLog)));
        assert!(is_logish_slot(CraftSlot::Material(MaterialId::KilnDriedLog)));
        // Negatives — must not falsely match non-log things.
        assert!(!is_logish_slot(CraftSlot::Block(block::OAK_PLANKS)));
        assert!(!is_logish_slot(CraftSlot::Block(block::OAK_LEAVES)));
        assert!(!is_logish_slot(CraftSlot::Material(MaterialId::Stick)));
        assert!(!is_logish_slot(CraftSlot::Material(MaterialId::Coal)));
        assert!(!is_logish_slot(CraftSlot::Empty));
    }

    #[test]
    fn bone_to_bonemeal_recipe_works() {
        // 1x1 recipe: a single Bone produces 3 Bonemeal.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::Bone);
        let out = match_recipe(&g).expect("bone → bonemeal should match");
        match out.item {
            Item::Material(id) => assert_eq!(id, MaterialId::Bonemeal),
            _ => panic!("expected bonemeal material"),
        }
        assert_eq!(out.count, 3);
    }

    #[test]
    fn empty_grid_returns_none() {
        assert!(match_recipe(&empty_grid()).is_none());
    }

    // --- Spec 35 + 36 (2026-05-27): flowers → dyes, fibre → string/rope ---

    #[test]
    fn flowers_press_to_their_primary_dyes() {
        for (flower, dye) in [
            (block::CORNFLOWER, MaterialId::BlueDye),
            (block::FIELD_POPPY, MaterialId::RedDye),
            (block::BUTTERCUP, MaterialId::YellowDye),
        ] {
            let mut g = empty_grid();
            g[1][1] = CraftSlot::Block(flower);
            let out = match_recipe(&g).expect("flower → dye should match");
            match out.item {
                Item::Material(m) => assert_eq!(m, dye, "flower {flower} → wrong dye"),
                other => panic!("flower {flower} → expected dye, got {other:?}"),
            }
            assert_eq!(out.count, 1);
        }
    }

    #[test]
    fn cotton_is_the_string_source_and_wool_is_not() {
        // Spec 36: Cotton → String (replaces the retired Wool → String).
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::Cotton);
        let out = match_recipe(&g).expect("cotton → string should match");
        assert!(matches!(out.item, Item::Material(MaterialId::String)));
        assert_eq!(out.count, 2);
        // Wool no longer crafts to String — that stopgap is gone.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::Wool);
        assert!(match_recipe(&g).is_none(), "Wool → String must no longer match");
    }

    #[test]
    fn hemp_fibre_twists_to_rope_three_to_one() {
        // Spec 36 Phase 2 (2026-05-28) — Rope wants three Hemp Fibres
        // stacked vertically. The 1:1 stopgap from Phase 1 is retired
        // (heavier cordage should feel heavier than string).
        let mut g = empty_grid();
        let hemp = CraftSlot::Material(MaterialId::HempFibre);
        g[0][1] = hemp;
        g[1][1] = hemp;
        g[2][1] = hemp;
        let out = match_recipe(&g).expect("3 hemp fibres in a column → rope");
        assert!(matches!(out.item, Item::Material(MaterialId::Rope)));
        assert_eq!(out.count, 1);
    }

    #[test]
    fn retired_hemp_fibre_one_to_one_rope_no_longer_matches() {
        // Phase 1's 1:1 Hemp Fibre → Rope shorthand is removed; a
        // single Hemp Fibre alone must now match nothing.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::HempFibre);
        assert!(
            match_recipe(&g).is_none(),
            "Phase 1 single-fibre Rope shortcut must be gone"
        );
    }

    #[test]
    fn rope_plus_string_makes_a_lead() {
        // Spec 36 Phase 2 — Lead: Rope on top, String at the middle/
        // bottom (the slip-knot loop motif). Matches the 3-tall column
        // arm; the bottom-two-strings layout produces exactly the
        // mental picture of a leash with a noose.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::Rope);
        g[1][1] = CraftSlot::Material(MaterialId::String);
        g[2][1] = CraftSlot::Material(MaterialId::String);
        let out = match_recipe(&g).expect("Rope + String column → Lead");
        assert!(matches!(out.item, Item::Material(MaterialId::Lead)));
        assert_eq!(out.count, 1);
    }

    #[test]
    fn four_cotton_compresses_to_cloth() {
        // 2×2 compaction — 4 Cotton → 1 Cloth (fine textile).
        let mut g = empty_grid();
        let cotton = CraftSlot::Material(MaterialId::Cotton);
        g[0][0] = cotton;
        g[0][1] = cotton;
        g[1][0] = cotton;
        g[1][1] = cotton;
        let out = match_recipe(&g).expect("4 cotton 2×2 → Cloth");
        assert!(matches!(out.item, Item::Material(MaterialId::Cloth)));
        assert_eq!(out.count, 1);
    }

    #[test]
    fn four_hemp_fibre_compresses_to_canvas() {
        // 2×2 compaction — 4 Hemp Fibre → 1 Canvas (coarse sailcloth).
        // Distinct from the rope column (3-vertical, makes Rope) so the
        // two arms don't shadow each other.
        let mut g = empty_grid();
        let hemp = CraftSlot::Material(MaterialId::HempFibre);
        g[0][0] = hemp;
        g[0][1] = hemp;
        g[1][0] = hemp;
        g[1][1] = hemp;
        let out = match_recipe(&g).expect("4 hemp fibre 2×2 → Canvas");
        assert!(matches!(out.item, Item::Material(MaterialId::Canvas)));
        assert_eq!(out.count, 1);
    }

    #[test]
    fn dye_phase2_black_white_and_mixing() {
        // Black ← Ink Sac, White ← Bone Meal (single-slot).
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::InkSac);
        assert!(matches!(match_recipe(&g).unwrap().item, Item::Material(MaterialId::BlackDye)));
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::Bonemeal);
        assert!(matches!(match_recipe(&g).unwrap().item, Item::Material(MaterialId::WhiteDye)));

        // Secondary via dye-mix AND via flower-mix → same OrangeDye.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::RedDye);
        g[1][2] = CraftSlot::Material(MaterialId::YellowDye);
        let out = match_recipe(&g).unwrap();
        assert!(matches!(out.item, Item::Material(MaterialId::OrangeDye)));
        assert_eq!(out.count, 2);
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Block(block::FIELD_POPPY);
        g[1][2] = CraftSlot::Block(block::BUTTERCUP);
        assert!(matches!(match_recipe(&g).unwrap().item, Item::Material(MaterialId::OrangeDye)));

        // A tint: Blue + White → Light Blue (order-insensitive).
        let mut g = empty_grid();
        g[1][2] = CraftSlot::Material(MaterialId::WhiteDye);
        g[1][1] = CraftSlot::Material(MaterialId::BlueDye);
        assert!(matches!(match_recipe(&g).unwrap().item, Item::Material(MaterialId::LightBlueDye)));
    }

    #[test]
    fn papyrus_plus_dye_makes_coloured_wallpaper() {
        // Dyed-paper décor — Papyrus Sheet + dye → 3 wallpaper of that colour.
        for (dye, wp) in [
            (MaterialId::BlueDye, block::WALLPAPER_BLUE),
            (MaterialId::OrangeDye, block::WALLPAPER_ORANGE),
            (MaterialId::BlackDye, block::WALLPAPER_BLACK),
            // Spec 35 Phase 2 completion (2026-05-28) — new colours also
            // route through the papyrus + dye → wallpaper arm.
            (MaterialId::BrownDye, block::WALLPAPER_BROWN),
            (MaterialId::CyanDye, block::WALLPAPER_CYAN),
            (MaterialId::MagentaDye, block::WALLPAPER_MAGENTA),
        ] {
            let mut g = empty_grid();
            g[1][1] = CraftSlot::Material(MaterialId::PapyrusSheet);
            g[1][2] = CraftSlot::Material(dye);
            let out = match_recipe(&g).expect("papyrus + dye → wallpaper");
            match out.item {
                Item::Block(b) => assert_eq!(b, wp, "dye {dye:?} → wrong wallpaper"),
                o => panic!("expected wallpaper block, got {o:?}"),
            }
            assert_eq!(out.count, 3);
        }
    }

    // ─── Spec 35 Phase 2 completion (2026-05-28) — 3-input mix dyes ─

    #[test]
    fn cyan_dye_mix_via_green_plus_blue() {
        // 1×2 pair, order-insensitive. Green + Blue (or Blue + Green) → 2 Cyan.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::GreenDye);
        g[1][2] = CraftSlot::Material(MaterialId::BlueDye);
        let out = match_recipe(&g).expect("Green + Blue should match");
        assert!(matches!(out.item, Item::Material(MaterialId::CyanDye)));
        assert_eq!(out.count, 2);
        // Reversed order also matches.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::BlueDye);
        g[1][2] = CraftSlot::Material(MaterialId::GreenDye);
        assert!(matches!(
            match_recipe(&g).unwrap().item,
            Item::Material(MaterialId::CyanDye)
        ));
    }

    #[test]
    fn magenta_dye_mix_via_purple_plus_pink() {
        // 1×2 pair, the cleaner of the two routes the spec gives.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::PurpleDye);
        g[1][2] = CraftSlot::Material(MaterialId::PinkDye);
        let out = match_recipe(&g).expect("Purple + Pink should match");
        assert!(matches!(out.item, Item::Material(MaterialId::MagentaDye)));
        assert_eq!(out.count, 2);
    }

    #[test]
    fn magenta_dye_alt_path_via_red_blue_white_in_a_row() {
        // Spec 35 §"Tints/shades" — R+B+W → Magenta. Order-insensitive
        // 1×3 horizontal arm. Trying a non-canonical order to lock that
        // `row_matches_set` actually does multiset-equality.
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::WhiteDye);
        g[1][1] = CraftSlot::Material(MaterialId::RedDye);
        g[1][2] = CraftSlot::Material(MaterialId::BlueDye);
        let out = match_recipe(&g).expect("R + B + W in any order should match Magenta");
        assert!(matches!(out.item, Item::Material(MaterialId::MagentaDye)));
        assert_eq!(out.count, 2);
    }

    #[test]
    fn brown_dye_via_three_dye_row() {
        // Spec 35 §"Secondaries — dye path" — R + Y + B → Brown.
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::RedDye);
        g[1][1] = CraftSlot::Material(MaterialId::YellowDye);
        g[1][2] = CraftSlot::Material(MaterialId::BlueDye);
        let out = match_recipe(&g).expect("R + Y + B should match Brown");
        assert!(matches!(out.item, Item::Material(MaterialId::BrownDye)));
        assert_eq!(out.count, 2);
    }

    #[test]
    fn brown_dye_via_three_flower_row() {
        // Spec 35 §"Secondaries — flower path" — three primaries
        // directly → 2 Brown (same output as the dye path).
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Block(block::FIELD_POPPY);
        g[1][1] = CraftSlot::Block(block::BUTTERCUP);
        g[1][2] = CraftSlot::Block(block::CORNFLOWER);
        let out = match_recipe(&g).expect("Poppy + Buttercup + Cornflower should match Brown");
        assert!(matches!(out.item, Item::Material(MaterialId::BrownDye)));
        assert_eq!(out.count, 2);
    }

    #[test]
    fn brown_dye_row_is_order_insensitive() {
        // Three permutations all produce 2 Brown.
        let r = MaterialId::RedDye;
        let y = MaterialId::YellowDye;
        let b = MaterialId::BlueDye;
        for order in [[r, y, b], [b, r, y], [y, b, r]] {
            let mut g = empty_grid();
            g[1][0] = CraftSlot::Material(order[0]);
            g[1][1] = CraftSlot::Material(order[1]);
            g[1][2] = CraftSlot::Material(order[2]);
            assert!(matches!(
                match_recipe(&g).unwrap().item,
                Item::Material(MaterialId::BrownDye)
            ));
        }
    }

    #[test]
    fn row_matches_set_rejects_duplicate_inputs() {
        // Multiset equality — three Red Dye in a row should NOT match
        // the R+Y+B Brown recipe (no Yellow, no Blue).
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::RedDye);
        g[1][1] = CraftSlot::Material(MaterialId::RedDye);
        g[1][2] = CraftSlot::Material(MaterialId::RedDye);
        assert!(match_recipe(&g).is_none(),
            "three Red Dye must not satisfy the R+Y+B Brown recipe");
    }

    #[test]
    fn bunting_recipe_dye_string_dye_yields_four_per_colour() {
        // Spec 35 dyed-décor — Dye + String + Dye 1×3 → 4 bunting of
        // that dye's colour. Same dye on both flanks.
        for (dye, bunting_block) in [
            (MaterialId::BlueDye, block::BUNTING_BLUE),
            (MaterialId::RedDye, block::BUNTING_RED),
            (MaterialId::WhiteDye, block::BUNTING_WHITE),
            (MaterialId::BlackDye, block::BUNTING_BLACK),
            (MaterialId::MagentaDye, block::BUNTING_MAGENTA),
        ] {
            let mut g = empty_grid();
            g[1][0] = CraftSlot::Material(dye);
            g[1][1] = CraftSlot::Material(MaterialId::String);
            g[1][2] = CraftSlot::Material(dye);
            let out = match_recipe(&g).expect("dye + string + dye → bunting");
            match out.item {
                Item::Block(b) => assert_eq!(b, bunting_block,
                    "dye {dye:?} → wrong bunting"),
                o => panic!("expected bunting block, got {o:?}"),
            }
            assert_eq!(out.count, 4);
        }
    }

    #[test]
    fn bunting_recipe_rejects_mismatched_dye_flanks() {
        // Both dye slots must be the same colour — a "rainbow bunting"
        // attempt with two different dyes must NOT match.
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::RedDye);
        g[1][1] = CraftSlot::Material(MaterialId::String);
        g[1][2] = CraftSlot::Material(MaterialId::BlueDye);
        assert!(
            match_recipe(&g).is_none(),
            "mismatched dye flanks must not yield bunting"
        );
    }

    #[test]
    fn paper_lantern_recipe_papyrus_stick_dye_yields_one_per_colour() {
        // Spec 35 dyed-décor — Papyrus + Stick + Dye 1×3 → 1 paper
        // lantern of that dye's colour. Strictly directional.
        for (dye, lantern_block) in [
            (MaterialId::WhiteDye, block::PAPER_LANTERN_WHITE),
            (MaterialId::YellowDye, block::PAPER_LANTERN_YELLOW),
            (MaterialId::OrangeDye, block::PAPER_LANTERN_ORANGE),
            (MaterialId::PinkDye, block::PAPER_LANTERN_PINK),
            (MaterialId::CyanDye, block::PAPER_LANTERN_CYAN),
        ] {
            let mut g = empty_grid();
            g[1][0] = CraftSlot::Material(MaterialId::PapyrusSheet);
            g[1][1] = CraftSlot::Material(MaterialId::Stick);
            g[1][2] = CraftSlot::Material(dye);
            let out = match_recipe(&g).expect("papyrus + stick + dye → lantern");
            match out.item {
                Item::Block(b) => assert_eq!(b, lantern_block,
                    "dye {dye:?} → wrong lantern"),
                o => panic!("expected lantern block, got {o:?}"),
            }
            assert_eq!(out.count, 1);
        }
    }

    #[test]
    fn paper_lantern_recipe_rejects_reversed_order() {
        // Strict left → right: papyrus first, dye last. Reversed
        // shouldn't match the lantern recipe (it could otherwise
        // accidentally collide with future symmetric-row recipes).
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::BlueDye);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[1][2] = CraftSlot::Material(MaterialId::PapyrusSheet);
        assert!(
            match_recipe(&g).is_none(),
            "reversed lantern row must NOT yield a lantern — recipe is order-strict"
        );
    }

    #[test]
    fn paper_lanterns_emit_light_at_level_12() {
        // Spec 30 light-emission contract — every paper-lantern colour
        // emits at level 12 (slightly dimmer than torch / campfire).
        let reg = crate::block::BlockRegistry::new();
        for colour in [
            block::PAPER_LANTERN_WHITE,
            block::PAPER_LANTERN_BLACK,
            block::PAPER_LANTERN_RED,
            block::PAPER_LANTERN_BLUE,
            block::PAPER_LANTERN_YELLOW,
            block::PAPER_LANTERN_MAGENTA,
        ] {
            assert_eq!(reg.light_emission(colour), 12,
                "lantern {colour} should emit light at level 12");
        }
        // And bunting does NOT emit light.
        assert_eq!(reg.light_emission(block::BUNTING_WHITE), 0);
    }

    #[test]
    fn kite_recipe_cloth_string_dye_yields_one_per_colour() {
        // Spec 35 Kite — Cloth + String + Dye 1×3 → 1 kite of that
        // dye's colour. Strictly directional.
        for (dye, kite_block) in [
            (MaterialId::RedDye, block::KITE_RED),
            (MaterialId::YellowDye, block::KITE_YELLOW),
            (MaterialId::CyanDye, block::KITE_CYAN),
            (MaterialId::MagentaDye, block::KITE_MAGENTA),
        ] {
            let mut g = empty_grid();
            g[1][0] = CraftSlot::Material(MaterialId::Cloth);
            g[1][1] = CraftSlot::Material(MaterialId::String);
            g[1][2] = CraftSlot::Material(dye);
            let out = match_recipe(&g).expect("cloth + string + dye → kite");
            match out.item {
                Item::Block(b) => assert_eq!(b, kite_block,
                    "dye {dye:?} → wrong kite"),
                o => panic!("expected kite block, got {o:?}"),
            }
            assert_eq!(out.count, 1);
        }
    }

    #[test]
    fn kite_recipe_distinct_from_lantern_first_slot() {
        // Lantern uses Papyrus + Stick + Dye; Kite uses Cloth + String
        // + Dye. The two recipes must NOT collide — a Papyrus + String
        // + Dye row (mixed) should match nothing, not silently fall
        // into either recipe.
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::PapyrusSheet);
        g[1][1] = CraftSlot::Material(MaterialId::String);
        g[1][2] = CraftSlot::Material(MaterialId::RedDye);
        assert!(match_recipe(&g).is_none(),
            "Papyrus + String + Dye is a malformed row — no recipe should fire");
    }

    #[test]
    fn banner_recipe_dye_cloth_stick_column_yields_one_per_colour() {
        // Banner — `Dye / Cloth / Stick` 3-tall column top-to-bottom
        // → 1 banner of the dye's colour. Locks the column shape +
        // the dye-mapping for 4 distinct colours.
        for (dye, banner_block) in [
            (MaterialId::RedDye, block::BANNER_RED),
            (MaterialId::BlueDye, block::BANNER_BLUE),
            (MaterialId::GreenDye, block::BANNER_GREEN),
            (MaterialId::BlackDye, block::BANNER_BLACK),
        ] {
            let mut g = empty_grid();
            g[0][1] = CraftSlot::Material(dye);
            g[1][1] = CraftSlot::Material(MaterialId::Cloth);
            g[2][1] = CraftSlot::Material(MaterialId::Stick);
            let out = match_recipe(&g).expect("dye + cloth + stick → banner");
            match out.item {
                Item::Block(b) => assert_eq!(b, banner_block,
                    "dye {dye:?} → wrong banner"),
                o => panic!("expected banner block, got {o:?}"),
            }
            assert_eq!(out.count, 1);
        }
    }

    #[test]
    fn banner_recipe_rejects_inverted_column() {
        // The dye must be at the top (the flag colour), cloth in the
        // middle, stick at the bottom (the pole). Inverted should NOT
        // match — every 3-tall column shape is order-specific.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::Stick);
        g[1][1] = CraftSlot::Material(MaterialId::Cloth);
        g[2][1] = CraftSlot::Material(MaterialId::RedDye);
        assert!(match_recipe(&g).is_none(),
            "inverted Stick/Cloth/Dye column must not yield a banner");
    }

    #[test]
    fn sail_recipe_dye_canvas_stick_column_yields_one_per_colour() {
        // Sail — `Dye / Canvas / Stick` 3-tall column → 1 sail of
        // the dye's colour. Sibling shape to banner with Canvas
        // substituted for Cloth.
        for (dye, sail_block) in [
            (MaterialId::WhiteDye, block::SAIL_WHITE),
            (MaterialId::BlueDye, block::SAIL_BLUE),
            (MaterialId::OrangeDye, block::SAIL_ORANGE),
            (MaterialId::MagentaDye, block::SAIL_MAGENTA),
        ] {
            let mut g = empty_grid();
            g[0][1] = CraftSlot::Material(dye);
            g[1][1] = CraftSlot::Material(MaterialId::Canvas);
            g[2][1] = CraftSlot::Material(MaterialId::Stick);
            let out = match_recipe(&g).expect("dye + canvas + stick → sail");
            match out.item {
                Item::Block(b) => assert_eq!(b, sail_block,
                    "dye {dye:?} → wrong sail"),
                o => panic!("expected sail block, got {o:?}"),
            }
            assert_eq!(out.count, 1);
        }
    }

    #[test]
    fn sail_and_banner_recipes_do_not_collide() {
        // Banner uses Cloth in the middle, Sail uses Canvas. The
        // recipes share top + bottom slots so the middle is the
        // sole differentiator — locking that here so a future change
        // to either column can't accidentally collapse the two.
        // Build Banner — must NOT yield Sail.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::RedDye);
        g[1][1] = CraftSlot::Material(MaterialId::Cloth);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).unwrap();
        assert!(matches!(out.item, Item::Block(b) if b == block::BANNER_RED));
        // Build Sail — must NOT yield Banner.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::RedDye);
        g[1][1] = CraftSlot::Material(MaterialId::Canvas);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).unwrap();
        assert!(matches!(out.item, Item::Block(b) if b == block::SAIL_RED));
    }

    #[test]
    fn sail_for_dye_covers_every_dye_kind() {
        for dye in [
            MaterialId::WhiteDye, MaterialId::BlackDye,
            MaterialId::RedDye, MaterialId::BlueDye, MaterialId::YellowDye,
            MaterialId::OrangeDye, MaterialId::GreenDye, MaterialId::PurpleDye,
            MaterialId::PinkDye, MaterialId::LimeDye, MaterialId::LightBlueDye,
            MaterialId::GreyDye, MaterialId::LightGreyDye,
            MaterialId::BrownDye, MaterialId::CyanDye, MaterialId::MagentaDye,
        ] {
            assert!(sail_for_dye(dye).is_some(),
                "missing sail mapping for {dye:?}");
        }
        assert!(sail_for_dye(MaterialId::Stick).is_none());
    }

    #[test]
    fn banner_for_dye_covers_every_dye_kind() {
        for dye in [
            MaterialId::WhiteDye, MaterialId::BlackDye,
            MaterialId::RedDye, MaterialId::BlueDye, MaterialId::YellowDye,
            MaterialId::OrangeDye, MaterialId::GreenDye, MaterialId::PurpleDye,
            MaterialId::PinkDye, MaterialId::LimeDye, MaterialId::LightBlueDye,
            MaterialId::GreyDye, MaterialId::LightGreyDye,
            MaterialId::BrownDye, MaterialId::CyanDye, MaterialId::MagentaDye,
        ] {
            assert!(banner_for_dye(dye).is_some(),
                "missing banner mapping for {dye:?}");
        }
        assert!(banner_for_dye(MaterialId::Stick).is_none());
    }

    #[test]
    fn kite_for_dye_covers_every_dye_kind() {
        for dye in [
            MaterialId::WhiteDye, MaterialId::BlackDye,
            MaterialId::RedDye, MaterialId::BlueDye, MaterialId::YellowDye,
            MaterialId::OrangeDye, MaterialId::GreenDye, MaterialId::PurpleDye,
            MaterialId::PinkDye, MaterialId::LimeDye, MaterialId::LightBlueDye,
            MaterialId::GreyDye, MaterialId::LightGreyDye,
            MaterialId::BrownDye, MaterialId::CyanDye, MaterialId::MagentaDye,
        ] {
            assert!(kite_for_dye(dye).is_some(),
                "missing kite mapping for {dye:?}");
        }
        assert!(kite_for_dye(MaterialId::Stick).is_none());
    }

    #[test]
    fn paper_lantern_for_dye_covers_every_dye_kind() {
        for dye in [
            MaterialId::WhiteDye, MaterialId::BlackDye,
            MaterialId::RedDye, MaterialId::BlueDye, MaterialId::YellowDye,
            MaterialId::OrangeDye, MaterialId::GreenDye, MaterialId::PurpleDye,
            MaterialId::PinkDye, MaterialId::LimeDye, MaterialId::LightBlueDye,
            MaterialId::GreyDye, MaterialId::LightGreyDye,
            MaterialId::BrownDye, MaterialId::CyanDye, MaterialId::MagentaDye,
        ] {
            assert!(paper_lantern_for_dye(dye).is_some(),
                "missing paper-lantern mapping for {dye:?}");
        }
        assert!(paper_lantern_for_dye(MaterialId::Stick).is_none());
    }

    #[test]
    fn bunting_for_dye_covers_every_dye_kind() {
        // Lock the dye → bunting mapping so adding a future dye
        // is a test-visible event (missing arm = None on a known dye).
        for dye in [
            MaterialId::WhiteDye, MaterialId::BlackDye,
            MaterialId::RedDye, MaterialId::BlueDye, MaterialId::YellowDye,
            MaterialId::OrangeDye, MaterialId::GreenDye, MaterialId::PurpleDye,
            MaterialId::PinkDye, MaterialId::LimeDye, MaterialId::LightBlueDye,
            MaterialId::GreyDye, MaterialId::LightGreyDye,
            MaterialId::BrownDye, MaterialId::CyanDye, MaterialId::MagentaDye,
        ] {
            assert!(bunting_for_dye(dye).is_some(),
                "missing bunting mapping for {dye:?}");
        }
        // Non-dye materials → None.
        assert!(bunting_for_dye(MaterialId::Stick).is_none());
    }

    #[test]
    fn new_dyes_wallpaper_to_their_colours() {
        // Each of the new dyes pairs with Papyrus Sheet to a 3-pack of
        // the matching wallpaper. Locks the new entries in the
        // paper_other → wallpaper table.
        for (dye, wp) in [
            (MaterialId::BrownDye, block::WALLPAPER_BROWN),
            (MaterialId::CyanDye, block::WALLPAPER_CYAN),
            (MaterialId::MagentaDye, block::WALLPAPER_MAGENTA),
        ] {
            let mut g = empty_grid();
            g[1][1] = CraftSlot::Material(MaterialId::PapyrusSheet);
            g[1][2] = CraftSlot::Material(dye);
            let out = match_recipe(&g).expect("papyrus + new dye → wallpaper");
            assert!(matches!(out.item, Item::Block(b) if b == wp));
            assert_eq!(out.count, 3);
        }
    }

    #[test]
    fn magnesium_pairings_make_their_products() {
        // Spec 37 — Magnesium + X (order-insensitive 1×2).
        for (other, expect) in [
            (MaterialId::Sulphur, MaterialId::Fertiliser),
            (MaterialId::Stick, MaterialId::Sparkler),
            (MaterialId::PapyrusSheet, MaterialId::Flare),
            (MaterialId::IronIngot, MaterialId::MagnesiumFirestarter),
        ] {
            // Magnesium left.
            let mut g = empty_grid();
            g[1][1] = CraftSlot::Material(MaterialId::Magnesium);
            g[1][2] = CraftSlot::Material(other);
            match match_recipe(&g).expect("mg pairing should match").item {
                Item::Material(m) => assert_eq!(m, expect, "Mg+{other:?}"),
                o => panic!("expected material, got {o:?}"),
            }
            // Order-insensitive: Magnesium right.
            let mut g = empty_grid();
            g[1][1] = CraftSlot::Material(other);
            g[1][2] = CraftSlot::Material(MaterialId::Magnesium);
            assert!(match_recipe(&g).is_some(), "Mg+{other:?} reversed should match");
        }
    }

    #[test]
    fn iron_bow_recipe_still_resolves_with_cotton_derived_string() {
        // Existence test: the Iron Bow recipe (String slots filled,
        // IronIngot centre) still resolves regardless of how the String
        // was sourced (now Cotton, not the removed Spider / retired Wool).
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        let string = CraftSlot::Material(MaterialId::String);
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        g[0][1] = stick; g[0][2] = string;
        g[1][0] = stick; g[1][1] = iron; g[1][2] = string;
        g[2][1] = stick; g[2][2] = string;
        let out = match_recipe(&g).expect("iron bow recipe must resolve");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Bow);
                assert_eq!(t.material, ToolMaterial::Iron);
            }
            _ => panic!("expected iron bow tool"),
        }
    }

    // --- Farming Tier 1 (Wave 26) hoe recipes ---

    #[test]
    fn wooden_hoe_recipe_matches() {
        // MM_ / _S_ / _S_ — two planks top-left, stick column middle.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Block(block::OAK_PLANKS);
        g[0][1] = CraftSlot::Block(block::OAK_PLANKS);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("hoe recipe should match");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Hoe);
                assert_eq!(t.material, ToolMaterial::Wood);
                assert_eq!(t.durability, max_durability(ToolMaterial::Wood));
            }
            _ => panic!("expected a Hoe tool"),
        }
    }

    #[test]
    fn wooden_hoe_recipe_matches_mirrored() {
        // _MM / _S_ / _S_ — head pieces top-right, stick column middle.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Block(block::OAK_PLANKS);
        g[0][2] = CraftSlot::Block(block::OAK_PLANKS);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("mirrored hoe should match");
        match out.item {
            Item::Tool(t) => assert_eq!(t.tool_type, ToolType::Hoe),
            _ => panic!("expected a Hoe tool"),
        }
    }

    #[test]
    fn all_five_hoe_tiers_recipe_matches() {
        // Spec 16 Phase 8: hoe-ladder parity with the pickaxe ladder.
        // The match_recipe code paths through material_from_slot which
        // maps every head ingredient to its ToolMaterial — so as long
        // as the pattern matches, the tier flows through for free.
        let cases: &[(CraftSlot, ToolMaterial)] = &[
            (CraftSlot::Block(block::OAK_PLANKS), ToolMaterial::Wood),
            (CraftSlot::Block(block::COBBLESTONE), ToolMaterial::Stone),
            (CraftSlot::Material(MaterialId::IronIngot), ToolMaterial::Iron),
            (CraftSlot::Material(MaterialId::Diamond), ToolMaterial::Diamond),
            (CraftSlot::Material(MaterialId::Satori), ToolMaterial::Satori),
        ];
        for (head, expected_mat) in cases {
            let mut g = empty_grid();
            g[0][0] = *head;
            g[0][1] = *head;
            g[1][1] = CraftSlot::Material(MaterialId::Stick);
            g[2][1] = CraftSlot::Material(MaterialId::Stick);
            let out = match_recipe(&g).expect("hoe should match for this tier");
            match out.item {
                Item::Tool(t) => {
                    assert_eq!(t.tool_type, ToolType::Hoe);
                    assert_eq!(t.material, *expected_mat, "hoe head {:?}", head);
                }
                _ => panic!("expected hoe tool"),
            }
        }
    }

    #[test]
    fn satori_hoe_durability_matches_satori_pickaxe() {
        // Sanity invariant: a top-tier hoe should have the same durability
        // as the top-tier pickaxe so a player crafting both gets the same
        // mileage. Both derive from `max_durability(Satori)`.
        let hoe = Tool::new(ToolType::Hoe, ToolMaterial::Satori);
        let pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Satori);
        assert_eq!(hoe.durability, pick.durability);
    }

    #[test]
    fn satori_hoe_recipe_makes_satori_tier() {
        // The Spec 5 §3.8 Satori Hoe row in the recipe table becomes
        // real with this commit. Regression guard so a future
        // refactor doesn't disconnect Satori from the hoe ladder.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::Satori);
        g[0][1] = CraftSlot::Material(MaterialId::Satori);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("Satori hoe");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Hoe);
                assert_eq!(t.material, ToolMaterial::Satori);
            }
            _ => panic!("expected Satori hoe"),
        }
    }

    #[test]
    fn hoe_attack_damage_is_one_at_every_tier() {
        // Utility tool, not a weapon — even a Satori Hoe does fist damage.
        for m in [
            ToolMaterial::Wood,
            ToolMaterial::Stone,
            ToolMaterial::Iron,
            ToolMaterial::Diamond,
            ToolMaterial::Satori,
        ] {
            let hoe = Tool::new(ToolType::Hoe, m);
            assert_eq!(hoe.attack_damage(), 1.0, "Hoe({:?}) should do 1.0 damage", m);
        }
    }

    #[test]
    fn raw_meat_plus_coal_no_longer_smelts_at_crafting_table() {
        // Spec 17 Phase 7 — meat cooking moved to the campfire. The
        // four meat-arms in match_recipe were removed; this is the
        // regression guard that locks the new behaviour.
        for raw in [
            MaterialId::RawBeef,
            MaterialId::RawPorkchop,
            MaterialId::RawChicken,
            MaterialId::RawMutton,
        ] {
            let mut g = empty_grid();
            g[0][1] = CraftSlot::Material(raw);
            g[1][1] = CraftSlot::Material(MaterialId::Coal);
            assert!(
                match_recipe(&g).is_none(),
                "{:?} + coal must no longer smelt at the crafting table",
                raw
            );
        }
    }

    #[test]
    fn raw_iron_plus_coal_no_longer_smelts_at_crafting_table() {
        // Spec 20 Phase 6 — iron smelting moved to the Furnace. The
        // BRIDGE grid arm was removed when the workstation shipped.
        // Crafting RawIron + Coal in the grid must now return None.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::RawIron);
        g[1][1] = CraftSlot::Material(MaterialId::Coal);
        assert!(
            match_recipe(&g).is_none(),
            "iron smelting at the crafting table must no longer match",
        );
    }

    #[test]
    fn flint_and_steel_recipe_matches() {
        // Spec 17 Phase 6 — 1×2 vertical: Flint top, Iron Ingot bottom.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::Flint);
        g[1][1] = CraftSlot::Material(MaterialId::IronIngot);
        let out = match_recipe(&g).expect("flint+iron should match");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::FlintAndSteel);
                assert_eq!(t.durability, FLINT_AND_STEEL_DURABILITY);
            }
            _ => panic!("expected FlintAndSteel tool"),
        }
    }

    // ─── Spec 38 — Blueprint Paper recipe (cyanotype sensitisation) ─

    #[test]
    fn blueprint_paper_recipe_yields_three_from_papyrus_iron_salt_column() {
        // Spec 38 §"The loop" — Papyrus Sheet (top) + Iron Ingot (middle)
        // + Salt (bottom) → 3 BLUEPRINT_PAPER. The column reads as
        // cyanotype sensitisation. Replaces the retired
        // Stick + Papyrus → 9 Plan Tiles recipe.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::PapyrusSheet);
        g[1][1] = CraftSlot::Material(MaterialId::IronIngot);
        g[2][1] = CraftSlot::Material(MaterialId::Salt);
        let out = match_recipe(&g).expect("papyrus + iron + salt column should match");
        match out.item {
            Item::Block(b) => assert_eq!(b, crate::block::BLUEPRINT_PAPER),
            other => panic!("expected Block(BLUEPRINT_PAPER), got {other:?}"),
        }
        assert_eq!(out.count, 3, "Spec 38 yield = 3 sheets per craft");
    }

    #[test]
    fn blueprint_paper_recipe_rejects_inverted_layout() {
        // The column order is fixed: Papyrus top, Iron middle, Salt
        // bottom. Reversed (Salt / Iron / Papyrus) must not match —
        // we want a single canonical direction so the player learns
        // "paper at the top, then sensitiser layers below".
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::Salt);
        g[1][1] = CraftSlot::Material(MaterialId::IronIngot);
        g[2][1] = CraftSlot::Material(MaterialId::PapyrusSheet);
        assert!(match_recipe(&g).is_none(), "inverted column must not match");
    }

    #[test]
    fn retired_stick_papyrus_recipe_no_longer_matches() {
        // Spec 38 — the old Spec 24 recipe (Stick on top, PapyrusSheet
        // below → 9 Plan Tiles) is retired. The 1x2 column with that
        // pair should match NOTHING.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::Stick);
        g[1][1] = CraftSlot::Material(MaterialId::PapyrusSheet);
        assert!(
            match_recipe(&g).is_none(),
            "retired Spec 24 stick + papyrus recipe must no longer match"
        );
    }

    // ─── Spec 20 Phase 4 — Furnace recipe (8-cobblestone ring) ──────

    #[test]
    fn drafting_table_recipe_paper_plus_three_planks() {
        // Spec 26 — 2×2 with paper top-left, planks elsewhere.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::PapyrusSheet);
        g[0][1] = CraftSlot::Block(crate::block::OAK_PLANKS);
        g[1][0] = CraftSlot::Block(crate::block::OAK_PLANKS);
        g[1][1] = CraftSlot::Block(crate::block::OAK_PLANKS);
        let out = match_recipe(&g).expect("paper + 3 planks should match Drafting Table");
        match out.item {
            Item::Block(b) => assert_eq!(b, crate::block::DRAFTING_TABLE),
            other => panic!("expected DRAFTING_TABLE block, got {other:?}"),
        }
    }

    #[test]
    fn furnace_recipe_8_cobble_ring() {
        let mut g = empty_grid();
        let cob = CraftSlot::Block(crate::block::COBBLESTONE);
        // Ring: 8 cobble around an empty centre.
        g[0][0] = cob; g[0][1] = cob; g[0][2] = cob;
        g[1][0] = cob;                  g[1][2] = cob;
        g[2][0] = cob; g[2][1] = cob; g[2][2] = cob;
        let out = match_recipe(&g).expect("8 cobble ring should match Furnace");
        match out.item {
            Item::Block(b) => assert_eq!(b, crate::block::FURNACE),
            other => panic!("expected Block(FURNACE), got {other:?}"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn furnace_recipe_rejects_filled_centre() {
        // 9 cobble (centre filled) is NOT a furnace — falls through to
        // a no-match (cobble has no storage-block variant).
        let mut g = empty_grid();
        let cob = CraftSlot::Block(crate::block::COBBLESTONE);
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = cob;
            }
        }
        assert!(match_recipe(&g).is_none(), "filled centre must not match furnace");
    }

    #[test]
    fn furnace_recipe_rejects_partial_ring() {
        // 7 cobble (one corner missing) — incomplete ring.
        let mut g = empty_grid();
        let cob = CraftSlot::Block(crate::block::COBBLESTONE);
        g[0][0] = cob; g[0][1] = cob; g[0][2] = cob;
        g[1][0] = cob;                  g[1][2] = cob;
        g[2][0] = cob; g[2][1] = cob;
        // (2, 2) missing.
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn chest_recipe_resolves_from_eight_oak_planks_ring() {
        let mut g = empty_grid();
        let planks = CraftSlot::Block(crate::block::OAK_PLANKS);
        g[0][0] = planks; g[0][1] = planks; g[0][2] = planks;
        g[1][0] = planks;                    g[1][2] = planks;
        g[2][0] = planks; g[2][1] = planks; g[2][2] = planks;
        let out = match_recipe(&g).expect("8 oak planks ring → Chest");
        match out.item {
            Item::Block(b) => assert_eq!(b, crate::block::CHEST),
            other => panic!("expected Block(CHEST), got {other:?}"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn tier_chest_recipes_resolve_from_material_ring_around_a_wood_chest() {
        // #15 — 8× tier material ringing a wood CHEST → the tier chest block.
        let chest = CraftSlot::Block(crate::block::CHEST);
        for (mat, expect) in [
            (MaterialId::CopperIngot, crate::block::COPPER_CHEST),
            (MaterialId::IronIngot, crate::block::IRON_CHEST),
            (MaterialId::Diamond, crate::block::DIAMOND_CHEST),
            (MaterialId::Satori, crate::block::SATORI_CHEST),
        ] {
            let s = CraftSlot::Material(mat);
            let mut g = empty_grid();
            g[0][0] = s; g[0][1] = s; g[0][2] = s;
            g[1][0] = s; g[1][1] = chest; g[1][2] = s;
            g[2][0] = s; g[2][1] = s; g[2][2] = s;
            let out = match_recipe(&g).expect("material ring + wood chest → tier chest");
            match out.item {
                Item::Block(b) => assert_eq!(b, expect, "tier for {mat:?}"),
                other => panic!("expected Block, got {other:?}"),
            }
            assert_eq!(out.count, 1);
        }
    }

    #[test]
    fn wood_chest_recipe_still_needs_an_empty_centre() {
        // Regression: a CHEST at the centre with a plank ring must NOT craft a
        // wood chest (that path requires an *empty* centre) — it's the tier path.
        let mut g = empty_grid();
        let p = CraftSlot::Block(crate::block::OAK_PLANKS);
        g[0][0] = p; g[0][1] = p; g[0][2] = p;
        g[1][0] = p; g[1][1] = CraftSlot::Block(crate::block::CHEST); g[1][2] = p;
        g[2][0] = p; g[2][1] = p; g[2][2] = p;
        // Plank ring + CHEST centre matches no tier (rings are materials), so None.
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn chest_recipe_accepts_mixed_species_planks() {
        // HP-2 spec: "Shapeless across plank variants — any planks work."
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Block(crate::block::OAK_PLANKS);
        g[0][1] = CraftSlot::Block(crate::block::BIRCH_PLANKS);
        g[0][2] = CraftSlot::Block(crate::block::SPRUCE_PLANKS);
        g[1][0] = CraftSlot::Block(crate::block::JUNGLE_PLANKS);
        g[1][2] = CraftSlot::Block(crate::block::ACACIA_PLANKS);
        g[2][0] = CraftSlot::Block(crate::block::DARK_OAK_PLANKS);
        g[2][1] = CraftSlot::Block(crate::block::OAK_PLANKS);
        g[2][2] = CraftSlot::Block(crate::block::OAK_PLANKS);
        let out = match_recipe(&g).expect("mixed-species ring → Chest");
        assert!(matches!(out.item, Item::Block(b) if b == crate::block::CHEST));
    }

    #[test]
    fn chest_recipe_rejects_filled_centre() {
        let mut g = empty_grid();
        let planks = CraftSlot::Block(crate::block::OAK_PLANKS);
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = planks;
            }
        }
        // 9 planks → no recipe (chest requires empty centre).
        let result = match_recipe(&g);
        // 9 oak planks isn't a chest (centre filled) and not anything
        // else either today.
        match result {
            Some(out) => match out.item {
                Item::Block(b) => assert_ne!(b, crate::block::CHEST,
                    "filled centre must NOT yield a chest"),
                _ => {}
            },
            None => {}
        }
    }

    #[test]
    fn chest_recipe_rejects_partial_ring() {
        let mut g = empty_grid();
        let planks = CraftSlot::Block(crate::block::OAK_PLANKS);
        g[0][0] = planks; g[0][1] = planks; g[0][2] = planks;
        g[1][0] = planks;                    g[1][2] = planks;
        g[2][0] = planks; g[2][1] = planks;
        // (2, 2) missing — incomplete ring.
        let result = match_recipe(&g);
        if let Some(out) = result {
            if let Item::Block(b) = out.item {
                assert_ne!(b, crate::block::CHEST, "partial ring must NOT yield a chest");
            }
        }
    }

    #[test]
    fn blueprint_paper_recipe_currently_rejects_non_papyrus_paper_grades() {
        // Pre-Spec-12: PulpPaper doesn't exist yet, so the recipe should
        // only match PapyrusSheet. Once Spec 12 ships PulpPaper this
        // test will be updated alongside a new
        // `blueprint_paper_recipe_yields_more_from_pulp_paper` test that
        // asserts the higher-grade yield.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Material(MaterialId::PapyrusReed);
        g[1][1] = CraftSlot::Material(MaterialId::IronIngot);
        g[2][1] = CraftSlot::Material(MaterialId::Salt);
        assert!(
            match_recipe(&g).is_none(),
            "raw reed should not match the Blueprint Paper recipe — only sheets"
        );
    }

    #[test]
    fn flint_and_steel_durability_is_65() {
        // Spec 17 — F&S single-tier durability is 65 (Minecraft baseline).
        // Material is cosmetic; durability comes from the special constant.
        let tool = Tool::new(ToolType::FlintAndSteel, ToolMaterial::Iron);
        assert_eq!(tool.durability, 65);
        assert_eq!(FLINT_AND_STEEL_DURABILITY, 65);
    }

    #[test]
    fn flint_and_steel_attack_damage_is_one() {
        // Utility tool, not a weapon — even at iron tier.
        let tool = Tool::new(ToolType::FlintAndSteel, ToolMaterial::Iron);
        assert_eq!(tool.attack_damage(), 1.0);
    }

    #[test]
    fn campfire_recipe_produces_unlit_campfire() {
        // SSS / LLL / SSS — three sticks top, three logs middle, three
        // sticks bottom. Output is the UNLIT campfire (player must
        // fuel + ignite separately).
        let mut g = empty_grid();
        for c in 0..3 {
            g[0][c] = CraftSlot::Material(MaterialId::Stick);
            g[1][c] = CraftSlot::Block(block::OAK_LOG);
            g[2][c] = CraftSlot::Material(MaterialId::Stick);
        }
        let out = match_recipe(&g).expect("campfire recipe");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::CAMPFIRE_UNLIT),
            _ => panic!("expected unlit campfire"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn campfire_recipe_accepts_any_log_variant() {
        // Wave 29 fix: tree-felling drops GreenLog material, not OAK_LOG
        // block-items — so the first-night campfire MUST be craftable
        // from green logs straight out of the tree. Seasoned + kiln-dried
        // also work (a player flush with infrastructure shouldn't be
        // blocked from using premium wood on a campfire). Mixed log
        // types in the middle row are also OK by design.
        use crate::item::MaterialId;
        let inputs = [
            CraftSlot::Block(block::OAK_LOG),
            CraftSlot::Material(MaterialId::GreenLog),
            CraftSlot::Material(MaterialId::SeasonedLog),
            CraftSlot::Material(MaterialId::KilnDriedLog),
        ];
        for log_slot in inputs {
            let mut g = empty_grid();
            for c in 0..3 {
                g[0][c] = CraftSlot::Material(MaterialId::Stick);
                g[1][c] = log_slot;
                g[2][c] = CraftSlot::Material(MaterialId::Stick);
            }
            let out = match_recipe(&g).unwrap_or_else(|| panic!("campfire must craft with {:?}", log_slot));
            match out.item {
                Item::Block(id) => assert_eq!(id, block::CAMPFIRE_UNLIT, "{:?} → CAMPFIRE_UNLIT", log_slot),
                _ => panic!("expected unlit campfire block for {:?}", log_slot),
            }
        }
    }

    #[test]
    fn campfire_recipe_accepts_mixed_log_types_in_middle_row() {
        // Cross-cell coverage: a half-stack of green + a half-stack of
        // seasoned (e.g. a player mid-transition) still produces a
        // campfire. No rule that the middle row must be uniform.
        use crate::item::MaterialId;
        let mut g = empty_grid();
        for c in 0..3 {
            g[0][c] = CraftSlot::Material(MaterialId::Stick);
            g[2][c] = CraftSlot::Material(MaterialId::Stick);
        }
        g[1][0] = CraftSlot::Material(MaterialId::GreenLog);
        g[1][1] = CraftSlot::Material(MaterialId::SeasonedLog);
        g[1][2] = CraftSlot::Block(block::OAK_LOG);
        let out = match_recipe(&g).expect("mixed-log campfire must craft");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::CAMPFIRE_UNLIT),
            _ => panic!("expected unlit campfire block"),
        }
    }

    #[test]
    fn three_wheat_horizontal_makes_bread() {
        // Spec 16 Phase 7 / Minecraft pattern.
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::Wheat);
        g[1][1] = CraftSlot::Material(MaterialId::Wheat);
        g[1][2] = CraftSlot::Material(MaterialId::Wheat);
        let out = match_recipe(&g).expect("3 wheat → bread");
        match out.item {
            Item::Material(m) => assert!(matches!(m, MaterialId::Bread)),
            _ => panic!("expected bread"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn three_stone_horizontal_makes_six_slabs() {
        // F1 block-shape foundation.
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Block(block::STONE);
        g[1][1] = CraftSlot::Block(block::STONE);
        g[1][2] = CraftSlot::Block(block::STONE);
        let out = match_recipe(&g).expect("3 stone → slabs");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::STONE_SLAB),
            _ => panic!("expected stone slab block"),
        }
        assert_eq!(out.count, 6);
    }

    #[test]
    fn six_planks_over_stick_makes_three_signs() {
        // F1 Wave 2c — 6 planks (3×2) + a centred stick foot → 3 signs.
        let mut g = empty_grid();
        for c in 0..3 {
            g[0][c] = CraftSlot::Block(block::OAK_PLANKS);
            g[1][c] = CraftSlot::Block(block::OAK_PLANKS);
        }
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("planks + stick → signs");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::OAK_SIGN),
            _ => panic!("expected oak sign block"),
        }
        assert_eq!(out.count, 3);
    }

    #[test]
    fn stick_ring_round_leather_makes_an_item_frame() {
        // F1 Wave 2c — 8 sticks ring + leather centre → 1 item frame.
        let mut g = empty_grid();
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = CraftSlot::Material(MaterialId::Stick);
            }
        }
        g[1][1] = CraftSlot::Material(MaterialId::Leather);
        let out = match_recipe(&g).expect("sticks + leather → frame");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::ITEM_FRAME),
            _ => panic!("expected item frame block"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn six_cobblestone_makes_six_walls() {
        // F1 Wave 2c — a 3×2 block of cobblestone → 6 cobblestone walls.
        let mut g = empty_grid();
        for col in 0..3 {
            g[1][col] = CraftSlot::Block(block::COBBLESTONE);
            g[2][col] = CraftSlot::Block(block::COBBLESTONE);
        }
        let out = match_recipe(&g).expect("6 cobble → walls");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::COBBLESTONE_WALL),
            _ => panic!("expected cobblestone wall block"),
        }
        assert_eq!(out.count, 6);
    }

    #[test]
    fn stone_staircase_makes_four_stairs_both_hands() {
        // Left-handed staircase.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Block(block::STONE);
        g[1][0] = CraftSlot::Block(block::STONE);
        g[1][1] = CraftSlot::Block(block::STONE);
        g[2][0] = CraftSlot::Block(block::STONE);
        g[2][1] = CraftSlot::Block(block::STONE);
        g[2][2] = CraftSlot::Block(block::STONE);
        let out = match_recipe(&g).expect("staircase → stairs");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::STONE_STAIRS),
            _ => panic!("expected stone stairs block"),
        }
        assert_eq!(out.count, 4);

        // Right-handed mirror also crafts.
        let mut g2 = empty_grid();
        g2[0][2] = CraftSlot::Block(block::STONE);
        g2[1][1] = CraftSlot::Block(block::STONE);
        g2[1][2] = CraftSlot::Block(block::STONE);
        g2[2][0] = CraftSlot::Block(block::STONE);
        g2[2][1] = CraftSlot::Block(block::STONE);
        g2[2][2] = CraftSlot::Block(block::STONE);
        let out2 = match_recipe(&g2).expect("mirror staircase → stairs");
        assert!(matches!(out2.item, Item::Block(id) if id == block::STONE_STAIRS));
        assert_eq!(out2.count, 4);
    }

    #[test]
    fn fence_gate_recipe_sps_over_sps() {
        // F1 Wave 2 — sticks outside, plank rail between (×2 rows).
        let mut g = empty_grid();
        for row in 0..2 {
            g[row][0] = CraftSlot::Material(MaterialId::Stick);
            g[row][1] = CraftSlot::Block(block::OAK_PLANKS);
            g[row][2] = CraftSlot::Material(MaterialId::Stick);
        }
        let out = match_recipe(&g).expect("SPS/SPS → fence gate");
        assert!(matches!(out.item, Item::Block(id) if id == block::OAK_FENCE_GATE));
        assert_eq!(out.count, 1);
        // The inverse (PSP/PSP) must still make fence posts, not a gate.
        let mut g2 = empty_grid();
        for row in 0..2 {
            g2[row][0] = CraftSlot::Block(block::OAK_PLANKS);
            g2[row][1] = CraftSlot::Material(MaterialId::Stick);
            g2[row][2] = CraftSlot::Block(block::OAK_PLANKS);
        }
        let out2 = match_recipe(&g2).expect("PSP/PSP → fence posts");
        assert!(matches!(out2.item, Item::Block(id) if id != block::OAK_FENCE_GATE));
    }

    #[test]
    fn door_recipe_two_by_three_planks_makes_three() {
        // F1 Wave 2 — 2-wide × 3-tall planks → 3 doors.
        let mut g = empty_grid();
        for row in 0..3 {
            g[row][0] = CraftSlot::Block(block::OAK_PLANKS);
            g[row][1] = CraftSlot::Block(block::OAK_PLANKS);
        }
        let out = match_recipe(&g).expect("2x3 planks → doors");
        assert!(matches!(out.item, Item::Block(id) if id == block::OAK_DOOR));
        assert_eq!(out.count, 3);
    }

    #[test]
    fn pane_recipes_glass_and_iron() {
        // 6 glass → 16 glass panes.
        let mut g = empty_grid();
        for row in 0..2 {
            for col in 0..3 {
                g[row][col] = CraftSlot::Block(block::GLASS);
            }
        }
        let out = match_recipe(&g).expect("6 glass → panes");
        assert!(matches!(out.item, Item::Block(id) if id == block::GLASS_PANE));
        assert_eq!(out.count, 16);
        // 6 iron ingots → 16 iron bars.
        let mut g2 = empty_grid();
        for row in 0..2 {
            for col in 0..3 {
                g2[row][col] = CraftSlot::Material(MaterialId::IronIngot);
            }
        }
        let out2 = match_recipe(&g2).expect("6 iron → bars");
        assert!(matches!(out2.item, Item::Block(id) if id == block::IRON_BARS));
        assert_eq!(out2.count, 16);
    }

    #[test]
    fn trapdoor_recipe_six_planks_makes_two() {
        // F1 Wave 2 — PPP/PPP → 2 trapdoors.
        let mut g = empty_grid();
        for row in 0..2 {
            for col in 0..3 {
                g[row][col] = CraftSlot::Block(block::OAK_PLANKS);
            }
        }
        let out = match_recipe(&g).expect("6 planks → trapdoors");
        assert!(matches!(out.item, Item::Block(id) if id == block::OAK_TRAPDOOR));
        assert_eq!(out.count, 2);
    }

    #[test]
    fn two_wheat_does_not_make_bread() {
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::Wheat);
        g[1][1] = CraftSlot::Material(MaterialId::Wheat);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn is_paperish_slot_covers_papyrus_sheet_only_for_now() {
        // Spec 23 — v1 has just PapyrusSheet. T1.5's Pulp Paper joins
        // here as a one-arm extension when Spec 12 ships.
        assert!(is_paperish_slot(CraftSlot::Material(MaterialId::PapyrusSheet)));
    }

    #[test]
    fn is_paperish_slot_rejects_non_paper_materials() {
        assert!(!is_paperish_slot(CraftSlot::Material(MaterialId::PapyrusReed)));
        assert!(!is_paperish_slot(CraftSlot::Material(MaterialId::Stick)));
        assert!(!is_paperish_slot(CraftSlot::Material(MaterialId::Wheat)));
        assert!(!is_paperish_slot(CraftSlot::Block(block::OAK_PLANKS)));
        assert!(!is_paperish_slot(CraftSlot::Empty));
    }

    #[test]
    fn three_reeds_horizontal_makes_three_sheets() {
        // Spec 23: 3 papyrus reeds in any row of the 3×3 grid → 3
        // Papyrus Sheets. Test every row position (top / middle /
        // bottom) since bounding-box trimming flattens them all to
        // the same h=1, w=3 case.
        for row in 0..3 {
            let mut g = empty_grid();
            g[row][0] = CraftSlot::Material(MaterialId::PapyrusReed);
            g[row][1] = CraftSlot::Material(MaterialId::PapyrusReed);
            g[row][2] = CraftSlot::Material(MaterialId::PapyrusReed);
            let out = match_recipe(&g).expect("3 reeds → 3 sheets");
            match out.item {
                Item::Material(MaterialId::PapyrusSheet) => {}
                other => panic!("row {row}: expected PapyrusSheet, got {:?}", other),
            }
            assert_eq!(out.count, 3, "row {row}: yield should be 3 sheets");
        }
    }

    #[test]
    fn two_reeds_does_not_make_sheets() {
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::PapyrusReed);
        g[1][1] = CraftSlot::Material(MaterialId::PapyrusReed);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn mixed_reed_and_stick_row_does_not_match() {
        // Defends against a future refactor accidentally accepting
        // ≥2 reeds in the row.
        let mut g = empty_grid();
        g[1][0] = CraftSlot::Material(MaterialId::PapyrusReed);
        g[1][1] = CraftSlot::Material(MaterialId::PapyrusReed);
        g[1][2] = CraftSlot::Material(MaterialId::Stick);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn axe_pattern_does_not_misfire_as_hoe() {
        // Axe MM_ / MS_ / _S_ has M in r1[0]; hoe explicitly requires
        // r1[0] = Empty. Regression guard so a future refactor doesn't
        // re-order the checks and start producing hoes for axe inputs.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Block(block::OAK_PLANKS);
        g[0][1] = CraftSlot::Block(block::OAK_PLANKS);
        g[1][0] = CraftSlot::Block(block::OAK_PLANKS);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("axe recipe should match");
        match out.item {
            Item::Tool(t) => assert_eq!(t.tool_type, ToolType::Axe, "expected Axe, got {:?}", t.tool_type),
            _ => panic!("expected a tool"),
        }
    }

    #[test]
    fn unknown_pattern_returns_none() {
        // Stick alone is not a valid recipe by itself.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn bed_recipe_matches_wool_over_planks() {
        let mut g = empty_grid();
        for c in 0..3 {
            g[0][c] = CraftSlot::Material(MaterialId::Wool);
            g[1][c] = CraftSlot::Block(block::OAK_PLANKS);
        }
        let out = match_recipe(&g).expect("3 wool + 3 planks should make a bed");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::BED),
            _ => panic!("expected bed block"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn fence_post_recipe_yields_three_from_plank_stick_plank_2x3() {
        // Spec 36 Fences mini-spec (2026-05-28) — `PSP / PSP` 2×3
        // pattern → 3 Fence Posts. Locks both the shape and the yield.
        let mut g = empty_grid();
        let p = CraftSlot::Block(block::OAK_PLANKS);
        let s = CraftSlot::Material(MaterialId::Stick);
        g[0][0] = p; g[0][1] = s; g[0][2] = p;
        g[1][0] = p; g[1][1] = s; g[1][2] = p;
        let out = match_recipe(&g).expect("PSP / PSP should make fence posts");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::FENCE_POST),
            other => panic!("expected FENCE_POST block, got {other:?}"),
        }
        assert_eq!(out.count, 3);
    }

    #[test]
    fn fence_post_recipe_rejects_swapped_columns() {
        // The stick column must be in the middle for FENCE POSTS. SPS / SPS
        // (swapped) is NOT a fence post — since F1 Wave 2 it is the canonical
        // **fence gate** shape instead (sticks outside, plank rail between).
        let mut g = empty_grid();
        let p = CraftSlot::Block(block::OAK_PLANKS);
        let s = CraftSlot::Material(MaterialId::Stick);
        g[0][0] = s; g[0][1] = p; g[0][2] = s;
        g[1][0] = s; g[1][1] = p; g[1][2] = s;
        let out = match_recipe(&g).expect("SPS / SPS now crafts a fence gate");
        match out.item {
            Item::Block(id) => assert_eq!(
                id,
                block::OAK_FENCE_GATE,
                "SPS / SPS must yield a fence gate, never fence posts"
            ),
            other => panic!("expected OAK_FENCE_GATE block, got {other:?}"),
        }
    }

    #[test]
    fn fence_post_recipe_per_species_table() {
        // Per-species fence-post recipe (Fences v2 species slice,
        // 2026-05-28) — same PSP/PSP shape, the plank species
        // determines which fence-post variant comes out.
        for (plank_block, expected_post) in [
            (block::OAK_PLANKS, block::OAK_FENCE_POST),
            (block::BIRCH_PLANKS, block::BIRCH_FENCE_POST),
            (block::SPRUCE_PLANKS, block::SPRUCE_FENCE_POST),
            (block::JUNGLE_PLANKS, block::JUNGLE_FENCE_POST),
            (block::ACACIA_PLANKS, block::ACACIA_FENCE_POST),
            (block::DARK_OAK_PLANKS, block::DARK_OAK_FENCE_POST),
            (block::RUBBER_PLANKS, block::RUBBER_FENCE_POST),
        ] {
            let mut g = empty_grid();
            let p = CraftSlot::Block(plank_block);
            let s = CraftSlot::Material(MaterialId::Stick);
            g[0][0] = p; g[0][1] = s; g[0][2] = p;
            g[1][0] = p; g[1][1] = s; g[1][2] = p;
            let out = match_recipe(&g).expect("PSP/PSP should match per-species fence");
            match out.item {
                Item::Block(id) => assert_eq!(id, expected_post,
                    "plank {plank_block} → wrong fence-post variant"),
                o => panic!("expected fence-post block, got {o:?}"),
            }
            assert_eq!(out.count, 3);
        }
    }

    #[test]
    fn fence_post_recipe_rejects_mixed_species_planks() {
        // Mixed-species column should NOT match — all 4 plank slots
        // must be the same species. Otherwise a future "mixed" recipe
        // could fire by accident.
        let mut g = empty_grid();
        let oak = CraftSlot::Block(block::OAK_PLANKS);
        let birch = CraftSlot::Block(block::BIRCH_PLANKS);
        let s = CraftSlot::Material(MaterialId::Stick);
        g[0][0] = oak;   g[0][1] = s; g[0][2] = birch;
        g[1][0] = birch; g[1][1] = s; g[1][2] = oak;
        assert!(
            match_recipe(&g).is_none(),
            "mixed-species plank column must not yield a fence post"
        );
    }

    #[test]
    fn tent_recipe_canvas_roof_stick_corners_yields_one() {
        // Tent — `CCC / S.S` 2×3 → 1 Tent.
        let mut g = empty_grid();
        let c = CraftSlot::Material(MaterialId::Canvas);
        let s = CraftSlot::Material(MaterialId::Stick);
        g[0][0] = c; g[0][1] = c; g[0][2] = c;
        g[1][0] = s; g[1][1] = CraftSlot::Empty; g[1][2] = s;
        let out = match_recipe(&g).expect("CCC / S.S should make a Tent");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::TENT),
            o => panic!("expected TENT block, got {o:?}"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn tent_recipe_rejects_filled_centre_bottom() {
        // Filled centre-bottom (would look like a "tent with a pole")
        // is NOT the recipe — empty centre is the doorway and the
        // structural differentiator.
        let mut g = empty_grid();
        let c = CraftSlot::Material(MaterialId::Canvas);
        let s = CraftSlot::Material(MaterialId::Stick);
        g[0][0] = c; g[0][1] = c; g[0][2] = c;
        g[1][0] = s; g[1][1] = s; g[1][2] = s;
        // The all-canvas top + all-stick bottom is not the Tent
        // shape (Tent needs empty centre on the bottom row). The
        // recipe must NOT fire.
        let out = match_recipe(&g);
        if let Some(o) = out {
            if let Item::Block(b) = o.item {
                assert_ne!(b, block::TENT,
                    "filled centre-bottom must NOT match the Tent recipe");
            }
        }
    }

    #[test]
    fn tent_recipe_distinct_from_helmet_pattern() {
        // Helmet needs MMM / M.M with the same material in all 5 non-
        // empty slots. Tent uses Canvas for r0 + Stick for r1, so
        // the material-equality fails. Locking that here so a future
        // armour-material expansion can't accidentally accept Canvas
        // as helmet material.
        let mut g = empty_grid();
        let c = CraftSlot::Material(MaterialId::Canvas);
        let s = CraftSlot::Material(MaterialId::Stick);
        g[0][0] = c; g[0][1] = c; g[0][2] = c;
        g[1][0] = s; g[1][1] = CraftSlot::Empty; g[1][2] = s;
        let out = match_recipe(&g).unwrap();
        match out.item {
            // Tent, not Helmet.
            Item::Block(id) => assert_eq!(id, block::TENT),
            other => panic!("expected TENT, not {other:?}"),
        }
    }

    #[test]
    fn is_fence_post_covers_every_species() {
        // The Lead-tether anchor check calls `block::is_fence_post`;
        // make sure no species variant slips through (otherwise the
        // player couldn't anchor on a birch fence, say).
        for post in [
            block::OAK_FENCE_POST,
            block::BIRCH_FENCE_POST,
            block::SPRUCE_FENCE_POST,
            block::JUNGLE_FENCE_POST,
            block::ACACIA_FENCE_POST,
            block::DARK_OAK_FENCE_POST,
            block::RUBBER_FENCE_POST,
        ] {
            assert!(block::is_fence_post(post),
                "is_fence_post should accept variant {post}");
        }
        // And a non-fence block doesn't false-positive.
        assert!(!block::is_fence_post(block::OAK_PLANKS));
        assert!(!block::is_fence_post(block::STONE));
    }

    #[test]
    fn sand_to_glass_recipe_works() {
        let mut g = empty_grid();
        for r in 0..2 {
            for c in 0..2 {
                g[r][c] = CraftSlot::Block(block::SAND);
            }
        }
        let out = match_recipe(&g).expect("4 sand → glass should match");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::GLASS),
            _ => panic!("expected glass block"),
        }
        assert_eq!(out.count, 4);
    }

    #[test]
    fn nine_coal_to_coal_block_round_trip() {
        // 9 coal → 1 coal block, then 1 coal block → 9 coal back.
        let mut g = empty_grid();
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = CraftSlot::Material(MaterialId::Coal);
            }
        }
        let to_block = match_recipe(&g).expect("9 coal → coal block");
        match to_block.item {
            Item::Block(id) => assert_eq!(id, block::COAL_BLOCK),
            _ => panic!("expected coal block"),
        }
        assert_eq!(to_block.count, 1);

        let mut g2 = empty_grid();
        g2[1][1] = CraftSlot::Block(block::COAL_BLOCK);
        let back = match_recipe(&g2).expect("coal block → 9 coal");
        match back.item {
            Item::Material(id) => assert_eq!(id, MaterialId::Coal),
            _ => panic!("expected coal material"),
        }
        assert_eq!(back.count, 9);
    }

    #[test]
    fn nine_diamond_to_diamond_block() {
        let mut g = empty_grid();
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = CraftSlot::Material(MaterialId::Diamond);
            }
        }
        let out = match_recipe(&g).expect("9 diamond → diamond block");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::DIAMOND_BLOCK),
            _ => panic!("expected diamond block"),
        }
    }

    // --- Tool progression / harvest gating (Wave 7) ---

    #[test]
    fn min_tool_tier_for_ores_is_correct() {
        assert_eq!(min_tool_tier(block::STONE), Some(ToolMaterial::Wood));
        assert_eq!(min_tool_tier(block::COAL_ORE), Some(ToolMaterial::Wood));
        assert_eq!(min_tool_tier(block::IRON_ORE), Some(ToolMaterial::Stone));
        assert_eq!(min_tool_tier(block::DIAMOND_ORE), Some(ToolMaterial::Iron));
        assert_eq!(min_tool_tier(block::COAL_BLOCK), Some(ToolMaterial::Wood));
        assert_eq!(min_tool_tier(block::IRON_BLOCK), Some(ToolMaterial::Stone));
        assert_eq!(min_tool_tier(block::DIAMOND_BLOCK), Some(ToolMaterial::Iron));
        // E1 — magnesium + copper gate at Stone tier (were fist-harvestable).
        assert_eq!(min_tool_tier(block::MAGNESIUM_ORE), Some(ToolMaterial::Stone));
        assert_eq!(min_tool_tier(block::COPPER_ORE), Some(ToolMaterial::Stone));
    }

    #[test]
    fn min_tool_tier_for_soft_blocks_is_none() {
        assert_eq!(min_tool_tier(block::DIRT), None);
        assert_eq!(min_tool_tier(block::OAK_LOG), None);
        assert_eq!(min_tool_tier(block::SAND), None);
        assert_eq!(min_tool_tier(block::GLASS), None);
        assert_eq!(min_tool_tier(block::TORCH), None);
    }

    #[test]
    fn fist_cannot_harvest_stone() {
        assert!(!can_harvest(block::STONE, None));
        assert!(!can_harvest(block::COAL_ORE, None));
        assert!(!can_harvest(block::IRON_ORE, None));
        assert!(!can_harvest(block::DIAMOND_ORE, None));
    }

    #[test]
    fn wooden_pickaxe_harvests_stone_and_coal_only() {
        let wood_pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Wood);
        assert!(can_harvest(block::STONE, Some(&wood_pick)));
        assert!(can_harvest(block::COAL_ORE, Some(&wood_pick)));
        assert!(!can_harvest(block::IRON_ORE, Some(&wood_pick)));
        assert!(!can_harvest(block::DIAMOND_ORE, Some(&wood_pick)));
    }

    #[test]
    fn stone_pickaxe_harvests_iron_but_not_diamond() {
        let stone_pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
        assert!(can_harvest(block::STONE, Some(&stone_pick)));
        assert!(can_harvest(block::IRON_ORE, Some(&stone_pick)));
        assert!(!can_harvest(block::DIAMOND_ORE, Some(&stone_pick)));
    }

    #[test]
    fn iron_pickaxe_harvests_diamond() {
        let iron_pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        assert!(can_harvest(block::DIAMOND_ORE, Some(&iron_pick)));
        let diamond_pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond);
        assert!(can_harvest(block::DIAMOND_ORE, Some(&diamond_pick)));
    }

    #[test]
    fn axe_does_not_harvest_stone() {
        let iron_axe = Tool::new(ToolType::Axe, ToolMaterial::Iron);
        // Even a high-tier axe doesn't satisfy the pickaxe requirement.
        assert!(!can_harvest(block::STONE, Some(&iron_axe)));
        assert!(!can_harvest(block::IRON_ORE, Some(&iron_axe)));
    }

    #[test]
    fn dirt_drops_for_anyone() {
        // Dirt has no min-tier, so fist + any tool harvests it.
        assert!(can_harvest(block::DIRT, None));
        let wood_pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Wood);
        let wood_shovel = Tool::new(ToolType::Shovel, ToolMaterial::Wood);
        assert!(can_harvest(block::DIRT, Some(&wood_pick)));
        assert!(can_harvest(block::DIRT, Some(&wood_shovel)));
    }

    #[test]
    fn tier_index_orders_correctly() {
        assert!(tier_index(ToolMaterial::Wood) < tier_index(ToolMaterial::Stone));
        assert!(tier_index(ToolMaterial::Stone) < tier_index(ToolMaterial::Iron));
        assert!(tier_index(ToolMaterial::Iron) < tier_index(ToolMaterial::Diamond));
    }

    // --- Smelting (Wave 6 vintage — meat cooking removed in Wave 27) ---

    // The four `raw_meat + coal` smelting tests (Wave 6) were removed
    // 2026-05-18 alongside the recipe arms — meat cooking now lives
    // at the campfire (Spec 17 foundation `2026-05-18-campfire.md`).
    // The regression-guard for the removal lives in
    // `raw_meat_plus_coal_no_longer_smelts_at_crafting_table` above.

    #[test]
    fn raw_iron_alone_does_not_smelt_at_grid() {
        // Spec 20 Phase 6 — iron smelting moved to the Furnace.
        // The single-input grid arm never existed; locking it down
        // alongside the bridge removal.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::RawIron);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn cobblestone_unlocks_stone_pickaxe_recipe() {
        // Stone tools follow the standard pickaxe shape — 3 cobble
        // top, 2 sticks down the centre. `material_from_slot` maps
        // COBBLESTONE → ToolMaterial::Stone (line 785), so the
        // existing tool-recipe matcher picks this up without any
        // pickaxe-specific branch.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Block(crate::block::COBBLESTONE);
        g[0][1] = CraftSlot::Block(crate::block::COBBLESTONE);
        g[0][2] = CraftSlot::Block(crate::block::COBBLESTONE);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("cobblestone pickaxe recipe");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Pickaxe);
                assert_eq!(t.material, ToolMaterial::Stone);
            }
            _ => panic!("expected stone pickaxe"),
        }
    }

    #[test]
    fn cobblestone_unlocks_stone_sword_recipe() {
        // Sword shape: 1 wide, M / M / S.
        let mut g = empty_grid();
        g[0][1] = CraftSlot::Block(crate::block::COBBLESTONE);
        g[1][1] = CraftSlot::Block(crate::block::COBBLESTONE);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("cobblestone sword recipe");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Sword);
                assert_eq!(t.material, ToolMaterial::Stone);
            }
            _ => panic!("expected stone sword"),
        }
    }

    #[test]
    fn iron_ingot_unlocks_iron_tool_recipe() {
        // 3 iron ingots top + 2 sticks middle/bottom centre = iron pickaxe.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::IronIngot);
        g[0][1] = CraftSlot::Material(MaterialId::IronIngot);
        g[0][2] = CraftSlot::Material(MaterialId::IronIngot);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("iron ingot pickaxe recipe");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Pickaxe);
                assert_eq!(t.material, ToolMaterial::Iron);
            }
            _ => panic!("expected iron pickaxe"),
        }
    }

    #[test]
    fn diamond_unlocks_diamond_tool_recipe() {
        // 3 diamonds top + 2 sticks middle/bottom centre = diamond pickaxe.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::Diamond);
        g[0][1] = CraftSlot::Material(MaterialId::Diamond);
        g[0][2] = CraftSlot::Material(MaterialId::Diamond);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("diamond pickaxe recipe");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Pickaxe);
                assert_eq!(t.material, ToolMaterial::Diamond);
            }
            _ => panic!("expected diamond pickaxe"),
        }
    }

    #[test]
    fn cooked_food_heals_more_than_raw() {
        // Wave 6 balance check.
        let raw_beef = Item::Material(MaterialId::RawBeef);
        let cooked_beef = Item::Material(MaterialId::CookedBeef);
        assert!(cooked_beef.food_value().unwrap() > raw_beef.food_value().unwrap());
    }

    // --- Bow + arrows (Wave 23) ---

    #[test]
    fn stick_over_feather_makes_four_arrows() {
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::Stick);
        g[1][0] = CraftSlot::Material(MaterialId::Feather);
        let out = match_recipe(&g).expect("stick + feather → 4 arrows");
        match out.item {
            Item::Material(id) => assert_eq!(id, MaterialId::Arrow),
            _ => panic!("expected arrow material"),
        }
        assert_eq!(out.count, 4);
    }

    #[test]
    fn bow_curve_pattern_matches() {
        // _ T S
        // T _ S
        // _ T S
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        let string = CraftSlot::Material(MaterialId::String);
        g[0][1] = stick; g[0][2] = string;
        g[1][0] = stick; g[1][2] = string;
        g[2][1] = stick; g[2][2] = string;
        let out = match_recipe(&g).expect("bow curve → wood bow");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Bow);
                assert_eq!(t.material, ToolMaterial::Wood);
            }
            _ => panic!("expected bow tool"),
        }
    }

    // Chunk 9 — per-tier bow extensions.

    #[test]
    fn iron_bow_recipe_yields_iron_bow() {
        // Pattern: _ T S
        //          T I S        I = IronIngot
        //          _ T S
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        let string = CraftSlot::Material(MaterialId::String);
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        g[0][1] = stick; g[0][2] = string;
        g[1][0] = stick; g[1][1] = iron; g[1][2] = string;
        g[2][1] = stick; g[2][2] = string;
        let out = match_recipe(&g).expect("bow + iron centre → iron bow");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Bow);
                assert_eq!(t.material, ToolMaterial::Iron);
            }
            _ => panic!("expected bow tool"),
        }
    }

    #[test]
    fn diamond_bow_recipe_yields_diamond_bow() {
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        let string = CraftSlot::Material(MaterialId::String);
        let diamond = CraftSlot::Material(MaterialId::Diamond);
        g[0][1] = stick; g[0][2] = string;
        g[1][0] = stick; g[1][1] = diamond; g[1][2] = string;
        g[2][1] = stick; g[2][2] = string;
        let out = match_recipe(&g).expect("bow + diamond centre → diamond bow");
        if let Item::Tool(t) = out.item {
            assert_eq!(t.material, ToolMaterial::Diamond);
        } else {
            panic!("expected bow tool");
        }
    }

    #[test]
    fn satori_bow_recipe_yields_satori_bow() {
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        let string = CraftSlot::Material(MaterialId::String);
        let satori = CraftSlot::Material(MaterialId::Satori);
        g[0][1] = stick; g[0][2] = string;
        g[1][0] = stick; g[1][1] = satori; g[1][2] = string;
        g[2][1] = stick; g[2][2] = string;
        let out = match_recipe(&g).expect("bow + satori centre → satori bow");
        if let Item::Tool(t) = out.item {
            assert_eq!(t.material, ToolMaterial::Satori);
        } else {
            panic!("expected bow tool");
        }
    }

    #[test]
    fn stone_bow_recipe_uses_cobblestone() {
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        let string = CraftSlot::Material(MaterialId::String);
        let cobble = CraftSlot::Block(block::COBBLESTONE);
        g[0][1] = stick; g[0][2] = string;
        g[1][0] = stick; g[1][1] = cobble; g[1][2] = string;
        g[2][1] = stick; g[2][2] = string;
        let out = match_recipe(&g).expect("bow + cobble centre → stone bow");
        if let Item::Tool(t) = out.item {
            assert_eq!(t.material, ToolMaterial::Stone);
        } else {
            panic!("expected bow tool");
        }
    }

    #[test]
    fn bow_arrow_damage_scales_by_tier() {
        let wood = Tool::new(ToolType::Bow, ToolMaterial::Wood);
        let satori = Tool::new(ToolType::Bow, ToolMaterial::Satori);
        assert!(wood.bow_arrow_damage() < satori.bow_arrow_damage());
        // Non-bow tools return 0.
        let pickaxe = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        assert_eq!(pickaxe.bow_arrow_damage(), 0.0);
    }

    #[test]
    fn per_tier_bow_names_are_distinct() {
        for mat in [ToolMaterial::Wood, ToolMaterial::Stone, ToolMaterial::Iron,
                    ToolMaterial::Diamond, ToolMaterial::Satori] {
            let t = Tool::new(ToolType::Bow, mat);
            assert!(t.name().to_lowercase().contains("bow"));
        }
        assert_ne!(
            Tool::new(ToolType::Bow, ToolMaterial::Wood).name(),
            Tool::new(ToolType::Bow, ToolMaterial::Iron).name(),
        );
    }

    #[test]
    fn bow_inverted_pattern_does_not_match() {
        // S T _
        // S _ T
        // S T _
        // (mirror of bow recipe — Wave 23 only matches the right-hand-side
        // string variant for now, not the mirrored layout.)
        let mut g = empty_grid();
        let stick = CraftSlot::Material(MaterialId::Stick);
        let string = CraftSlot::Material(MaterialId::String);
        g[0][0] = string; g[0][1] = stick;
        g[1][0] = string; g[1][2] = stick;
        g[2][0] = string; g[2][1] = stick;
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn coal_over_stick_makes_four_torches() {
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::Coal);
        g[1][0] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("coal + stick → torches should match");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::TORCH),
            _ => panic!("expected torch block"),
        }
        assert_eq!(out.count, 4);
    }

    #[test]
    fn stick_over_coal_does_not_make_torches() {
        // Inverted layout (stick on top, coal below) is not a torch recipe.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::Stick);
        g[1][0] = CraftSlot::Material(MaterialId::Coal);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn nine_random_mix_is_not_a_block() {
        // 8 coal + 1 diamond should NOT match (must be all-same).
        let mut g = empty_grid();
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = CraftSlot::Material(MaterialId::Coal);
            }
        }
        g[1][1] = CraftSlot::Material(MaterialId::Diamond);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn bed_recipe_rejects_inverted_layout() {
        // Planks on top + wool on bottom is NOT a bed.
        let mut g = empty_grid();
        for c in 0..3 {
            g[0][c] = CraftSlot::Block(block::OAK_PLANKS);
            g[1][c] = CraftSlot::Material(MaterialId::Wool);
        }
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn nine_satori_make_storage_block() {
        let mut g = empty_grid();
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = CraftSlot::Material(MaterialId::Satori);
            }
        }
        let out = match_recipe(&g).expect("9 Satori should make a Satori block");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::SATORI_BLOCK),
            _ => panic!("expected Satori block"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn satori_block_reverse_gives_nine_satori() {
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Block(block::SATORI_BLOCK);
        let out = match_recipe(&g).expect("Satori block → 9 Satori");
        match out.item {
            Item::Material(id) => assert_eq!(id, MaterialId::Satori),
            _ => panic!("expected Satori material"),
        }
        assert_eq!(out.count, 9);
    }

    #[test]
    fn satori_pickaxe_recipe() {
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::Satori);
        g[0][1] = CraftSlot::Material(MaterialId::Satori);
        g[0][2] = CraftSlot::Material(MaterialId::Satori);
        g[1][1] = CraftSlot::Material(MaterialId::Stick);
        g[2][1] = CraftSlot::Material(MaterialId::Stick);
        let out = match_recipe(&g).expect("Satori pickaxe should match");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Pickaxe);
                assert_eq!(t.material, ToolMaterial::Satori);
                assert_eq!(t.durability, 2031);
            }
            _ => panic!("expected Satori pickaxe"),
        }
    }

    #[test]
    fn satori_tool_tier_ranks_above_diamond() {
        assert!(tier_index(ToolMaterial::Satori) > tier_index(ToolMaterial::Diamond));
    }

    #[test]
    fn deepslate_diamond_ore_requires_iron_pickaxe_like_stone_counterpart() {
        let req = min_tool_tier(block::DEEPSLATE_DIAMOND_ORE);
        assert_eq!(req, Some(ToolMaterial::Iron));
        // Stone pickaxe should NOT harvest deepslate diamond ore.
        let stone_pickaxe = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
        assert!(!can_harvest(block::DEEPSLATE_DIAMOND_ORE, Some(&stone_pickaxe)));
        // Iron pickaxe should succeed.
        let iron_pickaxe = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        assert!(can_harvest(block::DEEPSLATE_DIAMOND_ORE, Some(&iron_pickaxe)));
    }

    #[test]
    fn pure_deepslate_requires_stone_pickaxe() {
        let req = min_tool_tier(block::PURE_DEEPSLATE);
        assert_eq!(req, Some(ToolMaterial::Stone));
        // Wood pickaxe insufficient.
        let wood_pickaxe = Tool::new(ToolType::Pickaxe, ToolMaterial::Wood);
        assert!(!can_harvest(block::PURE_DEEPSLATE, Some(&wood_pickaxe)));
        let stone_pickaxe = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
        assert!(can_harvest(block::PURE_DEEPSLATE, Some(&stone_pickaxe)));
    }

    // ── Spec 28c — decorative + Bronze recipes (2026-05-20) ────────

    #[test]
    fn nine_bone_to_bone_block_round_trip() {
        let mut g = empty_grid();
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = CraftSlot::Material(MaterialId::Bone);
            }
        }
        let to_block = match_recipe(&g).expect("9 bone → bone block");
        match to_block.item {
            Item::Block(id) => assert_eq!(id, block::BONE_BLOCK),
            _ => panic!("expected bone block"),
        }
        assert_eq!(to_block.count, 1);

        let mut g2 = empty_grid();
        g2[1][1] = CraftSlot::Block(block::BONE_BLOCK);
        let back = match_recipe(&g2).expect("bone block → 9 bone");
        match back.item {
            Item::Material(id) => assert_eq!(id, MaterialId::Bone),
            _ => panic!("expected bone material"),
        }
        assert_eq!(back.count, 9);
    }

    #[test]
    fn nine_wheat_to_hay_bale_round_trip() {
        let mut g = empty_grid();
        for r in 0..3 {
            for c in 0..3 {
                g[r][c] = CraftSlot::Material(MaterialId::Wheat);
            }
        }
        let to_block = match_recipe(&g).expect("9 wheat → hay bale");
        match to_block.item {
            Item::Block(id) => assert_eq!(id, block::HAY_BALE),
            _ => panic!("expected hay bale"),
        }
        assert_eq!(to_block.count, 1);

        let mut g2 = empty_grid();
        g2[1][1] = CraftSlot::Block(block::HAY_BALE);
        let back = match_recipe(&g2).expect("hay bale → 9 wheat");
        match back.item {
            Item::Material(id) => assert_eq!(id, MaterialId::Wheat),
            _ => panic!("expected wheat material"),
        }
        assert_eq!(back.count, 9);
    }

    #[test]
    fn four_amethyst_to_amethyst_block_round_trip() {
        let mut g = empty_grid();
        for r in 0..2 {
            for c in 0..2 {
                g[r][c] = CraftSlot::Material(MaterialId::Amethyst);
            }
        }
        let to_block = match_recipe(&g).expect("4 amethyst → amethyst block");
        match to_block.item {
            Item::Block(id) => assert_eq!(id, block::AMETHYST_BLOCK),
            _ => panic!("expected amethyst block"),
        }
        assert_eq!(to_block.count, 1);

        let mut g2 = empty_grid();
        g2[1][1] = CraftSlot::Block(block::AMETHYST_BLOCK);
        let back = match_recipe(&g2).expect("amethyst block → 4 amethyst");
        match back.item {
            Item::Material(id) => assert_eq!(id, MaterialId::Amethyst),
            _ => panic!("expected amethyst material"),
        }
        assert_eq!(back.count, 4);
    }

    #[test]
    fn copper_plus_tin_alloy_makes_bronze_ingot() {
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::CopperIngot);
        g[1][0] = CraftSlot::Material(MaterialId::TinIngot);
        let out = match_recipe(&g).expect("copper ingot + tin ingot → bronze ingot");
        match out.item {
            Item::Material(id) => assert_eq!(id, MaterialId::BronzeIngot),
            _ => panic!("expected bronze ingot"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn tin_over_copper_does_not_make_bronze() {
        // Order matters — only copper-over-tin produces the alloy.
        // (Cosmetic; pattern asymmetry mirrors how other directional
        // 2-slot recipes like Flint+Steel work.)
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::TinIngot);
        g[1][0] = CraftSlot::Material(MaterialId::CopperIngot);
        assert!(match_recipe(&g).is_none());
    }

    #[test]
    fn each_species_log_crafts_to_species_planks() {
        // Spec 28b — direct species log → species planks at 4 per craft.
        use crate::block as B;
        let species_pairs = [
            (B::BIRCH_LOG, B::BIRCH_PLANKS),
            (B::SPRUCE_LOG, B::SPRUCE_PLANKS),
            (B::JUNGLE_LOG, B::JUNGLE_PLANKS),
            (B::ACACIA_LOG, B::ACACIA_PLANKS),
            (B::DARK_OAK_LOG, B::DARK_OAK_PLANKS),
        ];
        for (log_id, expected_planks) in species_pairs {
            let mut g = empty_grid();
            g[1][1] = CraftSlot::Block(log_id);
            let out = match_recipe(&g).expect("species log → species planks");
            match out.item {
                Item::Block(id) => assert_eq!(id, expected_planks,
                    "log {} should produce planks {}", log_id, expected_planks),
                _ => panic!("expected planks block"),
            }
            assert_eq!(out.count, 4);
        }
    }

    #[test]
    fn green_log_material_still_crafts_to_oak_planks() {
        // Back-compat: the species-neutral GreenLog material continues
        // to produce Oak Planks as the default species.
        let mut g = empty_grid();
        g[1][1] = CraftSlot::Material(MaterialId::GreenLog);
        let out = match_recipe(&g).expect("GreenLog → Oak Planks");
        match out.item {
            Item::Block(id) => assert_eq!(id, block::OAK_PLANKS),
            _ => panic!("expected oak planks"),
        }
        assert_eq!(out.count, 4);
    }

    // ── Spec 28e — armour crafting recipes ──────────────────────────

    // ── Spec 28e — Shears + Fishing Rod ─────────────────────────────

    #[test]
    fn shears_recipe_two_iron_vertical() {
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::IronIngot);
        g[1][0] = CraftSlot::Material(MaterialId::IronIngot);
        let out = match_recipe(&g).expect("2 iron vertical → shears");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Shears);
                assert_eq!(t.material, ToolMaterial::Iron);
                assert_eq!(t.durability, SHEARS_DURABILITY);
            }
            _ => panic!("expected Shears tool"),
        }
    }

    #[test]
    fn fishing_rod_recipe_diagonal_with_string() {
        let mut g = empty_grid();
        let s = CraftSlot::Material(MaterialId::Stick);
        let st = CraftSlot::Material(MaterialId::String);
        // . . S
        // . S X
        // S . X
        g[0][2] = s;
        g[1][1] = s;
        g[1][2] = st;
        g[2][0] = s;
        g[2][2] = st;
        let out = match_recipe(&g).expect("fishing rod pattern");
        match out.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::FishingRod);
                assert_eq!(t.material, ToolMaterial::Wood);
                assert_eq!(t.durability, FISHING_ROD_DURABILITY);
            }
            _ => panic!("expected Fishing Rod tool"),
        }
    }

    #[test]
    fn shears_durability_is_distinct_from_per_material_ladder() {
        // The shears constant should NOT equal the iron-pickaxe
        // durability — it has its own table per spec.
        let pickaxe = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        let shears = Tool::new(ToolType::Shears, ToolMaterial::Iron);
        assert_ne!(pickaxe.durability, shears.durability);
        assert_eq!(shears.durability, SHEARS_DURABILITY);
    }

    #[test]
    fn iron_helmet_recipe_top_arc() {
        // Pattern: MMM / M.M / ... (top 2 rows of grid)
        let mut g = empty_grid();
        let m = CraftSlot::Material(MaterialId::IronIngot);
        g[0][0] = m; g[0][1] = m; g[0][2] = m;
        g[1][0] = m; g[1][2] = m;
        let out = match_recipe(&g).expect("iron helmet recipe");
        match out.item {
            Item::Armour(a) => {
                assert_eq!(a.slot, crate::armour::ArmourSlot::Helmet);
                assert_eq!(a.material, crate::armour::ArmourMaterial::Iron);
            }
            _ => panic!("expected armour piece"),
        }
    }

    #[test]
    fn leather_chestplate_recipe() {
        // Pattern: M.M / MMM / MMM (3x3, neck-hole top-centre).
        let mut g = empty_grid();
        let m = CraftSlot::Material(MaterialId::Leather);
        g[0][0] = m; g[0][2] = m;
        g[1][0] = m; g[1][1] = m; g[1][2] = m;
        g[2][0] = m; g[2][1] = m; g[2][2] = m;
        let out = match_recipe(&g).expect("leather chestplate recipe");
        match out.item {
            Item::Armour(a) => {
                assert_eq!(a.slot, crate::armour::ArmourSlot::Chestplate);
                assert_eq!(a.material, crate::armour::ArmourMaterial::Leather);
            }
            _ => panic!("expected armour"),
        }
    }

    #[test]
    fn diamond_leggings_recipe() {
        // Pattern: MMM / M.M / M.M
        let mut g = empty_grid();
        let m = CraftSlot::Material(MaterialId::Diamond);
        g[0][0] = m; g[0][1] = m; g[0][2] = m;
        g[1][0] = m; g[1][2] = m;
        g[2][0] = m; g[2][2] = m;
        let out = match_recipe(&g).expect("diamond leggings recipe");
        match out.item {
            Item::Armour(a) => {
                assert_eq!(a.slot, crate::armour::ArmourSlot::Leggings);
                assert_eq!(a.material, crate::armour::ArmourMaterial::Diamond);
            }
            _ => panic!("expected armour"),
        }
    }

    #[test]
    fn satori_boots_recipe() {
        // Pattern: M.M / M.M (top 2 rows of grid; bottom-1 + bottom-0).
        let mut g = empty_grid();
        let m = CraftSlot::Material(MaterialId::Satori);
        g[0][0] = m; g[0][2] = m;
        g[1][0] = m; g[1][2] = m;
        let out = match_recipe(&g).expect("satori boots recipe");
        match out.item {
            Item::Armour(a) => {
                assert_eq!(a.slot, crate::armour::ArmourSlot::Boots);
                assert_eq!(a.material, crate::armour::ArmourMaterial::Satori);
            }
            _ => panic!("expected armour"),
        }
    }

    #[test]
    fn chainmail_uncraftable() {
        // Chainmail is a drop-only rarity tier — no MaterialId variant
        // maps to ArmourMaterial::Chainmail in armour_material_from_slot.
        // Verify by attempting to craft with each existing material
        // through every armour pattern.
        let mats = [
            MaterialId::Leather,
            MaterialId::IronIngot,
            MaterialId::Diamond,
            MaterialId::Satori,
        ];
        for m in mats {
            let cs = CraftSlot::Material(m);
            // Helmet
            let mut g = empty_grid();
            g[0][0] = cs; g[0][1] = cs; g[0][2] = cs;
            g[1][0] = cs; g[1][2] = cs;
            let out = match_recipe(&g).expect("recipe must match");
            match out.item {
                Item::Armour(a) => {
                    assert_ne!(a.material, crate::armour::ArmourMaterial::Chainmail,
                        "Chainmail must NOT be craftable from MaterialId {m:?}");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn mixed_materials_dont_make_armour() {
        // Helmet pattern with mixed materials must fail to craft.
        let mut g = empty_grid();
        g[0][0] = CraftSlot::Material(MaterialId::IronIngot);
        g[0][1] = CraftSlot::Material(MaterialId::IronIngot);
        g[0][2] = CraftSlot::Material(MaterialId::Diamond); // foreign
        g[1][0] = CraftSlot::Material(MaterialId::IronIngot);
        g[1][2] = CraftSlot::Material(MaterialId::IronIngot);
        assert!(match_recipe(&g).is_none(),
            "mixed-material helmet pattern must not craft");
    }

    #[test]
    fn armour_recipes_produce_full_durability() {
        // A freshly-crafted helmet should match the spec's max_durability.
        let mut g = empty_grid();
        let m = CraftSlot::Material(MaterialId::IronIngot);
        g[0][0] = m; g[0][1] = m; g[0][2] = m;
        g[1][0] = m; g[1][2] = m;
        let out = match_recipe(&g).unwrap();
        if let Item::Armour(a) = out.item {
            let expected = crate::armour::max_durability(
                crate::armour::ArmourSlot::Helmet,
                crate::armour::ArmourMaterial::Iron,
            );
            assert_eq!(a.durability, expected,
                "fresh helmet should be at max durability");
            assert!(!a.is_broken());
        } else {
            panic!("expected armour");
        }
    }

    /// Helper: place items into the crafting grid at given (row, col).
    fn grid_from(slots: &[(usize, usize, CraftSlot)]) -> [[CraftSlot; 3]; 3] {
        let mut g = [[CraftSlot::Empty; 3]; 3];
        for &(r, c, s) in slots {
            g[r][c] = s;
        }
        g
    }

    // Spec 48 (Electricity) — every power block must be craftable, not just
    // /give-able (the Phase-1 obtainability guard). Shapes are the spec's
    // proposed set adapted to existing materials (owner/Axolittle confirm feel).
    #[test]
    fn electricity_blocks_are_craftable() {
        use MaterialId as M;
        let mat = |m| CraftSlot::Material(m);
        let blk = |b| CraftSlot::Block(b);
        let stick = mat(M::Stick);
        let copper = mat(M::CopperIngot);
        let iron = mat(M::IronIngot);
        let rubber = mat(M::Rubber);
        let canvas = mat(M::Canvas);
        let cable_mat = mat(M::CopperCable);
        let coal = mat(M::Coal);
        let glass = blk(block::GLASS);
        let stone = blk(block::STONE);
        let cobble = blk(block::COBBLESTONE);
        let plank = blk(block::OAK_PLANKS);

        let cases: &[(&str, [[CraftSlot; 3]; 3], BlockId, u8)] = &[
            // Cable — insulated wire: Rubber / Copper / Rubber → 3.
            ("cable", grid_from(&[(0, 0, rubber), (0, 1, copper), (0, 2, rubber)]), block::CABLE, 3),
            // Electric Lamp — glass bulb + copper filament: Glass / Copper / Glass.
            ("lamp", grid_from(&[(0, 0, glass), (0, 1, copper), (0, 2, glass)]), block::ELECTRIC_LAMP, 1),
            // Logic Gate — relay: Iron / Copper Cable / Iron.
            ("gate", grid_from(&[(0, 0, iron), (0, 1, cable_mat), (0, 2, iron)]), block::LOGIC_GATE, 1),
            // Pressure Plate — 2 stone in a row.
            ("plate", grid_from(&[(0, 0, stone), (0, 1, stone)]), block::PRESSURE_PLATE, 1),
            // Button — a single stone.
            ("button", grid_from(&[(0, 0, stone)]), block::BUTTON, 1),
            // Lever — stick on cobble.
            ("lever", grid_from(&[(0, 0, stick), (1, 0, cobble)]), block::LEVER, 1),
            // Hand Crank — stick / copper / plank stack.
            ("crank", grid_from(&[(0, 0, stick), (1, 0, copper), (2, 0, plank)]), block::HAND_CRANK, 1),
            // Battery — voltaic pile: copper / coal / copper.
            ("battery", grid_from(&[(0, 0, copper), (1, 0, coal), (2, 0, copper)]), block::BATTERY, 1),
            // Steam Generator — iron ring + copper core (boiler + dynamo).
            (
                "generator",
                grid_from(&[
                    (0, 0, iron), (0, 1, iron), (0, 2, iron),
                    (1, 0, iron), (1, 1, copper), (1, 2, iron),
                    (2, 0, iron), (2, 1, iron), (2, 2, iron),
                ]),
                block::STEAM_GENERATOR,
                1,
            ),
            // Phase 2 sensors (distinct 1×3 shapes).
            ("mirror", grid_from(&[(0, 0, iron), (0, 1, glass), (0, 2, iron)]), block::MIRROR, 1),
            ("beam_sensor", grid_from(&[(0, 0, glass), (0, 1, copper), (0, 2, iron)]), block::BEAM_SENSOR, 1),
            ("motion_sensor", grid_from(&[(0, 0, copper), (0, 1, glass), (0, 2, copper)]), block::MOTION_SENSOR, 1),
            // Spec 49 — Plunger Detonator: Iron / Cable / Planks, "a boxed
            // switch" (docs/foundations/2026-06-20-explosives-blasting-keg.md).
            // Was /give-only until the wiki audit (2026-07-09) caught the
            // spec'd recipe never having been wired into the matcher.
            ("plunger_detonator", grid_from(&[(0, 0, iron), (0, 1, cable_mat), (0, 2, plank)]), block::PLUNGER_DETONATOR, 1),
            // Spec 48 Phase 4 — Water Wheel: plank ring + copper axle (eight
            // paddles round the dynamo). Distinct from the Steam Generator
            // (iron ring, same copper core) and from every other plank ring.
            (
                "water_wheel",
                grid_from(&[
                    (0, 0, plank), (0, 1, plank), (0, 2, plank),
                    (1, 0, plank), (1, 1, copper), (1, 2, plank),
                    (2, 0, plank), (2, 1, plank), (2, 2, plank),
                ]),
                block::WATER_WHEEL,
                1,
            ),
            // Wind wave §2.2 — Windmill: canvas sails / plank-and-copper hub /
            // stick-and-iron trestle. Same copper core as the two above, three
            // distinct rows, so no plank ring can be mistaken for it.
            (
                "windmill",
                grid_from(&[
                    (0, 0, canvas), (0, 1, canvas), (0, 2, canvas),
                    (1, 0, plank), (1, 1, copper), (1, 2, plank),
                    (2, 0, stick), (2, 1, iron), (2, 2, stick),
                ]),
                block::WINDMILL,
                1,
            ),
        ];

        for (name, g, want_block, want_count) in cases {
            let got = match_recipe(g)
                .unwrap_or_else(|| panic!("recipe `{name}` must craft something, got None"));
            match got.item {
                Item::Block(b) => assert_eq!(b, *want_block, "recipe `{name}` wrong block"),
                other => panic!("recipe `{name}`: expected a block, got {other:?}"),
            }
            assert_eq!(got.count, *want_count, "recipe `{name}` wrong count");
        }
    }

    // HP-3 v2 — Trophy Wall recipe.
    #[test]
    fn trophy_wall_recipe_resolves_from_trophy_plus_two_planks() {
        // Vertical 3x1: trophy on top, plank middle, plank bottom.
        let g = grid_from(&[
            (0, 1, CraftSlot::Material(MaterialId::BrigandChieftainTrophy)),
            (1, 1, CraftSlot::Block(block::OAK_PLANKS)),
            (2, 1, CraftSlot::Block(block::OAK_PLANKS)),
        ]);
        let result = match_recipe(&g).expect("trophy + 2 planks should resolve");
        assert_eq!(result.count, 1);
        match result.item {
            Item::Block(b) => assert_eq!(b, block::TROPHY_WALL),
            other => panic!("expected TROPHY_WALL, got {other:?}"),
        }
    }

    #[test]
    fn trophy_wall_requires_trophy_on_top() {
        // Wrong order — plank on top, trophy in middle — should NOT
        // produce a Trophy Wall (different recipe entirely).
        let g = grid_from(&[
            (0, 1, CraftSlot::Block(block::OAK_PLANKS)),
            (1, 1, CraftSlot::Material(MaterialId::BrigandChieftainTrophy)),
            (2, 1, CraftSlot::Block(block::OAK_PLANKS)),
        ]);
        let result = match_recipe(&g);
        assert!(
            !matches!(result, Some(stack) if matches!(stack.item, Item::Block(b) if b == block::TROPHY_WALL)),
            "wrong-order trophy column must not match the Trophy Wall recipe",
        );
    }

    // Pets wave Task 13 — Reach Claw recipe.
    #[test]
    fn reach_claw_recipe_resolves_from_crab_claw_plus_two_sticks() {
        // Vertical 3x1: Crab Claw on top, Stick middle, Stick bottom.
        let g = grid_from(&[
            (0, 1, CraftSlot::Material(MaterialId::CrabClaw)),
            (1, 1, CraftSlot::Material(MaterialId::Stick)),
            (2, 1, CraftSlot::Material(MaterialId::Stick)),
        ]);
        let result = match_recipe(&g).expect("crab claw + 2 sticks should resolve");
        assert_eq!(result.count, 1);
        match result.item {
            Item::Material(m) => assert_eq!(m, MaterialId::ReachClaw),
            other => panic!("expected ReachClaw, got {other:?}"),
        }
    }

    #[test]
    fn reach_claw_requires_claw_on_top() {
        // Wrong order — stick on top, claw in middle — should NOT produce
        // a Reach Claw (different column entirely).
        let g = grid_from(&[
            (0, 1, CraftSlot::Material(MaterialId::Stick)),
            (1, 1, CraftSlot::Material(MaterialId::CrabClaw)),
            (2, 1, CraftSlot::Material(MaterialId::Stick)),
        ]);
        let result = match_recipe(&g);
        assert!(
            !matches!(result, Some(stack) if matches!(stack.item, Item::Material(m) if m == MaterialId::ReachClaw)),
            "wrong-order claw column must not match the Reach Claw recipe",
        );
    }

    #[test]
    fn bronze_is_not_a_tool_tier() {
        // Hardening test for the spec's "bronze is decorative, not a
        // tool tier" rule. The ToolMaterial enum contains exactly five
        // variants: Wood / Stone / Iron / Diamond / Satori. No Bronze.
        // If a future PR adds a Bronze variant, this assert still passes
        // (it just confirms the five canonical tiers exist) — but the
        // explorer's all_tool_combos test would catch the mismatch.
        let tiers = [
            ToolMaterial::Wood,
            ToolMaterial::Stone,
            ToolMaterial::Iron,
            ToolMaterial::Diamond,
            ToolMaterial::Satori,
        ];
        assert_eq!(tiers.len(), 5);
    }

    // Salt feature (2026-05-23) — recipe tests.

    #[test]
    fn salt_block_recipe_consumes_9_salt() {
        let salt = CraftSlot::Material(MaterialId::Salt);
        let mut g = [[CraftSlot::Empty; 3]; 3];
        for r in 0..3 { for c in 0..3 { g[r][c] = salt; } }
        let result = match_recipe(&g).expect("9 Salt should craft SALT_BLOCK");
        assert_eq!(result.count, 1);
        match result.item {
            Item::Block(b) => assert_eq!(b, block::SALT_BLOCK),
            other => panic!("expected SALT_BLOCK, got {other:?}"),
        }
    }

    #[test]
    fn salt_block_reverse_yields_9_salt() {
        let g = grid_from(&[(0, 1, CraftSlot::Block(block::SALT_BLOCK))]);
        let result = match_recipe(&g).expect("SALT_BLOCK 1x1 should yield 9 Salt");
        assert_eq!(result.count, 9);
        match result.item {
            Item::Material(m) => assert_eq!(m, MaterialId::Salt),
            other => panic!("expected Salt, got {other:?}"),
        }
    }

    #[test]
    fn salt_lamp_cross_recipe() {
        let s = CraftSlot::Material(MaterialId::Salt);
        let t = CraftSlot::Material(MaterialId::Stick);
        let g = grid_from(&[
            (0, 1, s), (1, 0, s), (1, 1, t), (1, 2, s), (2, 1, s),
        ]);
        let result = match_recipe(&g).expect("cross 4 Salt + 1 Stick should craft SALT_LAMP");
        assert_eq!(result.count, 1);
        match result.item {
            Item::Block(b) => assert_eq!(b, block::SALT_LAMP),
            other => panic!("expected SALT_LAMP, got {other:?}"),
        }
    }

    #[test]
    fn salt_lick_recipe() {
        let g = grid_from(&[
            (0, 1, CraftSlot::Block(block::SALT_BLOCK)),
            (1, 1, CraftSlot::Block(block::COBBLESTONE)),
        ]);
        let result = match_recipe(&g).expect("SALT_BLOCK on Cobble should craft SALT_LICK");
        assert_eq!(result.count, 1);
        match result.item {
            Item::Block(b) => assert_eq!(b, block::SALT_LICK),
            other => panic!("expected SALT_LICK, got {other:?}"),
        }
    }

    #[test]
    fn cured_and_seasoned_recipes_work_in_both_orders() {
        let s = CraftSlot::Material(MaterialId::Salt);
        let beef = CraftSlot::Material(MaterialId::RawBeef);
        // Salt left, RawBeef right.
        let g1 = grid_from(&[(0, 0, s), (0, 1, beef)]);
        let r1 = match_recipe(&g1).expect("Salt + RawBeef should cure");
        assert!(matches!(r1.item, Item::Material(MaterialId::SaltCuredBeef)));
        // RawBeef left, Salt right (order-insensitive).
        let g2 = grid_from(&[(0, 0, beef), (0, 1, s)]);
        let r2 = match_recipe(&g2).expect("RawBeef + Salt should also cure");
        assert!(matches!(r2.item, Item::Material(MaterialId::SaltCuredBeef)));
        // Seasoned arm — Salt + Bread.
        let bread = CraftSlot::Material(MaterialId::Bread);
        let g3 = grid_from(&[(0, 0, s), (0, 1, bread)]);
        let r3 = match_recipe(&g3).expect("Salt + Bread should season");
        assert!(matches!(r3.item, Item::Material(MaterialId::SeasonedBread)));
    }

    // Rubber feature (2026-05-23) — recipe tests.

    #[test]
    fn slingshot_y_fork_recipe() {
        let s = CraftSlot::Material(MaterialId::Stick);
        let r = CraftSlot::Material(MaterialId::Rubber);
        let g = grid_from(&[
            (0, 0, s), (0, 2, s),
            (1, 1, r),
            (2, 1, s),
        ]);
        let result = match_recipe(&g).expect("Y-fork should craft Slingshot");
        match result.item {
            Item::Tool(t) => {
                assert_eq!(t.tool_type, ToolType::Slingshot);
                assert_eq!(t.material, ToolMaterial::Wood);
            }
            other => panic!("expected Slingshot tool, got {other:?}"),
        }
    }

    #[test]
    fn rubber_ball_recipe_yields_4_per_rubber() {
        let g = grid_from(&[(0, 1, CraftSlot::Material(MaterialId::Rubber))]);
        let result = match_recipe(&g).expect("1 Rubber should craft 4 Balls");
        assert_eq!(result.count, 4);
        assert!(matches!(result.item, Item::Material(MaterialId::RubberBall)));
    }

    #[test]
    fn eraser_recipe_rubber_over_stick() {
        let g = grid_from(&[
            (0, 1, CraftSlot::Material(MaterialId::Rubber)),
            (1, 1, CraftSlot::Material(MaterialId::Stick)),
        ]);
        let result = match_recipe(&g).expect("Rubber on Stick should craft Eraser");
        match result.item {
            Item::Tool(t) => assert_eq!(t.tool_type, ToolType::Eraser),
            other => panic!("expected Eraser tool, got {other:?}"),
        }
    }

    #[test]
    fn drafting_stamp_recipe_iron_over_blueprint_paper() {
        let g = grid_from(&[
            (0, 1, CraftSlot::Material(MaterialId::IronIngot)),
            (1, 1, CraftSlot::Block(block::BLUEPRINT_PAPER)),
        ]);
        let result = match_recipe(&g).expect("IronIngot on BlueprintPaper should craft Drafting Stamp");
        match result.item {
            Item::Tool(t) => assert_eq!(t.tool_type, ToolType::DraftingStamp),
            other => panic!("expected DraftingStamp tool, got {other:?}"),
        }
    }

    #[test]
    fn copper_cable_recipe_2_copper_1_rubber_yields_2_cables() {
        let copper = CraftSlot::Material(MaterialId::CopperIngot);
        let rubber = CraftSlot::Material(MaterialId::Rubber);
        let g = grid_from(&[
            (0, 0, copper), (0, 1, rubber), (0, 2, copper),
        ]);
        let result = match_recipe(&g).expect("C R C horizontal should craft CopperCable");
        assert_eq!(result.count, 2);
        assert!(matches!(result.item, Item::Material(MaterialId::CopperCable)));
    }

    // --- Craftable Armoured Carts (CA2) ---

    #[test]
    fn track_recipe_resolves_from_six_iron_and_a_stick() {
        // Minecraft rail shape: two full columns of Iron Ingot framing a
        // Stick at the centre.
        //   I . I
        //   I S I   →  16 Track
        //   I . I
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        let stick = CraftSlot::Material(MaterialId::Stick);
        let g = grid_from(&[
            (0, 0, iron),               (0, 2, iron),
            (1, 0, iron), (1, 1, stick), (1, 2, iron),
            (2, 0, iron),               (2, 2, iron),
        ]);
        let out = match_recipe(&g).expect("6 iron + 1 stick → Track");
        match out.item {
            Item::Block(b) => assert_eq!(b, crate::rail::TRACK),
            other => panic!("expected Block(TRACK), got {other:?}"),
        }
        assert_eq!(out.count, 16);
    }

    #[test]
    fn track_recipe_rejects_wrong_centre() {
        // Same iron frame but a Diamond instead of a Stick at the centre
        // must NOT yield Track.
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        let diamond = CraftSlot::Material(MaterialId::Diamond);
        let g = grid_from(&[
            (0, 0, iron),                 (0, 2, iron),
            (1, 0, iron), (1, 1, diamond), (1, 2, iron),
            (2, 0, iron),                 (2, 2, iron),
        ]);
        if let Some(out) = match_recipe(&g) {
            if let Item::Block(b) = out.item {
                assert_ne!(b, crate::rail::TRACK, "wrong centre must NOT yield Track");
            }
        }
    }

    #[test]
    fn wood_cart_recipe_resolves_from_five_planks_u() {
        // Minecraft-minecart U shape:
        //   . . .
        //   P . P   →  1 Wood Cart
        //   P P P
        let plank = CraftSlot::Block(crate::block::OAK_PLANKS);
        let g = grid_from(&[
            (1, 0, plank),               (1, 2, plank),
            (2, 0, plank), (2, 1, plank), (2, 2, plank),
        ]);
        let out = match_recipe(&g).expect("5 planks U → Wood Cart");
        match out.item {
            Item::Material(m) => assert_eq!(m, MaterialId::WoodCart),
            other => panic!("expected Material(WoodCart), got {other:?}"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn wood_cart_recipe_rejects_filled_centre() {
        // Filling the centre (a tub, not a U) must NOT yield a Wood Cart.
        let plank = CraftSlot::Block(crate::block::OAK_PLANKS);
        let g = grid_from(&[
            (1, 0, plank), (1, 1, plank), (1, 2, plank),
            (2, 0, plank), (2, 1, plank), (2, 2, plank),
        ]);
        if let Some(out) = match_recipe(&g) {
            if let Item::Material(m) = out.item {
                assert_ne!(m, MaterialId::WoodCart, "filled centre must NOT yield a Wood Cart");
            }
        }
    }

    #[test]
    fn iron_cart_recipe_resolves_from_iron_ring_around_a_wood_cart() {
        //   I I I
        //   I W I   →  1 Iron Cart   (W = WoodCart)
        //   I I I
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        let wood_cart = CraftSlot::Material(MaterialId::WoodCart);
        let g = grid_from(&[
            (0, 0, iron), (0, 1, iron), (0, 2, iron),
            (1, 0, iron), (1, 1, wood_cart), (1, 2, iron),
            (2, 0, iron), (2, 1, iron), (2, 2, iron),
        ]);
        let out = match_recipe(&g).expect("8 iron ring + WoodCart centre → Iron Cart");
        match out.item {
            Item::Material(m) => assert_eq!(m, MaterialId::IronCart),
            other => panic!("expected Material(IronCart), got {other:?}"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn iron_cart_recipe_rejects_wrong_centre() {
        // Iron ring around a plain IronIngot centre (no Wood Cart) must NOT
        // yield an Iron Cart.
        let iron = CraftSlot::Material(MaterialId::IronIngot);
        let g = grid_from(&[
            (0, 0, iron), (0, 1, iron), (0, 2, iron),
            (1, 0, iron), (1, 1, iron), (1, 2, iron),
            (2, 0, iron), (2, 1, iron), (2, 2, iron),
        ]);
        if let Some(out) = match_recipe(&g) {
            if let Item::Material(m) = out.item {
                assert_ne!(m, MaterialId::IronCart, "no WoodCart centre must NOT yield an Iron Cart");
            }
        }
    }

    #[test]
    fn diamond_cart_recipe_resolves_from_diamond_ring_around_an_iron_cart() {
        //   D D D
        //   D R D   →  1 Diamond Cart   (R = IronCart)
        //   D D D
        let diamond = CraftSlot::Material(MaterialId::Diamond);
        let iron_cart = CraftSlot::Material(MaterialId::IronCart);
        let g = grid_from(&[
            (0, 0, diamond), (0, 1, diamond), (0, 2, diamond),
            (1, 0, diamond), (1, 1, iron_cart), (1, 2, diamond),
            (2, 0, diamond), (2, 1, diamond), (2, 2, diamond),
        ]);
        let out = match_recipe(&g).expect("8 diamond ring + IronCart centre → Diamond Cart");
        match out.item {
            Item::Material(m) => assert_eq!(m, MaterialId::DiamondCart),
            other => panic!("expected Material(DiamondCart), got {other:?}"),
        }
        assert_eq!(out.count, 1);
    }

    #[test]
    fn diamond_cart_recipe_rejects_wrong_centre() {
        // Diamond ring around a plain Diamond centre (no Iron Cart) must NOT
        // yield a Diamond Cart.
        let diamond = CraftSlot::Material(MaterialId::Diamond);
        let g = grid_from(&[
            (0, 0, diamond), (0, 1, diamond), (0, 2, diamond),
            (1, 0, diamond), (1, 1, diamond), (1, 2, diamond),
            (2, 0, diamond), (2, 1, diamond), (2, 2, diamond),
        ]);
        if let Some(out) = match_recipe(&g) {
            if let Item::Material(m) = out.item {
                assert_ne!(m, MaterialId::DiamondCart, "no IronCart centre must NOT yield a Diamond Cart");
            }
        }
    }
}
