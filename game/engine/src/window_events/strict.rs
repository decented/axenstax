//! Strict admission uses the historical raw shared transition, never a future window click.
use super::*;
use crate::staged_window::{MarkerSlots, MarkerWindow, SourceBacking, StagedClick, PLAYER_PLACES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrictStageError { Debt, Replacement, AmbiguousWear, MissingPayment, Source, Fit,
    /// A place or pending stack the marker projection can't carry exactly
    /// (C3d review L1): refused with a typed fate, never dropped or clamped.
    /// A property of the action's result or of a pending event: corrected.
    Unrepresentable,
    /// C3d run 13 (run-12 review LOW-2) — the server's OWN raw window (or a
    /// waiting `Replace`'s) can't round-trip the marker projection: a
    /// correction would ship the same window back, so the caller refuses,
    /// then tears down (`StrictTeardown::Unrepresentable`).
    RawUnrepresentable }

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

fn marker_stack(stack: &ItemStack, registry: &crate::block::BlockRegistry) -> Result<ItemStack, StrictStageError> {
    crate::staged_window::exact_marker(stack, registry).ok_or(StrictStageError::Unrepresentable)
}

fn raw_marker(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> Result<MarkerWindow, StrictStageError> {
    MarkerWindow::try_capture_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, registry).ok_or(StrictStageError::RawUnrepresentable)
}

/// Views do not change personal holdings. Replace/correction cannot establish
/// initial-source lineage for this attempt and must cross an applied rebase.
fn marker_event(event: &WindowEvent, registry: &crate::block::BlockRegistry) -> Result<Option<WindowEvent>, StrictStageError> {
    Ok(Some(match event {
        WindowEvent::Grant(stack) => WindowEvent::Grant(marker_stack(stack, registry)?),
        WindowEvent::Take { slot, item, n, how } => WindowEvent::Take {
            slot: *slot, item: marker_item(item), n: *n, how,
        },
        WindowEvent::WearArmour { hits } => WindowEvent::WearArmour { hits: *hits },
        WindowEvent::WearWeapon { slot, tool } => WindowEvent::WearWeapon { slot: *slot, tool: *tool },
        WindowEvent::ContainerView(_) | WindowEvent::PackView => return Ok(None),
        #[cfg(not(target_arch = "wasm32"))]
        WindowEvent::PersonalSync(_) => return Ok(None),
        WindowEvent::Replace { .. } | WindowEvent::PackCanonical { .. } => return Err(StrictStageError::Replacement),
        WindowEvent::Correction(_) => return Err(StrictStageError::Debt),
    }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FoldFact { Grant(u8), Take(u8), Wear }

fn apply_marker_event(window: &mut MarkerWindow, event: &WindowEvent) -> Result<FoldFact, StrictStageError> {
    let view = window.view();
    if let WindowEvent::WearWeapon { slot, tool } = event {
        let item = Item::Tool(*tool);
        // C3d run 16 (run-15 review MEDIUM-1) — the named held slot first;
        // ambiguous only when it doesn't hold the kind and several do.
        if crate::joiner_actions::strict_wear_target(view.inv, *slot, &item).is_none() {
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

/// `ctx` and `client_ok` are the server's EXISTING transition
/// (`window_ops::server_click_ctx` / `server_click`, C3d review M2).
pub fn stage_strict_click(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, click: &crate::window::WindowClick, ctx: &crate::window::ClickCtx, client_ok: bool) -> Result<StrictWindowStage, StrictStageError> {
    let raw = strict_raw_marker(sp, registry)?;
    let staged = crate::staged_window::stage_with(&raw, click, ctx.station, |view| crate::window_ops::server_click(view, click, ctx, client_ok));
    admit_staged(sp, registry, &raw, &staged)?;
    Ok(StrictWindowStage { raw: staged })
}

/// Raw R at the action's boundary as marker-only scratch holdings,
/// `RawUnrepresentable` when a place can't round-trip exactly (review L1;
/// run-12 review LOW-2).
pub fn strict_raw_marker(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> Result<MarkerWindow, StrictStageError> {
    strict_obligations(sp, registry)?;
    raw_marker(sp, registry)
}

/// Run-2 review LOW-2 — a Strict Autofill the client REFUSED creates
/// nothing: its client either moved nothing (`NeedsTable`: out of its exact
/// reach, where the server's half-block slack may still see the table) or
/// only returned the grid and cursor to the bag (`Refused`: step 1). Strict
/// stages exactly the safe subset whose digest is the client's; `None` when
/// neither is (a deterministic refusal).
pub fn stage_strict_refused_autofill(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, click: &crate::window::WindowClick, digest: u32) -> Result<Option<StrictWindowStage>, StrictStageError> {
    let raw = strict_raw_marker(sp, registry)?;
    for returned in [false, true] {
        let staged = crate::staged_window::stage_with(&raw, click, sp.station, |view| {
            if returned { crate::window::autofill_return(view); crate::window::ClickResult::Refused } else { crate::window::ClickResult::NeedsTable }
        });
        let station = crate::window::station_after(sp.station, click, &staged.result);
        if staged.after.digest_with(station, None) == digest {
            admit_staged(sp, registry, &raw, &staged)?;
            return Ok(Some(StrictWindowStage { raw: staged }));
        }
    }
    Ok(None)
}

/// Admit a staged raw transition (a window click or a shared container
/// click): every initial source survives each existing pending prefix, and
/// each pending event folds onto the candidate exactly as onto raw R.
pub fn admit_staged(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, raw: &MarkerWindow, staged: &StagedClick) -> Result<(), StrictStageError> {
    let mut backing = SourceBacking::new(raw.slots());
    let mut prefix = raw.clone();
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
    Ok(())
}

/// Existing pending events are folded by the real owed rule. Callers use the
/// surviving piece for capability, while the historical slot supplies heldness.
pub fn strict_effective_marker(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> Result<MarkerWindow, StrictStageError> {
    strict_obligations(sp, registry)
}

/// Decision 10 (run-2 review LOW-1) — the ONE predicate "Strict leaves no
/// unpaid obligation", used both at staging and by the post-commit guard:
/// the effective fold proves zero (correction debt, pending corrections,
/// session history: `ServerPlayer::obligations_proven_zero`) AND every
/// waiting event folds payably onto raw R (a pending Take the window can't
/// pay is an unpaid obligation, `Debt`). Returns the effective window.
pub fn strict_obligations(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> Result<MarkerWindow, StrictStageError> {
    if !sp.window_events.debt.is_empty() || !sp.obligations_proven_zero() { return Err(StrictStageError::Debt); }
    let mut prefix = raw_marker(sp, registry)?;
    for waiting in &sp.window_events.waiting {
        // Run 4 (run-3 review M1) — a waiting whole-window replacement (the
        // join or death sync) is no obligation: the fold continues from the
        // window it carries, whose carried debt must be empty (Strict
        // admission proved the record; a death's is empty). Source lineage
        // across it still can't be established (`admit_staged`).
        if let WindowEvent::Replace { window, debt, .. } = &waiting.event {
            if !debt.is_empty() { return Err(StrictStageError::Debt); }
            prefix = MarkerWindow::try_capture_parts(&window.inv, &window.armour, &window.cursor, &window.grid, registry).ok_or(StrictStageError::RawUnrepresentable)?;
            // Run 6 (run-4 review Nit) — one source with the applied
            // `Replace`: its locks, the raw copy's auto-refill.
            prefix.set_auto_refill(sp.inventory.auto_refill);
            continue;
        }
        if let Some(event) = marker_event(&waiting.event, registry)? {
            apply_marker_event(&mut prefix, &event).map_err(|e| if e == StrictStageError::MissingPayment { StrictStageError::Debt } else { e })?;
        }
    }
    Ok(prefix)
}

/// [`strict_obligations`] as the post-commit guard reads it.
pub fn strict_leaves_no_unpaid_obligation(sp: &ServerPlayer, registry: &crate::block::BlockRegistry) -> bool {
    strict_obligations(sp, registry).is_ok()
}

/// All immediate takes/wear/grants are staged together. No external effect or
/// raw mutation is permitted merely because an earlier effect was affordable.
pub fn stage_strict_effects(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, all: &[WindowEvent]) -> Result<(), StrictStageError> {
    stage_strict_effects_on(sp, registry, strict_raw_marker(sp, registry)?, all)
}

/// [`stage_strict_effects`] over `raw`: raw R itself, or (a typed edit) raw R
/// with the edit's own predicted change already staged on it.
fn stage_strict_effects_on(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, raw: MarkerWindow, all: &[WindowEvent]) -> Result<(), StrictStageError> {
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

/// C3d run 6 — a typed block edit's window half. The client PREDICTS its
/// edit at its boundary (a placement's `take_placeable_from_hotbar`, auto-
/// refill included; a break's `use_tool_at`), so the server stages that
/// EXISTING shared transition on raw R exactly as a window click does
/// (Decision 6; `stage_strict_click`): every place it changed is a source at
/// its whole initial value, and every waiting event must fold onto the
/// candidate as onto raw R (`admit_staged`). A numbered `Take` would not do:
/// `take_owed_window` never refills, and it would land after the pending
/// prefix the client predicted before. The break's yield (`grants`) is not
/// predicted: numbered grants, staged after the pending prefix on the
/// candidate. Returns the candidate the caller commits as raw R.
pub fn stage_strict_edit(sp: &ServerPlayer, registry: &crate::block::BlockRegistry, direct: impl FnOnce(&mut crate::window::WindowMut), grants: &[WindowEvent]) -> Result<MarkerWindow, StrictStageError> {
    let raw = strict_raw_marker(sp, registry)?;
    let mut after = raw.clone();
    direct(&mut after.view());
    let (before, now) = (raw.slots(), after.slots());
    let mut sources: MarkerSlots = std::array::from_fn(|_| None);
    for i in 0..PLAYER_PLACES { if before[i] != now[i] { sources[i] = before[i].clone(); } }
    let staged = StagedClick { after, result: crate::window::ClickResult::Done, sources };
    admit_staged(sp, registry, &raw, &staged)?;
    stage_strict_effects_on(sp, registry, staged.after.clone(), grants)?;
    Ok(staged.after)
}

/// The complete numbered list owns its allocations before the caller commits
/// an external effect. The waiting deque's capacity is reserved at this point.
pub struct PreparedEvents {
    expected_last: u32,
    expected_waiting: usize,
    /// The deque's capacity before this reservation: an abandoned
    /// reservation gives back exactly what it took.
    capacity_before: usize,
    events: Box<[WindowEvent]>,
    last: u32,
}

/// The heap one item owns beyond its inline size: a Plan's body.
pub(super) fn item_heap_bytes(item: &Item) -> Result<usize, StrictStageError> {
    match item {
        Item::Plan(body) => crate::body_residency::allocated_body_bytes(body).map(|bytes| bytes - std::mem::size_of::<crate::plan::PlanData>()).map_err(|_| StrictStageError::Fit),
        _ => Ok(0),
    }
}

fn event_heap_bytes(event: &WindowEvent) -> Result<usize, StrictStageError> {
    let item_bytes = item_heap_bytes;
    match event {
        WindowEvent::Grant(stack) => item_bytes(&stack.item),
        WindowEvent::Take { item, .. } => item_bytes(item),
        WindowEvent::WearArmour { .. } | WindowEvent::WearWeapon { .. } | WindowEvent::PackView => Ok(0),
        // These require their own complete typed allocation reservation. Never
        // treat unknown correction/view allocations as a zero-byte event.
        WindowEvent::Replace { .. } | WindowEvent::PackCanonical { .. } => Err(StrictStageError::Replacement),
        WindowEvent::Correction(_) => Err(StrictStageError::Debt),
        // C3d — a waiting view's real owned bytes count against the budget
        // (it never makes an unrelated reservation refuse).
        WindowEvent::ContainerView(view) => view_heap_bytes(view, &item_bytes),
        #[cfg(not(target_arch = "wasm32"))]
        WindowEvent::PersonalSync(snapshot) => snapshot.bytes.capacity().checked_mul(2).and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<crate::protocol::PersonalSnapshot>())).ok_or(StrictStageError::Fit),
    }
}

/// The heap a container view owns: its slot storage and any Plan body in it.
pub(super) fn view_heap_bytes(view: &crate::window_ops::ViewEvent, item_bytes: &dyn Fn(&Item) -> Result<usize, StrictStageError>) -> Result<usize, StrictStageError> {
    use crate::container_window::ContainerData;
    let mut total = 0usize;
    let mut add = |n: usize| -> Result<(), StrictStageError> { total = total.checked_add(n).ok_or(StrictStageError::Fit)?; Ok(()) };
    match view {
        crate::window_ops::ViewEvent::Opened(mirror) => {
            match &mirror.contents {
                ContainerData::Chest(chest) => add(chest.slots.capacity().checked_mul(std::mem::size_of::<Option<ItemStack>>()).ok_or(StrictStageError::Fit)?)?,
                ContainerData::Furnace(_) => add(std::mem::size_of::<crate::furnace::FurnaceData>())?,
            }
            let contents = mirror.contents.as_ref();
            for i in 0..contents.len() {
                if let Some(stack) = contents.get(i) { add(item_bytes(&stack.item)?)?; }
            }
        }
        crate::window_ops::ViewEvent::Sets { sets, .. } => {
            add(sets.capacity().checked_mul(std::mem::size_of::<(usize, Option<ItemStack>)>()).ok_or(StrictStageError::Fit)?)?;
            for stack in sets.iter().filter_map(|(_, s)| s.as_ref()) { add(item_bytes(&stack.item)?)?; }
        }
    }
    Ok(total)
}

/// C3d run 12b (S1) — a whole-window `Replace`'s real owned heap: its boxed
/// window, every Plan body in it, and its carried debt's entries. Only the
/// pending-window bound ([`strict_replace_fits`]) measures a `Replace` this
/// way; staging still never crosses a waiting one ([`event_heap_bytes`]
/// keeps refusing it, `Replacement`).
pub(super) fn replace_heap_bytes(window: &EffectiveWindow, debt: &crate::container_window::CorrectionDebt) -> Result<usize, StrictStageError> {
    let mut total = std::mem::size_of::<EffectiveWindow>();
    let mut add = |n: usize| -> Result<(), StrictStageError> { total = total.checked_add(n).ok_or(StrictStageError::Fit)?; Ok(()) };
    let grid = window.grid.iter().flatten().flatten();
    for stack in window.inv.slots_iter().flatten().chain(window.cursor.iter()).chain(grid) { add(item_heap_bytes(&stack.item)?)?; }
    add(debt.allocated_entries().checked_mul(std::mem::size_of::<(Item, u32)>()).ok_or(StrictStageError::Fit)?)?;
    for (item, _) in debt.entries() { add(item_heap_bytes(item)?)?; }
    Ok(total)
}

/// C3d run 12 (S1) — would one more numbered `event` (a live correction
/// `Replace`) fit joiner `sp`'s pending window events: the count and
/// retained-bytes caps [`prepare_strict_events`] checks (1024 events,
/// 2 MiB)? `queue` would otherwise force the oldest waiting event applied.
/// A `Replace` (the probe, or a join/death sync still waiting) counts its
/// real owned heap ([`replace_heap_bytes`]); a waiting container
/// `Correction` has no reservation measure, so nothing fits beside it
/// (fail closed: the caller tears down).
pub fn strict_replace_fits(sp: &ServerPlayer, event: &WindowEvent) -> bool {
    let waiting = &sp.window_events;
    let count = waiting.waiting.len().saturating_add(1);
    if count > crate::action_journal::MAX_EVENT_ENTRIES { return false; }
    let bytes = |e: &WindowEvent| match e {
        WindowEvent::Replace { window, debt, .. } | WindowEvent::PackCanonical { window, debt, .. } => replace_heap_bytes(window, debt),
        other => event_heap_bytes(other),
    };
    let mut heap = 0usize;
    for e in waiting.waiting.iter().map(|w| &w.event).chain(std::iter::once(event)) {
        match bytes(e).ok().and_then(|n| heap.checked_add(n)) { Some(h) => heap = h, None => return false }
    }
    count.max(waiting.waiting.capacity()).checked_mul(std::mem::size_of::<Waiting>()).and_then(|n| n.checked_add(heap)).is_some_and(|n| n <= crate::action_journal::MAX_RETAINED_BYTES)
}

/// C3d run 13 (run-12 review LOW-1) — reserve room for one live correction
/// `Replace` (`probe`) in joiner `sp`'s pending window events, as
/// [`prepare_strict_events`] does: the deque's slot is reserved first, then
/// the count and byte bound are measured on its REAL capacity (so the
/// `push_back` that queues it can't grow it past 2 MiB). On `false` the
/// capacity is restored and nothing is queued (the caller tears down).
pub fn reserve_strict_replace(sp: &mut ServerPlayer, probe: &WindowEvent) -> bool {
    let capacity_before = sp.window_events.waiting.capacity();
    if sp.window_events.waiting.try_reserve_exact(1).is_ok() && strict_replace_fits(sp, probe) { return true; }
    sp.window_events.waiting.shrink_to(capacity_before);
    false
}

pub fn prepare_strict_events(sp: &mut ServerPlayer, events: Box<[WindowEvent]>) -> Result<PreparedEvents, StrictStageError> {
    prepare_events_with(sp, events, event_heap_bytes)
}

/// Allocation reservation is shared; each admission path measures only the
/// event forms it can carry. Strict's correction guards remain unchanged.
pub(super) fn prepare_events_with(sp: &mut ServerPlayer, events: Box<[WindowEvent]>, measure: impl Fn(&WindowEvent) -> Result<usize, StrictStageError>) -> Result<PreparedEvents, StrictStageError> {
    let waiting = &mut sp.window_events;
    let count = waiting.waiting.len().checked_add(events.len()).ok_or(StrictStageError::Fit)?;
    if count > crate::action_journal::MAX_EVENT_ENTRIES { return Err(StrictStageError::Fit); }
    let last = waiting.last_sent.checked_add(u32::try_from(events.len()).map_err(|_| StrictStageError::Fit)?).ok_or(StrictStageError::Fit)?;
    let mut heap = 0usize;
    for event in waiting.waiting.iter().map(|w| &w.event).chain(events.iter()) {
        heap = heap.checked_add(measure(event)?).ok_or(StrictStageError::Fit)?;
    }
    let estimated_capacity = waiting.waiting.capacity().max(count);
    let staged = events.len().checked_mul(std::mem::size_of::<WindowEvent>()).ok_or(StrictStageError::Fit)?;
    let fits = |capacity: usize| capacity.checked_mul(std::mem::size_of::<Waiting>()).and_then(|n| n.checked_add(heap)).and_then(|n| n.checked_add(staged)).is_some_and(|n| n <= crate::action_journal::MAX_RETAINED_BYTES);
    if !fits(estimated_capacity) { return Err(StrictStageError::Fit); }
    let capacity_before = waiting.waiting.capacity();
    waiting.waiting.try_reserve_exact(events.len()).map_err(|_| StrictStageError::Fit)?;
    if !fits(waiting.waiting.capacity()) {
        waiting.waiting.shrink_to(capacity_before);
        return Err(StrictStageError::Fit);
    }
    Ok(PreparedEvents { expected_last: waiting.last_sent, expected_waiting: waiting.waiting.len(), capacity_before, events, last })
}

impl PreparedEvents {
    pub fn last_number(&self) -> u32 { self.last }
    /// A later reservation of the same action failed: nothing is numbered,
    /// queued or kept, and the deque returns to its capacity before.
    pub fn abandon(self, sp: &mut ServerPlayer) {
        let waiting = &mut sp.window_events;
        assert_eq!(waiting.last_sent, self.expected_last, "prepared events were invalidated before abandonment");
        waiting.waiting.shrink_to(self.capacity_before);
    }
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
        assert!(matches!(stage_strict_click(&sp, &registry, &crate::window::WindowClick::Slot { slot: 0, right: false }, &ctx(), true), Err(StrictStageError::Source)));
        assert_eq!(sp.inventory.slot(0).unwrap().count, 1);
        assert!(sp.cursor.is_none());
        assert_eq!(sp.window_events.waiting.len(), 2);
    }
    #[test]
    fn c3d_actual_raw_putdown_preserves_the_future_grant_instead_of_swapping() {
        let mut sp = player();
        sp.cursor = Some(ItemStack::new_block(crate::block::DIRT, 1));
        queue(&mut sp, WindowEvent::Grant(ItemStack::new_block(crate::block::STONE, 1)), 0);
        let mut staged = stage_strict_click(&sp, &crate::block::BlockRegistry::new(), &crate::window::WindowClick::Slot { slot: 0, right: false }, &ctx(), true).unwrap();
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
        // Run 16 (run-15 review MEDIUM-1) — a second piece of the kind
        // elsewhere: the named slot still holds one, so it is not ambiguous
        // and the named piece wears; ambiguous only once the named slot
        // doesn't hold the kind and two others do.
        sp.inventory.set_slot(1, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
        let mut effective = strict_effective_marker(&sp, &registry).expect("the named slot resolves the wear");
        assert!(matches!(effective.view().inv.slot(0).unwrap().item, Item::Tool(t) if t.durability == 2), "the named piece wore");
        assert!(matches!(effective.view().inv.slot(1).unwrap().item, Item::Tool(t) if t.durability == 3), "the spare didn't");
        let named = sp.inventory.slot(0).cloned();
        sp.inventory.set_slot(0, None);
        sp.inventory.set_slot(2, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
        assert!(matches!(strict_effective_marker(&sp, &registry), Err(StrictStageError::AmbiguousWear)), "named slot empty, two elsewhere");
        sp.inventory.set_slot(2, None);
        sp.inventory.set_slot(0, named);
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
        assert!(matches!(stage_strict_click(&sp, &crate::block::BlockRegistry::new(), &crate::window::WindowClick::Close, &ctx(), true), Err(StrictStageError::Fit)));
        assert!(sp.inventory.slot(0).is_none());
        assert_eq!(sp.cursor.as_ref().unwrap().item, Item::Block(crate::block::OAK_LOG));
        assert!(sp.window_events.debt.is_empty());
    }
    /// Run 13 (run-12 review LOW-2) — the two `Unrepresentable`s are told
    /// apart: the server's own raw window is `RawUnrepresentable` (refused,
    /// then torn down); an action's result (here a yield grant) is
    /// `Unrepresentable` (corrected).
    #[test]
    fn c3d_raw_and_result_unrepresentable_are_distinct() {
        let registry = crate::block::BlockRegistry::new();
        let mut over = crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Stone);
        over.durability = over.max_durability() + 5;
        let over = ItemStack { item: Item::Tool(over), count: 1 };
        let mut sp = player();
        assert!(matches!(stage_strict_effects(&sp, &registry, &[WindowEvent::Grant(over.clone())]), Err(StrictStageError::Unrepresentable)), "a result the projection can't carry");
        sp.inventory.set_slot(4, Some(over));
        assert!(matches!(strict_raw_marker(&sp, &registry), Err(StrictStageError::RawUnrepresentable)), "the server's own window");
    }
    /// C3d run 14 (owed control for run 13's LOW-1) — a reserved live
    /// correction `Replace` is measured on the deque's REAL capacity: with
    /// the deque full to its capacity, the reservation grows it first, and
    /// the `push_back` that queues the `Replace` then never grows it past
    /// the capacity the byte bound measured (a doubling would make the next
    /// honest action fall behind).
    #[test]
    fn c3d_s1_reserved_replace_never_grows_the_deque_past_its_measure() {
        let mut sp = player();
        for _ in 0..16 { queue(&mut sp, WindowEvent::WearArmour { hits: 1 }, 0); }
        sp.window_events.waiting.shrink_to_fit();
        assert_eq!(sp.window_events.waiting.capacity(), sp.window_events.waiting.len(), "premise: the deque is full to its capacity");
        let probe = WindowEvent::WearArmour { hits: 1 };
        assert!(reserve_strict_replace(&mut sp, &probe), "one more event fits");
        let measured = sp.window_events.waiting.capacity();
        assert!(measured > sp.window_events.waiting.len(), "the slot is reserved before it is measured");
        queue(&mut sp, probe, 0);
        assert_eq!(sp.window_events.waiting.capacity(), measured, "the push never grows the deque past the capacity the fit measured");
        assert_eq!(sp.window_events.tally.forced, 0);
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
