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
        /// RESERVED, unread (run 11, run-9b review LOW-5): the client stamps
        /// its next input sequence here; the server never reads it, and
        /// nothing may depend on it until a decision gives it a meaning.
        source_tick: u64,
        change: protocol::BlockChange,
        hand: protocol::EditHand,
        mined: Option<protocol::MinedBlock>,
        use_tag: Option<protocol::UseTag>,
        /// C3d run 11 (run-10 review LOW-1, LOW-2) — the cell as the client
        /// saw it when it acted (block id, meta): the server's Drift check
        /// compares its own cell with it (`strict_edits::drifted`).
        old: (u16, u8),
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

impl ActionIntent {
    /// C3d run 16 — does the client predict NOTHING for this intent (no
    /// window, world or wear change before the server's answer)? The ONE
    /// table both sides read: the client asserts it where it reserves
    /// (`strict_requests::route_attack`), and only such an intent may be
    /// refused for a world fact with a receipt alone, no correction
    /// (`HostedServer::refuse_world_fact`). A class that starts predicting
    /// must leave this table, which puts its refusals back on the corrected
    /// path (the server falls back to it for any other intent).
    ///
    /// C3d run 17 (run-16 review LOW-1) — a shot too (run 13: nothing is
    /// predicted until its outcome). The table is a promise; the observed
    /// guard is `strict_requests::NoPrediction`, which the game loop runs
    /// around the swing and the shot (a window change there is an error).
    pub fn predicts_nothing(&self) -> bool {
        matches!(self, ActionIntent::Attack(_))
            || matches!(self, ActionIntent::Item(p) if matches!(p.action, protocol::ItemAction::Shoot { .. }))
    }
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

/// The client sends this only after ordered world/receipt and included events
/// apply. It frees server result metadata, never settles a window or action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppliedActionAck {
    pub session: [u8; 16],
    pub epoch: u32,
    pub life: u32,
    pub through: u64,
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
#[expect(dead_code, reason = "C3d recovery milestone: bounds DescribeManifest.parts when the server assembles a Describe")]
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

    /// C3d run 12 (run-11 review LOW-3) — the v89 `Edit` layout, pinned
    /// byte for byte (bincode: little-endian fixed ints, a u32 variant tag,
    /// a u8 `Option` tag).
    #[test]
    fn c3d_wire_edit_layout_is_pinned() {
        let action = ActionHeader { session: [7; 16], epoch: 2, life: 3, seq: 900, events_applied: 4 };
        let intent = ActionIntent::Edit {
            source_tick: 5,
            change: protocol::BlockChange { x: 1, y: -2, z: 3, new_block: 0x0102, meta: 9 },
            hand: (4, 1, 0x0304),
            mined: Some(protocol::MinedBlock { x: 1, y: -2, z: 3, tool: protocol::WireItem::None }),
            use_tag: None,
            old: (0x0506, 7),
        };
        let bytes = ActionPacket { action, intent }.encode().unwrap();
        let mut want: Vec<u8> = vec![7; 16];
        want.extend(2u32.to_le_bytes());
        want.extend(900u64.to_le_bytes());
        want.extend(3u32.to_le_bytes());
        want.extend(4u32.to_le_bytes());
        want.extend(0u32.to_le_bytes()); // ActionIntent::Edit
        want.extend(5u64.to_le_bytes()); // source_tick
        for v in [1i32, -2, 3] { want.extend(v.to_le_bytes()); }
        want.extend(0x0102u16.to_le_bytes());
        want.push(9);
        want.extend([4u8, 1]);
        want.extend(0x0304u16.to_le_bytes());
        want.push(1); // Some(mined)
        for v in [1i32, -2, 3] { want.extend(v.to_le_bytes()); }
        want.extend(0u32.to_le_bytes()); // WireItem::None
        want.push(0); // use_tag: None
        want.extend(0x0506u16.to_le_bytes());
        want.push(7);
        assert_eq!(&*bytes, &want[..], "the v89 Edit layout");
        // Run 13 (run-12 review LOW-4) — the negative: a payload without
        // `old` (the run-10 layout) doesn't decode.
        assert!(ActionPacket::decode(&want).is_ok(), "the pinned bytes decode");
        assert!(ActionPacket::decode(&want[..want.len() - 3]).is_err(), "a payload without `old` fails to decode");
    }

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

    /// C3d run 5 — the fate tags are append-only: `Superseded` is 4, after
    /// the four the wire already had.
    #[test]
    fn c3d_wire_fate_tags_append_superseded_last() {
        use protocol::{deserialize_header, serialize_packet, PacketType};
        let action = ActionHeader { session: [7; 16], epoch: 0, life: 0, seq: 1, events_applied: 0 };
        let fates = [ActionFate::Accepted, ActionFate::Refused, ActionFate::NormalisedAcceptedNoop, ActionFate::CancelledNoop, ActionFate::Superseded];
        for (tag, fate) in fates.into_iter().enumerate() {
            let bytes = serialize_packet(PacketType::ActionReceipt, &ActionReceipt { action, fate, included_events: 0 });
            let (_, payload) = deserialize_header(&bytes).unwrap();
            // session 16 + epoch 4 + seq 8 + life 4 + events_applied 4, then the fate.
            assert_eq!(&payload[36..40], &(tag as u32).to_le_bytes(), "{fate:?}");
            assert_eq!(protocol::safe_deserialize::<ActionReceipt>(payload).unwrap().fate, fate);
        }
    }

    /// v89 (provisional): the four C3d tags follow OC's v88 layout, which
    /// added no tag (it extended `InventorySync`, 67). Append-only, no
    /// collision, and nothing past 71 decodes.
    #[test]
    fn c3d_wire_tags_append_after_oc_v88_without_collision() {
        use protocol::{deserialize_header, serialize_packet, PacketType};
        assert_eq!(protocol::PROTOCOL_VERSION, 89);
        let action = ActionHeader { session: [7; 16], epoch: 0, life: 0, seq: 1, events_applied: 0 };
        let tagged = [
            (serialize_packet(PacketType::Action, &ActionPacket { action, intent: ActionIntent::CancelledNoop }), PacketType::Action, 68),
            (serialize_packet(PacketType::ActionReceipt, &ActionReceipt { action, fate: ActionFate::Refused, included_events: 0 }), PacketType::ActionReceipt, 69),
            (serialize_packet(PacketType::Recovery, &RecoveryPacket::Ack(RecoveryCut { session: [7; 16], id: 1, epoch: 1, life: 0, through: 0 })), PacketType::Recovery, 70),
            (serialize_packet(PacketType::AppliedActionAck, &AppliedActionAck { session: [7; 16], epoch: 0, life: 0, through: 1, included_events: 0 }), PacketType::AppliedActionAck, 71),
        ];
        for (bytes, tag, byte) in tagged {
            assert_eq!(bytes[0], byte);
            assert_eq!(deserialize_header(&bytes).map(|(t, _)| t), Some(tag));
        }
        assert_eq!(deserialize_header(&[67]).map(|(t, _)| t), Some(PacketType::InventorySync), "OC's v88 tag 67 is unchanged");
        assert!(deserialize_header(&[72]).is_none(), "no tag past 71");
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
