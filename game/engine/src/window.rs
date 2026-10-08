//! The inventory window's click rules (C3a-1, 2026-10-07): every move a
//! player makes in the inventory screen is ONE pure transition, [`apply`],
//! over a borrowed view of the state that screen touches ([`WindowMut`]).
//!
//! The client runs it on its own copy at once; C3a-2 has the server run the
//! same transition on its copy of a joiner's window, in arrival order, so the
//! two layouts stay in lockstep by construction
//! (`docs/foundations/2026-10-07-c3-server-owned-inventory.md` §2).
//!
//! No egui, no GPU, no I/O. `craft_ui.rs` keeps the drawing, hover and open
//! state, and turns what the player clicked into a [`WindowClick`].
//!
//! Every rule is lossless: nothing is created or destroyed except by
//! [`WindowClick::Trash`] (destroys the cursor) and [`WindowClick::Result`]
//! (one of each grid cell becomes the crafted output).
//!
//! C3a-2a (protocol v75): a [`WindowClick`] is wire data. A joined client
//! sends every click it applies as a window op (`window_ops`), with the
//! [`digest`] of its window after it, and the server applies the same
//! [`apply`] to its copy of that joiner's window. A table's reach is part of
//! the rule ([`ClickCtx::table_present`]), so each side judges it from its
//! own body and its own world.

use serde::{Deserialize, Deserializer, Serialize};

use crate::armour::{ArmourItem, ArmourSlot};
use crate::block::BlockId;
use crate::crafting::{self, CraftSlot};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};

/// The crafting grid's storage: always 3×3; the player's 2×2 uses the
/// top-left four cells ([`Station::grid_size`]).
pub type CraftGrid = [[Option<ItemStack>; 3]; 3];

/// Inventory slots: 0..9 hotbar, 9..36 the bag.
pub const SLOTS: usize = 36;
/// The first bag slot. [`WindowClick::Sort`] tidies `BAG_START..SLOTS`; the
/// hotbar is the player's curated bar and is never sorted.
pub const BAG_START: usize = 9;

/// The most slots one drag gesture paints: every inventory slot and every
/// grid cell once (36 + 9). A longer list is refused by the rule and doesn't
/// decode off the wire.
pub const MAX_DRAG_SLOTS: usize = SLOTS + 9;

/// A borrowed view over everything one inventory screen touches.
pub struct WindowMut<'a> {
    /// The 36 slots, with their locks and `auto_refill`.
    pub inv: &'a mut Inventory,
    /// The four armour slots (`PlayerSlot.armour_slots`), indexed by
    /// `ArmourSlot as usize`.
    pub armour: &'a mut [Option<ArmourItem>; 4],
    /// The stack carried on the mouse cursor.
    pub cursor: &'a mut Option<ItemStack>,
    /// The crafting grid (its usable size comes from [`ClickCtx::station`]).
    pub grid: &'a mut CraftGrid,
    /// C3b-1 — the open container (a chest of any tier, a dispenser or
    /// dropper, a furnace): `container_window::apply_container`'s clicks
    /// move items between it and the rest of the window, and [`digest`]
    /// covers it. `None` for every other screen.
    pub container: Option<crate::container_window::ContainerMut<'a>>,
}

/// Which crafting grid the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Station {
    /// The player's own 2×2 grid.
    Player,
    /// The 3×3 grid of the crafting table at `cell` (C3a-2a: the station
    /// carries its table, so the rule can check it is still there).
    Table { cell: [i32; 3] },
}

impl Station {
    /// The grid's side: 2 for the player, 3 for a table.
    pub fn grid_size(self) -> usize {
        match self {
            Station::Player => 2,
            Station::Table { .. } => 3,
        }
    }
}

/// What a rule needs that it can't read from the view.
#[derive(Clone, Debug)]
pub struct ClickCtx {
    /// The player is in creative mode. No rule differs in creative yet.
    #[allow(dead_code)] // carried for C3b, whose container rules may need it.
    pub creative: bool,
    /// The grid on screen: bounds a [`WindowClick::Grid`] click, and is what
    /// the result click's [`recipe_output`] matches against.
    pub station: Station,
    /// C3a-2a — the acting body's eye: the client's own player, or the
    /// server's body of a joiner. A table's reach is judged from it.
    pub eye: glam::Vec3,
    /// C3a-2a — the block the acting side's world holds at the station's
    /// table cell (`AIR` at the player's grid). Each side reads its own world.
    pub table_block: BlockId,
    /// C3a-fix-2 B-L2 — extra reach, in blocks, the table check allows. 0 on
    /// the client (the exact rule); [`SERVER_TABLE_REACH_SLACK`] on the server.
    pub reach_slack: f32,
    /// C3a-fix-2 B-L2 — the table is allowed to be gone: it changed within
    /// [`SERVER_TABLE_GRACE_TICKS`], so the client may not have heard.
    /// `false` on the client; the server sets it ([`Self::with_server_slack`]).
    pub table_grace: bool,
    /// C3b-1 — the open container is a shared one: a joined client's mirror
    /// of the server's, or the server applying a joiner's op. A Plan neither
    /// goes in nor comes out (`container_window`). `false` for single-player
    /// and a host's own screens, whose rules don't change.
    pub shared: bool,
}

/// B-L2 — the server judges a joiner's table reach this much more kindly than
/// the client does, so its verdict is a superset of an honest client's (the
/// client eye and the server body differ by lag and knockback).
pub const SERVER_TABLE_REACH_SLACK: f32 = 0.5;

/// B-L2 — how many ticks after its table's cell changed the server still lets
/// a joiner craft at it: the client acts on a world that is a few ticks old.
pub const SERVER_TABLE_GRACE_TICKS: u8 = 10;

impl ClickCtx {
    /// A click at `station` by a body whose eye is at `eye`, in a world
    /// whose blocks `block_at` reads.
    pub fn new(creative: bool, station: Station, eye: glam::Vec3, block_at: impl Fn([i32; 3]) -> BlockId) -> Self {
        let table_block = match station {
            Station::Table { cell } => block_at(cell),
            Station::Player => crate::block::AIR,
        };
        ClickCtx { creative, station, eye, table_block, reach_slack: 0.0, table_grace: false, shared: false }
    }

    /// B-L2 — the SERVER's context: [`SERVER_TABLE_REACH_SLACK`] more reach,
    /// and, when `table_changed_lately`, a table that is no longer there
    /// still counts. The client keeps [`Self::new`]'s exact rule.
    pub fn with_server_slack(mut self, table_changed_lately: bool) -> Self {
        self.reach_slack = SERVER_TABLE_REACH_SLACK;
        self.table_grace = table_changed_lately;
        self
    }

    /// C3b-1 — the same context, judging a shared container (`shared`).
    pub fn with_shared(mut self, shared: bool) -> Self {
        self.shared = shared;
        self
    }

    /// May this station craft now? The player's own grid always; a table
    /// only while it stands in reach of the acting body ([`table_in_reach`],
    /// the rule its screen closes by; the server's verdict carries a slack,
    /// [`Self::with_server_slack`]). The result click and `Autofill` ask,
    /// so a screen whose forced close couldn't return everything can no
    /// longer craft at, or lay a recipe into, a table that's gone.
    pub fn table_present(&self) -> bool {
        match self.station {
            Station::Player => true,
            Station::Table { cell } => {
                (self.table_block == crate::block::CRAFTING_TABLE || self.table_grace)
                    && crate::item_actions::cell_in_reach_with(self.eye, cell, self.reach_slack)
            }
        }
    }
}

/// A slot a drag gesture paints over. Wire data (C3a-2a), append only:
/// `Inv` = 0, `Grid` = 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowSlot {
    /// Inventory slot 0..36.
    Inv(usize),
    /// Crafting-grid cell `(row, col)`.
    Grid(usize, usize),
}

/// One transition the inventory screen performs. Plain data, sent as a
/// window op (C3a-2a, `protocol::WireWindowOp::Click`).
///
/// Wire-stable, APPEND ONLY: Slot = 0, Grid = 1, Armour = 2, Result = 3,
/// Trash = 4, DragDistribute = 5, DragGather = 6, Sort = 7, ToggleLock = 8,
/// Autofill = 9, Close = 10 (pinned by `protocol::tests::window_op_packets_round_trip`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowClick {
    /// Click inventory slot `slot` (0..36). Left (`right == false`) works on
    /// the whole stack: pick up, put down, merge up to max, or swap. Right
    /// works on one: pick up the ceil-half, put one down, or add one.
    Slot { slot: usize, right: bool },
    /// Click a crafting-grid cell, with the same left/right rules as `Slot`.
    /// A cell outside the station's grid is refused.
    Grid { row: usize, col: usize, right: bool },
    /// Click armour slot `slot` (`ArmourSlot as usize`): pick the piece up,
    /// or equip the cursor's piece if it is for this slot (swapping). Anything
    /// else on the cursor is refused and stays put.
    Armour { slot: usize },
    /// Click the craft result: one craft of what the grid matches now
    /// ([`recipe_output`]) lands on the cursor (merged when it
    /// stacks; any overflow, or the whole output under a different cursor
    /// item, must fit the inventory), then one is taken from every non-empty
    /// grid cell. Refused, consuming nothing, when it can't land, or at a
    /// table no longer in reach ([`ClickCtx::table_present`]).
    Result,
    /// The trash slot: destroy the stack on the cursor.
    Trash,
    /// RMB drag paint: drop one carried item into each slot in order (an
    /// empty slot, or a matching one with room). Others are skipped. At most
    /// [`MAX_DRAG_SLOTS`] slots (more is refused, and doesn't decode).
    DragDistribute {
        #[serde(deserialize_with = "bounded_drag")]
        slots: Vec<WindowSlot>,
    },
    /// LMB drag paint: gather each slot's matching items into the cursor, in
    /// order. An empty cursor adopts the first painted stack. At most
    /// [`MAX_DRAG_SLOTS`] slots.
    DragGather {
        #[serde(deserialize_with = "bounded_drag")]
        slots: Vec<WindowSlot>,
    },
    /// The Sort button: merge and order the bag (`BAG_START..SLOTS`),
    /// keeping locked slots where they are.
    Sort,
    /// Alt+click: toggle the lock on inventory slot `slot` (0..36).
    ToggleLock { slot: usize },
    /// Recipe-book "Fill from bag": return the grid and cursor to the
    /// inventory, then lay `example` from it if the inventory holds every
    /// item; otherwise take nothing and leave the grid empty. A recipe
    /// bigger than the player's 2×2 is refused there (`NeedsTable`) before
    /// anything moves; one that fits is laid in the 2×2's corner. At a table
    /// no longer in reach nothing moves either (`NeedsTable`).
    Autofill { example: [[CraftSlot; 3]; 3] },
    /// Closing the screen: return the grid and cursor to the inventory.
    /// What doesn't fit stays where it was (and the screen stays open).
    Close,
}

/// The wire bound on a drag's slot list ([`MAX_DRAG_SLOTS`]): a longer list
/// doesn't decode, so the op is never applied. Bounded by the packet's size
/// limit while it is read (`protocol::safe_deserialize`).
fn bounded_drag<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<WindowSlot>, D::Error> {
    let slots = Vec::<WindowSlot>::deserialize(d)?;
    if slots.len() > MAX_DRAG_SLOTS {
        return Err(serde::de::Error::invalid_length(slots.len(), &"at most 45 drag slots"));
    }
    Ok(slots)
}

/// What the caller must do outside the window.
#[derive(Clone, Debug, PartialEq)]
pub enum ClickResult {
    /// The click did what it asked.
    Done,
    /// Refused, or nothing to do. Anything a refused `Autofill` or `Close`
    /// already moved stays moved (losslessly).
    Refused,
    /// One craft gave this (challenge events, first-craft hints).
    Crafted(ItemStack),
    /// The trash destroyed this stack.
    Binned(ItemStack),
    /// `Autofill` of a recipe bigger than 2×2 into the player's grid: nothing
    /// moved (the caller toasts "Needs a crafting table").
    NeedsTable,
    /// C3b-1 — a Plan can't go into, or come out of, a shared container:
    /// nothing moved (the caller toasts "Plans can't go in shared containers
    /// yet").
    PlanStays,
}

impl ClickResult {
    /// Did the click succeed (the old `bool` the click handlers returned)?
    pub fn ok(&self) -> bool {
        !matches!(self, ClickResult::Refused | ClickResult::NeedsTable | ClickResult::PlanStays)
    }
}

/// What one craft from `grid` at `station` gives: `crafting::match_recipe`
/// over the grid's items (tools, plans and armour are never ingredients).
///
/// M3 (C2b verify): an item in a cell the station's grid doesn't have (row
/// or column 2 of the player's 2×2) crafts nothing, so no path crafts a
/// table recipe without a table.
///
/// L1 (C2b verify): a tool, armour piece or Plan anywhere in the grid
/// crafts nothing. The matcher reads them as empty cells, and the result
/// click takes one from every non-empty cell, so a craft used to destroy
/// one left in the grid.
pub fn recipe_output(grid: &CraftGrid, station: Station) -> Option<ItemStack> {
    let size = station.grid_size();
    let mut cells = [[CraftSlot::Empty; 3]; 3];
    for (r, grid_row) in grid.iter().enumerate() {
        for (c, cell) in grid_row.iter().enumerate() {
            let Some(stack) = cell else { continue };
            if r >= size || c >= size {
                return None;
            }
            let slot = CraftSlot::from_item(&stack.item);
            if slot == CraftSlot::Empty {
                // L1 — a non-ingredient blocks the craft (it would be
                // consumed with the rest).
                return None;
            }
            cells[r][c] = slot;
        }
    }
    crafting::match_recipe(&cells)
}

/// L3 (C2b verify) — may a crafting table's screen stay open? Only while
/// `cell` still holds a crafting table (`block_at_cell`) within the block
/// reach of the player's body (eye at `eye`), by the rule the server judges
/// a joiner's table craft with (`item_actions::cell_in_reach`). Minecraft
/// closes the screen the same way.
pub fn table_in_reach(block_at_cell: crate::block::BlockId, cell: [i32; 3], eye: glam::Vec3) -> bool {
    block_at_cell == crate::block::CRAFTING_TABLE && crate::item_actions::cell_in_reach(eye, cell)
}

/// M3 — `example` as it is laid at `station`: unchanged when it fits where
/// it is, moved to the top-left corner when it fits the grid but not there,
/// and `None` when it is bigger than the grid (a 3×3 recipe at the player's
/// 2×2).
fn fit_example(example: &[[CraftSlot; 3]; 3], station: Station) -> Option<[[CraftSlot; 3]; 3]> {
    if example.iter().flatten().all(|s| *s == CraftSlot::Empty) {
        return Some(*example);
    }
    let size = station.grid_size();
    let (min_r, max_r, min_c, max_c) = crafting::grid_bounds(example);
    if max_r - min_r >= size || max_c - min_c >= size {
        return None;
    }
    if max_r < size && max_c < size {
        return Some(*example);
    }
    let mut laid = [[CraftSlot::Empty; 3]; 3];
    for r in min_r..=max_r {
        for c in min_c..=max_c {
            laid[r - min_r][c - min_c] = example[r][c];
        }
    }
    Some(laid)
}

/// Apply one click to the window.
pub fn apply(view: &mut WindowMut, click: &WindowClick, ctx: &ClickCtx) -> ClickResult {
    match click {
        WindowClick::Slot { slot, right } => click_slot(view, *slot, *right),
        WindowClick::Grid { row, col, right } => click_grid(view, *row, *col, *right, ctx.station),
        WindowClick::Armour { slot } => click_armour(view, *slot),
        WindowClick::Result if !ctx.table_present() => ClickResult::Refused,
        WindowClick::Result => click_result(view, ctx.station),
        WindowClick::Trash => match view.cursor.take() {
            Some(stack) => ClickResult::Binned(stack),
            None => ClickResult::Refused,
        },
        WindowClick::DragDistribute { slots } | WindowClick::DragGather { slots } if slots.len() > MAX_DRAG_SLOTS => {
            ClickResult::Refused
        }
        WindowClick::DragDistribute { slots } => {
            let mut moved = false;
            for &at in slots {
                moved |= distribute_one(view, at, ctx.station);
            }
            done_if(moved)
        }
        WindowClick::DragGather { slots } => {
            let mut moved = false;
            for &at in slots {
                moved |= gather(view, at, ctx.station);
            }
            done_if(moved)
        }
        WindowClick::Sort => {
            view.inv.sort_region(BAG_START, SLOTS);
            ClickResult::Done
        }
        WindowClick::ToggleLock { slot } => {
            if *slot >= SLOTS {
                return ClickResult::Refused;
            }
            view.inv.toggle_lock(*slot);
            ClickResult::Done
        }
        WindowClick::Autofill { .. } if !ctx.table_present() => ClickResult::NeedsTable,
        WindowClick::Autofill { example } => match fit_example(example, ctx.station) {
            Some(laid) => done_if(autofill(view, &laid)),
            None => ClickResult::NeedsTable,
        },
        WindowClick::Close => done_if(return_grid_and_cursor(view)),
    }
}

fn done_if(ok: bool) -> ClickResult {
    if ok { ClickResult::Done } else { ClickResult::Refused }
}

/// Two items are the same kind (block id / material id). Tools, plans and
/// armour are never crafting ingredients, so they're never equal here.
fn same_item(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::Block(x), Item::Block(y)) => x == y,
        (Item::Material(x), Item::Material(y)) => x == y,
        _ => false,
    }
}

/// Total count of `item` across all inventory slots.
fn inv_count(inv: &Inventory, item: &Item) -> u32 {
    (0..SLOTS)
        .filter_map(|i| inv.slot(i))
        .filter(|s| same_item(&s.item, item))
        .map(|s| s.count as u32)
        .sum()
}

/// Remove one unit of `item` from the lowest matching inventory slot.
/// Returns false if none was found.
fn inv_remove_one(inv: &mut Inventory, item: &Item) -> bool {
    for i in 0..SLOTS {
        if let Some(s) = inv.slot(i)
            && same_item(&s.item, item) && s.count >= 1 {
                let mut ns = s.clone();
                ns.count -= 1;
                inv.set_slot(i, if ns.count == 0 { None } else { Some(ns) });
                return true;
            }
    }
    false
}

/// [`WindowClick::Slot`]. All paths are lossless.
fn click_slot(view: &mut WindowMut, slot: usize, right: bool) -> ClickResult {
    if slot >= SLOTS {
        // `Inventory::set_slot` ignores an index past the end, so placing
        // there would delete the cursor. The screen never sends one.
        return ClickResult::Refused;
    }
    let inventory = &mut *view.inv;
    if let Some(cursor) = view.cursor.take() {
        match inventory.slot(slot).cloned() {
            // Empty slot.
            None => {
                if right {
                    // Right: place one, keep the rest on the cursor.
                    inventory.set_slot(slot, Some(ItemStack { item: cursor.item.clone(), count: 1 }));
                    if cursor.count > 1 {
                        *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - 1 });
                    }
                } else {
                    // Left: place the whole stack.
                    inventory.set_slot(slot, Some(cursor));
                }
            }
            // Same item — merge.
            Some(existing) if existing.item.can_stack_with(&cursor.item) => {
                let existing_count = existing.count;
                // saturating_sub: an over-max slot (legacy save, or the old
                // chest 255-cap bug) leaves zero room instead of underflowing;
                // `existing_count + add` can't overflow u8 because add <= room
                // (engine audit 2026-06-04, A: u8 overflow in this branch).
                let room = cursor.item.max_stack().saturating_sub(existing_count);
                if right {
                    // Right: add one if there's room; else no-op (restore cursor).
                    if room >= 1 {
                        inventory.set_slot(slot, Some(ItemStack { item: cursor.item.clone(), count: existing_count + 1 }));
                        if cursor.count > 1 {
                            *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - 1 });
                        }
                    } else {
                        *view.cursor = Some(cursor);
                    }
                } else {
                    // Left: merge the whole stack up to max; remainder stays on cursor.
                    let add = cursor.count.min(room);
                    if add == cursor.count {
                        inventory.set_slot(slot, Some(ItemStack { item: cursor.item, count: existing_count + add }));
                    } else {
                        inventory.set_slot(slot, Some(ItemStack { item: cursor.item.clone(), count: existing_count + add }));
                        *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - add });
                    }
                }
            }
            // Different item → swap (both buttons). Lossless.
            Some(existing) => {
                inventory.set_slot(slot, Some(cursor));
                *view.cursor = Some(existing);
            }
        }
    } else if let Some(stack) = inventory.slot(slot).cloned() {
        // Empty cursor → pick up. Left = whole, right = ceil-half (leave the rest).
        if right {
            let take = stack.count / 2 + stack.count % 2;
            let leave = stack.count - take;
            *view.cursor = Some(ItemStack { item: stack.item.clone(), count: take });
            if leave > 0 {
                inventory.set_slot(slot, Some(ItemStack { item: stack.item, count: leave }));
            } else {
                inventory.set_slot(slot, None);
            }
        } else {
            *view.cursor = Some(stack);
            inventory.set_slot(slot, None);
        }
    }
    ClickResult::Done
}

/// [`WindowClick::Grid`]. Minecraft semantics: left works on the whole
/// stack, right on a single item. All paths are lossless.
fn click_grid(view: &mut WindowMut, row: usize, col: usize, right: bool, station: Station) -> ClickResult {
    let size = station.grid_size();
    if row >= size || col >= size {
        return ClickResult::Refused;
    }
    let grid = &mut *view.grid;
    if let Some(cursor) = view.cursor.take() {
        match grid[row][col].take() {
            // Empty cell.
            None => {
                if right {
                    // Right: drop one, keep the rest on the cursor.
                    grid[row][col] = Some(ItemStack { item: cursor.item.clone(), count: 1 });
                    if cursor.count > 1 {
                        *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - 1 });
                    }
                } else {
                    // Left: drop the whole stack (capped at max_stack; a
                    // cursor never exceeds max in practice, but stay safe).
                    let max = cursor.item.max_stack();
                    if cursor.count <= max {
                        grid[row][col] = Some(cursor);
                    } else {
                        grid[row][col] = Some(ItemStack { item: cursor.item.clone(), count: max });
                        *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - max });
                    }
                }
            }
            // Same item already in the cell.
            Some(mut cell) if cell.item.can_stack_with(&cursor.item) => {
                let room = cell.item.max_stack().saturating_sub(cell.count);
                if right {
                    // Right: add one if there's room (click-to-stack); else no-op.
                    if room >= 1 {
                        cell.count += 1;
                        grid[row][col] = Some(cell);
                        if cursor.count > 1 {
                            *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - 1 });
                        }
                    } else {
                        grid[row][col] = Some(cell);
                        *view.cursor = Some(cursor);
                    }
                } else {
                    // Left: merge the whole stack up to max; remainder stays on cursor.
                    let add = cursor.count.min(room);
                    cell.count += add;
                    grid[row][col] = Some(cell);
                    if add < cursor.count {
                        *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - add });
                    }
                }
            }
            // Different item → swap (both buttons). Lossless: the cell holds
            // the whole cursor (≤ max), the cursor takes the displaced cell.
            Some(cell) => {
                grid[row][col] = Some(cursor);
                *view.cursor = Some(cell);
            }
        }
    } else if let Some(cell) = grid[row][col].take() {
        // Empty cursor → pick up. Left = whole, right = ceil-half (leave the rest).
        if right {
            let take = cell.count / 2 + cell.count % 2;
            let leave = cell.count - take;
            *view.cursor = Some(ItemStack { item: cell.item.clone(), count: take });
            if leave > 0 {
                grid[row][col] = Some(ItemStack { item: cell.item, count: leave });
            }
        } else {
            *view.cursor = Some(cell);
        }
    }
    ClickResult::Done
}

/// [`WindowClick::Armour`] (Spec 28e), Minecraft's armour-slot picker:
///  * Holding nothing + slot has piece → pick the piece up to cursor.
///  * Holding an `Item::Armour` that matches this slot → equip it (the
///    previously-equipped piece, if any, goes onto the cursor as a swap).
///  * Holding any other item, or armour for the wrong slot → refused
///    (cursor stays put, slot stays put) so the player can't lose items by
///    accidentally clicking an unrelated slot.
fn click_armour(view: &mut WindowMut, armour_slot_idx: usize) -> ClickResult {
    let expected_slot = match armour_slot_idx {
        0 => ArmourSlot::Helmet,
        1 => ArmourSlot::Chestplate,
        2 => ArmourSlot::Leggings,
        3 => ArmourSlot::Boots,
        _ => return ClickResult::Refused,
    };
    let armour_slots = &mut *view.armour;
    match view.cursor.take() {
        None => {
            // Unequip path: lift the piece (if any) into the cursor.
            if let Some(piece) = armour_slots[armour_slot_idx].take() {
                *view.cursor = Some(ItemStack { item: Item::Armour(piece), count: 1 });
            }
            ClickResult::Done
        }
        Some(cursor) => match &cursor.item {
            Item::Armour(piece) if piece.slot == expected_slot => {
                // Equip — swap with whatever's in the slot.
                let previous = armour_slots[armour_slot_idx].take();
                armour_slots[armour_slot_idx] = Some(*piece);
                // If the cursor stack carried more than one (shouldn't —
                // armour never stacks — but defensive) put the rest back.
                if cursor.count > 1 {
                    *view.cursor = Some(ItemStack { item: cursor.item.clone(), count: cursor.count - 1 });
                    // The cursor is occupied, so the swapped-out piece goes
                    // back into the slot we just vacated. Realistically armour
                    // never stacks so this branch never fires, but keep the
                    // path sound.
                    if let Some(prev) = previous {
                        armour_slots[armour_slot_idx] = Some(prev);
                    }
                } else {
                    *view.cursor = previous.map(|p| ItemStack { item: Item::Armour(p), count: 1 });
                }
                ClickResult::Done
            }
            _ => {
                // Wrong-slot armour or non-armour item — bounce the cursor
                // back unchanged so the player doesn't lose it on a stray click.
                *view.cursor = Some(cursor);
                ClickResult::Refused
            }
        },
    }
}

/// [`WindowClick::Result`]: one craft of what the grid makes NOW.
///
/// L2 (C2b verify): re-matched from the grid at the click, never a cached
/// result, so a grid a refused close half-emptied can't craft what it used
/// to hold.
fn click_result(view: &mut WindowMut, station: Station) -> ClickResult {
    let Some(result) = recipe_output(view.grid, station) else {
        return ClickResult::Refused;
    };
    // Plan where the result lands BEFORE consuming anything (audit
    // 2026-09-27): if the cursor can't absorb it and the inventory has
    // no room for what's left, refuse the craft and consume nothing.
    //  - empty cursor → the result goes on the cursor.
    //  - result stacks with the cursor → merge up to max_stack onto the
    //    cursor; only the *overflow* must fit the inventory.
    //  - different item on the cursor → the whole result must fit the
    //    inventory. The cursor is never overwritten or re-added.
    let mut new_cursor = view.cursor.clone();
    let mut new_inventory: Option<Inventory> = None;
    match new_cursor.as_mut() {
        None => new_cursor = Some(result.clone()),
        Some(cursor) => {
            let to_inventory = if cursor.item.can_stack_with(&result.item) {
                let space = cursor.item.max_stack().saturating_sub(cursor.count);
                let merge = result.count.min(space);
                cursor.count += merge;
                let overflow = result.count - merge;
                (overflow > 0).then(|| ItemStack { item: result.item.clone(), count: overflow })
            } else {
                Some(result.clone())
            };
            if let Some(stack) = to_inventory {
                let mut trial = view.inv.clone();
                if trial.add_item(stack).is_some() {
                    return ClickResult::Refused;
                }
                new_inventory = Some(trial);
            }
        }
    }

    for cell in view.grid.iter_mut().flatten() {
        if let Some(stack) = cell {
            stack.count -= 1;
            if stack.count == 0 {
                *cell = None;
            }
        }
    }
    *view.cursor = new_cursor;
    if let Some(inv) = new_inventory {
        *view.inv = inv;
    }
    ClickResult::Crafted(result)
}

/// RMB drag paint: drop one carried item into `at` (empty or matching with
/// room). A grid cell the `station` doesn't have is skipped (B-L3: bounded
/// like [`click_grid`]). Returns true if a unit moved.
fn distribute_one(view: &mut WindowMut, at: WindowSlot, station: Station) -> bool {
    let Some(cursor) = view.cursor.take() else { return false };
    let target = match at {
        WindowSlot::Inv(slot) if slot < SLOTS => view.inv.slot(slot).cloned(),
        WindowSlot::Grid(r, c) if r < station.grid_size() && c < station.grid_size() => view.grid[r][c].clone(),
        _ => {
            *view.cursor = Some(cursor);
            return false;
        }
    };
    let deposit = match target {
        None => Some(ItemStack { item: cursor.item.clone(), count: 1 }),
        Some(ex) if ex.item.can_stack_with(&cursor.item) && ex.count < cursor.item.max_stack() => {
            Some(ItemStack { item: cursor.item.clone(), count: ex.count + 1 })
        }
        _ => None,
    };
    let Some(deposit) = deposit else {
        *view.cursor = Some(cursor);
        return false;
    };
    match at {
        WindowSlot::Inv(slot) => view.inv.set_slot(slot, Some(deposit)),
        WindowSlot::Grid(r, c) => view.grid[r][c] = Some(deposit),
    }
    // Keep the remaining cursor (or clear it if that was the last unit).
    if cursor.count > 1 {
        *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count - 1 });
    }
    true
}

/// LMB drag paint: gather matching items from `at` into the cursor. An
/// empty cursor adopts the slot's item type and starts collecting.
fn gather(view: &mut WindowMut, at: WindowSlot, station: Station) -> bool {
    let stack = match at {
        WindowSlot::Inv(slot) => view.inv.slot(slot).cloned(),
        WindowSlot::Grid(r, c) if r < station.grid_size() && c < station.grid_size() => view.grid[r][c].clone(),
        WindowSlot::Grid(..) => None,
    };
    let Some(stack) = stack else { return false };
    let leave = match view.cursor.take() {
        None => {
            *view.cursor = Some(stack);
            None
        }
        Some(cursor) => {
            if !cursor.item.can_stack_with(&stack.item) {
                *view.cursor = Some(cursor);
                return false;
            }
            let room = cursor.item.max_stack().saturating_sub(cursor.count);
            let take = stack.count.min(room);
            if take == 0 {
                *view.cursor = Some(cursor);
                return false;
            }
            *view.cursor = Some(ItemStack { item: cursor.item, count: cursor.count + take });
            let leave = stack.count - take;
            (leave > 0).then_some(ItemStack { item: stack.item, count: leave })
        }
    };
    match at {
        WindowSlot::Inv(slot) => view.inv.set_slot(slot, leave),
        WindowSlot::Grid(r, c) => view.grid[r][c] = leave,
    }
    true
}

/// Return every grid cell, then the cursor, to the inventory. `add_item` is
/// non-atomic, so what doesn't fit STAYS in its cell or on the cursor —
/// nothing is ever deleted (engine audit 2026-06-04, A). True when
/// everything went back.
fn return_grid_and_cursor(view: &mut WindowMut) -> bool {
    let mut all_placed = true;
    for cell in view.grid.iter_mut().flatten() {
        if let Some(stack) = cell.take()
            && let Some(remainder) = view.inv.add_item(stack) {
                *cell = Some(remainder); // keep what didn't fit
                all_placed = false;
            }
    }
    if let Some(cursor) = view.cursor.take()
        && let Some(remainder) = view.inv.add_item(cursor) {
            *view.cursor = Some(remainder);
            all_placed = false;
        }
    all_placed
}

/// [`WindowClick::Autofill`] (recipe book, 2026-06-12). Lay a catalogue
/// card's `example` grid by pulling the exact items from the inventory, so
/// the matcher then produces the card's output. The book never crafts
/// directly — it only drives this grid.
///
/// 1. The grid (and the cursor) go back to the inventory first; nothing is
///    destroyed. If the inventory fills mid-return, what's left stays put
///    and the fill is refused.
/// 2. If the inventory holds every item the example needs, they're moved in
///    and the grid becomes `example`.
/// 3. If anything is short, nothing is taken (step-1 returns stand) and the
///    grid is left empty; refused, so the caller can toast "not enough
///    materials".
///
/// v1 fills the example's *exact* items (e.g. an Oak Log for the any-log
/// plank recipe). Substituting a different in-set item the player happens to
/// hold (spruce log) is a deferred nicety.
fn autofill(view: &mut WindowMut, example: &[[CraftSlot; 3]; 3]) -> bool {
    // Step 1 — clear the grid + cursor back into the inventory. A cell that
    // doesn't fit stops the return there (later cells and the cursor are
    // left alone).
    for cell in view.grid.iter_mut().flatten() {
        if let Some(stack) = cell.take()
            && let Some(rem) = view.inv.add_item(stack) {
                *cell = Some(rem);
                return false;
            }
    }
    if let Some(cursor) = view.cursor.take()
        && let Some(rem) = view.inv.add_item(cursor) {
            *view.cursor = Some(rem);
            return false;
        }

    // Step 2 — tally what the example needs (exact items).
    let mut needs: Vec<(Item, u32)> = Vec::new();
    for slot in example.iter().flatten() {
        let item = match *slot {
            CraftSlot::Block(b) => Item::Block(b),
            CraftSlot::Material(m) => Item::Material(m),
            CraftSlot::Empty => continue,
        };
        if let Some(entry) = needs.iter_mut().find(|(it, _)| same_item(it, &item)) {
            entry.1 += 1;
        } else {
            needs.push((item, 1));
        }
    }

    // Step 3 — affordability check before taking anything.
    if needs.iter().any(|(item, count)| inv_count(view.inv, item) < *count) {
        return false; // grid already empty; items already returned
    }

    // Step 4 — remove from inventory and lay the grid.
    for (item, count) in &needs {
        for _ in 0..*count {
            let _ = inv_remove_one(view.inv, item);
        }
    }
    for (grid_row, example_row) in view.grid.iter_mut().zip(example.iter()) {
        for (cell, ex) in grid_row.iter_mut().zip(example_row.iter()) {
            *cell = match *ex {
                CraftSlot::Block(b) => Some(ItemStack::new_block(b, 1)),
                CraftSlot::Material(m) => Some(ItemStack::new_material(m, 1)),
                CraftSlot::Empty => None,
            };
        }
    }
    true
}

/// C3a-2a — one landed hit's wear on a set of equipped armour: every worn
/// piece loses one durability, and a piece that breaks is unequipped so it
/// neither keeps contributing points nor lingers as a zero-durability ghost.
/// The one rule: a player's own hits (`PlayerSlot::wear_armour`), a joined
/// client's `ArmourWorn`, and the server's copy of that joiner's armour.
pub fn wear_armour(armour: &mut [Option<ArmourItem>; 4]) {
    for slot in armour.iter_mut() {
        if let Some(piece) = slot.as_mut()
            && !piece.is_broken()
        {
            piece.durability = piece.durability.saturating_sub(1);
        }
        if slot.as_ref().is_some_and(|p| p.is_broken()) {
            *slot = None;
        }
    }
}

/// C3a-fix-1 — the station a window is at after `click` (applied at
/// `station`) gave `result`: a close that returned everything is back at the
/// player's grid; every other click leaves it. One rule for both copies of a
/// joiner's window (the client's screen, `window_ops::serve_op`).
pub fn station_after(station: Station, click: &WindowClick, result: &ClickResult) -> Station {
    if *click == WindowClick::Close && result.ok() {
        Station::Player
    } else {
        station
    }
}

/// C3a-2a — the window's digest ([`digest_parts`] over a view), at
/// `station` (C3a-fix-1: the screen the window is open at, after the op).
/// C3b-1 — with the open container's slots when the view has one
/// ([`digest_with`]).
pub fn digest(view: &WindowMut, station: Station) -> u32 {
    digest_with(view.inv, view.armour, view.cursor, view.grid, station, view.container.as_ref().map(|c| c.as_ref()))
}

/// C3a-2a — a stable 32-bit hash of a window: the 36 slots, the four armour
/// slots, the cursor and the grid (each slot's kind, id, count and
/// durability), then `auto_refill`. A joined client sends it with each
/// window op, after applying it; the server compares its own copy's
/// (log-only, `PossessionTally::window_mismatch`).
///
/// FNV-1a over explicit little-endian bytes, so every platform and build
/// agrees. A slot is a presence byte, then kind, id (u16), count and
/// durability (u16): Block = 1 (block id), Tool = 2 (type << 8 | tier),
/// Material = 3 (material id), Armour = 4 (slot << 8 | tier), Plan = 5 (no
/// id: a Plan has no wire form yet, so the server sees that slot empty and
/// the digest says so).
///
/// C3a-fix-1 (B-L6) — then the session locks (a u64 mask, bit `i` = slot `i`
/// locked, little-endian) and the station (`Player` = 0; `Table` = 1, then
/// the cell's x, y, z as i32): a lock or a station that differs shows at the
/// op that made it differ, not only at the next Sort or result click. Both
/// sides digest the station AFTER the op (a successful close is back at
/// `Player`). The hotbar selection is still not in it.
pub fn digest_parts(
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    cursor: &Option<ItemStack>,
    grid: &CraftGrid,
    station: Station,
) -> u32 {
    digest_with(inv, armour, cursor, grid, station, None)
}

/// C3b-1 — [`digest_parts`], then, when a container is open, a presence
/// byte, its kind (chest 0, furnace 1), its slot count (u16) and each slot
/// (a furnace's input, fuel, output). A container op's digest covers the
/// container on both sides; every other op's has none, so its digest is
/// [`digest_parts`]'s. A furnace's progress is not in it: it moves on the
/// server between the client's pushes.
pub fn digest_with(
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    cursor: &Option<ItemStack>,
    grid: &CraftGrid,
    station: Station,
    container: Option<crate::container_window::ContainerRef>,
) -> u32 {
    let mut h = Fnv32::default();
    for i in 0..SLOTS {
        h.stack(inv.slot(i));
    }
    for piece in armour {
        match piece {
            Some(p) => h.item(&Item::Armour(*p), 1),
            None => h.byte(0),
        }
    }
    h.stack(cursor.as_ref());
    for cell in grid.iter().flatten() {
        h.stack(cell.as_ref());
    }
    h.byte(u8::from(inv.auto_refill));
    let locks = (0..SLOTS).filter(|&i| inv.is_locked(i)).fold(0u64, |m, i| m | (1 << i));
    for b in locks.to_le_bytes() {
        h.byte(b);
    }
    match station {
        Station::Player => h.byte(0),
        Station::Table { cell } => {
            h.byte(1);
            for v in cell {
                for b in v.to_le_bytes() {
                    h.byte(b);
                }
            }
        }
    }
    if let Some(c) = container {
        h.byte(1);
        h.byte(match c {
            crate::container_window::ContainerRef::Chest(_) => 0,
            crate::container_window::ContainerRef::Furnace(_) => 1,
        });
        let n = c.len().min(usize::from(u16::MAX));
        h.u16(n as u16);
        for i in 0..n {
            h.stack(c.get(i));
        }
    }
    h.0
}

/// C3b-1 — one slot's fingerprint: the digest's hash of that slot alone
/// (kind, id, count, durability; a Plan content-free). Two slots print the
/// same exactly when the digest can't tell them apart. Names the slots a
/// container click changed, and what a joiner was last sent.
pub fn slot_print(stack: Option<&ItemStack>) -> u32 {
    let mut h = Fnv32::default();
    h.stack(stack);
    h.0
}

/// C3b-1 — an armour slot's fingerprint ([`slot_print`] of the piece).
pub fn armour_print(piece: Option<&ArmourItem>) -> u32 {
    let mut h = Fnv32::default();
    match piece {
        Some(p) => h.item(&Item::Armour(*p), 1),
        None => h.byte(0),
    }
    h.0
}

/// FNV-1a, 32 bits.
struct Fnv32(u32);

impl Default for Fnv32 {
    fn default() -> Self {
        Fnv32(0x811c_9dc5)
    }
}

impl Fnv32 {
    fn byte(&mut self, b: u8) {
        self.0 = (self.0 ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }

    fn u16(&mut self, v: u16) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }

    fn stack(&mut self, stack: Option<&ItemStack>) {
        match stack {
            Some(s) => self.item(&s.item, s.count),
            None => self.byte(0),
        }
    }

    fn item(&mut self, item: &Item, count: u8) {
        let (kind, id, durability) = match item {
            Item::Block(b) => (1, *b, 0),
            Item::Tool(t) => match crate::inventory::item_to_wire_full(item) {
                crate::protocol::WireItem::Tool { tool_type, material, .. } => {
                    (2, u16::from(tool_type) << 8 | u16::from(material), t.durability)
                }
                _ => (2, 0, t.durability),
            },
            Item::Material(m) => (3, *m as u16, 0),
            Item::Armour(a) => match crate::inventory::item_to_wire_full(item) {
                crate::protocol::WireItem::Armour { slot, material, .. } => {
                    (4, u16::from(slot) << 8 | u16::from(material), a.durability)
                }
                _ => (4, 0, a.durability),
            },
            Item::Plan(_) => (5, 0, 0),
        };
        self.byte(1);
        self.byte(kind);
        self.u16(id);
        self.byte(count);
        self.u16(durability);
    }
}

#[cfg(test)]
mod tests {
    //! Every `WindowClick` on plain values: no egui, no GPU.
    use super::*;
    use crate::armour::ArmourMaterial;
    use crate::block;
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::item::MaterialId;

    /// Owns the state a `WindowMut` borrows.
    struct Win {
        inv: Inventory,
        armour: [Option<ArmourItem>; 4],
        cursor: Option<ItemStack>,
        grid: CraftGrid,
    }

    impl Win {
        fn new() -> Self {
            Win { inv: Inventory::new(), armour: [None; 4], cursor: None, grid: Default::default() }
        }

        /// Apply `click` at `station`, by a body standing beside the table
        /// ([`NEAR`]) in a world where [`TABLE`]'s cell holds one.
        fn at(&mut self, station: Station, click: WindowClick) -> ClickResult {
            self.by(station, NEAR, block::CRAFTING_TABLE, click)
        }

        /// Apply `click` at `station` by a body whose eye is `eye`, where
        /// the table's cell holds `table_block`.
        fn by(&mut self, station: Station, eye: glam::Vec3, table_block: block::BlockId, click: WindowClick) -> ClickResult {
            let mut view = WindowMut {
                inv: &mut self.inv,
                armour: &mut self.armour,
                cursor: &mut self.cursor,
                grid: &mut self.grid,
                container: None,
            };
            apply(&mut view, &click, &ClickCtx::new(false, station, eye, |_| table_block))
        }

        fn click(&mut self, click: WindowClick) -> ClickResult {
            self.at(TABLE, click)
        }

        fn digest(&mut self) -> u32 {
            self.digest_at(Station::Player)
        }

        fn digest_at(&mut self, station: Station) -> u32 {
            digest_parts(&self.inv, &self.armour, &self.cursor, &self.grid, station)
        }

        fn cursor_count(&self) -> Option<u8> {
            self.cursor.as_ref().map(|s| s.count)
        }

        fn slot_count(&self, i: usize) -> Option<u8> {
            self.inv.slot(i).map(|s| s.count)
        }

        fn cell_count(&self, r: usize, c: usize) -> Option<u8> {
            self.grid[r][c].as_ref().map(|s| s.count)
        }

        /// Every unit of block `b` anywhere in the window.
        fn total(&self, b: crate::block::BlockId) -> u32 {
            let is_b = |s: &&ItemStack| matches!(s.item, Item::Block(x) if x == b);
            let slots: u32 = self.inv.slots_iter().flatten().filter(is_b).map(|s| u32::from(s.count)).sum();
            let grid: u32 = self.grid.iter().flatten().flatten().filter(is_b).map(|s| u32::from(s.count)).sum();
            let cursor: u32 = self.cursor.iter().filter(is_b).map(|s| u32::from(s.count)).sum();
            slots + grid + cursor
        }

        fn fill_with_pickaxes(&mut self) {
            for i in 0..SLOTS {
                self.inv.set_slot(i, Some(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
            }
        }
    }

    /// The crafting table the tests' 3×3 grid is opened from.
    const TABLE: Station = Station::Table { cell: [0, 64, 0] };

    /// An eye two blocks from [`TABLE`]'s cell: in reach.
    const NEAR: glam::Vec3 = glam::Vec3::new(0.5, 65.6, 2.5);

    fn stone(n: u8) -> ItemStack {
        ItemStack::new_block(block::STONE, n)
    }

    fn dirt(n: u8) -> ItemStack {
        ItemStack::new_block(block::DIRT, n)
    }

    fn piece(slot: ArmourSlot) -> ArmourItem {
        ArmourItem::new(slot, ArmourMaterial::Iron)
    }

    fn slot(slot: usize, right: bool) -> WindowClick {
        WindowClick::Slot { slot, right }
    }

    fn cell(row: usize, col: usize, right: bool) -> WindowClick {
        WindowClick::Grid { row, col, right }
    }

    // ── Slot ────────────────────────────────────────────────────────────

    #[test]
    fn slot_left_picks_up_the_whole_stack() {
        let mut w = Win::new();
        w.inv.set_slot(9, Some(stone(10)));
        assert_eq!(w.click(slot(9, false)), ClickResult::Done);
        assert_eq!(w.cursor_count(), Some(10));
        assert!(w.inv.slot(9).is_none());
    }

    #[test]
    fn slot_right_picks_up_the_ceil_half() {
        let mut w = Win::new();
        w.inv.set_slot(9, Some(stone(7)));
        w.click(slot(9, true));
        assert_eq!(w.cursor_count(), Some(4));
        assert_eq!(w.slot_count(9), Some(3));
        // A single item: the whole of it.
        w.cursor = None;
        w.inv.set_slot(9, Some(stone(1)));
        w.click(slot(9, true));
        assert_eq!(w.cursor_count(), Some(1));
        assert!(w.inv.slot(9).is_none());
    }

    #[test]
    fn slot_left_puts_the_whole_stack_down() {
        let mut w = Win::new();
        w.cursor = Some(stone(5));
        w.click(slot(9, false));
        assert_eq!(w.slot_count(9), Some(5));
        assert!(w.cursor.is_none());
    }

    #[test]
    fn slot_right_puts_one_down_and_adds_one() {
        let mut w = Win::new();
        w.cursor = Some(stone(5));
        w.click(slot(9, true));
        assert_eq!(w.slot_count(9), Some(1));
        assert_eq!(w.cursor_count(), Some(4));
        w.click(slot(9, true));
        assert_eq!(w.slot_count(9), Some(2));
        assert_eq!(w.cursor_count(), Some(3));
        // A full matching slot takes nothing.
        w.inv.set_slot(9, Some(stone(64)));
        w.click(slot(9, true));
        assert_eq!(w.slot_count(9), Some(64));
        assert_eq!(w.cursor_count(), Some(3));
    }

    #[test]
    fn slot_left_merges_up_to_max_and_keeps_the_rest() {
        let mut w = Win::new();
        w.inv.set_slot(9, Some(stone(60)));
        w.cursor = Some(stone(20));
        w.click(slot(9, false));
        assert_eq!(w.slot_count(9), Some(64));
        assert_eq!(w.cursor_count(), Some(16));
    }

    #[test]
    fn slot_merge_onto_an_overfull_slot_is_a_safe_no_op() {
        let mut w = Win::new();
        let mut over = stone(64);
        over.count = 250;
        w.inv.set_slot(9, Some(over));
        w.cursor = Some(stone(50));
        w.click(slot(9, false));
        assert_eq!(w.slot_count(9), Some(250));
        assert_eq!(w.cursor_count(), Some(50));
    }

    #[test]
    fn slot_swaps_a_different_item_with_either_button() {
        for right in [false, true] {
            let mut w = Win::new();
            w.inv.set_slot(9, Some(stone(3)));
            w.cursor = Some(dirt(5));
            w.click(slot(9, right));
            assert_eq!(w.inv.slot(9), Some(&dirt(5)));
            assert_eq!(w.cursor, Some(stone(3)));
        }
    }

    #[test]
    fn slot_empty_on_empty_is_done_and_changes_nothing() {
        let mut w = Win::new();
        assert_eq!(w.click(slot(9, false)), ClickResult::Done);
        assert!(w.cursor.is_none() && w.inv.slot(9).is_none());
    }

    #[test]
    fn slot_out_of_range_is_refused_and_keeps_the_cursor() {
        let mut w = Win::new();
        w.cursor = Some(stone(5));
        assert_eq!(w.click(slot(SLOTS, false)), ClickResult::Refused);
        assert_eq!(w.cursor_count(), Some(5));
    }

    #[test]
    fn a_locked_slot_still_takes_clicks() {
        // A lock binds sort, quick-stack and auto-refill, never a click.
        let mut w = Win::new();
        w.inv.set_slot(9, Some(stone(4)));
        w.inv.toggle_lock(9);
        w.click(slot(9, false));
        assert_eq!(w.cursor_count(), Some(4));
        assert!(w.inv.is_locked(9));
    }

    // ── Grid ────────────────────────────────────────────────────────────

    #[test]
    fn grid_left_and_right_pick_up_whole_and_half() {
        let mut w = Win::new();
        w.grid[0][0] = Some(stone(5));
        w.click(cell(0, 0, true));
        assert_eq!(w.cursor_count(), Some(3));
        assert_eq!(w.cell_count(0, 0), Some(2));
        w.cursor = None;
        w.click(cell(0, 0, false));
        assert_eq!(w.cursor_count(), Some(2));
        assert!(w.grid[0][0].is_none());
    }

    #[test]
    fn grid_put_down_whole_or_one() {
        let mut w = Win::new();
        w.cursor = Some(stone(5));
        w.click(cell(0, 0, true));
        assert_eq!(w.cell_count(0, 0), Some(1));
        assert_eq!(w.cursor_count(), Some(4));
        w.click(cell(0, 1, false));
        assert_eq!(w.cell_count(0, 1), Some(4));
        assert!(w.cursor.is_none());
    }

    #[test]
    fn grid_right_tops_up_by_one_and_left_merges_to_max() {
        let mut w = Win::new();
        w.grid[0][0] = Some(stone(2));
        w.cursor = Some(stone(70 - 6)); // 64
        w.click(cell(0, 0, true));
        assert_eq!(w.cell_count(0, 0), Some(3));
        assert_eq!(w.cursor_count(), Some(63));
        w.click(cell(0, 0, false));
        assert_eq!(w.cell_count(0, 0), Some(64));
        assert_eq!(w.cursor_count(), Some(2));
        // Full cell: neither button moves anything.
        w.click(cell(0, 0, true));
        w.click(cell(0, 0, false));
        assert_eq!(w.cell_count(0, 0), Some(64));
        assert_eq!(w.cursor_count(), Some(2));
    }

    #[test]
    fn grid_swaps_a_different_item_losslessly() {
        let mut w = Win::new();
        w.grid[0][0] = Some(stone(1));
        w.cursor = Some(dirt(10));
        w.click(cell(0, 0, false));
        assert_eq!(w.grid[0][0], Some(dirt(10)));
        assert_eq!(w.cursor, Some(stone(1)));
    }

    #[test]
    fn grid_cell_outside_the_players_2x2_is_refused() {
        let mut w = Win::new();
        w.cursor = Some(stone(5));
        assert_eq!(w.at(Station::Player, cell(2, 0, false)), ClickResult::Refused);
        assert_eq!(w.at(Station::Player, cell(0, 2, false)), ClickResult::Refused);
        assert_eq!(w.cursor_count(), Some(5));
        assert_eq!(w.at(Station::Player, cell(1, 1, false)), ClickResult::Done);
        assert_eq!(w.cell_count(1, 1), Some(5));
        // A table's 3×3 takes the corner; beyond the array is refused.
        w.cursor = Some(stone(1));
        assert_eq!(w.at(TABLE, cell(2, 2, false)), ClickResult::Done);
        assert_eq!(w.at(TABLE, cell(3, 0, false)), ClickResult::Refused);
    }

    // ── Armour ──────────────────────────────────────────────────────────

    #[test]
    fn armour_picks_up_the_piece() {
        let mut w = Win::new();
        w.armour[0] = Some(piece(ArmourSlot::Helmet));
        assert_eq!(w.click(WindowClick::Armour { slot: 0 }), ClickResult::Done);
        assert_eq!(w.cursor, Some(ItemStack { item: Item::Armour(piece(ArmourSlot::Helmet)), count: 1 }));
        assert!(w.armour[0].is_none());
    }

    #[test]
    fn armour_equips_and_swaps() {
        let mut w = Win::new();
        let old = ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Leather);
        w.armour[3] = Some(old);
        w.cursor = Some(ItemStack { item: Item::Armour(piece(ArmourSlot::Boots)), count: 1 });
        assert_eq!(w.click(WindowClick::Armour { slot: 3 }), ClickResult::Done);
        assert_eq!(w.armour[3], Some(piece(ArmourSlot::Boots)));
        assert_eq!(w.cursor, Some(ItemStack { item: Item::Armour(old), count: 1 }));
    }

    #[test]
    fn armour_for_the_wrong_slot_is_refused() {
        let mut w = Win::new();
        let boots = ItemStack { item: Item::Armour(piece(ArmourSlot::Boots)), count: 1 };
        w.cursor = Some(boots.clone());
        assert_eq!(w.click(WindowClick::Armour { slot: 0 }), ClickResult::Refused);
        assert_eq!(w.cursor, Some(boots));
        assert!(w.armour[0].is_none());
        // Not armour at all, or no such slot: refused too.
        w.cursor = Some(stone(1));
        assert_eq!(w.click(WindowClick::Armour { slot: 1 }), ClickResult::Refused);
        assert_eq!(w.click(WindowClick::Armour { slot: 4 }), ClickResult::Refused);
        assert_eq!(w.cursor, Some(stone(1)));
    }

    // ── Result ──────────────────────────────────────────────────────────

    #[test]
    fn result_consumes_one_per_cell_and_lands_on_the_cursor() {
        let mut w = Win::new();
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            w.grid[r][c] = Some(ItemStack::new_block(block::OAK_PLANKS, 2));
        }
        let table = recipe_output(&w.grid, TABLE).expect("four planks make a crafting table");
        assert_eq!(w.click(WindowClick::Result), ClickResult::Crafted(table.clone()));
        assert_eq!(w.cursor, Some(table));
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            assert_eq!(w.cell_count(r, c), Some(1), "one plank left in ({r},{c})");
        }
        // Again: the last plank of each cell goes, the cells empty.
        assert!(w.click(WindowClick::Result).ok());
        assert!(w.grid.iter().flatten().all(Option::is_none));
        assert_eq!(w.cursor_count(), Some(2));
    }

    #[test]
    fn result_merges_onto_a_matching_cursor_and_spills_the_overflow() {
        let mut w = Win::new();
        w.grid[0][0] = Some(ItemStack::new_block(block::OAK_LOG, 1));
        w.cursor = Some(ItemStack::new_block(block::OAK_PLANKS, 62));
        let planks = recipe_output(&w.grid, TABLE).expect("a log makes planks");
        assert!(w.click(WindowClick::Result).ok());
        assert_eq!(w.cursor_count(), Some(64), "the cursor fills to max");
        let spilled = u32::from(planks.count) - 2;
        assert_eq!(w.total(block::OAK_PLANKS), 62 + u32::from(planks.count));
        assert_eq!(w.inv.slots_iter().flatten().map(|s| u32::from(s.count)).sum::<u32>(), spilled);
    }

    #[test]
    fn result_under_a_different_cursor_goes_to_the_inventory() {
        let mut w = Win::new();
        w.grid[0][0] = Some(ItemStack::new_block(block::OAK_LOG, 1));
        w.cursor = Some(dirt(3));
        assert!(w.click(WindowClick::Result).ok());
        assert_eq!(w.cursor, Some(dirt(3)), "the cursor is never overwritten");
        assert!(w.total(block::OAK_PLANKS) > 0);
        assert!(w.grid[0][0].is_none());
    }

    #[test]
    fn result_that_does_not_fit_is_refused_and_consumes_nothing() {
        let mut w = Win::new();
        w.fill_with_pickaxes();
        w.grid[0][0] = Some(ItemStack::new_block(block::OAK_LOG, 1));
        w.cursor = Some(dirt(3));
        assert_eq!(w.click(WindowClick::Result), ClickResult::Refused);
        assert_eq!(w.cell_count(0, 0), Some(1));
        assert_eq!(w.cursor, Some(dirt(3)));
        w.cursor = Some(ItemStack::new_block(block::OAK_PLANKS, 62));
        assert_eq!(w.click(WindowClick::Result), ClickResult::Refused, "2 would be lost");
        assert_eq!(w.cell_count(0, 0), Some(1));
        assert_eq!(w.cursor_count(), Some(62));
    }

    #[test]
    fn result_of_a_grid_with_no_recipe_is_refused() {
        let mut w = Win::new();
        w.grid[0][0] = Some(dirt(1));
        w.grid[2][2] = Some(stone(1));
        assert_eq!(recipe_output(&w.grid, TABLE), None, "dirt and stone in opposite corners make nothing");
        assert_eq!(w.click(WindowClick::Result), ClickResult::Refused);
        assert_eq!(w.cell_count(0, 0), Some(1));
        assert_eq!(w.cell_count(2, 2), Some(1));
    }

    #[test]
    fn a_refused_close_leaves_a_grid_that_crafts_only_what_it_now_holds() {
        // L2: four planks, room for just one more plank. The close returns
        // one cell and is refused; the three planks left craft no table.
        let mut w = Win::new();
        w.fill_with_pickaxes();
        w.inv.set_slot(0, Some(ItemStack::new_block(block::OAK_PLANKS, 63)));
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            w.grid[r][c] = Some(ItemStack::new_block(block::OAK_PLANKS, 1));
        }
        let table = recipe_output(&w.grid, Station::Player).expect("four planks make a table");
        assert_eq!(w.at(Station::Player, WindowClick::Close), ClickResult::Refused);
        assert_eq!(w.grid.iter().flatten().flatten().count(), 3, "one plank went back");
        assert_eq!(w.total(block::OAK_PLANKS), 67, "no plank made or lost");
        assert_ne!(recipe_output(&w.grid, Station::Player), Some(table.clone()));
        assert_ne!(w.at(Station::Player, WindowClick::Result), ClickResult::Crafted(table));
    }

    // ── Trash ───────────────────────────────────────────────────────────

    #[test]
    fn trash_destroys_the_cursor_and_nothing_else() {
        let mut w = Win::new();
        w.inv.set_slot(0, Some(stone(9)));
        w.cursor = Some(dirt(5));
        assert_eq!(w.click(WindowClick::Trash), ClickResult::Binned(dirt(5)));
        assert!(w.cursor.is_none());
        assert_eq!(w.slot_count(0), Some(9));
        assert_eq!(w.click(WindowClick::Trash), ClickResult::Refused, "nothing held, nothing binned");
    }

    // ── Drags ───────────────────────────────────────────────────────────

    #[test]
    fn drag_distribute_drops_one_per_slot_across_inventory_and_grid() {
        let mut w = Win::new();
        w.cursor = Some(stone(5));
        w.inv.set_slot(12, Some(dirt(1)));
        let slots = vec![WindowSlot::Inv(9), WindowSlot::Inv(10), WindowSlot::Inv(12), WindowSlot::Grid(0, 0)];
        assert_eq!(w.click(WindowClick::DragDistribute { slots }), ClickResult::Done);
        assert_eq!(w.slot_count(9), Some(1));
        assert_eq!(w.slot_count(10), Some(1));
        assert_eq!(w.inv.slot(12), Some(&dirt(1)), "a different item is skipped");
        assert_eq!(w.cell_count(0, 0), Some(1));
        assert_eq!(w.cursor_count(), Some(2));
        assert_eq!(w.total(block::STONE), 5, "count conserved");
    }

    #[test]
    fn drag_distribute_stops_when_the_cursor_runs_out() {
        let mut w = Win::new();
        w.cursor = Some(stone(2));
        let slots = (9..13).map(WindowSlot::Inv).collect();
        assert!(w.click(WindowClick::DragDistribute { slots }).ok());
        assert_eq!((9..13).map(|s| w.slot_count(s)).collect::<Vec<_>>(), vec![Some(1), Some(1), None, None]);
        assert!(w.cursor.is_none());
        // Nothing carried, or nothing deposited: refused.
        let again = WindowClick::DragDistribute { slots: vec![WindowSlot::Inv(20)] };
        assert_eq!(w.click(again), ClickResult::Refused);
        w.cursor = Some(stone(1));
        w.inv.set_slot(20, Some(dirt(1)));
        let skip = WindowClick::DragDistribute { slots: vec![WindowSlot::Inv(20)] };
        assert_eq!(w.click(skip), ClickResult::Refused);
        assert_eq!(w.cursor_count(), Some(1));
    }

    #[test]
    fn drag_gather_pulls_matching_into_the_cursor() {
        let mut w = Win::new();
        w.inv.set_slot(9, Some(stone(3)));
        w.inv.set_slot(10, Some(stone(4)));
        w.inv.set_slot(11, Some(dirt(9)));
        w.grid[1][1] = Some(stone(2));
        let slots = vec![WindowSlot::Inv(9), WindowSlot::Inv(11), WindowSlot::Inv(10), WindowSlot::Grid(1, 1)];
        assert_eq!(w.click(WindowClick::DragGather { slots }), ClickResult::Done);
        assert_eq!(w.cursor_count(), Some(9), "3 + 4 + 2 gathered, the empty cursor adopted stone");
        assert_eq!(w.inv.slot(11), Some(&dirt(9)), "a different item is skipped");
        assert!(w.inv.slot(9).is_none() && w.inv.slot(10).is_none() && w.grid[1][1].is_none());
    }

    #[test]
    fn drag_gather_caps_at_max_and_refuses_when_nothing_moves() {
        let mut w = Win::new();
        w.cursor = Some(stone(60));
        w.inv.set_slot(9, Some(stone(10)));
        assert!(w.click(WindowClick::DragGather { slots: vec![WindowSlot::Inv(9)] }).ok());
        assert_eq!(w.cursor_count(), Some(64));
        assert_eq!(w.slot_count(9), Some(6));
        assert_eq!(w.click(WindowClick::DragGather { slots: vec![WindowSlot::Inv(9)] }), ClickResult::Refused);
        assert_eq!(w.click(WindowClick::DragGather { slots: vec![WindowSlot::Inv(30)] }), ClickResult::Refused);
    }

    // ── Sort and locks ──────────────────────────────────────────────────

    #[test]
    fn sort_merges_the_bag_and_leaves_the_hotbar() {
        let mut w = Win::new();
        w.inv.set_slot(0, Some(dirt(1)));
        w.inv.set_slot(20, Some(stone(30)));
        w.inv.set_slot(30, Some(stone(40)));
        assert_eq!(w.click(WindowClick::Sort), ClickResult::Done);
        assert_eq!(w.inv.slot(0), Some(&dirt(1)), "the hotbar is never sorted");
        assert_eq!(w.slot_count(BAG_START), Some(64));
        assert_eq!(w.slot_count(BAG_START + 1), Some(6));
        assert_eq!(w.total(block::STONE), 70);
    }

    #[test]
    fn sort_keeps_a_locked_slot_where_it_is() {
        let mut w = Win::new();
        w.inv.set_slot(30, Some(stone(5)));
        w.inv.set_slot(15, Some(dirt(2)));
        assert_eq!(w.click(WindowClick::ToggleLock { slot: 30 }), ClickResult::Done);
        assert!(w.inv.is_locked(30));
        w.click(WindowClick::Sort);
        assert_eq!(w.inv.slot(30), Some(&stone(5)), "the locked stack stays put");
        assert!(w.inv.slot(15).is_none(), "the unlocked one moved to the front");
        assert_eq!(w.inv.slot(BAG_START), Some(&dirt(2)));
    }

    #[test]
    fn toggle_lock_toggles_and_refuses_out_of_range() {
        let mut w = Win::new();
        w.click(WindowClick::ToggleLock { slot: 4 });
        assert!(w.inv.is_locked(4));
        w.click(WindowClick::ToggleLock { slot: 4 });
        assert!(!w.inv.is_locked(4));
        assert_eq!(w.click(WindowClick::ToggleLock { slot: SLOTS }), ClickResult::Refused);
    }

    // ── Autofill ────────────────────────────────────────────────────────

    fn example(name: &str) -> [[CraftSlot; 3]; 3] {
        crate::crafting_catalogue::all_cards()
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no card named {name}"))
            .example_grid
    }

    #[test]
    fn autofill_lays_the_example_from_the_inventory() {
        let mut w = Win::new();
        w.inv.set_slot(0, Some(ItemStack::new_material(MaterialId::IronIngot, 3)));
        w.inv.set_slot(1, Some(ItemStack::new_material(MaterialId::Stick, 2)));
        let ex = example("Iron Pickaxe");
        assert_eq!(w.click(WindowClick::Autofill { example: ex }), ClickResult::Done);
        assert!(recipe_output(&w.grid, TABLE).is_some(), "the laid grid crafts");
        assert_eq!(w.inv.count_material(MaterialId::IronIngot), 0);
        assert_eq!(w.inv.count_material(MaterialId::Stick), 0);
    }

    #[test]
    fn autofill_short_takes_nothing_and_empties_the_grid() {
        let mut w = Win::new();
        w.inv.set_slot(0, Some(ItemStack::new_material(MaterialId::IronIngot, 1)));
        w.inv.set_slot(1, Some(ItemStack::new_material(MaterialId::Stick, 2)));
        w.grid[2][2] = Some(dirt(5));
        w.cursor = Some(stone(3));
        let ex = example("Iron Pickaxe");
        assert_eq!(w.click(WindowClick::Autofill { example: ex }), ClickResult::Refused);
        assert_eq!(w.inv.count_material(MaterialId::IronIngot), 1);
        assert_eq!(w.inv.count_material(MaterialId::Stick), 2);
        assert!(w.grid.iter().flatten().all(Option::is_none));
        assert!(w.cursor.is_none(), "the cursor went back to the inventory");
        assert_eq!(w.total(block::DIRT), 5);
        assert_eq!(w.total(block::STONE), 3);
    }

    #[test]
    fn autofill_returns_the_old_grid_first() {
        let mut w = Win::new();
        w.grid[2][2] = Some(dirt(5));
        w.inv.set_slot(0, Some(ItemStack::new_block(block::OAK_PLANKS, 4)));
        assert!(w.click(WindowClick::Autofill { example: example("Crafting Table") }).ok());
        assert_eq!(w.total(block::DIRT), 5);
        assert!(w.grid.iter().flatten().flatten().all(|s| s.item == Item::Block(block::OAK_PLANKS)));
    }

    #[test]
    fn autofill_into_a_full_inventory_keeps_the_old_grid() {
        let mut w = Win::new();
        w.fill_with_pickaxes();
        w.grid[0][0] = Some(dirt(5));
        let ex = example("Crafting Table");
        assert_eq!(w.click(WindowClick::Autofill { example: ex }), ClickResult::Refused);
        assert_eq!(w.grid[0][0], Some(dirt(5)), "nothing destroyed");
    }

    #[test]
    fn autofill_of_a_table_recipe_into_the_players_grid_needs_a_table() {
        let mut w = Win::new();
        w.inv.set_slot(0, Some(ItemStack::new_material(MaterialId::IronIngot, 3)));
        w.inv.set_slot(1, Some(ItemStack::new_material(MaterialId::Stick, 2)));
        w.grid[0][0] = Some(dirt(5));
        let ex = example("Iron Pickaxe");
        assert_eq!(w.at(Station::Player, WindowClick::Autofill { example: ex }), ClickResult::NeedsTable);
        assert!(!ClickResult::NeedsTable.ok());
        assert_eq!(w.inv.count_material(MaterialId::IronIngot), 3, "nothing taken");
        assert_eq!(w.grid[0][0], Some(dirt(5)), "nothing moved");
        // At a table the same fill goes.
        assert!(w.at(TABLE, WindowClick::Autofill { example: ex }).ok());
    }

    #[test]
    fn every_2x2_card_fills_and_crafts_in_the_players_grid() {
        use crate::crafting_catalogue::{all_cards, CraftStation};
        for card in all_cards().iter().filter(|c| c.station == CraftStation::PlayerGrid) {
            let mut w = Win::new();
            for slot in card.example_grid.iter().flatten() {
                let stack = match *slot {
                    CraftSlot::Block(b) => ItemStack::new_block(b, 1),
                    CraftSlot::Material(m) => ItemStack::new_material(m, 1),
                    CraftSlot::Empty => continue,
                };
                assert!(w.inv.add_item(stack).is_none());
            }
            let fill = WindowClick::Autofill { example: card.example_grid };
            assert_eq!(w.at(Station::Player, fill), ClickResult::Done, "{} fills the 2×2", card.name);
            assert!(
                (0..3).all(|i| w.grid[2][i].is_none() && w.grid[i][2].is_none()),
                "{} is laid inside the 2×2",
                card.name
            );
            assert!(recipe_output(&w.grid, Station::Player).is_some(), "{} crafts in the 2×2", card.name);
        }
    }

    #[test]
    fn an_item_outside_the_players_2x2_crafts_nothing() {
        let mut w = Win::new();
        // Four planks in the bottom-right 2×2: a crafting table at a table,
        // nothing in the player's grid (those cells aren't on its screen).
        for (r, c) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
            w.grid[r][c] = Some(ItemStack::new_block(block::OAK_PLANKS, 1));
        }
        assert!(recipe_output(&w.grid, TABLE).is_some());
        assert_eq!(recipe_output(&w.grid, Station::Player), None);
        assert_eq!(w.at(Station::Player, WindowClick::Result), ClickResult::Refused);
        assert_eq!(w.total(block::OAK_PLANKS), 4, "nothing consumed");
    }

    #[test]
    fn a_tool_or_armour_in_the_grid_blocks_the_craft_and_survives() {
        let pickaxe = ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
        let helmet = ItemStack { item: Item::Armour(piece(ArmourSlot::Helmet)), count: 1 };
        for odd in [pickaxe, helmet] {
            let mut w = Win::new();
            for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                w.grid[r][c] = Some(ItemStack::new_block(block::OAK_PLANKS, 1));
            }
            assert!(recipe_output(&w.grid, TABLE).is_some(), "four planks alone craft");
            w.grid[2][2] = Some(odd.clone());
            assert_eq!(recipe_output(&w.grid, TABLE), None, "{odd:?} blocks the craft");
            assert_eq!(w.click(WindowClick::Result), ClickResult::Refused);
            assert_eq!(w.grid[2][2], Some(odd), "never destroyed");
            assert_eq!(w.total(block::OAK_PLANKS), 4);
        }
    }

    // ── The table's screen (L3) ─────────────────────────────────────────

    #[test]
    fn a_table_screen_stays_open_only_while_its_table_is_in_reach() {
        let cell = [0, 64, 0];
        let near = glam::Vec3::new(0.5, 65.6, 2.5);
        assert!(table_in_reach(block::CRAFTING_TABLE, cell, near));
        assert!(!table_in_reach(block::AIR, cell, near), "the table was broken");
        assert!(!table_in_reach(block::STONE, cell, near), "something else stands there");
        let far = glam::Vec3::new(0.5, 65.6, 12.5);
        assert!(!table_in_reach(block::CRAFTING_TABLE, cell, far), "walked or knocked out of reach");
        // The same line the server draws for a joiner's table craft.
        for z in 0..12 {
            let eye = glam::Vec3::new(0.5, 65.6, 0.5 + z as f32);
            assert_eq!(
                table_in_reach(block::CRAFTING_TABLE, cell, eye),
                crate::item_actions::cell_in_reach(eye, cell),
            );
        }
    }

    // ── Close ───────────────────────────────────────────────────────────

    #[test]
    fn close_returns_the_grid_and_cursor() {
        let mut w = Win::new();
        w.grid[0][0] = Some(stone(5));
        w.cursor = Some(dirt(3));
        assert_eq!(w.click(WindowClick::Close), ClickResult::Done);
        assert!(w.grid[0][0].is_none() && w.cursor.is_none());
        assert_eq!(w.total(block::STONE) + w.total(block::DIRT), 8);
    }

    #[test]
    fn close_into_a_full_inventory_keeps_what_does_not_fit() {
        let mut w = Win::new();
        w.fill_with_pickaxes();
        w.grid[0][0] = Some(stone(5));
        w.cursor = Some(dirt(3));
        assert_eq!(w.click(WindowClick::Close), ClickResult::Refused);
        assert_eq!(w.cell_count(0, 0), Some(5));
        assert_eq!(w.cursor_count(), Some(3));
    }

    // ── C3a-2a: the table's reach is part of the rule ───────────────────

    fn planks_table_grid(w: &mut Win) {
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            w.grid[r][c] = Some(ItemStack::new_block(block::OAK_PLANKS, 1));
        }
    }

    #[test]
    fn a_table_result_click_out_of_reach_or_at_a_gone_table_is_refused_and_moves_nothing() {
        let far = glam::Vec3::new(0.5, 65.6, 12.5);
        for (eye, at_cell) in [(far, block::CRAFTING_TABLE), (NEAR, block::AIR), (NEAR, block::STONE)] {
            let mut w = Win::new();
            planks_table_grid(&mut w);
            let before = w.digest();
            assert_eq!(w.by(TABLE, eye, at_cell, WindowClick::Result), ClickResult::Refused);
            assert_eq!(w.digest(), before, "nothing moved");
            // Grid clicks still work: the player can empty the grid by hand.
            assert_eq!(w.by(TABLE, eye, at_cell, cell(0, 0, false)), ClickResult::Done);
            assert_eq!(w.cursor_count(), Some(1));
        }
        // In reach, the same click crafts.
        let mut w = Win::new();
        planks_table_grid(&mut w);
        assert!(matches!(w.by(TABLE, NEAR, block::CRAFTING_TABLE, WindowClick::Result), ClickResult::Crafted(_)));
        // The player's own 2×2 needs no table.
        let mut w = Win::new();
        planks_table_grid(&mut w);
        assert!(matches!(w.by(Station::Player, far, block::AIR, WindowClick::Result), ClickResult::Crafted(_)));
    }

    #[test]
    fn autofill_at_a_table_out_of_reach_needs_a_table_and_moves_nothing() {
        let mut w = Win::new();
        w.inv.set_slot(0, Some(ItemStack::new_block(block::OAK_PLANKS, 4)));
        w.grid[0][0] = Some(dirt(2));
        let before = w.digest();
        let fill = WindowClick::Autofill { example: example("Crafting Table") };
        assert_eq!(w.by(TABLE, NEAR, block::AIR, fill.clone()), ClickResult::NeedsTable);
        assert_eq!(w.digest(), before);
        assert!(w.by(TABLE, NEAR, block::CRAFTING_TABLE, fill).ok());
    }

    #[test]
    fn a_drag_over_the_slot_bound_is_refused() {
        let mut w = Win::new();
        w.cursor = Some(stone(64));
        let mut slots: Vec<WindowSlot> = (0..SLOTS).map(WindowSlot::Inv).collect();
        slots.extend((0..3).flat_map(|r| (0..3).map(move |c| WindowSlot::Grid(r, c))));
        assert_eq!(slots.len(), MAX_DRAG_SLOTS);
        let mut over = slots.clone();
        over.push(WindowSlot::Inv(0));
        assert_eq!(w.click(WindowClick::DragDistribute { slots: over.clone() }), ClickResult::Refused);
        assert_eq!(w.click(WindowClick::DragGather { slots: over }), ClickResult::Refused);
        assert_eq!(w.cursor_count(), Some(64), "nothing moved");
        assert_eq!(w.click(WindowClick::DragDistribute { slots }), ClickResult::Done);
        assert_eq!(w.cursor_count(), Some(64 - MAX_DRAG_SLOTS as u8));
    }

    /// C3a-fix-2 B-L2 — the server's table verdict is a superset of an honest
    /// client's: a click 6.6 blocks from the server body while the client's
    /// eye is 6.3 off is judged in reach on both sides, but the server's slack
    /// stops at half a block, and the client's rule stays exact.
    #[test]
    fn the_servers_table_verdict_is_a_superset_of_the_clients() {
        let cell = [0, 64, 0];
        let centre = glam::Vec3::new(0.5, 64.5, 0.5);
        let ctx = |d: f32, block: block::BlockId| {
            ClickCtx::new(false, Station::Table { cell }, centre + glam::Vec3::new(0.0, 0.0, d), move |_| block)
        };
        let table = block::CRAFTING_TABLE;
        assert!(ctx(6.3, table).table_present(), "the client's eye is in reach");
        assert!(ctx(6.3, table).with_server_slack(false).table_present());
        assert!(!ctx(6.6, table).table_present(), "the client's own rule is exact");
        assert!(ctx(6.6, table).with_server_slack(false).table_present(), "the server's body is a little further");
        assert!(!ctx(6.9, table).with_server_slack(false).table_present(), "but the slack is half a block");
        // A table that just went: the client has closed it; the server allows
        // it for the grace, and only in reach.
        assert!(!ctx(2.0, block::AIR).table_present());
        assert!(!ctx(2.0, block::AIR).with_server_slack(false).table_present());
        assert!(ctx(2.0, block::AIR).with_server_slack(true).table_present());
        assert!(!ctx(9.0, block::AIR).with_server_slack(true).table_present(), "the grace never extends the reach");
        // The player's own grid needs no table.
        let own = ClickCtx::new(false, Station::Player, glam::Vec3::ZERO, |_| block::AIR);
        assert!(own.table_present() && own.with_server_slack(false).table_present());
    }

    /// C3a-fix-2 B-L3 — a drag is bounded by the station's grid, as a click
    /// is: at the player's 2×2 the hidden row and column take nothing and
    /// give nothing, at a table the whole 3×3 works.
    #[test]
    fn a_drag_cannot_reach_a_grid_cell_the_station_does_not_have() {
        let hidden = [WindowSlot::Grid(2, 2), WindowSlot::Grid(0, 2), WindowSlot::Grid(2, 0)];
        // Distribute: nothing lands in a hidden cell; the visible one takes.
        let mut w = Win::new();
        w.cursor = Some(stone(8));
        let mut slots = hidden.to_vec();
        assert_eq!(w.at(Station::Player, WindowClick::DragDistribute { slots: slots.clone() }), ClickResult::Refused);
        assert_eq!(w.cursor_count(), Some(8), "nothing moved");
        assert!(w.grid.iter().flatten().all(Option::is_none));
        slots.push(WindowSlot::Grid(1, 1));
        assert_eq!(w.at(Station::Player, WindowClick::DragDistribute { slots }), ClickResult::Done);
        assert_eq!(w.cell_count(1, 1), Some(1));
        assert_eq!(w.cursor_count(), Some(7));
        assert!(w.grid[2][2].is_none() && w.grid[0][2].is_none() && w.grid[2][0].is_none());
        // Gather: a stack parked in a hidden cell is not pulled out.
        let mut w = Win::new();
        w.grid[2][2] = Some(stone(3));
        assert_eq!(w.at(Station::Player, WindowClick::DragGather { slots: hidden.to_vec() }), ClickResult::Refused);
        assert!(w.cursor.is_none());
        assert_eq!(w.cell_count(2, 2), Some(3));
        // At a table the same cells work.
        assert_eq!(w.at(TABLE, WindowClick::DragGather { slots: hidden.to_vec() }), ClickResult::Done);
        assert_eq!(w.cursor_count(), Some(3));
        let mut w = Win::new();
        w.cursor = Some(stone(8));
        assert_eq!(w.at(TABLE, WindowClick::DragDistribute { slots: hidden.to_vec() }), ClickResult::Done);
        assert_eq!(w.cursor_count(), Some(5));
        assert_eq!(w.total(block::STONE), 8, "count conserved");
    }

    // ── C3a-2a: the digest ──────────────────────────────────────────────

    #[test]
    fn the_digest_sees_every_part_of_the_window() {
        let mut w = Win::new();
        let empty = w.digest();
        let mut seen = vec![empty];
        let mut changed = |w: &mut Win, what: &str| {
            let d = w.digest();
            assert!(!seen.contains(&d), "{what} changed nothing");
            seen.push(d);
        };
        w.inv.set_slot(35, Some(stone(1)));
        changed(&mut w, "a bag slot");
        w.inv.set_slot(35, Some(stone(2)));
        changed(&mut w, "its count");
        w.inv.set_slot(35, Some(dirt(2)));
        changed(&mut w, "its block");
        let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        w.inv.set_slot(0, Some(ItemStack::new_tool(pick)));
        changed(&mut w, "a tool");
        pick.durability -= 1;
        w.inv.set_slot(0, Some(ItemStack::new_tool(pick)));
        changed(&mut w, "its wear");
        w.armour[2] = Some(piece(ArmourSlot::Leggings));
        changed(&mut w, "an armour piece");
        w.armour[2].as_mut().unwrap().durability -= 1;
        changed(&mut w, "its wear");
        w.cursor = Some(ItemStack::new_material(MaterialId::Stick, 3));
        changed(&mut w, "the cursor");
        w.grid[2][2] = Some(stone(1));
        changed(&mut w, "a grid cell");
        w.inv.auto_refill = !w.inv.auto_refill;
        changed(&mut w, "auto-refill");
        // C3a-fix-1 (B-L6) — the locks and the station.
        w.inv.toggle_lock(35);
        changed(&mut w, "a lock");
        w.inv.toggle_lock(0);
        changed(&mut w, "another lock");
        let player = w.digest();
        let table = w.digest_at(TABLE);
        assert_ne!(player, table, "the station");
        assert_ne!(table, w.digest_at(Station::Table { cell: [0, 64, 1] }), "the table's cell");
        assert!(!seen.contains(&table));
        // Where a stack sits matters, not just what is held.
        let mut a = Win::new();
        a.inv.set_slot(9, Some(stone(5)));
        let mut b = Win::new();
        b.inv.set_slot(10, Some(stone(5)));
        assert_ne!(a.digest(), b.digest());
        // A Plan is no empty slot (the server, which can't hold one yet,
        // sees the slot empty: the digest tells them apart).
        let mut p = Win::new();
        p.inv.set_slot(3, Some(ItemStack { item: Item::Plan(crate::satoshi::starter_hut_plan()), count: 1 }));
        assert_ne!(p.digest(), Win::new().digest());
    }

    #[test]
    fn the_digest_is_stable() {
        // FNV-1a over fixed little-endian bytes: the same window hashes the
        // same on every platform and in every build (client and server).
        let mut w = Win::new();
        assert_eq!(w.digest(), EMPTY_WINDOW_DIGEST);
        w.inv.set_slot(0, Some(stone(3)));
        let d = w.digest();
        let mut again = Win::new();
        again.inv.set_slot(0, Some(stone(3)));
        assert_eq!(again.digest(), d);
    }

    /// The digest of an empty window with auto-refill on, at the player's
    /// grid (pinned): FNV-1a over fifty empty-slot bytes (36 + 4 + cursor +
    /// 9), the setting's 1, eight zero bytes of lock mask and the station's 0
    /// (C3a-fix-1 added the last two).
    const EMPTY_WINDOW_DIGEST: u32 = 2_574_539_244;

    // ── C3a-2a: armour wear ─────────────────────────────────────────────

    #[test]
    fn wear_armour_wears_every_piece_once_and_unequips_a_broken_one() {
        let mut armour = [Some(piece(ArmourSlot::Helmet)), None, Some(piece(ArmourSlot::Leggings)), None];
        let full = armour[0].unwrap().durability;
        armour[2].as_mut().unwrap().durability = 1;
        wear_armour(&mut armour);
        assert_eq!(armour[0].unwrap().durability, full - 1);
        assert_eq!(armour[2], None, "the last point broke it: unequipped");
        assert_eq!(armour[1], None);
    }
}
