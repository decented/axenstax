//! D4 slice 4: the real authority death boundary, without a private client sweep.
use crate::test_harness::{TestConfig, TestHost};
use crate::{
    combat::{Attacker, Health, LastAttacker},
    entity::{MobKind, Position},
    mob::MobType,
};
use glam::Vec3;

fn dead() -> Health {
    let mut h = Health::new(10.0);
    h.current = 0.0;
    h
}

#[test]
fn d4_death_dedicated_rescues_owned_pet_before_any_drop_or_credit() {
    let mut host = TestHost::start_with(TestConfig {
        num_players: 0,
        ..TestConfig::default()
    });
    host.server.world.set_block(5, 64, 0, crate::block::PET_BED);
    let mut wolf = crate::wolf::WolfData::untamed();
    wolf.ownership.owner_pubkey = "local-player-0".into();
    let pet = host.server.ecs.spawn((
        MobKind(MobType::Wolf),
        Position(Vec3::new(0.0, 64.0, 0.0)),
        dead(),
        wolf,
    ));
    host.tick(1);
    assert!(
        host.server.ecs.contains(pet),
        "authority must rescue before despawn"
    );
    assert_eq!(
        host.server.ecs.get::<&Position>(pet).unwrap().0,
        Vec3::new(5.5, 65.0, 0.5)
    );
    assert_eq!(
        host.server.ecs.get::<&Health>(pet).unwrap().current,
        crate::mob::mob_def(MobType::Wolf).health as f32
    );
    assert_eq!(
        host.server
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .count(),
        0
    );
    assert!(host.server.pending_kill_events.is_empty());
    host.tick(1);
    assert!(host.server.ecs.contains(pet));
}

#[test]
fn d4_death_colocated_wild_nostrich_keeps_its_own_tame_flag() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.server.players[0].server_simulated = true;
    host.server.players[0].player.pos = Vec3::new(50.0, 64.0, 0.0);
    let generation = host.server.players[0].attach_gen;
    for tamed in [true, false] {
        let mut data = crate::nostrich::NostrichData::untamed();
        if tamed {
            data.ownership.owner_pubkey = "local-player-0".into();
        }
        host.server.ecs.spawn((
            MobKind(MobType::Nostrich),
            Position(Vec3::new(0.0, 64.0, 0.0)),
            dead(),
            data,
            LastAttacker(Attacker::Remote {
                slot: 0,
                generation,
            }),
        ));
    }
    host.tick(1);
    let mut flags: Vec<_> = host
        .server
        .pending_kill_events
        .iter()
        .map(|(_, k, _)| k.victim_flags)
        .collect();
    flags.sort_unstable();
    assert_eq!(
        flags,
        vec![0, crate::protocol::entity_flags::TAMED],
        "each exact victim retains its tame status"
    );
}

#[test]
fn d4_death_remote_backpressure_leaves_rescue_and_kill_pending() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.server.players[0].server_simulated = true;
    host.server.players[0].player.pos = Vec3::new(50.0, 64.0, 0.0);
    let generation = host.server.players[0].attach_gen;
    for _ in 0..crate::sim_death::MAX_REMOTE_EVENTS {
        host.server
            .queue_kill_event(0, generation, MobType::Cow, Vec3::ZERO, false, false);
    }
    let credits = host.server.players[0].progression_credits;
    let cow = host.server.ecs.spawn((
        MobKind(MobType::Cow),
        Position(Vec3::new(0.0, 64.0, 0.0)),
        dead(),
    ));
    let mut wolf = crate::wolf::WolfData::untamed();
    wolf.ownership.owner_pubkey = "local-player-0".into();
    let pet = host.server.ecs.spawn((
        MobKind(MobType::Wolf),
        Position(Vec3::new(0.0, 64.0, 0.0)),
        dead(),
        wolf,
    ));
    host.server.world.set_block(5, 64, 0, crate::block::PET_BED);
    host.tick(1);
    assert!(host.server.ecs.contains(cow));
    assert!(
        host.server.ecs.get::<&Health>(pet).unwrap().is_dead(),
        "no rescue before delivery reservation"
    );
    assert_eq!(host.server.players[0].progression_credits, credits);
    assert_eq!(
        host.server
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .count(),
        0
    );
    host.server.pending_kill_events.clear();
    host.tick(1);
    assert!(!host.server.ecs.contains(cow));
    assert!(!host.server.ecs.get::<&Health>(pet).unwrap().is_dead());
    assert_eq!(host.server.pending_kill_events.len(), 1);
    assert_eq!(host.server.players[0].progression_credits, credits + 1);
}

#[test]
fn d4_death_remote_budget_charges_allocated_capacity_even_when_empty() {
    let mut host = TestHost::start_with(TestConfig::default());
    let bytes_per = std::mem::size_of::<(usize, crate::protocol::KillEventPacket, u32)>();
    host.server.pending_kill_events =
        Vec::with_capacity(crate::sim_death::MAX_REMOTE_BYTES / bytes_per + 1);
    let cow = host
        .server
        .ecs
        .spawn((MobKind(MobType::Cow), Position(Vec3::ZERO), dead()));
    host.tick(1);
    assert!(
        host.server.ecs.contains(cow),
        "allocated bytes, not just count, block mutation"
    );
    host.server.pending_kill_events = Vec::new();
    host.tick(1);
    assert!(!host.server.ecs.contains(cow));
}

#[test]
fn d4_death_undrained_local_effects_defer_world_mutations_and_second_phase_is_refused() {
    let mut host = TestHost::start_with(TestConfig::default());
    let (mut world, mut ecs) = (crate::world::World::new(), hecs::World::new());
    let cow = ecs.spawn((MobKind(MobType::Cow), Position(Vec3::ZERO), dead()));
    host.server
        .sim_events
        .push(crate::sim_mobs::SimEvent::ParrotAlarm {
            pet: cow,
            pos: Vec3::ZERO,
        });
    let locals = [(Vec3::ZERO, false)];
    host.server
        .run_host_death_phase(&mut world, &mut ecs, &locals, (1, 7501), false);
    assert!(ecs.contains(cow));
    host.server.sim_events.take();
    host.server
        .run_host_death_phase(&mut world, &mut ecs, &locals, (1, 7501), true);
    assert!(!ecs.contains(cow));
    let events = host.server.sim_events.take();
    let notices = events
        .iter()
        .flat_map(|e| match e {
            crate::sim_mobs::SimEvent::Deaths(batch) => batch.notices(),
            _ => &[],
        })
        .count();
    assert_eq!(notices, 1);
    let later = ecs.spawn((MobKind(MobType::Cow), Position(Vec3::ZERO), dead()));
    host.server
        .run_host_death_phase(&mut world, &mut ecs, &locals, (1, 7501), true);
    assert!(ecs.contains(later));
    assert_eq!(
        world.sim_tally.get(crate::sim_lend::SimSystem::DespawnDead),
        1
    );
    assert!(host.server.sim_events.take().is_empty());
}

#[test]
fn d4_death_no_lend_departures_are_forgotten_in_both_world_copies() {
    let mut host = TestHost::start_with(TestConfig::default());
    let (mut world, mut ecs) = (crate::world::World::new(), hecs::World::new());
    let attacker = Attacker::Remote {
        slot: 1,
        generation: 9,
    };
    let mut copies = Vec::new();
    for copy in [&mut host.server.ecs, &mut ecs] {
        let mob = copy.spawn((
            MobKind(MobType::Cow),
            Position(Vec3::ZERO),
            Health::new(20.0),
            LastAttacker(attacker),
        ));
        copies.push(mob);
    }
    host.server.released_joiners.push((1, 9));
    host.server
        .run_host_death_phase(&mut world, &mut ecs, &[], (1, 7501), false);
    assert_eq!(
        ecs.get::<&LastAttacker>(copies[1]).unwrap().0,
        Attacker::Departed
    );
    assert_eq!(
        host.server.released_joiners,
        vec![(1, 9)],
        "authority copy still needs cleanup"
    );
    host.server.forget_released_joiners();
    assert_eq!(
        host.server.ecs.get::<&LastAttacker>(copies[0]).unwrap().0,
        Attacker::Departed
    );
    assert!(host.server.released_joiners.is_empty());
}

#[test]
fn d4_death_awaiting_lent_victims_neither_regenerate_nor_breed() {
    let (mut hs, mut host) = super::lent_world::start_lent("dead-backlog-pulse");
    host.ecs.clear();
    host.clock.tick_counter = 99;
    let at = Vec3::new(40.5, 80.0, 40.5);
    host.world.set_block(40, 79, 40, crate::block::SALT_LICK);
    let mut mobs = Vec::new();
    for _ in 0..34 {
        let cow = crate::entity::spawn_mob(&mut host.ecs, MobType::Cow, at);
        host.ecs.get::<&mut Health>(cow).unwrap().current = 0.0;
        host.ecs
            .insert_one(
                cow,
                crate::breeding::InLove {
                    until_tick: 1000,
                    fed_by: Some(Attacker::Local(0)),
                },
            )
            .unwrap();
        mobs.push(cow);
    }
    host.lend_tick(&mut hs);
    let remaining: Vec<_> = mobs.into_iter().filter(|e| host.ecs.contains(*e)).collect();
    assert_eq!(remaining.len(), 2);
    for mob in remaining {
        assert!(
            host.ecs.get::<&Health>(mob).unwrap().is_dead(),
            "pending victim cannot regenerate"
        );
        assert_eq!(
            host.ecs.get::<&Position>(mob).unwrap().0,
            at,
            "pending victim stays inert"
        );
    }
    assert_eq!(
        host.ecs.query::<&crate::breeding::Baby>().iter().count(),
        0,
        "no baby or breed credit from dead parents"
    );
}

#[test]
fn d4_death_awaiting_delivery_preserves_deliberate_cull_after_ttl() {
    let mut host = TestHost::start_with(TestConfig::default());
    let (mut world, mut ecs) = (crate::world::World::new(), hecs::World::new());
    world.set_block(2, 64, 0, crate::block::PET_BED);
    let mut wolf = crate::wolf::WolfData::untamed();
    wolf.ownership.owner_pubkey = "local-player-0".into();
    let pet = ecs.spawn((
        MobKind(MobType::Wolf),
        Position(Vec3::new(0.0, 64.0, 0.0)),
        dead(),
        wolf,
        crate::tameable::DeliberateCull { last_hit_tick: 1 },
    ));
    host.server
        .sim_events
        .push(crate::sim_mobs::SimEvent::ParrotAlarm {
            pet,
            pos: Vec3::ZERO,
        });
    host.server
        .run_host_death_phase(&mut world, &mut ecs, &[], (1, 7501), false);
    assert!(ecs.contains(pet));
    host.server.sim_events.take();
    host.server
        .run_host_death_phase(&mut world, &mut ecs, &[], (200, 7700), false);
    assert!(
        !ecs.contains(pet),
        "delivery delay must not convert an owner's deliberate cull to a rescue"
    );
}

#[test]
fn d4_death_awaiting_cargo_cannot_be_captured_as_a_living_steed() {
    let mut ecs = hecs::World::new();
    let donkey = crate::entity::spawn_mob(&mut ecs, MobType::Donkey, Vec3::ZERO);
    let mut pack = crate::chest::ChestData::new();
    pack.slots[0] = Some(crate::item::ItemStack::new_material(
        crate::item::MaterialId::Bread,
        7,
    ));
    {
        let mut horse = ecs.get::<&mut crate::horse_ai::HorseData>(donkey).unwrap();
        horse.kept_by = Some(0);
        horse.pack = Some(pack);
    }
    ecs.get::<&mut Health>(donkey).unwrap().current = 0.0;
    assert!(
        crate::save::tamed_mobs_to_saved(&ecs).is_err(),
        "capture must wait for pending death/cargo delivery"
    );
}

#[test]
fn d4_death_owned_projectile_kill_is_swept_in_the_same_tick() {
    let mut host = TestHost::start_with(TestConfig {
        num_players: 0,
        ..TestConfig::default()
    });
    let at = Vec3::new(0.0, 64.0, 0.0);
    let cow = crate::entity::spawn_mob(&mut host.server.ecs, MobType::Cow, at);
    host.server.ecs.get::<&mut Health>(cow).unwrap().current = 1.0;
    crate::entity::spawn_arrow(
        &mut host.server.ecs,
        at + Vec3::new(-0.5, 0.5, 0.0),
        Vec3::X,
        100.0,
        None,
    );
    host.tick(1);
    assert!(
        !host.server.ecs.contains(cow),
        "owned server retains its late post-projectile sweep"
    );
    assert!(host.dropped_material_count(crate::item::MaterialId::RawBeef) > 0);
}

#[test]
fn d4_death_full_delivery_queue_keeps_products_species_physics_and_tethers_inert() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.server.players[0].server_simulated = true;
    host.server.players[0].player.pos = Vec3::new(0.5, 64.0, 0.5);
    let generation = host.server.players[0].attach_gen;
    for _ in 0..crate::sim_death::MAX_REMOTE_EVENTS {
        host.server
            .queue_kill_event(0, generation, MobType::Cow, Vec3::ZERO, false, false);
    }
    host.server.tick_counter = 11999;
    let at = Vec3::new(1.0, 64.0, 0.0);
    host.server
        .world
        .set_block(1, 63, 0, crate::block::SALT_LICK);
    let mut mobs = Vec::new();
    for kind in [
        MobType::Nostrich,
        MobType::Chicken,
        MobType::Wolf,
        MobType::Goat,
        MobType::Shark,
        MobType::Bear,
        MobType::Brigand,
    ] {
        let mob = crate::entity::spawn_mob(&mut host.server.ecs, kind, at);
        host.server.ecs.get::<&mut Health>(mob).unwrap().current = 0.0;
        host.server
            .ecs
            .get::<&mut crate::entity::Velocity>(mob)
            .unwrap()
            .0 = Vec3::ONE;
        host.server
            .ecs
            .insert_one(
                mob,
                crate::tether::Tethered {
                    target: crate::tether::TetherTarget::Player(99),
                },
            )
            .unwrap();
        if kind == MobType::Nostrich {
            host.server
                .ecs
                .get::<&mut crate::nostrich::NostrichData>(mob)
                .unwrap()
                .lay_ticks_remaining = 0;
        }
        mobs.push(mob);
    }
    host.tick(1);
    for mob in mobs {
        assert!(host.server.ecs.get::<&Health>(mob).unwrap().is_dead());
        assert_eq!(host.server.ecs.get::<&Position>(mob).unwrap().0, at);
        assert_eq!(
            host.server
                .ecs
                .get::<&crate::entity::Velocity>(mob)
                .unwrap()
                .0,
            Vec3::ONE
        );
        assert!(host.server.ecs.get::<&crate::tether::Tethered>(mob).is_ok());
    }
    assert_eq!(
        host.server
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .count(),
        0,
        "no egg, Lead or loot before capacity is reserved"
    );
    assert_eq!(
        host.server.players[0].combat.health, 20.0,
        "dead hostile bodies land no hits"
    );
    assert_eq!(
        host.server.players[0].player.pos.x, 0.5,
        "waiting corpses cannot push a player"
    );
    assert_eq!(host.server.players[0].player.pos.z, 0.5);
}
