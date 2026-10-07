//! Server-side death-drop regression tests (wave-hardening backlog,
//! 2026-07-11). `GameServer::tick` used to DISCARD `combat::despawn_dead`'s
//! return, so a mob killed in the hosted/dedicated-server sim dropped
//! nothing at all — no loot table, no wolf-specific table, no cargo-pack
//! spill. The shared `death_drops` routing now spawns the same loot on
//! both simulation sides.
//!
//! (These tests pin the server-authoritative half. The wire half shipped as
//! death-drops phase 2 — the entity diff (`entity_broadcast`) sends item spawns with stack
//! payload, and `test_integration/late_join.rs` covers the late-joiner
//! backfill of those spawns.)

use crate::test_harness::{TestConfig, TestHost};

fn dead_health() -> crate::combat::Health {
    let mut h = crate::combat::Health::new(10.0);
    h.current = 0.0;
    h
}

#[test]
fn server_kill_spawns_the_generic_loot_table() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.server.ecs.spawn((
        crate::entity::MobKind(crate::mob::MobType::Cow),
        crate::entity::Position(glam::Vec3::new(8.0, 64.0, 8.0)),
        dead_health(),
    ));

    host.tick(1);

    // A cow ALWAYS drops 1-3 RawBeef (`mob::drops_for`), so >= 1 is
    // deterministic regardless of the seed.
    assert!(
        host.dropped_material_count(crate::item::MaterialId::RawBeef) >= 1,
        "a cow killed in the server sim must drop its loot table"
    );
}

#[test]
fn server_kill_routes_wolves_through_the_wolf_table() {
    let mut host = TestHost::start_with(TestConfig::default());
    // An untamed wolf → leather + bones via `wolf::drops_for_wolf`.
    host.server.ecs.spawn((
        crate::entity::MobKind(crate::mob::MobType::Wolf),
        crate::entity::Position(glam::Vec3::new(8.0, 64.0, 8.0)),
        dead_health(),
        crate::wolf::WolfData::untamed(),
    ));
    // A tamed wolf 40 blocks away → drops NOTHING (emotional loss).
    let mut tamed = crate::wolf::WolfData::untamed();
    tamed.ownership.owner_pubkey = "local-player-0".into();
    host.server.ecs.spawn((
        crate::entity::MobKind(crate::mob::MobType::Wolf),
        crate::entity::Position(glam::Vec3::new(48.0, 64.0, 8.0)),
        dead_health(),
        tamed,
    ));

    host.tick(1);

    assert_eq!(
        host.dropped_material_count(crate::item::MaterialId::Leather),
        1,
        "exactly the untamed wolf's leather — the tamed one drops nothing"
    );
    assert!(
        host.dropped_material_count(crate::item::MaterialId::Bone) >= 1,
        "the untamed wolf's bones dropped"
    );
}

#[test]
fn server_kill_spills_a_dead_steeds_cargo_pack() {
    let mut host = TestHost::start_with(TestConfig::default());
    let mut pack = crate::chest::ChestData::new();
    pack.slots[0] = Some(crate::item::ItemStack::new_material(
        crate::item::MaterialId::IronIngot,
        7,
    ));
    let mut horse_data = crate::horse_ai::HorseData::new();
    horse_data.pack = Some(pack);
    host.server.ecs.spawn((
        crate::entity::MobKind(crate::mob::MobType::Donkey),
        crate::entity::Position(glam::Vec3::new(8.0, 64.0, 8.0)),
        dead_health(),
        horse_data,
    ));

    host.tick(1);

    assert_eq!(
        host.dropped_material_count(crate::item::MaterialId::IronIngot),
        7,
        "a dead donkey's cargo pack spills its contents instead of destroying them"
    );
}
