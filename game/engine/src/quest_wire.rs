//! Bounded ordinary quest views and sequential results. No monetary field or
//! private Plan body is present in a quest term or reply.
use crate::{
    action_identity::{ActionFate, ActionHeader},
    action_wire::BoundedList,
    pack_wire::PackWindow,
    progression::GameplayId,
    protocol::WireStack,
    quest::QuestFlavour,
    villager::Profession,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text<const N: usize>(BoundedList<u8, N>);
impl<const N: usize> Text<N> {
    pub(crate) fn new(value: &str) -> Result<Self, String> {
        Ok(Self(
            BoundedList::new(value.as_bytes().to_vec()).map_err(str::to_string)?,
        ))
    }
    pub(crate) fn as_str(&self) -> Result<&str, String> {
        std::str::from_utf8(self.0.as_slice()).map_err(|_| "Quest text is not UTF-8".into())
    }
    fn owned_bytes(&self) -> usize {
        self.0.allocated_bytes()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestOperation {
    Talk,
    Accept { id: GameplayId },
    Decline { id: GameplayId },
    TurnIn { id: GameplayId },
    Close,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestRequest {
    pub actor_respawns: u32,
    pub target: u32,
    pub before: u32,
    /// Zero on Talk; every subsequent action names its actual open view.
    pub incarnation: u64,
    pub operation: QuestOperation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestTerms {
    pub id: GameplayId,
    pub flavour: QuestFlavour,
    pub rewards: BoundedList<WireStack, 16>,
    pub reputation: i16,
}
impl QuestTerms {
    pub(crate) fn capture(quest: &crate::progression::AcceptedQuest) -> Result<Self, String> {
        quest.validate()?;
        let rewards =
            crate::quest_play::reward_items(quest).map_err(|_| "Quest rewards are invalid")?;
        Ok(Self {
            id: quest.id,
            flavour: quest.flavour,
            rewards: BoundedList::new(
                rewards
                    .iter()
                    .map(crate::inventory::stack_to_wire)
                    .collect(),
            )
            .map_err(str::to_string)?,
            reputation: quest.reputation,
        })
    }
    pub(crate) fn native(
        &self,
        registry: &crate::block::BlockRegistry,
    ) -> Result<crate::quest::Quest, String> {
        let mut items = Vec::with_capacity(self.rewards.as_slice().len());
        for wire in self.rewards.as_slice() {
            if wire.item_kind == crate::protocol::item_kind::PLAN {
                return Err("A quest reward contains a Plan".into());
            }
            let stack = crate::inventory::stack_from_wire(
                wire,
                registry,
                crate::inventory::PlanDecode::Marker,
            )
            .ok_or("A quest reward does not decode")?;
            if crate::inventory::stack_to_wire(&stack) != *wire {
                return Err("A quest reward does not round-trip exactly".into());
            }
            items.push(stack);
        }
        Ok(crate::quest::Quest {
            flavour: self.flavour,
            reward: crate::quest::QuestReward {
                items,
                sats: 0,
                reputation: self.reputation,
            },
            accepted_by: None,
        })
    }
    fn validate(&self) -> Result<(), String> {
        let count = match self.flavour {
            QuestFlavour::Fetch { count, .. }
            | QuestFlavour::Make { count, .. }
            | QuestFlavour::Kill { count, .. } => count,
        };
        if self.id == [0; 16]
            || count == 0
            || self
                .rewards
                .as_slice()
                .iter()
                .any(|wire| wire.item_kind == crate::protocol::item_kind::PLAN || wire.count == 0)
        {
            return Err("Invalid ordinary quest terms".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestStatus {
    NoQuest,
    Offer,
    Accepted { progress: u32, target: u32 },
    Cooldown { remaining: u32 },
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestView {
    pub target: u32,
    pub npc: GameplayId,
    pub incarnation: u64,
    pub revision: u64,
    pub label: Text<128>,
    pub profession: Profession,
    pub gossip: Option<Text<512>>,
    pub terms: Option<QuestTerms>,
    pub status: QuestStatus,
    pub reputation: i16,
}
impl QuestView {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.label.as_str()?;
        if let Some(gossip) = &self.gossip {
            gossip.as_str()?;
        }
        if self.incarnation == 0
            || self.profession == Profession::Builder
            || !(crate::reputation::REP_MIN..=crate::reputation::REP_MAX).contains(&self.reputation)
        {
            return Err("Invalid ordinary quest view".into());
        }
        if let Some(terms) = &self.terms {
            terms.validate()?;
        }
        match self.status {
            QuestStatus::Offer if self.npc != [0; 16] && self.terms.is_some() => (),
            QuestStatus::Accepted { progress, target }
                if self.npc != [0; 16]
                    && target != 0
                    && progress <= target
                    && self.terms.as_ref().is_some_and(|terms| {
                        let count = match terms.flavour {
                            QuestFlavour::Fetch { count, .. }
                            | QuestFlavour::Make { count, .. }
                            | QuestFlavour::Kill { count, .. } => count,
                        };
                        u32::from(count) == target
                    }) =>
            {
                ()
            }
            QuestStatus::Cooldown { remaining }
                if self.npc != [0; 16]
                    && self.terms.is_none()
                    && remaining > 0
                    && remaining <= crate::villager_ui::DECLINE_COOLDOWN_TICKS as u32 =>
            {
                ()
            }
            QuestStatus::NoQuest | QuestStatus::Unavailable if self.terms.is_none() => (),
            _ => return Err("Quest status differs from its bounded terms".into()),
        }
        Ok(())
    }
    pub(crate) fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.label.owned_bytes())
            .saturating_add(self.gossip.as_ref().map_or(0, Text::owned_bytes))
            .saturating_add(
                self.terms
                    .as_ref()
                    .map_or(0, |terms| terms.rewards.allocated_bytes()),
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestNote {
    Done,
    Updated,
    OutOfReach,
    Denied,
    Dead,
    NoGiver,
    NoOffer,
    Cooldown,
    AlreadyAccepted,
    MissingItems,
    IncompleteKills,
    Untracked,
    Waiting,
    Completed,
}
impl QuestNote {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Done => "",
            Self::Updated => "Your quest or items changed; try again.",
            Self::OutOfReach => "Move closer to the villager.",
            Self::Denied => "Quest use is restricted here.",
            Self::Dead => "Quests are waiting until you return.",
            Self::NoGiver => "The original quest giver is unavailable. Your quest is kept.",
            Self::NoOffer => "There is no current quest offer.",
            Self::Cooldown => "This villager needs a little more time.",
            Self::AlreadyAccepted => "You already have this villager's quest.",
            Self::MissingItems => "Bring the required items in your bag.",
            Self::IncompleteKills => "The required kills are not complete yet.",
            Self::Untracked => "This quest's kill progress cannot be verified.",
            Self::Waiting => "The quest is waiting; try again.",
            Self::Completed => "Quest complete.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestResult {
    pub actor_respawns: u32,
    pub action: ActionHeader,
    pub fate: ActionFate,
    pub note: QuestNote,
    /// Cost-only canonical state. Genuine reward Grants follow this event.
    pub window_event: u32,
    /// The LAST progression fence, also named by the action receipt.
    pub included_events: u32,
    pub window: PackWindow,
    pub view: Option<QuestView>,
}
impl QuestResult {
    fn validate(&self) -> Result<(), String> {
        if self.window_event == 0
            || self.included_events <= self.window_event
            || self.included_events.saturating_sub(self.window_event)
                > crate::progression::REWARD_LIMIT as u32 + 1
        {
            return Err("Quest result has an invalid complete fence".into());
        }
        if let Some(view) = &self.view {
            view.validate()?;
        }
        Ok(())
    }
    pub(crate) fn encode(&self) -> Result<Box<[u8]>, String> {
        self.validate()?;
        let bytes = bincode::serialize(self).map_err(|error| error.to_string())?;
        if bytes.len() > crate::action_journal::MAX_PAGE_BYTES {
            return Err("Quest result exceeds its carrier bound".into());
        }
        Ok(bytes.into_boxed_slice())
    }
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > crate::action_journal::MAX_PAGE_BYTES {
            return Err("Quest result exceeds its carrier bound".into());
        }
        let value: Self =
            crate::protocol::safe_deserialize(bytes).map_err(|error| error.to_string())?;
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.window.slots.allocated_bytes())
            .saturating_add(self.window.owed.allocated_bytes())
            .saturating_add(self.view.as_ref().map_or(0, QuestView::owned_bytes))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestViewPush {
    pub actor_respawns: u32,
    pub session: [u8; 16],
    pub epoch: u32,
    pub life: u32,
    pub window_event: u32,
    pub target: u32,
    pub incarnation: u64,
    pub view: Option<QuestView>,
}
impl QuestViewPush {
    pub(crate) fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.view.as_ref().map_or(0, QuestView::owned_bytes))
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.window_event == 0 || self.incarnation == 0 {
            return Err("Quest view has no numbered identity".into());
        }
        if let Some(view) = &self.view {
            view.validate()?;
            if view.target != self.target || view.incarnation != self.incarnation {
                return Err("Quest push differs from its open view".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "quest_wire_tests.rs"]
mod tests;
