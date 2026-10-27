//! D4 slice 3 — the mob simulation runs once, in the server, on every kind of
//! world it runs (`sim_mobs`): a lending host's (its host client runs none of
//! it) and a dedicated server's, which now simulates the animals' life a
//! joiner's feeding, taming, Leads and pet commands rely on.

use super::*;
use crate::sim_lend::{SimSide, SimSystem};

/// The mob systems the server owns on a lent world since D4 slice 3, all of
/// them every-tick systems.
const MOB_SYSTEMS: [SimSystem; 6] = [
    SimSystem::MobAi,
    SimSystem::EntityPhysics,
    SimSystem::Products,
    SimSystem::Breeding,
    SimSystem::Species,
    SimSystem::Tethers,
];

impl Rig {
    fn tally(&self) -> crate::sim_lend::SimTally {
        self.host.as_ref().map_or(self.hs.server.world.sim_tally, |h| h.world.sim_tally)
    }

    fn pos(&self, e: hecs::Entity) -> Vec3 {
        self.ecs().get::<&crate::entity::Position>(e).unwrap().0
    }

    fn eggs(&self) -> u32 {
        self.ecs()
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter(|(_, it)| it.stack.item == mat(MaterialId::Egg))
            .map(|(_, it)| u32::from(it.stack.count))
            .sum()
    }
}

/// Saved local-seat owners never acquire a joiner's body with the same slot
/// number, whether the server owns the world or borrows a host's world.
#[test]
fn local_pet_keys_never_resolve_to_joiner_slots() {
    for mut rig in [Rig::new("local-pet-owner-dedicated", 1), Rig::new_lent("local-pet-owner-lent", 1)] {
        let slot = rig.joiners[0].slot;
        rig.sign_in(0);
        let key = crate::tameable::local_owner_key(slot);
        let mut pets = Vec::new();
        for kind in [MobType::Wolf, MobType::Cat, MobType::Parrot, MobType::Nostrich] {
            let (pet, _) = rig.spawn(kind, Vec3::new(6.0, 0.0, 0.0));
            pets.push((kind, pet));
        }
        for &(kind, pet) in &pets {
            rig.place(pet, Vec3::new(6.0, 0.0, 0.0));
            rig.ecs_mut().get::<&mut crate::mob_ai::MobAi>(pet).unwrap().state =
                crate::mob_ai::AiState::Idle { timer: 100 };
            match kind {
                MobType::Wolf => {
                    rig.ecs_mut().get::<&mut crate::wolf::WolfData>(pet).unwrap().ownership.owner_pubkey = key.clone();
                }
                MobType::Nostrich => {
                    let mut data = rig.ecs_mut().get::<&mut crate::nostrich::NostrichData>(pet).unwrap();
                    data.ownership.owner_pubkey = key.clone();
                    data.state = crate::nostrich::NostrichAiState::Follow;
                }
                _ => {
                    let mut data = rig.ecs_mut().get::<&mut crate::companion::CompanionData>(pet).unwrap();
                    data.ownership.owner_pubkey = key.clone();
                    data.state = if kind == MobType::Parrot {
                        crate::companion::CompanionState::Perch
                    } else {
                        crate::companion::CompanionState::Follow
                    };
                }
            }
        }
        rig.tick(1);
        for (kind, pet) in pets {
            let pos = rig.pos(pet);
            assert!((pos.x - rig.at.x - 6.0).abs() < 0.01, "{kind:?} must not follow or perch on joiner slot {slot}: {pos}");
            let velocity = rig.ecs().get::<&crate::entity::Velocity>(pet).unwrap().0;
            assert_eq!((velocity.x, velocity.z), (0.0, 0.0), "{kind:?}: absent owner holds still");
        }
    }
}

/// Hostile species overrides must use the same protected targets as generic
/// mob AI; raw owner/Lead bodies remain available to the other species.
#[test]
fn spawn_protection_blocks_shark_bites_and_hostile_species_hunts() {
    for mut rig in [Rig::new("species-shield-dedicated", 1), Rig::new_lent("species-shield-lent", 1)] {
        rig.hs.server.difficulty = crate::survival::Difficulty::Normal;
        rig.hs.server.world_time = 0;
        let slot = rig.joiners[0].slot;
        if slot > 0 {
            rig.hs.server.players[0].connected = false;
        }
        let at = rig.at;
        for x in 40..=42 {
            for z in 40..=41 {
                rig.world_mut().set_block(x, 80, z, block::WATER);
                rig.world_mut().set_block(x, 81, z, block::WATER);
            }
        }
        let shark = crate::entity::spawn_mob(rig.ecs_mut(), MobType::Shark, at + Vec3::X);
        let hyena = crate::entity::spawn_mob(rig.ecs_mut(), MobType::Hyena, at + 4.0 * Vec3::X);
        let bear = crate::entity::spawn_mob(rig.ecs_mut(), MobType::Bear, at + 6.0 * Vec3::X);
        rig.ecs_mut().get::<&mut crate::bear_ai::BearData>(bear).unwrap().state =
            crate::bear_ai::BearAiState::Aggro { attacker_pidx: slot, ticks_remaining: 100 };
        for mob in [shark, hyena, bear] {
            rig.ecs_mut().get::<&mut crate::mob_ai::MobAi>(mob).unwrap().state =
                crate::mob_ai::AiState::Idle { timer: 100 };
        }
        crate::spawn_protection::arm(&mut rig.hs.server.players[slot]);
        rig.tick(1);
        assert_eq!(rig.hs.server.players[slot].combat.health, 20.0, "a protected swimmer takes no shark bite");
        for mob in [hyena, bear] {
            let velocity = rig.ecs().get::<&crate::entity::Velocity>(mob).unwrap().0;
            assert!(velocity.x >= 0.0, "hostile species must not hunt the protected body to their left: {velocity}");
        }
        crate::spawn_protection::disarm(&mut rig.hs.server.players[slot]);
        rig.hs.server.players[slot].player.pos = at;
        rig.hs.server.players[slot].player.velocity = Vec3::ZERO;
        rig.place(shark, Vec3::X);
        rig.tick(1);
        assert!(rig.hs.server.players[slot].combat.health < 20.0, "after protection the shark bites");
        for mob in [hyena, bear] {
            let velocity = rig.ecs().get::<&crate::entity::Velocity>(mob).unwrap().0;
            assert!(velocity.x < 0.0, "after protection hostile species hunt the body to their left: {velocity}");
        }
    }
}

/// One logical tick runs each mob phase exactly once on the world, lent or
/// owned; on a lent world the host client's side runs none of them, so the
/// server's run is the only one, and a second would be the tripwire's fault.
#[test]
fn one_mob_pass_per_logical_tick_on_a_lent_world_and_a_dedicated_server() {
    for mut rig in [Rig::new_lent("mob-pass-lent", 1), Rig::new("mob-pass-dedicated", 1)] {
        let lent = rig.host.is_some();
        let (cow, _) = rig.spawn(MobType::Cow, Vec3::new(0.0, 3.0, 2.0));
        let before = rig.tally();
        let y = rig.pos(cow).y;
        rig.tick(1);
        let after = rig.tally();
        for s in MOB_SYSTEMS {
            assert_eq!(after.get(s) - before.get(s), 1, "lent={lent}: {s:?} once");
            assert!(s.every_tick(), "{s:?}");
            if lent {
                assert_eq!(s.lent_owner(), SimSide::Server, "{s:?}");
                assert!(!s.runs_on(SimSide::HostClient, true), "the host client runs no {s:?}");
            }
        }
        assert!(rig.pos(cow).y < y, "lent={lent}: the server's physics moved the mob (it falls)");
        let mut doubled = after;
        doubled.bump(SimSystem::Species);
        assert!(doubled.one_tick_faults(&before).contains(&(SimSystem::Species, 2)), "a second pass is a fault");
    }
}

/// A Lead holds an animal: a joiner who leashed a cow and walks off pulls it
/// along, on a lent world and on a dedicated server (the server's Lead pass).
#[test]
fn a_lead_holds_an_animal_on_a_lent_world_and_a_dedicated_server() {
    for mut rig in [Rig::new_lent("lead-hold-lent", 1), Rig::new("lead-hold-dedicated", 1)] {
        let lent = rig.host.is_some();
        let slot = rig.joiners[0].slot;
        let (cow, id) = rig.spawn(MobType::Cow, Vec3::new(0.0, 0.0, 1.5));
        rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
        let s = rig.joiners[0].interact(id, InteractKind::LeadAttach, Some(&mat(MaterialId::Lead)));
        rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
        assert!(rig.joiners[0].inbox.outcome(s).accepted, "lent={lent}");
        // The joiner steps 6 blocks off along -x (past the Lead's slack).
        let to = rig.at + Vec3::new(-6.0, 0.0, 0.0);
        rig.hs.server.players[slot].player.pos = to;
        rig.place(cow, Vec3::new(0.0, 0.0, 1.5));
        let start = (rig.pos(cow) - to).length();
        for _ in 0..40 {
            rig.hs.server.players[slot].player.pos = to;
            rig.hs.server.players[slot].player.velocity = Vec3::ZERO;
            rig.tick(1);
        }
        let end = (rig.pos(cow) - to).length();
        assert!(rig.ecs().get::<&crate::tether::Tethered>(cow).is_ok(), "lent={lent}: still on the Lead");
        assert!(
            end < start - 2.0 && end <= crate::tether::TETHER_SLACK_DISTANCE + 1.0,
            "lent={lent}: pulled to the joiner ({start} -> {end})"
        );
    }
}

/// D4 slice 3 — a dedicated server simulates the animals' life, so a
/// joiner's taming, Lead and pet command are served there now (no
/// "not on this server").
#[test]
fn a_joiner_tames_leads_and_commands_a_pet_on_a_dedicated_server() {
    let mut rig = Rig::new("life-dedicated-acts", 1);
    let key = rig.sign_in(0);
    let (cat, cat_id) = rig.spawn(MobType::Cat, Vec3::new(0.0, 0.0, 1.5));
    rig.place(cat, Vec3::new(0.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(cat_id, InteractKind::Tame, Some(&mat(MaterialId::CatTreat)));
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted, "{out:?}");
    assert_eq!(note(out), InteractNote::Tamed);
    assert_eq!(crate::tameable::pet_owner_of(rig.ecs(), cat), Some(key.clone()));

    let (wolf, wolf_id) = rig.spawn(MobType::Wolf, Vec3::new(1.0, 0.0, 1.5));
    rig.ecs_mut().get::<&mut crate::wolf::WolfData>(wolf).unwrap().ownership.owner_pubkey = key;
    rig.place(wolf, Vec3::new(1.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(wolf_id, InteractKind::SitToggle, None);
    rig.tick(crate::hosted_server::INTERACT_COOLDOWN_TICKS);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted, "{out:?}");
    assert_eq!(note(out), InteractNote::Sat);
    assert_eq!(rig.ecs().get::<&crate::wolf::WolfData>(wolf).unwrap().state, crate::wolf::WolfAiState::Sit);

    let (cow, cow_id) = rig.spawn(MobType::Cow, Vec3::new(-1.0, 0.0, 1.5));
    rig.place(cow, Vec3::new(-1.0, 0.0, 1.5));
    let s = rig.joiners[0].interact(cow_id, InteractKind::LeadAttach, Some(&mat(MaterialId::Lead)));
    rig.tick(1);
    let out = rig.joiners[0].inbox.outcome(s);
    assert!(out.accepted, "{out:?}");
    assert_ne!(note(out), InteractNote::NotOnThisServer);
    assert!(rig.ecs().get::<&crate::tether::Tethered>(cow).is_ok());
}

/// Product timers: a chicken lays one egg per cadence on a lent world — the
/// server's product phase alone, so never a second egg from a second sim.
#[test]
fn a_chicken_lays_one_egg_per_cadence_on_a_lent_world() {
    let mut rig = Rig::new_lent("egg", 1);
    // Far enough from the joiner that it never picks an egg up.
    // Only this hen lays: the world's other layers and their eggs go first.
    let others: Vec<hecs::Entity> = rig
        .ecs()
        .query::<(&crate::entity::MobKind,)>()
        .iter()
        .filter(|(_, (k,))| matches!(k.0, MobType::Chicken | MobType::Nostrich))
        .map(|(e, _)| e)
        .chain(rig.ecs().query::<&crate::entity::ItemEntity>().iter().map(|(e, _)| e))
        .collect();
    for e in others {
        let _ = rig.ecs_mut().despawn(e);
    }
    let (hen, _) = rig.spawn(MobType::Chicken, Vec3::new(6.0, 0.0, 6.0));
    // A fresh chicken lays at once: the spawn tick's egg.
    assert_eq!(rig.eggs(), 1, "one egg on the first product tick");
    rig.tick(20);
    assert_eq!(rig.eggs(), 1, "nothing more before the cadence");
    // The clock jumps to one cadence after that first egg, less a few ticks.
    let laid = rig.ecs().get::<&crate::animal_products::AnimalProductState>(hen).unwrap().last_action_tick;
    let laid = laid.expect("the first egg was recorded");
    rig.hs.server.tick_counter = laid + crate::animal_products::EGG_LAY_INTERVAL_TICKS - 3;
    rig.tick(10);
    assert_eq!(rig.eggs(), 2, "one more egg when the cadence comes round, once");
}

/// A dedicated server's restart keeps a joiner's tamed pet with its owner,
/// and the restarted server's species AI walks the pet to that owner when
/// they are back.
#[test]
fn a_dedicated_restart_keeps_a_tamed_pet_with_its_owner() {
    let mut rig = Rig::new("pet-restart", 1);
    let key = rig.sign_in(0);
    let (wolf, _) = rig.spawn(MobType::Wolf, Vec3::new(2.0, 0.0, 2.0));
    rig.ecs_mut().get::<&mut crate::wolf::WolfData>(wolf).unwrap().ownership.owner_pubkey = key.clone();
    rig.tick(1);
    // The test's joiner signed in by hand after joining as a guest: back to a
    // guest for the save (a guest's progression is never written).
    let slot = rig.joiners[0].slot;
    rig.hs.server.players[slot].verified_pubkey = None;
    rig.hs.server.try_save().expect("the dedicated server saves");
    let name = rig.hs.server.world_name.clone();
    drop(rig);

    let hs = HostedServer::start(0, name, 42, 0, RemoteTransport::WebSocket { port: 0 })
        .expect("the dedicated server starts again on its saved world");
    let owned: Vec<hecs::Entity> = hs
        .server
        .ecs
        .query::<&crate::wolf::WolfData>()
        .iter()
        .filter(|(_, d)| d.owner_pubkey() == key)
        .map(|(e, _)| e)
        .collect();
    assert_eq!(owned.len(), 1, "the tamed wolf is back, still its owner's");
    let wolf = owned[0];
    // `Rig::seat` without its mob purge (which would take the pet too).
    let mut rig = Rig { hs, host: None, joiners: Vec::new(), at: Vec3::ZERO };
    rig.join("J0");
    rig.hs.server.column_streamer = None;
    rig.hs.server.column_refill_per_tick = 0;
    rig.hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    assert!(rig.ecs().get::<&crate::wolf::WolfData>(wolf).is_ok(), "the pet is still in the world");
    assert_eq!(rig.sign_in(0), key, "its owner joins again");
    rig.place(wolf, Vec3::new(6.0, 0.0, 6.0));
    let start = (rig.pos(wolf) - rig.at).length();
    for _ in 0..60 {
        let (slot, at) = (rig.joiners[0].slot, rig.at);
        rig.hs.server.players[slot].player.pos = at;
        rig.tick(1);
    }
    let end = (rig.pos(wolf) - rig.at).length();
    assert!(end < start - 2.0, "the restarted server's species AI walks the pet to its owner ({start} -> {end})");
}

/// Spawn protection on a lending host (D4 slice 3: its mob AI is the
/// server's): a protected joiner is no target and takes no blast; once the
/// protection ends it is both.
#[test]
fn a_protected_joiner_on_a_lending_host_is_no_target_and_takes_no_blast() {
    let mut rig = Rig::new_lent("protected", 1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Normal;
    let slot = rig.joiners[0].slot;
    // The host's own player (slot 0) far off: the joiner is the only body
    // the mob could chase.
    rig.hs.server.players[0].player.pos = rig.at + Vec3::new(500.0, 0.0, 500.0);
    crate::spawn_protection::arm(&mut rig.hs.server.players[slot]);
    let (mob, _) = rig.spawn(MobType::Berserker, Vec3::new(4.0, 0.0, 0.0));
    let hold = |rig: &mut Rig| {
        rig.place(mob, Vec3::new(4.0, 0.0, 0.0));
        rig.ecs_mut().get::<&mut crate::mob_ai::MobAi>(mob).unwrap().state =
            crate::mob_ai::AiState::Idle { timer: 0 };
    };
    let chasing = |rig: &Rig| {
        matches!(rig.ecs().get::<&crate::mob_ai::MobAi>(mob).unwrap().state, crate::mob_ai::AiState::Chase)
    };
    for _ in 0..5 {
        hold(&mut rig);
        rig.tick(1);
        assert!(crate::spawn_protection::shielded(&rig.hs.server.players[slot]));
        assert!(!chasing(&rig), "a protected joiner is no target on a lending host");
    }
    // A keg goes off beside it.
    let at = rig.at;
    let keg = (at.x.floor() as i32 + 1, at.y as i32, at.z.floor() as i32);
    let blast = |rig: &mut Rig| {
        let registry = crate::block::BlockRegistry::new();
        let world = std::mem::replace(rig.world_mut(), crate::world::World::new());
        let landed = crate::explosion::apply_joiner_blast_damage(
            &mut rig.hs.server.players,
            &world,
            &registry,
            keg,
            crate::play_mode::PlayMode::Survival,
        );
        *rig.world_mut() = world;
        landed
    };
    assert!(blast(&mut rig).is_empty(), "no blast lands on a protected joiner");
    assert_eq!(rig.hs.server.players[slot].combat.health, 20.0);

    crate::spawn_protection::disarm(&mut rig.hs.server.players[slot]);
    assert_eq!(blast(&mut rig), vec![slot], "after the window the blast lands");
    assert!(rig.hs.server.players[slot].combat.health < 20.0);
    hold(&mut rig);
    rig.tick(1);
    assert!(chasing(&rig), "and the joiner is a target again");
}

/// On a lent world a hostile beside a LOCAL body (the host's player) is found
/// by the server after its physics and handed to the host client through the
/// outbox, to land on its own `PlayerSlot`; the server lands nothing on the
/// local slot's copy itself.
#[test]
fn a_lent_world_hands_a_local_bodys_hostile_contacts_to_its_client() {
    let mut rig = Rig::new_lent("local-contact", 1);
    rig.hs.server.difficulty = crate::survival::Difficulty::Normal;
    assert!(!rig.hs.server.players[0].server_simulated, "slot 0 is the host's own player");
    let local = rig.at + Vec3::new(-4.0, 0.0, -4.0);
    let (mob, _) = rig.spawn(MobType::Berserker, Vec3::new(-4.0, 0.0, -3.5));
    rig.host.as_mut().unwrap().events.clear();
    rig.hs.server.players[0].player.pos = local;
    rig.ecs_mut().get::<&mut crate::entity::Position>(mob).unwrap().0 = local + Vec3::new(0.0, 0.0, 0.5);
    rig.tick(1);
    let health = rig.hs.server.players[0].combat.health;
    let contacts: Vec<&crate::sim_mobs::SimEvent> = rig
        .host
        .as_ref()
        .unwrap()
        .events
        .iter()
        .filter(|e| matches!(e, crate::sim_mobs::SimEvent::HostileContacts { slot: 0, .. }))
        .collect();
    assert_eq!(contacts.len(), 1, "one contact report for the tick: {contacts:?}");
    let crate::sim_mobs::SimEvent::HostileContacts { contacts, .. } = contacts[0] else { unreachable!() };
    assert!(contacts.iter().any(|c| c.mob == mob && c.kind == MobType::Berserker));
    assert_eq!(health, 20.0, "the server lands nothing on the local slot's copy");
}
