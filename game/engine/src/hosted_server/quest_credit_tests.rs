//! Death-world context at the existing generation-checked credit dispatch.
use super::{c3d_tests, quest_lifecycle_fixture::Room, quest_lifecycle_tests::key};
use crate::{
    action_identity::InventoryMode,
    combat::Attacker,
    mob::MobType,
    progression::{GameplayContext, WorldPlayer},
    protocol,
    quest_wire::QuestOperation,
    server::{DeathLocation, route_client_kill},
    world::World,
};
use glam::Vec3;

fn current(room: &Room, who: usize) -> (usize, u32) {
    let i = room.actors[who].i();
    let sp = &room.host.server.players[i];
    assert!(sp.server_simulated && sp.is_in_world());
    assert!(!room.host.slot_is_free(i));
    assert_eq!(sp.verified_pubkey, Some(key(c3d_tests::KEY)));
    assert!(matches!(room.record(who).context, GameplayContext::WorldLocal {
        scope, player: WorldPlayer::Npub(npub),
    } if Some(scope) == room.host.server.world.quest_scope && npub == key(c3d_tests::KEY)));
    assert!(sp.personal_delivery.quiescent(sp.personal.as_ref().unwrap(), sp.respawns));
    (i, sp.attach_gen)
}

fn dispatch(room: &mut Room, i: usize, generation: u32, location: DeathLocation, nearest: bool) {
    let attacker = if nearest { None } else { Some(Attacker::Remote { slot: i, generation }) };
    assert_eq!(
        route_client_kill(Some(&mut room.host.server), attacker, MobType::Villager, location, false, &[]),
        None,
        "a remote credit never donates to a local seat"
    );
}

fn event(event: &protocol::KillEventPacket, pos: Vec3, nearest: bool) {
    assert_eq!(event.victim, protocol::EntityKind::Villager);
    assert_eq!(event.reason, if nearest { protocol::kill_reason::NEAREST } else { protocol::kill_reason::LAST_HIT });
    assert_eq!([event.x, event.y, event.z], pos.to_array());
    assert_eq!(event.victim_flags, 0);
}

#[test]
fn d2c3_death_location_resolves_the_actual_world_and_explicit_absence() {
    let pos = Vec3::new(40.5, 80.0, 42.5);
    let mut actual = World::new();
    assert_eq!(DeathLocation::from_world(&actual, pos).village(), None);
    actual.village_anchors.insert((2, 3), [40, 80, 42]);
    actual.village_anchors.insert((8, 9), [900, 80, 900]);
    let captured = DeathLocation::from_world(&actual, pos);
    assert_eq!(captured.village(), Some((2, 3)));
    let mut other = World::new();
    other.village_anchors.insert((7, 6), [40, 80, 42]);
    assert_eq!(DeathLocation::from_world(&other, pos).village(), Some((7, 6)));
    actual.village_anchors.clear();
    assert_eq!(captured.village(), Some((2, 3)), "the death carries its already-resolved scalar");
    assert_eq!(DeathLocation::from_world(&actual, pos).village(), None);
}

#[test]
fn d2c3_explicit_absent_death_village_never_falls_back_to_server_world() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("credit-absent", mode, true);
        let (i, generation) = current(&room, 0);
        let pos = room.host.server.players[i].player.pos + Vec3::Z * 2.5;
        assert_eq!(DeathLocation::from_world(&room.host.server.world, pos).village(), Some((2, 3)));
        // A distinct actual death world has no village. The owning server's
        // genuine anchor must not replace that authoritative absence.
        let death_world = World::new();
        let location = DeathLocation::from_world(&death_world, pos);
        assert_eq!(location.village(), None);
        let before = room.record(0);
        assert!(before.kills.is_empty() && before.reputation.is_empty());
        let credits = room.host.server.players[i].progression_credits;
        assert!(room.host.server.pending_kill_events.is_empty());
        dispatch(&mut room, i, generation, location, false);
        let mut expected = before;
        expected.kills.push((MobType::Villager, 1));
        expected.revision += 1;
        assert_eq!(room.record(0), expected, "absence still counts the kill without inventing a village penalty");
        assert_eq!(room.host.server.players[i].progression_credits, credits + 1);
        assert_eq!(room.host.server.pending_kill_events.len(), 1);
        assert_eq!(room.host.server.pending_kill_events[0].0, i);
        event(&room.host.server.pending_kill_events[0].1, pos, false);
        room.quiesce();
        assert_eq!(room.actors[0].kills.len(), 1);
        event(&room.actors[0].kills[0], pos, false);
        assert_eq!(room.actors[0].slot.kill_counter[&MobType::Villager], 1);
        assert_eq!(room.actors[0].slot.reputation.score((2, 3)), 0);
        assert_eq!(room.actors[0].slot.quest_progress.penalty_left, 0);
        current(&room, 0); // Only the real current returned Applied settles delivery.
    }
}

#[test]
fn d2c3_current_generation_death_credit_commits_counts_penalty_and_events_once() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("credit-current", mode, true);
        let offered = room.quest(0, 0, QuestOperation::Talk).view.unwrap();
        room.quest(0, offered.incarnation, QuestOperation::Decline { id: offered.terms.unwrap().id });
        room.quiesce();
        let (i, generation) = current(&room, 0);
        let pos = room.host.server.players[i].player.pos + Vec3::Z * 2.5;
        let location = DeathLocation::from_world(&room.host.server.world, pos);
        assert_eq!(location.village(), Some((2, 3)));
        let before = room.record(0);
        assert_eq!(before.quests.declines.len(), 1);
        assert!(before.kills.is_empty() && before.reputation.is_empty());
        let credits = room.host.server.players[i].progression_credits;
        assert!(room.host.server.pending_kill_events.is_empty());
        dispatch(&mut room, i, generation, location, false);
        let mut expected = before;
        expected.kills.push((MobType::Villager, 1));
        expected.reputation.push(((2, 3), -25));
        expected.quests.penalty_left = 600;
        expected.revision += 1;
        assert_eq!(room.record(0), expected, "one dispatch commits the same projected holder once");
        assert_eq!(room.host.server.players[i].progression_credits, credits + 1);
        assert_eq!(room.host.server.pending_kill_events.len(), 1);
        assert_eq!(room.host.server.pending_kill_events[0].0, i);
        event(&room.host.server.pending_kill_events[0].1, pos, false);
        // A second genuine dispatch in the same cooldown counts once again,
        // with the existing nearest attribution and without a second penalty.
        dispatch(&mut room, i, generation, location, true);
        expected.kills[0].1 = 2;
        expected.revision += 1;
        assert_eq!(room.record(0), expected);
        assert_eq!(room.host.server.players[i].progression_credits, credits + 2);
        assert_eq!(room.host.server.pending_kill_events.len(), 2);
        assert_eq!(room.host.server.pending_kill_events[1].0, i);
        event(&room.host.server.pending_kill_events[1].1, pos, true);
        room.quiesce();
        assert_eq!(room.actors[0].kills.len(), 2);
        event(&room.actors[0].kills[0], pos, false);
        event(&room.actors[0].kills[1], pos, true);
        assert_eq!(room.actors[0].slot.kill_counter[&MobType::Villager], 2);
        assert_eq!(room.actors[0].slot.reputation.score((2, 3)), -25);
        assert!(room.actors[0].slot.quest_progress.penalty_left > 0);
        let revision = room.record(0).revision;
        room.quiesce();
        assert_eq!(room.actors[0].kills.len(), 2, "later polling emits no duplicate credit event");
        assert_eq!(room.record(0).revision, revision);
        assert_eq!(room.host.server.players[i].progression_credits, credits + 2);
        current(&room, 0);
    }
}

#[test]
fn d2c3_stale_generation_death_location_changes_no_holder_or_event() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("credit-stale", mode, true);
        let (i, old_generation) = current(&room, 0);
        room.leave(0);
        let replacement = room.connect(Some(c3d_tests::KEY), [59; 32]);
        let (new_i, generation) = current(&room, replacement);
        assert_eq!(new_i, i);
        assert_ne!(generation, old_generation, "actual release and fresh signed attachment");
        let pos = room.host.server.players[i].player.pos + Vec3::Z * 2.5;
        let location = DeathLocation::from_world(&room.host.server.world, pos);
        assert_eq!(location.village(), Some((2, 3)));
        let sp = &room.host.server.players[i];
        let bytes = sp.personal.as_ref().unwrap().bytes.clone();
        let clock = sp.gameplay_clock;
        let credits = sp.progression_credits;
        let failed = sp.personal_read_failed;
        assert!(room.host.server.pending_kill_events.is_empty());
        let local_beside = [(pos, false)];
        assert_eq!(
            route_client_kill(
                Some(&mut room.host.server),
                Some(Attacker::Remote { slot: i, generation: old_generation }),
                MobType::Villager,
                location,
                false,
                &local_beside
            ),
            None
        );
        // The unchanged public owning wrapper must also refuse that real old
        // generation before mutating either the common holder or legacy queue.
        room.host.server.queue_kill_event(i, old_generation, MobType::Villager, pos, false, false);
        let sp = &room.host.server.players[i];
        assert_eq!(sp.personal.as_ref().unwrap().bytes, bytes);
        assert_eq!(sp.gameplay_clock, clock);
        assert_eq!(sp.personal_read_failed, failed);
        assert_eq!(sp.progression_credits, credits);
        assert!(room.host.server.pending_kill_events.is_empty());
        room.quiesce();
        assert!(room.actors[replacement].kills.is_empty());
        assert!(room.record(replacement).kills.is_empty());
        assert_eq!(room.actors[replacement].slot.reputation.score((2, 3)), 0);
        current(&room, replacement);
    }
}
