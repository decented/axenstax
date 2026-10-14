//! Narrow pack admission in both advertised modes, preserving ordered debt.
use super::*;
use std::borrow::Cow;
use crate::staged_window::{MarkerSlots, MarkerWindow, SourceBacking, StagedClick};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackStageError { Unrepresentable, Boundary, Source, Obligation, Fit }

#[derive(Debug)]
pub struct PackWindowStage {
    pub window: Box<EffectiveWindow>,
    pub debt: crate::container_window::CorrectionDebt,
}

fn exact_debt(debt: &crate::container_window::CorrectionDebt, registry: &crate::block::BlockRegistry) -> Result<crate::container_window::CorrectionDebt, PackStageError> {
    let copy = crate::container_window::CorrectionDebt::from_entries(debt.entries().to_vec()).map_err(|_| PackStageError::Unrepresentable)?;
    let decoded = crate::container_window::CorrectionDebt::from_wire(&copy.to_wire(), registry).map_err(|_| PackStageError::Unrepresentable)?;
    if decoded != copy { return Err(PackStageError::Unrepresentable); }
    Ok(copy)
}

/// Actual raw places, without global Strict's debt/history requirements.
pub fn pack_raw_marker(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> Result<MarkerWindow, PackStageError> {
    MarkerWindow::try_capture_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, registry).ok_or(PackStageError::Unrepresentable)
}

fn holding_event<'a>(event: &'a WindowEvent, registry: &crate::block::BlockRegistry) -> Result<Cow<'a, WindowEvent>, PackStageError> {
    let exact = |stack: &ItemStack| crate::staged_window::exact_marker(stack, registry).ok_or(PackStageError::Unrepresentable);
    match event {
        WindowEvent::Grant(stack) => Ok(Cow::Owned(WindowEvent::Grant(exact(stack)?))),
        WindowEvent::Take { slot, item, n, how } => {
            let marker = exact(&ItemStack { item: marker_item(item), count: 1 })?;
            Ok(Cow::Owned(WindowEvent::Take { slot: *slot, item: marker.item, n: *n, how }))
        }
        WindowEvent::Correction(c) => {
            // Existing shared-container deltas never move Plans. Refuse a
            // defensive malformed prefix instead of cloning a private body.
            if let Some(own) = &c.own {
                for stack in own.take.iter().map(|(_, stack)| stack).chain(own.give.iter()) {
                    if matches!(stack.item, Item::Plan(_)) { return Err(PackStageError::Boundary); }
                    exact(stack)?;
                }
            }
            Ok(Cow::Borrowed(event))
        }
        WindowEvent::Replace { .. } | WindowEvent::PackCanonical { .. } => Err(PackStageError::Boundary),
        _ => Ok(Cow::Borrowed(event)),
    }
}

fn marker_item(item: &Item) -> Item {
    match item {
        Item::Plan(body) => Item::Plan(crate::plan::PlanData::marker_placeholder(crate::plan::marker(body), body.develop_state.is_developed())),
        _ => item.clone(),
    }
}

fn into_window(window: MarkerWindow) -> Box<EffectiveWindow> {
    let (inv, armour, cursor, grid) = window.into_parts();
    Box::new(EffectiveWindow { inv, armour, cursor, grid })
}

/// Compare actual per-place removals/additions, including exact piece/marker
/// metadata. Counts are deltas, so an unchanged earlier source may still
/// merge normally. A prior obligation cannot move to newly withdrawn stock.
fn slot_effects(before: &MarkerSlots, after: &MarkerSlots) -> Vec<(usize, crate::protocol::WireSlot, crate::protocol::WireSlot)> {
    let mut effects = Vec::new();
    for (at, (old, new)) in before.iter().zip(after).enumerate() {
        if old == new { continue; }
        let same = matches!((old, new), (Some(a), Some(b)) if a.item_kind == b.item_kind && a.item_id == b.item_id && a.full_item == b.full_item);
        let (removed, added) = if same {
            let a = old.as_ref().expect("same identity has old stack");
            let b = new.as_ref().expect("same identity has new stack");
            let removed = (a.count > b.count).then(|| { let mut s = a.clone(); s.count -= b.count; s });
            let added = (b.count > a.count).then(|| { let mut s = b.clone(); s.count -= a.count; s });
            (removed, added)
        } else { (old.clone(), new.clone()) };
        effects.push((at, removed, added));
    }
    effects
}

fn fold(window: &mut MarkerWindow, debt: &mut crate::container_window::CorrectionDebt, event: &WindowEvent) -> (Effect, Vec<(usize, crate::protocol::WireSlot, crate::protocol::WireSlot)>) {
    let before = window.slots();
    let view = window.view();
    let effect = apply_to(&mut Parts { inv: view.inv, armour: view.armour, cursor: view.cursor, grid: view.grid }, debt, event);
    (effect, slot_effects(&before, &window.slots()))
}

/// Admit the exact staged personal transition against every earlier prefix.
/// The returned canonical window is AFTER that prefix; no raw bag is changed.
pub fn admit_pack_staged(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, raw: &MarkerWindow, staged: &StagedClick) -> Result<PackWindowStage, PackStageError> {
    let mut original = raw.clone();
    let mut candidate = staged.after.clone();
    let mut original_debt = exact_debt(&sp.window_events.debt, registry)?;
    let mut candidate_debt = original_debt.clone();
    let mut backing = SourceBacking::new(raw.slots());
    for waiting in &sp.window_events.waiting {
        if matches!(&waiting.event, WindowEvent::PackCanonical { actor_respawns, .. } if *actor_respawns != sp.respawns) { continue; }
        let event = holding_event(&waiting.event, registry)?;
        let before = original.slots();
        let expected = fold(&mut original, &mut original_debt, &event);
        let after = original.slots();
        if matches!(&*event, WindowEvent::WearWeapon { .. } | WindowEvent::WearArmour { .. }) {
            backing.observe_authorised_wear(&before, &after);
        }
        backing.observe_prefix(&after);
        if fold(&mut candidate, &mut candidate_debt, &event) != expected || candidate_debt != original_debt {
            return Err(PackStageError::Obligation);
        }
    }
    if !backing.backs(&staged.sources) { return Err(PackStageError::Source); }
    let debt = exact_debt(&candidate_debt, registry)?;
    Ok(PackWindowStage { window: into_window(candidate), debt })
}

/// An exact zero-effect terminal fence (including a stale-view refusal).
/// Lifecycle replacements retain their own semantics before this fence.
pub fn pack_canonical_window(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> Result<PackWindowStage, PackStageError> {
    let mut window = pack_raw_marker(sp, registry)?;
    let mut debt = exact_debt(&sp.window_events.debt, registry)?;
    for waiting in &sp.window_events.waiting {
        if matches!(&waiting.event, WindowEvent::PackCanonical { actor_respawns, .. } if *actor_respawns != sp.respawns) { continue; }
        match &waiting.event {
            WindowEvent::Replace { window: replaced, debt: carried, .. }
            | WindowEvent::PackCanonical { window: replaced, debt: carried, .. } => {
                let prefs = window.view().inv;
                let refill = prefs.auto_refill;
                let locks = prefs.locked_indices();
                window = MarkerWindow::try_capture_parts(&replaced.inv, &replaced.armour, &replaced.cursor, &replaced.grid, registry).ok_or(PackStageError::Unrepresentable)?;
                let prefs = window.view().inv;
                prefs.auto_refill = refill;
                if sp.inventory_mode == crate::action_identity::InventoryMode::Mirror { prefs.set_locked_from(&locks); }
                debt = exact_debt(carried, registry)?;
            }
            event => { fold(&mut window, &mut debt, holding_event(event, registry)?.as_ref()); }
        }
    }
    Ok(PackWindowStage { window: into_window(window), debt: exact_debt(&debt, registry)? })
}

fn delta_heap_bytes(delta: &crate::container_window::ItemDelta) -> Result<usize, super::strict::StrictStageError> {
    use super::strict::{item_heap_bytes, StrictStageError};
    let mut bytes = delta.take.capacity().checked_mul(std::mem::size_of::<(u8, ItemStack)>()).and_then(|n| delta.give.capacity().checked_mul(std::mem::size_of::<ItemStack>()).and_then(|give| n.checked_add(give))).ok_or(StrictStageError::Fit)?;
    for stack in delta.take.iter().map(|(_, stack)| stack).chain(delta.give.iter()) {
        bytes = bytes.checked_add(item_heap_bytes(&stack.item)?).ok_or(StrictStageError::Fit)?;
    }
    Ok(bytes)
}

/// Complete allocation accounting allows existing corrections beside a pack
/// fence, without relaxing Strict's separate admission/measurement guards.
fn event_heap_bytes(event: &WindowEvent) -> Result<usize, super::strict::StrictStageError> {
    use super::strict::{item_heap_bytes, replace_heap_bytes, view_heap_bytes, StrictStageError};
    match event {
        WindowEvent::Replace { window, debt, .. } | WindowEvent::PackCanonical { window, debt, .. } => replace_heap_bytes(window, debt),
        WindowEvent::Grant(stack) => item_heap_bytes(&stack.item),
        WindowEvent::Take { item, .. } => item_heap_bytes(item),
        WindowEvent::WearArmour { .. } | WindowEvent::WearWeapon { .. } | WindowEvent::PackView => Ok(0),
        #[cfg(not(target_arch = "wasm32"))]
        WindowEvent::PersonalSync(snapshot) => snapshot.bytes.capacity().checked_mul(2).and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<crate::protocol::PersonalSnapshot>())).ok_or(StrictStageError::Fit),
        WindowEvent::ContainerView(view) => view_heap_bytes(view, &item_heap_bytes),
        WindowEvent::Correction(c) => {
            let mut bytes = std::mem::size_of::<crate::window_ops::Correction>().checked_add(delta_heap_bytes(&c.client)?).and_then(|n| n.checked_add(view_heap_bytes(&c.view, &item_heap_bytes).ok()?)).ok_or(StrictStageError::Fit)?;
            if let Some(own) = &c.own { bytes = bytes.checked_add(delta_heap_bytes(own)?).ok_or(StrictStageError::Fit)?; }
            Ok(bytes)
        }
    }
}

pub fn prepare_pack_events(sp: &mut ServerPlayer, events: Box<[WindowEvent]>) -> Result<super::strict::PreparedEvents, PackStageError> {
    super::strict::prepare_events_with(sp, events, event_heap_bytes).map_err(|_| PackStageError::Fit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_identity::InventoryMode;
    use crate::container_window::{ContainerClick, ContainerData, CorrectionDebt, ItemDelta};

    fn player(mode: InventoryMode) -> ServerPlayer {
        let mut sp = ServerPlayer::new(glam::Vec3::ZERO);
        sp.server_simulated = true;
        sp.inventory_mode = mode;
        sp
    }
    fn ctx() -> crate::window::ClickCtx {
        crate::window::ClickCtx::new(false, crate::window::Station::Player, glam::Vec3::ZERO, |_| crate::block::AIR).with_shared(true)
    }
    fn correction(delta: ItemDelta) -> WindowEvent {
        WindowEvent::Correction(Box::new(crate::window_ops::Correction {
            client: delta.clone(), own: Some(delta),
            view: crate::window_ops::ViewEvent::Sets { sets: Vec::new(), furnace: None },
        }))
    }

    #[test]
    fn d2c2_pack_admission_preserves_unrelated_mirror_debt_and_pending_correction_in_both_modes() {
        let registry = crate::block::BlockRegistry::new();
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let mut sp = player(mode);
            sp.inventory.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 3)));
            let bread = Item::Material(crate::item::MaterialId::Bread);
            sp.window_events.debt = CorrectionDebt::from_entries(vec![(bread.clone(), 2)]).unwrap();
            sp.obligations.mark(crate::joiner_save::IncompleteCause::LegacyV1);
            let history = sp.obligations.get();
            queue(&mut sp, correction(ItemDelta { take: Vec::new(), give: vec![ItemStack { item: bread.clone(), count: 1 }] }), 0);
            let raw = pack_raw_marker(&sp, &registry).unwrap();
            let mut pack = ContainerData::Chest(crate::chest::ChestData::new());
            let staged = crate::staged_window::stage_container(&raw, &mut pack, &ContainerClick::Deposit { slot: 0, all: false }, &ctx());
            let result = admit_pack_staged(&sp, &registry, &raw, &staged).unwrap();
            assert_eq!(result.window.inv.slot(0).unwrap().count, 2);
            assert_eq!(result.debt.entries(), &[(bread.clone(), 1)]);
            assert_eq!(sp.window_events.debt.entries(), &[(bread, 2)], "admission changes no live ledger");
            assert_eq!(sp.inventory.slot(0).unwrap().count, 3);
            assert_eq!(sp.window_events.waiting(), 1);
            assert_eq!(sp.obligations.get(), history, "incomplete history is preserved without global Strict admission");
            let ContainerData::Chest(pack) = pack else { panic!("pack") };
            assert_eq!(pack.slots[0].as_ref().unwrap().count, 1);
        }
    }

    #[test]
    fn d2c2_pack_pending_take_then_grant_cannot_restore_a_deposit_source() {
        let registry = crate::block::BlockRegistry::new();
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let mut sp = player(mode);
            sp.inventory.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
            queue(&mut sp, WindowEvent::Take { slot: 0, item: Item::Block(crate::block::STONE), n: 1, how: "pack test" }, 0);
            queue(&mut sp, WindowEvent::Grant(ItemStack::new_block(crate::block::STONE, 1)), 0);
            let raw = pack_raw_marker(&sp, &registry).unwrap();
            let mut pack = ContainerData::Chest(crate::chest::ChestData::new());
            let staged = crate::staged_window::stage_container(&raw, &mut pack, &ContainerClick::Deposit { slot: 0, all: true }, &ctx());
            assert!(admit_pack_staged(&sp, &registry, &raw, &staged).is_err());
            assert_eq!(sp.inventory.slot(0).unwrap().count, 1);
            assert_eq!(sp.window_events.waiting(), 2);
        }
    }

    #[test]
    fn d2c2_pack_withdraw_cannot_redirect_earlier_tool_payment_to_new_cargo() {
        let registry = crate::block::BlockRegistry::new();
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let mut sp = player(mode);
            let mut old = crate::crafting::Tool::new(crate::crafting::ToolType::Bow, crate::crafting::ToolMaterial::Wood);
            old.durability = 5;
            let mut new = old;
            new.durability = 10;
            sp.inventory.set_slot(1, Some(ItemStack { item: Item::Tool(old), count: 1 }));
            queue(&mut sp, WindowEvent::Take { slot: 0, item: Item::Tool(old), n: 1, how: "pack tool payment" }, 0);
            let raw = pack_raw_marker(&sp, &registry).unwrap();
            let mut chest = crate::chest::ChestData::new();
            chest.slots[0] = Some(ItemStack { item: Item::Tool(new), count: 1 });
            let mut pack = ContainerData::Chest(chest);
            let staged = crate::staged_window::stage_container(&raw, &mut pack, &ContainerClick::Withdraw { slot: 0, all: true }, &ctx());
            assert_eq!(admit_pack_staged(&sp, &registry, &raw, &staged).unwrap_err(), PackStageError::Obligation);
            assert_eq!(sp.inventory.slot(1).unwrap().item, Item::Tool(old));
        }
    }

    #[test]
    fn d2c2_pack_canonical_event_preserves_forced_history_locks_and_debt() {
        let registry = crate::block::BlockRegistry::new();
        let mut sp = player(InventoryMode::Mirror);
        sp.inventory.toggle_lock(3);
        sp.inventory.auto_refill = true;
        sp.window_events.baseline = Some(false);
        sp.window_events.forced_corrections.push_back((1, ItemDelta::default()));
        let bread = Item::Material(crate::item::MaterialId::Bread);
        sp.window_events.debt = CorrectionDebt::from_entries(vec![(bread.clone(), 2)]).unwrap();
        let snapshot = pack_canonical_window(&sp, &registry).unwrap();
        let prepared = prepare_pack_events(&mut sp, vec![WindowEvent::PackCanonical { actor_respawns: 0, window: snapshot.window, debt: snapshot.debt }].into_boxed_slice()).unwrap();
        prepared.commit(&mut sp, 0);
        apply_front(&mut sp, 0, true);
        assert_eq!(sp.window_events.forced_corrections.len(), 1, "pack application cannot erase unrelated forced correction history");
        assert_eq!(sp.window_events.debt.entries(), &[(bread, 2)]);
        assert_eq!(sp.window_events.baseline, Some(false));
        assert!(sp.inventory.is_locked(3));
        assert!(sp.inventory.auto_refill);
        assert_eq!(sp.window_events.last_replace, 0, "a pack result is not a lifecycle replacement");
    }

    #[test]
    fn d2c2_pack_reservation_counts_pending_correction_allocations_without_global_strict_admission() {
        let registry = crate::block::BlockRegistry::new();
        let mut sp = player(InventoryMode::Mirror);
        queue(&mut sp, correction(ItemDelta { take: Vec::new(), give: vec![ItemStack::new_block(crate::block::STONE, 1)] }), 0);
        let snapshot = pack_canonical_window(&sp, &registry).unwrap();
        let prepared = prepare_pack_events(&mut sp, vec![WindowEvent::PackCanonical { actor_respawns: 0, window: snapshot.window, debt: snapshot.debt }].into_boxed_slice()).unwrap();
        assert_eq!(prepared.last_number(), 2);
        prepared.abandon(&mut sp);
        assert_eq!(sp.window_events.waiting(), 1);
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn d2c2_personal_fence_is_a_genuine_no_window_change_prefix_and_owned_bytes_are_bounded() {
        let registry = crate::block::BlockRegistry::new();
        let context = crate::personal_value::PersonalContext { participant: [7; 32], lineage: [1; 16], generation: 1, transfer: [4; 16] };
        let record = crate::handover::personal::PersonalRecord::from_maps(context, [], [], [], None).unwrap();
        let snapshot = crate::protocol::PersonalSnapshot { session: [8; 16], life: 0, revision: 1, window_event: 1, context, bytes: record.encode().unwrap() };
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let mut sp = player(mode);
            sp.inventory.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 3)));
            let event = queue(&mut sp, WindowEvent::PersonalSync(Box::new(snapshot.clone())), 0);
            assert_eq!(event, 1);
            let raw = pack_raw_marker(&sp, &registry).unwrap();
            let mut cargo = ContainerData::Chest(crate::chest::ChestData::new());
            let staged = crate::staged_window::stage_container(&raw, &mut cargo, &ContainerClick::Deposit { slot: 0, all: false }, &ctx());
            let admitted = admit_pack_staged(&sp, &registry, &raw, &staged).unwrap();
            assert_eq!(admitted.window.inv.slot(0).unwrap().count, 2, "progression cannot consume or fabricate a deposit source");
            let event = WindowEvent::PackCanonical { actor_respawns: 0, window: admitted.window, debt: admitted.debt };
            let prepared = prepare_pack_events(&mut sp, Box::new([event])).unwrap();
            assert_eq!(prepared.last_number(), 2, "the personal snapshot occupies its real preceding number");
            prepared.abandon(&mut sp);
            assert_eq!(sp.inventory.slot(0).unwrap().count, 3);
        }
        let mut oversized = snapshot;
        oversized.bytes = Vec::with_capacity(crate::action_journal::MAX_RETAINED_BYTES);
        oversized.bytes.push(1);
        let mut sp = player(InventoryMode::Mirror);
        assert!(prepare_pack_events(&mut sp, Box::new([WindowEvent::PersonalSync(Box::new(oversized))])).is_err(), "actual body capacity and retained delivery ownership cannot hide behind a one-byte populated snapshot");
        assert_eq!(sp.window_events.waiting(), 0);
    }

}
