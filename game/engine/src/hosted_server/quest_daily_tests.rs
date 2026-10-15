//! First causal daily-cadence assertion observes real connected simulation.
use super::{
    quest_lifecycle_fixture::Room,
    quest_lifecycle_tests::{key, score},
};
use crate::{
    action_identity::InventoryMode,
    quest_wire::{QuestNote, QuestOperation},
};

#[test]
fn d2c3_connected_reputation_does_not_decay_before_a_real_default_world_day() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("daily-cadence", mode, true);
        let secret = [46; 32];
        let negative = room.connect(Some(secret), [47; 32]);
        let offered = room.quest(0, 0, QuestOperation::Talk).view.unwrap();
        let terms = offered.terms.as_ref().unwrap();
        let crate::quest::QuestFlavour::Fetch { item, count } = terms.flavour else {
            panic!("actual natural Blacksmith Fetch")
        };
        room.quest(
            0,
            offered.incarnation,
            QuestOperation::Accept { id: terms.id },
        );
        room.grant(0, vec![crate::item::ItemStack::new_material(item, count)]);
        let completed = room.quest(
            0,
            offered.incarnation,
            QuestOperation::TurnIn { id: terms.id },
        );
        assert_eq!(completed.note, QuestNote::Completed);
        assert_eq!(score(&room.record(0)), terms.reputation);
        assert!(terms.reputation > 1);
        room.kill(negative, crate::mob::MobType::Villager);
        assert_eq!(score(&room.record(negative)), -25);
        room.quiesce();
        let before = [score(&room.record(0)), score(&room.record(negative))];
        let contexts = [room.record(0).context, room.record(negative).context];
        assert_eq!(
            room.host.server.players[room.actors[negative].i()].verified_pubkey,
            Some(key(secret))
        );
        let start_tick = room.host.server.tick_counter;
        let start_phase = room.host.server.world_time;
        let start_total = room.host.server.world.total_ticks;
        assert!(start_tick < 6000);
        // A fresh ordinary owning server starts naturally at phase6000 and step1.
        assert_eq!(start_phase, (6000 + start_tick as u32) % 24000);
        assert!(start_phase < 12000, "no near-rollover ambiguity");
        let mut wraps = 0u32;
        while room.host.server.tick_counter < 6000 {
            let tick = room.host.server.tick_counter;
            let phase = room.host.server.world_time;
            let total = room.host.server.world.total_ticks;
            for actor in &room.actors {
                assert!(actor.active && actor.rejects.is_empty());
                let sp = &room.host.server.players[actor.i()];
                assert!(sp.server_simulated && sp.is_in_world() && !sp.combat.dead);
                assert!(sp.personal_delivery.initialised);
            }
            room.tick();
            assert_eq!(room.host.server.tick_counter, tick + 1);
            assert_eq!(room.host.server.world.total_ticks, total + 1);
            assert_eq!(
                room.host.server.world_time,
                (phase + 1) % 24000,
                "actual default solar step1"
            );
            wraps += u32::from(room.host.server.world_time < phase);
        }
        let boundary_phase = room.host.server.world_time;
        let elapsed = 6000 - start_tick;
        assert_eq!(wraps, 0);
        assert_eq!(boundary_phase - start_phase, elapsed as u32);
        assert_eq!(room.host.server.world.total_ticks - start_total, elapsed);
        assert!(elapsed < 24000);
        let at_boundary = [score(&room.record(0)), score(&room.record(negative))];
        assert_eq!(room.actors[0].slot.reputation.score((2, 3)), at_boundary[0]);
        assert_eq!(
            room.actors[negative].slot.reputation.score((2, 3)),
            at_boundary[1]
        );
        assert_eq!(
            [room.record(0).context, room.record(negative).context],
            contexts
        );
        room.quiesce(); // Return actual received current tuples through normal ticks.
        assert_eq!(
            [score(&room.record(0)), score(&room.record(negative))],
            at_boundary
        );
        eprintln!(
            "daily-own mode={mode:?} start_tick={start_tick} start_phase={start_phase} boundary_tick=6000 boundary_phase={boundary_phase} elapsed={elapsed} wraps={wraps} before={before:?} at_boundary={at_boundary:?} settled_tick={}",
            room.host.server.tick_counter
        );
        // FIRST OWN: prerequisites above prove both signs, real connected owners,
        // actual physical/personal installation and no natural day crossing.
        assert_eq!(
            at_boundary, before,
            "reputation must not decay before a real default world day"
        );
        // The real owner accumulates solar units from its actual first tick.
        while room.host.server.tick_counter < 23_999 {
            let tick = room.host.server.tick_counter;
            let phase = room.host.server.world_time;
            let total = room.host.server.world.total_ticks;
            room.tick();
            assert_eq!(room.host.server.tick_counter, tick + 1);
            assert_eq!(room.host.server.world.total_ticks, total + 1);
            assert_eq!(room.host.server.world_time, (phase + 1) % 24_000);
            assert_eq!(
                [score(&room.record(0)), score(&room.record(negative))],
                before
            );
        }
        assert_eq!(room.host.server.reputation_day.remainder(), 23_999);
        let revisions = [room.record(0).revision, room.record(negative).revision];
        // Real lethal combat damage keeps a connected dead holder eligible.
        assert!(
            room.host.server.players[room.actors[negative].i()]
                .combat
                .take_damage_from(100.0, crate::survival::DamageCause::Fall)
        );
        room.tick();
        assert!(
            room.host.server.players[room.actors[negative].i()]
                .combat
                .dead
        );
        assert!(room.host.server.players[room.actors[negative].i()].is_in_world());
        let first_day = [before[0] - 1, before[1] + 1];
        assert_eq!(
            [score(&room.record(0)), score(&room.record(negative))],
            first_day
        );
        assert_eq!(
            [room.record(0).revision, room.record(negative).revision],
            [revisions[0] + 1, revisions[1] + 1]
        );
        assert_eq!(room.host.server.reputation_day.remainder(), 0);
        room.quiesce();
        assert_eq!(room.actors[0].slot.reputation.score((2, 3)), first_day[0]);
        assert_eq!(
            room.actors[negative].slot.reputation.score((2, 3)),
            first_day[1]
        );
        while room.host.server.tick_counter < 48_000 {
            room.tick();
        }
        assert_eq!(
            [score(&room.record(0)), score(&room.record(negative))],
            [before[0] - 2, before[1] + 2]
        );
        room.quiesce();
        assert_eq!(
            [room.record(0).context, room.record(negative).context],
            contexts
        );
    }
}

#[test]
fn d2c3_offline_reconnect_and_server_restart_preserve_scores_without_day_catch_up() {
    let mut room = Room::new("day-offline-restart", InventoryMode::Strict, true);
    let secret = super::c3d_tests::KEY;
    let giver = room.quest(0, 0, QuestOperation::Talk).view.unwrap();
    room.quest(0, giver.incarnation, QuestOperation::Close);
    room.kill(0, crate::mob::MobType::Villager);
    assert_eq!(score(&room.record(0)), -25);
    room.ticks(100);
    let owner = room.host.server.reputation_day.owner();
    assert!(room.host.server.reputation_day.remainder() > 0);
    room.leave(0);
    let before = room.saved(key(secret)).encode().unwrap();
    room.ticks(24_000);
    assert_eq!(room.saved(key(secret)).encode().unwrap(), before);
    let rejoined = room.connect(Some(secret), [53; 32]);
    assert_eq!(
        score(&room.record(rejoined)),
        -25,
        "no offline day catch-up"
    );
    assert_eq!(room.actors[rejoined].slot.reputation.score((2, 3)), -25);
    room.leave(rejoined);
    room.host.server.try_save().unwrap();
    let stored = room.saved(key(secret)).personal.unwrap().record().unwrap();
    let mut room = room.restart();
    assert_ne!(room.host.server.reputation_day.owner(), owner);
    assert_eq!(
        room.host.server.reputation_day.remainder(),
        1,
        "only the actual restart tick accrued"
    );
    let actor = room.connect(Some(secret), [54; 32]);
    assert_eq!(score(&room.record(actor)), score(&stored));
    let context = room.record(actor).context;
    while room.host.server.tick_counter < 23_999 {
        room.tick();
        assert_eq!(score(&room.record(actor)), score(&stored));
    }
    room.tick();
    assert_eq!(score(&room.record(actor)), -24);
    room.quiesce();
    assert_eq!(room.record(actor).context, context);
    assert_eq!(room.actors[actor].slot.reputation.score((2, 3)), -24);
}

#[test]
fn d2c3_real_signed_admission_and_departure_at_a_day_boundary_use_current_membership() {
    use crate::transport::ClientTransport as _;
    let mut room = Room::new("day-current-membership", InventoryMode::Strict, true);
    let departing_secret = super::c3d_tests::KEY;
    room.kill(0, crate::mob::MobType::Villager);
    let arriving_secret = [55; 32];
    let old = room.connect(Some(arriving_secret), [56; 32]);
    room.kill(old, crate::mob::MobType::Villager);
    assert_eq!(score(&room.record(old)), -25);
    room.leave(old);
    while room.host.server.tick_counter < 23_998 {
        room.tick();
    }
    room.actors[0]
        .client
        .send_to_server(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::Disconnect,
            &(),
        ));
    room.tick();
    assert_eq!(room.host.server.tick_counter, 23_999);
    assert!(!room.actors[0].active && room.host.disconnected[room.actors[0].i()]);
    let departure = room.saved(key(departing_secret)).encode().unwrap();
    assert_eq!(
        score(
            &room
                .saved(key(departing_secret))
                .personal
                .unwrap()
                .record()
                .unwrap()
        ),
        -25
    );
    let arrived = room.connect(Some(arriving_secret), [57; 32]);
    assert_eq!(
        room.actors[arrived].initial_carriers,
        [(1, "sync"), (2, "personal")]
    );
    assert!(
        room.actors[arrived].applied > 2,
        "the actual day revision follows genuine initial carriers"
    );
    assert_eq!(
        room.host.server.players[room.actors[arrived].i()].verified_pubkey,
        Some(key(arriving_secret))
    );
    assert_eq!(
        score(&room.record(arrived)),
        -24,
        "genuine boundary admission is connected at application"
    );
    assert_eq!(room.actors[arrived].slot.reputation.score((2, 3)), -24);
    assert_eq!(
        room.saved(key(departing_secret)).encode().unwrap(),
        departure,
        "departure before application remains offline"
    );
}
