//! Bounded entity-pack requests/results; private Plan bodies never travel here.
use serde::{Deserialize, Serialize};
use crate::{action_identity::{ActionFate, ActionHeader}, action_wire::BoundedList, container_window::{ContainerClick, ContainerTarget}, protocol::{WireSlot, WireStack}, window_events::{EffectiveWindow, PackWindowStage}};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackOperation {
    Equip,
    Open,
    Click { incarnation: u64, revision: u64, click: ContainerClick },
    Remove { incarnation: u64, revision: u64 },
    Close { incarnation: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackRequest {
    /// The actual respawn generation, separate from the retained namespace.
    pub actor_respawns: u32,
    pub target: ContainerTarget,
    /// The actual selected place; a held item claim cannot supply payment.
    pub hotbar_slot: u8,
    pub before: u32,
    pub operation: PackOperation,
}

/// A counted complete personal window, with no unrelated history reset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackWindow {
    pub slots: BoundedList<WireSlot, 50>,
    pub owed: BoundedList<(WireStack, u32), { crate::protocol::MAX_SYNC_OWED }>,
    pub locks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackView {
    pub target: ContainerTarget,
    pub incarnation: u64,
    pub revision: u64,
    pub slots: BoundedList<WireSlot, 27>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackNote { Done, Updated, NoPack, NeedsKeeper, Species, OutOfReach, Denied, NotEmpty, Dead, Waiting }

impl PackNote {
    pub fn message(self) -> &'static str {
        match self {
            Self::Done => "",
            Self::Updated => "The pack or inventory changed; try again.",
            Self::NoPack => "That steed has no pack.",
            Self::NeedsKeeper => "Someone must keep this steed before equipping a pack.",
            Self::Species => "Only a donkey or mule can carry a pack.",
            Self::OutOfReach => "Move closer to the steed.",
            Self::Denied => "Pack use is restricted here.",
            Self::NotEmpty => "Empty the pack before removing it.",
            Self::Dead => "Pack use is waiting until you return.",
            Self::Waiting => "Inventory action is waiting; try again.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackResult {
    pub actor_respawns: u32,
    pub action: ActionHeader,
    pub fate: ActionFate,
    pub note: PackNote,
    pub window_event: u32,
    pub window: PackWindow,
    pub view: Option<PackView>,
}

/// Views change no personal place; they are still numbered and life-scoped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackViewPush {
    pub actor_respawns: u32,
    pub session: [u8; 16],
    pub epoch: u32,
    pub life: u32,
    pub window_event: u32,
    pub target: ContainerTarget,
    pub incarnation: u64,
    pub view: Option<PackView>,
}

impl PackViewPush {
    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(self.view.as_ref().map_or(0, |view| view.slots.allocated_bytes()))
    }
}

fn exact_slot(slot: &WireSlot, registry: &crate::block::BlockRegistry) -> Result<Option<crate::item::ItemStack>, String> {
    let Some(wire) = slot else { return Ok(None) };
    let stack = crate::inventory::stack_from_wire(wire, registry, crate::inventory::PlanDecode::Marker).ok_or("A pack slot does not decode")?;
    if crate::inventory::stack_to_wire(&stack) != *wire { return Err("A pack slot does not round-trip exactly".into()); }
    Ok(Some(stack))
}

impl PackWindow {
    pub fn capture(window: &EffectiveWindow, debt: &crate::container_window::CorrectionDebt, registry: &crate::block::BlockRegistry) -> Result<Self, String> {
        let marker = crate::staged_window::MarkerWindow::try_capture_parts(&window.inv, &window.armour, &window.cursor, &window.grid, registry).ok_or("The personal window cannot be carried exactly")?;
        let owed = debt.to_wire();
        if crate::container_window::CorrectionDebt::from_wire(&owed, registry)? != *debt { return Err("The personal debt cannot be carried exactly".into()); }
        Ok(Self { slots: BoundedList::new(marker.slots().into_iter().collect()).map_err(str::to_string)?, owed: BoundedList::new(owed).map_err(str::to_string)?, locks: window.inv.locked_indices().iter().fold(0, |mask, slot| mask | (1u64 << slot)) })
    }

    /// Decode every place and the whole debt before the caller changes one.
    pub fn decode_window(&self, registry: &crate::block::BlockRegistry) -> Result<PackWindowStage, String> {
        if self.slots.as_slice().len() != 50 || self.locks >> 36 != 0 { return Err("A pack window is incomplete".into()); }
        let debt = crate::container_window::CorrectionDebt::from_wire(self.owed.as_slice(), registry)?;
        let stacks = self.slots.as_slice().iter().map(|slot| exact_slot(slot, registry)).collect::<Result<Vec<_>, _>>()?;
        let mut window = EffectiveWindow { inv: crate::inventory::Inventory::new(), armour: [None; 4], cursor: None, grid: Default::default() };
        for (at, stack) in stacks.into_iter().enumerate() {
            match at {
                0..36 => window.inv.set_slot(at, stack),
                36..40 => {
                    window.armour[at - 36] = match stack {
                        None => None,
                        Some(crate::item::ItemStack { item: crate::item::Item::Armour(armour), count: 1 }) => Some(armour),
                        _ => return Err("A pack window armour place holds another item".into()),
                    };
                }
                40 => window.cursor = stack,
                _ => window.grid[(at - 41) / 3][(at - 41) % 3] = stack,
            }
        }
        window.inv.set_locked_from(&(0..36u32).filter(|slot| self.locks & (1u64 << slot) != 0).collect::<Vec<_>>());
        Ok(PackWindowStage { window: Box::new(window), debt })
    }
}

impl PackView {
    pub fn capture(target: ContainerTarget, incarnation: u64, revision: u64, pack: &crate::chest::ChestData) -> Result<Self, String> {
        if !matches!(target, ContainerTarget::Entity(_)) || incarnation == 0 || revision == 0 || pack.tier != crate::chest::ChestTier::Wood || pack.slots.len() != 27 { return Err("An entity pack view is invalid".into()); }
        Ok(Self { target, incarnation, revision, slots: BoundedList::new(pack.slots.iter().map(|slot| slot.as_ref().map(crate::inventory::stack_to_wire)).collect()).map_err(str::to_string)? })
    }
    pub fn decode_contents(&self, registry: &crate::block::BlockRegistry) -> Result<crate::chest::ChestData, String> {
        if !matches!(self.target, ContainerTarget::Entity(_)) || self.incarnation == 0 || self.revision == 0 || self.slots.as_slice().len() != 27 { return Err("An entity pack view is invalid".into()); }
        Ok(crate::chest::ChestData { tier: crate::chest::ChestTier::Wood, slots: self.slots.as_slice().iter().map(|slot| exact_slot(slot, registry)).collect::<Result<Vec<_>, _>>()? })
    }
}

impl PackResult {
    /// WireStack contains only fixed-size WireItem fields (including Plan markers).
    /// Count Vec capacity, rather than its wire length or populated places.
    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(self.window.slots.allocated_bytes())
            .saturating_add(self.window.owed.allocated_bytes())
            .saturating_add(self.view.as_ref().map_or(0, |view| view.slots.allocated_bytes()))
    }

    pub fn encode(&self) -> Result<Box<[u8]>, String> {
        let bytes = bincode::serialize(self).map_err(|error| error.to_string())?;
        if self.window_event == 0 || bytes.len() > crate::action_journal::MAX_PAGE_BYTES { return Err("A pack result exceeds its carrier bound".into()); }
        Ok(bytes.into_boxed_slice())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > crate::action_journal::MAX_PAGE_BYTES { return Err("A pack result exceeds its carrier bound".into()); }
        let packet: Self = crate::protocol::safe_deserialize(bytes).map_err(|error| error.to_string())?;
        if packet.window_event == 0 { return Err("A pack result has no applied fence".into()); }
        Ok(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header() -> ActionHeader { ActionHeader { session: [7; 16], epoch: 0, life: 1, seq: 1, events_applied: 0 } }

    #[test]
    fn d2c2_pack_wire_pins_tags_and_appended_unpredicted_intent() {
        assert_eq!(crate::protocol::PROTOCOL_VERSION, 95);
        assert_eq!(crate::protocol::PacketType::PackResult as u8, 74);
        assert_eq!(crate::protocol::PacketType::PackView as u8, 75);
        assert_eq!(crate::protocol::entity_flags::PACK, 128);
        for (intent, tag) in [(crate::action_wire::ActionIntent::CancelledNoop, 11u32), (crate::action_wire::ActionIntent::Pack(PackRequest { actor_respawns: 0, target: ContainerTarget::Entity(9), hotbar_slot: 0, before: 3, operation: PackOperation::Open }), 12)] {
            let packet = crate::action_wire::ActionPacket { action: header(), intent };
            let bytes = packet.encode().unwrap();
            assert_eq!(&bytes[36..40], &tag.to_le_bytes());
            if tag == 12 { assert!(packet.intent.predicts_nothing()); }
            assert_eq!(crate::action_wire::ActionPacket::decode(&bytes).unwrap().action, header());
        }
    }

    #[test]
    fn d2c2_pack_result_carries_exact_window_debt_and_marker_view_without_a_plan_body() {
        let registry = crate::block::BlockRegistry::new();
        let mut window = EffectiveWindow { inv: crate::inventory::Inventory::new(), armour: [None; 4], cursor: None, grid: Default::default() };
        window.inv.set_slot(3, Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 2)));
        window.inv.toggle_lock(3);
        let debt = crate::container_window::CorrectionDebt::from_entries(vec![(crate::item::Item::Block(crate::block::STONE), 4)]).unwrap();
        let mut pack = crate::chest::ChestData::new();
        let body = crate::plan::PlanData::debug_3x3_stone();
        let marker = crate::plan::marker(&body);
        pack.slots[8] = Some(crate::item::ItemStack { item: crate::item::Item::Plan(body), count: 1 });
        let result = PackResult { actor_respawns: 0, action: header(), fate: ActionFate::Accepted, note: PackNote::Done, window_event: 7, window: PackWindow::capture(&window, &debt, &registry).unwrap(), view: Some(PackView::capture(ContainerTarget::Entity(9), 1, 2, &pack).unwrap()) };
        let decoded = PackResult::decode(&result.encode().unwrap()).unwrap();
        assert_eq!(decoded, result);
        let personal = decoded.window.decode_window(&registry).unwrap();
        assert_eq!(personal.debt, debt);
        assert_eq!(personal.window.inv.slot(3).unwrap().count, 2);
        assert!(personal.window.inv.is_locked(3));
        let cargo = decoded.view.unwrap().decode_contents(&registry).unwrap();
        let crate::item::Item::Plan(stub) = &cargo.slots[8].as_ref().unwrap().item else { panic!("Plan marker") };
        assert_eq!(crate::plan::marker(stub), marker);
        assert!(stub.cells.is_empty(), "no private Plan body travels in a cargo view");
    }

    #[test]
    fn d2c2_pack_wire_refuses_excess_or_incomplete_places_and_whole_bad_debt() {
        let bytes = bincode::serialize(&vec![None::<WireStack>; 51]).unwrap();
        assert!(crate::protocol::safe_deserialize::<BoundedList<WireSlot, 50>>(&bytes).is_err());
        let registry = crate::block::BlockRegistry::new();
        let mut packet = PackWindow { slots: BoundedList::new(vec![None; 49]).unwrap(), owed: BoundedList::new(Vec::new()).unwrap(), locks: 0 };
        assert!(packet.decode_window(&registry).is_err());
        packet.slots = BoundedList::new(vec![None; 50]).unwrap();
        let wire = crate::inventory::stack_to_wire(&crate::item::ItemStack::new_block(crate::block::STONE, 1));
        packet.owed = BoundedList::new(vec![(wire, 0)]).unwrap();
        assert!(packet.decode_window(&registry).is_err(), "an invalid obligation is refused whole");
    }
}
