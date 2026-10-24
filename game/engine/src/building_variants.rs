//! Building variants — slabs, stairs, doors, trapdoors, fence gates and walls
//! in every wood and stone material the game already has.
//!
//! One data table ([`MATERIALS`]) drives everything: the block ids, registry
//! names, textures (copied from the material's base block, so no new texture
//! layer), mining numbers, flammability, fuel value, recipes and `/give` names.
//! Adding a material is one row; adding a shape to a family is one entry in
//! its shape list.
//!
//! **Ids.** The six variants that predate this table keep the ids they were
//! minted with (`STONE_SLAB`, `STONE_STAIRS`, `OAK_FENCE_GATE`, `OAK_TRAPDOOR`,
//! `OAK_DOOR`, `COBBLESTONE_WALL`). Every other (material, shape) pair takes
//! the next id from [`FIRST_NEW_ID`], in table order. The table is
//! append-only: never reorder materials or shapes, add new rows at the end.
//!
//! **State.** A variant stores exactly what its oak/stone original does (see
//! `block_shape`): orientation, half and open bits in the meta byte, so each
//! (material, shape) is ONE id. A vertical slab is a slab with a side facing.
//!
//! The world generator never places any of these (the v2 worldgen golden pins
//! that); players craft or `/give` them.

use std::sync::OnceLock;

use crate::block::{self, BlockId};
use crate::block_shape::BlockShape;
use crate::crafting::{ToolMaterial, ToolType};

/// The shapes a material can come in. Doors, trapdoors and gates are wood;
/// walls are stone; slabs and stairs are both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VShape {
    Slab,
    Stairs,
    Door,
    Trapdoor,
    FenceGate,
    Wall,
}

impl VShape {
    /// Name suffix: `spruce_` + `stairs`. Also the `/give` suffix.
    pub fn suffix(self) -> &'static str {
        match self {
            VShape::Slab => "slab",
            VShape::Stairs => "stairs",
            VShape::Door => "door",
            VShape::Trapdoor => "trapdoor",
            VShape::FenceGate => "fence_gate",
            VShape::Wall => "wall",
        }
    }

    /// The geometry family `block_shape::shape_of` reports.
    pub const fn block_shape(self) -> BlockShape {
        match self {
            VShape::Slab => BlockShape::Slab,
            VShape::Stairs => BlockShape::Stairs,
            VShape::Door => BlockShape::Door,
            VShape::Trapdoor => BlockShape::Trapdoor,
            VShape::FenceGate => BlockShape::FenceGate,
            VShape::Wall => BlockShape::Wall,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Wood,
    Stone,
}

/// Shapes per family, in id order.
pub const WOOD_SHAPES: [VShape; 5] =
    [VShape::Slab, VShape::Stairs, VShape::Door, VShape::Trapdoor, VShape::FenceGate];
pub const STONE_SHAPES: [VShape; 3] = [VShape::Slab, VShape::Stairs, VShape::Wall];

impl Family {
    pub const fn shapes(self) -> &'static [VShape] {
        match self {
            Family::Wood => &WOOD_SHAPES,
            Family::Stone => &STONE_SHAPES,
        }
    }
}

/// One row: a material every shape of its family is made from.
pub struct Material {
    /// Registry/`/give` key: `genesis:{key}_{shape}`.
    pub key: &'static str,
    /// The full-cube block it is made from (planks, or the stone itself). Its
    /// texture layers and colour are copied; it is also the recipe ingredient.
    pub base: BlockId,
    pub family: Family,
    /// Seconds to break with a bare fist (`crafting::block_hardness`).
    pub hardness: f32,
    /// Lowest pickaxe that gets a drop (stone family only; wood drops for any
    /// tool).
    pub min_tier: Option<ToolMaterial>,
}

/// Indices into [`MATERIALS`] the pre-table variants hang on.
pub const OAK: usize = 0;
pub const STONE: usize = 7;
pub const COBBLESTONE: usize = 8;

/// Every material. APPEND-ONLY: a variant's id is a function of its position.
const TABLE: [Material; 15] = [
    // Woods: one per `block::WoodSpecies`, in that enum's order.
    wood("oak", block::OAK_PLANKS),
    wood("birch", block::BIRCH_PLANKS),
    wood("spruce", block::SPRUCE_PLANKS),
    wood("jungle", block::JUNGLE_PLANKS),
    wood("acacia", block::ACACIA_PLANKS),
    wood("dark_oak", block::DARK_OAK_PLANKS),
    wood("rubber", block::RUBBER_PLANKS),
    // Stones. Mined like cobblestone (a stone slab is not the 10 s full block).
    stone("stone", block::STONE, 2.0, Some(ToolMaterial::Wood)),
    stone("cobblestone", block::COBBLESTONE, 2.0, Some(ToolMaterial::Wood)),
    stone("sandstone", block::SANDSTONE, 0.8, Some(ToolMaterial::Wood)),
    stone("granite", block::GRANITE, 2.0, Some(ToolMaterial::Wood)),
    stone("marble", block::MARBLE, 2.0, Some(ToolMaterial::Wood)),
    stone("limestone", block::LIMESTONE, 2.0, Some(ToolMaterial::Wood)),
    stone("slate", block::SLATE, 2.0, Some(ToolMaterial::Wood)),
    // Harder than stone, like the full block (but no Satori roll: only the
    // pure-deepslate family has one).
    stone("deepslate", block::PURE_DEEPSLATE, 4.5, Some(ToolMaterial::Stone)),
];

/// The table as a static, so lookups can hand out `&'static Material`.
pub static MATERIALS: [Material; 15] = TABLE;

const fn wood(key: &'static str, base: BlockId) -> Material {
    Material { key, base, family: Family::Wood, hardness: 2.0, min_tier: None }
}
const fn stone(key: &'static str, base: BlockId, hardness: f32, min_tier: Option<ToolMaterial>) -> Material {
    Material { key, base, family: Family::Stone, hardness, min_tier }
}

/// The first id this table mints (the id after `WINDMILL_TURNING`).
pub const FIRST_NEW_ID: BlockId = 320;

/// The id a pre-table (material, shape) pair already has, if any.
const fn existing_id(mat: usize, shape: VShape) -> Option<BlockId> {
    match (mat, shape) {
        (STONE, VShape::Slab) => Some(block::STONE_SLAB),
        (STONE, VShape::Stairs) => Some(block::STONE_STAIRS),
        (OAK, VShape::FenceGate) => Some(block::OAK_FENCE_GATE),
        (OAK, VShape::Trapdoor) => Some(block::OAK_TRAPDOOR),
        (OAK, VShape::Door) => Some(block::OAK_DOOR),
        (COBBLESTONE, VShape::Wall) => Some(block::COBBLESTONE_WALL),
        _ => None,
    }
}

/// How many ids this table mints: 7 woods x 5 shapes + 8 stones x 3 shapes,
/// less the 6 that already existed.
pub const NEW_COUNT: usize = 7 * 5 + 8 * 3 - 6;
/// One past the last id this table mints.
pub const END_NEW_ID: BlockId = FIRST_NEW_ID + NEW_COUNT as BlockId;

#[derive(Clone, Copy)]
struct Entry {
    mat: u8,
    shape: VShape,
}

/// New ids in order: `NEW[id - FIRST_NEW_ID]`.
const NEW: [Entry; NEW_COUNT] = build_new();

const fn build_new() -> [Entry; NEW_COUNT] {
    let mut out = [Entry { mat: 0, shape: VShape::Slab }; NEW_COUNT];
    let mut n = 0;
    let mut m = 0;
    while m < TABLE.len() {
        let shapes = TABLE[m].family.shapes();
        let mut s = 0;
        while s < shapes.len() {
            if existing_id(m, shapes[s]).is_none() {
                out[n] = Entry { mat: m as u8, shape: shapes[s] };
                n += 1;
            }
            s += 1;
        }
        m += 1;
    }
    assert!(n == NEW_COUNT);
    out
}

/// What a variant block is: its material row and shape.
pub fn info(id: BlockId) -> Option<(&'static Material, VShape)> {
    if (FIRST_NEW_ID..END_NEW_ID).contains(&id) {
        let e = NEW[(id - FIRST_NEW_ID) as usize];
        return Some((&MATERIALS[e.mat as usize], e.shape));
    }
    let (mat, shape) = match id {
        block::STONE_SLAB => (STONE, VShape::Slab),
        block::STONE_STAIRS => (STONE, VShape::Stairs),
        block::OAK_FENCE_GATE => (OAK, VShape::FenceGate),
        block::OAK_TRAPDOOR => (OAK, VShape::Trapdoor),
        block::OAK_DOOR => (OAK, VShape::Door),
        block::COBBLESTONE_WALL => (COBBLESTONE, VShape::Wall),
        _ => return None,
    };
    Some((&MATERIALS[mat], shape))
}

/// The geometry family of a NEW id (the six old ids keep their own arms in
/// `block_shape::shape_of`). `None` outside the minted range. O(1).
pub fn new_block_shape(id: BlockId) -> Option<BlockShape> {
    if (FIRST_NEW_ID..END_NEW_ID).contains(&id) {
        Some(NEW[(id - FIRST_NEW_ID) as usize].shape.block_shape())
    } else {
        None
    }
}

/// The block id for (material index, shape); `None` if the family has no such
/// shape.
pub fn id_of(mat: usize, shape: VShape) -> Option<BlockId> {
    if !MATERIALS[mat].family.shapes().iter().any(|s| *s == shape) {
        return None;
    }
    if let Some(id) = existing_id(mat, shape) {
        return Some(id);
    }
    NEW.iter()
        .position(|e| e.mat as usize == mat && e.shape == shape)
        .map(|i| FIRST_NEW_ID + i as BlockId)
}

/// The id for a `/give`-style name: `spruce_stairs`, `dark_oak_fence_gate`,
/// `sandstone_slab`.
pub fn id_by_name(key: &str) -> Option<BlockId> {
    for (mi, m) in MATERIALS.iter().enumerate() {
        let Some(rest) = key.strip_prefix(m.key).and_then(|r| r.strip_prefix('_')) else { continue };
        for shape in m.family.shapes() {
            if rest == shape.suffix() || (*shape == VShape::FenceGate && rest == "gate") {
                return id_of(mi, *shape);
            }
        }
    }
    None
}

/// Every (id, material row, shape), existing and new, in material then shape
/// order.
pub fn all() -> Vec<(BlockId, &'static Material, VShape)> {
    let mut out = Vec::new();
    for (mi, m) in MATERIALS.iter().enumerate() {
        for shape in m.family.shapes() {
            out.push((id_of(mi, *shape).expect("table shape has an id"), m, *shape));
        }
    }
    out
}

/// The material whose base block is `base` (planks, or the stone): the recipe
/// ingredient lookup.
pub fn material_for_base(base: BlockId) -> Option<usize> {
    MATERIALS.iter().position(|m| m.base == base)
}

/// The variant of `shape` made from the full-cube block `base`: spruce planks
/// + `Stairs` gives spruce stairs; `None` if `base` is not a table material or
/// its family has no such shape. The recipe matcher's one lookup.
pub fn id_for_base(base: BlockId, shape: VShape) -> Option<BlockId> {
    id_of(material_for_base(base)?, shape)
}

/// Registry name for a NEW id (`genesis:spruce_stairs`). Built once; the
/// strings live for the program.
pub fn new_name(id: BlockId) -> &'static str {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        NEW.iter()
            .map(|e| {
                let s = format!("genesis:{}_{}", MATERIALS[e.mat as usize].key, e.shape.suffix());
                &*Box::leak(s.into_boxed_str())
            })
            .collect()
    });
    names[(id - FIRST_NEW_ID) as usize]
}

/// The material row of a NEW id, for the registry to copy textures from.
pub fn new_material(id: BlockId) -> &'static Material {
    &MATERIALS[NEW[(id - FIRST_NEW_ID) as usize].mat as usize]
}

/// Mining speed number: seconds with a bare fist.
pub fn hardness(id: BlockId) -> Option<f32> {
    info(id).map(|(m, _)| m.hardness)
}

/// Best tool: woods want an axe, stones a pickaxe.
pub fn best_tool(id: BlockId) -> Option<ToolType> {
    info(id).map(|(m, _)| match m.family {
        Family::Wood => ToolType::Axe,
        Family::Stone => ToolType::Pickaxe,
    })
}

/// Lowest pickaxe that gets a drop; `None` = any tool (or fist).
pub fn min_tier(id: BlockId) -> Option<ToolMaterial> {
    info(id).and_then(|(m, _)| m.min_tier)
}

/// Can fire consume it? Wood variants burn, like their planks.
pub fn flammable(id: BlockId) -> bool {
    info(id).is_some_and(|(m, _)| m.family == Family::Wood)
}

/// Burn time as furnace/campfire fuel, in 20-TPS ticks. A slab is half a
/// plank block's worth (6 s); everything else matches planks (12 s). Stone
/// does not burn.
pub fn fuel_ticks(id: BlockId) -> Option<u32> {
    let (m, shape) = info(id)?;
    if m.family != Family::Wood {
        return None;
    }
    Some(if shape == VShape::Slab { 6 * 20 } else { 12 * 20 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_mints_a_contiguous_range_after_the_old_last_id() {
        assert_eq!(FIRST_NEW_ID, block::WINDMILL_TURNING + 1);
        assert_eq!(NEW_COUNT, 53);
        assert_eq!(END_NEW_ID, 373);
        assert_eq!(all().len(), 35 + 24, "7 woods x 5 + 8 stones x 3");
    }

    #[test]
    fn every_pair_has_a_unique_id_and_round_trips() {
        let mut seen = std::collections::HashSet::new();
        for (id, m, shape) in all() {
            assert!(seen.insert(id), "{} {:?} duplicates id {id}", m.key, shape);
            let (m2, s2) = info(id).expect("info for a table id");
            assert_eq!((m2.key, s2), (m.key, shape));
            assert_eq!(id_by_name(&format!("{}_{}", m.key, shape.suffix())), Some(id));
        }
    }

    #[test]
    fn old_ids_keep_their_ids() {
        assert_eq!(id_of(STONE, VShape::Slab), Some(block::STONE_SLAB));
        assert_eq!(id_of(STONE, VShape::Stairs), Some(block::STONE_STAIRS));
        assert_eq!(id_of(OAK, VShape::Door), Some(block::OAK_DOOR));
        assert_eq!(id_of(OAK, VShape::Trapdoor), Some(block::OAK_TRAPDOOR));
        assert_eq!(id_of(OAK, VShape::FenceGate), Some(block::OAK_FENCE_GATE));
        assert_eq!(id_of(COBBLESTONE, VShape::Wall), Some(block::COBBLESTONE_WALL));
        // First new id: oak's slab (oak is row 0, slab is its first shape).
        assert_eq!(id_of(OAK, VShape::Slab), Some(FIRST_NEW_ID));
    }

    #[test]
    fn families_only_have_their_own_shapes() {
        assert_eq!(id_of(OAK, VShape::Wall), None);
        assert_eq!(id_of(STONE, VShape::Door), None);
        assert_eq!(id_by_name("granite_trapdoor"), None);
        assert_eq!(id_by_name("birch_gate"), id_by_name("birch_fence_gate"));
    }

    #[test]
    fn material_rows_use_the_stated_base_blocks() {
        assert_eq!(MATERIALS[OAK].key, "oak");
        assert_eq!(MATERIALS[STONE].key, "stone");
        assert_eq!(MATERIALS[COBBLESTONE].key, "cobblestone");
        assert_eq!(MATERIALS[COBBLESTONE].base, block::COBBLESTONE);
        // Wood rows follow WoodSpecies order, base = that species' planks.
        for (i, s) in block::ALL_WOOD_SPECIES.iter().enumerate() {
            assert_eq!(MATERIALS[i].base, block::planks_block_for(*s));
        }
    }
}
