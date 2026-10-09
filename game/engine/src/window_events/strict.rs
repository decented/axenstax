//! Strict admission uses the historical raw shared transition, never a future window click.
use super::*;
use crate::staged_window::{MarkerSlots, MarkerWindow, SourceBacking, StagedClick, PLAYER_PLACES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictStageError { Debt, Replacement, AmbiguousWear, MissingPayment, Source, Fit }

/// Staged raw result, admitted only while every original source survives each
/// existing pending prefix. The future fold is evidence, not committed state.
pub struct StrictWindowStage { pub raw: StagedClick }

fn marker_item(item: &Item) -> Item {
    match item {
        Item::Plan(body) => Item::Plan(crate::plan::PlanData::marker_placeholder(crate::plan::marker(body), body.develop_state.is_developed())),
        Item::Block(block) => Item::Block(*block),
        Item::Material(material) => Item::Material(*material),
        Item::Tool(tool) => Item::Tool(*tool),
        Item::Armour(armour) => Item::Armour(*armour),
    }
}

fn marker_stack(stack: &ItemStack, registry: &crate::block::BlockRegistry) -> ItemStack {
    crate::inventory::stack_from_wire(&crate::inventory::stack_to_wire(stack), registry, crate::inventory::PlanDecode::Marker)
        .expect("an existing stack has a valid marker representation")
}

/// Views do not change personal holdings. Replace/correction cannot establish
/// initial-source lineage for this attempt and must cross an applied rebase.
fn marker_event(event: &WindowEvent, registry: &crate::block::BlockRegistry) -> Result<Option<WindowEvent>, StrictStageError> {
    Ok(Some(match event {
        WindowEvent::Grant(stack) => WindowEvent::Grant(marker_stack(stack, registry)),
        WindowEvent::Take { slot, item, n, how } => WindowEvent::Take {
            slot: *slot, item: marker_item(item), n: *n, how: *how,
        },
        WindowEvent::WearArmour { hits } => WindowEvent::WearArmour { hits: *hits },
        WindowEvent::WearWeapon { slot, tool } => WindowEvent::WearWeapon { slot: *slot, tool: *tool },
        WindowEvent::ContainerView(_) => return Ok(None),
        WindowEvent::Replace(_) => return Err(StrictStageError::Replacement),
        WindowEvent::Correction(_) => return Err(StrictStageError::Debt),
    }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FoldFact { Grant(u8), Take(u8), Wear }

fn apply_marker_event(window: &mut MarkerWindow, event: &WindowEvent) -> Result<FoldFact, StrictStageError> {
    let view = window.view();
    if let WindowEvent::WearWeapon { slot, tool } = event {
        let item = Item::Tool(*tool);
        let matches = view.inv.slots_iter().flatten().filter(|stack| crate::joiner_actions::same_item(&stack.item, &item)).count();
        if matches != 1 || crate::joiner_actions::where_now(view.inv, *slot, &item).is_none() {
            return Err(StrictStageError::AmbiguousWear);
        }
    }
    let mut parts = Parts { inv: view.inv, armour: view.armour, cursor: view.cursor, grid: view.grid };
    match apply_to(&mut parts, &mut Default::default(), event) {
        Effect::Granted { landed } => Ok(FoldFact::Grant(landed)),
        Effect::Took { taken } => {
            if let WindowEvent::Take { n, .. } = event && taken != *n { return Err(StrictStageError::MissingPayment); }
            Ok(FoldFact::Take(taken))
        }
        Effect::Weapon(crate::joiner_inventory::WearCheck::Mismatched) => Err(StrictStageError::AmbiguousWear),
        Effect::Weapon(_) | Effect::Wore => Ok(FoldFact::Wear),
        Effect::Viewed | Effect::Corrected(_) => unreachable!("marker projection removes non-holding events"),
    }
}

pub fn stage_strict_click(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, click: &crate::window::WindowClick, ctx: &crate::window::ClickCtx) -> Result<StrictWindowStage, StrictStageError> {
    if !sp.window_events.debt.is_empty() { return Err(StrictStageError::Debt); }
    let raw = MarkerWindow::capture_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, registry);
    let staged = crate::staged_window::stage(&raw, click, ctx);
    let mut backing = SourceBacking::new(raw.slots());
    let mut prefix = raw;
    let mut candidate = staged.after.clone();
    for waiting in &sp.window_events.waiting {
        let Some(event) = marker_event(&waiting.event, registry)? else { continue };
        let before = prefix.slots();
        let expected = apply_marker_event(&mut prefix, &event)?;
        let after = prefix.slots();
        if matches!(event, WindowEvent::WearWeapon { .. } | WindowEvent::WearArmour { .. }) {
            backing.observe_authorised_wear(&before, &after);
        }
        backing.observe_prefix(&after);
        if apply_marker_event(&mut candidate, &event)? != expected { return Err(StrictStageError::Fit); }
    }
    if !backing.backs(&staged.sources) { return Err(StrictStageError::Source); }
    Ok(StrictWindowStage { raw: staged })
}

/// Existing pending events are folded by the real owed rule. Callers use the
/// surviving piece for capability, while the historical slot supplies heldness.
pub fn strict_effective_marker(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> Result<MarkerWindow, StrictStageError> {
    if !sp.window_events.debt.is_empty() { return Err(StrictStageError::Debt); }
    let mut prefix = MarkerWindow::capture_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, registry);
    for waiting in &sp.window_events.waiting {
        if let Some(event) = marker_event(&waiting.event, registry)? { apply_marker_event(&mut prefix, &event)?; }
    }
    Ok(prefix)
}

/// All immediate takes/wear/grants are staged together. No external effect or
/// raw mutation is permitted merely because an earlier effect was affordable.
pub fn stage_strict_effects(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, all: &[WindowEvent]) -> Result<(), StrictStageError> {
    if !sp.window_events.debt.is_empty() { return Err(StrictStageError::Debt); }
    let raw = MarkerWindow::capture_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, registry);
    let before = raw.slots();
    let mut candidate = raw.clone();
    let mut sources: MarkerSlots = std::array::from_fn(|_| None);
    let mut raw_facts = Vec::with_capacity(all.len());
    for original in all {
        let event = marker_event(original, registry)?.ok_or(StrictStageError::Replacement)?;
        let prior = candidate.slots();
        let wear_at = if let WindowEvent::WearWeapon { slot, tool } = &event {
            crate::joiner_actions::where_now(candidate.view().inv, *slot, &Item::Tool(*tool))
        } else { None };
        raw_facts.push(apply_marker_event(&mut candidate, &event)?);
        let after = candidate.slots();
        match event {
            WindowEvent::Take { .. } => {
                for i in 0..PLAYER_PLACES {
                    let removed = prior[i].as_ref().map_or(0, |s| s.count).saturating_sub(after[i].as_ref().map_or(0, |s| s.count));
                    if removed > 0 && let Some(initial) = &before[i] {
                        let source = sources[i].get_or_insert_with(|| { let mut s = initial.clone(); s.count = 0; s });
                        source.count = source.count.checked_add(removed).ok_or(StrictStageError::Source)?;
                    }
                }
            }
            WindowEvent::WearWeapon { .. } => {
                let at = wear_at.ok_or(StrictStageError::AmbiguousWear)?;
                sources[at] = before[at].clone();
            }
            WindowEvent::WearArmour { .. } => { sources[36..40].clone_from_slice(&before[36..40]); }
            WindowEvent::Grant(_) => {},
            _ => return Err(StrictStageError::Replacement),
        }
    }
    let mut prefix = raw;
    let mut backing = SourceBacking::new(before);
    for waiting in &sp.window_events.waiting {
        let Some(event) = marker_event(&waiting.event, registry)? else { continue };
        let previous = prefix.slots();
        let expected = apply_marker_event(&mut prefix, &event)?;
        let after = prefix.slots();
        if matches!(event, WindowEvent::WearWeapon { .. } | WindowEvent::WearArmour { .. }) { backing.observe_authorised_wear(&previous, &after); }
        backing.observe_prefix(&after);
        if apply_marker_event(&mut candidate, &event)? != expected { return Err(StrictStageError::Fit); }
    }
    if !backing.backs(&sources) { return Err(StrictStageError::Source); }
    // New events really follow the existing pending prefix. This second fold
    // proves they remain payable there without materialising it into raw R.
    for (original, expected) in all.iter().zip(raw_facts) {
        let event = marker_event(original, registry)?.ok_or(StrictStageError::Replacement)?;
        if apply_marker_event(&mut prefix, &event)? != expected { return Err(StrictStageError::Fit); }
    }
    Ok(())
}

/// The complete numbered list owns its allocations before the caller commits
/// an external effect. The waiting deque's capacity is reserved at this point.
pub struct PreparedEvents {
    expected_last: u32,
    expected_waiting: usize,
    events: Box<[WindowEvent]>,
    last: u32,
}

fn event_heap_bytes(event: &WindowEvent) -> Result<usize, StrictStageError> {
    let item_bytes = |item: &Item| -> Result<usize, StrictStageError> {
        match item {
            Item::Plan(body) => crate::body_residency::allocated_body_bytes(body).map(|bytes| bytes - std::mem::size_of::<crate::plan::PlanData>()).map_err(|_| StrictStageError::Fit),
            _ => Ok(0),
        }
    };
    match event {
        WindowEvent::Grant(stack) => item_bytes(&stack.item),
        WindowEvent::Take { item, .. } => item_bytes(item),
        WindowEvent::WearArmour { .. } | WindowEvent::WearWeapon { .. } => Ok(0),
        // These require their own complete typed allocation reservation. Never
        // treat unknown correction/view allocations as a zero-byte event.
        WindowEvent::Replace(_) => Err(StrictStageError::Replacement),
        WindowEvent::Correction(_) => Err(StrictStageError::Debt),
        WindowEvent::ContainerView(_) => Err(StrictStageError::Fit),
    }
}

pub fn prepare_strict_events(sp: &mut ServerPlayer, events: Box<[WindowEvent]>) -> Result<PreparedEvents, StrictStageError> {
    let waiting = &mut sp.window_events;
    let count = waiting.waiting.len().checked_add(events.len()).ok_or(StrictStageError::Fit)?;
    if count > crate::action_journal::MAX_EVENT_ENTRIES { return Err(StrictStageError::Fit); }
    let last = waiting.last_sent.checked_add(u32::try_from(events.len()).map_err(|_| StrictStageError::Fit)?).ok_or(StrictStageError::Fit)?;
    let mut heap = 0usize;
    for event in waiting.waiting.iter().map(|w| &w.event).chain(events.iter()) {
        heap = heap.checked_add(event_heap_bytes(event)?).ok_or(StrictStageError::Fit)?;
    }
    let estimated_capacity = waiting.waiting.capacity().max(count);
    let staged = events.len().checked_mul(std::mem::size_of::<WindowEvent>()).ok_or(StrictStageError::Fit)?;
    let fits = |capacity: usize| capacity.checked_mul(std::mem::size_of::<Waiting>()).and_then(|n| n.checked_add(heap)).and_then(|n| n.checked_add(staged)).is_some_and(|n| n <= crate::action_journal::MAX_RETAINED_BYTES);
    if !fits(estimated_capacity) { return Err(StrictStageError::Fit); }
    waiting.waiting.try_reserve_exact(events.len()).map_err(|_| StrictStageError::Fit)?;
    if !fits(waiting.waiting.capacity()) { return Err(StrictStageError::Fit); }
    Ok(PreparedEvents { expected_last: waiting.last_sent, expected_waiting: waiting.waiting.len(), events, last })
}

impl PreparedEvents {
    pub fn last_number(&self) -> u32 { self.last }
    pub fn commit(self, sp: &mut ServerPlayer, now: u64) -> u32 {
        let waiting = &mut sp.window_events;
        assert_eq!(waiting.last_sent, self.expected_last, "prepared events were invalidated before external commit");
        assert_eq!(waiting.waiting.len(), self.expected_waiting, "prepared event prefix was changed before external commit");
        // Capacity and checked sequence were reserved for the WHOLE list. This
        // loop allocates nothing and never forces an older event to apply.
        for event in self.events.into_vec() {
            waiting.last_sent += 1;
            waiting.tally.sent = waiting.tally.sent.saturating_add(1);
            waiting.waiting.push_back(Waiting { seq: waiting.last_sent, sent: now, event });
        }
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn player() -> ServerPlayer { ServerPlayer::new(glam::Vec3::ZERO) }
    fn ctx() -> crate::window::ClickCtx { crate::window::ClickCtx::new(false, crate::window::Station::Player, glam::Vec3::ZERO, |_| crate::block::AIR) }
    #[test]
    fn c3d_actual_pending_take_and_later_grant_cannot_back_the_old_window_source() {
        let mut sp = player();
        sp.inventory.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
        sp.inventory.set_slot(1, Some(ItemStack::new_block(crate::block::STONE, 64)));
        queue(&mut sp, WindowEvent::Take { slot: 0, item: Item::Block(crate::block::STONE), n: 1, how: "test owed take" }, 0);
        queue(&mut sp, WindowEvent::Grant(ItemStack::new_block(crate::block::STONE, 1)), 0);
        let registry = crate::block::BlockRegistry::new();
        assert!(matches!(stage_strict_click(&sp, &registry, &crate::window::WindowClick::Slot { slot: 0, right: false }, &ctx()), Err(StrictStageError::Source)));
        assert_eq!(sp.inventory.slot(0).unwrap().count, 1);
        assert!(sp.cursor.is_none());
        assert_eq!(sp.window_events.waiting.len(), 2);
    }
    #[test]
    fn c3d_actual_raw_putdown_preserves_the_future_grant_instead_of_swapping() {
        let mut sp = player();
        sp.cursor = Some(ItemStack::new_block(crate::block::DIRT, 1));
        queue(&mut sp, WindowEvent::Grant(ItemStack::new_block(crate::block::STONE, 1)), 0);
        let mut staged = stage_strict_click(&sp, &crate::block::BlockRegistry::new(), &crate::window::WindowClick::Slot { slot: 0, right: false }, &ctx()).unwrap();
        assert!(staged.raw.after.view().cursor.is_none());
        assert_eq!(staged.raw.after.view().inv.slot(0).unwrap().item, Item::Block(crate::block::DIRT));
        assert!(sp.inventory.slot(0).is_none(), "proof must not materialise the pending grant into raw R");
    }
    #[test]
    fn c3d_actual_authorised_pending_wear_backs_only_the_same_surviving_piece() {
        let mut sp = player();
        let mut tool = crate::crafting::Tool::new(crate::crafting::ToolType::Bow, crate::crafting::ToolMaterial::Wood);
        tool.durability = 3;
        sp.inventory.set_slot(0, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
        queue(&mut sp, WindowEvent::WearWeapon { slot: 0, tool }, 0);
        let registry = crate::block::BlockRegistry::new();
        let mut effective = strict_effective_marker(&sp, &registry).unwrap();
        assert!(matches!(effective.view().inv.slot(0).unwrap().item, Item::Tool(t) if t.durability == 2));
        assert!(stage_strict_effects(&sp, &registry, &[WindowEvent::WearWeapon { slot: 0, tool }]).is_ok());
        assert!(matches!(sp.inventory.slot(0).unwrap().item, Item::Tool(t) if t.durability == 3));
        sp.inventory.set_slot(1, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
        assert!(matches!(strict_effective_marker(&sp, &registry), Err(StrictStageError::AmbiguousWear)));
        sp.inventory.set_slot(1, None);
        sp.inventory.set_slot(0, Some(ItemStack { item: Item::Tool(crate::crafting::Tool { durability: 1, ..tool }), count: 1 }));
        assert!(matches!(stage_strict_effects(&sp, &registry, &[WindowEvent::WearWeapon { slot: 0, tool }]), Err(StrictStageError::Source | StrictStageError::AmbiguousWear)));
    }
    #[test]
    fn c3d_actual_close_pending_product_fit_crossing_leaves_all_raw_places_intact() {
        let mut sp = player();
        for slot in 1..36 { sp.inventory.set_slot(slot, Some(ItemStack::new_block(crate::block::STONE, 64))); }
        sp.cursor = Some(ItemStack::new_block(crate::block::OAK_LOG, 1));
        queue(&mut sp, WindowEvent::Grant(ItemStack::new_block(crate::block::DIRT, 1)), 0);
        assert!(matches!(stage_strict_click(&sp, &crate::block::BlockRegistry::new(), &crate::window::WindowClick::Close, &ctx()), Err(StrictStageError::Fit)));
        assert!(sp.inventory.slot(0).is_none());
        assert_eq!(sp.cursor.as_ref().unwrap().item, Item::Block(crate::block::OAK_LOG));
        assert!(sp.window_events.debt.is_empty());
    }
    #[test]
    fn c3d_actual_event_deque_reserves_both_shot_effects_without_forcing_an_old_event() {
        let mut sp = player();
        for _ in 0..1023 { queue(&mut sp, WindowEvent::WearArmour { hits: 1 }, 0); }
        let events = vec![WindowEvent::Take { slot: 0, item: Item::Block(crate::block::STONE), n: 1, how: "test" }, WindowEvent::WearArmour { hits: 1 }].into_boxed_slice();
        assert!(matches!(prepare_strict_events(&mut sp, events), Err(StrictStageError::Fit)));
        assert_eq!(sp.window_events.waiting.len(), 1023);
        assert_eq!(sp.window_events.last_number(), 1023);
        assert_eq!(sp.window_events.tally.forced, 0);
        assert!(sp.window_events.debt.is_empty());
    }
}
