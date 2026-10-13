//! Marker-safe item slots shared by the existing sidecar and steed cargo.
//! The first two variants and fields are the sidecar's original byte layout.
use crate::{item::{Item, ItemStack}, save::SavedSlot};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SavedItemSlot {
    Saved(SavedSlot),
    Plan { marker: [u8; 32], developed: bool },
}

impl SavedItemSlot {
    pub(crate) fn capture(stack: Option<&ItemStack>) -> Self {
        if let Some(ItemStack { item: Item::Plan(plan), .. }) = stack {
            return Self::Plan { marker: crate::plan::marker(plan), developed: plan.develop_state.is_developed() };
        }
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, stack.cloned());
        Self::Saved(crate::save::serialize_inventory_raw(&inv).remove(0))
    }

    /// New cargo saves reject values that the old slot conversion would
    /// clamp. No source metadata or stack count is silently discarded.
    pub(crate) fn capture_exact(stack: Option<&ItemStack>) -> Result<Self, String> {
        let saved = Self::capture(stack);
        match stack {
            Some(ItemStack { item: Item::Plan(_), count }) if *count != 1 => return Err("A cargo Plan must occupy one slot as one item".into()),
            Some(ItemStack { item: Item::Plan(_), .. }) => {},
            _ if saved.restore()? != stack.cloned() => return Err("A cargo slot could not be saved exactly".into()),
            _ => {},
        }
        Ok(saved)
    }

    pub(crate) fn restore(&self) -> Result<Option<ItemStack>, String> {
        match self {
            Self::Plan { marker, developed } => Ok(Some(ItemStack {
                item: Item::Plan(crate::plan::PlanData::marker_placeholder(*marker, *developed)), count: 1,
            })),
            Self::Saved(SavedSlot::Plan { .. }) => Err("sidecar contains a Plan body".into()),
            Self::Saved(saved) => {
                let mut inv = crate::inventory::Inventory::new();
                crate::save::restore_inventory(&mut inv, std::slice::from_ref(saved));
                Ok(inv.slot(0).cloned())
            },
        }
    }

    /// Local readers retain the existing private body when it is available;
    /// a missing body remains the same marker-bearing stub.
    pub(crate) fn restore_local(&self) -> Result<Option<ItemStack>, String> {
        let mut stack = self.restore()?;
        if let Self::Plan { marker, developed } = self
            && let Some(body) = crate::plan_store::resolve(*marker)
            && body.develop_state.is_developed() == *developed
            && let Some(stack) = stack.as_mut()
        {
            // The marker hashes development too: changing the resolved body
            // under the old marker would invent a different Plan identity.
            stack.item = Item::Plan(body);
        }
        Ok(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    enum OldSlot { Saved(SavedSlot), Plan { marker: [u8; 32], developed: bool } }

    #[test]
    fn d2c2_shared_slot_keeps_sidecar_bytes_and_item_metadata() {
        let stack = ItemStack::new_material(crate::item::MaterialId::Bread, 7);
        let saved = SavedItemSlot::capture_exact(Some(&stack)).unwrap();
        let SavedItemSlot::Saved(slot) = &saved else { panic!("ordinary slot") };
        assert_eq!(bincode::serialize(&saved).unwrap(), bincode::serialize(&OldSlot::Saved(slot.clone())).unwrap());
        assert_eq!(saved.restore().unwrap(), Some(stack));
        let marker = [31; 32];
        let saved = SavedItemSlot::Plan { marker, developed: false };
        assert_eq!(bincode::serialize(&saved).unwrap(), bincode::serialize(&OldSlot::Plan { marker, developed: false }).unwrap());
    }

    #[test]
    fn d2c2_local_cargo_plan_restore_keeps_marker_and_recorded_development() {
        let _guard = crate::save::WorldsRootGuard::new("d2c2-cargo-plan");
        let mut body = crate::plan::PlanData::debug_3x3_stone();
        body.name = "Private cargo body".into();
        body.develop_state = crate::plan::DevelopState::Latent { exposure_ticks: 12 };
        crate::plan_store::remember_checked(&body, 0).unwrap();
        let marker = crate::plan::marker(&body);
        let slot = SavedItemSlot::Plan { marker, developed: false };
        let Some(ItemStack { item: Item::Plan(local), .. }) = slot.restore_local().unwrap() else { panic!("local Plan") };
        assert_eq!(crate::plan::marker(&local), marker);
        assert_eq!(local.cells, body.cells, "the local body is retained");
        assert_eq!(local.develop_state, body.develop_state, "the save's actual development is retained");
        let unknown = [93; 32];
        let Some(ItemStack { item: Item::Plan(stub), .. }) = (SavedItemSlot::Plan { marker: unknown, developed: true }).restore_local().unwrap() else { panic!("stub") };
        assert_eq!(crate::plan::marker(&stub), unknown);
        assert!(stub.marker.is_some() && stub.cells.is_empty());
        assert!(stub.develop_state.is_developed());
    }
}
