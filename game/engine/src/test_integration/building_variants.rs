//! Building variants (slabs, stairs, doors, trapdoors, fence gates, walls in
//! every wood and stone): the contract of the `building_variants` table, and
//! a TestHost craft -> place -> collide -> toggle -> break run for one wood
//! and one stone.

use crate::block::{self, BlockId, BlockRegistry};
use crate::block_shape::{self, BlockShape};
use crate::building_variants::{self as bv, Family, VShape};
use crate::commands::builtins::give::resolve_item;
use crate::crafting::{match_recipe, CraftSlot, Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack, MaterialId};
use crate::meta::{self, Facing};
use crate::strict_doors::{toggle_metas, OpenRule};
use crate::test_harness::{TestConfig, TestHost};

/// The texture-layer count before this change. Variants copy their base
/// block's layers, so it must not move.
const TEXTURE_COUNT: u32 = 511;

fn expected_shape(s: VShape) -> BlockShape {
    match s {
        VShape::Slab => BlockShape::Slab,
        VShape::Stairs => BlockShape::Stairs,
        VShape::Door => BlockShape::Door,
        VShape::Trapdoor => BlockShape::Trapdoor,
        VShape::FenceGate => BlockShape::FenceGate,
        VShape::Wall => BlockShape::Wall,
    }
}

fn mat(key: &str) -> usize {
    bv::MATERIALS.iter().position(|m| m.key == key).unwrap_or_else(|| panic!("no material {key}"))
}

fn variant(key: &str, shape: VShape) -> BlockId {
    bv::id_of(mat(key), shape).unwrap_or_else(|| panic!("no {key} {shape:?}"))
}

#[test]
fn the_matrix_is_seven_woods_by_five_and_eight_stones_by_three() {
    let all = bv::all();
    let wood: Vec<_> = all.iter().filter(|(_, m, _)| m.family == Family::Wood).collect();
    let stone: Vec<_> = all.iter().filter(|(_, m, _)| m.family == Family::Stone).collect();
    assert_eq!((wood.len(), stone.len()), (35, 24));
    assert_eq!(bv::NEW_COUNT, 53, "59 pairs less the 6 that already had ids");
    for k in ["oak", "birch", "spruce", "jungle", "acacia", "dark_oak", "rubber"] {
        for s in bv::WOOD_SHAPES {
            assert!(bv::id_of(mat(k), s).is_some(), "{k} {s:?}");
        }
        assert!(bv::id_of(mat(k), VShape::Wall).is_none());
    }
    for k in ["stone", "cobblestone", "sandstone", "granite", "marble", "limestone", "slate", "deepslate"] {
        for s in bv::STONE_SHAPES {
            assert!(bv::id_of(mat(k), s).is_some(), "{k} {s:?}");
        }
        assert!(bv::id_of(mat(k), VShape::Door).is_none());
    }
}

#[test]
fn every_variant_is_registered_with_its_shape_and_its_materials_textures() {
    let reg = BlockRegistry::new();
    let mut names = std::collections::HashSet::new();
    for (id, m, shape) in bv::all() {
        assert!(reg.is_known(id), "{} {shape:?} (id {id}) is not in the registry", m.key);
        assert_eq!(block_shape::shape_of(id), expected_shape(shape), "{} {shape:?}", m.key);
        assert!(block_shape::is_shaped(id));
        let toggles = matches!(shape, VShape::Door | VShape::Trapdoor | VShape::FenceGate);
        assert_eq!(block_shape::is_toggleable(id), toggles, "{} {shape:?}", m.key);
        assert_eq!(block_shape::is_door(id), shape == VShape::Door);
        let def = reg.get(id);
        let base = reg.get(m.base);
        assert!(def.solid && def.transparent && !def.gravity, "shaped like the originals");
        assert_eq!((def.tex_top, def.tex_bottom, def.tex_side), (base.tex_top, base.tex_bottom, base.tex_side), "{} reuses its base's layers", def.name);
        assert!(names.insert(def.name), "duplicate registry name {}", def.name);
        assert_eq!(def.name, format!("genesis:{}_{}", m.key, shape.suffix()));
    }
    // Appended after the old last id: nothing renumbered.
    assert_eq!(reg.len(), bv::END_NEW_ID as usize);
    assert_eq!(reg.get(block::WINDMILL_TURNING).name, "electricity:windmill_turning");
    assert_eq!(reg.get(block::STONE_SLAB).name, "genesis:stone_slab");
    assert_eq!(reg.get(block::OAK_DOOR).name, "genesis:oak_door");
}

#[test]
fn no_new_texture_layer_is_added() {
    assert_eq!(crate::texture_gen::texture_count(), TEXTURE_COUNT);
    let reg = BlockRegistry::new();
    for (id, _, _) in bv::all() {
        let d = reg.get(id);
        for t in [d.tex_top, d.tex_bottom, d.tex_side] {
            assert!(t < TEXTURE_COUNT, "{} names layer {t}", d.name);
        }
    }
}

/// A drop is the block itself (no double-drop: the stone slab never had one),
/// and the tool the table names harvests it.
#[test]
fn every_variant_drops_itself_to_the_right_tool() {
    let reg = BlockRegistry::new();
    for (id, m, shape) in bv::all() {
        let drop = reg.mine_drop(id);
        assert_eq!((drop.item.clone(), drop.count), (Item::Block(id), 1), "{} {shape:?}", m.key);
        match m.family {
            Family::Wood => {
                // Doors and trapdoors never burn (fire would orphan a door half).
                assert_eq!(block::flammable(id), !matches!(shape, VShape::Door | VShape::Trapdoor));
                assert_eq!(crate::crafting::best_tool_for(id), Some(ToolType::Axe));
                assert!(crate::crafting::can_harvest(id, None), "a wooden door drops for a fist");
            }
            Family::Stone => {
                assert!(!block::flammable(id));
                assert_eq!(crate::crafting::best_tool_for(id), Some(ToolType::Pickaxe));
                let tier = m.min_tier.expect("stone needs a pickaxe");
                let pick = |mt| Tool::new(ToolType::Pickaxe, mt);
                assert!(!crate::crafting::can_harvest(id, None));
                assert!(crate::crafting::can_harvest(id, Some(&pick(ToolMaterial::Diamond))));
                assert!(crate::crafting::can_harvest(id, Some(&pick(tier))));
            }
        }
        assert!(crate::crafting::block_hardness(id) > 0.0);
    }
}

fn grid_of(cells: &[(usize, usize, CraftSlot)]) -> [[CraftSlot; 3]; 3] {
    let mut g = [[CraftSlot::Empty; 3]; 3];
    for &(r, c, s) in cells {
        g[r][c] = s;
    }
    g
}

/// Every variant has a catalogue card whose example grid the live matcher turns
/// into exactly the card's output, in Minecraft's amounts.
#[test]
fn every_variant_has_a_recipe_that_crafts_it() {
    for (id, m, shape) in bv::all() {
        let idx = crate::crafting_catalogue::recipe_index_for_output(&Item::Block(id))
            .unwrap_or_else(|| panic!("{} {shape:?} has no recipe card", m.key));
        let card = &crate::crafting_catalogue::all_cards()[idx];
        let out = match_recipe(&card.example_grid).unwrap_or_else(|| panic!("{} does not craft", card.name));
        assert_eq!(out.item, Item::Block(id), "{}", card.name);
        let want = match shape {
            VShape::Slab => 6,
            VShape::Stairs => 4,
            VShape::Door => 3,
            VShape::Trapdoor => 2,
            VShape::FenceGate => 1,
            VShape::Wall => 6,
        };
        assert_eq!(out.count, want, "{}", card.name);
        // The card's bill is the material, in the right amount.
        let base_needed: u8 = card.ingredients.iter().filter(|i| i.item == Item::Block(m.base)).map(|i| i.count).sum();
        let base_expected = match shape {
            VShape::Slab => 3,
            VShape::FenceGate => 2,
            _ => 6,
        };
        assert_eq!(base_needed, base_expected, "{}", card.name);
    }
}

#[test]
fn recipes_need_one_material_and_both_stair_handednesses_work() {
    let spruce = CraftSlot::Block(block::SPRUCE_PLANKS);
    let birch = CraftSlot::Block(block::BIRCH_PLANKS);
    let granite = CraftSlot::Block(block::GRANITE);
    let s = spruce;
    let stairs = variant("spruce", VShape::Stairs);
    let left = grid_of(&[(0, 0, s), (1, 0, s), (1, 1, s), (2, 0, s), (2, 1, s), (2, 2, s)]);
    let right = grid_of(&[(0, 2, s), (1, 1, s), (1, 2, s), (2, 0, s), (2, 1, s), (2, 2, s)]);
    for g in [left, right] {
        let out = match_recipe(&g).expect("a spruce staircase crafts");
        assert_eq!((out.item, out.count), (Item::Block(stairs), 4));
    }
    // One birch plank in a spruce staircase crafts nothing.
    let mut mixed = left;
    mixed[0][0] = birch;
    assert!(match_recipe(&mixed).is_none());
    // Mixed doors, trapdoors, slabs and walls do not craft either.
    let mut door = grid_of(&[(0, 0, s), (0, 1, s), (1, 0, s), (1, 1, s), (2, 0, s), (2, 1, s)]);
    assert_eq!(match_recipe(&door).unwrap().item, Item::Block(variant("spruce", VShape::Door)));
    door[2][1] = birch;
    assert!(match_recipe(&door).is_none());
    let slab = grid_of(&[(0, 0, granite), (0, 1, granite), (0, 2, granite)]);
    assert_eq!(match_recipe(&slab).unwrap().item, Item::Block(variant("granite", VShape::Slab)));
    let mixed_slab = grid_of(&[(0, 0, granite), (0, 1, granite), (0, 2, spruce)]);
    assert!(match_recipe(&mixed_slab).is_none());
    // A stone cannot be a door, and planks cannot be a wall: nothing to craft.
    let planks_wall = grid_of(&[(0, 0, s), (0, 1, s), (0, 2, s), (1, 0, s), (1, 1, s), (1, 2, s)]);
    assert_eq!(match_recipe(&planks_wall).unwrap().item, Item::Block(variant("spruce", VShape::Trapdoor)));
    let stone_door = grid_of(&[(0, 0, granite), (0, 1, granite), (1, 0, granite), (1, 1, granite), (2, 0, granite), (2, 1, granite)]);
    assert!(match_recipe(&stone_door).is_none());
    // Sticks and a plank rail: the gate of THAT wood.
    let st = CraftSlot::Material(MaterialId::Stick);
    let gate = grid_of(&[(0, 0, st), (0, 1, birch), (0, 2, st), (1, 0, st), (1, 1, birch), (1, 2, st)]);
    assert_eq!(match_recipe(&gate).unwrap().item, Item::Block(variant("birch", VShape::FenceGate)));
}

#[test]
fn the_originals_still_craft_what_they_did() {
    let oak = CraftSlot::Block(block::OAK_PLANKS);
    let cobble = CraftSlot::Block(block::COBBLESTONE);
    let stone = CraftSlot::Block(block::STONE);
    let door = grid_of(&[(0, 0, oak), (0, 1, oak), (1, 0, oak), (1, 1, oak), (2, 0, oak), (2, 1, oak)]);
    assert_eq!(match_recipe(&door).unwrap().item, Item::Block(block::OAK_DOOR));
    let trap = grid_of(&[(0, 0, oak), (0, 1, oak), (0, 2, oak), (1, 0, oak), (1, 1, oak), (1, 2, oak)]);
    assert_eq!(match_recipe(&trap).unwrap().item, Item::Block(block::OAK_TRAPDOOR));
    let wall = grid_of(&[(0, 0, cobble), (0, 1, cobble), (0, 2, cobble), (1, 0, cobble), (1, 1, cobble), (1, 2, cobble)]);
    assert_eq!(match_recipe(&wall).unwrap().item, Item::Block(block::COBBLESTONE_WALL));
    let slab = grid_of(&[(0, 0, stone), (0, 1, stone), (0, 2, stone)]);
    assert_eq!(match_recipe(&slab).unwrap().item, Item::Block(block::STONE_SLAB));
}

#[test]
fn every_variant_has_a_give_alias_and_a_creative_entry_beside_its_sibling() {
    let reg = BlockRegistry::new();
    let entries = crate::inventory_explorer::enumerate_all_items(&reg);
    let pos = |id: BlockId| entries.iter().position(|e| e.item == Item::Block(id)).unwrap_or_else(|| panic!("no creative entry for {id}"));
    for (id, m, shape) in bv::all() {
        let name = format!("{}_{}", m.key, shape.suffix());
        let given = resolve_item(&name, 3).unwrap_or_else(|e| panic!("/give {name}: {e}"));
        assert_eq!((given.item, given.count), (Item::Block(id), 3), "/give {name}");
        let e = &entries[pos(id)];
        assert_eq!(e.category, crate::inventory_explorer::ExplorerCategory::Blocks);
    }
    // The readable spellings the brief names.
    assert_eq!(resolve_item("spruce_stairs", 1).unwrap().item, Item::Block(variant("spruce", VShape::Stairs)));
    assert_eq!(resolve_item("sandstone_slab", 1).unwrap().item, Item::Block(variant("sandstone", VShape::Slab)));
    assert_eq!(resolve_item("dark-oak-door", 1).unwrap().item, Item::Block(variant("dark_oak", VShape::Door)));
    assert_eq!(resolve_item("granite_wall", 1).unwrap().item, Item::Block(variant("granite", VShape::Wall)));
    assert_eq!(resolve_item("birch_gate", 1).unwrap().item, Item::Block(variant("birch", VShape::FenceGate)));
    // The short aliases still mean the originals.
    assert_eq!(resolve_item("slab", 1).unwrap().item, Item::Block(block::STONE_SLAB));
    assert_eq!(resolve_item("door", 1).unwrap().item, Item::Block(block::OAK_DOOR));
    assert_eq!(resolve_item("wall", 1).unwrap().item, Item::Block(block::COBBLESTONE_WALL));
    // Beside the sibling: every new slab follows the stone slab, in the Blocks tab.
    let slab0 = pos(block::STONE_SLAB);
    let slabs: Vec<_> = bv::all().into_iter().filter(|(id, _, s)| *s == VShape::Slab && *id != block::STONE_SLAB).collect();
    for (i, (id, _, _)) in slabs.iter().enumerate() {
        assert_eq!(pos(*id), slab0 + 1 + i, "slab {id} follows the stone slab");
    }
    // Doors follow the oak door in table order (birch, spruce, ...).
    let door0 = pos(block::OAK_DOOR);
    assert_eq!(pos(variant("birch", VShape::Door)), door0 + 1);
    assert_eq!(pos(variant("spruce", VShape::Door)), door0 + 2);
    let wall0 = pos(block::COBBLESTONE_WALL);
    assert_eq!(pos(variant("stone", VShape::Wall)), wall0 + 1, "the stone wall is the first new wall");
    assert_eq!(pos(variant("sandstone", VShape::Wall)), wall0 + 2, "then sandstone (cobblestone's is the original)");
}

#[test]
fn only_wood_fuels_and_doors_and_trapdoors_do_not_burn() {
    for (id, m, shape) in bv::all() {
        let fuel = crate::campfire::fuel_value(None, Some(id));
        match m.family {
            Family::Wood => {
                let burns = !matches!(shape, VShape::Door | VShape::Trapdoor);
                assert_eq!(block::flammable(id), burns, "{} {shape:?} burns", m.key);
                assert_eq!(fuel, Some(if shape == VShape::Slab { 120 } else { 240 }));
            }
            Family::Stone => {
                assert!(!block::flammable(id));
                assert_eq!(fuel, None);
            }
        }
    }
}

fn point_blocked(boxes: &[block_shape::Aabb], p: [f32; 3]) -> bool {
    boxes.iter().any(|b| (0..3).all(|a| p[a] > b.min[a] && p[a] < b.max[a]))
}

/// Craft, place, walk into, open, walk through, shut and break: one wood (a
/// spruce door and gate) and one stone (granite stairs, a slab and a wall).
#[test]
fn spruce_door_and_gate_and_granite_stairs_and_wall_work_end_to_end() {
    let mut host = TestHost::start_with(TestConfig::default());
    let reg = BlockRegistry::new();
    let spruce = CraftSlot::Block(block::SPRUCE_PLANKS);
    let granite = CraftSlot::Block(block::GRANITE);
    let st = CraftSlot::Material(MaterialId::Stick);
    let inv = &mut host.server.players[0].inventory;
    inv.add_item(ItemStack::new_block(block::SPRUCE_PLANKS, 8));
    inv.add_item(ItemStack::new_block(block::GRANITE, 12));
    inv.add_item(ItemStack::new_material(MaterialId::Stick, 4));

    // Craft: 6 planks -> 3 doors; 6 granite -> 4 stairs; 6 granite -> 6 walls;
    // sticks + 2 planks cannot (only 2 planks left) ... so craft the gate first.
    let gate_grid = grid_of(&[(0, 0, st), (0, 1, spruce), (0, 2, st), (1, 0, st), (1, 1, spruce), (1, 2, st)]);
    assert_eq!(host.craft(gate_grid).unwrap().item, Item::Block(variant("spruce", VShape::FenceGate)));
    let door_grid = grid_of(&[(0, 0, spruce), (0, 1, spruce), (1, 0, spruce), (1, 1, spruce), (2, 0, spruce), (2, 1, spruce)]);
    let doors = host.craft(door_grid).unwrap();
    assert_eq!((doors.item, doors.count), (Item::Block(variant("spruce", VShape::Door)), 3));
    let (g, e) = (granite, CraftSlot::Empty);
    let stairs_grid = [[g, e, e], [g, g, e], [g, g, g]];
    assert_eq!(host.craft(stairs_grid).unwrap().count, 4);
    let wall_grid = grid_of(&[(0, 0, g), (0, 1, g), (0, 2, g), (1, 0, g), (1, 1, g), (1, 2, g)]);
    let walls = host.craft(wall_grid).unwrap();
    assert_eq!((walls.item, walls.count), (Item::Block(variant("granite", VShape::Wall)), 6));

    // Floor.
    for x in -3..=6 {
        for z in -3..=6 {
            host.set_block(x, 63, z, block::STONE);
        }
    }

    // --- Door: placed facing north at (0,64,0), a whole two-cell door.
    let door = variant("spruce", VShape::Door);
    let north = glam::Vec3::new(0.0, 0.0, -1.0);
    let bottom_meta = crate::placement_meta::placement_meta(door, [0, 1, 0], north).expect("a door stamps its facing");
    assert_eq!(meta::facing(bottom_meta), Facing::North);
    assert!(host.place_from_inventory(0, 64, 0, door, Facing::North));
    host.server.world.set_meta((0, 64, 0), bottom_meta);
    host.server.world.place_player_block(0, 65, 0, door);
    host.server.world.set_meta((0, 65, 0), crate::use_edits::door_top_meta(bottom_meta));
    let boxes = |host: &TestHost, c: [i32; 3]| host.world().collision_boxes_at(c[0], c[1], c[2], &reg);
    // Collide: shut, it fills the doorway (feet and head height).
    let shut_low = boxes(&host, [0, 64, 0]);
    let shut_high = boxes(&host, [0, 65, 0]);
    assert!(point_blocked(&shut_low, [0.5, 64.5, 0.09]) && point_blocked(&shut_high, [0.5, 65.5, 0.09]));
    // Toggle: opening swings both halves clear of the doorway.
    let metas = toggle_metas(host.world(), [0, 64, 0], OpenRule::FlipEach);
    assert_eq!(metas.len(), 2, "the door and its other half");
    for (c, m) in &metas {
        host.server.world.set_meta((c[0], c[1], c[2]), *m);
    }
    assert!(block_shape::is_open(host.server.world.meta_at(0, 64, 0)) && block_shape::is_open(host.server.world.meta_at(0, 65, 0)));
    assert!(!point_blocked(&boxes(&host, [0, 64, 0]), [0.5, 64.5, 0.09]), "open: the doorway is clear");
    // A neighbouring door of ANOTHER wood opens with it (double door).
    let birch_door = variant("birch", VShape::Door);
    host.server.world.set_block(1, 64, 0, birch_door);
    host.server.world.set_meta((1, 64, 0), bottom_meta);
    host.server.world.set_block(1, 65, 0, birch_door);
    host.server.world.set_meta((1, 65, 0), crate::use_edits::door_top_meta(bottom_meta));
    let metas = toggle_metas(host.world(), [0, 64, 0], OpenRule::Canonical);
    assert_eq!(metas.len(), 4, "both doors, both halves");
    // Break: a bare hand harvests a wooden door; the drop is the door itself.
    let drop = host.mine_into_inventory(0, 64, 0).expect("wood drops for a fist");
    assert_eq!(drop.item, Item::Block(door));

    // --- Fence gate at (3,64,0), facing north: shut blocks, open lets through.
    let gate = variant("spruce", VShape::FenceGate);
    assert!(host.place_from_inventory(3, 64, 0, gate, Facing::North));
    host.server.world.set_meta((3, 64, 0), meta::with_facing(0, Facing::North));
    assert!(point_blocked(&boxes(&host, [3, 64, 0]), [3.5, 64.5, 0.5]), "shut gate blocks the centre");
    for (c, m) in toggle_metas(host.world(), [3, 64, 0], OpenRule::FlipEach) {
        host.server.world.set_meta((c[0], c[1], c[2]), m);
    }
    assert!(block_shape::is_open(host.server.world.meta_at(3, 64, 0)));
    assert!(!point_blocked(&boxes(&host, [3, 64, 0]), [3.5, 64.5, 0.5]), "open gate lets you through");

    // --- Granite stairs at (0,64,3): two boxes, the tall one on the facing side.
    let granite_stairs = variant("granite", VShape::Stairs);
    assert!(host.place_from_inventory(0, 64, 3, granite_stairs, Facing::North));
    host.server.world.set_meta((0, 64, 3), meta::with_facing(0, Facing::North));
    let stair_boxes = boxes(&host, [0, 64, 3]);
    assert_eq!(stair_boxes.len(), 2);
    assert!(point_blocked(&stair_boxes, [0.5, 64.25, 3.5]) && point_blocked(&stair_boxes, [0.5, 64.75, 3.25]));
    assert!(!point_blocked(&stair_boxes, [0.5, 64.75, 3.75]), "the upper step sits on the facing side only");

    // --- Granite wall at (4,64,3): a post, and an arm once a neighbour joins.
    let granite_wall = variant("granite", VShape::Wall);
    assert!(host.place_from_inventory(4, 64, 3, granite_wall, Facing::North));
    assert_eq!(host.world().connection_mask(4, 64, 3, BlockShape::Wall, &reg), 0);
    host.server.world.set_block(5, 64, 3, granite_wall);
    assert_eq!(host.world().connection_mask(4, 64, 3, BlockShape::Wall, &reg), block_shape::CONN_E, "walls of one stone join");
    host.server.world.set_block(3, 64, 3, variant("cobblestone", VShape::Wall));
    assert_eq!(host.world().connection_mask(4, 64, 3, BlockShape::Wall, &reg), block_shape::CONN_E | block_shape::CONN_W, "and walls of another");

    // --- Break the stone: a fist breaks it but yields nothing; a pickaxe yields it.
    assert_eq!(host.mine_into_inventory(0, 64, 3), None, "a fist cannot harvest stairs");
    host.server.players[0].inventory.add_item(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Wood)));
    assert!(host.equip_tool(ToolType::Pickaxe, ToolMaterial::Wood));
    host.server.world.set_block(0, 64, 3, granite_stairs);
    let drop = host.mine_into_inventory(0, 64, 3).expect("a wooden pickaxe harvests granite stairs");
    assert_eq!(drop.item, Item::Block(granite_stairs));
    assert_eq!(host.get_block(0, 64, 3), block::AIR);
}
