//! Typed action and recovery carriers. Movement never supplies an action ACK.

use serde::{Deserialize, Deserializer, Serialize};
use crate::action_identity::{ActionFate, ActionHeader, RecoveryCut};
use crate::action_journal::{MAX_CONTEXT_CELLS, MAX_INTENT_BYTES, MAX_PAGE_BYTES};
use crate::protocol;
use crate::rebase_assembly::RebaseManifest;

/// One logical action. An intervening non-edit action splits an edit batch.
/// Existing request counters identify UI feedback only, never retirement.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActionIntent {
    Edit {
        source_tick: u64,
        change: protocol::BlockChange,
        hand: protocol::EditHand,
        mined: Option<protocol::MinedBlock>,
        use_tag: Option<protocol::UseTag>,
    },
    Window(protocol::WindowOpPacket),
    Attack(protocol::EntityAttackPacket),
    Interact(protocol::EntityInteractPacket),
    Item(protocol::ItemActionPacket),
    Device { cell: [i32; 3], hotbar_slot: u8 },
    /// Only the bottom is asserted; the server derives both halves atomically.
    DoorPlace { bottom: [i32; 3], facing: u8, hotbar_slot: u8 },
    ToggleBlock { cell: [i32; 3], expected_block: u16 },
    HoldBegin { target: [i32; 3], hotbar_slot: u8 },
    HoldCancel { hold: ActionHeader },
    HoldStrike { hold: ActionHeader },
    /// A retained semantic intent invalidated by the authoritative base.
    CancelledNoop,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionPacket {
    pub action: ActionHeader,
    pub intent: ActionIntent,
}

impl ActionPacket {
    /// The same bounded encoding is retained by the journal and server queue.
    pub fn encode(&self) -> Result<Box<[u8]>, String> {
        let bytes = bincode::serialize(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_INTENT_BYTES { return Err("Action exceeds its byte limit".into()); }
        Ok(bytes.into_boxed_slice())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_INTENT_BYTES { return Err("Action exceeds its byte limit".into()); }
        protocol::safe_deserialize(bytes).map_err(|e| e.to_string())
    }
}

/// This E is authoritative included state, never the client's claimed E.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub action: ActionHeader,
    pub fate: ActionFate,
    pub included_events: u32,
}

/// Count-bounded owned data; rejects a hostile length before allocating it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BoundedList<T, const N: usize>(Vec<T>);

impl<T, const N: usize> BoundedList<T, N> {
    pub fn new(values: Vec<T>) -> Result<Self, &'static str> {
        if values.len() > N { return Err("List exceeds its count limit"); }
        Ok(Self(values))
    }
    pub fn as_slice(&self) -> &[T] { &self.0 }
    pub fn into_vec(self) -> Vec<T> { self.0 }
}

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for BoundedList<T, N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor<T, const N: usize>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Visitor<T, N> {
            type Value = BoundedList<T, N>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "at most {N} values") }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                if seq.size_hint().is_some_and(|n| n > N) { return Err(serde::de::Error::custom("List exceeds its count limit")); }
                let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
                while let Some(value) = seq.next_element()? {
                    if values.len() == N { return Err(serde::de::Error::custom("List exceeds its count limit")); }
                    values.push(value);
                }
                Ok(BoundedList(values))
            }
        }
        deserializer.deserialize_seq(Visitor::<T, N>(std::marker::PhantomData))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RebaseDescribe {
    pub cut: RecoveryCut,
    pub index: u16,
    /// Includes unsent predictions and structural dependencies.
    pub cells: BoundedList<[i32; 3], DESCRIBE_PAGE_CELLS>,
}

/// 24 KiB of cells, leaving ample carrier room below both page/wire caps.
pub const DESCRIBE_PAGE_CELLS: usize = 2048;
pub const MAX_DESCRIBE_PARTS: usize = MAX_CONTEXT_CELLS / DESCRIBE_PAGE_CELLS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DescribeManifest {
    pub cut: RecoveryCut,
    pub cells: u16,
    pub parts: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RebasePage {
    pub cut: RecoveryCut,
    pub index: u16,
    pub bytes: BoundedList<u8, MAX_PAGE_BYTES>,
}

/// A complete encoded bundle in the outbox is one indivisible queue entry.
/// Parsing these ordered carriers never applies the fence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecoveryPacket {
    Prepare(RecoveryCut),
    DescribeBegin(DescribeManifest),
    Describe(RebaseDescribe),
    DescribeEnd(RecoveryCut),
    Begin(RebaseManifest),
    Page(RebasePage),
    End(RecoveryCut),
    Fence(RecoveryCut),
    Ack(RecoveryCut),
    DeathDescribe { session: [u8; 16], life: u32, highest_allocated: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c3d_wire_action_is_distinct_from_movement_and_bounded_before_retention() {
        let action = ActionHeader { session: [7; 16], epoch: 2, life: 3, seq: 900, events_applied: 4 };
        let packet = ActionPacket { action, intent: ActionIntent::CancelledNoop };
        let bytes = packet.encode().unwrap();
        assert_eq!(ActionPacket::decode(&bytes).unwrap().action, action);
        assert!(ActionPacket::decode(&vec![0; MAX_INTENT_BYTES + 1]).is_err());
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert!(ActionPacket::decode(&trailing).is_err());
        let receipt = ActionReceipt { action, fate: ActionFate::Refused, included_events: 12 };
        assert_ne!(receipt.included_events, receipt.action.events_applied);
    }

    #[test]
    fn c3d_wire_recovery_count_rejects_a_hostile_length_before_allocation() {
        use bincode::Options;
        let bytes = bincode::serialize(&vec![[0i32; 3]; MAX_CONTEXT_CELLS + 1]).unwrap();
        assert!(bincode::DefaultOptions::new().with_fixint_encoding().deserialize::<BoundedList<[i32; 3], MAX_CONTEXT_CELLS>>(&bytes).is_err());
        let hostile = u64::MAX.to_le_bytes();
        assert!(bincode::DefaultOptions::new().with_fixint_encoding().deserialize::<BoundedList<u8, MAX_PAGE_BYTES>>(&hostile).is_err());
    }
}
