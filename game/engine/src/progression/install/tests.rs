use super::*;
use crate::{
    progression::{AcceptedQuest, GameplayContext, GameplayRecord, KillTracking},
    quest::QuestFlavour,
};

fn candidate(slot: &PlayerSlot) -> GameplayRecord {
    let mut record =
        GameplayRecord::from_slot(GameplayContext::Guest { session: [1; 16], generation: 1 }, slot).unwrap();
    // Numerical/map fixtures test installation, not earned progression.
    record.reputation = vec![((2, 3), -25)];
    record.kills = vec![(MobType::Cow, 3)];
    record.claims = vec![(1, 2)];
    record.revision = 10;
    record.tracking = KillTracking::Authoritative;
    record.quests.accepted.push(AcceptedQuest {
        id: [7; 16],
        npc: [8; 16],
        village: (2, 3),
        flavour: QuestFlavour::Kill { mob: MobType::Cow, count: 4 },
        rewards: vec![],
        reputation: 8,
        baseline: 2,
    });
    record.quests.declines.push(([9; 16], 30));
    record.quests.penalty_left = 20;
    record.validate().unwrap();
    record
}
#[test]
fn d2c3_prepared_progression_does_not_mutate_until_commit_or_touch_physical_places() {
    let mut slot = PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    slot.inventory.set_slot(1, Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Stick, 3)));
    slot.gameplay_clock.committed(7);
    slot.mark_gameplay_incomplete();
    let clock = slot.gameplay_clock;
    let binding = slot.personal_binding.clone();
    let physical = bincode::serialize(&slot.inventory.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>()).unwrap();
    let places = (
        &slot.inventory as *const _ as usize,
        &slot.armour_slots as *const _ as usize,
        &slot.crafting_ui.cursor_item as *const _ as usize,
        &slot.crafting_ui.grid as *const _ as usize,
    );
    let record = candidate(&slot);
    let prepared = PreparedProgressionInstall::prepare(record.clone()).unwrap();
    assert!(prepared.owned_bytes() > std::mem::size_of::<PreparedProgressionInstall>());
    assert_eq!(slot.gameplay_revision, 0);
    assert_eq!(slot.gameplay_clock, clock);
    assert!(slot.kill_counter.is_empty() && slot.quest_progress.is_empty());
    prepared.commit(&mut slot);
    assert_eq!(slot.reputation.per_village.get(&(2, 3)), Some(&-25));
    assert_eq!(slot.kill_counter.get(&MobType::Cow), Some(&3));
    assert_eq!(slot.bounties_claimed.get(&1), Some(&2));
    assert_eq!(slot.quest_progress, record.quests);
    assert_eq!(slot.gameplay_revision, 10);
    assert_eq!(slot.gameplay_clock, Default::default());
    assert_eq!(slot.personal_binding, binding);
    assert!(!slot.gameplay_carry_complete(), "installation cannot repair an incomplete current holder");
    assert_eq!(
        bincode::serialize(&slot.inventory.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>()).unwrap(),
        physical
    );
    assert_eq!(
        (
            &slot.inventory as *const _ as usize,
            &slot.armour_slots as *const _ as usize,
            &slot.crafting_ui.cursor_item as *const _ as usize,
            &slot.crafting_ui.grid as *const _ as usize
        ),
        places
    );
}
#[test]
fn d2c3_prepared_progression_accounts_for_retained_quest_capacity_and_abandons_without_install() {
    let slot = PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
    let mut record = candidate(&slot);
    let ordinary = PreparedProgressionInstall::prepare(record.clone()).unwrap().owned_bytes();
    record.quests.declines.reserve_exact(100);
    let prepared = PreparedProgressionInstall::prepare(record).unwrap();
    assert!(prepared.owned_bytes() >= ordinary + 99 * std::mem::size_of::<(crate::progression::GameplayId, u32)>());
    drop(prepared);
    assert!(slot.kill_counter.is_empty() && slot.quest_progress.is_empty());
    assert_eq!(slot.gameplay_revision, 0);
}
