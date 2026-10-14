//! Private bodies survive the actual canonical installation boundary.
use super::*;
use crate::{action_identity::InventoryMode, container_window::CorrectionDebt, pack_wire::PackWindow, window_events::EffectiveWindow};

fn empty() -> EffectiveWindow {
    EffectiveWindow { inv: crate::inventory::Inventory::new(), armour: Default::default(), cursor: None, grid: Default::default() }
}
fn apply(window: &mut EffectiveWindow, packet: &PackWindow, uses: &crate::use_edits::SentUses, debt: &mut CorrectionDebt, mode: InventoryMode) -> Result<(), String> {
    apply_canonical_window(&mut WindowMut { inv: &mut window.inv, armour: &mut window.armour, cursor: &mut window.cursor, grid: &mut window.grid, container: None }, packet, &crate::block::BlockRegistry::new(), uses, debt, mode)
}
fn plan(stack: &ItemStack) -> &crate::plan::PlanData {
    let Item::Plan(body) = &stack.item else { panic!("canonical place lost its Plan") };
    body
}

#[test]
fn d2c2_canonical_pack_moves_held_body_and_retains_pending_use_body_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let _root = crate::save::WorldsRootGuard::new(&format!("d2c2-canonical-custody-{mode:?}"));
        let mut held = crate::plan::PlanData::debug_3x3_stone();
        held.name = "Exact held pack fence body".into();
        let encoded = bincode::serialize(&held).unwrap();
        let marker = crate::plan::marker(&held);
        let original_allocation = held.cells.as_ptr();
        let mut window = empty();
        window.inv.auto_refill = false;
        window.inv.set_locked_from(&[3]);
        window.inv.set_slot(3, Some(ItemStack { item: Item::Plan(held), count: 1 }));
        let mut history = crate::use_edits::SentUses::default();
        let mut pending = crate::plan::PlanData::debug_3x3_stone();
        pending.name = "Pending use remains owned across pack fence".into();
        let pending_encoded = bincode::serialize(&pending).unwrap();
        let pending_marker = crate::plan::marker(&pending);
        history.record(crate::use_edits::UseRecord { cell: [1, 2, 3], kind: 0, slot: 0, cost: Some(Item::Plan(pending)), landed: None, made_at: 1 });
        let mut canonical = empty();
        canonical.cursor = Some(ItemStack { item: Item::Plan(crate::plan::PlanData::marker_placeholder(marker, true)), count: 1 });
        let packet = PackWindow::capture(&canonical, &CorrectionDebt::default(), &crate::block::BlockRegistry::new()).unwrap();
        apply(&mut window, &packet, &history, &mut CorrectionDebt::default(), mode).unwrap();
        let actual = plan(window.cursor.as_ref().unwrap());
        assert_eq!(actual.cells.as_ptr(), original_allocation, "a real held body moves ownership rather than being recreated from its marker");
        assert_eq!(bincode::serialize(actual).unwrap(), encoded);
        assert!(window.inv.slot(3).is_none());
        assert!(!window.inv.auto_refill);
        assert_eq!(window.inv.is_locked(3), mode == InventoryMode::Mirror);
        assert_eq!(bincode::serialize(history.private_plans().next().unwrap()).unwrap(), pending_encoded, "canonical installation never retires pending use history");
        assert_eq!(bincode::serialize(&crate::plan_store::resolve(marker).unwrap()).unwrap(), encoded);
        assert_eq!(bincode::serialize(&crate::plan_store::resolve(pending_marker).unwrap()).unwrap(), pending_encoded, "history body retention completes before physical replacement");
        // A later window can remove the last held place only after its exact
        // body has already become recoverable in the checked local store.
        let cleared = PackWindow::capture(&empty(), &CorrectionDebt::default(), &crate::block::BlockRegistry::new()).unwrap();
        apply(&mut window, &cleared, &history, &mut CorrectionDebt::default(), mode).unwrap();
        assert!(window.cursor.is_none());
        assert_eq!(bincode::serialize(&crate::plan_store::resolve(marker).unwrap()).unwrap(), encoded);
    }
}

#[test]
fn d2c2_canonical_unknown_marker_and_wrong_develop_state_stay_honest_stubs() {
    let _root = crate::save::WorldsRootGuard::new("d2c2-canonical-stubs");
    let mut body = crate::plan::PlanData::debug_3x3_stone();
    body.name = "Known body with a different develop state".into();
    crate::plan_store::remember_checked(&body, 0).unwrap();
    let marker = crate::plan::marker(&body);
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut canonical = empty();
        canonical.inv.set_slot(2, Some(ItemStack { item: Item::Plan(crate::plan::PlanData::marker_placeholder([0xD2; 32], true)), count: 1 }));
        canonical.cursor = Some(ItemStack { item: Item::Plan(crate::plan::PlanData::marker_placeholder(marker, false)), count: 1 });
        let packet = PackWindow::capture(&canonical, &CorrectionDebt::default(), &crate::block::BlockRegistry::new()).unwrap();
        let mut window = empty();
        apply(&mut window, &packet, &Default::default(), &mut Default::default(), mode).unwrap();
        let unknown = plan(window.inv.slot(2).unwrap());
        assert_eq!(crate::plan::marker(unknown), [0xD2; 32]);
        assert!(unknown.marker.is_some() && unknown.cells.is_empty());
        let latent = plan(window.cursor.as_ref().unwrap());
        assert_eq!(crate::plan::marker(latent), marker);
        assert!(!latent.develop_state.is_developed());
        assert!(latent.marker.is_some() && latent.cells.is_empty(), "a cached developed body cannot replace a latent marker");
    }
}

#[test]
fn d2c2_canonical_retention_failure_leaves_every_physical_place_unchanged() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let _root = crate::save::WorldsRootGuard::new(&format!("d2c2-canonical-retain-failure-{mode:?}"));
        let mut body = crate::plan::PlanData::debug_3x3_stone();
        body.name = "x".repeat(crate::action_capacity::MAX_PRIVATE_BODY_BYTES + 1);
        let mut window = empty();
        window.inv.set_slot(7, Some(ItemStack { item: Item::Plan(body), count: 1 }));
        window.cursor = Some(ItemStack::new_block(crate::block::STONE, 3));
        let before = bincode::serialize(&(window.inv.slots_iter().collect::<Vec<_>>(), &window.cursor, &window.grid, &window.armour)).unwrap();
        let packet = PackWindow::capture(&empty(), &Default::default(), &crate::block::BlockRegistry::new()).unwrap();
        let mut debt = CorrectionDebt::default();
        assert!(apply(&mut window, &packet, &Default::default(), &mut debt, mode).is_err(), "an unstorable private body must refuse before the canonical replacement");
        assert_eq!(bincode::serialize(&(window.inv.slots_iter().collect::<Vec<_>>(), &window.cursor, &window.grid, &window.armour)).unwrap(), before);
        assert!(debt.is_empty());
    }
}
