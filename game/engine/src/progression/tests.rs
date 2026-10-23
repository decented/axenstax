use super::*;
#[test]
fn d2c3_same_progression_codec_preserves_quests_timers_context_and_exact_rewards() {
    let contexts = [
        GameplayContext::Imported(PersonalContext {
            participant: [7; 32],
            lineage: [3; 16],
            generation: 19,
            transfer: [4; 16],
        }),
        GameplayContext::WorldLocal {
            scope: [5; 16],
            player: WorldPlayer::Npub([7; 32]),
        },
        GameplayContext::Guest {
            session: [9; 16],
            generation: 17,
        },
    ];
    for context in contexts {
        let mut record = GameplayRecord::empty(context).unwrap();
        record.revision = 39;
        record.reputation = vec![((3, -7), 21)];
        record.kills = vec![(MobType::Pig, 44)];
        record.claims = vec![(13, 61)];
        record.quests.accepted.push(AcceptedQuest {
            id: [1; 16],
            npc: [2; 16],
            village: (3, -7),
            flavour: QuestFlavour::Kill {
                mob: MobType::Pig,
                count: 4,
            },
            rewards: vec![
                SavedItemSlot::capture_exact(Some(&crate::item::ItemStack::new_material(
                    crate::item::MaterialId::Bread,
                    5,
                )))
                .unwrap(),
            ],
            reputation: 3,
            baseline: 40,
        });
        record.quests.declines = vec![([3; 16], 1234)];
        record.quests.penalty_left = 129;
        let bytes = record.encode().unwrap();
        assert_eq!(GameplayRecord::decode(&bytes, context).unwrap(), record);
        let foreign = GameplayContext::Guest {
            session: [8; 16],
            generation: 17,
        };
        assert!(GameplayRecord::decode(&bytes, foreign).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(GameplayRecord::decode(&trailing, context).is_err());
        assert!(GameplayRecord::decode(&bytes[..bytes.len() - 1], context).is_err());
        record.quests.accepted[0].baseline = 43;
        assert_ne!(record.encode().unwrap(), bytes);
    }
}
#[test]
fn d2c3_schema1_authentic_migration_and_schema2_extras_share_the_same_holder() {
    let context = PersonalContext {
        participant: [2; 32],
        lineage: [3; 16],
        generation: 5,
        transfer: [4; 16],
    };
    let mut old = crate::handover::personal::PersonalRecord::from_maps(
        context,
        [(&(1, -1), &17)],
        [(&MobType::Pig, &37)],
        [(&7, &19)],
        None,
    )
    .unwrap();
    let legacy = old.encode_schema1().unwrap();
    assert_eq!(
        crate::handover::personal::PersonalRecord::declared_schema(&legacy).unwrap(),
        1
    );
    let migrated = decode_schema1(&legacy, context).unwrap();
    assert_eq!(migrated.reputation, old.reputation);
    assert_eq!(migrated.kills, old.kills);
    assert_eq!(migrated.claims, old.claims);
    assert_eq!(migrated.tracking, KillTracking::LegacySnapshot);
    assert!(migrated.quests.is_empty());
    old.revision = 42;
    old.quests.declines = vec![([5; 16], 4999)];
    let current = old.encode().unwrap();
    assert_eq!(
        crate::handover::personal::PersonalRecord::declared_schema(&current).unwrap(),
        2
    );
    let decoded = crate::handover::personal::PersonalRecord::decode(&current, &context).unwrap();
    assert_eq!(decoded.to_gameplay().quests.declines, vec![([5; 16], 4999)]);
    assert_eq!(decoded.to_gameplay().revision, 42);
    assert!(
        old.encode_schema1().is_err(),
        "active timer suffix cannot be dropped into an old member"
    );
}
