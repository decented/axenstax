use super::*;
fn header() -> ActionHeader {
    ActionHeader {
        session: [7; 16],
        epoch: 0,
        seq: 1,
        life: 0,
        events_applied: 0,
    }
}
fn view() -> QuestView {
    let quest = crate::progression::AcceptedQuest {
        id: [3; 16],
        npc: [4; 16],
        village: (2, -4),
        flavour: QuestFlavour::Fetch {
            item: crate::item::MaterialId::Wheat,
            count: 3,
        },
        rewards: vec![
            crate::saved_item_slot::SavedItemSlot::capture_exact(Some(
                &crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 2),
            ))
            .unwrap(),
        ],
        reputation: 12,
        baseline: 0,
    };
    QuestView {
        target: 19,
        npc: quest.npc,
        incarnation: 1,
        revision: 0,
        label: Text::new("Farmer").unwrap(),
        profession: Profession::Farmer,
        gossip: Some(Text::new("The wheat is ready.").unwrap()),
        terms: Some(QuestTerms::capture(&quest).unwrap()),
        status: QuestStatus::Offer,
        reputation: 0,
    }
}
fn result() -> QuestResult {
    let window = crate::window_events::EffectiveWindow {
        inv: crate::inventory::Inventory::new(),
        armour: [None; 4],
        cursor: None,
        grid: Default::default(),
    };
    QuestResult {
        actor_respawns: 0,
        action: header(),
        fate: ActionFate::Accepted,
        note: QuestNote::Done,
        window_event: 3,
        included_events: 5,
        window: PackWindow::capture(
            &window,
            &Default::default(),
            &crate::block::BlockRegistry::new(),
        )
        .unwrap(),
        view: Some(view()),
    }
}

#[test]
fn d2c3_quest_wire_binds_cost_to_last_fence_and_has_no_money_terms() {
    let packet = result();
    let decoded = QuestResult::decode(&packet.encode().unwrap()).unwrap();
    assert_eq!(decoded, packet);
    let native = decoded
        .view
        .unwrap()
        .terms
        .unwrap()
        .native(&crate::block::BlockRegistry::new())
        .unwrap();
    assert_eq!(native.reward.sats, 0);
    assert_eq!(
        native.reward.items,
        vec![crate::item::ItemStack::new_material(
            crate::item::MaterialId::Bread,
            2
        )]
    );
    assert!(!crate::quest::summarise(&native, true).contains("sats"));
    let mut bad = packet.clone();
    bad.included_events = bad.window_event;
    assert!(
        bad.encode().is_err(),
        "cost event alone released the final progression fence"
    );
    bad.included_events = bad.window_event + crate::progression::REWARD_LIMIT as u32 + 2;
    assert!(
        bad.encode().is_err(),
        "reward count bypassed its event bound"
    );
}

#[test]
fn d2c3_quest_text_and_reward_counts_refuse_declared_growth_before_allocation() {
    let over_text = (129u64).to_le_bytes();
    assert!(crate::protocol::safe_deserialize::<Text<128>>(&over_text).is_err());
    let over_rewards = (17u64).to_le_bytes();
    assert!(
        crate::protocol::safe_deserialize::<BoundedList<WireStack, 16>>(&over_rewards).is_err()
    );
    let mut bad = view();
    bad.label = Text(BoundedList::new(vec![0xff]).unwrap());
    assert!(bad.validate().is_err());
    let mut bad = view();
    bad.status = QuestStatus::Accepted {
        progress: 1,
        target: 4,
    };
    assert!(
        bad.validate().is_err(),
        "target differed from frozen quest flavour"
    );
}

#[test]
fn d2c3_quest_result_owned_budget_counts_actual_text_and_reward_capacities() {
    let mut packet = result();
    let original = packet.owned_bytes();
    let view = packet.view.as_mut().unwrap();
    let mut label = Vec::with_capacity(128);
    label.extend_from_slice(b"Farmer");
    view.label = Text(BoundedList::new(label).unwrap());
    let mut rewards = Vec::with_capacity(16);
    rewards.extend_from_slice(view.terms.as_ref().unwrap().rewards.as_slice());
    view.terms.as_mut().unwrap().rewards = BoundedList::new(rewards).unwrap();
    assert!(
        packet.owned_bytes() > original + 100,
        "logical text/item length replaced actual owned capacity"
    );
    let encoded = packet.encode().unwrap();
    assert_eq!(QuestResult::decode(&encoded).unwrap(), packet);
    let mut trailing = encoded.into_vec();
    trailing.push(0);
    assert!(QuestResult::decode(&trailing).is_err());
}

#[test]
fn d2c3_quest_reward_plan_and_mismatched_view_identity_refuse() {
    let mut bad = view();
    let plan = crate::plan::PlanData::marker_placeholder([31; 32], false);
    let stack = crate::item::ItemStack {
        item: crate::item::Item::Plan(plan),
        count: 1,
    };
    bad.terms.as_mut().unwrap().rewards =
        BoundedList::new(vec![crate::inventory::stack_to_wire(&stack)]).unwrap();
    assert!(bad.validate().is_err());
    assert!(
        bad.terms
            .unwrap()
            .native(&crate::block::BlockRegistry::new())
            .is_err()
    );
    let mut push = QuestViewPush {
        actor_respawns: 0,
        session: [7; 16],
        epoch: 0,
        life: 0,
        window_event: 3,
        target: 19,
        incarnation: 1,
        view: Some(view()),
    };
    assert!(push.validate().is_ok());
    push.incarnation = 2;
    assert!(push.validate().is_err());
}

#[test]
fn d2c3_quest_wire_appends_intent13_and_result76_view77_without_changing_pack_bytes() {
    assert_eq!(crate::protocol::PROTOCOL_VERSION, 92);
    assert_eq!(crate::protocol::PacketType::QuestResult as u8, 76);
    assert_eq!(crate::protocol::PacketType::QuestView as u8, 77);
    let intent = crate::action_wire::ActionIntent::Quest(QuestRequest {
        actor_respawns: 0,
        target: 19,
        before: 5,
        incarnation: 0,
        operation: QuestOperation::Talk,
    });
    assert!(intent.predicts_nothing());
    let packet = crate::action_wire::ActionPacket {
        action: header(),
        intent,
    };
    let bytes = packet.encode().unwrap();
    assert_eq!(&bytes[36..40], &13u32.to_le_bytes());
    assert!(matches!(
        crate::action_wire::ActionPacket::decode(&bytes)
            .unwrap()
            .intent,
        crate::action_wire::ActionIntent::Quest(QuestRequest {
            operation: QuestOperation::Talk,
            ..
        })
    ));
    let pack = crate::action_wire::ActionPacket {
        action: header(),
        intent: crate::action_wire::ActionIntent::Pack(crate::pack_wire::PackRequest {
            actor_respawns: 0,
            target: crate::container_window::ContainerTarget::Entity(9),
            hotbar_slot: 0,
            before: 3,
            operation: crate::pack_wire::PackOperation::Open,
        }),
    };
    assert_eq!(&pack.encode().unwrap()[36..40], &12u32.to_le_bytes());
}
