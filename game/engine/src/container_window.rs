//! C3b-1 (2026-10-08, protocol v77; C3b-fix-a, v78) — the container screens' click rules,
//! one pure model shared by every side
//! (`docs/foundations/2026-10-07-c3-server-owned-inventory.md` §2, C3b row).
//!
//! A chest of any tier, a dispenser or dropper (9 slots on a `ChestData`)
//! and a furnace are part of the same window as the 36 slots, the armour,
//! the cursor and the craft grid: [`apply_container`] is ONE transition over
//! a [`WindowMut`] whose `container` hook holds the open container
//! ([`ContainerMut`]). It calls the existing pure rules (`chest_ui`'s
//! withdraw, deposit, Sort, Dump matching, Restock, Take all; `furnace`'s
//! slot click) and reports the slots it changed.
//!
//! - **Single-player and a host** apply it to the real block entity in
//!   their own world, exactly as the dialogs always did.
//! - **A joined client** applies it to a mirror of the server's container
//!   ([`SharedContainer`], from `ContainerOpened`) plus its own window, and
//!   sends it as a window op (`WireWindowOp::Container`).
//! - **The server** applies the same rule to the real container and its copy
//!   of the joiner's window, compares digests, and corrects the client when
//!   the real container changed the outcome (`WindowSlotSet`).
//!
//! **C3b-fix-a (v78) — corrections move items, never set a slot.** A
//! correction's player part is an [`ItemDelta`] ("take N of X", "give N of
//! X"), resolved on the window as it is when it lands, with a
//! [`CorrectionDebt`] so a take that finds its item gone is paid by the next
//! give of it; the server's re-run debits the claims of what corrections on
//! their way will take back ([`ClaimedWindow::debit`], the phantom ledger),
//! and a claim above its stack is refused ([`claims_fit_stacks`]).
//!
//! **Plans stay with their holder** (design §5). In a shared container
//! ([`ClickCtx::shared`]) a Plan can't go in, and a Plan already there (a
//! host put it in) can't come out: [`ClickResult::PlanStays`] on both
//! sides. A Plan's body has no wire form, so the joiner's mirror holds a
//! body-less placeholder (`plan::PlanData::placeholder`) that digests like
//! the real one (a Plan digests content-free).
//!
//! No egui, no GPU, no I/O.

use serde::{Deserialize, Serialize};

use crate::armour::ArmourItem;
use crate::block::{self, BlockId};
use crate::chest::{ChestData, ChestTier};
use crate::furnace::{ClickMode, FurnaceData, SlotClickResult, SlotKind};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};
use crate::protocol::{FurnaceView, WindowSlotSetPacket, WireSlot, WireWindowSlot};
use crate::window::{ClickCtx, ClickResult, CraftGrid, WindowMut, SLOTS};
use crate::world::World;

/// A furnace's slots in the container hook: input, fuel, output.
pub const FURNACE_INPUT: usize = 0;
pub const FURNACE_FUEL: usize = 1;
pub const FURNACE_OUTPUT: usize = 2;

/// The kind of container a screen shows. Wire data
/// (`ContainerOpenedPacket::kind`), APPEND ONLY: Chest = 0, Dispenser = 1,
/// Dropper = 2, Furnace = 3 (pinned by `protocol::tests::container_packets_round_trip`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerKind {
    /// A chest of `tier` (27 to 72 slots).
    Chest { tier: ChestTier },
    /// A dispenser's 9 slots.
    Dispenser,
    /// A dropper's 9 slots.
    Dropper,
    /// A furnace, lit or not: input, fuel, output.
    Furnace,
}

impl ContainerKind {
    /// The container a block is, if it is one a player opens a screen on.
    pub fn of_block(b: BlockId) -> Option<Self> {
        if let Some(tier) = ChestTier::from_block(b) {
            return Some(ContainerKind::Chest { tier });
        }
        match b {
            block::DISPENSER => Some(ContainerKind::Dispenser),
            block::DROPPER => Some(ContainerKind::Dropper),
            block::FURNACE | block::FURNACE_LIT => Some(ContainerKind::Furnace),
            _ => None,
        }
    }

    /// The screen's title.
    pub fn title(self) -> &'static str {
        match self {
            ContainerKind::Chest { tier } => tier.display_name(),
            ContainerKind::Dispenser => "Dispenser",
            ContainerKind::Dropper => "Dropper",
            ContainerKind::Furnace => "Furnace",
        }
    }
}

/// One click a container screen performs. Plain data, sent as a window op
/// (`protocol::WireWindowOp::Container`).
///
/// Wire-stable, APPEND ONLY: Withdraw = 0, Deposit = 1, Sort = 2,
/// DumpMatching = 3, Restock = 4, TakeAll = 5, Furnace = 6 (pinned by
/// `protocol::tests::container_packets_round_trip`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerClick {
    /// Click (or scroll over) chest/dispenser slot `slot`: one item into
    /// the inventory, or the whole stack (`all`: shift)
    /// (`chest_ui::withdraw_chest_slot`). What doesn't fit stays.
    Withdraw { slot: usize, all: bool },
    /// Click (or scroll over) inventory slot `slot` (0..36) in the chest
    /// screen: one item into the container, or the whole stack (`all`)
    /// (`chest_ui::deposit_to_chest`). What doesn't fit stays.
    Deposit { slot: usize, all: bool },
    /// The Sort button: merge and order the container (`chest_ui::sort_chest`).
    Sort,
    /// "Dump matching": every unlocked stack whose item the container
    /// already holds goes in (`chest_ui::dump_matching`).
    DumpMatching,
    /// "Restock": every container stack whose item the player already holds
    /// comes out (`chest_ui::restock_from_chest`).
    Restock,
    /// "Take all": the whole container into the inventory, best-effort
    /// (`chest_ui::take_all`).
    TakeAll,
    /// A furnace slot click (`furnace::apply_slot_click`): `kind` × `mode`,
    /// with hotbar slot `hotbar` held (input and fuel are fed from the held
    /// stack). The slot rides with the click: the server's latest input may
    /// name an older one.
    Furnace { kind: SlotKind, mode: ClickMode, hotbar: usize },
}

/// The open container a [`WindowMut`] borrows.
pub enum ContainerMut<'a> {
    /// A chest of any tier, or a dispenser/dropper's embedded 9 slots.
    Chest(&'a mut ChestData),
    /// A furnace (slots [`FURNACE_INPUT`], [`FURNACE_FUEL`], [`FURNACE_OUTPUT`]).
    Furnace(&'a mut FurnaceData),
}

/// A read-only view of an open container.
#[derive(Clone, Copy)]
pub enum ContainerRef<'a> {
    Chest(&'a ChestData),
    Furnace(&'a FurnaceData),
}

impl ContainerMut<'_> {
    pub fn as_ref(&self) -> ContainerRef<'_> {
        match self {
            ContainerMut::Chest(c) => ContainerRef::Chest(c),
            ContainerMut::Furnace(f) => ContainerRef::Furnace(f),
        }
    }

    /// C3b-fix-a — become `data` (a copy of this container a click was
    /// first run on); a copy of the other kind changes nothing.
    pub fn replace_with(&mut self, data: &ContainerData) {
        match (self, data) {
            (ContainerMut::Chest(c), ContainerData::Chest(d)) => **c = d.clone(),
            (ContainerMut::Furnace(f), ContainerData::Furnace(d)) => **f = (**d).clone(),
            _ => {}
        }
    }

    /// Set slot `i` (ignored past the end).
    pub fn set(&mut self, i: usize, stack: Option<ItemStack>) {
        match self {
            ContainerMut::Chest(c) => {
                if let Some(slot) = c.slots.get_mut(i) {
                    *slot = stack;
                }
            }
            ContainerMut::Furnace(f) => match i {
                FURNACE_INPUT => f.input = stack,
                FURNACE_FUEL => f.fuel = stack,
                FURNACE_OUTPUT => f.output = stack,
                _ => {}
            },
        }
    }
}

impl<'a> ContainerRef<'a> {
    /// How many slots it has.
    pub fn len(self) -> usize {
        match self {
            ContainerRef::Chest(c) => c.slots.len(),
            ContainerRef::Furnace(_) => 3,
        }
    }

    /// Slot `i`, `None` when empty or past the end.
    pub fn get(self, i: usize) -> Option<&'a ItemStack> {
        match self {
            ContainerRef::Chest(c) => c.slots.get(i).and_then(Option::as_ref),
            ContainerRef::Furnace(f) => match i {
                FURNACE_INPUT => f.input.as_ref(),
                FURNACE_FUEL => f.fuel.as_ref(),
                FURNACE_OUTPUT => f.output.as_ref(),
                _ => None,
            },
        }
    }

    /// A furnace's progress (`None` for a chest).
    pub fn furnace_view(self) -> Option<FurnaceView> {
        match self {
            ContainerRef::Furnace(f) => Some(FurnaceView::of(f)),
            ContainerRef::Chest(_) => None,
        }
    }

    /// Every slot on the wire, in order.
    pub fn wire_slots(self) -> Vec<WireSlot> {
        (0..self.len()).map(|i| self.get(i).map(crate::inventory::stack_to_wire)).collect()
    }

    /// Every slot's fingerprint ([`crate::window::slot_print`]), in order:
    /// the server's record of what a joiner was last sent.
    pub fn prints(self) -> Vec<u32> {
        (0..self.len()).map(|i| crate::window::slot_print(self.get(i))).collect()
    }
}

/// What one container click did.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerApplied {
    /// The rule's result.
    pub result: ClickResult,
    /// The window slots (container ones included) whose contents it changed.
    pub touched: Vec<WireWindowSlot>,
}

/// Apply one container click to the window and its open container
/// (`view.container`). No container open, or a click for the other kind
/// (a furnace click at a chest), is `Refused` and changes nothing. In a
/// shared container (`ctx.shared`) a Plan neither goes in nor comes out
/// ([`ClickResult::PlanStays`]), and Take all leaves Plans where they are.
pub fn apply_container(view: &mut WindowMut, click: &ContainerClick, ctx: &ClickCtx) -> ContainerApplied {
    let before = Prints::of(view);
    let result = apply_rule(view, click, ctx);
    let touched = before.changed(view);
    ContainerApplied { result, touched }
}

fn is_plan(stack: Option<&ItemStack>) -> bool {
    matches!(stack, Some(ItemStack { item: Item::Plan(_), .. }))
}

fn done_if(ok: bool) -> ClickResult {
    if ok { ClickResult::Done } else { ClickResult::Refused }
}

fn apply_rule(view: &mut WindowMut, click: &ContainerClick, ctx: &ClickCtx) -> ClickResult {
    let WindowMut { inv, container, .. } = view;
    let inv = &mut **inv;
    let Some(container) = container.as_mut() else {
        return ClickResult::Refused;
    };
    match (container, click) {
        (ContainerMut::Chest(chest), ContainerClick::Withdraw { slot, all }) => {
            if ctx.shared && is_plan(chest.slots.get(*slot).and_then(Option::as_ref)) {
                return ClickResult::PlanStays;
            }
            done_if(crate::chest_ui::withdraw_chest_slot(chest, *slot, inv, *all))
        }
        (ContainerMut::Chest(chest), ContainerClick::Deposit { slot, all }) => {
            if *slot >= SLOTS {
                return ClickResult::Refused;
            }
            if ctx.shared && is_plan(inv.slot(*slot)) {
                return ClickResult::PlanStays;
            }
            done_if(crate::chest_ui::deposit_to_chest(inv, *slot, chest, *all))
        }
        (ContainerMut::Chest(chest), ContainerClick::Sort) => {
            crate::chest_ui::sort_chest(chest);
            ClickResult::Done
        }
        // A Plan never stacks, so it never "matches" what the other side
        // holds: Dump matching and Restock never move one.
        (ContainerMut::Chest(chest), ContainerClick::DumpMatching) => done_if(crate::chest_ui::dump_matching(inv, chest)),
        (ContainerMut::Chest(chest), ContainerClick::Restock) => done_if(crate::chest_ui::restock_from_chest(inv, chest)),
        (ContainerMut::Chest(chest), ContainerClick::TakeAll) => {
            let shared = ctx.shared;
            done_if(crate::chest_ui::take_all_where(inv, chest, |s| !(shared && is_plan(Some(s)))))
        }
        (ContainerMut::Furnace(furnace), ContainerClick::Furnace { kind, mode, hotbar }) => {
            if *hotbar >= crate::window::BAG_START {
                return ClickResult::Refused;
            }
            match crate::furnace::apply_slot_click(furnace, inv, *hotbar, *kind, *mode) {
                SlotClickResult::Moved { .. } => ClickResult::Done,
                SlotClickResult::NoOp => ClickResult::Refused,
            }
        }
        _ => ClickResult::Refused,
    }
}

/// Every slot's fingerprint before a click, to name what it changed.
struct Prints {
    inv: [u32; SLOTS],
    armour: [u32; 4],
    cursor: u32,
    grid: [u32; 9],
    container: Vec<u32>,
}

impl Prints {
    fn of(view: &WindowMut) -> Self {
        use crate::window::slot_print;
        let mut inv = [0; SLOTS];
        for (i, p) in inv.iter_mut().enumerate() {
            *p = slot_print(view.inv.slot(i));
        }
        let mut armour = [0; 4];
        for (p, piece) in armour.iter_mut().zip(view.armour.iter()) {
            *p = crate::window::armour_print(piece.as_ref());
        }
        let mut grid = [0; 9];
        for (p, cell) in grid.iter_mut().zip(view.grid.iter().flatten()) {
            *p = slot_print(cell.as_ref());
        }
        Prints {
            inv,
            armour,
            cursor: slot_print(view.cursor.as_ref()),
            grid,
            container: view.container.as_ref().map(|c| c.as_ref().prints()).unwrap_or_default(),
        }
    }

    /// The slots of `view` whose print differs from this one's.
    fn changed(&self, view: &WindowMut) -> Vec<WireWindowSlot> {
        let now = Prints::of(view);
        let mut touched = Vec::new();
        for i in 0..SLOTS {
            if now.inv[i] != self.inv[i] {
                touched.push(WireWindowSlot::Inv(i as u8));
            }
        }
        for i in 0..4 {
            if now.armour[i] != self.armour[i] {
                touched.push(WireWindowSlot::Armour(i as u8));
            }
        }
        if now.cursor != self.cursor {
            touched.push(WireWindowSlot::Cursor);
        }
        for i in 0..9 {
            if now.grid[i] != self.grid[i] {
                touched.push(WireWindowSlot::Grid((i / 3) as u8, (i % 3) as u8));
            }
        }
        let n = now.container.len().max(self.container.len());
        for i in 0..n {
            if now.container.get(i) != self.container.get(i) {
                touched.push(WireWindowSlot::Container(i as u8));
            }
        }
        touched
    }
}

// ── Claims (v77): a container op re-run over the client's own slots ─────

/// The window slots a container click names: the container slot it
/// withdraws from, the inventory slot it deposits from, or the furnace slot
/// (and, feeding it, the held hotbar slot) it clicks. A correction always
/// covers its container slots, and an op always claims its player slots,
/// whatever either side's apply changed.
pub fn named_slots(click: &ContainerClick) -> Vec<WireWindowSlot> {
    let small = |i: usize| u8::try_from(i).ok();
    match click {
        ContainerClick::Withdraw { slot, .. } => small(*slot).map(WireWindowSlot::Container).into_iter().collect(),
        ContainerClick::Deposit { slot, .. } => {
            small(*slot).filter(|&i| usize::from(i) < SLOTS).map(WireWindowSlot::Inv).into_iter().collect()
        }
        ContainerClick::Furnace { kind, hotbar, .. } => {
            let at = match kind {
                SlotKind::Input => FURNACE_INPUT,
                SlotKind::Fuel => FURNACE_FUEL,
                SlotKind::Output => FURNACE_OUTPUT,
            };
            let mut named = vec![WireWindowSlot::Container(at as u8)];
            if *kind != SlotKind::Output
                && let Some(h) = small(*hotbar).filter(|&h| usize::from(h) < crate::window::BAG_START)
            {
                named.push(WireWindowSlot::Inv(h));
            }
            named
        }
        ContainerClick::Sort | ContainerClick::DumpMatching | ContainerClick::Restock | ContainerClick::TakeAll => Vec::new(),
    }
}

/// Is `at` one of the player's own slots (36, armour, cursor, grid), not
/// the container's?
pub fn is_player_slot(at: WireWindowSlot) -> bool {
    !matches!(at, WireWindowSlot::Container(_))
}

/// The player slots a container op claims (`WindowOpPacket::claims`): the
/// ones its apply changed (`touched`), the ones the click names
/// ([`named_slots`]: a deposit's source even when nothing moved, a furnace
/// click's held slot), and for Restock all 36 — its rule reads every slot
/// ("does the player already hold it?"). Every other slot is a blocker in
/// the server's re-run ([`ClaimedWindow`]), so the claims must cover every
/// player slot whose value the rule's outcome depends on.
pub fn claim_slots(click: &ContainerClick, touched: &[WireWindowSlot]) -> Vec<WireWindowSlot> {
    let mut out: Vec<WireWindowSlot> = Vec::new();
    let mut push = |at: WireWindowSlot| {
        if is_player_slot(at) && !out.contains(&at) {
            out.push(at);
        }
    };
    touched.iter().copied().for_each(&mut push);
    named_slots(click).into_iter().for_each(&mut push);
    if *click == ContainerClick::Restock {
        (0..SLOTS).for_each(|i| push(WireWindowSlot::Inv(i as u8)));
    }
    out
}

/// A copy of a window's player slots, taken before a container click so the
/// claims carry their values before it.
#[derive(Clone, Debug)]
pub struct PlayerSlots {
    inv: Vec<Option<ItemStack>>,
    armour: [Option<ArmourItem>; 4],
    cursor: Option<ItemStack>,
    grid: CraftGrid,
}

impl PlayerSlots {
    pub fn of(inv: &Inventory, armour: &[Option<ArmourItem>; 4], cursor: &Option<ItemStack>, grid: &CraftGrid) -> Self {
        PlayerSlots { inv: (0..SLOTS).map(|i| inv.slot(i).cloned()).collect(), armour: *armour, cursor: cursor.clone(), grid: grid.clone() }
    }

    /// Player slot `at`'s stack; `None` for a container slot or one past
    /// the end.
    pub fn stack(&self, at: WireWindowSlot) -> Option<Option<ItemStack>> {
        Some(match at {
            WireWindowSlot::Inv(i) => self.inv.get(usize::from(i))?.clone(),
            WireWindowSlot::Armour(i) => {
                self.armour.get(usize::from(i))?.map(|p| ItemStack { item: Item::Armour(p), count: 1 })
            }
            WireWindowSlot::Cursor => self.cursor.clone(),
            WireWindowSlot::Grid(r, c) if r < 3 && c < 3 => self.grid[usize::from(r)][usize::from(c)].clone(),
            _ => return None,
        })
    }

    /// Player slot `at` on the wire, at full fidelity (a Plan as the
    /// placeholder kind).
    pub fn wire(&self, at: WireWindowSlot) -> Option<WireSlot> {
        self.stack(at).map(|s| s.as_ref().map(crate::inventory::stack_to_wire))
    }
}

/// The stand-in an unclaimed inventory slot holds in the server's re-run: a
/// one-off tool, which never stacks with anything (`Item::can_stack_with`),
/// so `add_item` can't merge into or place in it, Dump matching and Restock
/// never match it, and nothing is ever taken from it (a deposit's source and
/// a furnace's held slot are always claimed). Never sent.
fn blocker() -> ItemStack {
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Wood))
}

/// The window the server re-runs a joiner's container op over (C3b-1,
/// v77): the claimed player slots at the values the client claims they held
/// before it (`WindowOpPacket::claims`; a Plan as the placeholder, which
/// stays put), and every other inventory slot a [`blocker`]. Run against the
/// REAL container it is the shared truth for the container and, for the
/// claimed slots, a result relative to the client's own state — never the
/// server's copy, which drifts until C3d (local uses aren't mirrored yet).
/// A claim that doesn't decode stays a blocker.
pub struct ClaimedWindow {
    inv: Inventory,
    armour: [Option<ArmourItem>; 4],
    cursor: Option<ItemStack>,
    grid: CraftGrid,
    /// The player slots claimed, in claim order.
    claimed: Vec<WireWindowSlot>,
}

impl ClaimedWindow {
    pub fn from_claims(claims: &[(WireWindowSlot, WireSlot)], registry: &crate::block::BlockRegistry) -> Self {
        let mut inv = Inventory::new();
        for i in 0..SLOTS {
            inv.set_slot(i, Some(blocker()));
        }
        let mut w = ClaimedWindow { inv, armour: [None; 4], cursor: None, grid: Default::default(), claimed: Vec::new() };
        for (at, value) in claims {
            if !is_player_slot(*at) || w.claimed.contains(at) {
                continue;
            }
            let stack = match value {
                None => None,
                Some(wire) => match crate::inventory::stack_from_wire(wire, registry, true) {
                    Some(s) => Some(s),
                    None => continue,
                },
            };
            let set = match *at {
                WireWindowSlot::Inv(i) if usize::from(i) < SLOTS => {
                    w.inv.set_slot(usize::from(i), stack);
                    true
                }
                WireWindowSlot::Armour(i) if i < 4 => match stack {
                    None => true,
                    Some(ItemStack { item: Item::Armour(piece), .. }) => {
                        w.armour[usize::from(i)] = Some(piece);
                        true
                    }
                    Some(_) => false,
                },
                WireWindowSlot::Cursor => {
                    w.cursor = stack;
                    true
                }
                WireWindowSlot::Grid(r, c) if r < 3 && c < 3 => {
                    w.grid[usize::from(r)][usize::from(c)] = stack;
                    true
                }
                _ => false,
            };
            if set {
                w.claimed.push(*at);
            }
        }
        w
    }

    /// Apply `click` to this window and `container` (the shared rule,
    /// [`apply_container`]).
    pub fn apply(&mut self, container: ContainerMut, click: &ContainerClick, ctx: &ClickCtx) -> ContainerApplied {
        let mut view = WindowMut {
            inv: &mut self.inv,
            armour: &mut self.armour,
            cursor: &mut self.cursor,
            grid: &mut self.grid,
            container: Some(container),
        };
        apply_container(&mut view, click, ctx)
    }

    /// Claimed slot `at`'s stack now (`None` for an unclaimed one).
    #[cfg(test)]
    pub fn stack(&self, at: WireWindowSlot) -> Option<Option<ItemStack>> {
        if !self.claimed.contains(&at) {
            return None;
        }
        PlayerSlots::of(&self.inv, &self.armour, &self.cursor, &self.grid).stack(at)
    }

    /// C3b-fix-a — the first claimed inventory slot holding `item` (by
    /// `joiner_actions::same_item`): where a correction's take of it looks
    /// first (where the prediction put it).
    pub fn slot_holding(&self, item: &Item) -> Option<u8> {
        self.claimed.iter().find_map(|at| match *at {
            WireWindowSlot::Inv(i)
                if self.inv.slot(usize::from(i)).is_some_and(|s| crate::joiner_actions::same_item(&s.item, item)) =>
            {
                Some(i)
            }
            _ => None,
        })
    }

    /// C3b-fix-a (v78) — the phantom ledger: take `n` of `item` off the
    /// CLAIMED slots, by the owed search's order (the 36 slots, then the
    /// grid, then the cursor; never a blocker): units a later op claims that
    /// a correction still on its way takes back. Returns how many it took.
    pub fn debit(&mut self, item: &Item, n: u32) -> u32 {
        let mut left = n;
        let order = self
            .claimed
            .iter()
            .copied()
            .filter(|at| matches!(at, WireWindowSlot::Inv(_)))
            .chain(self.claimed.iter().copied().filter(|at| matches!(at, WireWindowSlot::Grid(..))))
            .chain(self.claimed.iter().copied().filter(|at| matches!(at, WireWindowSlot::Cursor)))
            .collect::<Vec<_>>();
        for at in order {
            if left == 0 {
                break;
            }
            let cell: &mut Option<ItemStack> = match at {
                WireWindowSlot::Inv(i) if usize::from(i) < SLOTS => {
                    let mut stack = self.inv.take_slot(usize::from(i));
                    let took = take_from(&mut stack, item, left);
                    self.inv.set_slot(usize::from(i), stack);
                    left -= took;
                    continue;
                }
                WireWindowSlot::Grid(r, c) if r < 3 && c < 3 => &mut self.grid[usize::from(r)][usize::from(c)],
                WireWindowSlot::Cursor => &mut self.cursor,
                _ => continue,
            };
            left -= take_from(cell, item, left);
        }
        n - left
    }
}

/// Take up to `n` of `item` (by `joiner_actions::same_item`) from `cell`;
/// how many it took.
fn take_from(cell: &mut Option<ItemStack>, item: &Item, n: u32) -> u32 {
    let Some(stack) = cell.as_mut().filter(|s| crate::joiner_actions::same_item(&s.item, item)) else { return 0 };
    let took = u32::from(stack.count).min(n);
    stack.count -= took as u8;
    if stack.count == 0 {
        *cell = None;
    }
    took
}

/// C3b-fix-a (C-L3) — could a well-behaved client hold every claimed stack?
/// None counts above its item's `max_stack()` (so a tool or armour piece
/// above one). A claim that doesn't decode stays a blocker, as before.
pub fn claims_fit_stacks(claims: &[(WireWindowSlot, WireSlot)], registry: &crate::block::BlockRegistry) -> bool {
    claims.iter().all(|(_, value)| {
        value
            .as_ref()
            .and_then(|w| crate::inventory::stack_from_wire(w, registry, true))
            .is_none_or(|s| s.count <= s.item.max_stack())
    })
}

// ── C3b-fix-a (v78): a correction moves items, never sets a slot ────────

/// Units of each item, by exact identity, in a canonical order (by the
/// item's print), so the client and the server split a delta the same way.
pub type ItemCounts = Vec<(Item, i64)>;

fn item_key(item: &Item) -> u32 {
    crate::window::slot_print(Some(&ItemStack { item: item.clone(), count: 1 }))
}

fn add_count(counts: &mut ItemCounts, item: &Item, n: i64) {
    if n == 0 {
        return;
    }
    match counts.iter_mut().find(|(i, _)| i == item) {
        Some((_, c)) => *c += n,
        None => counts.push((item.clone(), n)),
    }
}

/// Every item container `c` holds, counted.
pub fn counts_of(c: ContainerRef) -> ItemCounts {
    let mut out = Vec::new();
    for i in 0..c.len() {
        if let Some(s) = c.get(i) {
            add_count(&mut out, &s.item, i64::from(s.count));
        }
    }
    out
}

/// `a − b`, item by item, zeros dropped, in canonical order.
pub fn counts_minus(a: &ItemCounts, b: &ItemCounts) -> ItemCounts {
    let mut out = a.clone();
    for (item, n) in b {
        add_count(&mut out, item, -n);
    }
    out.retain(|(_, n)| *n != 0);
    out.sort_by_key(|(item, _)| item_key(item));
    out
}

/// What a container click gave the player, item by item: what the
/// container lost from `before` to `after` (the rules only move items
/// between the two; a negative count is what the player put in).
pub fn player_gain(before: ContainerRef, after: ContainerRef) -> ItemCounts {
    counts_minus(&counts_of(before), &counts_of(after))
}

/// C3b-fix-a (v78) — a correction's change to a player's window, by item,
/// never by slot (C-H1): take `n` of an item (from a hint slot first, then
/// wherever it is, `joiner_actions::take_owed_window`), and give stacks
/// (`Inventory::add_item`). Resolved when the correction is applied, on the
/// window as it is then, so an op the client made meanwhile keeps its
/// effect. A Plan is never in one (no rule moves a Plan in a shared
/// container, and it has no wire form).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemDelta {
    /// Each `(hint, stack)`: at most a stack, looked for in inventory slot
    /// `hint` first.
    pub take: Vec<(u8, ItemStack)>,
    /// Each at most a stack.
    pub give: Vec<ItemStack>,
}

impl ItemDelta {
    /// From net counts (positive: give, negative: take), each split into
    /// stacks of at most its item's `max_stack()`. `hint` names the slot a
    /// take of an item looks in first.
    pub fn from_counts(net: &ItemCounts, hint: impl Fn(&Item) -> u8) -> Self {
        let mut d = ItemDelta::default();
        for (item, n) in net {
            if matches!(item, Item::Plan(_)) {
                continue;
            }
            let max = i64::from(item.max_stack().max(1));
            let mut left = n.abs();
            while left > 0 {
                let count = left.min(max);
                left -= count;
                let stack = ItemStack { item: item.clone(), count: count as u8 };
                if *n > 0 {
                    d.give.push(stack);
                } else {
                    d.take.push((hint(item), stack));
                }
            }
        }
        d
    }

    pub fn is_empty(&self) -> bool {
        self.take.is_empty() && self.give.is_empty()
    }

    /// Net counts: what it gives minus what it takes, item by item.
    pub fn counts(&self) -> ItemCounts {
        let mut out = Vec::new();
        for s in &self.give {
            add_count(&mut out, &s.item, i64::from(s.count));
        }
        for (_, s) in &self.take {
            add_count(&mut out, &s.item, -i64::from(s.count));
        }
        out.retain(|(_, n)| *n != 0);
        out
    }

    /// On the wire (`WindowSlotSetPacket::take`, `::give`).
    pub fn to_wire(&self) -> (Vec<(u8, WireStack)>, Vec<WireStack>) {
        let take = self.take.iter().map(|(hint, s)| (*hint, crate::inventory::stack_to_wire(s))).collect();
        let give = self.give.iter().map(crate::inventory::stack_to_wire).collect();
        (take, give)
    }

    /// From the wire: what doesn't decode, a Plan, or a count above the
    /// item's stack is left out.
    pub fn from_wire(take: &[(u8, WireStack)], give: &[WireStack], registry: &crate::block::BlockRegistry) -> Self {
        let decode = |w: &WireStack| {
            crate::inventory::stack_from_wire(w, registry, false)
                .filter(|s| !matches!(s.item, Item::Plan(_)) && s.count <= s.item.max_stack())
        };
        ItemDelta {
            take: take.iter().filter_map(|(hint, w)| decode(w).map(|s| (*hint, s))).collect(),
            give: give.iter().filter_map(decode).collect(),
        }
    }
}

use crate::protocol::WireStack;

/// C3b-fix-a (v78) — units of each item a window still owes from a
/// correction's take it couldn't pay: the item had already left the window
/// when the correction arrived (deposited back into the container, say, by an
/// op made inside the round trip, whose own correction gives it back). The
/// next correction gives of that item pay it first, so the take and the give
/// cancel whichever order the window met them in, and a take never falls on
/// an unrelated stack of the same item twice. One per joined client
/// (`window_events::WindowInbox::debt`) and one per joiner's server copy
/// (`window_events::WindowEvents::debt`), kept by the same rule.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CorrectionDebt {
    owed: Vec<(Item, u32)>,
}

impl CorrectionDebt {
    /// Units of `item` owed.
    #[cfg(test)]
    pub fn owed(&self, item: &Item) -> u32 {
        self.owed.iter().filter(|(i, _)| crate::joiner_actions::same_item(i, item)).map(|(_, n)| n).sum()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.owed.is_empty()
    }

    /// Apply `delta` to a window (`inv`, `grid`, `cursor`): every take
    /// (`joiner_actions::take_owed_window` from its hint first; what it can't
    /// pay is owed), then every give (what is owed of its item first, then
    /// `Inventory::add_item`). Returns, per give in order, how many of its
    /// units were settled — a debt paid or landed in the window; the rest
    /// didn't fit (the client reports it, `ItemAction::GrantUnfit`).
    pub fn apply(
        &mut self,
        inv: &mut Inventory,
        grid: &mut CraftGrid,
        cursor: &mut Option<ItemStack>,
        delta: &ItemDelta,
    ) -> Vec<u8> {
        for (hint, stack) in &delta.take {
            let taken = crate::joiner_actions::take_owed_window(inv, grid, cursor, usize::from(*hint), &stack.item, stack.count);
            if taken < stack.count {
                let short = u32::from(stack.count - taken);
                match self.owed.iter_mut().find(|(i, _)| i == &stack.item) {
                    Some((_, n)) => *n += short,
                    None => self.owed.push((stack.item.clone(), short)),
                }
            }
        }
        let mut settled = Vec::with_capacity(delta.give.len());
        for stack in &delta.give {
            let mut paid = 0u8;
            for (item, n) in self.owed.iter_mut() {
                if paid < stack.count && crate::joiner_actions::same_item(item, &stack.item) {
                    let pay = (*n).min(u32::from(stack.count - paid)) as u8;
                    *n -= u32::from(pay);
                    paid += pay;
                }
            }
            self.owed.retain(|(_, n)| *n > 0);
            let rest = stack.count - paid;
            let unfit = if rest > 0 {
                inv.add_item(ItemStack { item: stack.item.clone(), count: rest }).map_or(0, |r| r.count)
            } else {
                0
            };
            settled.push(stack.count - unfit);
        }
        settled
    }
}

impl ContainerData {
    /// A copy of a container's contents (a chest's, dispenser's or
    /// dropper's slots, or a furnace).
    pub fn of(c: ContainerRef) -> Self {
        match c {
            ContainerRef::Chest(d) => ContainerData::Chest(d.clone()),
            ContainerRef::Furnace(f) => ContainerData::Furnace(Box::new(f.clone())),
        }
    }

    /// As the window's container hook.
    pub fn as_mut(&mut self) -> ContainerMut<'_> {
        match self {
            ContainerData::Chest(c) => ContainerMut::Chest(c),
            ContainerData::Furnace(f) => ContainerMut::Furnace(f),
        }
    }

    pub fn as_ref(&self) -> ContainerRef<'_> {
        match self {
            ContainerData::Chest(c) => ContainerRef::Chest(c),
            ContainerData::Furnace(f) => ContainerRef::Furnace(f),
        }
    }

    /// Set slot `i` (ignored past the end).
    pub fn set(&mut self, i: usize, stack: Option<ItemStack>) {
        self.as_mut().set(i, stack);
    }
}

/// A joined client's mirror of the server's container it has open
/// (`ContainerOpened`), kept by the server's corrections and pushes
/// (`WindowSlotSet`). Its screen draws from it; its clicks are predicted on
/// it.
#[derive(Clone, Debug)]
pub struct SharedContainer {
    pub cell: [i32; 3],
    pub kind: ContainerKind,
    pub contents: ContainerData,
}

/// A mirror's contents.
#[derive(Clone, Debug)]
pub enum ContainerData {
    /// A chest of any tier, a dispenser or a dropper.
    Chest(ChestData),
    Furnace(Box<FurnaceData>),
}

impl SharedContainer {
    /// The mirror an opened `ContainerOpened` describes: its slots decoded
    /// (`inventory::stack_from_wire`, a Plan as the placeholder), a furnace's
    /// progress set. `None` for a refusal.
    pub fn from_opened(pkt: &crate::protocol::ContainerOpenedPacket, registry: &crate::block::BlockRegistry) -> Option<Self> {
        if pkt.refused.is_some() {
            return None;
        }
        let slots: Vec<Option<ItemStack>> =
            pkt.slots.iter().map(|s| s.as_ref().and_then(|w| crate::inventory::stack_from_wire(w, registry, true))).collect();
        let contents = match pkt.kind {
            ContainerKind::Furnace => {
                let mut f = FurnaceData::default();
                let mut it = slots.into_iter();
                f.input = it.next().flatten();
                f.fuel = it.next().flatten();
                f.output = it.next().flatten();
                if let Some(view) = pkt.furnace {
                    view.apply_to(&mut f);
                }
                ContainerData::Furnace(Box::new(f))
            }
            ContainerKind::Chest { tier } => ContainerData::Chest(ChestData { slots, tier }),
            ContainerKind::Dispenser | ContainerKind::Dropper => ContainerData::Chest(ChestData { slots, tier: ChestTier::Wood }),
        };
        Some(SharedContainer { cell: pkt.cell, kind: pkt.kind, contents })
    }

    /// The mirror as the window's container hook.
    pub fn as_mut(&mut self) -> ContainerMut<'_> {
        self.contents.as_mut()
    }

    #[cfg(test)]
    pub fn as_ref(&self) -> ContainerRef<'_> {
        self.contents.as_ref()
    }

    /// May its screen stay open? Only while the cell still holds a container
    /// of the same kind (`block_at_cell`) within the block reach of the
    /// player's body (eye at `eye`), by the rule the server judges the
    /// joiner's container ops with ([`container_in_reach`]).
    pub fn still_open(&self, block_at_cell: BlockId, eye: glam::Vec3) -> bool {
        ContainerKind::of_block(block_at_cell) == Some(self.kind) && container_in_reach(eye, self.cell)
    }
}

/// Is the container at `cell` within reach of a body whose eye is at `eye`?
/// The block-edit reach a joiner's server body gets
/// (`item_actions::cell_in_reach`): the client's rule to ask, and to close
/// its screen by. The server judges the same rule with a slack
/// ([`container_in_server_reach`]).
pub fn container_in_reach(eye: glam::Vec3, cell: [i32; 3]) -> bool {
    crate::item_actions::cell_in_reach(eye, cell)
}

/// The SERVER's verdict on [`container_in_reach`] for a joiner's open and
/// ops: the table's slack (`window::SERVER_TABLE_REACH_SLACK`, C3a-fix-2
/// B-L2) more, so it is a superset of an honest client's (the client's eye
/// and the server's body differ by lag and knockback) and an open or a click
/// near the edge isn't refused.
pub fn container_in_server_reach(eye: glam::Vec3, cell: [i32; 3]) -> bool {
    crate::item_actions::cell_in_reach_with(eye, cell, crate::window::SERVER_TABLE_REACH_SLACK)
}

/// What applying one `WindowSlotSet` did ([`apply_slot_set`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotSetApplied {
    /// Container slots set.
    pub sets: usize,
    /// Per give of its delta, in order: the units that didn't fit (the
    /// client reports them, `ItemAction::GrantUnfit`, and the server spawns
    /// them as a ground item).
    pub unfit: Vec<u8>,
}

/// Apply a `WindowSlotSet` to a joined client's window and its open
/// container mirror (`view.container`), as one window event, in its turn:
/// - its container slots are set to the server's values (shared state: the
///   real container's contents), and a furnace's progress is shown; a
///   container slot with no mirror open (a push that crossed the close) is
///   skipped, and so is anything but a container slot (C3b-fix-a, v78: a
///   set never names a player slot);
/// - its player part, a correction's delta (`take`, `give`), is resolved by
///   item on the window as it is now ([`CorrectionDebt::apply`]), never by
///   slot: so an op this client made while the correction was on its way
///   keeps its effect (C-H1), and a Plan its holder has is never touched.
pub fn apply_slot_set(
    view: &mut WindowMut,
    pkt: &WindowSlotSetPacket,
    registry: &crate::block::BlockRegistry,
    debt: &mut CorrectionDebt,
) -> SlotSetApplied {
    let mut out = SlotSetApplied::default();
    for (at, value) in &pkt.sets {
        let WireWindowSlot::Container(i) = *at else { continue };
        let stack = match value {
            None => None,
            Some(w) => match crate::inventory::stack_from_wire(w, registry, true) {
                Some(s) => Some(s),
                None => continue,
            },
        };
        if let Some(container) = view.container.as_mut()
            && usize::from(i) < container.as_ref().len()
        {
            container.set(usize::from(i), stack);
            out.sets += 1;
        }
    }
    if let (Some(progress), Some(ContainerMut::Furnace(f))) = (pkt.furnace, view.container.as_mut()) {
        progress.apply_to(f);
    }
    let delta = ItemDelta::from_wire(&pkt.take, &pkt.give, registry);
    if !delta.is_empty() {
        let settled = debt.apply(view.inv, view.grid, view.cursor, &delta);
        out.unfit = delta.give.iter().zip(settled).map(|(g, s)| g.count - s).collect();
    }
    out
}

// ── Server: the real container at a cell ────────────────────────────────

/// The real container of `kind` at `cell` in `world`, if it is still there
/// (the block is that kind and its block entity exists).
pub fn container_at(world: &World, cell: [i32; 3], kind: ContainerKind) -> Option<ContainerRef<'_>> {
    let pos = (cell[0], cell[1], cell[2]);
    if ContainerKind::of_block(world.get_block(cell[0], cell[1], cell[2])) != Some(kind) {
        return None;
    }
    match kind {
        ContainerKind::Chest { .. } => world.chest_at(pos).map(ContainerRef::Chest),
        ContainerKind::Dispenser | ContainerKind::Dropper => world.dispenser_at(pos).map(|d| ContainerRef::Chest(&d.chest)),
        ContainerKind::Furnace => world.furnace_at(pos).map(ContainerRef::Furnace),
    }
}

/// [`container_at`], mutably (for a click).
pub fn container_at_mut(world: &mut World, cell: [i32; 3], kind: ContainerKind) -> Option<ContainerMut<'_>> {
    let pos = (cell[0], cell[1], cell[2]);
    if ContainerKind::of_block(world.get_block(cell[0], cell[1], cell[2])) != Some(kind) {
        return None;
    }
    match kind {
        ContainerKind::Chest { .. } => world.chest_at_mut(pos).map(ContainerMut::Chest),
        ContainerKind::Dispenser | ContainerKind::Dropper => world.dispenser_at_mut(pos).map(|d| ContainerMut::Chest(&mut d.chest)),
        ContainerKind::Furnace => world.furnace_at_mut(pos).map(ContainerMut::Furnace),
    }
}

/// Create the block entity of `kind` at `cell` if it is missing, as a
/// client's open always has (a chest sized to its tier, a fresh dispenser,
/// an empty furnace).
pub fn ensure_container(world: &mut World, cell: [i32; 3], kind: ContainerKind) {
    let pos = (cell[0], cell[1], cell[2]);
    match kind {
        ContainerKind::Chest { tier } => {
            if world.chest_at(pos).is_none() {
                world.insert_chest(pos, ChestData::for_tier(tier));
            }
        }
        ContainerKind::Dispenser | ContainerKind::Dropper => {
            if world.dispenser_at(pos).is_none() {
                world.insert_dispenser(pos, crate::dispenser::DispenserData::new());
            }
        }
        ContainerKind::Furnace => {
            if world.furnace_at(pos).is_none() {
                world.insert_furnace(pos, FurnaceData::default());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Every `ContainerClick` on plain values: no egui, no GPU.
    use super::*;
    use crate::armour::ArmourItem;
    use crate::inventory::Inventory;
    use crate::item::MaterialId;
    use crate::window::{CraftGrid, Station};

    /// Owns the state a `WindowMut` with a container borrows.
    struct Win {
        inv: Inventory,
        armour: [Option<ArmourItem>; 4],
        cursor: Option<ItemStack>,
        grid: CraftGrid,
        chest: ChestData,
        furnace: FurnaceData,
    }

    impl Win {
        fn new() -> Self {
            Win {
                inv: Inventory::new(),
                armour: [None; 4],
                cursor: None,
                grid: Default::default(),
                chest: ChestData::new(),
                furnace: FurnaceData::default(),
            }
        }

        fn chest(&mut self, click: ContainerClick, shared: bool) -> ContainerApplied {
            let mut view = WindowMut {
                inv: &mut self.inv,
                armour: &mut self.armour,
                cursor: &mut self.cursor,
                grid: &mut self.grid,
                container: Some(ContainerMut::Chest(&mut self.chest)),
            };
            apply_container(&mut view, &click, &ctx(shared))
        }

        fn furnace(&mut self, click: ContainerClick) -> ContainerApplied {
            let mut view = WindowMut {
                inv: &mut self.inv,
                armour: &mut self.armour,
                cursor: &mut self.cursor,
                grid: &mut self.grid,
                container: Some(ContainerMut::Furnace(&mut self.furnace)),
            };
            apply_container(&mut view, &click, &ctx(true))
        }

        fn none(&mut self, click: ContainerClick) -> ContainerApplied {
            let mut view = WindowMut {
                inv: &mut self.inv,
                armour: &mut self.armour,
                cursor: &mut self.cursor,
                grid: &mut self.grid,
                container: None,
            };
            apply_container(&mut view, &click, &ctx(true))
        }
    }

    fn ctx(shared: bool) -> ClickCtx {
        ClickCtx::new(false, Station::Player, glam::Vec3::ZERO, |_| block::AIR).with_shared(shared)
    }

    fn stone(n: u8) -> ItemStack {
        ItemStack::new_block(block::STONE, n)
    }

    fn plan() -> ItemStack {
        ItemStack { item: Item::Plan(crate::plan::PlanData::debug_3x3_stone()), count: 1 }
    }

    #[test]
    fn withdraw_one_or_the_stack_and_report_the_slots_it_touched() {
        let mut w = Win::new();
        w.chest.slots[4] = Some(stone(5));
        let one = w.chest(ContainerClick::Withdraw { slot: 4, all: false }, true);
        assert_eq!(one.result, ClickResult::Done);
        assert_eq!(one.touched, vec![WireWindowSlot::Inv(0), WireWindowSlot::Container(4)]);
        assert_eq!(w.chest.slots[4], Some(stone(4)));
        assert_eq!(w.inv.slot(0), Some(&stone(1)));
        let all = w.chest(ContainerClick::Withdraw { slot: 4, all: true }, true);
        assert_eq!(all.result, ClickResult::Done);
        assert_eq!(w.chest.slots[4], None);
        assert_eq!(w.inv.slot(0), Some(&stone(5)));
        let empty = w.chest(ContainerClick::Withdraw { slot: 4, all: true }, true);
        assert_eq!((empty.result, empty.touched), (ClickResult::Refused, vec![]), "an empty slot gives nothing");
        assert_eq!(w.chest(ContainerClick::Withdraw { slot: 99, all: true }, true).result, ClickResult::Refused);
    }

    #[test]
    fn deposit_one_or_the_stack_and_refuse_a_slot_past_the_inventory() {
        let mut w = Win::new();
        w.inv.set_slot(7, Some(stone(3)));
        let one = w.chest(ContainerClick::Deposit { slot: 7, all: false }, true);
        assert_eq!(one.result, ClickResult::Done);
        assert_eq!(one.touched, vec![WireWindowSlot::Inv(7), WireWindowSlot::Container(0)]);
        assert_eq!(w.chest(ContainerClick::Deposit { slot: 7, all: true }, true).result, ClickResult::Done);
        assert_eq!(w.chest.slots[0], Some(stone(3)));
        assert_eq!(w.inv.slot(7), None);
        assert_eq!(w.chest(ContainerClick::Deposit { slot: 36, all: true }, true).result, ClickResult::Refused);
    }

    #[test]
    fn the_four_buttons_call_the_chest_rules() {
        let mut w = Win::new();
        w.chest.slots[0] = Some(stone(30));
        w.chest.slots[10] = Some(stone(30));
        w.chest.slots[11] = Some(ItemStack::new_block(block::DIRT, 4));
        assert_eq!(w.chest(ContainerClick::Sort, true).result, ClickResult::Done);
        assert_eq!(w.chest.slots[0], Some(stone(60)), "merged to the front");
        w.inv.set_slot(0, Some(stone(2)));
        w.inv.set_slot(1, Some(ItemStack::new_material(MaterialId::Stick, 3)));
        assert_eq!(w.chest(ContainerClick::DumpMatching, true).result, ClickResult::Done);
        assert_eq!(w.inv.slot(0), None, "the stone went in");
        assert!(w.inv.slot(1).is_some(), "the sticks didn't: the chest holds none");
        assert_eq!(w.chest(ContainerClick::DumpMatching, true).result, ClickResult::Refused, "nothing left to dump");
        w.inv.set_slot(2, Some(ItemStack::new_block(block::DIRT, 1)));
        assert_eq!(w.chest(ContainerClick::Restock, true).result, ClickResult::Done);
        assert_eq!(w.inv.slot(2), Some(&ItemStack::new_block(block::DIRT, 5)), "dirt restocked");
        assert_eq!(w.chest(ContainerClick::TakeAll, true).result, ClickResult::Done);
        assert!(w.chest.slots.iter().all(Option::is_none), "everything taken");
        assert_eq!(w.chest(ContainerClick::TakeAll, true).result, ClickResult::Refused);
    }

    #[test]
    fn a_furnace_click_feeds_from_the_held_slot_and_takes_the_output() {
        let mut w = Win::new();
        w.inv.set_slot(2, Some(ItemStack::new_material(MaterialId::RawIron, 4)));
        w.inv.set_slot(3, Some(ItemStack::new_material(MaterialId::Coal, 2)));
        let input = w.furnace(ContainerClick::Furnace { kind: SlotKind::Input, mode: ClickMode::Stack, hotbar: 2 });
        assert_eq!(input.result, ClickResult::Done);
        assert_eq!(input.touched, vec![WireWindowSlot::Inv(2), WireWindowSlot::Container(FURNACE_INPUT as u8)]);
        let fuel = w.furnace(ContainerClick::Furnace { kind: SlotKind::Fuel, mode: ClickMode::Single, hotbar: 3 });
        assert_eq!(fuel.result, ClickResult::Done);
        assert_eq!(w.furnace.fuel, Some(ItemStack::new_material(MaterialId::Coal, 1)));
        w.furnace.output = Some(ItemStack::new_material(MaterialId::IronIngot, 2));
        let out = w.furnace(ContainerClick::Furnace { kind: SlotKind::Output, mode: ClickMode::Stack, hotbar: 0 });
        assert_eq!(out.result, ClickResult::Done);
        assert!(out.touched.contains(&WireWindowSlot::Container(FURNACE_OUTPUT as u8)));
        assert_eq!(w.furnace.output, None);
        let nothing = w.furnace(ContainerClick::Furnace { kind: SlotKind::Output, mode: ClickMode::Stack, hotbar: 0 });
        assert_eq!(nothing.result, ClickResult::Refused);
        let past = w.furnace(ContainerClick::Furnace { kind: SlotKind::Input, mode: ClickMode::Stack, hotbar: 9 });
        assert_eq!(past.result, ClickResult::Refused, "a hotbar slot past 8 is refused");
    }

    #[test]
    fn a_click_for_the_other_kind_or_with_nothing_open_is_refused() {
        let mut w = Win::new();
        w.chest.slots[0] = Some(stone(1));
        w.inv.set_slot(0, Some(ItemStack::new_material(MaterialId::RawIron, 1)));
        assert_eq!(w.furnace(ContainerClick::Withdraw { slot: 0, all: true }).result, ClickResult::Refused);
        assert_eq!(
            w.chest(ContainerClick::Furnace { kind: SlotKind::Input, mode: ClickMode::Stack, hotbar: 0 }, true).result,
            ClickResult::Refused
        );
        assert_eq!(w.none(ContainerClick::TakeAll).result, ClickResult::Refused);
        assert_eq!(w.chest.slots[0], Some(stone(1)), "nothing moved");
    }

    #[test]
    fn a_plan_neither_goes_into_nor_comes_out_of_a_shared_container() {
        let mut w = Win::new();
        w.inv.set_slot(0, Some(plan()));
        let put = w.chest(ContainerClick::Deposit { slot: 0, all: true }, true);
        assert_eq!((put.result, put.touched), (ClickResult::PlanStays, vec![]));
        assert!(!put_ok(&w), "refused");
        // A host's Plan already in it can't be taken, nor swept by Take all.
        w.chest.slots[3] = Some(plan());
        w.chest.slots[4] = Some(stone(2));
        assert_eq!(w.chest(ContainerClick::Withdraw { slot: 3, all: true }, true).result, ClickResult::PlanStays);
        assert_eq!(w.chest(ContainerClick::TakeAll, true).result, ClickResult::Done);
        assert!(is_plan(w.chest.slots[3].as_ref()), "the Plan stays");
        assert_eq!(w.chest.slots[4], None, "the stone came out");
        // Single-player and a host move Plans as they always have.
        assert_eq!(w.chest(ContainerClick::Withdraw { slot: 3, all: true }, false).result, ClickResult::Done);
        assert_eq!(w.chest(ContainerClick::Deposit { slot: 0, all: true }, false).result, ClickResult::Done);
        assert!(!ClickResult::PlanStays.ok());
    }

    fn put_ok(w: &Win) -> bool {
        w.chest.slots.iter().any(|s| is_plan(s.as_ref()))
    }

    /// C3b-fix-a (v78) — a slot set overwrites exactly the named CONTAINER
    /// slots (a player slot named in it is skipped: a set never names one),
    /// and its correction delta moves items by item, resolved on the window
    /// as it is: a take from its hint first, then wherever the item is; a
    /// give by `add_item`. A Plan the client holds is never touched.
    #[test]
    fn a_slot_set_sets_container_slots_and_moves_items_by_item() {
        let registry = crate::block::BlockRegistry::new();
        let mut w = Win::new();
        w.inv.set_slot(1, Some(stone(9)));
        w.inv.set_slot(2, Some(stone(3)));
        w.chest.slots[5] = Some(stone(1));
        w.inv.set_slot(8, Some(plan()));
        let wire = |s: &ItemStack| crate::inventory::stack_to_wire(s);
        let pkt = WindowSlotSetPacket {
            op_seq_applied: 4,
            reason: crate::protocol::slot_set_reason::CORRECTION,
            sets: vec![
                (WireWindowSlot::Inv(1), None),
                (WireWindowSlot::Container(5), Some(wire(&stone(7)))),
                (WireWindowSlot::Cursor, Some(wire(&ItemStack::new_block(block::DIRT, 2)))),
                (WireWindowSlot::Container(200), None),
                (WireWindowSlot::Inv(8), None),
            ],
            furnace: None,
            window_event: 1,
            // Take 4 stone from slot 2 first (it holds 3: the last one comes
            // from slot 1), give 2 dirt.
            take: vec![(2, wire(&stone(4)))],
            give: vec![wire(&ItemStack::new_block(block::DIRT, 2))],
        };
        let mut debt = CorrectionDebt::default();
        let mut view = WindowMut {
            inv: &mut w.inv,
            armour: &mut w.armour,
            cursor: &mut w.cursor,
            grid: &mut w.grid,
            container: Some(ContainerMut::Chest(&mut w.chest)),
        };
        let applied = apply_slot_set(&mut view, &pkt, &registry, &mut debt);
        assert_eq!(applied, SlotSetApplied { sets: 1, unfit: vec![0] }, "one container slot; the dirt fit");
        assert_eq!(w.chest.slots[5], Some(stone(7)));
        assert_eq!(w.inv.slot(2), None, "the take looked in its hint first");
        assert_eq!(w.inv.slot(1), Some(&stone(8)), "then wherever stone is");
        assert_eq!(w.cursor, None, "a player slot named in a set is skipped");
        assert_eq!(w.inv.slot(0), Some(&ItemStack::new_block(block::DIRT, 2)), "the give landed by add_item");
        assert!(is_plan(w.inv.slot(8)), "the held Plan survives");
        assert!(debt.is_empty());
    }

    /// C3b-fix-a (C-H1) — a correction's take that finds nothing (the item
    /// left the window before it arrived: deposited back, say) is owed, and
    /// the next give of that item pays it first, whichever order the window
    /// met them in; a give past the debt lands, and what doesn't fit is
    /// reported unfit.
    #[test]
    fn a_take_that_finds_nothing_is_owed_and_paid_by_the_next_give() {
        let mut inv = Inventory::new();
        let (mut grid, mut cursor): (CraftGrid, Option<ItemStack>) = (Default::default(), None);
        let mut debt = CorrectionDebt::default();
        inv.set_slot(9, Some(stone(10)));
        // Take 16: 10 found, 6 owed.
        let take = ItemDelta { take: vec![(0, stone(16))], give: vec![] };
        assert!(debt.apply(&mut inv, &mut grid, &mut cursor, &take).is_empty());
        assert_eq!((inv.slot(9), debt.owed(&Item::Block(block::STONE))), (None, 6));
        // Give 16: 6 pay the debt, 10 land.
        let give = ItemDelta { take: vec![], give: vec![stone(16)] };
        assert_eq!(debt.apply(&mut inv, &mut grid, &mut cursor, &give), vec![16], "all 16 settled");
        assert_eq!((inv.slot(0), debt.is_empty()), (Some(&stone(10)), true));
        // A full window: the give's rest doesn't fit.
        for i in 0..SLOTS {
            inv.set_slot(i, Some(ItemStack::new_block(block::DIRT, 64)));
        }
        let give = ItemDelta { take: vec![], give: vec![stone(5)] };
        assert_eq!(debt.apply(&mut inv, &mut grid, &mut cursor, &give), vec![0], "nothing settled: 5 unfit");
    }

    /// C3b-fix-a — a delta comes from net counts by item, split into stacks
    /// of at most the item's `max_stack()` (a tool one by one); a Plan never.
    #[test]
    fn a_delta_splits_counts_into_stacks() {
        let pick = ItemStack::new_tool(crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Iron));
        let net: ItemCounts = vec![(Item::Block(block::STONE), 100), (pick.item.clone(), -2), (plan().item, 1)];
        let d = ItemDelta::from_counts(&net, |_| 7);
        assert_eq!(d.give, vec![stone(64), stone(36)]);
        assert_eq!(d.take, vec![(7, pick.clone()), (7, pick.clone())]);
        let back = d.counts();
        assert_eq!(back, vec![(Item::Block(block::STONE), 100), (pick.item.clone(), -2)], "the Plan is left out");
        let registry = crate::block::BlockRegistry::new();
        let (take, give) = d.to_wire();
        assert_eq!(ItemDelta::from_wire(&take, &give, &registry), d, "round trip");
        // What the container lost is what the player gained.
        let (mut before, mut after) = (ChestData::new(), ChestData::new());
        before.slots[0] = Some(stone(20));
        after.slots[3] = Some(stone(5));
        after.slots[4] = Some(ItemStack::new_block(block::DIRT, 2));
        assert_eq!(
            player_gain(ContainerRef::Chest(&before), ContainerRef::Chest(&after)),
            {
                let mut v = vec![(Item::Block(block::STONE), 15), (Item::Block(block::DIRT), -2)];
                v.sort_by_key(|(item, _)| item_key(item));
                v
            }
        );
    }

    /// C3b-fix-a — the phantom ledger debits claimed slots only (never a
    /// blocker, even one that would match), in the owed search's order.
    #[test]
    fn a_debit_comes_off_claimed_slots_only() {
        let registry = crate::block::BlockRegistry::new();
        let wire = |s: &ItemStack| Some(crate::inventory::stack_to_wire(s));
        let claims = vec![
            (WireWindowSlot::Cursor, wire(&stone(4))),
            (WireWindowSlot::Inv(5), wire(&stone(10))),
            (WireWindowSlot::Inv(2), wire(&stone(3))),
        ];
        let mut w = ClaimedWindow::from_claims(&claims, &registry);
        assert_eq!(w.debit(&Item::Block(block::STONE), 15), 15);
        assert_eq!(w.stack(WireWindowSlot::Inv(5)), Some(None), "the 36 slots first, in claim order");
        assert_eq!(w.stack(WireWindowSlot::Inv(2)), Some(None));
        assert_eq!(w.stack(WireWindowSlot::Cursor), Some(Some(stone(2))), "then the cursor");
        assert_eq!(w.debit(&Item::Block(block::STONE), 10), 2, "no more claimed");
        assert_eq!(w.debit(&blocker().item, 1), 0, "a blocker is never debited");
        assert_eq!(w.slot_holding(&Item::Block(block::STONE)), None);
    }

    /// C3b-fix-a (C-L3) — a claim above its item's stack is not one an
    /// honest client can make: 65 stone, or two of one tool.
    #[test]
    fn claims_above_a_stack_are_rejected() {
        let registry = crate::block::BlockRegistry::new();
        let wire = |item: Item, count: u8| Some(crate::inventory::stack_to_wire(&ItemStack { item, count }));
        let pick = Item::Tool(crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Iron));
        assert!(claims_fit_stacks(&[(WireWindowSlot::Inv(0), wire(Item::Block(block::STONE), 64))], &registry));
        assert!(!claims_fit_stacks(&[(WireWindowSlot::Inv(0), wire(Item::Block(block::STONE), 65))], &registry));
        assert!(claims_fit_stacks(&[(WireWindowSlot::Inv(1), wire(pick.clone(), 1))], &registry));
        assert!(!claims_fit_stacks(&[(WireWindowSlot::Inv(1), wire(pick, 2))], &registry), "a tool counted above one");
        assert!(claims_fit_stacks(&[(WireWindowSlot::Inv(2), None)], &registry));
    }

    #[test]
    fn a_mirror_holds_a_plan_as_a_placeholder_that_digests_like_the_real_one() {
        let registry = crate::block::BlockRegistry::new();
        let mut real = ChestData::new();
        real.slots[2] = Some(plan());
        real.slots[3] = Some(stone(4));
        let pkt = crate::protocol::ContainerOpenedPacket {
            cell: [1, 2, 3],
            kind: ContainerKind::Chest { tier: ChestTier::Wood },
            slots: ContainerRef::Chest(&real).wire_slots(),
            furnace: None,
            refused: None,
            window_event: 1,
        };
        let mirror = SharedContainer::from_opened(&pkt, &registry).expect("opened");
        let ContainerData::Chest(m) = &mirror.contents else { panic!("a chest") };
        assert!(is_plan(m.slots[2].as_ref()), "a placeholder Plan");
        assert_eq!(m.slots[3], Some(stone(4)));
        assert_eq!(ContainerRef::Chest(m).prints(), ContainerRef::Chest(&real).prints(), "the same prints");
    }

    /// v77 — an op claims the player slots it changed, the ones the click
    /// names, and all 36 for Restock; the server's re-run over the claims
    /// lands items only in claimed slots (every other slot is a blocker).
    #[test]
    fn claims_cover_what_the_rule_reads_and_the_rerun_lands_only_in_them() {
        let refused_deposit = claim_slots(&ContainerClick::Deposit { slot: 7, all: true }, &[]);
        assert_eq!(refused_deposit, vec![WireWindowSlot::Inv(7)], "a deposit's source even when nothing moved");
        let feed = ContainerClick::Furnace { kind: SlotKind::Fuel, mode: ClickMode::Single, hotbar: 3 };
        assert_eq!(claim_slots(&feed, &[WireWindowSlot::Container(1)]), vec![WireWindowSlot::Inv(3)], "the held slot, never a container slot");
        assert_eq!(claim_slots(&ContainerClick::Restock, &[]).len(), SLOTS, "Restock reads every slot");

        // The client held 60 stone in slot 2 and withdrew 10: 4 merged, 6 into
        // slot 5 (its first empty). Claimed: slots 2 and 5. The real chest now
        // holds 20 there: the re-run puts 4 in slot 2, 6 in slot 5, and the
        // other 10 stay in the chest — never in an unclaimed slot.
        let registry = crate::block::BlockRegistry::new();
        let claims = vec![
            (WireWindowSlot::Inv(2), Some(crate::inventory::stack_to_wire(&stone(60)))),
            (WireWindowSlot::Inv(5), None),
        ];
        let mut w = ClaimedWindow::from_claims(&claims, &registry);
        let mut real = ChestData::new();
        real.slots[0] = Some(stone(20));
        let applied = w.apply(ContainerMut::Chest(&mut real), &ContainerClick::Withdraw { slot: 0, all: true }, &ctx(true));
        assert_eq!(applied.result, ClickResult::Done);
        assert_eq!(w.stack(WireWindowSlot::Inv(2)), Some(Some(stone(64))));
        assert_eq!(w.stack(WireWindowSlot::Inv(5)), Some(Some(stone(16))), "a claimed empty slot takes up to a stack");
        assert_eq!(w.stack(WireWindowSlot::Inv(0)), None, "an unclaimed slot isn't the client's to say");
        assert!(applied.touched.iter().all(|at| matches!(at, WireWindowSlot::Inv(2) | WireWindowSlot::Inv(5) | WireWindowSlot::Container(0))));
        assert_eq!(real.slots[0], None, "it all fit in the claimed slots");
        // With only slot 2 claimed, what doesn't fit stays in the chest.
        let mut w = ClaimedWindow::from_claims(&claims[..1], &registry);
        let mut real = ChestData::new();
        real.slots[0] = Some(stone(20));
        w.apply(ContainerMut::Chest(&mut real), &ContainerClick::Withdraw { slot: 0, all: true }, &ctx(true));
        assert_eq!(real.slots[0], Some(stone(16)), "the rest stays: no unclaimed slot takes it");
        // A Plan claimed stays put and is never a correction's value.
        let w = ClaimedWindow::from_claims(&[(WireWindowSlot::Inv(1), Some(crate::inventory::stack_to_wire(&plan())))], &registry);
        assert!(matches!(w.stack(WireWindowSlot::Inv(1)), Some(Some(ItemStack { item: Item::Plan(_), .. }))));
        // C3b-fix-a — and never a correction's to move: a delta leaves a Plan out.
        let net: ItemCounts = vec![(plan().item, 1)];
        assert!(ItemDelta::from_counts(&net, |_| 0).is_empty(), "a Plan is never moved by a correction");
    }

    /// C3a-fix-2's server slack for containers: just past the client's
    /// reach the server still opens and serves (its body lags the client's
    /// eye); half a block further it doesn't.
    #[test]
    fn the_server_judges_container_reach_with_a_slack() {
        let cell = [0, 64, 0];
        let mut eye = glam::Vec3::new(0.5, 64.5, 0.5);
        while container_in_reach(eye, cell) {
            eye.z += 0.05;
        }
        assert!(container_in_server_reach(eye, cell), "just past the client's reach: the server allows it");
        eye.z += crate::window::SERVER_TABLE_REACH_SLACK + 0.1;
        assert!(!container_in_server_reach(eye, cell), "beyond the slack: refused");
    }

    #[test]
    fn container_kinds_come_from_the_block() {
        assert_eq!(ContainerKind::of_block(block::CHEST), Some(ContainerKind::Chest { tier: ChestTier::Wood }));
        assert_eq!(ContainerKind::of_block(block::SATORI_CHEST), Some(ContainerKind::Chest { tier: ChestTier::Satori }));
        assert_eq!(ContainerKind::of_block(block::DISPENSER), Some(ContainerKind::Dispenser));
        assert_eq!(ContainerKind::of_block(block::DROPPER), Some(ContainerKind::Dropper));
        assert_eq!(ContainerKind::of_block(block::FURNACE), Some(ContainerKind::Furnace));
        assert_eq!(ContainerKind::of_block(block::FURNACE_LIT), Some(ContainerKind::Furnace));
        assert_eq!(ContainerKind::of_block(block::STONE), None);
        assert_eq!(ChestTier::Satori.slots(), crate::protocol::MAX_CONTAINER_SLOTS, "the largest tier bounds the wire");
    }
}
