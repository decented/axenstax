//! Genuine attachment, quest ownership, departure, replacement and restart.
use super::quest_lifecycle_fixture::Room;
use super::*;
use crate::{
    action_identity::{ActionFate, InventoryMode},
    quest_wire::{QuestNote, QuestOperation},
    transport::ClientTransport as _,
};
const OTHER: [u8; 32] = [39; 32];
pub(super) fn key(secret: [u8; 32]) -> [u8; 32] {
    nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap())
        .public_key()
        .to_bytes()
}
pub(super) fn score(record: &crate::progression::GameplayRecord) -> i16 {
    record
        .reputation
        .iter()
        .find(|(village, _)| *village == (2, 3))
        .map_or(0, |(_, value)| *value)
}
fn kills(record: &crate::progression::GameplayRecord, mob: crate::mob::MobType) -> u32 {
    record
        .kills
        .iter()
        .find(|(kind, _)| *kind == mob)
        .map_or(0, |(_, count)| *count)
}

#[test]
fn d2c3_two_signed_actors_keep_actual_quests_kills_and_reputation_separate_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("two-actors", mode, true);
        let other = room.connect(Some(OTHER), [40; 32]);
        assert_ne!(key(c3d_tests::KEY), key(OTHER));
        for (who, secret) in [(0, c3d_tests::KEY), (other, OTHER)] {
            assert_eq!(
                room.host.server.players[room.actors[who].i()].verified_pubkey,
                Some(key(secret))
            );
            assert!(
                matches!(room.record(who).context, crate::progression::GameplayContext::WorldLocal {
                player: crate::progression::WorldPlayer::Npub(npub), scope,
            } if npub == key(secret) && Some(scope) == room.host.server.world.quest_scope)
            );
        }
        assert_ne!(
            room.actors[0].accepted.action_session,
            room.actors[other].accepted.action_session
        );
        assert_ne!(room.record(0).context, room.record(other).context);
        let a = room.natural_kill_offer(0);
        let terms = a.terms.as_ref().unwrap();
        let b = room.quest(other, 0, QuestOperation::Talk).view.unwrap();
        assert_eq!(a.npc, b.npc, "same real giver, independent player holders");
        room.quest(0, a.incarnation, QuestOperation::Accept { id: terms.id });
        let before = room.record(0);
        room.quest(
            other,
            b.incarnation,
            QuestOperation::Decline {
                id: b.terms.as_ref().unwrap().id,
            },
        );
        let after = room.record(0);
        assert_eq!(after.quests.accepted, before.quests.accepted);
        assert!(after.quests.declines.is_empty());
        assert_eq!(room.record(other).quests.declines.len(), 1);
        assert!(room.record(other).quests.accepted.is_empty());
        let crate::quest::QuestFlavour::Kill { mob, .. } = terms.flavour else {
            unreachable!()
        };
        room.kill(other, mob);
        assert_eq!(kills(&room.record(0), mob), 0);
        assert_eq!(kills(&room.record(other), mob), 1);
        assert_eq!(room.record(0).quests.accepted, before.quests.accepted);
        let own_score = score(&room.record(0));
        room.kill(other, crate::mob::MobType::Villager);
        assert_eq!(score(&room.record(0)), own_score);
        assert_eq!(score(&room.record(other)), -25);
        assert_eq!(
            room.actors[0].slot.quest_progress.accepted,
            room.record(0).quests.accepted
        );
        assert_eq!(room.actors[other].slot.quest_progress.declines.len(), 1);
    }
}

#[test]
fn d2c3_actual_natural_kill_quest_freezes_preaccept_credit_and_completes_once_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("natural-kill", mode, true);
        let other = room.connect(Some(OTHER), [41; 32]);
        let opened = room.natural_kill_offer(0);
        let terms = opened.terms.as_ref().unwrap();
        let crate::quest::QuestFlavour::Kill { mob, count } = terms.flavour else {
            unreachable!()
        };
        assert_eq!(
            terms
                .native(&room.host.server.registry)
                .unwrap()
                .reward
                .sats,
            0
        );
        assert!(terms.rewards.as_slice().is_empty());
        room.kill(0, mob);
        room.kill(0, mob);
        assert_eq!(kills(&room.record(0), mob), 2);
        room.quest(
            0,
            opened.incarnation,
            QuestOperation::Accept { id: terms.id },
        );
        let accepted = room.record(0).quests.accepted[0].clone();
        assert_eq!(
            accepted.baseline, 2,
            "actual prior attributed kills are frozen at Accept"
        );
        assert_eq!(accepted.id, terms.id);
        let early = room.quest(
            0,
            opened.incarnation,
            QuestOperation::TurnIn { id: terms.id },
        );
        assert_eq!(early.fate, ActionFate::Refused);
        room.kill(other, mob);
        assert_eq!(kills(&room.record(0), mob), 2);
        for _ in 0..count {
            room.kill(0, mob);
        }
        assert_eq!(kills(&room.record(0), mob), 2 + u32::from(count));
        let other_before = room.record(other);
        let treasury = room.host.server.world.village_treasuries.clone();
        let completed = room.quest(
            0,
            opened.incarnation,
            QuestOperation::TurnIn { id: terms.id },
        );
        assert_eq!(completed.note, QuestNote::Completed);
        assert!(room.record(0).quests.accepted.is_empty());
        assert_eq!(score(&room.record(0)), terms.reputation);
        assert_eq!(room.host.server.world.village_treasuries, treasury);
        assert_eq!(room.record(other).reputation, other_before.reputation);
        assert_eq!(room.record(other).kills, other_before.kills);
        let before = room.record(0);
        let repeated = room.quest(
            0,
            opened.incarnation,
            QuestOperation::TurnIn { id: terms.id },
        );
        assert_eq!(repeated.fate, ActionFate::Refused);
        assert_eq!(room.record(0).kills, before.kills);
        assert_eq!(room.record(0).reputation, before.reputation);
        assert_eq!(room.record(0).quests.accepted, before.quests.accepted);
        assert_eq!(room.host.server.world.village_treasuries, treasury);
    }
}

#[test]
fn d2c3_signed_quest_reconnect_replacement_and_restart_keep_original_giver_and_live_timers() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("reconnect-restart", mode, true);
        let secret = c3d_tests::KEY;
        let opened = room.natural_kill_offer(0);
        room.quest(
            0,
            opened.incarnation,
            QuestOperation::Accept {
                id: opened.terms.as_ref().unwrap().id,
            },
        );
        let declined = room.decline_elsewhere(0);
        room.kill(0, crate::mob::MobType::Villager);
        room.ticks(7);
        room.quiesce();
        let actor = &room.actors[0];
        let old_namespace = actor.accepted.action_session;
        let expected = room.leave(0);
        let expected_record = expected.personal.as_ref().unwrap().record().unwrap();
        assert!(
            expected_record.quests.penalty_left > 0 && expected_record.quests.penalty_left < 600
        );
        assert!(
            expected_record
                .quests
                .declines
                .iter()
                .any(|(npc, ticks)| *npc == declined && *ticks > 0)
        );
        let saved = room.saved(key(secret));
        assert_eq!(
            saved.encode().unwrap(),
            expected.encode().unwrap(),
            "actual departure captured honest online state"
        );
        let saved_bytes = saved.encode().unwrap();
        room.ticks(53);
        assert_eq!(
            room.saved(key(secret)).encode().unwrap(),
            saved_bytes,
            "offline host ticks cannot rewrite this disconnected holder"
        );
        let rejoined = room.connect(Some(secret), [42; 32]);
        assert_ne!(room.actors[rejoined].accepted.action_session, old_namespace);
        let restored = room.record(rejoined);
        assert_eq!(restored.quests.accepted, expected_record.quests.accepted);
        assert!(restored.quests.penalty_left <= expected_record.quests.penalty_left);
        let old_decline = expected_record
            .quests
            .declines
            .iter()
            .find(|(npc, _)| *npc == declined)
            .unwrap()
            .1;
        let new_decline = restored
            .quests
            .declines
            .iter()
            .find(|(npc, _)| *npc == declined)
            .unwrap()
            .1;
        assert!(
            old_decline - new_decline < 12,
            "decline and penalty project from only actual online ticks"
        );
        assert!(
            expected_record.quests.penalty_left - restored.quests.penalty_left < 12,
            "only actual reattachment ticks elapsed"
        );
        let old_index = room.actors[rejoined].i();
        let old_generation = room.host.server.players[old_index].attach_gen;
        let old_namespace = room.actors[rejoined].accepted.action_session;
        let replacement = room.connect(Some(secret), [43; 32]);
        assert!(!room.actors[rejoined].active);
        assert!(room.host.disconnected[old_index]);
        assert!(!room.actors[rejoined].rejects.is_empty());
        assert_ne!(
            room.actors[replacement].accepted.action_session,
            old_namespace
        );
        assert_ne!(
            room.host.server.players[room.actors[replacement].i()].attach_gen,
            old_generation
        );
        assert_eq!(
            room.record(replacement).quests.accepted,
            expected_record.quests.accepted
        );
        assert_eq!(room.actors.iter().filter(|actor| actor.active).count(), 1);
        let scope = room.host.server.world.quest_scope;
        room.leave(replacement);
        let before_restart = room.saved(key(secret)).personal.unwrap().record().unwrap();
        room.host.server.try_save().unwrap();
        let carrier = crate::save::read_world_save(
            &std::fs::read(room.directory().join("world.dat")).unwrap(),
        )
        .unwrap()
        .quest_world
        .unwrap();
        assert_eq!(carrier.scope, scope.unwrap());
        assert_eq!(
            carrier
                .npcs
                .iter()
                .filter(|npc| npc.id == opened.npc)
                .count(),
            1
        );
        let mut room = room.restart();
        assert_eq!(room.host.server.world.quest_scope, scope);
        let rejoined = room.connect(Some(secret), [44; 32]);
        let record = room.record(rejoined);
        assert_eq!(record.quests.accepted, before_restart.quests.accepted);
        assert_eq!(record.kills, before_restart.kills);
        assert_eq!(record.reputation, before_restart.reputation);
        assert!(before_restart.quests.penalty_left - record.quests.penalty_left < 12);
        let before_decline = before_restart
            .quests
            .declines
            .iter()
            .find(|(npc, _)| *npc == declined)
            .unwrap()
            .1;
        let after_decline = record
            .quests
            .declines
            .iter()
            .find(|(npc, _)| *npc == declined)
            .unwrap()
            .1;
        assert!(
            before_decline - after_decline < 12,
            "restart retained the actual declined giver and honest live interval"
        );
        let view = room.quest(rejoined, 0, QuestOperation::Talk).view.unwrap();
        assert_eq!(view.npc, opened.npc);
        assert_eq!(view.terms.unwrap(), opened.terms.unwrap());
    }
}

fn player_files(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut todo = vec![directory.to_path_buf()];
    while let Some(at) = todo.pop() {
        if !at.exists() {
            continue;
        }
        for entry in std::fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                let relative = path.strip_prefix(directory).unwrap();
                let name = relative.to_string_lossy();
                if name.contains("joiners")
                    || name.contains("participants")
                    || name.contains("personal")
                    || name.starts_with(".handover")
                {
                    files.push(relative.to_path_buf());
                }
            }
        }
    }
    files.sort();
    files
}
fn guest_selected_is_world_only(room: &Room, giver: crate::progression::GameplayId) {
    let bytes = std::fs::read(room.directory().join("world.dat")).unwrap();
    let selected = crate::save::read_world_save(&bytes).unwrap();
    assert!(
        selected.local_personal.is_none(),
        "the selected world cannot hide an embedded guest holder"
    );
    let carrier = selected.quest_world.unwrap();
    assert_eq!(Some(carrier.scope), room.host.server.world.quest_scope);
    assert_eq!(
        carrier.npcs.iter().filter(|npc| npc.id == giver).count(),
        1,
        "world-owned giver persistence is permitted independently of a guest"
    );
}
#[test]
fn d2c3_guest_quest_leave_reuses_a_new_generation_without_a_personal_file() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut room = Room::new("guest-lifecycle", mode, false);
        let files_before = player_files(&room.directory());
        assert!(files_before.is_empty());
        let opened = room.natural_kill_offer(0);
        room.quest(
            0,
            opened.incarnation,
            QuestOperation::Accept {
                id: opened.terms.as_ref().unwrap().id,
            },
        );
        room.decline_elsewhere(0);
        room.kill(0, crate::mob::MobType::Villager);
        let (victim, retained_stamp) = room.wounded_mob(0);
        let context = room.record(0).context;
        assert!(matches!(
            context,
            crate::progression::GameplayContext::Guest { .. }
        ));
        assert_eq!(score(&room.record(0)), -25);
        let old_slot = room.actors[0].i();
        let old_generation = room.host.server.players[old_slot].attach_gen;
        let old_namespace = room.actors[0].accepted.action_session;
        room.host.server.try_save().unwrap();
        guest_selected_is_world_only(&room, opened.npc);
        let selected_bytes = std::fs::read(room.directory().join("world.dat")).unwrap();
        room.leave(0);
        assert_eq!(
            std::fs::read(room.directory().join("world.dat")).unwrap(),
            selected_bytes
        );
        guest_selected_is_world_only(&room, opened.npc);
        assert_eq!(player_files(&room.directory()), files_before);
        let fresh = room.connect(None, [45; 32]);
        assert_eq!(room.actors[fresh].i(), old_slot);
        assert_ne!(
            room.host.server.players[old_slot].attach_gen,
            old_generation
        );
        assert_ne!(room.actors[fresh].accepted.action_session, old_namespace);
        let record = room.record(fresh);
        assert_ne!(record.context, context);
        assert!(record.quests.accepted.is_empty() && record.quests.declines.is_empty());
        assert!(record.kills.is_empty() && record.reputation.is_empty());
        assert_eq!(record.quests.penalty_left, 0);
        // A genuine stamp retained outside the release pass still names the old generation.
        room.host
            .server
            .ecs
            .insert_one(victim, crate::combat::LastAttacker(retained_stamp))
            .unwrap();
        room.ticks(20);
        let lethal = room
            .host
            .server
            .ecs
            .get::<&crate::combat::Health>(victim)
            .unwrap()
            .max;
        assert!(
            room.host
                .server
                .ecs
                .get::<&mut crate::combat::Health>(victim)
                .unwrap()
                .take_damage(lethal)
        );
        room.tick();
        assert!(!room.host.server.ecs.contains(victim));
        room.quiesce();
        assert!(room.record(fresh).kills.is_empty() && room.record(fresh).reputation.is_empty());
        assert!(
            room.actors[fresh].kills.is_empty(),
            "a retained old actual attacker cannot donate credit to the reused slot"
        );
        room.host.server.try_save().unwrap();
        guest_selected_is_world_only(&room, opened.npc);
        assert_eq!(player_files(&room.directory()), files_before);
    }
}
