use super::*;
use crate::{
    combat::{Attacker, Health, LastAttacker},
    entity::{MobKind, Position},
    item::{Item, ItemStack, MaterialId},
};

fn dead(ecs: &mut hecs::World, kind: MobType, at: Vec3) -> hecs::Entity {
    let mut health = Health::new(20.0);
    health.current = 0.0;
    ecs.spawn((MobKind(kind), Position(at), health))
}
fn run(world: &mut World, ecs: &mut hecs::World, locals: &[(Vec3, bool)]) -> DeathBatch {
    let mut batch = DeathBatch::reserve().unwrap();
    sweep(
        world,
        ecs,
        DeathContext {
            tick: 100,
            world_time: 200,
            locals,
            joiners: &[],
        },
        &mut batch,
    );
    batch
}
fn units(ecs: &hecs::World, item: Item) -> u32 {
    ecs.query::<&crate::entity::ItemEntity>()
        .iter()
        .filter(|(_, drop)| drop.stack.item == item)
        .map(|(_, drop)| u32::from(drop.stack.count))
        .sum()
}

#[test]
fn mass_death_backlog_keeps_every_drop_and_credit_exactly_once() {
    let (mut world, mut ecs) = (World::new(), hecs::World::new());
    let at = Vec3::new(5.0, 64.0, 5.0);
    for _ in 0..77 {
        dead(&mut ecs, MobType::Cow, at);
    }
    let mut notices = 0;
    for expected in [32, 32, 13, 0] {
        let batch = run(&mut world, &mut ecs, &[(at, false)]);
        assert_eq!(batch.notices().len(), expected);
        assert!(batch.owned_bytes() <= MAX_LOCAL_BYTES);
        for notice in batch.notices() {
            assert!(matches!(
                notice,
                DeathNotice::Killed {
                    credit: KillCredit::Local(0),
                    ..
                }
            ));
        }
        notices += batch.notices().len();
    }
    assert_eq!(notices, 77);
    assert_eq!(ecs.query::<&Health>().iter().count(), 0);
    let expected: u32 =
        crate::mob::drops_for(MobType::Cow, crate::death_drops::death_seed(at, 200))
            .iter()
            .filter(|s| s.item == Item::Material(MaterialId::RawBeef))
            .map(|s| u32::from(s.count))
            .sum();
    assert_eq!(
        units(&ecs, Item::Material(MaterialId::RawBeef)),
        expected * 77
    );
}

#[test]
fn colocated_cargo_spills_each_exact_pack_once() {
    let (mut world, mut ecs) = (World::new(), hecs::World::new());
    for (kind, count) in [(MobType::Donkey, 3), (MobType::Mule, 7)] {
        let entity = dead(&mut ecs, kind, Vec3::ZERO);
        let mut data = crate::horse_ai::HorseData::new();
        let mut pack = crate::chest::ChestData::new();
        pack.slots[0] = Some(ItemStack::new_material(MaterialId::Bread, count));
        data.pack = Some(pack);
        ecs.insert_one(entity, data).unwrap();
    }
    assert_eq!(run(&mut world, &mut ecs, &[]).notices().len(), 2);
    assert!(run(&mut world, &mut ecs, &[]).is_empty());
    assert_eq!(units(&ecs, Item::Material(MaterialId::Bread)), 10);
    assert_eq!(units(&ecs, Item::Block(crate::block::CHEST)), 2);
}

#[test]
fn rescue_and_deliberate_cull_have_separate_exact_outcomes() {
    let (mut world, mut ecs) = (World::new(), hecs::World::new());
    world.set_block(2, 64, 0, crate::block::PET_BED);
    let mut pets = Vec::new();
    for cull in [false, true] {
        let pet = dead(&mut ecs, MobType::Wolf, Vec3::new(0.0, 64.0, 0.0));
        let mut wolf = crate::wolf::WolfData::untamed();
        wolf.ownership.owner_pubkey = "local-player-0".into();
        ecs.insert_one(pet, wolf).unwrap();
        if cull {
            ecs.insert_one(pet, crate::tameable::DeliberateCull { last_hit_tick: 100 })
                .unwrap();
        }
        pets.push(pet);
    }
    let batch = run(&mut world, &mut ecs, &[(Vec3::ZERO, false)]);
    assert!(ecs.contains(pets[0]));
    assert!(!ecs.contains(pets[1]));
    assert_eq!(
        batch
            .notices()
            .iter()
            .filter(|n| matches!(n, DeathNotice::Rescued { .. }))
            .count(),
        1
    );
    assert_eq!(
        batch
            .notices()
            .iter()
            .filter(|n| matches!(
                n,
                DeathNotice::Killed {
                    tamed: true,
                    credit: KillCredit::Local(0),
                    ..
                }
            ))
            .count(),
        1
    );
    assert_eq!(ecs.query::<&crate::entity::ItemEntity>().iter().count(), 0);
    assert!(run(&mut world, &mut ecs, &[]).is_empty());
}

#[test]
fn raid_and_hideout_decrement_once_without_joiner_settlement() {
    for attacker in [
        None,
        Some(Attacker::Departed),
        Some(Attacker::Remote {
            slot: 1,
            generation: 8,
        }),
    ] {
        let (mut world, mut ecs) = (World::new(), hecs::World::new());
        let mut raid = crate::raid::Raid::new_warning(
            1,
            (0, 0),
            [0, 64, 0],
            crate::raid::WaveKind::Small,
            100,
            0,
        );
        raid.mobs_alive = 2;
        world.active_raids.push(raid);
        world.brigand_hideouts.insert(
            (0, 0),
            crate::brigand_hideout_gen::HideoutData {
                anchor_world: [0, 64, 0],
                gx: 0,
                gz: 0,
                population: 2,
                population_target: 2,
                has_berserker: false,
                last_replenish_tick: 0,
                dormant_since: None,
                world_seed: 42,
            },
        );
        for _ in 0..2 {
            let mob = dead(&mut ecs, MobType::Brigand, Vec3::new(1.0, 64.0, 0.0));
            ecs.insert(
                mob,
                (
                    crate::raid::RaidMember { raid_id: 1 },
                    crate::brigand::HomeHideout { anchor: [0, 64, 0] },
                ),
            )
            .unwrap();
            if let Some(a) = attacker {
                ecs.insert_one(mob, LastAttacker(a)).unwrap();
            }
        }
        run(&mut world, &mut ecs, &[(Vec3::new(0.0, 64.0, 0.0), false)]);
        assert_eq!(world.brigand_hideouts[&(0, 0)].population, 0);
        let raid = &world.active_raids[0];
        assert_eq!(raid.mobs_alive, 0);
        assert_eq!(
            raid.killing_blow,
            if attacker.is_none() { Some(0) } else { None }
        );
        assert_eq!(raid.contribution_table.is_empty(), attacker.is_some());
        let after = raid.contribution_table.clone();
        assert!(run(&mut world, &mut ecs, &[]).is_empty());
        assert_eq!(world.active_raids[0].contribution_table, after);
    }
}

#[test]
fn column_cleanup_retains_pending_deaths_for_one_drop_and_credit() {
    let (mut world, mut ecs) = (World::new(), hecs::World::new());
    let at = Vec3::new(4.0, 80.0, 4.0);
    let scattered = dead(&mut ecs, MobType::Cow, at);
    ecs.insert_one(scattered, crate::entity::Scattered).unwrap();
    let night = dead(&mut ecs, MobType::Brigand, at);
    ecs.insert_one(night, crate::entity::NightSpawn).unwrap();
    let live = ecs.spawn((
        MobKind(MobType::Cow),
        Position(at),
        Health::new(20.0),
        crate::entity::Scattered,
    ));
    assert_eq!(crate::entity::despawn_mobs_in_column(&mut ecs, 0, 0), 1);
    assert!(!ecs.contains(live));
    crate::spawning::tick_mob_spawning(&mut ecs, &world, 7500, &[]);
    assert!(
        ecs.contains(scattered) && ecs.contains(night),
        "ordinary culling cannot erase an unsettled death"
    );
    let batch = run(&mut world, &mut ecs, &[(at, false)]);
    assert_eq!(batch.notices().len(), 2);
    assert!(!ecs.contains(scattered) && !ecs.contains(night));
    assert!(units(&ecs, Item::Material(MaterialId::RawBeef)) > 0);
    assert!(run(&mut world, &mut ecs, &[]).is_empty());
}
