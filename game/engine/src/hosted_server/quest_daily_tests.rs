//! First causal daily-cadence assertion observes real connected simulation.
use super::*;
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
        // Full-day, saved/offline phase and acceleration acceptance requires a
        // separately reviewed production contract after this causal baseline.
    }
}
