//! Keep slice 3: a death waiting to be swept is state, so it is in the checkpoint
//! and the restored world's next sweep does what the uninterrupted one's does
//! (Spec 01 §4.1.5). A save still waits for the sweep, so these persist first, then
//! the mob dies, and the corpses' record is written with what binds it.

use glam::Vec3;

use super::authority_checkpoint::{fixture, persist_and_checkpoint, restore, SEED};
use super::twin_run::{run_to, twin};
use crate::combat::{Attacker, Health, LastAttacker};
use crate::entity::{spawn_mob, MobKind, Position};
use crate::mob::MobType;
use crate::save::WorldsRootGuard;
use crate::server::GameServer;
use crate::test_harness::TestHost;

const AT: Vec3 = Vec3::new(4.5, 101.0, 20.5);

fn kill(h: &mut TestHost, mob: hecs::Entity) {
    h.server.ecs.get::<&mut Health>(mob).unwrap().current = 0.0;
}

fn corpses(s: &GameServer) -> Vec<(MobType, Vec3)> {
    let mut query = s.ecs.query::<(&Health, &MobKind, &Position)>();
    let mut out: Vec<_> = query.iter().filter(|(_, (h, ..))| h.is_dead()).map(|(_, (_, k, p))| (k.0, p.0)).collect();
    out.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    out
}

fn items(s: &GameServer) -> usize {
    s.ecs.query::<&crate::entity::ItemEntity>().iter().count()
}

/// What `checkpoint` used to refuse: it is written, and comes back byte for byte.
#[test]
fn a_pending_death_is_in_the_bytes_and_comes_back() {
    let _root = WorldsRootGuard::new("keep-s3-death-bytes");
    let mut h = fixture("keep-s3-death-bytes", 1);
    run_to(&mut h, 5);
    let clean = persist_and_checkpoint(&mut h);
    let cow = spawn_mob(&mut h.server.ecs, MobType::Cow, AT);
    kill(&mut h, cow);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().expect("a pending death isn't a refusal");
    assert_ne!(bytes, clean, "the death is in the bytes");
    h.server.state_hash().expect("nor is its hash");

    let back = restore(&h, &bytes).expect("restores");
    assert_eq!(corpses(&back), vec![(MobType::Cow, AT)]);
    assert!(crate::sim_death::has_pending(&back.ecs));
    assert_eq!(back.checkpoint().expect("again"), bytes, "exactly");
}

/// The next sweep in the restored world credits the same joiner and drops the
/// same loot.
#[test]
fn the_next_sweep_in_a_restored_world_does_the_same() {
    let _root = WorldsRootGuard::new("keep-s3-death-sweep");
    let mut h = fixture("keep-s3-death-sweep", 1);
    run_to(&mut h, 5);
    persist_and_checkpoint(&mut h);
    let slot = h.server.players.iter().position(|sp| sp.server_simulated).expect("a joiner");
    let generation = h.server.players[slot].attach_gen;
    let cow = spawn_mob(&mut h.server.ecs, MobType::Cow, AT);
    let _ = h.server.ecs.insert_one(cow, LastAttacker(Attacker::Remote { slot, generation }));
    kill(&mut h, cow);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().unwrap();
    let mut back = restore(&h, &bytes).expect("restores");

    // Mob AI rolls hash the entity id, which a restore changes (the restoration
    // ledger), and loot follows the place: this corpse makes no AI rolls in either.
    // What already lay on the ground isn't carried, and new loot merges with it.
    let still = |s: &mut GameServer| {
        let dead: Vec<_> = s.ecs.query::<&Health>().iter().filter(|(_, h)| h.is_dead()).map(|(e, _)| e).collect();
        for corpse in dead {
            let _ = s.ecs.remove_one::<crate::mob_ai::MobAi>(corpse);
        }
        let lying: Vec<_> = s.ecs.query::<&crate::entity::ItemEntity>().iter().map(|(e, _)| e).collect();
        for item in lying {
            let _ = s.ecs.despawn(item);
        }
    };
    still(&mut h.server);
    still(&mut back);
    h.server.tick();
    back.tick();
    assert!(!crate::sim_death::has_pending(&h.server.ecs) && !crate::sim_death::has_pending(&back.ecs), "swept in both");
    assert_eq!(h.server.pending_kill_events.len(), 1, "the uninterrupted world credits it");
    assert_eq!(back.pending_kill_events.len(), 1, "the restored one too");
    assert_eq!(back.pending_kill_events[0].0, slot);
    assert_eq!(back.players[slot].progression_credits, h.server.players[slot].progression_credits);
    let loot = |s: &GameServer| {
        let mut query = s.ecs.query::<(&crate::entity::ItemEntity, &Position)>();
        let near = query.iter().filter(|(_, (_, p))| (p.0.x - AT.x).abs() < 4.0 && (p.0.z - AT.z).abs() < 4.0);
        let mut totals = std::collections::BTreeMap::new();
        for (_, (item, _)) in near {
            *totals.entry(format!("{:?}", item.stack.item)).or_insert(0u32) += u32::from(item.stack.count);
        }
        totals
    };
    assert!(!loot(&back).is_empty(), "loot, of {} on the ground", items(&back));
    assert_eq!(loot(&back), loot(&h.server), "the same loot");
}

/// A tamed mob's saved row, taken alive at the last persist, doesn't come back
/// beside its own corpse.
#[test]
fn a_dead_pet_does_not_come_back_alive_beside_its_corpse() {
    let _root = WorldsRootGuard::new("keep-s3-death-pet");
    let mut h = fixture("keep-s3-death-pet", 1);
    run_to(&mut h, 5);
    let wolf = spawn_mob(&mut h.server.ecs, MobType::Wolf, AT);
    let mut other_wolf_data = None;
    {
        let mut data = h.server.ecs.get::<&mut crate::wolf::WolfData>(wolf).unwrap();
        data.ownership.owner_pubkey = "npub1owner".to_string();
        other_wolf_data.get_or_insert((*data).clone());
    }
    assert!(h.server.ecs.get::<&crate::wolf::WolfData>(wolf).unwrap().is_tamed());
    let friend = spawn_mob(&mut h.server.ecs, MobType::Wolf, AT + Vec3::new(30.0, 0.0, 0.0));
    *h.server.ecs.get::<&mut crate::wolf::WolfData>(friend).unwrap() = other_wolf_data.unwrap();
    persist_and_checkpoint(&mut h);
    kill(&mut h, wolf);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().unwrap();

    let mut back = restore(&h, &bytes).expect("restores");
    let tamed = |s: &GameServer| {
        let mut q = s.ecs.query::<(&Health, &crate::wolf::WolfData)>();
        let v: Vec<_> = q.iter().filter(|(_, (_, d))| d.is_tamed()).map(|(_, (h, _))| h.is_dead()).collect();
        (v.iter().filter(|d| **d).count(), v.iter().filter(|d| !**d).count())
    };
    assert_eq!(tamed(&back), (1, 1), "one corpse and the wolf that didn't die");
    assert_eq!(back.checkpoint().expect("again"), bytes);
    back.tick();
    assert_eq!(tamed(&back), (0, 1), "the sweep took the corpse");
}

/// A kept donkey's cargo is not lost with it: the restored corpse carries its
/// pack, and the sweep spills it.
#[test]
fn a_dead_pack_animals_cargo_spills_in_the_restored_world() {
    let _root = WorldsRootGuard::new("keep-s3-death-pack");
    let mut h = fixture("keep-s3-death-pack", 1);
    run_to(&mut h, 5);
    persist_and_checkpoint(&mut h);
    let donkey = spawn_mob(&mut h.server.ecs, MobType::Donkey, AT);
    {
        let mut data = h.server.ecs.get::<&mut crate::horse_ai::HorseData>(donkey).unwrap();
        data.kept_by = Some(0);
        let mut pack = crate::chest::ChestData::new();
        pack.slots[3] = Some(crate::item::ItemStack::new_block(crate::block::STONE, 9));
        data.pack = Some(pack);
    }
    kill(&mut h, donkey);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().unwrap();
    let mut back = restore(&h, &bytes).expect("restores");
    assert_eq!(back.checkpoint().expect("again"), bytes);
    let before = items(&back);
    back.tick();
    assert!(items(&back) >= before + 2, "the stack and the chest that held it");
}

/// What the sweep reads off a corpse besides its kind and place.
#[test]
fn a_corpses_hideout_raid_and_rescue_marks_come_back() {
    let _root = WorldsRootGuard::new("keep-s3-death-marks");
    let mut h = twin(SEED, "keep-s3-death-marks", 0, 2);
    run_to(&mut h, 2);
    h.server.persist_authority().unwrap();
    let raider = spawn_mob(&mut h.server.ecs, MobType::Cow, AT);
    let _ = h.server.ecs.insert(raider, (
        crate::brigand::HomeHideout { anchor: [1, 2, 3] },
        crate::raid::RaidMember { raid_id: 77 },
        crate::tameable::DeliberateCull { last_hit_tick: 123 },
        LastAttacker(Attacker::Departed),
    ));
    h.server.ecs.get::<&mut Health>(raider).unwrap().rescue_eligible = Some(false);
    kill(&mut h, raider);
    h.server.write_bound_records().expect("the records write");
    let bytes = h.server.checkpoint().unwrap();
    let back = restore(&h, &bytes).expect("restores");
    let mut query = back.ecs.query::<&Health>();
    let (corpse, health) = query.iter().find(|(_, h)| h.is_dead()).expect("the corpse");
    assert_eq!(health.rescue_eligible, Some(false));
    assert_eq!(back.ecs.get::<&crate::brigand::HomeHideout>(corpse).unwrap().anchor, [1, 2, 3]);
    assert_eq!(back.ecs.get::<&crate::raid::RaidMember>(corpse).unwrap().raid_id, 77);
    assert_eq!(back.ecs.get::<&crate::tameable::DeliberateCull>(corpse).unwrap().last_hit_tick, 123);
    assert!(matches!(back.ecs.get::<&LastAttacker>(corpse).unwrap().0, Attacker::Departed));
}

/// One state has one form, whatever order the mobs came in.
#[test]
fn corpses_are_written_in_one_order() {
    let _root = WorldsRootGuard::new("keep-s3-death-order");
    let mut a = twin(SEED, "keep-s3-death-order", 0, 2);
    let (x, y) = (AT, AT + Vec3::new(1.0, 0.0, 0.0));
    let first = spawn_mob(&mut a.server.ecs, MobType::Cow, x);
    let second = spawn_mob(&mut a.server.ecs, MobType::Pig, y);
    kill(&mut a, first);
    kill(&mut a, second);
    let one = a.server.checkpoint().unwrap();
    let mut q = a.server.ecs.query::<&Health>();
    let dead: Vec<_> = q.iter().filter(|(_, h)| h.is_dead()).map(|(e, _)| e).collect();
    drop(q);
    for entity in dead {
        a.server.ecs.despawn(entity).unwrap();
    }
    let second = spawn_mob(&mut a.server.ecs, MobType::Pig, y);
    let first = spawn_mob(&mut a.server.ecs, MobType::Cow, x);
    kill(&mut a, second);
    kill(&mut a, first);
    assert_eq!(a.server.checkpoint().unwrap(), one);
}
