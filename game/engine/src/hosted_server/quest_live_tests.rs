//! Actual ordinary quest action and reward regression controls.
use super::quest_live_fixture::Rig;
use super::*;
use crate::transport::ClientTransport as _;
use crate::{
    action_identity::InventoryMode,
    item::{Item, ItemStack, MaterialId},
    quest_wire::QuestOperation,
};

#[test]
fn d2c3_actual_quest_dispatch_freezes_offer_accepts_once_and_uses_last_progression_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for signed in [true, false] {
            let mut rig = Rig::new("offer", mode, signed);
            let talk = rig.request(0, QuestOperation::Talk);
            assert_eq!(
                talk.result.fate,
                crate::action_identity::ActionFate::Accepted
            );
            let view = talk.result.view.unwrap();
            let terms = view.terms.unwrap();
            let again = rig.request(0, QuestOperation::Talk);
            assert_eq!(
                again.result.view.unwrap().terms.unwrap(),
                terms,
                "render/reopen cannot re-roll an active encounter"
            );
            rig.request(view.incarnation, QuestOperation::Accept { id: terms.id });
            assert_eq!(rig.record().quests.accepted.len(), 1);
            assert_eq!(
                rig.actor.quest_progress.accepted.len(),
                1,
                "actual PersonalSync installed the same accepted quest"
            );
            let repeated = rig.request(view.incarnation, QuestOperation::Accept { id: terms.id });
            assert_eq!(
                repeated.result.fate,
                crate::action_identity::ActionFate::Refused
            );
            assert_eq!(rig.record().quests.accepted.len(), 1);
            assert_eq!(
                rig.host
                    .server
                    .ecs
                    .get::<&crate::quest_npcs::QuestNpcId>(rig.entity)
                    .unwrap()
                    .0,
                view.npc
            );
            assert!(
                rig.host
                    .server
                    .ecs
                    .get::<&crate::entity::Scattered>(rig.entity)
                    .is_err()
            );
        }
    }
}

#[test]
fn d2c3_actual_completion_reward_overflow_uses_original_dead_giver_once_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("giver-overflow", mode, true);
        rig.request(0, QuestOperation::Talk);
        // Current natural pools retain their existing zero item rewards.
        // This explicit server-owned frozen reward fixture exercises the
        // supported exact item carrier without adding production reward amounts.
        rig.host.server.players[rig.i]
            .quest_session
            .as_mut()
            .unwrap()
            .offer
            .as_mut()
            .unwrap()
            .quest
            .rewards = vec![
            crate::saved_item_slot::SavedItemSlot::capture_exact(Some(&ItemStack::new_material(
                MaterialId::Bread,
                2,
            )))
            .unwrap(),
        ];
        let opened = rig.request(0, QuestOperation::Talk).result.view.unwrap();
        let terms = opened.terms.as_ref().unwrap();
        rig.request(opened.incarnation, QuestOperation::Accept { id: terms.id });
        let crate::quest::QuestFlavour::Fetch { item, count } = terms.flavour else {
            panic!("Blacksmith's existing pool is Fetch");
        };
        // Genuine numbered grants, leaving one cost unit in its occupied slot
        // after payment so rewards genuinely cannot fit any of the 36 places.
        let mut kit = vec![ItemStack::new_material(item, count + 1)];
        kit.extend((1..36).map(|_| ItemStack::new_block(crate::block::STONE, 64)));
        rig.grant(kit);
        let packet = rig.packet(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
        let reply = rig.send(&packet);
        assert_eq!(reply.result.note, crate::quest_wire::QuestNote::Completed);
        assert!(rig.record().quests.accepted.is_empty());
        assert_eq!(reply.grants.len(), terms.rewards.as_slice().len());
        let origin = rig
            .host
            .server
            .ecs
            .get::<&crate::entity::Position>(rig.entity)
            .unwrap()
            .0;
        rig.host.server.ecs.despawn(rig.entity).unwrap();
        rig.apply(&reply);
        let reward_count: u32 = terms
            .rewards
            .as_slice()
            .iter()
            .map(|stack| u32::from(stack.count))
            .sum();
        let spilled: u32 = rig
            .host
            .server
            .ecs
            .query::<(&crate::entity::ItemEntity, &crate::entity::Position)>()
            .iter()
            .filter(|(_, (item, position))| {
                matches!(item.stack.item, Item::Material(MaterialId::Bread))
                    && position.0.distance(origin) < 1.5
            })
            .map(|(_, (item, _))| u32::from(item.stack.count))
            .sum();
        assert_eq!(
            spilled, reward_count,
            "only genuine overflow returns beside its original giver after death"
        );
        let reputation = rig.record().reputation.clone();
        rig.client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::ItemAction,
            &protocol::ItemActionPacket {
                seq: 2,
                action: protocol::ItemAction::GrantUnfit {
                    event: reply.grants[0].window_event,
                    count: 255,
                },
                events_applied: rig.applied,
            },
        ));
        rig.host.tick();
        assert_eq!(
            rig.record().reputation,
            reputation,
            "a repeated overflow report cannot award reputation"
        );
        let after_repeat: u32 = rig
            .host
            .server
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter(|(_, item)| matches!(item.stack.item, Item::Material(MaterialId::Bread)))
            .map(|(_, item)| u32::from(item.stack.count))
            .sum();
        assert_eq!(
            after_repeat, reward_count,
            "fresh-sequence repeated feedback cannot spill the same grant twice"
        );
        let repeat = rig.packet(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
        let repeated = rig.send(&repeat);
        assert_eq!(
            repeated.result.fate,
            crate::action_identity::ActionFate::Refused
        );
        assert!(
            repeated.grants.is_empty(),
            "consumed identity and dead giver cannot grant again"
        );
    }
}

#[test]
fn d2c3_natural_pool_completion_keeps_reward_amounts_and_zero_money_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for signed in [false, true] {
            let mut rig = Rig::new("natural-completion", mode, signed);
            let opened = rig.request(0, QuestOperation::Talk).result.view.unwrap();
            let terms = opened.terms.as_ref().unwrap();
            assert!(
                terms.rewards.as_slice().is_empty(),
                "existing natural item amounts stay unchanged"
            );
            assert_eq!(
                terms.native(&rig.host.server.registry).unwrap().reward.sats,
                0
            );
            rig.request(opened.incarnation, QuestOperation::Accept { id: terms.id });
            let crate::quest::QuestFlavour::Fetch { item, count } = terms.flavour else {
                panic!("existing Blacksmith Fetch pool");
            };
            rig.grant(vec![ItemStack::new_material(item, count)]);
            let done = rig.request(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
            assert_eq!(done.result.note, crate::quest_wire::QuestNote::Completed);
            assert!(done.grants.is_empty());
            assert!(rig.record().quests.accepted.is_empty());
            assert!(rig.actor.quest_progress.accepted.is_empty());
            assert!(
                !rig.actor
                    .inventory
                    .slots_iter()
                    .flatten()
                    .any(|stack| stack.item == Item::Material(item))
            );
            assert_eq!(
                rig.record()
                    .reputation
                    .iter()
                    .find(|(village, _)| *village == (2, 3))
                    .map(|(_, score)| *score),
                Some(terms.reputation)
            );
            let before = rig.record();
            let repeated = rig.request(opened.incarnation, QuestOperation::TurnIn { id: terms.id });
            assert_eq!(
                repeated.result.fate,
                crate::action_identity::ActionFate::Refused
            );
            assert_eq!(
                rig.record(),
                before,
                "consumed quest identity cannot pay or adjust reputation again"
            );
        }
    }
}
