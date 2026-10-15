//! Lower cut components with a real signed recipient and actual returned ACKs.
//! The default unbound local slot/scope=None are a numerical component fixture,
//! NOT native epoch authority, accepted controller flow or full carry evidence.
use super::*;
use crate::{
    action_identity::InventoryMode, hosted_server::quest_lifecycle_fixture::Room,
    quest_wire::QuestOperation, transport::ClientTransport as _,
};

const TRANSFER: Id = [0x71; 16];
const NOW: u64 = 100;
struct Fixture {
    room: Room,
    local: PlayerSlot,
    live: Live,
    seat: Seat,
    tick: u64,
    _lease: std::sync::Arc<()>,
}
impl Fixture {
    fn new(tag: &str, changed: bool) -> Self {
        let mut room = Room::new(tag, InventoryMode::Mirror, true);
        if changed {
            let view = room.quest(0, 0, QuestOperation::Talk).view.unwrap();
            let id = view.terms.unwrap().id;
            room.quest(0, view.incarnation, QuestOperation::Decline { id });
            room.ticks(3); // actual natural logical ticks; no assigned timer
            assert_eq!(room.record(0).quests.declines.len(), 1);
        }
        room.quiesce();
        let i = room.actors[0].i();
        let sp = &room.host.server.players[i];
        let key = sp.verified_pubkey.expect("actual signed Room admission");
        assert_eq!(
            sp.personal
                .as_ref()
                .unwrap()
                .binding
                .context()
                .participant(),
            Some(key)
        );
        let seat = Seat {
            slot: i,
            generation: sp.attach_gen,
            key,
        };
        let header = sp.action_frontier.as_ref().unwrap().header(0, 0);
        assert!(sp.personal_delivery.current(
            sp.personal.as_ref().unwrap(),
            header,
            &sp.window_events
        ));
        assert!(sp.progression_tracked && !sp.personal_read_failed);
        if changed {
            assert_ne!(
                sp.gameplay_clock
                    .project(
                        &sp.personal.as_ref().unwrap().record().unwrap(),
                        room.host.server.tick_counter
                    )
                    .0
                    .encode()
                    .unwrap(),
                sp.personal.as_ref().unwrap().bytes,
                "real elapsed decline timer needs a new cut fence"
            );
        }
        let lease = room
            .host
            .handover_bind(i, seat.generation, key, TRANSFER)
            .unwrap();
        room.host.handover_freeze(None).unwrap();
        let tick = room.host.server.tick_counter;
        Self {
            room,
            local: PlayerSlot::new(0, glam::Vec3::ZERO, 1.0),
            live: Live {
                signed_in: Some(key),
                local_seats: 1,
            },
            seat,
            tick,
            _lease: lease,
        }
    }
    fn identity(&self) -> CutIdentity {
        CutIdentity::checked(
            TRANSFER,
            &self.room.host,
            self.live,
            self.seat,
            self.tick,
            None,
        )
        .unwrap()
    }
    fn cut(&mut self) -> CutTicket {
        let identity = self.identity();
        let (ticket, output) = prepare_cut(&mut self.local, &mut self.room.host, identity, NOW)
            .unwrap()
            .commit();
        if let Some(wire) = output {
            self.room.host.send_carry_personal(self.seat.slot, &wire);
        }
        ticket
    }
    fn confirm(&mut self, ticket: &mut CutTicket) -> Result<CurrentPersonalProgress, Refusal> {
        ticket.confirm(
            &self.local,
            &self.room.host,
            self.live,
            self.tick,
            None,
            NOW + 1,
        )
    }
    fn actual_ack(&mut self, ticket: &CutTicket) {
        self.room.actors[0].drain(&self.room.host.server.registry);
        assert!(
            self.room
                .host
                .pump_handover_personal_applied(ticket, 8)
                .unwrap()
        );
    }
}
#[derive(Debug, PartialEq)]
struct Boundary {
    bytes: Vec<u8>,
    clock: crate::quest_state::GameplayClock,
    revision: u64,
    last: Option<PersonalAck>,
    pending: Option<PersonalAck>,
    last_event: u32,
    applied_event: u32,
    native_clock: crate::quest_state::GameplayClock,
    native_revision: u64,
}
fn boundary(f: &Fixture) -> Boundary {
    let sp = &f.room.host.server.players[f.seat.slot];
    Boundary {
        bytes: sp.personal.as_ref().unwrap().bytes.clone(),
        clock: sp.gameplay_clock,
        revision: sp.personal_delivery.revision,
        last: sp.personal_delivery.last,
        pending: sp
            .personal_delivery
            .pending
            .as_ref()
            .map(crate::personal_sync::acknowledgement),
        last_event: sp.window_events.last_number(),
        applied_event: sp.window_events.applied_number(),
        native_clock: f.local.gameplay_clock,
        native_revision: f.local.gameplay_revision,
    }
}
#[test]
fn d2c3_unchanged_current_delivery_requires_no_redundant_fence() {
    let mut f = Fixture::new("cut-unchanged", false);
    let before = boundary(&f);
    let mut ticket = f.cut();
    assert_eq!(ticket.expected(), before.last);
    let mut expected = before;
    expected.clock.committed(f.tick);
    assert_eq!(
        boundary(&f),
        expected,
        "already current bytes need no revision/event/Delivery change; clock reanchors honestly"
    );
    assert!(!ticket.settled());
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Settled
    );
    assert!(ticket.settled());
    // Inspect actual wire arrivals: an unchanged cut emits only the real
    // pause Notice, never a redundant Personal packet requiring another ACK.
    let mut heard = 0;
    while let Some(bytes) = f.room.actors[0].client.try_recv_from_server() {
        let (kind, payload) = crate::protocol::deserialize_header(&bytes).unwrap();
        assert_eq!(kind, crate::protocol::PacketType::Handover);
        assert!(matches!(
            crate::handover::live::wire::Frame::decode(payload).unwrap(),
            crate::handover::live::wire::Frame::Notice { paused: true, .. }
        ));
        heard += 1;
    }
    assert_eq!(heard, 1);
    assert_eq!(f.room.host.server.tick_counter, f.tick);
}
#[test]
fn d2c3_changed_cut_abandon_preserves_same_holders_clocks_delivery_and_numbers() {
    let mut f = Fixture::new("cut-abandon", true);
    let before = boundary(&f);
    let identity = f.identity();
    let prepared = prepare_cut(&mut f.local, &mut f.room.host, identity, NOW).unwrap();
    drop(prepared);
    assert_eq!(boundary(&f), before);
    assert!(
        f.room.host.server.players[f.seat.slot]
            .window_events
            .is_quiescent()
    );
    assert!(f.local.personal_binding.is_none());
    assert!(f.local.inventory.slots_iter().all(|slot| slot.is_none()));
}
#[test]
fn d2c3_changed_cut_waits_for_actual_same_actor_returned_current_ack() {
    let mut f = Fixture::new("cut-current", true);
    let before = boundary(&f);
    let mut ticket = f.cut();
    let expected = ticket.expected().unwrap();
    assert_eq!(expected.window_event, before.last_event + 1);
    assert_eq!(expected.revision, before.revision + 1);
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Waiting
    );
    assert!(!ticket.settled());
    assert_eq!(f.room.host.server.tick_counter, f.tick);
    f.actual_ack(&ticket);
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Settled
    );
    assert!(ticket.settled());
    let sp = &f.room.host.server.players[f.seat.slot];
    assert_eq!(sp.personal_delivery.last, Some(expected));
    assert!(sp.personal_delivery.pending.is_none());
    assert_eq!(sp.window_events.applied_number(), expected.window_event);
    let mut expected_clock = crate::quest_state::GameplayClock::default();
    expected_clock.committed(f.tick);
    assert_eq!(sp.gameplay_clock, expected_clock);
}
#[test]
fn d2c3_old_actual_ack_does_not_settle_a_new_cut_and_foreign_future_ack_refuses() {
    for variant in 0..3 {
        let mut f = Fixture::new(&format!("cut-old-ack-{variant}"), true);
        let old = f.room.host.server.players[f.seat.slot]
            .personal_delivery
            .last
            .unwrap();
        let mut ticket = f.cut();
        let expected = ticket.expected().unwrap();
        assert_ne!(old, expected);
        let mut input = old;
        if variant == 1 {
            input.session[0] ^= 1;
        }
        if variant == 2 {
            input.revision = expected.revision + 1;
            input.window_event = u32::MAX;
        }
        // The first input is the real previous ACK; the second is explicitly
        // malformed namespace input, never credited as an actual application.
        f.room.actors[0]
            .client
            .send_to_server(&crate::protocol::serialize_packet(
                crate::protocol::PacketType::PersonalSync,
                &crate::protocol::PersonalSyncPacket::Applied(input),
            ));
        let before = boundary(&f);
        let result = f.room.host.pump_handover_personal_applied(&ticket, 8);
        if variant != 0 {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap());
        }
        assert_eq!(boundary(&f), before);
        assert_eq!(
            f.confirm(&mut ticket).unwrap(),
            CurrentPersonalProgress::Waiting
        );
        assert!(!ticket.settled());
    }
}
#[test]
fn d2c3_settled_cut_loses_readiness_on_real_pending_reissue_and_invalid_current_body() {
    let mut f = Fixture::new("cut-proof-changed", true);
    let mut ticket = f.cut();
    let actual = f.room.host.server.players[f.seat.slot]
        .personal_delivery
        .pending
        .clone()
        .unwrap();
    f.actual_ack(&ticket);
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Settled
    );
    // Deliberately perturb current pending proof using the SAME genuine
    // captured carrier via Delivery.issue. This is a negative fault, never
    // a claimed new fence/ACK/readiness or authorised runtime reissue.
    f.room.host.server.players[f.seat.slot]
        .personal_delivery
        .issue(actual, f.tick);
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Waiting
    );
    assert!(!ticket.settled(), "Waiting revokes earlier settlement");
    // A duplicate ACK for the already-folded event cannot legitimise this
    // deliberately inconsistent pending reissue: no matching new window exists.
    f.room.host.resend_carry_personal(&ticket).unwrap();
    f.room.actors[0].drain(&f.room.host.server.registry);
    assert!(
        f.room
            .host
            .pump_handover_personal_applied(&ticket, 8)
            .is_err()
    );
    assert!(!ticket.settled());
    drop(f); // End the isolated worlds-root guard before the next fixture.
    let mut g = Fixture::new("cut-body-changed", true);
    let mut current = g.cut();
    g.actual_ack(&current);
    assert_eq!(
        g.confirm(&mut current).unwrap(),
        CurrentPersonalProgress::Settled
    );
    // Deliberate negative body fault after real settlement; do not persist it.
    g.room.host.server.players[g.seat.slot]
        .personal
        .as_mut()
        .unwrap()
        .bytes[0] ^= 1;
    assert!(
        current
            .check(&g.local, &g.room.host, g.live, g.tick, None)
            .is_err()
    );
    assert!(
        !current.settled(),
        "failed current recheck revokes earlier settlement"
    );
}
#[test]
fn d2c3_current_cut_reservation_refusal_and_abandon_emit_no_new_authority() {
    let mut f = Fixture::new("cut-reservations", false);
    let before = boundary(&f);
    let bytes = before.bytes.clone();
    assert!(
        f.room
            .host
            .prepare_carry_personal(
                f.seat.slot,
                &bytes,
                crate::action_journal::MAX_RETAINED_BYTES
            )
            .is_err()
    );
    assert_eq!(boundary(&f), before);
    let prepared = f
        .room
        .host
        .prepare_carry_personal(f.seat.slot, &bytes, 0)
        .unwrap();
    assert_eq!(boundary(&f), before);
    assert!(prepared.ready(&f.room.host.server.players[f.seat.slot]));
    prepared.abandon(&mut f.room.host.server.players[f.seat.slot]);
    assert_eq!(boundary(&f), before);
    let mut ticket = f.cut();
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Settled
    );
}
#[test]
fn d2c3_late_ordinary_chosen_raw_fifo_prevents_current_ack_extraction() {
    let mut f = Fixture::new("cut-late-ordinary", true);
    let mut ticket = f.cut();
    let actor = &mut f.room.actors[0];
    let at = actor.slot.player.pos;
    actor.input_seq += 1;
    actor
        .client
        .send_to_server(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::ClientInput,
            &crate::protocol::InputPacket {
                tick: actor.input_seq,
                x: at.x,
                y: at.y,
                z: at.z,
                health: 20.0,
                events_applied: actor.applied,
                ..Default::default()
            },
        ));
    // A later genuine returned ACK may not jump over that actual older input.
    f.room.actors[0].drain(&f.room.host.server.registry);
    let before = boundary(&f);
    assert!(
        f.room
            .host
            .pump_handover_personal_applied(&ticket, 8)
            .is_err()
    );
    assert!(f.room.host.handover_test_inbound_bytes(f.seat.slot) > 0);
    assert_eq!(boundary(&f), before);
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Waiting
    );
    assert!(!ticket.settled());
    assert_eq!(f.room.host.server.tick_counter, f.tick);
}

#[test]
fn d2c3_current_cut_deadline_is_checked_even_when_real_handover_control_waits() {
    let mut f = Fixture::new("cut-control-deadline", false);
    let mut ticket = f.cut();
    assert_eq!(
        f.confirm(&mut ticket).unwrap(),
        CurrentPersonalProgress::Settled
    );
    let frame = crate::handover::live::wire::Frame::Notice {
        transfer: TRANSFER,
        paused: true,
    }
    .encode()
    .unwrap();
    let mut packet = vec![crate::protocol::PacketType::Handover as u8];
    packet.extend_from_slice(&frame);
    f.room.actors[0].client.send_to_server(&packet);
    ticket
        .check(&f.local, &f.room.host, f.live, f.tick, None)
        .unwrap();
    assert!(!ticket.settled());
    assert!(
        !f.room
            .host
            .pump_handover_personal_applied(&ticket, 8)
            .unwrap()
    );
    assert!(f.room.host.handover_test_inbound_bytes(f.seat.slot) > 0);
    assert!(ticket.check_deadline(NOW + 61).is_err());
    assert!(!ticket.settled());
    assert_eq!(f.room.host.server.tick_counter, f.tick);
}
