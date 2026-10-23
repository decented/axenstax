//! Current-attachment failures on real signed actors. Numerical/private-record
//! faults below are deliberate test inputs, never claimed earned progression.
use super::{c3d_tests, quest_lifecycle_fixture::Room, quest_lifecycle_tests::key};
use crate::{
    action_identity::InventoryMode,
    combat::Attacker,
    handover::live::eligibility,
    mob::MobType,
    progression::GameplayRecord,
    protocol,
    server::{DeathLocation, route_client_kill},
    transport::ClientTransport as _,
};

fn seat(room: &Room) -> eligibility::Seat {
    let i = room.actors[0].i();
    let sp = &room.host.server.players[i];
    assert!(sp.server_simulated && sp.is_in_world() && !room.host.slot_is_free(i));
    assert_eq!(sp.verified_pubkey, Some(key(c3d_tests::KEY)));
    eligibility::Seat {
        slot: i,
        generation: sp.attach_gen,
        key: key(c3d_tests::KEY),
    }
}
fn current_ack(room: &Room, who: usize) {
    let sp = &room.host.server.players[room.actors[who].i()];
    let header = sp.action_frontier.as_ref().unwrap().header(0, 0);
    assert!(
        sp.personal_delivery
            .current(sp.personal.as_ref().unwrap(), header, &sp.window_events)
    );
    assert!(sp.window_events.is_quiescent());
    assert!(
        sp.action_results
            .as_ref()
            .is_none_or(|results| results.is_quiescent())
    );
    assert!(room.actors[who].idle());
}
fn baseline(room: &Room) -> eligibility::Seat {
    let seat = seat(room);
    current_ack(room, 0);
    let sp = &room.host.server.players[seat.slot];
    assert!(sp.progression_tracked && !sp.personal_read_failed);
    assert_eq!(sp.progression_credits, 0);
    assert!(room.record(0).kills.is_empty());
    assert!(eligibility::seats(&room.host.server, seat).is_ok());
    seat
}
/// Existing authoritative generation/attribution dispatch, without a tick or
/// a fabricated record/ACK. The full physical combat control is used below.
fn dispatch(room: &mut Room, seat: eligibility::Seat, kind: MobType) {
    let before = room.host.server.players[seat.slot].progression_credits;
    let queued = room.host.server.pending_kill_events.len();
    let pos = room.host.server.players[seat.slot].player.pos + glam::Vec3::Z * 2.5;
    let death = DeathLocation::from_world(&room.host.server.world, pos);
    assert_eq!(
        route_client_kill(
            Some(&mut room.host.server),
            Some(Attacker::Remote {
                slot: seat.slot,
                generation: seat.generation
            }),
            kind,
            death,
            false,
            &[]
        ),
        None
    );
    assert_eq!(
        room.host.server.players[seat.slot].progression_credits,
        before + 1
    );
    assert_eq!(room.host.server.pending_kill_events.len(), queued + 1);
    let (recipient, event) = &room.host.server.pending_kill_events[queued];
    assert_eq!(*recipient, seat.slot);
    assert_eq!(event.reason, protocol::kill_reason::LAST_HIT);
    assert_eq!(event.victim, crate::entity_broadcast::wire_kind_for(kind));
}
fn file(room: &Room) -> Option<Vec<u8>> {
    let path =
        crate::handover::live::personal_adapter::path(&room.directory(), key(c3d_tests::KEY));
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("actual selected file could not be read: {error}"),
    }
}
#[test]
fn d2c3_missing_current_holder_stays_incomplete_after_real_success_and_current_ack() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("carry-missing", mode, true);
        let chosen = baseline(&room);
        let selected = file(&room);
        let sp = &mut room.host.server.players[chosen.slot];
        // Deliberate absent-holder fault on this real current attachment.
        let holder = sp.personal.take().unwrap();
        let bytes = holder.bytes.clone();
        let clock = sp.gameplay_clock;
        dispatch(&mut room, chosen, MobType::Cow);
        let sp = &room.host.server.players[chosen.slot];
        assert!(sp.personal.is_none());
        assert_eq!(sp.gameplay_clock, clock);
        assert!(!sp.progression_tracked);
        assert!(
            !sp.personal_read_failed,
            "absence leaves the established error flag unchanged"
        );
        assert_eq!(holder.bytes, bytes);
        assert_eq!(file(&room), selected);
        // Restore only the deliberately removed fixture holder, not flags,
        // records, counters, clocks, readiness or acknowledgement state.
        room.host.server.players[chosen.slot].personal = Some(holder);
        room.quiesce();
        assert_eq!(room.actors[0].kills.len(), 1);
        room.kill(0, MobType::Cow); // actual backed empty-hand request + death
        room.quiesce();
        current_ack(&room, 0);
        let sp = &room.host.server.players[chosen.slot];
        assert_eq!(room.record(0).kills, [(MobType::Cow, 1)]);
        assert_eq!(
            room.actors[0].kills.len(),
            2,
            "one legacy event per actual dispatch"
        );
        assert_eq!(sp.progression_credits, 2);
        assert!(!sp.progression_tracked && !sp.personal_read_failed);
        assert!(eligibility::seats(&room.host.server, chosen).is_err());
    }
}

#[derive(Clone, Copy, Debug)]
enum Fault {
    Corrupt,
    Oversized,
    Count,
    Revision,
    EncodeEntries,
}
fn fault_bytes(mut record: GameplayRecord, fault: Fault) -> Vec<u8> {
    match fault {
        Fault::Corrupt => return vec![0, 1, 2],
        Fault::Oversized => return vec![0; crate::progression::RECORD_LIMIT + 1],
        Fault::Count => record.kills = vec![(MobType::Cow, u32::MAX)],
        Fault::Revision => record.revision = u64::MAX,
        Fault::EncodeEntries => {
            // Valid checked-bound source body. The real village penalty adds
            // a new row, so encoding its 2049th reputation entry must refuse.
            record.reputation = (0..crate::progression::REPUTATION_LIMIT)
                .map(|n| ((10_000 + n as i32, 10_000), 0))
                .collect();
            assert!(
                !record
                    .reputation
                    .iter()
                    .any(|(village, _)| *village == (2, 3))
            );
        }
    }
    record
        .encode()
        .expect("deliberate numerical source fixture is valid before mutation")
}
#[test]
fn d2c3_current_common_faults_preserve_bytes_clock_and_sticky_error_policy() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for fault in [
            Fault::Corrupt,
            Fault::Oversized,
            Fault::Count,
            Fault::Revision,
            Fault::EncodeEntries,
        ] {
            let mut room = Room::new(&format!("carry-fault-{fault:?}"), mode, true);
            let chosen = baseline(&room);
            let selected = file(&room);
            let original = room.host.server.players[chosen.slot]
                .personal
                .as_ref()
                .unwrap()
                .bytes
                .clone();
            let invalid = fault_bytes(room.record(0), fault);
            let sp = &mut room.host.server.players[chosen.slot];
            sp.personal.as_mut().unwrap().bytes = invalid.clone();
            let clock = sp.gameplay_clock;
            let kind = if matches!(fault, Fault::EncodeEntries) {
                MobType::Villager
            } else {
                MobType::Cow
            };
            dispatch(&mut room, chosen, kind);
            let sp = &room.host.server.players[chosen.slot];
            assert_eq!(
                sp.personal.as_ref().unwrap().bytes,
                invalid,
                "fault={fault:?}"
            );
            assert_eq!(sp.gameplay_clock, clock, "fault={fault:?}");
            assert!(!sp.progression_tracked && sp.personal_read_failed);
            assert_eq!(file(&room), selected);
            // A later missing-holder path must not clear the actual error.
            let holder = room.host.server.players[chosen.slot]
                .personal
                .take()
                .unwrap();
            dispatch(&mut room, chosen, MobType::Cow);
            assert!(room.host.server.players[chosen.slot].personal_read_failed);
            room.host.server.players[chosen.slot].personal = Some(holder);
            // Restore deliberate fault bytes only, then take a genuinely
            // successful common mutation through the real current dispatch.
            room.host.server.players[chosen.slot]
                .personal
                .as_mut()
                .unwrap()
                .bytes = original;
            dispatch(&mut room, chosen, MobType::Cow);
            let sp = &room.host.server.players[chosen.slot];
            assert_eq!(
                sp.personal.as_ref().unwrap().record().unwrap().kills,
                [(MobType::Cow, 1)]
            );
            assert_eq!(sp.progression_credits, 3);
            assert!(!sp.progression_tracked && sp.personal_read_failed);
            assert!(eligibility::seats(&room.host.server, chosen).is_err());
            // Existing readFailed policy disconnects before another live ACK.
            // Do not clear it or claim a current ACK on this errored holder.
            room.host.flush_personal_updates();
            assert!(room.host.disconnected[chosen.slot]);
            assert_eq!(
                file(&room),
                selected,
                "existing selected bytes remain protected"
            );
        }
    }
}

#[test]
fn d2c3_stale_generation_cannot_downgrade_replacement_or_emit_a_vow_event() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("carry-stale-incomplete", mode, true);
        let old = baseline(&room);
        room.leave(0);
        let who = room.connect(Some(c3d_tests::KEY), [79; 32]);
        let i = room.actors[who].i();
        assert_eq!(i, old.slot);
        let sp = &room.host.server.players[i];
        assert_ne!(sp.attach_gen, old.generation);
        current_ack(&room, who);
        let bytes = sp.personal.as_ref().unwrap().bytes.clone();
        let clock = sp.gameplay_clock;
        let credits = sp.progression_credits;
        let flags = (sp.progression_tracked, sp.personal_read_failed);
        assert_eq!(flags, (true, false));
        let pos = sp.player.pos + glam::Vec3::Z * 2.5;
        room.host
            .server
            .queue_kill_event(i, old.generation, MobType::Nostrich, pos, false, false);
        let sp = &room.host.server.players[i];
        assert_eq!(sp.personal.as_ref().unwrap().bytes, bytes);
        assert_eq!(sp.gameplay_clock, clock);
        assert_eq!(sp.progression_credits, credits);
        assert_eq!((sp.progression_tracked, sp.personal_read_failed), flags);
        assert!(room.host.server.pending_kill_events.is_empty());
        room.quiesce();
        assert!(room.actors[who].kills.is_empty());
        current_ack(&room, who);
    }
}

fn nostrich_death(room: &mut Room, nearest: bool, tamed: bool) {
    room.quiesce();
    let i = room.actors[0].i();
    while room.host.server.tick_counter < room.host.server.players[i].next_swing_tick {
        room.tick();
    }
    let at = room.host.server.players[i].player.pos + glam::Vec3::Z * 2.5;
    let entity = crate::entity::spawn_mob(&mut room.host.server.ecs, MobType::Nostrich, at);
    room.tick();
    let id = room
        .host
        .server
        .ecs
        .get::<&crate::entity::ProtocolId>(entity)
        .unwrap()
        .0;
    if tamed {
        // Authored physical pet fixture, not claimed player tame/progression.
        // The actual death sweep derives TAMED from this real component.
        let owner = hex::encode(key(c3d_tests::KEY));
        let mut data = room
            .host
            .server
            .ecs
            .get::<&mut crate::nostrich::NostrichData>(entity)
            .unwrap();
        for seed in 0..128 {
            crate::nostrich::attempt_tame_with_berry(
                &mut data,
                Some(crate::item::MaterialId::Berries),
                &owner,
                seed,
            );
            if data.is_tamed() {
                break;
            }
        }
        assert!(data.is_tamed());
    }
    room.host
        .server
        .ecs
        .get::<&mut crate::entity::Position>(entity)
        .unwrap()
        .0 = at;
    room.host
        .server
        .ecs
        .get::<&mut crate::combat::Health>(entity)
        .unwrap()
        .current = 1.0;
    if nearest {
        // Real unattributed environmental damage; the ordinary death sweep
        // itself chooses the current nearest living signed actor.
        assert!(
            room.host
                .server
                .ecs
                .get::<&mut crate::combat::Health>(entity)
                .unwrap()
                .take_damage(1.0)
        );
    } else {
        let actor = &mut room.actors[0];
        assert!(actor.slot.inventory.slot(8).is_none());
        actor.legacy_seq += 1;
        let request = protocol::EntityAttackPacket {
            seq: actor.legacy_seq,
            entity: id,
            held_kind: protocol::item_kind::EMPTY,
            held_id: 0,
            held_full: protocol::WireItem::None,
            sprint: false,
            sneak: false,
            hotbar_slot: 8,
            events_applied: actor.applied,
        };
        if room.mode == InventoryMode::Strict {
            actor.send(crate::action_wire::ActionIntent::Attack(request));
        } else {
            actor.client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::EntityAttack,
                &request,
            ));
        }
    }
    room.tick();
    assert!(!room.host.server.ecs.contains(entity));
    room.quiesce();
}
#[test]
fn d2c3_wild_nostrich_is_incomplete_after_last_hit_or_nearest_and_current_ack() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for nearest in [false, true] {
            let mut room = Room::new(&format!("carry-vow-{nearest}"), mode, true);
            let chosen = baseline(&room);
            nostrich_death(&mut room, nearest, false);
            current_ack(&room, 0);
            let sp = &room.host.server.players[chosen.slot];
            assert_eq!(room.record(0).kills, [(MobType::Nostrich, 1)]);
            assert_eq!(room.actors[0].kills.len(), 1);
            let event = &room.actors[0].kills[0];
            assert_eq!(
                event.reason,
                if nearest {
                    protocol::kill_reason::NEAREST
                } else {
                    protocol::kill_reason::LAST_HIT
                }
            );
            assert_eq!(event.victim_flags & protocol::entity_flags::TAMED, 0);
            assert_eq!(sp.progression_credits, 1);
            assert!(!sp.progression_tracked && !sp.personal_read_failed);
            assert!(eligibility::seats(&room.host.server, chosen).is_err());
            room.kill(0, MobType::Cow);
            current_ack(&room, 0);
            let sp = &room.host.server.players[chosen.slot];
            assert_eq!(sp.progression_credits, 2);
            assert_eq!(room.actors[0].kills.len(), 2);
            assert!(!sp.progression_tracked && !sp.personal_read_failed);
            assert!(eligibility::seats(&room.host.server, chosen).is_err());
        }
    }
}
#[test]
fn d2c3_actual_tamed_nostrich_death_keeps_common_completeness_in_both_attributions() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for nearest in [false, true] {
            let mut room = Room::new(&format!("carry-tamed-{nearest}"), mode, true);
            let chosen = baseline(&room);
            nostrich_death(&mut room, nearest, true);
            current_ack(&room, 0);
            let sp = &room.host.server.players[chosen.slot];
            assert_eq!(room.record(0).kills, [(MobType::Nostrich, 1)]);
            assert_eq!(sp.progression_credits, 1);
            assert!(sp.progression_tracked && !sp.personal_read_failed);
            assert_eq!(room.actors[0].kills.len(), 1);
            let event = &room.actors[0].kills[0];
            assert_eq!(
                event.reason,
                if nearest {
                    protocol::kill_reason::NEAREST
                } else {
                    protocol::kill_reason::LAST_HIT
                }
            );
            assert_ne!(event.victim_flags & protocol::entity_flags::TAMED, 0);
            assert!(eligibility::seats(&room.host.server, chosen).is_ok());
        }
    }
}
