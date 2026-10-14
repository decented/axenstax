//! One bounded gameplay record, with explicit world or attachment authority.
//! Imported proof remains a native installation concern; these values are portable.
use crate::{
    mob::MobType, personal_value::PersonalContext, quest::QuestFlavour,
    saved_item_slot::SavedItemSlot,
};
use serde::{Deserialize, Serialize};

pub(crate) mod codec;
pub(crate) type GameplayId = [u8; 16];
pub(crate) const RECORD_LIMIT: usize = 64 * 1024;
pub(crate) const REPUTATION_LIMIT: usize = 2048;
pub(crate) const KILL_KINDS_LIMIT: usize = 256;
pub(crate) const CLAIMS_LIMIT: usize = 4096;
pub(crate) const QUEST_LIMIT: usize = 64;
pub(crate) const DECLINE_LIMIT: usize = 256;
pub(crate) const REWARD_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldPlayer {
    Npub([u8; 32]),
    /// Only a trusted local seat can use this identity; never a hosted join.
    LegacyLocal(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameplayContext {
    Imported(PersonalContext),
    WorldLocal {
        scope: GameplayId,
        player: WorldPlayer,
    },
    Guest {
        session: [u8; 16],
        generation: u32,
    },
}
impl GameplayContext {
    pub(crate) fn validate(self) -> Result<(), String> {
        match self {
            Self::Imported(_) => Ok(()), // exact proof/context checked by its installer
            Self::WorldLocal { scope, player } => {
                if scope == [0; 16]
                    || matches!(player, WorldPlayer::Npub(key) if key == [0; 32])
                    || matches!(player, WorldPlayer::LegacyLocal(seat) if seat >= 4)
                {
                    return Err("invalid world-local gameplay identity".into());
                }
                Ok(())
            }
            Self::Guest { session, .. } if session != [0; 16] => Ok(()),
            Self::Guest { .. } => Err("invalid guest gameplay attachment".into()),
        }
    }
    pub(crate) fn participant(self) -> Option<[u8; 32]> {
        match self {
            Self::Imported(context) => Some(context.participant),
            Self::WorldLocal {
                player: WorldPlayer::Npub(key),
                ..
            } => Some(key),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum GameplayBinding {
    Imported(crate::joiner_save::PersonalBinding),
    WorldLocal {
        scope: GameplayId,
        player: WorldPlayer,
    },
    Guest {
        session: [u8; 16],
        generation: u32,
    },
}
impl GameplayBinding {
    pub(crate) fn context(&self) -> GameplayContext {
        match self {
            Self::Imported(binding) => GameplayContext::Imported(binding.context),
            Self::WorldLocal { scope, player } => GameplayContext::WorldLocal {
                scope: *scope,
                player: *player,
            },
            Self::Guest {
                session,
                generation,
            } => GameplayContext::Guest {
                session: *session,
                generation: *generation,
            },
        }
    }
    pub(crate) fn imported(&self) -> Option<&crate::joiner_save::PersonalBinding> {
        match self {
            Self::Imported(binding) => Some(binding),
            _ => None,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.context().validate()?;
        if self
            .imported()
            .is_some_and(|binding| binding.manifest == [0; 32] || binding.core == [0; 32])
        {
            return Err("invalid imported gameplay proof binding".into());
        }
        Ok(())
    }
    pub(crate) fn persists(&self) -> bool {
        !matches!(self, Self::Guest { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalRestore {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub hotbar: u8,
    pub locks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KillTracking {
    LegacyUntracked,
    Authoritative,
    /// Schema1 supplied actual counters, without claiming earlier absent history.
    LegacySnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcceptedQuest {
    pub id: GameplayId,
    pub npc: GameplayId,
    pub village: (i32, i32),
    pub flavour: QuestFlavour,
    pub rewards: Vec<SavedItemSlot>,
    pub reputation: i16,
    pub baseline: u32,
}
impl AcceptedQuest {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.id == [0; 16] || self.npc == [0; 16] || self.rewards.len() > REWARD_LIMIT {
            return Err("invalid accepted quest identity or rewards".into());
        }
        let count = match self.flavour {
            QuestFlavour::Fetch { count, .. }
            | QuestFlavour::Make { count, .. }
            | QuestFlavour::Kill { count, .. } => count,
        };
        if count == 0 {
            return Err("empty accepted quest target".into());
        }
        for reward in &self.rewards {
            if matches!(
                reward,
                SavedItemSlot::Plan { .. }
                    | SavedItemSlot::Saved(crate::save::SavedSlot::Plan { .. })
            ) {
                return Err("quest rewards cannot carry private Plans".into());
            }
            let stack = reward.restore()?.ok_or("empty quest reward")?;
            if SavedItemSlot::capture_exact(Some(&stack))? != *reward {
                return Err("quest reward did not restore exactly".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct QuestProgress {
    pub accepted: Vec<AcceptedQuest>,
    pub declines: Vec<(GameplayId, u32)>,
    pub penalty_left: u32,
}
impl QuestProgress {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.accepted.len() > QUEST_LIMIT || self.declines.len() > DECLINE_LIMIT {
            return Err("quest progression exceeds its entry limit".into());
        }
        if self.accepted.windows(2).any(|w| w[0].npc >= w[1].npc)
            || self.declines.windows(2).any(|w| w[0].0 >= w[1].0)
            || self.declines.iter().any(|(id, ticks)| {
                *id == [0; 16]
                    || *ticks == 0
                    || *ticks > crate::villager_ui::DECLINE_COOLDOWN_TICKS as u32
            })
            || self.penalty_left > crate::reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS as u32
        {
            return Err("invalid quest progression keys or remaining ticks".into());
        }
        for (at, quest) in self.accepted.iter().enumerate() {
            quest.validate()?;
            if self.accepted[..at].iter().any(|prior| prior.id == quest.id) {
                return Err("duplicate accepted quest identity".into());
            }
        }
        Ok(())
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.accepted.is_empty() && self.declines.is_empty() && self.penalty_left == 0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GameplayRecord {
    pub context: GameplayContext,
    pub revision: u64,
    pub reputation: Vec<((i32, i32), i16)>,
    pub kills: Vec<(MobType, u32)>,
    pub claims: Vec<(u32, u32)>,
    pub local: Option<LocalRestore>,
    pub tracking: KillTracking,
    pub quests: QuestProgress,
}
impl GameplayRecord {
    pub(crate) fn empty(context: GameplayContext) -> Result<Self, String> {
        let value = Self {
            context,
            revision: 0,
            reputation: Vec::new(),
            kills: Vec::new(),
            claims: Vec::new(),
            local: None,
            tracking: KillTracking::Authoritative,
            quests: Default::default(),
        };
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.context.validate()?;
        if self.reputation.len() > REPUTATION_LIMIT
            || self.kills.len() > KILL_KINDS_LIMIT
            || self.claims.len() > CLAIMS_LIMIT
        {
            return Err("gameplay progression exceeds its map limit".into());
        }
        if self.reputation.windows(2).any(|w| w[0].0 >= w[1].0)
            || self.claims.windows(2).any(|w| w[0].0 >= w[1].0)
            || self.reputation.iter().any(|(_, score)| {
                !(crate::reputation::REP_MIN..=crate::reputation::REP_MAX).contains(score)
            })
        {
            return Err("invalid gameplay progression map".into());
        }
        let mut last = None;
        for (kind, _) in &self.kills {
            let wire = bincode::serialize(kind).map_err(|e| e.to_string())?;
            let index =
                u32::from_le_bytes(wire.try_into().map_err(|_| "invalid gameplay mob kind")?);
            if last.is_some_and(|prior| prior >= index) {
                return Err("duplicate gameplay kill kind".into());
            }
            last = Some(index);
        }
        if let Some(local) = &self.local {
            if !local
                .pos
                .iter()
                .chain([&local.yaw, &local.pitch])
                .all(|value| value.is_finite())
                || local.hotbar > 8
                || local.locks & !((1 << 36) - 1) != 0
            {
                return Err("invalid local gameplay restore".into());
            }
        }
        self.quests.validate()
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        codec::encode(self)
    }
    pub(crate) fn decode(bytes: &[u8], expected: GameplayContext) -> Result<Self, String> {
        codec::decode(bytes, expected)
    }
    pub(crate) fn is_default(&self) -> bool {
        self.reputation.iter().all(|(_, score)| *score == 0)
            && self.kills.iter().all(|(_, count)| *count == 0)
            && self.claims.is_empty()
            && self.quests.is_empty()
    }
    pub(crate) fn install_progression(&self, slot: &mut crate::player_slot::PlayerSlot) {
        slot.reputation.per_village.clear();
        slot.reputation
            .per_village
            .extend(self.reputation.iter().copied());
        slot.kill_counter.clear();
        slot.kill_counter.extend(self.kills.iter().copied());
        slot.bounties_claimed.clear();
        slot.bounties_claimed.extend(self.claims.iter().copied());
        slot.quest_progress = self.quests.clone();
        slot.gameplay_revision = self.revision;
        slot.kill_tracking = self.tracking;
    }
    pub(crate) fn from_slot(
        context: GameplayContext,
        slot: &crate::player_slot::PlayerSlot,
    ) -> Result<Self, String> {
        let mut reputation: Vec<_> = slot
            .reputation
            .per_village
            .iter()
            .map(|(key, value)| (*key, *value))
            .collect();
        reputation.sort_by_key(|entry| entry.0);
        let mut kills: Vec<_> = slot
            .kill_counter
            .iter()
            .map(|(key, value)| (*key, *value))
            .collect();
        kills.sort_by_key(|entry| {
            bincode::serialize(&entry.0)
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .map(u32::from_le_bytes)
        });
        let mut claims: Vec<_> = slot
            .bounties_claimed
            .iter()
            .map(|(key, value)| (*key, *value))
            .collect();
        claims.sort_by_key(|entry| entry.0);
        let locks = slot
            .inventory
            .locked_indices()
            .iter()
            .fold(0u64, |bits, index| {
                if *index < 36 {
                    bits | (1 << index)
                } else {
                    bits
                }
            });
        let value = Self {
            context,
            revision: slot.gameplay_revision,
            reputation,
            kills,
            claims,
            local: Some(LocalRestore {
                pos: slot.player.pos.to_array(),
                yaw: slot.camera.yaw,
                pitch: slot.camera.pitch,
                hotbar: u8::try_from(slot.hotbar_slot).map_err(|_| "invalid local hotbar")?,
                locks,
            }),
            tracking: slot.kill_tracking,
            quests: slot.quest_progress.clone(),
        };
        value.validate()?;
        Ok(value)
    }
}

pub(crate) fn encode_extras(record: &GameplayRecord) -> Result<Vec<u8>, String> {
    codec::encode_extras(record)
}
pub(crate) fn decode_extras(bytes: &[u8]) -> Result<(u64, KillTracking, QuestProgress), String> {
    codec::decode_extras(bytes)
}
pub(crate) fn decode_schema1(
    bytes: &[u8],
    expected: PersonalContext,
) -> Result<GameplayRecord, String> {
    codec::decode_schema1(bytes, expected)
}

#[cfg(test)]
mod tests;
