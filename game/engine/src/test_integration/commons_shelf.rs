//! Living world Tier 2 S6: the Commons Shelf.
//!
//! The shelf is a chest under another block id, with one byte of authority state
//! in `block_meta` (fill in the low seven bits, looted in the top bit) kept true
//! by the shared machine phase. The joiner paths (window ops, the byte reaching
//! the joiner, a joiner's break) are in `shared_containers.rs`, which owns the
//! real joined-client rig; this file has the local seat, hopper, save, recipe and
//! item-list halves.
use crate::block;
use crate::chest::{ChestData, ChestTier};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::protocol::BlockChange;
use crate::save::{read_world_save, world_dir, WorldsRootGuard};
use crate::server::GameServer;
use crate::shelf::{fill_of, fill_of_meta, is_looted, FILL_MAX};
use crate::sim_machines::{step, LocalMachineState, MachineBatch, MachineRules};
use crate::world::World;

const POS: (i32, i32, i32) = (3, 70, 3);

/// One shared machine phase at `tick` over `world`: what the standalone game,
/// a lending host and a dedicated server all run.
fn beat(world: &mut World, tick: u64, changes: &mut Vec<BlockChange>) {
    let (mut ecs, mut water, mut lava, mut fire) = (
        hecs::World::new(),
        crate::water::WaterSystem::new(),
        crate::lava::LavaSystem::new(),
        crate::fire::FireSystem::new(),
    );
    let registry = block::BlockRegistry::new();
    let mut ctx = crate::block_machines::MachineCtx {
        world,
        ecs: &mut ecs,
        water: &mut water,
        lava: &mut lava,
        fire: &mut fire,
        registry: &registry,
        tick,
    };
    let rules = MachineRules {
        raining: false,
        seed: 42,
        can_detonate: false,
        mode: crate::play_mode::PlayMode::Survival,
    };
    let mut batch = MachineBatch::reserve(true).unwrap();
    step(&mut ctx, rules, &LocalMachineState::default(), &mut batch, changes, |_, _| {});
}

fn shelf_world() -> World {
    let mut w = World::new();
    w.set_block(POS.0, POS.1, POS.2, block::COMMONS_SHELF);
    w.insert_chest(POS, ChestData::for_tier(ChestTier::Shelf));
    w
}

fn meta(w: &World) -> u8 {
    w.meta_at(POS.0, POS.1, POS.2)
}

fn stocked_inventory(n: u16) -> Inventory {
    let mut inv = Inventory::new();
    for i in 0..n {
        inv.set_slot(usize::from(i), Some(ItemStack::new_block(1 + i, 1)));
    }
    inv
}

/// A seat putting into and taking from the shelf through the container screen's
/// own rules; the next machine beat follows the contents, and a beat with nothing
/// new writes nothing.
#[test]
fn a_seat_putting_and_taking_moves_the_byte_and_a_quiet_beat_writes_nothing() {
    let mut quiet = shelf_world();
    let mut changes = Vec::new();
    beat(&mut quiet, 4, &mut changes);
    assert!(changes.is_empty() && meta(&quiet) == 0, "an empty shelf says nothing");

    // A fresh shelf (the day has not seen it yet, so its baseline is what it holds
    // at the first beat after the seat has stocked it).
    let mut w = shelf_world();

    // Twenty slots in.
    let mut inv = stocked_inventory(20);
    for slot in 0..20 {
        assert!(crate::chest_ui::deposit_to_chest(&mut inv, slot, w.chest_at_mut(POS).unwrap(), true));
    }
    beat(&mut w, 8, &mut changes);
    let full = fill_of(&w.chest_at(POS).unwrap().slots);
    assert_eq!(meta(&w), full, "20 of 27 slots, not looted");
    assert_eq!(changes.len(), 1);
    assert_eq!((changes[0].x, changes[0].y, changes[0].z, changes[0].new_block), (POS.0, POS.1, POS.2, block::COMMONS_SHELF));
    assert_eq!(changes[0].meta, full, "the change carries the byte");
    beat(&mut w, 12, &mut changes);
    assert_eq!(changes.len(), 1, "no change, no write");

    // The seat takes the lot out (Take all): half or more gone in one day.
    assert!(crate::chest_ui::take_all(&mut inv, w.chest_at_mut(POS).unwrap()));
    beat(&mut w, 16, &mut changes);
    assert!(is_looted(meta(&w)), "cleaned out within the day");
    assert_eq!(fill_of_meta(meta(&w)), fill_of(&w.chest_at(POS).unwrap().slots));
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[1].meta, meta(&w));
}

/// A small take that leaves more than half is not looting; a day that began under
/// a quarter full cannot be looted at all.
#[test]
fn a_small_take_or_a_nearly_empty_shelf_is_not_looted() {
    let mut w = shelf_world();
    for s in w.chest_at_mut(POS).unwrap().slots.iter_mut().take(20) {
        *s = Some(ItemStack::new_block(block::STONE, 2));
    }
    let mut changes = Vec::new();
    beat(&mut w, 4, &mut changes);
    for s in w.chest_at_mut(POS).unwrap().slots.iter_mut().take(8) {
        *s = None;
    }
    beat(&mut w, 8, &mut changes);
    assert!(!is_looted(meta(&w)), "20 -> 12 slots is a take, not a raid");

    let mut thin = shelf_world();
    thin.chest_at_mut(POS).unwrap().slots[0] = Some(ItemStack::new_block(block::STONE, 2));
    thin.chest_at_mut(POS).unwrap().slots[1] = Some(ItemStack::new_block(block::STONE, 2));
    beat(&mut thin, 4, &mut Vec::new());
    thin.chest_at_mut(POS).unwrap().slots[0] = None;
    thin.chest_at_mut(POS).unwrap().slots[1] = None;
    beat(&mut thin, 8, &mut Vec::new());
    assert!(!is_looted(meta(&thin)), "two slots is under a quarter: never looted");
    assert_eq!(meta(&thin), 0);
}

/// A hopper above a shelf fills it from a chest above the hopper; the byte follows
/// without any hook in the hopper, because the sweep reads the contents.
#[test]
fn a_hopper_feeding_a_shelf_moves_the_byte() {
    let mut w = shelf_world();
    w.set_block(POS.0, POS.1 + 1, POS.2, block::HOPPER);
    let src = (POS.0, POS.1 + 2, POS.2);
    w.set_block(src.0, src.1, src.2, block::CHEST);
    let mut source = ChestData::new();
    for (i, slot) in source.slots.iter_mut().enumerate().take(12) {
        *slot = Some(ItemStack::new_block(1 + i as u16, 1));
    }
    w.insert_chest(src, source);
    let mut changes = Vec::new();
    let mut tick = 0u64;
    for _ in 0..12 {
        tick += crate::hopper::HOPPER_INTERVAL_TICKS;
        crate::hopper::tick_hoppers(&mut w, tick);
    }
    assert_eq!(w.chest_at(POS).unwrap().occupied(), 12, "the hopper moved a slot's worth a pass");
    beat(&mut w, tick, &mut changes);
    assert_eq!(fill_of_meta(meta(&w)), fill_of(&w.chest_at(POS).unwrap().slots));
    assert!(!is_looted(meta(&w)));
    assert_eq!(changes.len(), 1);
}

/// Contents and byte survive a save and a reload; the day's starting fill does
/// not (a day after a load begins at whatever the shelf holds).
#[test]
fn a_shelf_keeps_its_contents_and_byte_across_a_save_and_load() {
    let _g = WorldsRootGuard::new("shelf-roundtrip");
    let mut s = GameServer::new(0, "w".into(), 42);
    s.world.set_block(POS.0, POS.1, POS.2, block::COMMONS_SHELF);
    let mut data = ChestData::for_tier(ChestTier::Shelf);
    for (i, slot) in data.slots.iter_mut().enumerate().take(20) {
        *slot = Some(ItemStack::new_block(1 + i as u16, 4));
    }
    s.world.insert_chest(POS, data.clone());
    beat(&mut s.world, 4, &mut Vec::new());
    // Cleaned out: looted.
    for slot in s.world.chest_at_mut(POS).unwrap().slots.iter_mut().skip(3) {
        *slot = None;
    }
    beat(&mut s.world, 8, &mut Vec::new());
    let saved_meta = meta(&s.world);
    assert!(is_looted(saved_meta));
    s.try_save().unwrap();
    let on_disk = read_world_save(&std::fs::read(world_dir("w").join("world.dat")).unwrap()).unwrap();
    assert!(on_disk.block_meta.contains(&(POS.0, POS.1, POS.2, saved_meta)));

    let mut back = GameServer::new(0, "w".into(), 42);
    back.initial_load().unwrap();
    assert_eq!(back.world.get_block(POS.0, POS.1, POS.2), block::COMMONS_SHELF);
    assert_eq!(meta(&back.world), saved_meta, "the byte survives");
    let chest = back.world.chest_at(POS).expect("the shelf's contents survive");
    assert_eq!(chest.tier, ChestTier::Shelf);
    assert_eq!(chest.slots.len(), 27);
    assert_eq!(chest.slots[..3], data.slots[..3]);
    assert!(chest.slots[3..].iter().all(Option::is_none));
    assert!(back.world.shelf_days.is_empty(), "the day's starting fill is runtime only");
    // After the reload a beat leaves a looted, still-empty shelf looted, and writes nothing.
    let mut changes = Vec::new();
    beat(&mut back.world, 4, &mut changes);
    assert!(changes.is_empty());
    assert_eq!(meta(&back.world), saved_meta);
    // The shelf is a block the world knows about and the chunk counter finds.
    assert_eq!(back.world.shelf_cells(), vec![POS]);
}

/// A shelf breaks like a chest: the contents spill, the block drops itself, and
/// the block entity is gone.
#[test]
fn breaking_a_shelf_drops_its_contents_and_itself() {
    let mut w = shelf_world();
    w.chest_at_mut(POS).unwrap().slots[0] = Some(ItemStack::new_block(block::STONE, 5));
    w.chest_at_mut(POS).unwrap().slots[9] = Some(ItemStack::new_material(MaterialId::Stick, 7));
    assert_eq!(crate::world::mirrored_family(block::COMMONS_SHELF), Some(crate::world::MirroredFamily::Chest));
    assert!(crate::world::mirrored_family(block::COMMONS_SHELF).unwrap().is_container());
    let spill = crate::chest::cleanup_chest(&mut w, POS.0, POS.1, POS.2);
    assert_eq!(spill.len(), 2);
    assert_eq!(spill.iter().map(|s| u32::from(s.count)).sum::<u32>(), 12);
    assert!(w.chest_at(POS).is_none());
    let reg = block::BlockRegistry::new();
    let drop = reg.mine_drop(block::COMMONS_SHELF);
    assert_eq!((drop.item, drop.count), (Item::Block(block::COMMONS_SHELF), 1), "a shelf drops a shelf");
}

/// The shelf is an item like any block: in the recipe book and the creative list,
/// craftable only from its own shape, and no other card makes it.
#[test]
fn the_shelf_is_in_the_book_and_the_creative_list_and_collides_with_nothing() {
    let reg = block::BlockRegistry::new();
    let shelf = ItemStack::new_block(block::COMMONS_SHELF, 1);
    let cards: Vec<_> = crate::crafting_catalogue::all_cards().iter().filter(|c| c.output == shelf).collect();
    assert_eq!(cards.len(), 1, "one card makes a shelf");
    assert_eq!(cards[0].name, "Commons Shelf");
    for c in crate::crafting_catalogue::all_cards() {
        if c.output != shelf {
            let got = crate::crafting::match_recipe(&c.example_grid);
            assert_ne!(got, Some(shelf.clone()), "{}'s grid must not craft a shelf", c.name);
        }
    }
    assert_eq!(crate::crafting::match_recipe(&cards[0].example_grid), Some(shelf));
    let items = crate::inventory_explorer::enumerate_all_items(&reg);
    assert!(items.iter().any(|e| e.item == Item::Block(block::COMMONS_SHELF) && e.label.contains("Commons Shelf")));
    assert_eq!(Item::Block(block::COMMONS_SHELF).name(&reg), "Commons Shelf");
    // The creative list names the block, and its words are not money words.
    assert!(crate::copy_lint::banned_in("Commons Shelf", crate::copy_lint::BANNED_MONEY_WORDS).is_empty());
}

/// A mismatch between what the chest tier says and what the block id says is
/// impossible to state: the tier maps to the block and back, and the chunk's
/// maintained counter sees a shelf whichever way it got there.
#[test]
fn the_chunk_counter_finds_shelves_however_they_were_set() {
    let mut w = World::new();
    assert!(w.shelf_cells().is_empty());
    w.set_block(1, 5, 1, block::COMMONS_SHELF);
    w.place_player_block(40, 70, -3, block::COMMONS_SHELF);
    assert_eq!(w.shelf_cells(), vec![(1, 5, 1), (40, 70, -3)]);
    w.set_block(1, 5, 1, block::AIR);
    assert_eq!(w.shelf_cells(), vec![(40, 70, -3)]);
    assert!(FILL_MAX == 127);
}
