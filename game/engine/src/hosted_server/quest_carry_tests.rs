//! Predecessor-compilable own boundary for supported, settled common credit.
use super::{c3d_tests, quest_lifecycle_fixture::Room, quest_lifecycle_tests::key};
use crate::{action_identity::InventoryMode, handover::live::eligibility, mob::MobType};

#[test]
fn d2c3_settled_supported_kill_credit_is_eligible_for_the_chosen_recipient() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("carry-credit", mode, true);
        room.kill(0, MobType::Cow);
        room.quiesce();
        let i = room.actors[0].i();
        let sp = &room.host.server.players[i];
        let record = room.record(0);
        assert!(sp.verified_pubkey == Some(key(c3d_tests::KEY)), "the actual signed npub differs");
        assert!(sp.server_simulated && sp.is_in_world() && !sp.combat.dead);
        assert!(sp.progression_tracked);
        assert!(!sp.personal_read_failed);
        assert_eq!(record.kills, [(MobType::Cow, 1)]);
        assert_eq!(sp.progression_credits, 1);
        assert!(record.quests.is_empty());
        assert_eq!(record.tracking, crate::progression::KillTracking::Authoritative);
        assert!(room.host.server.pending_item_grants.is_empty());
        assert!(room.host.server.pending_bred_events.is_empty());
        assert!(sp.obligations_proven_zero());
        assert!(sp.personal_delivery.quiescent(sp.personal.as_ref().unwrap(), sp.respawns));
        assert!(sp.window_events.is_quiescent());
        assert!(room.host.server.pending_kill_events.is_empty());
        assert_eq!(room.actors[0].slot.kill_counter[&MobType::Cow], 1);
        assert_eq!(room.actors[0].kills.len(), 1);
        let before = sp.personal.as_ref().unwrap().bytes.clone();
        let clock = sp.gameplay_clock;
        let credits = sp.progression_credits;
        let seat = eligibility::Seat { slot: i, generation: sp.attach_gen, key: key(c3d_tests::KEY) };
        let result = eligibility::seats(&room.host.server, seat);
        eprintln!(
            "carry-credit own mode={mode:?} slot={i} generation={} credits={credits} revision={} result={result:?}",
            seat.generation, record.revision
        );
        let sp = &room.host.server.players[i];
        assert_eq!(sp.personal.as_ref().unwrap().bytes, before);
        assert_eq!(sp.gameplay_clock, clock);
        assert_eq!(sp.progression_credits, credits);
        assert!(room.host.server.pending_kill_events.is_empty());
        // FIRST OWN: this boundary currently refuses a genuinely supported,
        // successfully committed and currently applied common Cow credit.
        assert!(result.is_ok(), "settled supported common credit must be eligible: {result:?}");
    }
}
