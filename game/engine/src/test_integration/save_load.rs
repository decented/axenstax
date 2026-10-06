//! Multi-player save/load round-trips. Locks in behaviour before the
//! single-player/HostedServer fork is collapsed: a 2-player save must
//! restore both players, a legacy single-player save must still load
//! into the first slot, and `GameServer::initial_load` must pick up N
//! players from disk.
//!
//! Tests write to `worlds/<unique-name>/` (relative to the engine manifest
//! dir) and clean up after themselves. Native-only — save_world is not
//! exposed as a sync API on WASM.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec3;

use crate::block::STONE;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::ItemStack;
use crate::player_slot::PlayerSlot;
use crate::save::{self, PlayerSaveData, SavedSlot, WorldSave};
use crate::server::GameServer;
use crate::world::World;

/// Build a PlayerSlot with a handcrafted position, yaw, pitch, health,
/// hotbar slot, and a deterministic inventory (slot 0 filled, slot 5 tool).
fn make_slot(index: usize, pos: Vec3, yaw: f32, pitch: f32, health: f32) -> PlayerSlot {
    let mut slot = PlayerSlot::new(index, pos, 1.0);
    slot.camera.yaw = yaw;
    slot.camera.pitch = pitch;
    slot.combat.health = health;
    slot.hotbar_slot = 3;

    slot.inventory.set_slot(0, Some(ItemStack::new_block(STONE, 42)));
    slot.inventory.set_slot(
        5,
        Some(ItemStack::new_tool(Tool::new(
            ToolType::Pickaxe,
            ToolMaterial::Stone,
        ))),
    );
    slot
}

/// Remove any leftover world directory for this test name, in both the
/// before and after phases. Failure to clean is not fatal — the directory
/// may not exist.
fn scrub(name: &str) {
    let _ = save::delete_world(name);
}

#[test]
fn save_world_persists_two_players() {
    let name = "__test_save_two_players__";
    scrub(name);

    let world = World::new();
    let p1 = make_slot(0, Vec3::new(10.0, 80.0, 20.0), 1.5, -0.2, 18.0);
    let p2 = make_slot(1, Vec3::new(-30.0, 65.0, 5.0), -0.5, 0.1, 12.0);

    save::save_world(name, &world, &[p1, p2], 42, &[], &[]).expect("save_world");

    let mut loaded_world = World::new();
    let (save, _chunk_count) = save::load_world(name, &mut loaded_world).expect("load_world");

    assert_eq!(save.players.len(), 2, "both players must be in the save");
    assert_eq!(save.players[0].x, 10.0);
    assert_eq!(save.players[0].y, 80.0);
    assert_eq!(save.players[0].z, 20.0);
    assert_eq!(save.players[0].yaw, 1.5);
    assert_eq!(save.players[0].health, 18.0);
    assert_eq!(save.players[0].hotbar_slot, 3);
    assert_eq!(save.players[1].x, -30.0);
    assert_eq!(save.players[1].y, 65.0);
    assert_eq!(save.players[1].z, 5.0);
    assert_eq!(save.players[1].health, 12.0);

    // Legacy top-level fields must mirror player 0 for backward compat.
    assert_eq!(save.player_x, 10.0);
    assert_eq!(save.player_health, 18.0);

    // Inventory content must survive the round-trip for both players.
    for p in &save.players {
        match &p.inventory[0] {
            SavedSlot::Block { block_id, count } => {
                assert_eq!(*block_id, STONE);
                assert_eq!(*count, 42);
            }
            _ => panic!("expected block in slot 0"),
        }
        match &p.inventory[5] {
            SavedSlot::Tool {
                tool_type,
                material,
                ..
            } => {
                assert_eq!(*tool_type, ToolType::Pickaxe);
                assert_eq!(*material, ToolMaterial::Stone);
            }
            _ => panic!("expected tool in slot 5"),
        }
    }

    scrub(name);
}

#[test]
fn armour_slots_round_trip_through_save_with_full_iron_set() {
    // Spec 28e — equipping a full Iron set on a player must survive
    // serialise + deserialise, with each piece's slot/material/durability
    // preserved.
    use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot, max_durability};

    let name = "__test_save_full_iron_armour__";
    scrub(name);

    let world = World::new();
    let mut p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    // Dirty up the Chestplate durability so the round-trip proves it's
    // exact-restored, not just material-restored.
    p1.armour_slots[ArmourSlot::Helmet as usize] =
        Some(ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron));
    let mut chest = ArmourItem::new(ArmourSlot::Chestplate, ArmourMaterial::Iron);
    chest.durability = max_durability(ArmourSlot::Chestplate, ArmourMaterial::Iron) - 17;
    p1.armour_slots[ArmourSlot::Chestplate as usize] = Some(chest);
    p1.armour_slots[ArmourSlot::Leggings as usize] =
        Some(ArmourItem::new(ArmourSlot::Leggings, ArmourMaterial::Iron));
    p1.armour_slots[ArmourSlot::Boots as usize] =
        Some(ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Iron));

    save::save_world(name, &world, &[p1], 7, &[], &[]).expect("save_world");

    let mut loaded_world = World::new();
    let (save, _chunks) = save::load_world(name, &mut loaded_world).expect("load_world");

    let p = &save.players[0];
    let restored = save::restore_armour_slots(&p.armour_slots);
    for slot in [
        ArmourSlot::Helmet, ArmourSlot::Chestplate,
        ArmourSlot::Leggings, ArmourSlot::Boots,
    ] {
        let piece = restored[slot as usize].as_ref()
            .unwrap_or_else(|| panic!("{:?} slot lost on save round-trip", slot));
        assert_eq!(piece.slot, slot);
        assert_eq!(piece.material, ArmourMaterial::Iron);
    }
    // Damaged chestplate durability must round-trip exactly.
    let restored_chest = restored[ArmourSlot::Chestplate as usize].as_ref().unwrap();
    let expected_dur = max_durability(ArmourSlot::Chestplate, ArmourMaterial::Iron) - 17;
    assert_eq!(restored_chest.durability, expected_dur);

    scrub(name);
}

#[test]
fn legacy_single_player_save_synthesises_one_player_entry() {
    // A save with an empty `players` Vec (pre-multi-player format) must
    // still restore, using the top-level legacy fields as synthetic
    // player 0 data. This path is exercised in both chunk_stream.rs and
    // server.rs initial_load.
    let legacy = WorldSave {
        seed: 7,
        player_x: 4.0,
        player_y: 70.0,
        player_z: -2.0,
        player_health: 17.0,
        hotbar_slot: 1,
        inventory: vec![SavedSlot::Block {
            block_id: STONE,
            count: 8,
        }],
        players: vec![],
        campfires: vec![],
        furnaces: vec![],
        vendors: vec![],
        drying_racks: vec![],
        hives: vec![],
        chests: vec![],
        tip_jars: vec![],
        auctions: vec![],
        latent_prints: vec![],
        plots: vec![],
        market_hubs: vec![],
        construction_anchors: vec![],
        architect_plaques: vec![],
        village_anchors: vec![],
        populated_villages: vec![],
        village_bells: vec![],
        village_treasuries: vec![],
        active_raids: vec![],
        raid_scheduler: crate::raid::RaidScheduler::new(),
        raid_kills: vec![],
        brigand_hideouts: vec![],
        bounties: vec![],
        bounty_next_id: 0,
        bounty_last_refresh_tick: 0,
        face_overlays: vec![],
        face_blueprints: vec![],
        face_blueprint_blanks: vec![],
        workshop: Default::default(),
        carts: vec![],
        graves: vec![],
        waypoints: vec![],
        block_meta: vec![],
        power_devices: vec![],
        signs: vec![],
        item_frames: vec![],
        locked_slots: vec![],
        hostile_acts: vec![],
        rigs: vec![],
        exhibits: Vec::new(),
        composters: Vec::new(),
        saved_mobs: Vec::new(),
        satoshi: Default::default(),
        dispensers: Vec::new(),
        rig_clips: Vec::new(),
    };

    // Round-trip via bincode to mimic the on-disk path. Defensive: a
    // pre-Wave-29 save (no drying_racks field) must also deserialise
    // cleanly via `#[serde(default)]`. The above struct construction
    // exercises the new-format path; the missing-field path is covered
    // by the bincode-version test elsewhere.
    let bytes = bincode::serialize(&legacy).expect("serialize");
    let decoded: WorldSave = bincode::deserialize(&bytes).expect("deserialize");

    let player_saves: Vec<PlayerSaveData> = if decoded.players.is_empty() {
        vec![PlayerSaveData {
            x: decoded.player_x,
            y: decoded.player_y,
            z: decoded.player_z,
            yaw: 0.0,
            pitch: 0.0,
            health: decoded.player_health,
            hotbar_slot: decoded.hotbar_slot,
            inventory: decoded.inventory.clone(),
            spawn_pos: None, // legacy-save fixture
            hunger: 20,
            reputation: vec![],
            tamed_pets: vec![],
            armour_slots: [None, None, None, None],
            kill_counter: vec![],
            bounties_claimed: vec![],
        }]
    } else {
        decoded.players.clone()
    };

    assert_eq!(player_saves.len(), 1);
    assert_eq!(player_saves[0].x, 4.0);
    assert_eq!(player_saves[0].health, 17.0);
    assert_eq!(player_saves[0].hotbar_slot, 1);
    assert_eq!(player_saves[0].inventory.len(), 1);
}

#[test]
fn grave_block_entity_round_trips_through_save() {
    // #47 — a grave's snapshotted inventory survives save+load, index-aligned,
    // and an old save with no `graves` field still loads (serde-default).
    use crate::item::ItemStack;

    let name = "__test_grave_round_trip__";
    scrub(name);

    let mut world = World::new();
    world.set_block(3, 64, 3, crate::block::GRAVE);
    let mut snapshot = vec![None; crate::grave::GRAVE_SLOTS];
    snapshot[0] = Some(ItemStack::new_block(crate::block::STONE, 42));
    snapshot[17] = Some(ItemStack::new_block(crate::block::DIRT, 9));
    world.insert_grave((3, 64, 3), crate::grave::GraveData::from_snapshot(snapshot, 555));

    let p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 99, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let (loaded_save, _) = save::load_world(name, &mut loaded).expect("load_world");

    assert_eq!(loaded_save.graves.len(), 1, "grave persisted to the save");
    let g = loaded.grave_at((3, 64, 3)).expect("grave restored to world");
    assert_eq!(g.created_tick, 555);
    // Index-aligned: slot 0 = 42 stone, slot 17 = 9 dirt, slot 1 empty.
    assert_eq!(g.slots[0].as_ref().map(|s| s.count), Some(42));
    assert_eq!(g.slots[17].as_ref().map(|s| s.count), Some(9));
    assert!(g.slots[1].is_none());

    scrub(name);
}

#[test]
fn exhibits_authored_in_world_survive_save_reload() {
    // Creator Gallery (Spec 2026-06-19 §9, Phase 1c) — exhibits authored on the
    // live `World.exhibits` list (the effect of `/exhibit place`) are snapshotted
    // into the save and restored to a fresh world on load. Proves the whole 1c
    // chain end-to-end: runtime list → WorldSave snapshot → bincode → restore.
    use crate::exhibit::{Exhibit, Presentation};

    let name = "__test_exhibit_round_trip__";
    scrub(name);

    let mut world = World::new();
    world.exhibits.push(Exhibit {
        x: 1,
        y: 64,
        z: 1,
        presentation: Presentation::Wall,
        image_ref: "wall.png".to_string(),
        width: 2.0,
        height: 2.0,
        yaw: 0.0,
        label: "Wall One".to_string(),
        link: None,
        sku: None,
        price: None,
    });
    world.exhibits.push(Exhibit {
        x: 4,
        y: 65,
        z: -2,
        presentation: Presentation::Standing,
        image_ref: "stand.png".to_string(),
        width: 1.0,
        height: 3.0,
        yaw: 0.78,
        label: String::new(),
        link: None,
        sku: None,
        price: None,
    });

    let p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 7, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let (loaded_save, _) = save::load_world(name, &mut loaded).expect("load_world");

    // The save carries both authored exhibits...
    assert_eq!(loaded_save.exhibits.len(), 2, "both exhibits snapshotted to save");
    // ...and they are restored onto the live world for rendering.
    assert_eq!(loaded.exhibits.len(), 2, "both exhibits restored to world");
    assert_eq!(loaded.exhibits[0].image_ref, "wall.png");
    assert_eq!(loaded.exhibits[0].presentation, Presentation::Wall);
    assert_eq!(loaded.exhibits[0].label, "Wall One");
    assert_eq!(loaded.exhibits[1].image_ref, "stand.png");
    assert_eq!(loaded.exhibits[1].presentation, Presentation::Standing);
    assert!((loaded.exhibits[1].yaw - 0.78).abs() < 1e-6);

    scrub(name);
}

#[test]
fn primed_keg_and_loaded_composter_survive_save_reload() {
    // Spec 49 (Explosives) — a primed Blasting Keg (fuse mid-burn, persisted via
    // the `power_devices` side-table because the keg is a PowerDevice) and a
    // loaded Composter (input + ageing progress, its own side-table) round-trip
    // through save → bincode → restore.
    let name = "__test_explosives_round_trip__";
    scrub(name);

    let mut world = World::new();
    // A primed keg: a BLASTING_KEG block + a PowerDevice with a half-burnt fuse.
    world.set_block(2, 64, 2, crate::block::BLASTING_KEG);
    let mut keg = crate::power::PowerDeviceData::new(
        crate::power::PowerDeviceKind::BlastingKeg,
        crate::meta::Facing::Up,
    );
    keg.charge = 47; // fuse mid-burn
    world.insert_power_device((2, 64, 2), keg);
    // A loaded composter: WheatSeeds in the input, some ageing progress.
    world.set_block(5, 64, 5, crate::block::COMPOSTER);
    let mut comp = crate::workstation::WorkstationState::new();
    comp.input = Some(crate::item::ItemStack::new_material(
        crate::item::MaterialId::WheatSeeds,
        3,
    ));
    comp.progress_ticks = 120;
    world.insert_composter((5, 64, 5), comp);

    let p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 7, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let (loaded_save, _) = save::load_world(name, &mut loaded).expect("load_world");

    // The keg's fuse persisted via the power-device side-table.
    let keg_back = loaded
        .power_device_at((2, 64, 2))
        .expect("keg power device restored");
    assert_eq!(keg_back.kind, crate::power::PowerDeviceKind::BlastingKeg);
    assert_eq!(keg_back.charge, 47, "the half-burnt fuse persisted");

    // The composter's contents persisted via its own side-table.
    assert_eq!(loaded_save.composters.len(), 1, "composter snapshotted to save");
    let restored = loaded
        .composter_at((5, 64, 5))
        .expect("composter restored to world");
    assert_eq!(restored.progress_ticks, 120, "ageing progress persisted");
    let input = restored.input.as_ref().expect("input persisted");
    assert!(matches!(
        input.item,
        crate::item::Item::Material(crate::item::MaterialId::WheatSeeds)
    ));
    assert_eq!(input.count, 3);

    scrub(name);
}

#[test]
fn campfire_block_entity_round_trips_through_save() {
    // Wave 27 — campfire state survives save+load. Place a campfire,
    // fuel it, put a meat in slot 1, save, load, assert the state
    // matches.
    use crate::campfire::{CampfireData, CookSlot};
    use crate::item::MaterialId;

    let name = "__test_campfire_round_trip__";
    scrub(name);

    let mut world = World::new();
    world.set_block(5, 70, 5, crate::block::CAMPFIRE);
    let mut cf = CampfireData::default();
    cf.fuel_ticks = 1200; // 60 s of fuel = 1 log
    cf.slots[1] = CookSlot {
        item: Some(MaterialId::RawBeef),
        progress_ticks: 137,
    };
    world.insert_campfire((5, 70, 5), cf);

    let p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 99, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let (loaded_save, _) = save::load_world(name, &mut loaded).expect("load_world");

    // Save includes the campfire.
    assert_eq!(loaded_save.campfires.len(), 1);
    let saved_cf = &loaded_save.campfires[0];
    assert_eq!((saved_cf.x, saved_cf.y, saved_cf.z), (5, 70, 5));
    assert_eq!(saved_cf.data.fuel_ticks, 1200);
    assert_eq!(saved_cf.data.slots[1].item, Some(MaterialId::RawBeef));
    assert_eq!(saved_cf.data.slots[1].progress_ticks, 137);

    // Loaded world's block_entities map has the campfire too — via
    // the Spec 20 Phase 2 variant-aware helper.
    let restored = loaded.campfire_at((5, 70, 5)).expect("restored cf");
    assert_eq!(restored.fuel_ticks, 1200);
    assert_eq!(restored.slots[1].item, Some(MaterialId::RawBeef));

    scrub(name);
}

#[test]
fn drying_rack_round_trips_through_save() {
    // Wave 29 — drying-rack state survives save+load. Mirrors the
    // campfire round-trip pattern: populate three slots at different
    // seasoning stages, save, load, assert exact match.
    use crate::drying_rack::{DryingRackData, LogSpecies, RackSlot, SEASON_TICKS};

    let name = "__test_drying_rack_round_trip__";
    scrub(name);

    let mut world = World::new();
    world.set_block(3, 70, 4, crate::block::DRYING_RACK);
    let mut rack = DryingRackData::default();
    rack.slots[0] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: 0 };
    rack.slots[2] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS / 2 };
    rack.slots[5] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS };
    world.drying_racks.insert((3, 70, 4), rack);

    let p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 21, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let (loaded_save, _) = save::load_world(name, &mut loaded).expect("load_world");

    assert_eq!(loaded_save.drying_racks.len(), 1);
    let saved = &loaded_save.drying_racks[0];
    assert_eq!((saved.x, saved.y, saved.z), (3, 70, 4));
    assert!(saved.data.slots[0].species.is_some());
    assert_eq!(saved.data.slots[0].seasoning_ticks, 0);
    assert_eq!(saved.data.slots[2].seasoning_ticks, SEASON_TICKS / 2);
    assert!(saved.data.slots[5].is_mature());
    assert!(saved.data.slots[1].is_empty());

    let restored = loaded.drying_racks.get(&(3, 70, 4)).expect("restored rack");
    assert!(restored.slots[5].is_mature());
    assert_eq!(restored.slots[2].seasoning_ticks, SEASON_TICKS / 2);

    scrub(name);
}

#[test]
fn campfire_dupe_regression_break_clears_block_entities() {
    // Regression: post-2026-05-18 fix for the block-entity leak +
    // duplication exploit. The break path must clear `block_entities`
    // at the broken cell so a freshly-placed campfire starts with a
    // default `CampfireData`, NOT inherit the previous occupant's
    // fuel + cooked-meat slots.
    //
    // Pre-fix behaviour: cook 4 meats → break the (unlit) campfire →
    // place a new campfire on the same cell → empty-handed right-click
    // yielded the previous 4 cooked meats again. Repeatable dupe.
    //
    // This test exercises the World-side invariants directly: after
    // the break, block_entities must NOT contain the position. The
    // game_loop break paths (creative + survival) all now call
    // `world.block_entities.remove` on the broken cell.
    use crate::campfire::{CampfireData, CookSlot};
    use crate::item::MaterialId;

    let mut world = World::new();
    world.set_block(5, 70, 5, crate::block::CAMPFIRE);
    let mut cf = CampfireData::default();
    cf.fuel_ticks = 1200;
    cf.slots[0] = CookSlot {
        item: Some(MaterialId::CookedBeef),
        progress_ticks: crate::campfire::COOK_TICKS_PER_ITEM,
    };
    world.insert_campfire((5, 70, 5), cf);

    // Simulate a break — the patched break path runs both of these:
    world.set_block(5, 70, 5, crate::block::AIR);
    world.remove_block_entity((5, 70, 5));

    // Place a new campfire at the same cell.
    world.set_block(5, 70, 5, crate::block::CAMPFIRE_UNLIT);

    // The new campfire MUST NOT inherit any state from the old one.
    // The lazy-create paths in game_loop will materialise a fresh
    // CampfireData::default() on first interaction; until then the
    // entry is simply absent.
    assert!(
        world.block_entities.get(&(5, 70, 5)).is_none(),
        "block_entities must be empty after break — dupe regression"
    );
}

#[test]
fn campfire_mine_drop_normalises_to_unlit() {
    // Regression: breaking a lit campfire used to drop a CAMPFIRE
    // (lit) item via the default `new_block(other, 1)` arm. Placing
    // that item showed visible fire for one tick until the
    // no-fuel transition demoted it. Normalised to always drop
    // CAMPFIRE_UNLIT so the inventory icon is honest.
    let reg = crate::block::BlockRegistry::new();
    let drop_lit = reg.mine_drop(crate::block::CAMPFIRE);
    let drop_unlit = reg.mine_drop(crate::block::CAMPFIRE_UNLIT);
    match drop_lit.item {
        crate::item::Item::Block(id) => assert_eq!(id, crate::block::CAMPFIRE_UNLIT),
        _ => panic!("lit campfire must drop unlit block"),
    }
    match drop_unlit.item {
        crate::item::Item::Block(id) => assert_eq!(id, crate::block::CAMPFIRE_UNLIT),
        _ => panic!("unlit campfire must drop unlit block"),
    }
}

#[test]
fn legacy_save_without_campfires_loads_empty_block_entities() {
    // Bincode-positional plus #[serde(default)] means saves that
    // predate Wave 27 deserialise with campfires = [], and the loader
    // puts nothing in block_entities. Regression guard.
    let name = "__test_pre_campfire_save__";
    scrub(name);

    let world = World::new();
    let p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 1, &[], &[]).expect("save");

    let mut loaded = World::new();
    let (loaded_save, _) = save::load_world(name, &mut loaded).expect("load");

    assert!(loaded_save.campfires.is_empty());
    assert!(loaded.block_entities.is_empty());

    scrub(name);
}

#[test]
fn join_p3_then_p4_increases_slot_count() {
    // Data-model smoke test: the engine's player Vec + layout solver must
    // both accept up to 4 local players. (The actual press-A trigger is
    // unit-tested in `local_join::tests`.)
    let mut players: Vec<PlayerSlot> = vec![make_slot(
        0,
        Vec3::new(0.0, 80.0, 0.0),
        0.0,
        0.0,
        20.0,
    )];
    assert_eq!(crate::screen::compute_screen_layout(players.len(), 1920, 1080).len(), 1);

    players.push(make_slot(1, Vec3::new(3.0, 80.0, 0.0), 0.0, 0.0, 20.0));
    players.push(make_slot(2, Vec3::new(6.0, 80.0, 0.0), 0.0, 0.0, 20.0));
    assert_eq!(players.len(), 3);
    assert_eq!(crate::screen::compute_screen_layout(players.len(), 1920, 1080).len(), 3);

    players.push(make_slot(3, Vec3::new(9.0, 80.0, 0.0), 0.0, 0.0, 20.0));
    assert_eq!(players.len(), 4);
    assert_eq!(crate::screen::compute_screen_layout(players.len(), 1920, 1080).len(), 4);
}

#[test]
fn four_player_save_round_trip() {
    // Couch-co-op happy path: a 4-player world must round-trip through
    // save+load with all four players' positions, health, hotbar, and
    // inventory intact. `chunk_stream::initial_load` already loops
    // `while self.players.len() < player_saves.len()` so this is mostly
    // a verification test — but Phase 5 needs an automated guard so
    // future save-format changes don't silently drop P3/P4.
    let name = "__test_save_four_players__";
    scrub(name);

    let world = World::new();
    let p1 = make_slot(0, Vec3::new(100.0, 80.0, 50.0), 0.0, 0.0, 20.0);
    let p2 = make_slot(1, Vec3::new(103.0, 80.0, 50.0), 0.5, 0.1, 16.0);
    let p3 = make_slot(2, Vec3::new(106.0, 80.0, 50.0), -0.3, -0.2, 12.0);
    let p4 = make_slot(3, Vec3::new(109.0, 80.0, 50.0), 1.2, 0.05, 8.0);

    save::save_world(name, &world, &[p1, p2, p3, p4], 1234, &[], &[]).expect("save_world");

    let mut loaded_world = World::new();
    let (save, _chunks) = save::load_world(name, &mut loaded_world).expect("load_world");

    assert_eq!(save.players.len(), 4, "all four players must round-trip");
    let expected_xs = [100.0, 103.0, 106.0, 109.0];
    let expected_hps = [20.0, 16.0, 12.0, 8.0];
    for (i, p) in save.players.iter().enumerate() {
        assert_eq!(p.x, expected_xs[i], "player {} x", i);
        assert_eq!(p.y, 80.0, "player {} y", i);
        assert_eq!(p.z, 50.0, "player {} z", i);
        assert_eq!(p.health, expected_hps[i], "player {} health", i);
        assert_eq!(p.hotbar_slot, 3, "player {} hotbar_slot", i);
        match &p.inventory[0] {
            SavedSlot::Block { block_id, count } => {
                assert_eq!(*block_id, STONE);
                assert_eq!(*count, 42);
            }
            _ => panic!("player {} slot 0 not a block", i),
        }
    }

    // Sanity-check the layout solver agrees that a 4-player save uses the
    // 2x2 quad layout (4 screens, no fallback).
    let screens = crate::screen::compute_screen_layout(save.players.len(), 1920, 1080);
    assert_eq!(screens.len(), 4);

    scrub(name);
}

#[test]
fn joining_a_fifth_player_is_a_no_op() {
    // The join helper short-circuits at the cap. Even if a press-table
    // claims every gamepad is connected and pressing A, a 5th seat must
    // not be granted.
    use crate::local_join::try_join_next_player;
    let always_pressed = |_idx: usize| Some((true, true));
    let decision = try_join_next_player(4, None, always_pressed, Vec3::ZERO);
    assert!(decision.is_none(), "5th player must not be admitted");

    // And the layout solver agrees — 5 falls back to a single viewport
    // (one player rendered, the others dropped) rather than crashing.
    let screens = crate::screen::compute_screen_layout(5, 1920, 1080);
    assert_eq!(screens.len(), 1);
}

#[test]
fn game_server_initial_load_restores_two_players() {
    // End-to-end check at the server authority: save a 2-player world
    // from the client side (PlayerSlots), then spin up a GameServer
    // with num_players=2 and let its initial_load() populate the
    // ServerPlayer positions/inventories from disk.
    let name = "__test_server_initial_load__";
    scrub(name);

    let world = World::new();
    let p1 = make_slot(0, Vec3::new(100.0, 81.0, 200.0), 0.0, 0.0, 20.0);
    let p2 = make_slot(1, Vec3::new(-50.0, 68.0, 11.0), 0.0, 0.0, 9.0);
    save::save_world(name, &world, &[p1, p2], 42, &[], &[]).expect("save_world");

    let mut server = GameServer::new(2, name.to_string(), 42);
    server.initial_load().expect("initial load");

    assert_eq!(server.players.len(), 2);
    assert_eq!(server.players[0].player.pos, Vec3::new(100.0, 81.0, 200.0));
    assert_eq!(server.players[0].combat.health, 20.0);
    assert_eq!(server.players[0].hotbar_slot, 3);
    assert_eq!(server.players[1].player.pos, Vec3::new(-50.0, 68.0, 11.0));
    assert_eq!(server.players[1].combat.health, 9.0);

    // Inventory round-trip check: slot 0 should still hold 42 STONE for
    // both players after the server-side restore path.
    for p in &server.players {
        match p.inventory.slot(0).map(|s| &s.item) {
            Some(crate::item::Item::Block(id)) => assert_eq!(*id, STONE),
            _ => panic!("slot 0 not a stone block"),
        }
    }

    scrub(name);
}

#[test]
fn chest_save_load_preserves_27_slots() {
    // HP-2 — Chest block-entity state survives save+load. Verifies
    // (a) every slot index is preserved, (b) ItemStack contents
    // round-trip via bincode, (c) empty slots stay empty.
    use crate::chest::{ChestData, CHEST_SLOTS};
    use crate::item::MaterialId;

    let name = "__test_chest_round_trip__";
    scrub(name);

    let mut world = World::new();
    world.set_block(2, 70, 3, crate::block::CHEST);
    let mut chest = ChestData::new();
    chest.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 5));
    chest.slots[13] = Some(ItemStack::new_block(crate::block::STONE, 17));
    chest.slots[26] = Some(ItemStack::new_material(MaterialId::IronIngot, 1));
    world.insert_chest((2, 70, 3), chest);

    let p1 = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 13, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let (loaded_save, _) = save::load_world(name, &mut loaded).expect("load_world");

    assert_eq!(loaded_save.chests.len(), 1, "one chest saved");
    let saved = &loaded_save.chests[0];
    assert_eq!((saved.x, saved.y, saved.z), (2, 70, 3));
    assert_eq!(saved.data.slots.len(), CHEST_SLOTS);

    let restored = loaded.chest_at((2, 70, 3)).expect("chest restored");
    assert_eq!(restored.slots.len(), CHEST_SLOTS);
    assert_eq!(restored.slots[0].as_ref().unwrap().count, 5);
    match &restored.slots[0].as_ref().unwrap().item {
        crate::item::Item::Material(MaterialId::Bread) => {}
        other => panic!("bread expected at 0, got {other:?}"),
    }
    assert_eq!(restored.slots[13].as_ref().unwrap().count, 17);
    assert!(restored.slots[10].is_none(), "untouched slot stays empty");
    assert_eq!(restored.slots[26].as_ref().unwrap().count, 1);

    scrub(name);
}

#[test]
fn chest_break_drops_all_contents_via_cleanup() {
    // HP-2 — break drops contents. Driven through `cleanup_chest`
    // (the game_loop break path's bridge to spawn ItemEntities).
    use crate::chest::{cleanup_chest, ChestData};
    use crate::item::MaterialId;

    let mut world = World::new();
    world.set_block(7, 64, 7, crate::block::CHEST);
    let mut chest = ChestData::new();
    chest.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 3));
    chest.slots[5] = Some(ItemStack::new_material(MaterialId::IronIngot, 2));
    chest.slots[10] = Some(ItemStack::new_block(crate::block::STONE, 16));
    world.insert_chest((7, 64, 7), chest);

    let spill = cleanup_chest(&mut world, 7, 64, 7);
    assert_eq!(spill.len(), 3, "three non-empty slots → three spill stacks");
    let total: u32 = spill.iter().map(|s| s.count as u32).sum();
    assert_eq!(total, 3 + 2 + 16);
    assert!(world.chest_at((7, 64, 7)).is_none(), "block-entity removed");
}

#[test]
fn flower_micro_model_survives_save_load() {
    // Owner-inbox #18 — a flower's block id round-trips through save/load (it's a
    // normal block; the micro_registry is NOT persisted), and after reload it still
    // renders as a 3D micro-model once the world re-registers the built-ins (as
    // world init does). Locks the render-only-override + register-on-load contract.
    let name = "__test_flower_micro_roundtrip__";
    scrub(name);
    let registry = crate::block::BlockRegistry::new();

    let mut world = World::new();
    world.set_block(2, 64, 2, crate::block::CORNFLOWER);
    let p = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p], 42, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    save::load_world(name, &mut loaded).expect("load_world");
    assert_eq!(
        loaded.get_block(2, 64, 2),
        crate::block::CORNFLOWER,
        "flower block id must survive save/load"
    );

    // Reload registers the built-in micro-models, exactly as client world init does.
    loaded.load_bundled_micro_models(&registry);
    // (2,64,2) lives in chunk (0,4,0).
    let meshes = crate::mesh::build_chunk_meshes(0, 4, 0, &loaded, &registry);
    assert_eq!(
        meshes.micro_instances.len(),
        1,
        "reloaded flower must still render as a micro-model"
    );
    assert_eq!(meshes.micro_instances[0].0, crate::block::CORNFLOWER);
    scrub(name);
}

#[test]
fn load_world_recovers_block_entities_from_pre_overlay_save() {
    // Goal 3 / Task 1 — backward-compatible old-save load. A save written by an
    // OLDER engine (before `face_overlays` #33 was appended) is a byte-prefix of
    // the current WorldSave: it ends after `bounty_last_refresh_tick`. bincode is
    // positional + non-self-describing, so the current engine hits EOF on the
    // missing trailing field; the old all-or-nothing decode then fell through to
    // `LegacyWorldSave`, whose `upgrade()` hard-codes every block-entity Vec empty
    // — silently dropping chests (+ their items), plots, tip-jar escrow, etc. The
    // tolerant decode must instead recover everything the older save DID contain.
    use crate::chest::ChestData;
    use crate::item::MaterialId;

    let name = "__test_pre_overlay_block_entity_recovery__";
    scrub(name);

    // Build a REAL current-format WorldSave (so the field order + types are exactly
    // the canonical bincode wire layout — not a hand-replicated mirror), carrying a
    // chest, then drop the three trailing empty face-attachment Vecs
    // (`face_blueprint_blanks`, `face_blueprints`, then `face_overlays`) to make the
    // bytes look like a pre-2026-06-03 save that predates all three fields.
    let mut full: WorldSave = serde_json::from_str(
        r#"{ "seed": 7, "player_x": 1.0, "player_y": 64.0, "player_z": 1.0,
             "player_health": 20.0, "hotbar_slot": 0, "inventory": [] }"#,
    )
    .expect("build minimal WorldSave via serde defaults");
    let mut chest = ChestData::new();
    chest.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 5));
    full.chests = vec![save::SavedChest {
        x: 2,
        y: 70,
        z: 3,
        data: chest,
    }];
    assert!(full.face_overlays.is_empty(), "precondition: no overlays");
    assert!(full.face_blueprints.is_empty(), "precondition: no blueprints");
    assert!(full.face_blueprint_blanks.is_empty(), "precondition: no blank papers");

    let mut bytes = bincode::serialize(&full).expect("serialize WorldSave");
    // Drop the trailing empty side-table fields back to the CLEAN boundary just
    // after `face_blueprints`, so the tolerant decoder must default the absent
    // tail and still recover the chest (serialised much earlier) rather than
    // EOF→LegacyWorldSave. The fields from that boundary to the end are all
    // empty/zero: `face_blueprint_blanks` (8) + `workshop` (`WorkshopProjects` =
    // empty Vec 8 + `next_id: u32` 4 = 12) + `carts` (8) + `graves` (8) +
    // `waypoints` (8) + Spec 48 `block_meta` (8) + `power_devices` (8) + Wave 2c
    // `signs` (8) + `item_frames` (8) + Wave 3 `locked_slots` (8) + Wave 5
    // `hostile_acts` (8) = 92 trailing zero bytes. (Self-verifying: if bincode
    // ever changed the empty-Vec/u32 encoding, or a new trailing field is
    // appended without updating this count, the assert catches it.)
    // …through `saved_mobs` (116 bytes) + the newest `satoshi` field. Compute
    // the latter's default size so this self-adjusts if SatoshiState grows; the
    // 116 still pins every field before it.
    let satoshi_bytes =
        bincode::serialized_size(&crate::satoshi::SatoshiState::default()).unwrap() as usize;
    // +8: `dispensers` (2026-07-04) — one more empty-Vec prefix after satoshi.
    // +8: `rig_clips` (#19, 2026-09-06) — the per-rig clip side table.
    let tail = 116 + satoshi_bytes + 8 + 8;
    assert_eq!(
        &bytes[bytes.len() - tail..],
        vec![0u8; tail].as_slice(),
        "empty trailing side-tables (face_blueprint_blanks … saved_mobs, satoshi) must serialise as zero bytes"
    );
    bytes.truncate(bytes.len() - tail);

    // Write the crafted older bytes to the real save path; load via the real path.
    let dir = save::world_dir(name);
    std::fs::create_dir_all(&dir).expect("create world dir");
    std::fs::write(dir.join("world.dat"), &bytes).expect("write world.dat");

    let mut world = World::new();
    let (loaded, _chunks) = save::load_world(name, &mut world).expect("load_world");

    // The chest must survive — NOT be dropped by an EOF->LegacyWorldSave fallback
    // (legacy `upgrade()` hard-codes `chests = Vec::new()`).
    assert_eq!(
        loaded.chests.len(),
        1,
        "chest dropped — older save fell through to the lossy LegacyWorldSave path"
    );
    assert_eq!(
        (loaded.chests[0].x, loaded.chests[0].y, loaded.chests[0].z),
        (2, 70, 3)
    );
    let rehydrated = world
        .chest_at((2, 70, 3))
        .expect("chest not rehydrated into the world");
    assert_eq!(rehydrated.slots[0].as_ref().unwrap().count, 5);
    // The missing trailing fields default cleanly to empty — no error.
    assert!(loaded.face_overlays.is_empty());
    assert!(loaded.face_blueprints.is_empty());
    assert!(loaded.face_blueprint_blanks.is_empty());
    assert!(loaded.carts.is_empty());

    scrub(name);
}

#[test]
fn satoshi_state_round_trips_through_save() {
    // Satoshi's per-world progress + house origin must survive save/load. The
    // villager entity is re-derived on load; only this state persists.
    let mut full: WorldSave = serde_json::from_str(
        r#"{ "seed": 7, "player_x": 1.0, "player_y": 64.0, "player_z": 1.0,
             "player_health": 20.0, "hotbar_slot": 0, "inventory": [] }"#,
    )
    .expect("build minimal WorldSave");
    assert_eq!(
        full.satoshi,
        crate::satoshi::SatoshiState::default(),
        "a pre-Satoshi save defaults the field cleanly (serde default)"
    );
    full.satoshi = crate::satoshi::SatoshiState {
        home: Some([4, 65, 3]),
        has_greeted: true,
        has_gifted: true,
        schematic_gifted: true,
        waved_off: true,
        last_initiated_tick: 1234,
        enabled: true,
    };
    let bytes = bincode::serialize(&full).expect("serialize");
    let loaded = save::read_world_save(&bytes).expect("decode via the real tolerant path");
    assert_eq!(
        loaded.satoshi, full.satoshi,
        "Satoshi state survives the save round-trip intact"
    );
    assert_eq!(loaded.satoshi.home, Some([4, 65, 3]));
    assert!(loaded.satoshi.has_gifted && loaded.satoshi.schematic_gifted);
}

#[test]
fn load_autosave_recovers_block_entities_from_pre_overlay_save() {
    // Goal 3 / Task 1 — the tolerant decode (read_world_save) is wired into
    // load_autosave too, not only load_world. This independently locks that wiring: a
    // pre-overlay AUTOSAVE blob must recover its chest, not EOF->legacy-drop it. (A
    // full-format autosave — as the rehydration test uses — would NOT catch a
    // regression of this wiring, since full bytes decode under either decode path.)
    use crate::chest::ChestData;
    use crate::item::MaterialId;

    let name = "__test_pre_overlay_autosave_recovery__";
    scrub(name);

    // Same crafted pre-2026-06-03 byte stream as the load_world recovery test, written
    // to the autosave dir instead of the manual-save dir.
    let mut full: WorldSave = serde_json::from_str(
        r#"{ "seed": 7, "player_x": 1.0, "player_y": 64.0, "player_z": 1.0,
             "player_health": 20.0, "hotbar_slot": 0, "inventory": [] }"#,
    )
    .expect("build minimal WorldSave");
    let mut chest = ChestData::new();
    chest.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 5));
    full.chests = vec![save::SavedChest {
        x: 2,
        y: 70,
        z: 3,
        data: chest,
    }];
    let mut bytes = bincode::serialize(&full).expect("serialize WorldSave");
    // Same clean-boundary truncation as the load_world recovery test: drop the 92
    // trailing zero bytes (face_blueprint_blanks + workshop + carts + graves +
    // waypoints + Spec 48 block_meta + power_devices + Wave 2c signs +
    // item_frames + Wave 3 locked_slots + Wave 5 hostile_acts) so the autosave
    // path's tolerant decode must recover the chest, not EOF→legacy-drop it.
    let satoshi_bytes =
        bincode::serialized_size(&crate::satoshi::SatoshiState::default()).unwrap() as usize;
    // +8: `dispensers` (2026-07-04) — one more empty-Vec prefix after satoshi.
    // +8: `rig_clips` (#19, 2026-09-06) — the per-rig clip side table.
    let tail = 116 + satoshi_bytes + 8 + 8;
    assert_eq!(
        &bytes[bytes.len() - tail..],
        vec![0u8; tail].as_slice(),
        "empty trailing side-tables (face_blueprint_blanks … saved_mobs, satoshi) must serialise as zero bytes"
    );
    bytes.truncate(bytes.len() - tail);

    let dir = save::world_dir(name).join("autosave");
    std::fs::create_dir_all(&dir).expect("create autosave dir");
    std::fs::write(dir.join("world.dat"), &bytes).expect("write autosave world.dat");

    let mut world = World::new();
    let (loaded, _chunks) = save::load_autosave(name, &mut world).expect("load_autosave");

    assert_eq!(
        loaded.chests.len(),
        1,
        "chest dropped on autosave tolerant decode — load_autosave not recovering older saves"
    );
    assert!(
        world.chest_at((2, 70, 3)).is_some(),
        "chest not rehydrated into the world via load_autosave"
    );

    scrub(name);
}

#[test]
fn legacy_multiplayer_save_with_empty_primary_inventory_falls_back_not_dropped() {
    // Goal 3 / Task 1 regression — found by the adversarial review of this branch.
    // A genuine pre-item-persistence (legacy) save with an EMPTY primary inventory +
    // a second player in the legacy `players` Vec must route to the LegacyWorldSave
    // fallback and preserve that player — NOT be silently decoded as a modern save
    // with players=[]. The first tolerant-decode version regressed this: the empty
    // primary inventory let the strict prefix read cleanly, then the misaligned legacy
    // `players` bytes hit a mid-field EOF that read_tail blanket-swallowed -> the whole
    // decode returned Ok with players dropped, never reaching the legacy fallback.
    use crate::block::STONE;

    // Test-local Serialize mirrors of the legacy wire shapes (the production legacy
    // structs are Deserialize-only). Field order/types MUST match save.rs:380-409.
    #[derive(serde::Serialize)]
    struct LegacySlotWire {
        block_id: u16,
        count: u8,
    }
    #[derive(serde::Serialize)]
    struct LegacyPlayerWire {
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        pitch: f32,
        health: f32,
        hotbar_slot: usize,
        inventory: Vec<LegacySlotWire>,
    }
    #[derive(serde::Serialize)]
    struct LegacyWorldWire {
        seed: u32,
        player_x: f32,
        player_y: f32,
        player_z: f32,
        player_health: f32,
        hotbar_slot: usize,
        inventory: Vec<LegacySlotWire>,
        players: Vec<LegacyPlayerWire>,
    }

    let name = "__test_legacy_multiplayer_empty_primary_inv__";
    scrub(name);

    let legacy = LegacyWorldWire {
        seed: 11,
        player_x: 1.0,
        player_y: 64.0,
        player_z: 2.0,
        player_health: 20.0,
        hotbar_slot: 0,
        inventory: vec![], // EMPTY primary inventory — the aliasing trigger.
        players: vec![LegacyPlayerWire {
            x: 9.0,
            y: 65.0,
            z: 9.0,
            yaw: 0.5,
            pitch: -0.1,
            health: 14.0,
            hotbar_slot: 2,
            inventory: vec![LegacySlotWire {
                block_id: STONE,
                count: 4,
            }],
        }],
    };
    let bytes = bincode::serialize(&legacy).expect("serialize legacy stream");

    let dir = save::world_dir(name);
    std::fs::create_dir_all(&dir).expect("create world dir");
    std::fs::write(dir.join("world.dat"), &bytes).expect("write world.dat");

    let mut world = World::new();
    let (loaded, _chunks) = save::load_world(name, &mut world).expect("load_world");

    // The second player must survive via LegacyWorldSave::upgrade(), not be dropped.
    assert_eq!(
        loaded.players.len(),
        1,
        "legacy second player dropped — tolerant decode swallowed the misaligned EOF \
         instead of falling back to LegacyWorldSave"
    );
    assert_eq!(loaded.players[0].x, 9.0);
    assert_eq!(loaded.players[0].health, 14.0);
    assert_eq!(
        loaded.players[0].inventory.len(),
        1,
        "legacy player's inventory dropped"
    );

    scrub(name);
}

#[test]
fn autosave_rehydrates_overlay_chest_and_plot_into_world() {
    // Goal 3 / Task 2 — crash-recovery integrity. `load_autosave` previously
    // restored ONLY chunks, silently dropping every block-entity + overlay that
    // `autosave_world` had already written to disk. The shared
    // `apply_world_save_state` helper makes `load_autosave` rehydrate the world the
    // same way `load_world` does. Assert against the WORLD (not the returned
    // `WorldSave`): the bytes always round-tripped: the bug was that they were never
    // applied to the `&mut World`.
    use crate::chest::ChestData;
    use crate::item::MaterialId;
    use crate::plot::{PlotData, PlotOwner};
    use crate::world::FaceAttachment;

    let name = "__test_autosave_rehydrate_overlay_chest_plot__";
    scrub(name);

    let mut world = World::new();

    // A chest holding a known item.
    world.set_block(2, 70, 3, crate::block::CHEST);
    let mut chest = ChestData::new();
    chest.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, 5));
    world.insert_chest((2, 70, 3), chest);

    // A plot owned by local player 0.
    world
        .plots
        .push(PlotData::from_marker(PlotOwner::LocalPlayer(0), 10, 64, -5));

    // A face-overlay (wallpaper) on face 0 of a block.
    world.set_face_attachment(
        (1, 2, 3),
        0,
        FaceAttachment::Wallpaper(crate::block::WALLPAPER_RED),
    );

    let p = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::autosave_world(name, &world, &[p], 42, &[], &[]).expect("autosave_world");

    // Crash-recovery load. After the fix, the world is rehydrated — not just chunks.
    let mut loaded = World::new();
    save::load_autosave(name, &mut loaded).expect("load_autosave");

    let restored_chest = loaded
        .chest_at((2, 70, 3))
        .expect("chest dropped on autosave crash-recovery");
    assert_eq!(
        restored_chest.slots[0].as_ref().unwrap().count,
        5,
        "chest contents lost on autosave crash-recovery"
    );
    assert_eq!(loaded.plots.len(), 1, "plot dropped on autosave crash-recovery");
    assert!(
        matches!(loaded.plots[0].owner, PlotOwner::LocalPlayer(0)),
        "plot owner wrong after autosave crash-recovery"
    );
    assert!(
        matches!(loaded.face_attachment_at((1, 2, 3), 0), Some(FaceAttachment::Wallpaper(b)) if *b == crate::block::WALLPAPER_RED),
        "face overlay dropped on autosave crash-recovery"
    );

    scrub(name);
}

#[test]
fn raid_kills_leaderboard_survives_save_load() {
    // Goal 3 review follow-up — `raid_kills` was written on every save but restored
    // on NO load path (apply_world_save_state dropped it), so the per-(village,player)
    // raid-defender leaderboard reset to empty on every reload. It is now restored via
    // the shared helper (one fix repairs load_world + load_autosave + WASM at once).
    // VillageId = (i32, i32), PlayerKey = usize.
    let name = "__test_raid_kills_round_trip__";
    scrub(name);

    let mut world = World::new();
    world.raid_kills.insert(((3, 4), 0usize), 7u32);
    world.raid_kills.insert(((3, 4), 1usize), 2u32);
    world.raid_kills.insert(((-5, 9), 0usize), 11u32);

    let p = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p], 42, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    save::load_world(name, &mut loaded).expect("load_world");

    assert_eq!(loaded.raid_kills.len(), 3, "raid_kills dropped on load");
    assert_eq!(
        loaded.raid_kills.get(&((3, 4), 0)),
        Some(&7),
        "raid-kill total dropped on load"
    );
    assert_eq!(loaded.raid_kills.get(&((3, 4), 1)), Some(&2));
    assert_eq!(loaded.raid_kills.get(&((-5, 9), 0)), Some(&11));

    scrub(name);
}

#[test]
fn world_meta_world_override_round_trips_and_old_meta_loads_clean() {
    use crate::save::WorldMeta;
    // New field round-trips through the JSON encode/decode.
    let mut meta = WorldMeta::new("Test");
    meta.world_override = Some(vec![2, 7, 7, 7]); // a stand-in OverrideSet blob
    let json = serde_json::to_string(&meta).unwrap();
    let back: WorldMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(back.world_override, Some(vec![2, 7, 7, 7]));

    // An OLD meta JSON (no world_override key) loads clean via #[serde(default)].
    let old = r#"{"display_name":"Old","description":"","game_mode":"survival","created_at":"","icon":null}"#;
    let loaded: WorldMeta = serde_json::from_str(old).unwrap();
    assert_eq!(loaded.world_override, None, "absent field defaults to None");
}

#[test]
fn cart_mid_line_survives_save_load_round_trip() {
    // Rail freight Phase 1 (Task 1.6) — a cart caught MID-LINE (non-trivial
    // state: a specific cell, a came_from history, progress between 0 and 1, a
    // live speed, and freight in its cargo) must round-trip through the real
    // production serialize (`save::save_world`) + tolerant decode
    // (`load_world` → `read_world_save`) with every CartData field intact.
    //
    // Carts are ECS entities, so the save snapshots them from a `hecs::World`
    // (via `carts_to_saved`) and the load re-spawns them with `spawn_cart_from`.
    // This test drives that exact path: build the `Vec<SavedCart>` from an ECS,
    // save, load, then re-spawn into a fresh ECS and assert the restored cart.
    use crate::cart::{self, CartData};
    use crate::chest::ChestData;
    use crate::item::MaterialId;

    let name = "__test_cart_mid_line_round_trip__";
    scrub(name);

    // Source ECS with a single cart frozen mid-cell, carrying freight.
    let mut src_ecs = hecs::World::new();
    let mut cargo = ChestData::new();
    cargo.slots[2] = Some(ItemStack::new_material(MaterialId::IronIngot, 6));
    cargo.slots[9] = Some(ItemStack::new_material(MaterialId::Bread, 3));
    let original = CartData {
        cell: (5, 64, 8),
        came_from: Some((4, 64, 8)),
        progress: 0.42,
        speed: cart::CART_SPEED,
        facing: 1.0,
        cargo,
        // CA1 — a non-default (Diamond) hull so this end-to-end disk round-trip
        // also proves the appended armour tier survives save → load → restore.
        hull: cart::Hull::Diamond,
        // CA4 — transient breach accumulator (`#[serde(skip)]`); not persisted,
        // restores to 0.0 regardless. Fresh cart has none.
        breach: 0.0,
    };
    src_ecs.spawn((
        crate::entity::Position(cart::cell_centre(original.cell)),
        cart::CartEntity,
        original.clone(),
    ));

    // Snapshot exactly as every production save site does, then save+load via
    // the real disk path.
    let carts = save::carts_to_saved(&src_ecs);
    assert_eq!(carts.len(), 1, "one cart snapshotted from the source ECS");

    let world = World::new();
    let p = make_slot(0, Vec3::new(0.0, 80.0, 0.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p], 42, &carts, &[]).expect("save_world");

    let mut loaded_world = World::new();
    let (loaded, _chunks) = save::load_world(name, &mut loaded_world).expect("load_world");

    // The cart survived the WorldSave round-trip.
    assert_eq!(loaded.carts.len(), 1, "cart dropped on save/load");

    // Restore it into a fresh ECS exactly as the load path does, then read it
    // back from that ECS (proving spawn_cart_from + the components round-trip).
    let mut dst_ecs = hecs::World::new();
    for s in &loaded.carts {
        cart::spawn_cart_from(&mut dst_ecs, s.data.clone());
    }
    let restored: CartData = dst_ecs
        .query::<&CartData>()
        .iter()
        .map(|(_, d)| d.clone())
        .next()
        .expect("restored cart present in the destination ECS");

    // Every load-bearing field survives.
    assert_eq!(restored.cell, original.cell, "cell must round-trip");
    assert_eq!(restored.came_from, original.came_from, "came_from must round-trip");
    assert!((restored.progress - original.progress).abs() < 1e-6, "progress must round-trip");
    assert!((restored.speed - original.speed).abs() < 1e-6, "speed must round-trip");
    assert!((restored.facing - original.facing).abs() < 1e-6, "facing must round-trip");
    // CA1 — the hull armour tier survives the disk round-trip.
    assert_eq!(restored.hull, original.hull, "hull tier must round-trip");
    // Cargo contents (count + which slots) survive.
    assert_eq!(restored.cargo.occupied(), 2, "both cargo stacks survive");
    assert_eq!(restored.cargo.slots[2].as_ref().unwrap().count, 6, "iron stack count");
    assert_eq!(restored.cargo.slots[9].as_ref().unwrap().count, 3, "bread stack count");

    // The re-spawned cart carries the marker + a Position anchor (so it renders
    // and so tick_carts will pick it up).
    let (_id, _) = dst_ecs.query::<&cart::CartEntity>().iter().next().expect("CartEntity marker");
    let pos = dst_ecs
        .query::<&crate::entity::Position>()
        .iter()
        .map(|(_, p)| p.0)
        .next()
        .expect("Position anchor present");
    assert_eq!(pos, cart::cell_centre(original.cell), "anchor seeded from the saved cell");

    scrub(name);
}

#[test]
fn cart_survives_full_game_server_save_load_and_keeps_rolling() {
    // Rail freight Phase 1 (Task 1.6) — end-to-end at the server authority: a cart
    // dispatched on real track is snapshotted by `GameServer::save()` from the
    // SAME ECS `tick_carts` advances, and a fresh `GameServer::initial_load()`
    // re-spawns it into ITS ECS — so the restored cart actually keeps rolling.
    // This is the regression that proves the save snapshots the ticked ECS and
    // the load restores into the ticked ECS (not two different ones).
    use crate::cart;
    use crate::rail;

    let name = "__test_cart_server_round_trip__";
    scrub(name);

    // First session: lay track, spawn + dispatch a cart, let it roll a few
    // cells (so it's mid-line with a real came_from + progress), then save.
    let saved_state = {
        let mut server = GameServer::new(1, name.to_string(), 42);
        for x in 0..6 {
            server.world.set_block(x, 64, 0, rail::TRACK);
        }
        let id = cart::spawn_cart(&mut server.ecs, (0, 64, 0));
        {
            let mut c = server.ecs.get::<&mut cart::CartData>(id).unwrap();
            c.speed = cart::CART_SPEED;
        }
        // Roll for a while — enough to leave the terminus and be mid-line.
        for _ in 0..30 {
            cart::tick_carts(&mut server.ecs, &mut server.world);
        }
        let mid: cart::CartData = (*server.ecs.get::<&cart::CartData>(id).unwrap()).clone();
        assert!(mid.cell.0 > 0, "cart left the start terminus before save");
        assert!(mid.speed > 0.0, "cart still rolling at save time");
        server.save();
        mid
    };

    // Second session: a brand-new server for the same world loads from disk and
    // must re-spawn the cart into its ECS at the saved state.
    let mut server2 = GameServer::new(1, name.to_string(), 42);
    server2.initial_load().expect("the saved world loads");

    let restored: cart::CartData = server2
        .ecs
        .query::<&cart::CartData>()
        .iter()
        .map(|(_, d)| d.clone())
        .next()
        .expect("cart not restored into the reloaded server's ECS");
    assert_eq!(restored.cell, saved_state.cell, "restored cart cell matches save");
    assert_eq!(restored.came_from, saved_state.came_from, "came_from matches");
    assert!((restored.progress - saved_state.progress).abs() < 1e-6, "progress matches");
    assert!((restored.speed - saved_state.speed).abs() < 1e-6, "speed matches");

    // The reloaded cart KEEPS ROLLING when the reloaded server ticks — the whole
    // point of persistence. The track was reloaded from disk too, so tick_carts
    // can read it.
    let before = restored.cell;
    // Generous tick budget: CART_SPEED ≈ 0.08 cells/tick, so reaching + parking
    // at the east terminus from mid-line takes well under 200 ticks. The point
    // is the reloaded cart MOVES under its own sim, then parks at the end.
    for _ in 0..200 {
        server2.tick(); // GameServer::tick() runs cart::tick_carts internally
    }
    let after: cart::CartData = server2
        .ecs
        .query::<&cart::CartData>()
        .iter()
        .map(|(_, d)| d.clone())
        .next()
        .expect("cart still present after ticking the reloaded server");
    assert_eq!(after.cell, (5, 64, 0), "reloaded cart rolled on to the east terminus");
    assert!(after.cell.0 >= before.0, "reloaded cart advanced (never went backwards)");
    assert_eq!(after.speed, 0.0, "parked at the terminus after rolling");

    scrub(name);
}

/// Spec 02 §7.5 — an edited column streamed out of range lives in the evicted
/// store and is still written by the next save; save → load keeps the edit.
/// (Pre-fix, `stream_chunks` dropped the column and the next save overwrote
/// its file with regenerated terrain once the player came back.)
#[test]
fn evicted_edited_column_is_saved_and_round_trips() {
    let name = "__test_evicted_column_save__";
    scrub(name);

    let bg = crate::biome::BiomeGenerator::new(11);
    let mut world = World::new();
    world.generate_column(0, 0, &bg);
    world.generate_column(20, 0, &bg);
    // Edit a column far from the player, then stream it out.
    let (x, y, z) = (20 * 16 + 3, 50, 5);
    world.place_player_block(x, y, z, crate::block::GLASS);
    assert!(world.evict_column(20, 0), "edited column must be evicted, not dropped");
    assert!(!world.has_chunk(20, 3, 0));

    let p1 = make_slot(0, Vec3::new(8.0, 80.0, 8.0), 0.0, 0.0, 20.0);
    save::save_world(name, &world, &[p1], 11, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    save::load_world(name, &mut loaded).expect("load_world");
    assert_eq!(loaded.get_block(x, y, z), crate::block::GLASS, "edit survives save → load");
    assert!(loaded.is_placed(x, y, z));
    // Loaded chunks carry `persist`, so unloading them keeps them too.
    assert!(loaded.evict_column(20, 0));
    assert!(loaded.restore_column(20, 0));
    assert_eq!(loaded.get_block(x, y, z), crate::block::GLASS);

    scrub(name);
}
