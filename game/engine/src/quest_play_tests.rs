use super::*;
use crate::{
    item::{Item, MaterialId},
    progression::{GameplayContext, WorldPlayer},
    window_events,
};

fn record() -> GameplayRecord {
    GameplayRecord::empty(GameplayContext::WorldLocal {
        scope: [1; 16],
        player: WorldPlayer::Npub([2; 32]),
    })
    .unwrap()
}
fn offer(flavour: QuestFlavour) -> Offer {
    Offer {
        quest: AcceptedQuest {
            id: [3; 16],
            npc: [4; 16],
            village: (2, -4),
            flavour,
            rewards: vec![
                SavedItemSlot::capture_exact(Some(&ItemStack::new_material(MaterialId::Bread, 2)))
                    .unwrap(),
            ],
            reputation: 12,
            baseline: 0,
        },
    }
}
fn raw(sp: &crate::server::ServerPlayer) -> MarkerWindow {
    window_events::pack_raw_marker(sp, &crate::block::BlockRegistry::new()).unwrap()
}

#[test]
fn d2c3_quest_cost_uses_ordered_bag_only_and_shared_fetch_make_rule() {
    let mut sp = crate::server::ServerPlayer::new(glam::Vec3::ZERO);
    sp.inventory
        .set_slot(1, Some(ItemStack::new_material(MaterialId::Wheat, 2)));
    sp.inventory
        .set_slot(9, Some(ItemStack::new_material(MaterialId::Wheat, 5)));
    sp.cursor = Some(ItemStack::new_material(MaterialId::Wheat, 64));
    sp.craft_grid[0][0] = Some(ItemStack::new_material(MaterialId::Wheat, 64));
    for flavour in [
        QuestFlavour::Fetch {
            item: MaterialId::Wheat,
            count: 6,
        },
        QuestFlavour::Make {
            item: MaterialId::Wheat,
            count: 6,
        },
    ] {
        let staged = stage_cost(&raw(&sp), flavour).unwrap();
        let slots = staged.after.slots();
        assert!(slots[1].is_none());
        assert_eq!(slots[9].as_ref().unwrap().count, 1);
        assert_eq!(staged.sources[1].as_ref().unwrap().count, 2);
        assert_eq!(staged.sources[9].as_ref().unwrap().count, 4);
        assert!(staged.sources[36..].iter().all(Option::is_none));
        assert_eq!(slots[40].as_ref().unwrap().count, 64);
        assert_eq!(slots[41].as_ref().unwrap().count, 64);
    }
    assert!(matches!(
        stage_cost(
            &raw(&sp),
            QuestFlavour::Fetch {
                item: MaterialId::Wheat,
                count: 8
            }
        ),
        Err(Refusal::MissingItems)
    ));
    assert_eq!(sp.inventory.slot(1).unwrap().count, 2);
    assert_eq!(sp.inventory.slot(9).unwrap().count, 5);
}

#[test]
fn d2c3_quest_cost_reuses_both_mode_prefix_source_backing_without_shortfall() {
    for mode in [
        crate::action_identity::InventoryMode::Mirror,
        crate::action_identity::InventoryMode::Strict,
    ] {
        let mut sp = crate::server::ServerPlayer::new(glam::Vec3::ZERO);
        sp.inventory_mode = mode;
        sp.inventory
            .set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 5)));
        let marker = raw(&sp);
        let staged = stage_cost(
            &marker,
            QuestFlavour::Fetch {
                item: MaterialId::Wheat,
                count: 3,
            },
        )
        .unwrap();
        window_events::queue(
            &mut sp,
            window_events::WindowEvent::Take {
                slot: 0,
                item: Item::Material(MaterialId::Wheat),
                n: 3,
                how: "quest prefix test",
            },
            0,
        );
        let registry = crate::block::BlockRegistry::new();
        assert!(
            window_events::admit_pack_staged(&sp, &registry, &marker, &staged).is_err(),
            "{mode:?}: a believed shortfall became quest payment"
        );
        assert_eq!(
            sp.inventory.slot(0).unwrap().count,
            5,
            "admission changed the real source"
        );
        let mut valid = crate::server::ServerPlayer::new(glam::Vec3::ZERO);
        valid.inventory_mode = mode;
        valid
            .inventory
            .set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 5)));
        let marker = raw(&valid);
        let staged = stage_cost(
            &marker,
            QuestFlavour::Fetch {
                item: MaterialId::Wheat,
                count: 3,
            },
        )
        .unwrap();
        window_events::queue(
            &mut valid,
            window_events::WindowEvent::Take {
                slot: 0,
                item: Item::Material(MaterialId::Wheat),
                n: 1,
                how: "quest prefix test",
            },
            0,
        );
        let admitted =
            window_events::admit_pack_staged(&valid, &registry, &marker, &staged).unwrap();
        assert_eq!(admitted.window.inv.slot(0).unwrap().count, 1);
        assert!(admitted.debt.is_empty());
        assert_eq!(valid.inventory.slot(0).unwrap().count, 5);
    }
}

#[test]
fn d2c3_quest_kill_acceptance_freezes_current_baseline_and_completion_is_once_only() {
    let offer = offer(QuestFlavour::Kill {
        mob: crate::mob::MobType::Cow,
        count: 2,
    });
    let mut before = record();
    before.kills = vec![(crate::mob::MobType::Cow, 12)];
    let mut accepted = accept(&before, &offer).unwrap();
    assert_eq!(accepted.quests.accepted[0].baseline, 12);
    assert!(before.quests.accepted.is_empty());
    let sp = crate::server::ServerPlayer::new(glam::Vec3::ZERO);
    accepted.kills[0].1 = 13;
    assert!(matches!(
        turn_in(&accepted, offer.quest.npc, offer.quest.id, &raw(&sp)),
        Err(Refusal::IncompleteKills)
    ));
    accepted.kills[0].1 = 14;
    let completion = turn_in(&accepted, offer.quest.npc, offer.quest.id, &raw(&sp)).unwrap();
    assert!(completion.record.quests.accepted.is_empty());
    assert_eq!(completion.record.reputation, vec![((2, -4), 12)]);
    assert_eq!(
        completion.rewards,
        vec![ItemStack::new_material(MaterialId::Bread, 2)]
    );
    assert!(completion.cost.sources.iter().all(Option::is_none));
    assert!(matches!(
        turn_in(
            &completion.record,
            offer.quest.npc,
            offer.quest.id,
            &raw(&sp)
        ),
        Err(Refusal::Changed)
    ));
    assert!(matches!(
        turn_in(&accepted, [5; 16], offer.quest.id, &raw(&sp)),
        Err(Refusal::Changed)
    ));
    assert!(matches!(
        turn_in(&accepted, offer.quest.npc, [6; 16], &raw(&sp)),
        Err(Refusal::Changed)
    ));
    assert_eq!(accepted.quests.accepted.len(), 1);
    before.tracking = KillTracking::LegacyUntracked;
    assert!(matches!(accept(&before, &offer), Err(Refusal::Untracked)));
}

#[test]
fn d2c3_quest_decline_expiry_projects_all_timers_without_resetting_record_or_offline_time() {
    let offer = offer(QuestFlavour::Fetch {
        item: MaterialId::Wheat,
        count: 3,
    });
    let first = decline(&record(), &offer).unwrap();
    assert!(matches!(decline(&first, &offer), Err(Refusal::Cooldown)));
    let mut first = first;
    first.quests.penalty_left = 500;
    let original = first.encode().unwrap();
    let (mut remaining, expired) = project_timers(&first, 100);
    assert!(!expired);
    assert_eq!(remaining.quests.declines[0].1, 5900);
    assert_eq!(remaining.quests.penalty_left, 400);
    assert_eq!(
        first.encode().unwrap(),
        original,
        "pure clock projection rewrote the live authority"
    );
    remaining = GameplayRecord::decode(&remaining.encode().unwrap(), &remaining.context).unwrap();
    let (restored, _) = project_timers(&remaining, 0);
    assert_eq!(
        restored, remaining,
        "offline duration advanced restored gameplay timers"
    );
    let (expired, changed) = project_timers(&remaining, 5900);
    assert!(changed);
    assert!(expired.quests.declines.is_empty());
    assert_eq!(expired.quests.penalty_left, 0);
    assert!(accept(&expired, &offer).is_ok());
}

#[test]
fn d2c3_quest_frozen_rewards_have_zero_money_and_no_new_item_tier_multiplier() {
    for profession in [
        Profession::Farmer,
        Profession::Cook,
        Profession::Blacksmith,
        Profession::Scribe,
        Profession::Carpenter,
        Profession::Miller,
        Profession::Baker,
        Profession::Brewer,
    ] {
        for seed in 0..12 {
            let generated = crate::quest::generate_quest(profession, seed).unwrap();
            let offer = Offer::generate([4; 16], (2, -4), profession, seed)
                .unwrap()
                .unwrap();
            let restored = as_native(&offer.quest).unwrap();
            assert_eq!(restored.flavour, generated.flavour);
            assert_eq!(restored.reward.items, generated.reward.items);
            assert_eq!(restored.reward.reputation, generated.reward.reputation);
            assert_eq!(restored.reward.sats, 0);
            assert!(!crate::quest::summarise(&restored, true).contains("sats"));
        }
    }
    assert!(matches!(
        Offer::generate([4; 16], (2, -4), Profession::Builder, 0),
        Err(Refusal::Changed)
    ));
}
