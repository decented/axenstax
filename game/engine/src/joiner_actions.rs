//! A joiner's swings and right-clicks on the server's mobs, awaiting the
//! server's word (MP-D2b, Spec 04 §4.2d).
//!
//! A joiner's inventory is its own until phase C, but the server decides
//! whether a swing or an interaction happened. So the client sends the
//! request (`EntityAttack` / `EntityInteract`) and remembers what was in
//! which hotbar slot; when the `InteractOutcome` comes back accepted it takes
//! the consumed items from that slot (or wears the weapon), and only if the
//! slot still holds the item the request was made with — a stack swapped out
//! in the meantime is never charged for someone else's click. A refused
//! outcome changes nothing. Products (milk, the Lead back, wool and loot
//! picked up) arrive as `InventoryGrant`s, never through here.

use std::collections::VecDeque;

use crate::item::Item;
use crate::mob::MobType;
use crate::protocol::{InteractKind, InteractOutcomePacket};

/// Requests kept waiting for an answer. The server answers every request it
/// reads; one past its per-tick budget is dropped unanswered, so the oldest
/// waiting entry is forgotten once this many are outstanding.
pub const MAX_PENDING: usize = 64;

/// One request awaiting its outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    /// `None` = a swing; otherwise the interaction asked for.
    pub kind: Option<InteractKind>,
    /// The mob's species (the note's wording).
    pub mob: MobType,
    /// Where the held item was.
    pub hotbar_slot: usize,
    /// What it was (`None` = an empty hand).
    pub held: Option<Item>,
}

/// The joiner's outstanding requests. Empty unless joined.
#[derive(Default)]
pub struct JoinerActions {
    next_seq: u32,
    pending: VecDeque<(u32, Pending)>,
}

impl JoinerActions {
    /// Remember a request; returns the `seq` to send it under.
    pub fn record(&mut self, request: Pending) -> u32 {
        self.next_seq = self.next_seq.wrapping_add(1);
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back((self.next_seq, request));
        self.next_seq
    }

    /// The request `seq` answered, if this client is waiting for it.
    pub fn take(&mut self, seq: u32) -> Option<Pending> {
        let at = self.pending.iter().position(|(s, _)| *s == seq)?;
        self.pending.remove(at).map(|(_, p)| p)
    }

    /// Forget everything (the session ended).
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

/// What an outcome did to the inventory.
#[derive(Debug, Default)]
pub struct Applied {
    /// The weapon's wear, for the "tool worn / broke" feedback (accepted
    /// swings only).
    pub wear: Option<crate::inventory::ToolUseInfo>,
    /// Items taken from the held stack.
    pub consumed: u8,
}

/// Does `slot` still hold the item the request was made with? A tool is the
/// same tool by type and material (its durability is what wears).
fn still_holds(inv: &crate::inventory::Inventory, request: &Pending) -> bool {
    let now = inv.hotbar_slot(request.hotbar_slot).map(|s| &s.item);
    match (now, request.held.as_ref()) {
        (Some(Item::Tool(a)), Some(Item::Tool(b))) => {
            a.tool_type == b.tool_type && a.material == b.material
        }
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Apply an outcome to the joiner's inventory: an accepted swing wears the
/// weapon (`Inventory::use_hotbar_tool`, the single-player rule — a swing
/// that found its target wears it, damage or not); an accepted interaction
/// takes `consume_held` from the stack. Nothing on a refusal, and nothing if
/// the slot no longer holds the requested item.
pub fn apply_outcome(
    inv: &mut crate::inventory::Inventory,
    request: &Pending,
    outcome: &InteractOutcomePacket,
) -> Applied {
    let mut applied = Applied::default();
    if !outcome.accepted || !still_holds(inv, request) {
        return applied;
    }
    match request.kind {
        None => applied.wear = inv.use_hotbar_tool(request.hotbar_slot),
        Some(_) => {
            for _ in 0..outcome.consume_held {
                let took = match &request.held {
                    Some(Item::Material(m)) => inv.consume_one_material(request.hotbar_slot, *m),
                    Some(_) => inv.take_one_from_hotbar(request.hotbar_slot).is_some(),
                    None => false,
                };
                if !took {
                    break;
                }
                applied.consumed += 1;
            }
        }
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::inventory::Inventory;
    use crate::item::{ItemStack, MaterialId};

    fn outcome(seq: u32, accepted: bool, consume_held: u8) -> InteractOutcomePacket {
        InteractOutcomePacket { seq, entity: 1, kind: None, accepted, consume_held, note: 0 }
    }

    fn inv_with(slot: usize, stack: ItemStack) -> Inventory {
        let mut inv = Inventory::new();
        inv.set_slot(slot, Some(stack));
        inv
    }

    fn sword() -> Item {
        Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Iron))
    }

    fn durability(inv: &Inventory, slot: usize) -> u32 {
        match &inv.hotbar_slot(slot).unwrap().item {
            Item::Tool(t) => t.durability as u32,
            _ => unreachable!(),
        }
    }

    #[test]
    fn the_weapon_wears_only_on_a_confirmed_swing() {
        let mut inv = inv_with(2, ItemStack { item: sword(), count: 1 });
        let before = durability(&inv, 2);
        let swing = Pending { kind: None, mob: MobType::Cow, hotbar_slot: 2, held: Some(sword()) };
        let refused = apply_outcome(&mut inv, &swing, &outcome(1, false, 0));
        assert!(refused.wear.is_none());
        assert_eq!(durability(&inv, 2), before, "a refused swing wears nothing");
        let confirmed = apply_outcome(&mut inv, &swing, &outcome(1, true, 0));
        assert!(confirmed.wear.is_some());
        assert_eq!(durability(&inv, 2), before - 1);
    }

    #[test]
    fn an_accepted_interaction_takes_the_items_and_a_refused_one_takes_nothing() {
        let wheat = Item::Material(MaterialId::Wheat);
        let mut inv = inv_with(0, ItemStack { item: wheat.clone(), count: 5 });
        let feed = Pending {
            kind: Some(InteractKind::Feed),
            mob: MobType::Cow,
            hotbar_slot: 0,
            held: Some(wheat),
        };
        assert_eq!(apply_outcome(&mut inv, &feed, &outcome(1, false, 1)).consumed, 0);
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 5);
        assert_eq!(apply_outcome(&mut inv, &feed, &outcome(1, true, 1)).consumed, 1);
        assert_eq!(inv.hotbar_slot(0).unwrap().count, 4);
    }

    #[test]
    fn a_stack_swapped_out_meanwhile_is_never_charged() {
        let bucket = Item::Material(MaterialId::Bucket);
        let mut inv = inv_with(3, ItemStack::new_material(MaterialId::Bone, 4));
        let milk = Pending {
            kind: Some(InteractKind::Milk),
            mob: MobType::Cow,
            hotbar_slot: 3,
            held: Some(bucket),
        };
        assert_eq!(apply_outcome(&mut inv, &milk, &outcome(1, true, 1)).consumed, 0);
        assert_eq!(inv.hotbar_slot(3).unwrap().count, 4);
    }

    #[test]
    fn pending_requests_are_matched_by_seq_and_bounded() {
        let mut a = JoinerActions::default();
        let req = |slot| Pending { kind: None, mob: MobType::Pig, hotbar_slot: slot, held: None };
        let s1 = a.record(req(1));
        let s2 = a.record(req(2));
        assert_ne!(s1, s2);
        assert_eq!(a.take(s2).unwrap().hotbar_slot, 2);
        assert!(a.take(s2).is_none(), "answered once");
        for _ in 0..MAX_PENDING + 5 {
            a.record(req(0));
        }
        assert_eq!(a.len(), MAX_PENDING);
        assert!(a.take(s1).is_none(), "the oldest unanswered request was forgotten");
    }
}
