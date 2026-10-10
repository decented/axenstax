//! Stage the existing raw click rule on marker-only scratch holdings.
//! Source evidence is local to one attempt, never a second possession ledger.

use crate::{armour::ArmourItem, inventory::Inventory, item::{Item, ItemStack}, protocol::{WireSlot, WireStack}, window::{self, ClickCtx, ClickResult, CraftGrid, WindowClick, WindowMut, WindowSlot}};

pub const PLAYER_PLACES: usize = 50;
pub const CURSOR: usize = 40;
pub type MarkerSlots = [WireSlot; PLAYER_PLACES];

/// Private Plan bodies are never cloned into a staged window. Wire conversion
/// retains the complete marker/developed identity and creates only a stub.
#[derive(Clone)]
pub struct MarkerWindow {
    inv: Inventory,
    armour: [Option<ArmourItem>; 4],
    cursor: Option<ItemStack>,
    grid: CraftGrid,
}

impl MarkerWindow {
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d client reserve-before-predict milestone (stages the client's own WindowMut)"))]
    pub fn capture(view: &WindowMut, registry: &crate::block::BlockRegistry) -> Self {
        Self::capture_parts(view.inv, view.armour, view.cursor, view.grid, registry)
    }

    pub fn capture_parts(inv: &Inventory, armour: &[Option<ArmourItem>; 4], cursor: &Option<ItemStack>, grid: &CraftGrid, registry: &crate::block::BlockRegistry) -> Self {
        let marker = |s: &ItemStack| crate::inventory::stack_from_wire(&crate::inventory::stack_to_wire(s), registry, crate::inventory::PlanDecode::Marker);
        let mut copied = Inventory::new();
        copied.auto_refill = inv.auto_refill;
        for slot in 0..36 {
            copied.set_slot(slot, inv.slot(slot).and_then(marker));
            if inv.is_locked(slot) { copied.toggle_lock(slot); }
        }
        Self { inv: copied, armour: *armour, cursor: cursor.as_ref().and_then(marker),
            grid: std::array::from_fn(|r| std::array::from_fn(|c| grid[r][c].as_ref().and_then(marker))) }
    }

    /// C3d review L1 — [`Self::capture_parts`] for a window the server will
    /// COMMIT from: every place must round-trip the marker projection
    /// exactly (a Plan may become its marker placeholder, keeping its
    /// marker and developed state). A stack that doesn't (a broken tool or
    /// armour piece, an unregistered block, a clamped durability) is
    /// `None`: the caller refuses with a typed fate before any commit,
    /// never dropping or clamping it silently.
    pub fn try_capture_parts(inv: &Inventory, armour: &[Option<ArmourItem>; 4], cursor: &Option<ItemStack>, grid: &CraftGrid, registry: &crate::block::BlockRegistry) -> Option<Self> {
        let exact = |s: &ItemStack| exact_marker(s, registry).is_some();
        let armour_ok = armour.iter().flatten().all(|a| exact(&ItemStack { item: Item::Armour(*a), count: 1 }));
        let all = (0..36).filter_map(|i| inv.slot(i)).chain(cursor.as_ref()).chain(grid.iter().flatten().flatten());
        let places_ok = all.into_iter().all(exact);
        (armour_ok && places_ok).then(|| Self::capture_parts(inv, armour, cursor, grid, registry))
    }

    /// Moves marker-only holdings into the server's raw window after admission.
    pub(crate) fn into_parts(self) -> (Inventory, [Option<ArmourItem>; 4], Option<ItemStack>, CraftGrid) {
        (self.inv, self.armour, self.cursor, self.grid)
    }

    pub fn view(&mut self) -> WindowMut<'_> {
        WindowMut { inv: &mut self.inv, armour: &mut self.armour, cursor: &mut self.cursor, grid: &mut self.grid, container: None }
    }

    /// The window digest (`window::digest_with`) at `station`, with the
    /// container `container` as the screen shows it (a Strict container op's
    /// digest covers the mirror, C3d shared containers).
    pub fn digest_with(&self, station: window::Station, container: Option<crate::container_window::ContainerRef>) -> u32 {
        window::digest_with(&self.inv, &self.armour, &self.cursor, &self.grid, station, container)
    }

    /// A Strict `SetAutoRefill` stages the setting on the scratch copy.
    pub fn set_auto_refill(&mut self, on: bool) { self.inv.auto_refill = on; }

    pub fn slots(&self) -> MarkerSlots {
        std::array::from_fn(|at| match at {
            0..36 => self.inv.slot(at).map(crate::inventory::stack_to_wire),
            36..40 => self.armour[at - 36].map(|a| crate::inventory::stack_to_wire(&ItemStack { item: Item::Armour(a), count: 1 })),
            CURSOR => self.cursor.as_ref().map(crate::inventory::stack_to_wire),
            _ => self.grid[(at - 41) / 3][(at - 41) % 3].as_ref().map(crate::inventory::stack_to_wire),
        })
    }
}

/// `stack`'s marker projection when it round-trips exactly (a Plan by its
/// marker and developed state), else `None` (C3d review L1).
pub fn exact_marker(stack: &ItemStack, registry: &crate::block::BlockRegistry) -> Option<ItemStack> {
    let back = crate::inventory::stack_from_wire(&crate::inventory::stack_to_wire(stack), registry, crate::inventory::PlanDecode::Marker)?;
    let same = match (&stack.item, &back.item) {
        (Item::Plan(a), Item::Plan(b)) => stack.count == back.count && crate::plan::marker(a) == crate::plan::marker(b) && a.develop_state.is_developed() == b.develop_state.is_developed(),
        _ => *stack == back,
    };
    same.then_some(back)
}

pub struct StagedClick {
    pub after: MarkerWindow,
    pub result: ClickResult,
    /// Initial place-specific sources, including zero-net adopted stacks.
    pub sources: MarkerSlots,
}

/// Conservative complete initial sources are deliberate for sort/close/
/// autofill. They include intermediate returned-and-reused grid/cursor units.
pub fn stage(raw: &MarkerWindow, click: &WindowClick, ctx: &ClickCtx) -> StagedClick {
    stage_with(raw, click, ctx.station, |view| window::apply(view, click, ctx))
}

/// [`stage`] by a caller-supplied rule over the same click at `station`:
/// the server stages its EXISTING transition (`window_ops::server_click`,
/// with its slack and the client's verdict), never a second rule.
pub fn stage_with(raw: &MarkerWindow, click: &WindowClick, station: window::Station, apply: impl FnOnce(&mut WindowMut) -> ClickResult) -> StagedClick {
    let mut after = raw.clone();
    let result = apply(&mut after.view());
    let before = raw.slots();
    let mut sources: MarkerSlots = std::array::from_fn(|_| None);
    let mut need = [false; PLAYER_PLACES];
    let at = |slot: WindowSlot| match slot {
        WindowSlot::Inv(i) if i < 36 => Some(i),
        WindowSlot::Grid(r, c) if r < station.grid_size() && c < station.grid_size() => Some(41 + r * 3 + c),
        _ => None,
    };
    match click {
        WindowClick::Slot { slot, .. } if *slot < 36 => { need[*slot] = true; need[CURSOR] = true; }
        WindowClick::Grid { row, col, .. } => {
            if let Some(i) = at(WindowSlot::Grid(*row, *col)) { need[i] = true; need[CURSOR] = true; }
        }
        WindowClick::Armour { slot } if *slot < 4 => { need[36 + *slot] = true; need[CURSOR] = true; }
        WindowClick::Result if matches!(result, ClickResult::Crafted(_)) => {
            for i in 41..50 {
                sources[i] = before[i].clone().map(|mut s| { s.count = 1; s });
            }
        }
        WindowClick::Trash => need[CURSOR] = true,
        WindowClick::DragDistribute { slots } | WindowClick::DragGather { slots } => {
            need[CURSOR] = true;
            for slot in slots { if let Some(i) = at(*slot) { need[i] = true; } }
        }
        WindowClick::Sort => { for (i, n) in need.iter_mut().enumerate().take(36).skip(window::BAG_START) { *n = !raw.inv.is_locked(i); } }
        WindowClick::Close => { need[CURSOR..50].fill(true); }
        WindowClick::Autofill { .. } => { need[..36].fill(true); need[CURSOR..50].fill(true); }
        WindowClick::ToggleLock { .. } | WindowClick::Slot { .. } | WindowClick::Armour { .. } | WindowClick::Result => {}
    }
    for i in 0..PLAYER_PLACES { if need[i] { sources[i] = before[i].clone(); } }
    StagedClick { after, result, sources }
}

/// C3d shared containers — the shared container rule
/// (`container_window::apply_container`) over marker-only scratch holdings
/// and `container` (a scratch copy: the client's predicted view or the REAL
/// container's contents). Sources are conservative: every personal place
/// the click changed, at its whole initial value (a merge target included),
/// so a pending Take of any of them refuses backing.
pub fn stage_container(raw: &MarkerWindow, container: &mut crate::container_window::ContainerData, click: &crate::container_window::ContainerClick, ctx: &ClickCtx) -> StagedClick {
    let mut after = raw.clone();
    let applied = {
        let mut view = WindowMut { inv: &mut after.inv, armour: &mut after.armour, cursor: &mut after.cursor, grid: &mut after.grid, container: Some(container.as_mut()) };
        crate::container_window::apply_container(&mut view, click, ctx)
    };
    let before = raw.slots();
    let now = after.slots();
    let sources = std::array::from_fn(|i| if before[i] != now[i] { before[i].clone() } else { None });
    StagedClick { after, result: applied.result, sources }
}

/// Query the exact existing pending-event fold after EACH prefix. Eligibility
/// only falls: a later same-slot Grant cannot back an earlier removed source.
pub struct SourceBacking {
    initial: MarkerSlots,
    surviving_identity: MarkerSlots,
    eligible: [u8; PLAYER_PLACES],
}

impl SourceBacking {
    pub fn new(raw: MarkerSlots) -> Self {
        let eligible = std::array::from_fn(|i| raw[i].as_ref().map_or(0, |s| s.count));
        Self { surviving_identity: raw.clone(), initial: raw, eligible }
    }

    pub fn observe_prefix(&mut self, prefix: &MarkerSlots) {
        for (i, current) in prefix.iter().enumerate() {
            let same = match (&self.surviving_identity[i], current) {
                (Some(initial), Some(current)) => same_identity(initial, current),
                _ => false,
            };
            self.eligible[i] = self.eligible[i].min(if same { current.as_ref().map_or(0, |s| s.count) } else { 0 });
        }
    }

    /// Only an admitted existing pending wear may change a piece's expected
    /// durability. Eligibility remains non-increasing and stays at its place.
    pub(crate) fn observe_authorised_wear(&mut self, before: &MarkerSlots, after: &MarkerSlots) {
        use crate::protocol::WireItem;
        for i in 0..PLAYER_PLACES {
            let (Some(expected), Some(previous)) = (&self.surviving_identity[i], &before[i]) else { continue };
            if !same_identity(expected, previous) { continue; }
            let Some(next) = &after[i] else { continue };
            let worn = match (previous.full_item, next.full_item) {
                (WireItem::Tool { tool_type: a, material: am, durability: ad }, WireItem::Tool { tool_type: b, material: bm, durability: bd }) => a == b && am == bm && bd < ad,
                (WireItem::Armour { slot: a, material: am, durability: ad }, WireItem::Armour { slot: b, material: bm, durability: bd }) => a == b && am == bm && bd < ad,
                _ => false,
            };
            if worn && previous.count == 1 && next.count == 1 && previous.item_kind == next.item_kind && previous.item_id == next.item_id {
                self.surviving_identity[i] = Some(next.clone());
            }
        }
    }

    pub fn backs(&self, sources: &MarkerSlots) -> bool {
        sources.iter().enumerate().all(|(i, source)| source.as_ref().is_none_or(|s| {
            self.initial[i].as_ref().is_some_and(|raw| same_identity(raw, s)) && self.eligible[i] >= s.count
        }))
    }
}

fn same_identity(a: &WireStack, b: &WireStack) -> bool {
    a.item_kind == b.item_kind && a.item_id == b.item_id && a.full_item == b.full_item
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(inv: &mut Inventory, grid: &mut CraftGrid, cursor: &mut Option<ItemStack>) -> MarkerWindow {
        MarkerWindow::capture(&WindowMut { inv, armour: &mut [None; 4], cursor, grid, container: None }, &crate::block::BlockRegistry::new())
    }
    fn ctx() -> ClickCtx { ClickCtx::new(false, window::Station::Player, glam::Vec3::ZERO, |_| crate::block::AIR) }

    #[test]
    fn c3d_shared_raw_put_down_is_not_replaced_by_a_future_grant_swap() {
        let mut inv = Inventory::new();
        let mut grid = std::array::from_fn(|_| std::array::from_fn(|_| None));
        let mut cursor = Some(ItemStack::new_block(crate::block::DIRT, 1));
        let raw = raw(&mut inv, &mut grid, &mut cursor);
        let click = WindowClick::Slot { slot: 0, right: false };
        let staged = stage(&raw, &click, &ctx());
        assert_eq!(staged.after.slots()[0], Some(crate::inventory::stack_to_wire(&ItemStack::new_block(crate::block::DIRT, 1))));
        assert!(staged.after.slots()[CURSOR].is_none());
        let mut pending = raw.clone();
        let _ = pending.inv.add_item(ItemStack::new_block(crate::block::STONE, 1));
        let mut backing = SourceBacking::new(raw.slots());
        backing.observe_prefix(&pending.slots());
        assert!(backing.backs(&staged.sources));
        let effective_click = stage(&pending, &click, &ctx());
        assert!(effective_click.after.slots()[CURSOR].is_some(), "running the entire effective click would swap instead");
    }

    #[test]
    fn c3d_removed_source_cannot_use_an_unrelated_or_later_same_slot_grant() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 1)));
        inv.set_slot(1, Some(ItemStack::new_block(crate::block::STONE, 64)));
        let raw = raw(&mut inv, &mut std::array::from_fn(|_| std::array::from_fn(|_| None)), &mut None);
        let staged = stage(&raw, &WindowClick::Slot { slot: 0, right: false }, &ctx());
        let mut pending = raw.clone();
        // This is the SAME owed search used by window_events, not a substitute
        // total-count model. Only source0 is removed, genuine source1 remains.
        let taken = crate::joiner_actions::take_owed_window(&mut pending.inv, &mut pending.grid, &mut pending.cursor, 0, &Item::Block(crate::block::STONE), 1);
        assert_eq!(taken, 1);
        let mut backing = SourceBacking::new(raw.slots());
        backing.observe_prefix(&pending.slots());
        assert!(!backing.backs(&staged.sources));
        let _ = pending.inv.add_item(ItemStack::new_block(crate::block::STONE, 1));
        backing.observe_prefix(&pending.slots());
        assert_eq!(pending.slots(), raw.slots());
        assert!(!backing.backs(&staged.sources), "future grant cannot legitimise the old source");
    }

    #[test]
    fn c3d_close_autofill_sort_and_trash_include_initial_zero_net_sources() {
        let mut inv = Inventory::new();
        inv.set_slot(9, Some(ItemStack::new_block(crate::block::STONE, 1)));
        inv.set_slot(10, Some(ItemStack::new_block(crate::block::DIRT, 1)));
        inv.toggle_lock(10);
        let mut grid = std::array::from_fn(|_| std::array::from_fn(|_| None));
        grid[0][0] = Some(ItemStack::new_block(crate::block::OAK_LOG, 1));
        let mut cursor = Some(ItemStack::new_block(crate::block::DIRT, 1));
        let raw = raw(&mut inv, &mut grid, &mut cursor);
        let close = stage(&raw, &WindowClick::Close, &ctx());
        assert!(close.sources[41].is_some() && close.sources[CURSOR].is_some());
        let sort = stage(&raw, &WindowClick::Sort, &ctx());
        assert!(sort.sources[9].is_some() && sort.sources[10].is_none());
        let trash = stage(&raw, &WindowClick::Trash, &ctx());
        assert_eq!(trash.sources[CURSOR], raw.slots()[CURSOR]);
        let autofill = stage(&raw, &WindowClick::Autofill { example: [[crate::crafting::CraftSlot::Empty; 3]; 3] }, &ctx());
        assert!(autofill.sources[41].is_some() && autofill.sources[CURSOR].is_some() && autofill.sources[9].is_some());
    }
}
