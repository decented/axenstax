//! Allocation-capable preparation followed by one allocation-free map install.
//! Authority/codec validation belongs to the caller; no physical place or
//! binding is owned here, and a later install cannot repair incomplete credit.
use super::{GameplayRecord, KillTracking, QuestProgress};
use crate::{mob::MobType, player_slot::PlayerSlot};
use ahash::AHashMap;

pub(crate) struct PreparedProgressionInstall {
    reputation: AHashMap<(i32, i32), i16>,
    kills: AHashMap<MobType, u32>,
    claims: AHashMap<u32, u32>,
    quests: QuestProgress,
    revision: u64,
    tracking: KillTracking,
}
fn prepare_map<K: Eq + std::hash::Hash, V>(values: Vec<(K, V)>) -> Result<AHashMap<K, V>, String> {
    let mut map = AHashMap::new();
    map.try_reserve(values.len()).map_err(|_| "Gameplay map allocation failed")?;
    map.extend(values);
    Ok(map)
}
fn map_bytes<K, V>(map: &AHashMap<K, V>) -> usize {
    if map.capacity() == 0 {
        return 0;
    }
    // Charge conservatively for the table's load factor/control bytes as
    // well as its actual reserved capacity, rather than only occupied keys.
    let buckets = map.capacity().checked_next_power_of_two().unwrap_or(usize::MAX);
    buckets
        .saturating_mul(std::mem::size_of::<(K, V)>().saturating_add(1))
        .saturating_add(16)
        .saturating_add(std::mem::align_of::<(K, V)>())
}
fn quest_bytes(quests: &QuestProgress) -> usize {
    quests
        .accepted
        .capacity()
        .saturating_mul(std::mem::size_of::<super::AcceptedQuest>())
        .saturating_add(
            quests
                .accepted
                .iter()
                .map(|q| {
                    q.rewards.capacity().saturating_mul(std::mem::size_of::<crate::saved_item_slot::SavedItemSlot>())
                })
                .fold(0, usize::saturating_add),
        )
        .saturating_add(quests.declines.capacity().saturating_mul(std::mem::size_of::<(super::GameplayId, u32)>()))
}
impl PreparedProgressionInstall {
    pub(crate) fn prepare(record: GameplayRecord) -> Result<Self, String> {
        let GameplayRecord { reputation, kills, claims, quests, revision, tracking, .. } = record;
        Ok(Self {
            reputation: prepare_map(reputation)?,
            kills: prepare_map(kills)?,
            claims: prepare_map(claims)?,
            quests,
            revision,
            tracking,
        })
    }
    pub(crate) fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(map_bytes(&self.reputation))
            .saturating_add(map_bytes(&self.kills))
            .saturating_add(map_bytes(&self.claims))
            .saturating_add(quest_bytes(&self.quests))
    }
    /// Temporary decoded/clone storage overlaps the old holder and prepared maps.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn record_owned_bytes(record: &GameplayRecord) -> usize {
        std::mem::size_of::<GameplayRecord>()
            .saturating_add(record.reputation.capacity().saturating_mul(std::mem::size_of::<((i32, i32), i16)>()))
            .saturating_add(record.kills.capacity().saturating_mul(std::mem::size_of::<(MobType, u32)>()))
            .saturating_add(record.claims.capacity().saturating_mul(std::mem::size_of::<(u32, u32)>()))
            .saturating_add(quest_bytes(&record.quests))
    }
    pub(crate) fn slot_owned_bytes(slot: &PlayerSlot) -> usize {
        map_bytes(&slot.reputation.per_village)
            .saturating_add(map_bytes(&slot.kill_counter))
            .saturating_add(map_bytes(&slot.bounties_claimed))
            .saturating_add(quest_bytes(&slot.quest_progress))
    }
    /// No decode, map insertion, quest clone, reserve or external callback.
    pub(crate) fn commit(self, slot: &mut PlayerSlot) {
        slot.reputation.per_village = self.reputation;
        slot.kill_counter = self.kills;
        slot.bounties_claimed = self.claims;
        slot.quest_progress = self.quests;
        slot.gameplay_clock = Default::default();
        slot.gameplay_revision = self.revision;
        slot.kill_tracking = self.tracking;
    }
}

#[cfg(test)]
mod tests;
