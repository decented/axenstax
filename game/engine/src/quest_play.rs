//! Ordinary portable quests. Pure candidates share the native material rule;
//! callers reserve their complete backed events before committing a candidate.
use crate::{
    item::ItemStack,
    progression::{AcceptedQuest, GameplayId, GameplayRecord, KillTracking},
    quest::{Quest, QuestFlavour, QuestReward},
    saved_item_slot::SavedItemSlot,
    staged_window::{MarkerWindow, StagedClick},
    villager::Profession,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Changed,
    Cooldown,
    Accepted,
    MissingItems,
    IncompleteKills,
    Untracked,
    Capacity,
}

#[derive(Clone, Debug)]
pub(crate) struct Offer {
    pub quest: AcceptedQuest,
}

impl Offer {
    pub(crate) fn generate(
        npc: GameplayId,
        village: (i32, i32),
        profession: Profession,
        seed: u32,
    ) -> Result<Option<Self>, Refusal> {
        if profession == Profession::Builder {
            return Err(Refusal::Changed);
        }
        let Some(quest) = crate::quest::generate_quest(profession, seed) else {
            return Ok(None);
        };
        // Keep the existing item amounts, without a new tier multiplier.
        // Monetary rewards never enter the joined portable quest domain.
        let rewards = quest
            .reward
            .items
            .iter()
            .map(|stack| SavedItemSlot::capture_exact(Some(stack)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Refusal::Capacity)?;
        let accepted = AcceptedQuest {
            id: crate::quest_npcs::random_id().map_err(|_| Refusal::Capacity)?,
            npc,
            village,
            flavour: quest.flavour,
            rewards,
            reputation: quest.reward.reputation,
            baseline: 0,
        };
        accepted.validate().map_err(|_| Refusal::Capacity)?;
        Ok(Some(Self { quest: accepted }))
    }
}

pub(crate) fn as_native(accepted: &AcceptedQuest) -> Result<Quest, Refusal> {
    accepted.validate().map_err(|_| Refusal::Changed)?;
    Ok(Quest {
        flavour: accepted.flavour,
        reward: QuestReward {
            items: reward_items(accepted)?,
            sats: 0,
            reputation: accepted.reputation,
        },
        accepted_by: None,
    })
}

pub(crate) fn reward_items(accepted: &AcceptedQuest) -> Result<Vec<ItemStack>, Refusal> {
    accepted
        .rewards
        .iter()
        .map(|slot| {
            slot.restore()
                .map_err(|_| Refusal::Changed)?
                .ok_or(Refusal::Changed)
        })
        .collect()
}

fn revise(mut record: GameplayRecord) -> Result<GameplayRecord, Refusal> {
    record.revision = record.revision.checked_add(1).ok_or(Refusal::Capacity)?;
    record.validate().map_err(|_| Refusal::Capacity)?;
    record.encode().map_err(|_| Refusal::Capacity)?;
    Ok(record)
}

pub(crate) fn accept(record: &GameplayRecord, offer: &Offer) -> Result<GameplayRecord, Refusal> {
    record.validate().map_err(|_| Refusal::Changed)?;
    if record
        .quests
        .accepted
        .iter()
        .any(|quest| quest.npc == offer.quest.npc)
    {
        return Err(Refusal::Accepted);
    }
    if record
        .quests
        .declines
        .iter()
        .any(|(npc, _)| *npc == offer.quest.npc)
    {
        return Err(Refusal::Cooldown);
    }
    let mut accepted = offer.quest.clone();
    if let QuestFlavour::Kill { mob, .. } = accepted.flavour {
        if record.tracking == KillTracking::LegacyUntracked {
            return Err(Refusal::Untracked);
        }
        accepted.baseline = record
            .kills
            .iter()
            .find(|(kind, _)| *kind == mob)
            .map_or(0, |(_, count)| *count);
    }
    let mut next = record.clone();
    next.quests.accepted.push(accepted);
    next.quests.accepted.sort_by_key(|quest| quest.npc);
    revise(next)
}

pub(crate) fn decline(record: &GameplayRecord, offer: &Offer) -> Result<GameplayRecord, Refusal> {
    record.validate().map_err(|_| Refusal::Changed)?;
    if record
        .quests
        .accepted
        .iter()
        .any(|quest| quest.npc == offer.quest.npc)
    {
        return Err(Refusal::Accepted);
    }
    if record
        .quests
        .declines
        .iter()
        .any(|(npc, _)| *npc == offer.quest.npc)
    {
        return Err(Refusal::Cooldown);
    }
    let mut next = record.clone();
    match next
        .quests
        .declines
        .binary_search_by_key(&offer.quest.npc, |(npc, _)| *npc)
    {
        Ok(at) => next.quests.declines[at].1 = crate::villager_ui::DECLINE_COOLDOWN_TICKS as u32,
        Err(at) => next.quests.declines.insert(
            at,
            (
                offer.quest.npc,
                crate::villager_ui::DECLINE_COOLDOWN_TICKS as u32,
            ),
        ),
    }
    revise(next)
}

pub(crate) fn kill_progress(
    record: &GameplayRecord,
    quest: &AcceptedQuest,
) -> Result<u32, Refusal> {
    let QuestFlavour::Kill { mob, .. } = quest.flavour else {
        return Ok(0);
    };
    if record.tracking == KillTracking::LegacyUntracked {
        return Err(Refusal::Untracked);
    }
    Ok(record
        .kills
        .iter()
        .find(|(kind, _)| *kind == mob)
        .map_or(0, |(_, count)| *count)
        .saturating_sub(quest.baseline))
}

/// Spend only the existing ordered 36-place material rule. Cursor, grid,
/// armour and rewards never supply a cost. Prefix/source admission is external.
pub(crate) fn stage_cost(
    raw: &MarkerWindow,
    flavour: QuestFlavour,
) -> Result<StagedClick, Refusal> {
    let mut staged = StagedClick {
        after: raw.clone(),
        result: crate::window::ClickResult::Done,
        sources: std::array::from_fn(|_| None),
    };
    let quest = Quest {
        flavour,
        reward: QuestReward::default(),
        accepted_by: None,
    };
    if !crate::quest::fetch_make_consume(&quest, staged.after.view().inv) {
        return Err(Refusal::MissingItems);
    }
    let before = raw.slots();
    let after = staged.after.slots();
    for at in 0..36 {
        let Some(old) = &before[at] else { continue };
        let remaining = after[at].as_ref().map_or(0, |stack| stack.count);
        if old.count > remaining {
            let mut source = old.clone();
            source.count -= remaining;
            staged.sources[at] = Some(source);
        }
    }
    Ok(staged)
}

pub(crate) struct Completion {
    pub record: GameplayRecord,
    pub cost: StagedClick,
    pub rewards: Vec<ItemStack>,
}

pub(crate) fn turn_in(
    record: &GameplayRecord,
    npc: GameplayId,
    id: GameplayId,
    raw: &MarkerWindow,
) -> Result<Completion, Refusal> {
    record.validate().map_err(|_| Refusal::Changed)?;
    let at = record
        .quests
        .accepted
        .iter()
        .position(|quest| quest.npc == npc && quest.id == id)
        .ok_or(Refusal::Changed)?;
    let accepted = &record.quests.accepted[at];
    let quest = as_native(accepted)?;
    let kills = kill_progress(record, accepted)?;
    if !crate::quest::may_turn_in(&quest, raw.clone().view().inv, kills) {
        return Err(if matches!(quest.flavour, QuestFlavour::Kill { .. }) {
            Refusal::IncompleteKills
        } else {
            Refusal::MissingItems
        });
    }
    let cost = stage_cost(raw, accepted.flavour)?;
    let rewards = reward_items(accepted)?;
    let mut next = record.clone();
    next.quests.accepted.remove(at);
    let mut reputation = crate::reputation::Reputation::default();
    reputation.per_village = next.reputation.iter().copied().collect();
    reputation.adjust(accepted.village, accepted.reputation);
    next.reputation = reputation.per_village.into_iter().collect();
    next.reputation.sort_by_key(|(village, _)| *village);
    Ok(Completion {
        record: revise(next)?,
        cost,
        rewards,
    })
}

/// A runtime clock projects the SAME record, without per-tick byte/fence
/// churn. Callers re-anchor when they commit this projection or restore it.
pub(crate) fn project_timers(record: &GameplayRecord, elapsed: u64) -> (GameplayRecord, bool) {
    let mut next = record.clone();
    let elapsed = elapsed.min(u32::MAX as u64) as u32;
    let before = next.quests.declines.len();
    for (_, ticks) in &mut next.quests.declines {
        *ticks = ticks.saturating_sub(elapsed);
    }
    next.quests.declines.retain(|(_, ticks)| *ticks > 0);
    let penalty_expired = next.quests.penalty_left > 0 && next.quests.penalty_left <= elapsed;
    next.quests.penalty_left = next.quests.penalty_left.saturating_sub(elapsed);
    let expired = before != next.quests.declines.len() || penalty_expired;
    (next, expired)
}

#[cfg(test)]
#[path = "quest_play_tests.rs"]
mod tests;
