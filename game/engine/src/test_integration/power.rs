//! Spec 48 (Electricity) — power persistence round-trips.
//!
//! The transient power flood (`PowerState.energised`) is rebuilt on load by
//! `reseed_on_load`, but two things are NOT derivable from the saved block ids
//! and so MUST be persisted:
//!   * the per-block **meta byte** (facing / lever-latch / gate op), and
//!   * per-device **runtime state** (`PowerDeviceData`: generator fuel/charge,
//!     battery charge, hand-crank run-down counter).
//! Without persistence, a placed circuit silently resets on reload. These tests
//! lock in that a placed device + its meta survive a real `save_world` →
//! `load_world` disk round-trip, and that the load path reseeds the model.
//!
//! Native-only — `save_world` is not exposed as a sync API on WASM (mirrors
//! `save_load.rs`).

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec3;

use crate::block;
use crate::meta::Facing;
use crate::player_slot::PlayerSlot;
use crate::power::{PowerDeviceData, PowerDeviceKind};
use crate::save;
use crate::world::World;

fn scrub(name: &str) {
    let _ = save::delete_world(name);
}

#[test]
fn power_device_and_meta_survive_save_reload() {
    let name = "__test_power_persist__";
    scrub(name);

    let mut world = World::new();

    // A Steam Generator with NON-default runtime state (running, mid-charge):
    // none of `on`/`charge` is recoverable from the block id, so the round-trip
    // below proves they're persisted, not re-defaulted.
    let dev_pos = (5, 64, 1);
    world.set_block(dev_pos.0, dev_pos.1, dev_pos.2, block::STEAM_GENERATOR_LIT);
    let mut dev = PowerDeviceData::new(PowerDeviceKind::SteamGenerator, Facing::North);
    dev.on = true;
    dev.charge = 42;
    world.insert_power_device(dev_pos, dev);

    // A lever's facing + latch packed into a meta byte at another cell.
    let meta_pos = (3, 64, -2);
    world.set_block(meta_pos.0, meta_pos.1, meta_pos.2, block::LEVER);
    world.block_meta.insert(meta_pos, 0b0000_0110);

    let p1 = PlayerSlot::new(0, Vec3::new(0.0, 80.0, 0.0), 1.0);
    save::save_world(name, &world, &[p1], 7, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let (saved, _chunks) = save::load_world(name, &mut loaded).expect("load_world");

    // Collection + decode: the WorldSave carries both side-tables.
    assert_eq!(
        saved.block_meta,
        vec![(3, 64, -2, 0b0000_0110)],
        "meta byte collected into the save"
    );
    assert_eq!(saved.power_devices.len(), 1, "device collected into the save");
    assert_eq!(saved.power_devices[0].x, 5);

    // Restore: the freshly-loaded world has the meta byte + the device, with
    // its runtime state intact.
    assert_eq!(
        loaded.block_meta.get(&meta_pos).copied(),
        Some(0b0000_0110),
        "meta byte restored into the world"
    );
    let d = loaded.power_device_at(dev_pos).expect("device restored");
    assert_eq!(d.kind, PowerDeviceKind::SteamGenerator);
    assert!(d.on, "running state restored");
    assert_eq!(d.charge, 42, "mid-charge restored");

    // reseed_on_load ran on the load path: the restored device is enqueued so
    // the next `power_tick` recomputes lit cables/lamps from saved device state.
    assert!(
        loaded.scheduler.has_work(),
        "load reseeds the power model (device enqueued)"
    );

    scrub(name);
}

#[test]
fn water_wheel_device_survives_save_reload() {
    // Spec 48 Phase 4 — the Water Wheel was the last variant of the
    // bincode-positional `PowerDeviceKind` when it landed, and a round-trip is
    // the guard that an appended variant encodes and decodes as itself rather
    // than silently reading back as a neighbouring kind. (`Windmill` holds the
    // last slot now — see `windmill_device_survives_save_reload` below.) Saved
    // mid-turn (on the turning block id) so the restored state is non-default.
    let name = "__test_water_wheel_persist__";
    scrub(name);

    let mut world = World::new();
    let pos = (2, 63, -4);
    world.set_block(pos.0, pos.1, pos.2, block::WATER_WHEEL_TURNING);
    let mut dev = PowerDeviceData::new(PowerDeviceKind::WaterWheel, Facing::East);
    dev.on = true;
    world.insert_power_device(pos, dev);

    let p1 = PlayerSlot::new(0, Vec3::new(0.0, 80.0, 0.0), 1.0);
    save::save_world(name, &world, &[p1], 11, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let _ = save::load_world(name, &mut loaded).expect("load_world");

    let d = loaded.power_device_at(pos).expect("water wheel restored");
    assert_eq!(d.kind, PowerDeviceKind::WaterWheel, "appended kind round-trips as itself");
    assert!(d.on, "turning state restored");
    assert_eq!(
        loaded.get_block(pos.0, pos.1, pos.2),
        block::WATER_WHEEL_TURNING,
        "the turning block id round-trips"
    );

    scrub(name);
}

#[test]
fn windmill_device_survives_save_reload() {
    // Wind, Copper & Electricity wave §2.2 — `Windmill` is now the LAST variant
    // of the bincode-positional `PowerDeviceKind`, so it takes over the Water
    // Wheel's job as the round-trip guard: the appended variant must encode and
    // decode as itself, not as the neighbour it was appended after. Saved
    // mid-gale so the restored state is the non-default one.
    let name = "__test_windmill_persist__";
    scrub(name);

    let mut world = World::new();
    let pos = (-3, 71, 5);
    world.set_block(pos.0, pos.1, pos.2, block::WINDMILL_TURNING);
    let mut dev = PowerDeviceData::new(PowerDeviceKind::Windmill, Facing::South);
    dev.on = true;
    world.insert_power_device(pos, dev);

    let p1 = PlayerSlot::new(0, Vec3::new(0.0, 80.0, 0.0), 1.0);
    save::save_world(name, &world, &[p1], 12, &[], &[]).expect("save_world");

    let mut loaded = World::new();
    let _ = save::load_world(name, &mut loaded).expect("load_world");

    let d = loaded.power_device_at(pos).expect("windmill restored");
    assert_eq!(d.kind, PowerDeviceKind::Windmill, "appended kind round-trips as itself");
    assert_eq!(d.facing, Facing::South, "facing survives the round-trip");
    assert!(d.on, "turning state restored");
    assert_eq!(
        loaded.get_block(pos.0, pos.1, pos.2),
        block::WINDMILL_TURNING,
        "the turning block id round-trips"
    );

    scrub(name);
}

#[test]
fn old_save_without_power_fields_loads_clean() {
    // Forward-compat: a save written before Spec 48 (no block_meta /
    // power_devices bytes) must load with both side-tables empty, not fail.
    // The tolerant decoder defaults the appended fields via `read_tail`.
    let json = r#"{
        "seed": 1,
        "player_x": 0.0, "player_y": 64.0, "player_z": 0.0,
        "player_health": 20.0,
        "hotbar_slot": 0,
        "inventory": []
    }"#;
    let parsed: save::WorldSave = serde_json::from_str(json)
        .expect("pre-Spec-48 saves missing the power fields must default to empty");
    assert!(parsed.block_meta.is_empty());
    assert!(parsed.power_devices.is_empty());
}
