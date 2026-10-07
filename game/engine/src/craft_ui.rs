//! Crafting UI — inventory screen with 2x2 player crafting or 3x3 table crafting.
//!
//! Game logic (grid manipulation, recipe matching, click handling) is preserved.
//! Rendering uses egui for modern, clean UI with block textures as managed textures.

use crate::armour::{self, ArmourItem, ArmourSlot as ArmSlot};
use crate::block::BlockRegistry;
use crate::crafting::CraftSlot;
use crate::egui_integration::EguiIntegration;
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};
use crate::window::{self, ClickCtx, ClickResult, Station, WindowClick, WindowMut, WindowSlot};

/// Theme colours for crafting UI.
const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 210);
const SLOT_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(24, 24, 32, 255);
const SLOT_BORDER: egui::Color32 = egui::Color32::from_rgb(58, 58, 74);
const RESULT_BORDER: egui::Color32 = egui::Color32::from_rgb(100, 200, 80);
/// Controller slot-cursor highlight — warm gold, 2px, unmistakable at 48px.
const PAD_FOCUS_BORDER: egui::Color32 = egui::Color32::from_rgb(255, 210, 80);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 200, 120);
const LABEL_COLOR: egui::Color32 = egui::Color32::from_rgb(130, 130, 130);

/// The crafting UI state.
pub struct CraftingUi {
    pub open: bool,
    pub is_table: bool,
    /// C2b — the crafting table the 3×3 grid was opened from (`None` for
    /// the 2×2 player grid). A joined client names it in its
    /// `ItemAction::Craft`, so the server can check the table is there and
    /// in reach for a recipe bigger than 2×2.
    pub table: Option<[i32; 3]>,
    pub grid: [[Option<ItemStack>; 3]; 3],
    pub result: Option<ItemStack>,
    pub cursor_item: Option<ItemStack>,
    /// #45 P3 — Mouse-Tweaks paint gesture. `Some(true)` = RMB-drag-distribute
    /// (deposit one carried item per slot); `Some(false)` = LMB-drag-gather
    /// (pull matching items into the cursor). `None` = no drag in progress.
    pub drag_paint: Option<bool>,
    /// Slots already painted in the current drag gesture (so each is acted on
    /// exactly once). Keyed `0..36` for inventory, `100 + r*3 + c` for the grid.
    pub drag_visited: ahash::AHashSet<u32>,
    pub mouse_ndc: [f32; 2],
    /// Controller slot-cursor (gamepad UI navigation, 2026-06-12). `None`
    /// until the first d-pad press while the panel is open — mouse users
    /// never see a highlight. Holds which slot the pad cursor is on.
    pub pad_focus: Option<PadSlot>,
    /// Recipe book (2026-06-12). When `book_open`, the book panel replaces
    /// the grid view; selecting a card pins it as a placement guide (see
    /// `pinned_recipe`) and closes the book.
    pub book_open: bool,
    /// Active category tab — index into `RecipeCategory::ALL`.
    pub book_category: usize,
    /// Controller cursor over the visible card list (index into the filtered
    /// list shown for the active category).
    pub book_focus: usize,
    /// Recipe-book search query (2026-06-13). When non-empty, the book shows
    /// name-matching recipes across ALL categories instead of the active tab.
    pub book_search: String,
    /// Set the frame the book opens so the search field grabs keyboard focus.
    pub focus_book_search: bool,
    /// Placement-guide navigation stack (2026-06-12). Global catalogue indices;
    /// the LAST entry is the recipe currently shown as a "how to place it"
    /// mini-grid card right of the result slot. Empty = no guide. Clicking a
    /// craftable ingredient pushes its recipe (drill down); "← Back" / the
    /// output icon pops (drill up). Replaces the old single `pinned_recipe`.
    pub pinned_recipe_stack: Vec<usize>,
    /// #46 — JEI "show uses" filter. When `Some`, the book lists exactly these
    /// catalogue indices ("what can I make with X"), overriding the category tab;
    /// search still narrows within them. Cleared by Close and by switching tabs.
    pub book_uses_filter: Option<Vec<usize>>,
}

impl CraftingUi {
    /// The recipe currently shown in the placement guide, if any.
    pub fn pinned_recipe(&self) -> Option<usize> {
        self.pinned_recipe_stack.last().copied()
    }

    /// Open the recipe book fresh: empty search (auto-focused), first tab.
    pub fn open_book(&mut self) {
        self.book_open = true;
        self.book_focus = 0;
        self.book_search.clear();
        self.focus_book_search = true;
        self.book_uses_filter = None;
    }

    /// #46 — open the book filtered to "what can I make with this item" (JEI U /
    /// right-click "uses"). Empty `indices` still opens the book (shows nothing
    /// used it, which is itself useful information).
    pub fn open_book_uses(&mut self, indices: Vec<usize>) {
        self.book_open = true;
        self.book_focus = 0;
        self.book_search.clear();
        self.focus_book_search = false;
        self.book_uses_filter = Some(indices);
    }
}

impl CraftingUi {
    pub fn new() -> Self {
        Self {
            open: false,
            is_table: false,
            table: None,
            grid: [[None, None, None], [None, None, None], [None, None, None]],
            result: None,
            cursor_item: None,
            drag_paint: None,
            drag_visited: ahash::AHashSet::new(),
            mouse_ndc: [0.0, 0.0],
            pad_focus: None,
            book_open: false,
            book_category: 0,
            book_focus: 0,
            book_search: String::new(),
            focus_book_search: false,
            pinned_recipe_stack: Vec::new(),
            book_uses_filter: None,
        }
    }

    pub fn open_player_crafting(&mut self) {
        self.open = true;
        self.is_table = false;
        self.table = None;
        self.grid = [[None, None, None], [None, None, None], [None, None, None]];
        self.result = None;
        self.pad_focus = None;
        self.book_open = false;
        self.book_search.clear();
        self.pinned_recipe_stack.clear();
        self.book_uses_filter = None;
    }

    /// Open the 3×3 grid of the crafting table at `table` (C2b: recorded
    /// for a joiner's `ItemAction::Craft`).
    pub fn open_table_crafting(&mut self, table: [i32; 3]) {
        self.open = true;
        self.is_table = true;
        self.table = Some(table);
        self.grid = [[None, None, None], [None, None, None], [None, None, None]];
        self.result = None;
        self.pad_focus = None;
        self.book_open = false;
        self.book_search.clear();
        self.pinned_recipe_stack.clear();
        self.book_uses_filter = None;
    }

    /// The grid on screen: the player's 2×2 or a table's 3×3.
    pub fn station(&self) -> Station {
        if self.is_table { Station::Table } else { Station::Player }
    }

    /// C3a-1 — apply one window click: the pure rule (`window::apply`) over
    /// this screen's grid and cursor plus the player's `inv` and `armour`,
    /// then refresh the result shown. Every item move the screen makes goes
    /// through here.
    pub fn apply_click(
        &mut self,
        inv: &mut Inventory,
        armour: &mut [Option<ArmourItem>; 4],
        click: &WindowClick,
        creative: bool,
    ) -> ClickResult {
        let ctx = ClickCtx { creative, station: self.station(), craft: self.result.clone() };
        let mut view = WindowMut { inv, armour, cursor: &mut self.cursor_item, grid: &mut self.grid, container: None };
        let out = window::apply(&mut view, click, &ctx);
        // A close that couldn't return everything has always left the
        // result as it was.
        if *click != WindowClick::Close {
            self.update_result();
        }
        out
    }

    /// `apply_click` for a click that never touches the armour slots.
    fn apply_bag_click(&mut self, inv: &mut Inventory, click: &WindowClick) -> ClickResult {
        let mut no_armour = [None; 4];
        self.apply_click(inv, &mut no_armour, click, false)
    }

    /// Return all grid + cursor items to the inventory and close the panel
    /// (`WindowClick::Close`).
    ///
    /// `add_item` is non-atomic, so if the inventory can't hold everything the
    /// un-returnable items STAY in the grid/cursor and the panel stays **open**
    /// — nothing is ever deleted (engine audit 2026-06-04, A: `CraftingUi::close`
    /// silently dropped grid/cursor items on a full inventory). Returns `true`
    /// if the panel actually closed; callers can surface a "make room" hint on
    /// `false`.
    pub fn close(&mut self, inventory: &mut Inventory) -> bool {
        let all_placed = self.apply_bag_click(inventory, &WindowClick::Close).ok();
        if all_placed {
            self.open = false;
            self.result = None;
            self.pad_focus = None;
            self.book_open = false;
            self.book_search.clear();
            self.pinned_recipe_stack.clear();
        }
        all_placed
    }

    /// Recipe-book auto-fill (`WindowClick::Autofill`). Returns `false` when
    /// the inventory is short (the caller toasts "not enough materials").
    #[cfg(test)]
    pub fn autofill_from_example(&mut self, example: &[[CraftSlot; 3]; 3], inv: &mut Inventory) -> bool {
        self.apply_bag_click(inv, &WindowClick::Autofill { example: *example }).ok()
    }

    /// Recompute the result shown from the grid (`window::recipe_output`).
    pub fn update_result(&mut self) {
        self.result = window::recipe_output(&self.grid);
    }

    /// Click a crafting-grid cell (`WindowClick::Grid`).
    #[cfg(test)]
    pub fn click_grid_slot(&mut self, row: usize, col: usize, inventory: &mut Inventory, right: bool) -> bool {
        self.apply_bag_click(inventory, &WindowClick::Grid { row, col, right }).ok()
    }

    /// Click the result slot (`WindowClick::Result`).
    #[cfg(test)]
    pub fn click_result(&mut self, inventory: &mut Inventory) -> bool {
        self.apply_bag_click(inventory, &WindowClick::Result).ok()
    }

    /// Spec 28e — click an armour slot (`WindowClick::Armour`);
    /// `armour_slot_idx` is `ArmourSlot as usize` (0..=3).
    #[cfg(test)]
    pub fn click_armour_slot(&mut self, armour_slot_idx: usize, armour_slots: &mut [Option<ArmourItem>; 4]) -> bool {
        let mut unused = Inventory::new();
        self.apply_click(&mut unused, armour_slots, &WindowClick::Armour { slot: armour_slot_idx }, false).ok()
    }

    /// Click an inventory slot (`WindowClick::Slot`).
    #[cfg(test)]
    pub fn click_inventory_slot(&mut self, slot: usize, inventory: &mut Inventory, right: bool) -> bool {
        self.apply_bag_click(inventory, &WindowClick::Slot { slot, right }).ok()
    }

    /// End any in-progress drag gesture (call on panel close / no button down).
    pub fn clear_drag(&mut self) {
        self.drag_paint = None;
        self.drag_visited.clear();
    }

    // ── #45 P3 — Mouse-Tweaks drag paint (`WindowClick::Drag*`) ─────────────
    // The draw layer paints each slot once per gesture (`drag_visited`) and
    // sends one slot per paint.

    #[cfg(test)]
    pub fn drag_distribute_into_inventory(&mut self, slot: usize, inventory: &mut Inventory) -> bool {
        self.apply_bag_click(inventory, &WindowClick::DragDistribute { slots: vec![WindowSlot::Inv(slot)] }).ok()
    }

    #[cfg(test)]
    pub fn drag_gather_from_inventory(&mut self, slot: usize, inventory: &mut Inventory) -> bool {
        self.apply_bag_click(inventory, &WindowClick::DragGather { slots: vec![WindowSlot::Inv(slot)] }).ok()
    }
}

impl ClickTarget {
    /// The window transition this click makes, if it moves items. `FillPinned`
    /// needs the pinned card, so the caller builds its `WindowClick::Autofill`;
    /// the rest (book, guide, show-uses) only change what the screen shows.
    pub fn window_click(&self) -> Option<WindowClick> {
        let drag = |at: WindowSlot, distribute: bool| {
            if distribute {
                WindowClick::DragDistribute { slots: vec![at] }
            } else {
                WindowClick::DragGather { slots: vec![at] }
            }
        };
        Some(match *self {
            ClickTarget::GridSlot(row, col, right) => WindowClick::Grid { row, col, right },
            ClickTarget::ResultSlot => WindowClick::Result,
            ClickTarget::InventorySlot(slot, right) => WindowClick::Slot { slot, right },
            ClickTarget::DragInventory(slot, distribute) => drag(WindowSlot::Inv(slot), distribute),
            ClickTarget::DragGrid(r, c, distribute) => drag(WindowSlot::Grid(r, c), distribute),
            ClickTarget::ArmourSlot(slot) => WindowClick::Armour { slot },
            ClickTarget::SortInventory => WindowClick::Sort,
            ClickTarget::ToggleLock(slot) => WindowClick::ToggleLock { slot },
            ClickTarget::TrashCursor => WindowClick::Trash,
            ClickTarget::None
            | ClickTarget::OpenBook
            | ClickTarget::FillPinned
            | ClickTarget::DismissPinned
            | ClickTarget::DrillIngredient(_)
            | ClickTarget::PinnedBack
            | ClickTarget::ShowUses(_) => return None,
        })
    }
}

/// Click target for the crafting UI.
#[derive(Debug, PartialEq, Eq)]
pub enum ClickTarget {
    None,
    /// `(row, col, right_click)`. Left = whole stack, right = single item.
    GridSlot(usize, usize, bool),
    ResultSlot,
    /// `(slot, right_click)`. Left = whole stack, right = single item.
    InventorySlot(usize, bool),
    /// #45 P3 — Mouse-Tweaks drag paint over an inventory slot. `(slot,
    /// distribute)`: distribute=true → RMB deposit one; false → LMB gather.
    DragInventory(usize, bool),
    /// #45 P3 — drag paint over a crafting-grid cell. `(row, col, distribute)`.
    DragGrid(usize, usize, bool),
    /// Spec 28e — armour panel slot (0=Helmet, 1=Chestplate, 2=Leggings, 3=Boots).
    ArmourSlot(usize),
    /// Recipe book (2026-06-12) — the "📖 Recipes" button was clicked; open
    /// the book panel.
    OpenBook,
    /// Placement guide (2026-06-12) — "Fill from bag" on the pinned recipe
    /// card; auto-fill the grid from inventory for the pinned recipe.
    FillPinned,
    /// Placement guide — dismiss the pinned recipe card.
    DismissPinned,
    /// Placement guide drill-down (2026-06-12) — a craftable ingredient was
    /// clicked; push this recipe (global catalogue index) onto the nav stack.
    DrillIngredient(usize),
    /// Placement guide — pop the nav stack one level (← Back / output click).
    PinnedBack,
    /// #45 P1 — the "Sort" button: merge + order the main inventory (9..36),
    /// keeping locked slots fixed.
    SortInventory,
    /// #45 P1 — Alt+click a slot to toggle its lock (excluded from sort /
    /// quick-stack / auto-refill). Slot index 0..36.
    ToggleLock(usize),
    /// #45 #28 — the trash slot: clicking it while carrying a stack on the
    /// cursor destroys the carried stack.
    TrashCursor,
    /// #46 — JEI "show uses": hover an inventory slot and press U to open the
    /// recipe book filtered to recipes that consume that slot's item. Carries
    /// the slot index (0..36); game_loop reads the item + builds the filter.
    ShowUses(usize),
}

// ── Controller slot-cursor (gamepad UI navigation, 2026-06-12) ─────────────
//
// Console pattern (Minecraft Bedrock console editions): the d-pad moves a
// highlighted slot cursor; face buttons act on the highlighted slot
// (A = take/place whole stack, X = split/single, B = close). Presses are
// translated into the SAME `ClickTarget` actions the mouse path produces,
// so the lossless click logic above is reused untouched.

/// Which slot the controller cursor is on. Mirrors the panel layout:
/// armour column | crafting grid → result, above 3 main rows + hotbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadSlot {
    /// Armour column, 0=Helmet .. 3=Boots.
    Armour(usize),
    /// Crafting grid cell `(row, col)` — 2×2 or 3×3 depending on `is_table`.
    Grid(usize, usize),
    /// Crafting result slot.
    Result,
    /// Inventory slot: 0..9 = hotbar, 9..36 = main rows. Matches
    /// `ClickTarget::InventorySlot` indexing exactly.
    Inv(usize),
}

impl PadSlot {
    /// Where the cursor lands on the first d-pad press: hotbar slot 0 —
    /// the slot the player most recently interacted with in-world.
    pub fn start() -> Self {
        PadSlot::Inv(0)
    }
}

/// A d-pad direction press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadDir {
    Up,
    Down,
    Left,
    Right,
}

/// Move the slot cursor one step. Pure; clamps at edges (no wrap, v1).
///
/// Vertical seams: grid/armour/result ↕ main-inventory row 0 (columns
/// mapped: armour ↔ col ≤ 1, grid col c ↔ inv col 3+c, result ↔ col ≥ 7);
/// main row 2 ↕ hotbar (same column).
pub fn pad_move(cur: PadSlot, dir: PadDir, is_table: bool) -> PadSlot {
    use PadDir::*;
    use PadSlot::*;
    let g = if is_table { 3usize } else { 2 }; // grid is g×g

    match (cur, dir) {
        // ── Armour column ──────────────────────────────────────────────
        (Armour(i), Up) => Armour(i.saturating_sub(1)),
        (Armour(i), Down) => {
            if i < 3 {
                Armour(i + 1)
            } else {
                Inv(9) // Boots → first main-inventory slot
            }
        }
        (Armour(i), Right) => Grid(i.min(g - 1), 0),
        (Armour(i), Left) => Armour(i),

        // ── Crafting grid ──────────────────────────────────────────────
        (Grid(r, c), Up) => {
            if r > 0 {
                Grid(r - 1, c.min(g - 1))
            } else {
                Grid(r, c)
            }
        }
        (Grid(r, c), Down) => {
            if r < g - 1 {
                Grid(r + 1, c.min(g - 1))
            } else {
                Inv(9 + (3 + c).min(8)) // into main row 0, column-mapped
            }
        }
        (Grid(r, c), Left) => {
            if c > 0 {
                Grid(r, c - 1)
            } else {
                Armour(r.min(3))
            }
        }
        (Grid(r, c), Right) => {
            if c < g - 1 {
                Grid(r, c + 1)
            } else {
                Result
            }
        }

        // ── Result slot ────────────────────────────────────────────────
        (Result, Left) => Grid((g - 1) / 2, g - 1),
        (Result, Down) => Inv(9 + 8), // main row 0, rightmost column
        (Result, _) => Result,

        // ── Hotbar row (Inv 0..9) ──────────────────────────────────────
        (Inv(i), d) if i < 9 => match d {
            Up => Inv(9 + 18 + i), // main row 2, same column
            Down => Inv(i),
            Left => Inv(i.saturating_sub(1)),
            Right => Inv((i + 1).min(8)),
        },

        // ── Main inventory rows (Inv 9..36) ────────────────────────────
        (Inv(i), d) => {
            let i = i.min(35); // defensive clamp — Inv only ever holds 0..36
            let row = (i - 9) / 9;
            let col = (i - 9) % 9;
            match d {
                Up => {
                    if row > 0 {
                        Inv(i - 9)
                    } else if col <= 1 {
                        Armour(3) // up the left edge → Boots
                    } else if col >= 7 {
                        Result
                    } else {
                        Grid(g - 1, col.saturating_sub(3).min(g - 1))
                    }
                }
                Down => {
                    if row < 2 {
                        Inv(i + 9)
                    } else {
                        Inv(col) // main row 2 → hotbar, same column
                    }
                }
                Left => {
                    if col > 0 {
                        Inv(i - 1)
                    } else {
                        Inv(i)
                    }
                }
                Right => {
                    if col < 8 {
                        Inv(i + 1)
                    } else {
                        Inv(i)
                    }
                }
            }
        }
    }
}

/// Translate a face-button press on the focused slot into the click action
/// the mouse path would have produced. `secondary` = X (right-click
/// semantics: split / place-one); otherwise A (left-click semantics:
/// take / place whole stack). Armour and Result have no right-click
/// behaviour, so both buttons act the same there.
pub fn pad_activate(focus: PadSlot, secondary: bool) -> ClickTarget {
    match focus {
        PadSlot::Grid(r, c) => ClickTarget::GridSlot(r, c, secondary),
        PadSlot::Result => ClickTarget::ResultSlot,
        PadSlot::Inv(i) => ClickTarget::InventorySlot(i, secondary),
        PadSlot::Armour(i) => ClickTarget::ArmourSlot(i),
    }
}

// --- egui Rendering ---

/// Draw the crafting UI via egui. Returns a ClickTarget if the user clicked a slot.
pub fn draw_crafting_ui(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    ui_state: &mut CraftingUi,
    inventory: &Inventory,
    armour_slots: &[Option<ArmourItem>; 4],
    registry: &BlockRegistry,
    egui_integration: &EguiIntegration,
) -> ClickTarget {
    let mut click_target = ClickTarget::None;
    // #45 P3 — Mouse-Tweaks drag paint. Read the pointer button state once; a
    // drag ends (and its painted-slot set clears) the moment no button is held.
    let (any_down, drag_secondary) =
        ctx.input(|i| (i.pointer.any_down(), i.pointer.secondary_down()));
    if !any_down {
        ui_state.clear_drag();
    }
    let grid_size = if ui_state.is_table { 3usize } else { 2 };
    let slot_size = 48.0;
    let gap = 4.0;
    // Controller slot-cursor: rect of the focused slot, captured while
    // drawing so the carried item can anchor beside it (no mouse on a pad).
    let pad_focus = ui_state.pad_focus;
    let mut focused_rect: Option<egui::Rect> = None;
    // Placement-guide anchor: the result slot's screen rect, captured during
    // draw so the "how to place it" card can sit to its right.
    let mut result_rect: Option<egui::Rect> = None;

    // Per-viewport overlay — covers only this player's viewport rect.
    // Painted via layer_painter rather than an Area: in egui 0.31 a
    // `Sense::hover` Area with no widgets has zero layout size, which
    // clips ui.painter() to zero and silently swallows large `rect_filled`
    // calls (the original symptom — overlay invisible).
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("craft_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);


    // Crafting panel centred within the viewport.
    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 300.0,
        viewport.y as f32 + 30.0,
    );
    egui::Area::new(egui::Id::new(("craft_panel", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            // Pin the panel to exactly 600 wide BEFORE child widgets layout —
            // `set_min_width` alone is a post-layout constraint, so
            // `ui.available_width()` inside `vertical_centered` would return
            // the parent Area's still-zero width and every child would
            // collapse onto a zero-pixel column, leaving the entire panel
            // invisible.
            ui.set_min_width(600.0);
            ui.set_max_width(600.0);
            ui.vertical_centered(|ui| {
                ui.add_space(0.0);

                // Title
                let title = if ui_state.is_table { "Workbench" } else { "Crafting" };
                ui.label(
                    egui::RichText::new(title)
                        .size(24.0)
                        .color(TITLE_COLOR)
                        .strong(),
                );

                ui.add_space(16.0);

                // Armour column (4 vertical slots) + crafting grid + arrow + result.
                // Armour column is left of the grid — Minecraft's canonical placement.
                ui.horizontal(|ui| {
                    let total_w = (slot_size + gap) + 16.0 + 300.0;
                    ui.add_space((ui.available_width() - total_w) / 2.0);

                    // Armour column
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                        for (i, slot) in armour_slots.iter().enumerate() {
                            let pad_focused = pad_focus == Some(PadSlot::Armour(i));
                            let response = draw_armour_slot(
                                ui, i, slot.as_ref(),
                                egui_integration, slot_size, pad_focused,
                            );
                            if pad_focused {
                                focused_rect = Some(response.rect);
                            }
                            if response.clicked() {
                                click_target = ClickTarget::ArmourSlot(i);
                            }
                        }
                    });

                    ui.add_space(16.0);

                    // Crafting grid
                    ui.vertical(|ui| {
                        for r in 0..grid_size {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                                for c in 0..grid_size {
                                    let pad_focused = pad_focus == Some(PadSlot::Grid(r, c));
                                    let response = draw_item_slot(
                                        ui, &ui_state.grid[r][c], registry, egui_integration, slot_size, false, pad_focused, true,
                                    );
                                    if pad_focused {
                                        focused_rect = Some(response.rect);
                                    }
                                    if response.clicked() {
                                        click_target = ClickTarget::GridSlot(r, c, false);
                                    } else if response.secondary_clicked() {
                                        click_target = ClickTarget::GridSlot(r, c, true);
                                    }
                                    // #45 P3 — drag paint over grid cells.
                                    let gid = (100 + r * grid_size + c) as u32;
                                    if response.drag_started() {
                                        ui_state.drag_paint = Some(drag_secondary);
                                        ui_state.drag_visited.clear();
                                        ui_state.drag_visited.insert(gid);
                                        click_target = ClickTarget::DragGrid(r, c, drag_secondary);
                                    } else if let Some(distribute) = ui_state.drag_paint
                                        && response.hovered() && ui_state.drag_visited.insert(gid) {
                                            click_target = ClickTarget::DragGrid(r, c, distribute);
                                        }
                                }
                            });
                        }
                    });

                    // Arrow
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space(grid_size as f32 * (slot_size + gap) / 2.0 - 10.0);
                        ui.label(
                            egui::RichText::new("→")
                                .size(24.0)
                                .color(LABEL_COLOR),
                        );
                    });
                    ui.add_space(12.0);

                    // Result slot
                    ui.vertical(|ui| {
                        ui.add_space(grid_size as f32 * (slot_size + gap) / 2.0 - slot_size / 2.0);
                        let has_result = ui_state.result.is_some();
                        let pad_focused = pad_focus == Some(PadSlot::Result);
                        let response = draw_item_slot(
                            ui, &ui_state.result, registry, egui_integration, slot_size, has_result, pad_focused, false,
                        );
                        if pad_focused {
                            focused_rect = Some(response.rect);
                        }
                        result_rect = Some(response.rect);
                        if response.clicked() {
                            click_target = ClickTarget::ResultSlot;
                        }
                    });
                });

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(8.0);

                // Inventory label
                ui.label(
                    egui::RichText::new("Inventory")
                        .size(14.0)
                        .color(LABEL_COLOR),
                );
                ui.add_space(4.0);

                // Main inventory (3 rows of 9)
                for row in 0..3 {
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 9.0 * (slot_size + gap)) / 2.0);
                        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                        for col in 0..9 {
                            let slot_idx = 9 + row * 9 + col;
                            let stack = inventory.slot(slot_idx);
                            let pad_focused = pad_focus == Some(PadSlot::Inv(slot_idx));
                            let response = draw_item_slot(
                                ui, &stack.cloned(), registry, egui_integration, slot_size, false, pad_focused, true,
                            );
                            if inventory.is_locked(slot_idx) {
                                mark_locked(ui, response.rect);
                            }
                            if pad_focused {
                                focused_rect = Some(response.rect);
                            }
                            // #45 — Alt+click toggles the slot lock instead of moving.
                            let alt = ui.input(|i| i.modifiers.alt);
                            if response.clicked() {
                                click_target = if alt {
                                    ClickTarget::ToggleLock(slot_idx)
                                } else {
                                    ClickTarget::InventorySlot(slot_idx, false)
                                };
                            } else if response.secondary_clicked() {
                                click_target = ClickTarget::InventorySlot(slot_idx, true);
                            } else if response.hovered()
                                && stack.is_some()
                                && ui.input(|i| i.key_pressed(egui::Key::U))
                            {
                                // #46 — JEI "show uses" on the hovered item.
                                click_target = ClickTarget::ShowUses(slot_idx);
                            }
                            // #45 P3 — drag paint over main-inventory slots.
                            let iid = slot_idx as u32;
                            if response.drag_started() {
                                ui_state.drag_paint = Some(drag_secondary);
                                ui_state.drag_visited.clear();
                                ui_state.drag_visited.insert(iid);
                                click_target = ClickTarget::DragInventory(slot_idx, drag_secondary);
                            } else if let Some(distribute) = ui_state.drag_paint
                                && response.hovered() && ui_state.drag_visited.insert(iid) {
                                    click_target = ClickTarget::DragInventory(slot_idx, distribute);
                                }
                        }
                    });
                }

                ui.add_space(8.0);

                // Hotbar label
                ui.label(
                    egui::RichText::new("Hotbar")
                        .size(14.0)
                        .color(LABEL_COLOR),
                );
                ui.add_space(4.0);

                // Hotbar (9 slots)
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 9.0 * (slot_size + gap)) / 2.0);
                    ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                    for i in 0..9 {
                        let stack = inventory.hotbar_slot(i);
                        let pad_focused = pad_focus == Some(PadSlot::Inv(i));
                        let response = draw_item_slot(
                            ui, &stack.cloned(), registry, egui_integration, slot_size, false, pad_focused, true,
                        );
                        if inventory.is_locked(i) {
                            mark_locked(ui, response.rect);
                        }
                        if pad_focused {
                            focused_rect = Some(response.rect);
                        }
                        let alt = ui.input(|i| i.modifiers.alt);
                        if response.clicked() {
                            click_target = if alt {
                                ClickTarget::ToggleLock(i)
                            } else {
                                ClickTarget::InventorySlot(i, false)
                            };
                        } else if response.secondary_clicked() {
                            click_target = ClickTarget::InventorySlot(i, true);
                        } else if response.hovered()
                            && stack.is_some()
                            && ui.input(|inp| inp.key_pressed(egui::Key::U))
                        {
                            click_target = ClickTarget::ShowUses(i);
                        }
                        // #45 P3 — drag paint over hotbar slots (ids 0..9).
                        let hid = i as u32;
                        if response.drag_started() {
                            ui_state.drag_paint = Some(drag_secondary);
                            ui_state.drag_visited.clear();
                            ui_state.drag_visited.insert(hid);
                            click_target = ClickTarget::DragInventory(i, drag_secondary);
                        } else if let Some(distribute) = ui_state.drag_paint
                            && response.hovered() && ui_state.drag_visited.insert(hid) {
                                click_target = ClickTarget::DragInventory(i, distribute);
                            }
                    }
                });

                ui.add_space(12.0);
                // #45 — inventory QoL buttons. Ordinary egui buttons, so they're
                // reachable by mouse, touch, and (via the panel focus) the
                // gamepad cursor. Sort tidies the main 27-slot region; Trash bins
                // whatever is on the cursor; Alt+click a slot to lock it.
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 360.0).max(0.0) / 2.0);
                    if ui.add(qol_button("↕ Sort")).clicked() {
                        click_target = ClickTarget::SortInventory;
                    }
                    let trash_armed = ui_state.cursor_item.is_some();
                    let trash = egui::Button::new(
                        egui::RichText::new("🗑 Trash").size(14.0).color(
                            if trash_armed {
                                egui::Color32::from_rgb(230, 150, 150)
                            } else {
                                egui::Color32::from_rgb(120, 100, 100)
                            },
                        ),
                    )
                    .min_size(egui::vec2(110.0, 30.0))
                    .fill(egui::Color32::from_rgb(44, 28, 28))
                    .corner_radius(egui::CornerRadius::same(6))
                    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 50, 50)));
                    if ui.add(trash).clicked() {
                        click_target = ClickTarget::TrashCursor;
                    }
                });

                ui.add_space(6.0);
                // Recipe book opener (2026-06-12). Y on a controller also
                // opens it (wired in game_loop) — the hint says so.
                let book_btn = egui::Button::new(
                    egui::RichText::new("📖 Recipe Book").size(14.0)
                        .color(egui::Color32::from_rgb(220, 200, 120)),
                )
                .min_size(egui::vec2(160.0, 30.0))
                .fill(egui::Color32::from_rgb(48, 42, 28))
                .corner_radius(egui::CornerRadius::same(6))
                .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 78, 44)));
                if ui.add(book_btn).clicked() {
                    click_target = ClickTarget::OpenBook;
                }

                ui.add_space(10.0);
                // Footer legend — controller hints once the pad cursor is
                // active (console pattern: no hover tooltips on a pad, so
                // discoverability lives on-screen).
                let footer = if pad_focus.is_some() {
                    "D-pad Move · A Take/Place · X Split/One · Y Recipe Book · B Close"
                } else {
                    "Click to move · Alt+click to lock · hover+U for uses · Esc to close"
                };
                ui.label(
                    egui::RichText::new(footer)
                        .size(12.0)
                        .color(egui::Color32::from_rgb(90, 90, 90)),
                );
            });
        });

    // Placement guide (2026-06-12) — "how to place it" mini-grid card pinned
    // to the right of the result slot. Shows the exact ingredient layout for
    // the recipe the player picked from the book, so they can replicate it.
    // Drill-down: hover an ingredient for its name, click a craftable one to
    // jump to its recipe; ← Back / output click returns up the stack.
    if let (Some(gidx), Some(anchor)) = (ui_state.pinned_recipe(), result_rect)
        && let Some(card) = crate::crafting_catalogue::all_cards().get(gidx) {
            let has_parent = ui_state.pinned_recipe_stack.len() > 1;
            if let Some(guide_action) = draw_placement_guide(
                ctx, player_index, anchor, card, has_parent, registry, egui_integration,
            ) {
                click_target = guide_action;
            }
        }

    // Draw cursor item following mouse via `layer_painter` instead of an
    // `Area`. An Area would occupy hit-test space at the cursor — even with
    // `interactable(false)`, in egui 0.31 it blocks the click from reaching
    // the slot widget beneath, so click-to-place silently fails. A direct
    // painter has no widgets and no hit-test region, so clicks fall through
    // to the slot the user is placing onto.
    //
    // Controller slot-cursor: there is no meaningful pointer on a pad, so
    // while the pad cursor is active the carried item anchors beside the
    // focused slot instead of following the mouse.
    if let Some(cursor) = &ui_state.cursor_item {
        let anchor = if pad_focus.is_some() {
            focused_rect.map(|r| r.right_top() + egui::vec2(10.0, 6.0))
        } else {
            ctx.input(|i| i.pointer.latest_pos())
        };
        if let Some(pos) = anchor {
            let cursor_size = slot_size * 0.8;
            let icon_rect = egui::Rect::from_center_size(pos, egui::vec2(cursor_size, cursor_size));
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new(("cursor_item", player_index)),
            ));
            paint_item_icon(&painter, icon_rect, cursor, registry, egui_integration);

            // Floating name label next to the cursor — so the player can read
            // what they're carrying without having to drop it onto a slot.
            let label_text = if cursor.count > 1 {
                format!("{} ({})", cursor.item.name(registry), cursor.count)
            } else {
                cursor.item.name(registry)
            };
            let label_anchor = icon_rect.right_top() + egui::vec2(8.0, 0.0);
            let font = egui::FontId::proportional(13.0);
            let text_galley =
                painter.layout_no_wrap(label_text.clone(), font.clone(), egui::Color32::WHITE);
            let bg_rect = egui::Rect::from_min_size(
                label_anchor + egui::vec2(-4.0, -2.0),
                text_galley.size() + egui::vec2(8.0, 4.0),
            );
            painter.rect_filled(
                bg_rect,
                3.0,
                egui::Color32::from_rgba_premultiplied(0, 0, 0, 200),
            );
            painter.galley(label_anchor, text_galley, egui::Color32::WHITE);
        }
    }

    click_target
}

/// Spec 28e — material → swatch colour. Matches the values in
/// `Item::color` for armour so the slot column and the inventory grid
/// agree visually.
fn armour_material_color(mat: armour::ArmourMaterial) -> [f32; 3] {
    use armour::ArmourMaterial::*;
    match mat {
        Leather => [0.55, 0.35, 0.18],
        Iron => [0.75, 0.75, 0.78],
        Diamond => [0.40, 0.92, 0.90],
        Satori => [0.95, 0.55, 0.18],
        Chainmail => [0.55, 0.55, 0.60],
        Rubber => [0.42, 0.30, 0.22],
    }
}

/// Spec 28e — draw one armour slot in the inventory overlay's left column.
/// Shows the equipped piece (icon + durability bar) or an empty-slot
/// placeholder labelled with the slot name. Returns the click response.
fn draw_armour_slot(
    ui: &mut egui::Ui,
    armour_slot_idx: usize,
    piece: Option<&ArmourItem>,
    _egui_integration: &EguiIntegration,
    size: f32,
    pad_focused: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(size, size),
        egui::Sense::click(),
    );

    let bg = if response.hovered() || pad_focused {
        egui::Color32::from_rgba_premultiplied(40, 45, 60, 255)
    } else {
        SLOT_BG
    };
    // Armour slots get a slightly warmer border so they read as a
    // separate "equipment" region from the inventory grid. The controller
    // slot-cursor overrides with the shared gold focus ring.
    let (border, stroke_w) = if pad_focused {
        (PAD_FOCUS_BORDER, 2.0_f32)
    } else {
        (egui::Color32::from_rgb(110, 90, 60), 1.0_f32)
    };
    ui.painter().rect_filled(rect, 3.0, bg);
    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(stroke_w, border), egui::StrokeKind::Inside);

    let slot_kind = match armour_slot_idx {
        0 => ArmSlot::Helmet,
        1 => ArmSlot::Chestplate,
        2 => ArmSlot::Leggings,
        3 => ArmSlot::Boots,
        _ => ArmSlot::Helmet,
    };

    let icon_rect = rect.shrink(4.0);
    let label_text = match piece {
        Some(p) => armour::armour_label(p.slot, p.material).to_string(),
        None => match slot_kind {
            ArmSlot::Helmet => "Empty Helmet".into(),
            ArmSlot::Chestplate => "Empty Chestplate".into(),
            ArmSlot::Leggings => "Empty Leggings".into(),
            ArmSlot::Boots => "Empty Boots".into(),
        },
    };

    if let Some(p) = piece {
        // Material-tinted swatch — mirrors the inventory grid's Armour
        // rendering so the player sees the same colour at a glance.
        let color = armour_material_color(p.material);
        let arm_color = egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );
        ui.painter().rect_filled(icon_rect, 2.0, arm_color);

        // Slot-glyph in the corner so the four pieces are
        // distinguishable at a glance: H/C/L/B.
        let glyph = match p.slot {
            ArmSlot::Helmet => "H",
            ArmSlot::Chestplate => "C",
            ArmSlot::Leggings => "L",
            ArmSlot::Boots => "B",
        };
        ui.painter().text(
            icon_rect.left_top() + egui::vec2(2.0, 2.0),
            egui::Align2::LEFT_TOP,
            glyph,
            egui::FontId::proportional(11.0),
            egui::Color32::WHITE,
        );

        // Durability bar (green→red), same shape as the tool bar.
        let max_dur = armour::max_durability(p.slot, p.material);
        if p.durability < max_dur {
            let pct = p.durability as f32 / max_dur as f32;
            let bar_h = 3.0;
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.bottom() - bar_h - 2.0),
                egui::vec2(rect.width() - 4.0, bar_h),
            );
            ui.painter().rect_filled(bar_rect, 1.0, egui::Color32::from_rgb(200, 50, 50));
            let fill_rect = egui::Rect::from_min_size(
                bar_rect.left_top(),
                egui::vec2(bar_rect.width() * pct, bar_h),
            );
            ui.painter().rect_filled(fill_rect, 1.0, egui::Color32::from_rgb(50, 200, 50));
        }
    } else {
        // Empty-slot glyph — single letter dimmed to ~40% so a player
        // glances at the column and sees "H/C/L/B = head/chest/legs/boots".
        let glyph = match slot_kind {
            ArmSlot::Helmet => "H",
            ArmSlot::Chestplate => "C",
            ArmSlot::Leggings => "L",
            ArmSlot::Boots => "B",
        };
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(18.0),
            egui::Color32::from_rgb(90, 90, 100),
        );
    }

    response.on_hover_text(label_text)
}

/// Placement-guide card (2026-06-12): a small "how to place it" panel pinned
/// to the right of the result slot. Renders the recipe's exact ingredient
/// layout as a mini 3×3 grid → output, with a "Fill from bag" button and a
/// dismiss ✕.
///
/// Drill-down: each ingredient cell shows its name on hover; clicking a cell
/// whose item is itself craftable returns [`ClickTarget::DrillIngredient`] to
/// jump to that recipe. `has_parent` (we drilled in from another recipe) shows
/// a "← Back" button and makes the output icon a back affordance, both
/// returning [`ClickTarget::PinnedBack`].
fn draw_placement_guide(
    ctx: &egui::Context,
    player_index: usize,
    anchor: egui::Rect,
    card: &crate::crafting_catalogue::RecipeCard,
    has_parent: bool,
    registry: &BlockRegistry,
    egui_integration: &EguiIntegration,
) -> Option<ClickTarget> {
    let cell = 30.0_f32;
    let gap = 3.0;
    // Sit just right of the result slot, nudged up so the 3-row grid centres
    // roughly on the slot.
    let origin = anchor.right_top() + egui::vec2(20.0, -28.0);
    let mut result = None;

    egui::Area::new(egui::Id::new(("placement_guide", player_index)))
        .fixed_pos(origin)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(28, 30, 40))
                .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 78, 44)))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_max_width(3.0 * (cell + gap) + 150.0);
                    // Header: [← Back] title … [✕]
                    ui.horizontal(|ui| {
                        if has_parent
                            && ui
                                .add(egui::Button::new(egui::RichText::new("← Back").size(12.0))
                                    .fill(egui::Color32::from_rgb(40, 44, 60))
                                    .corner_radius(egui::CornerRadius::same(4)))
                                .on_hover_text("Back to the recipe you came from")
                                .clicked()
                        {
                            result = Some(ClickTarget::PinnedBack);
                        }
                        ui.label(
                            egui::RichText::new("How to place").size(13.0).color(TITLE_COLOR).strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(egui::Button::new(egui::RichText::new("✕").size(13.0))
                                    .fill(egui::Color32::from_rgb(46, 31, 31))
                                    .corner_radius(egui::CornerRadius::same(4)))
                                .on_hover_text("Hide this guide")
                                .clicked()
                            {
                                result = Some(ClickTarget::DismissPinned);
                            }
                        });
                    });
                    ui.label(egui::RichText::new(&card.name).size(12.0).color(egui::Color32::from_rgb(190, 200, 220)));
                    ui.add_space(6.0);

                    // Mini grid → output, on one row.
                    ui.horizontal(|ui| {
                        // 3×3 layout (cells anchored top-left as stored). Each
                        // non-empty cell is interactive: hover = name, click =
                        // drill into that ingredient's recipe if one exists.
                        let (grid_rect, _) = ui.allocate_exact_size(
                            egui::vec2(3.0 * cell + 2.0 * gap, 3.0 * cell + 2.0 * gap),
                            egui::Sense::hover(),
                        );
                        for r in 0..3 {
                            for c in 0..3 {
                                let slot = card.example_grid[r][c];
                                let cell_rect = egui::Rect::from_min_size(
                                    grid_rect.min + egui::vec2(c as f32 * (cell + gap), r as f32 * (cell + gap)),
                                    egui::vec2(cell, cell),
                                );
                                paint_mini_cell(ui.painter(), cell_rect, slot, registry, egui_integration);

                                let item = match slot {
                                    CraftSlot::Block(b) => Some(Item::Block(b)),
                                    CraftSlot::Material(m) => Some(Item::Material(m)),
                                    CraftSlot::Empty => None,
                                };
                                if let Some(item) = item {
                                    let drill =
                                        crate::crafting_catalogue::recipe_index_for_output(&item);
                                    let name = item.name(registry);
                                    let id = ui.id().with(("guide_cell", r, c));
                                    let resp = ui.interact(cell_rect, id, egui::Sense::click());
                                    // Highlight a craftable ingredient so it
                                    // reads as clickable.
                                    if drill.is_some() {
                                        ui.painter().rect_stroke(
                                            cell_rect,
                                            3.0,
                                            egui::Stroke::new(
                                                if resp.hovered() { 2.0_f32 } else { 1.0_f32 },
                                                egui::Color32::from_rgb(120, 170, 230),
                                            ),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    let hover = if drill.is_some() {
                                        format!("{name}\n(click to see how to make it)")
                                    } else {
                                        name
                                    };
                                    let resp = resp.on_hover_text(hover);
                                    if resp.clicked()
                                        && let Some(target) = drill {
                                            result = Some(ClickTarget::DrillIngredient(target));
                                        }
                                }
                            }
                        }
                        // Arrow + output. The output is a "← Back" affordance
                        // when we drilled in from a parent recipe.
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("→").size(20.0).color(LABEL_COLOR));
                        ui.add_space(6.0);
                        let (out_rect, _) = ui.allocate_exact_size(egui::vec2(cell, cell), egui::Sense::hover());
                        paint_item_icon(ui.painter(), out_rect, &card.output, registry, egui_integration);
                        let out_name = card.output.item.name(registry);
                        let out_id = ui.id().with("guide_output");
                        let out_resp = ui.interact(out_rect, out_id, egui::Sense::click());
                        let out_hover = if has_parent {
                            format!("{out_name}\n(click to go back)")
                        } else {
                            out_name
                        };
                        let out_resp = out_resp.on_hover_text(out_hover);
                        if out_resp.clicked() && has_parent {
                            result = Some(ClickTarget::PinnedBack);
                        }
                    });

                    ui.add_space(8.0);
                    if ui
                        .add(egui::Button::new(
                            egui::RichText::new("⛏ Fill from bag").size(12.0).color(egui::Color32::from_rgb(184, 232, 184)),
                        )
                        .fill(egui::Color32::from_rgb(45, 70, 45))
                        .corner_radius(egui::CornerRadius::same(5)))
                        .on_hover_text("Lay these items into the grid from your inventory")
                        .clicked()
                    {
                        result = Some(ClickTarget::FillPinned);
                    }
                });
        });

    result
}

/// #45 — a styled inventory-QoL button (Sort etc.), matching the Recipe Book
/// button's footprint for a tidy row.
fn qol_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label.to_string())
            .size(14.0)
            .color(egui::Color32::from_rgb(200, 210, 230)),
    )
    .min_size(egui::vec2(110.0, 30.0))
    .fill(egui::Color32::from_rgb(30, 36, 48))
    .corner_radius(egui::CornerRadius::same(6))
    .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 82, 110)))
}

/// #45 — paint a small amber corner wedge + padlock glyph on a locked slot so
/// the lock state reads at a glance.
fn mark_locked(ui: &egui::Ui, rect: egui::Rect) {
    let amber = egui::Color32::from_rgb(255, 200, 90);
    let p = ui.painter();
    let tl = rect.left_top();
    // Top-left wedge.
    p.add(egui::Shape::convex_polygon(
        vec![
            tl,
            tl + egui::vec2(12.0, 0.0),
            tl + egui::vec2(0.0, 12.0),
        ],
        amber,
        egui::Stroke::NONE,
    ));
    p.text(
        tl + egui::vec2(2.0, 1.0),
        egui::Align2::LEFT_TOP,
        "\u{1F512}",
        egui::FontId::proportional(9.0),
        egui::Color32::from_rgb(40, 30, 0),
    );
}

/// Paint one cell of the placement-guide mini grid: the ingredient icon, or a
/// faint empty slot.
fn paint_mini_cell(
    painter: &egui::Painter,
    rect: egui::Rect,
    slot: CraftSlot,
    registry: &BlockRegistry,
    egui_integration: &EguiIntegration,
) {
    painter.rect_filled(rect, 3.0, SLOT_BG);
    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, SLOT_BORDER), egui::StrokeKind::Inside);
    let stack = match slot {
        CraftSlot::Block(b) => Some(ItemStack::new_block(b, 1)),
        CraftSlot::Material(m) => Some(ItemStack::new_material(m, 1)),
        CraftSlot::Empty => None,
    };
    if let Some(s) = stack {
        paint_item_icon(painter, rect.shrink(3.0), &s, registry, egui_integration);
    }
}

fn draw_item_slot(
    ui: &mut egui::Ui,
    stack: &Option<ItemStack>,
    registry: &BlockRegistry,
    egui_integration: &EguiIntegration,
    size: f32,
    highlight: bool,
    pad_focused: bool,
    sense_drag: bool,
) -> egui::Response {
    // #45 P3 — inventory + grid slots sense drag so a Mouse-Tweaks paint gesture
    // (RMB-distribute / LMB-gather) registers, and `.clicked()` does NOT fire at
    // the end of a drag (so a paint doesn't also trigger a single-slot move).
    let sense = if sense_drag {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), sense);

    // Slot background
    let bg = if response.hovered() || pad_focused {
        egui::Color32::from_rgba_premultiplied(40, 45, 60, 255)
    } else {
        SLOT_BG
    };
    // Controller slot-cursor ring wins over the result highlight — the
    // player needs to see where the cursor is even on the result slot.
    let (border, stroke_w) = if pad_focused {
        (PAD_FOCUS_BORDER, 2.0_f32)
    } else if highlight {
        (RESULT_BORDER, 1.0_f32)
    } else {
        (SLOT_BORDER, 1.0_f32)
    };
    ui.painter().rect_filled(rect, 3.0, bg);
    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(stroke_w, border), egui::StrokeKind::Inside);

    // Item content
    if let Some(stack) = stack {
        let icon_rect = rect.shrink(4.0);

        match &stack.item {
            Item::Block(id) => {
                let tex_layer = registry.tex_top(*id);
                if let Some(tex_id) = egui_integration.block_texture(tex_layer) {
                    ui.painter().image(
                        tex_id,
                        icon_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
            Item::Tool(tool) => {
                let color = stack.item.color(registry);
                let tool_color = egui::Color32::from_rgb(
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                );
                ui.painter().rect_filled(icon_rect, 2.0, tool_color);
                let indicator = match tool.tool_type {
                    crate::crafting::ToolType::Sword => "S",
                    crate::crafting::ToolType::Pickaxe => "P",
                    crate::crafting::ToolType::Axe => "A",
                    crate::crafting::ToolType::Shovel => "V",
                    crate::crafting::ToolType::Bow => "B",
                    crate::crafting::ToolType::Hoe => "H",
                    crate::crafting::ToolType::FlintAndSteel => "F",
                    crate::crafting::ToolType::Shears => "Sh",
                    crate::crafting::ToolType::FishingRod => "Fr",
                    crate::crafting::ToolType::Slingshot => "Sl",
                    crate::crafting::ToolType::Eraser => "Er",
                    crate::crafting::ToolType::DraftingStamp => "Ds",
                };
                ui.painter().text(
                    icon_rect.left_top() + egui::vec2(2.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    indicator,
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }
            Item::Material(_) => {
                let color = stack.item.color(registry);
                let mat_color = egui::Color32::from_rgb(
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                );
                ui.painter().rect_filled(icon_rect, 2.0, mat_color);
            }
            Item::Plan(_) => {
                // Spec 24 — captured-building blueprint. Dark-gold
                // parchment swatch with a small "P" label. Full
                // inspect-dialog ships in Phase 7.
                let color = stack.item.color(registry);
                let plan_color = egui::Color32::from_rgb(
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                );
                ui.painter().rect_filled(icon_rect, 2.0, plan_color);
                ui.painter().text(
                    icon_rect.left_top() + egui::vec2(2.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    "P",
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }
            Item::Armour(_) => {
                // Spec 28e — armour piece. Tier-tinted swatch with an
                // "A" label until per-slot icons land.
                let color = stack.item.color(registry);
                let arm_color = egui::Color32::from_rgb(
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                );
                ui.painter().rect_filled(icon_rect, 2.0, arm_color);
                ui.painter().text(
                    icon_rect.left_top() + egui::vec2(2.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    "A",
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
            }
        }

        // Stack count
        if stack.count > 1 {
            let text = format!("{}", stack.count);
            ui.painter().text(
                rect.right_bottom() + egui::vec2(-3.0, -3.0),
                egui::Align2::RIGHT_BOTTOM,
                &text,
                egui::FontId::proportional(13.0),
                egui::Color32::from_rgb(20, 20, 20),
            );
            ui.painter().text(
                rect.right_bottom() + egui::vec2(-4.0, -4.0),
                egui::Align2::RIGHT_BOTTOM,
                &text,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }

        // Durability bar
        if let Item::Tool(tool) = &stack.item {
            let max_dur = crate::crafting::Tool::new(tool.tool_type, tool.material).durability;
            if tool.durability < max_dur {
                let pct = tool.durability as f32 / max_dur as f32;
                let bar_h = 3.0;
                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 2.0, rect.bottom() - bar_h - 2.0),
                    egui::vec2(rect.width() - 4.0, bar_h),
                );
                ui.painter().rect_filled(bar_rect, 1.0, egui::Color32::from_rgb(200, 50, 50));
                let fill_rect = egui::Rect::from_min_size(
                    bar_rect.left_top(),
                    egui::vec2(bar_rect.width() * pct, bar_h),
                );
                ui.painter().rect_filled(fill_rect, 1.0, egui::Color32::from_rgb(50, 200, 50));
            }
        }
    }

    // Hover tooltip — Minecraft-style "what is this thing?" label. Item
    // textures don't always read clearly at 48 px, and tools currently render
    // as single-letter colour squares; the name on hover removes the guesswork.
    

    if let Some(stack) = stack {
        let name = stack.item.name(registry);
        let label = if stack.count > 1 {
            format!("{} ({})", name, stack.count)
        } else {
            name
        };
        response.on_hover_text(label)
    } else {
        response
    }
}

/// Paint the cursor-following item icon directly via a `Painter`, with no
/// containing widget so clicks fall through to the slot beneath.
fn paint_item_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    stack: &ItemStack,
    registry: &BlockRegistry,
    egui_integration: &EguiIntegration,
) {
    match &stack.item {
        Item::Block(id) => {
            let tex_layer = registry.tex_top(*id);
            if let Some(tex_id) = egui_integration.block_texture(tex_layer) {
                painter.image(
                    tex_id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
        _ => {
            let color = stack.item.color(registry);
            painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
            ));
        }
    }

    if stack.count > 1 {
        painter.text(
            rect.right_bottom() + egui::vec2(-4.0, -4.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{}", stack.count),
            egui::FontId::proportional(13.0),
            egui::Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    //! Spec 28e — armour-slot click handling.
    //!
    //! These exercise the pure click-routing logic (`click_armour_slot`)
    //! without spinning up egui. The rendering path is exercised by hand
    //! in the playtest gate.
    use super::*;
    use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot as ArmSlot};

    fn empty_slots() -> [Option<ArmourItem>; 4] { [None, None, None, None] }

    #[test]
    fn equip_from_empty_cursor_picks_up_piece() {
        let mut ui = CraftingUi::new();
        let mut slots = empty_slots();
        slots[ArmSlot::Helmet as usize] = Some(ArmourItem::new(ArmSlot::Helmet, ArmourMaterial::Iron));
        let ok = ui.click_armour_slot(ArmSlot::Helmet as usize, &mut slots);
        assert!(ok);
        assert!(slots[ArmSlot::Helmet as usize].is_none(), "slot must be empty after unequip");
        match ui.cursor_item.as_ref().expect("cursor must carry the piece") {
            ItemStack { item: Item::Armour(a), count: 1 } => {
                assert_eq!(a.slot, ArmSlot::Helmet);
                assert_eq!(a.material, ArmourMaterial::Iron);
            }
            _ => panic!("cursor doesn't carry the expected armour piece"),
        }
    }

    #[test]
    fn equip_matching_armour_from_cursor_fills_slot() {
        let mut ui = CraftingUi::new();
        let mut slots = empty_slots();
        ui.cursor_item = Some(ItemStack {
            item: Item::Armour(ArmourItem::new(ArmSlot::Chestplate, ArmourMaterial::Diamond)),
            count: 1,
        });
        let ok = ui.click_armour_slot(ArmSlot::Chestplate as usize, &mut slots);
        assert!(ok);
        assert!(ui.cursor_item.is_none(), "cursor must be empty after equip");
        let p = slots[ArmSlot::Chestplate as usize].as_ref().unwrap();
        assert_eq!(p.material, ArmourMaterial::Diamond);
    }

    #[test]
    fn equip_swaps_when_slot_was_occupied() {
        let mut ui = CraftingUi::new();
        let mut slots = empty_slots();
        slots[ArmSlot::Boots as usize] = Some(ArmourItem::new(ArmSlot::Boots, ArmourMaterial::Leather));
        ui.cursor_item = Some(ItemStack {
            item: Item::Armour(ArmourItem::new(ArmSlot::Boots, ArmourMaterial::Iron)),
            count: 1,
        });
        let ok = ui.click_armour_slot(ArmSlot::Boots as usize, &mut slots);
        assert!(ok);
        assert_eq!(
            slots[ArmSlot::Boots as usize].unwrap().material,
            ArmourMaterial::Iron,
        );
        match ui.cursor_item.as_ref().expect("cursor carries the swapped piece") {
            ItemStack { item: Item::Armour(a), .. } => assert_eq!(a.material, ArmourMaterial::Leather),
            _ => panic!("cursor lost the swapped-out piece"),
        }
    }

    #[test]
    fn mismatched_slot_equip_is_a_no_op_and_keeps_cursor() {
        let mut ui = CraftingUi::new();
        let mut slots = empty_slots();
        ui.cursor_item = Some(ItemStack {
            item: Item::Armour(ArmourItem::new(ArmSlot::Helmet, ArmourMaterial::Iron)),
            count: 1,
        });
        // Try to put a Helmet into the Chestplate slot.
        let ok = ui.click_armour_slot(ArmSlot::Chestplate as usize, &mut slots);
        assert!(!ok, "wrong-slot equip should report no-op");
        assert!(slots[ArmSlot::Chestplate as usize].is_none(), "slot must remain empty");
        // Cursor still carries the helmet — player doesn't lose the item.
        match ui.cursor_item.as_ref().expect("cursor must keep the helmet") {
            ItemStack { item: Item::Armour(a), .. } => {
                assert_eq!(a.slot, ArmSlot::Helmet);
                assert_eq!(a.material, ArmourMaterial::Iron);
            }
            _ => panic!("cursor lost its item on mismatched click"),
        }
    }

    #[test]
    fn click_armour_slot_with_non_armour_cursor_is_no_op() {
        let mut ui = CraftingUi::new();
        let mut slots = empty_slots();
        ui.cursor_item = Some(ItemStack::new_block(crate::block::STONE, 32));
        let ok = ui.click_armour_slot(ArmSlot::Helmet as usize, &mut slots);
        assert!(!ok);
        assert!(slots[ArmSlot::Helmet as usize].is_none());
        // Cursor still carries the stone — player can drop it back into
        // the inventory grid without losing it.
        let cursor = ui.cursor_item.as_ref().unwrap();
        assert_eq!(cursor.count, 32);
        assert!(matches!(cursor.item, Item::Block(_)));
    }

    #[test]
    fn click_result_matching_cursor_merges_up_to_max() {
        // Regression (pre-alpha bug hunt 2026-06-22): picking up a crafted result
        // while the cursor holds the SAME item must merge onto the cursor up to
        // max_stack. The old code routed the result to the inventory and left the
        // cursor unchanged (and on a partial inventory fit it duplicated the result
        // onto the cursor). 60 + 8 → cursor fills to 64.
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        ui.cursor_item = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 60));
        ui.result = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 8));
        ui.click_result(&mut inv);
        assert_eq!(
            ui.cursor_item.as_ref().unwrap().count,
            64,
            "matching cursor merges the result up to max_stack"
        );
    }

    #[test]
    fn click_result_full_inventory_different_cursor_preserves_cursor() {
        // Regression: with a FULL inventory and a DIFFERENT item on the cursor, the
        // old code overwrote the cursor with the full result (conjuring it from
        // nowhere — an item dup) and dropped the cursor's own stack. The fix must
        // leave the cursor untouched when the result can't be delivered.
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        for _ in 0..36 {
            let _ = inv.add_item(ItemStack::new_block(crate::block::STONE, 64)); // fill every slot
        }
        ui.cursor_item = Some(ItemStack::new_block(crate::block::STONE, 10));
        ui.result = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 64));
        ui.click_result(&mut inv);
        let cursor = ui.cursor_item.as_ref().expect("cursor must be preserved");
        assert!(
            matches!(cursor.item, Item::Block(b) if b == crate::block::STONE) && cursor.count == 10,
            "cursor keeps its original Stone x10 — no conjured result, no loss"
        );
    }

    #[test]
    fn click_result_that_cannot_be_placed_consumes_nothing() {
        // Audit 2026-09-27: a pickaxe on the cursor + a full inventory used to
        // consume the planks' log and delete the 4-plank result.
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        for _ in 0..36 {
            let _ = inv.add_item(ItemStack::new_block(crate::block::STONE, 64));
        }
        ui.grid[0][0] = Some(ItemStack::new_block(crate::block::OAK_LOG, 1));
        ui.update_result();
        assert!(ui.result.is_some(), "a log crafts planks");
        ui.cursor_item = Some(ItemStack::new_block(crate::block::DIRT, 3));
        assert!(!ui.click_result(&mut inv), "the craft is refused");
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(1), "the log is not consumed");
        assert!(ui.result.is_some(), "the result is still on offer");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(3), "cursor untouched");

        // Same with a stacking cursor whose overflow has nowhere to go.
        ui.cursor_item = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 62));
        assert!(!ui.click_result(&mut inv), "2 fit the cursor but 2 would be lost");
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(1));
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(62));
    }

    #[test]
    fn click_empty_slot_with_empty_cursor_is_a_no_op() {
        let mut ui = CraftingUi::new();
        let mut slots = empty_slots();
        let ok = ui.click_armour_slot(ArmSlot::Leggings as usize, &mut slots);
        // Returning true here is harmless — the slot was empty so we
        // "picked up" nothing — assert state is unchanged.
        assert!(ok);
        assert!(ui.cursor_item.is_none());
        assert!(slots[ArmSlot::Leggings as usize].is_none());
    }

    #[test]
    fn invalid_armour_slot_index_returns_false() {
        let mut ui = CraftingUi::new();
        let mut slots = empty_slots();
        assert!(!ui.click_armour_slot(99, &mut slots));
    }

    // --- Crafting item-loss regressions (engine audit 2026-06-04, A) ---
    use crate::block;
    use crate::crafting::{Tool, ToolMaterial, ToolType};

    fn grid_cursor_count(ui: &CraftingUi, b: crate::block::BlockId) -> u32 {
        let mut n = 0u32;
        for r in 0..3 {
            for c in 0..3 {
                if let Some(s) = &ui.grid[r][c] {
                    if matches!(s.item, Item::Block(x) if x == b) { n += s.count as u32; }
                }
            }
        }
        if let Some(s) = &ui.cursor_item {
            if matches!(s.item, Item::Block(x) if x == b) { n += s.count as u32; }
        }
        n
    }

    #[test]
    fn multi_count_cursor_onto_occupied_grid_cell_loses_nothing() {
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 1));
        ui.cursor_item = Some(ItemStack::new_block(block::DIRT, 10));
        // Old behaviour placed 1 dirt and put the displaced stone on the cursor,
        // silently deleting the other 9 dirt.
        ui.click_grid_slot(0, 0, &mut Inventory::new(), false);
        assert_eq!(grid_cursor_count(&ui, block::DIRT), 10, "no dirt deleted");
        assert_eq!(grid_cursor_count(&ui, block::STONE), 1, "no stone deleted");
    }

    #[test]
    fn single_count_cursor_still_swaps_into_occupied_cell() {
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 1));
        ui.cursor_item = Some(ItemStack::new_block(block::DIRT, 1));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), false);
        // Swap: dirt into the cell, stone onto the cursor.
        assert!(matches!(ui.grid[0][0].as_ref().unwrap().item, Item::Block(b) if b == block::DIRT));
        assert!(matches!(ui.cursor_item.as_ref().unwrap().item, Item::Block(b) if b == block::STONE));
    }

    #[test]
    fn right_click_tops_up_matching_grid_cell_by_one() {
        // Minecraft: right-click adds one to a cell already holding the same
        // item; the cursor keeps the remainder. This is the click-to-stack /
        // batch-crafting path (left-click would dump the whole stack — see
        // left_click_merges_whole_stack_into_matching_grid_cell).
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 1));
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), true);
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(2), "cell topped up to 2");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(4), "cursor keeps the other 4");
    }

    // ── Minecraft left/right-click semantics (2026-06-07) ──────────────────
    // Left = whole stack, Right = single item / half on pickup.

    #[test]
    fn left_click_drops_whole_stack_into_empty_grid_cell() {
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), false);
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(5), "whole stack dropped");
        assert!(ui.cursor_item.is_none(), "cursor emptied");
    }

    #[test]
    fn right_click_drops_one_into_empty_grid_cell() {
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), true);
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(1), "one dropped");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(4), "cursor keeps 4");
    }

    #[test]
    fn left_click_merges_whole_stack_into_matching_grid_cell_up_to_max() {
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 60));
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 10));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), false);
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(64), "merged up to the 64 cap");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(6), "remainder stays on cursor");
    }

    #[test]
    fn right_click_empty_cursor_picks_up_half_from_grid_cell() {
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), true);
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(3), "picked up the ceil-half");
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(2), "left the floor-half");
    }

    #[test]
    fn left_click_empty_cursor_picks_up_whole_grid_cell() {
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), false);
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(5), "picked up the whole stack");
        assert!(ui.grid[0][0].is_none(), "cell emptied");
    }

    #[test]
    fn right_click_holding_places_one_into_empty_inventory_slot() {
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_inventory_slot(9, &mut inv, true);
        assert_eq!(inv.slot(9).map(|s| s.count), Some(1), "one placed");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(4), "cursor keeps 4");
    }

    #[test]
    fn right_click_holding_adds_one_to_matching_inventory_slot() {
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        inv.set_slot(9, Some(ItemStack::new_block(block::STONE, 2)));
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_inventory_slot(9, &mut inv, true);
        assert_eq!(inv.slot(9).map(|s| s.count), Some(3), "slot got one more");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(4), "cursor keeps 4");
    }

    #[test]
    fn right_click_empty_cursor_picks_up_half_from_inventory_slot() {
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        inv.set_slot(9, Some(ItemStack::new_block(block::STONE, 5)));
        ui.click_inventory_slot(9, &mut inv, true);
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(3), "picked up the ceil-half");
        assert_eq!(inv.slot(9).map(|s| s.count), Some(2), "left the floor-half");
    }

    #[test]
    fn left_click_holding_places_whole_stack_into_empty_inventory_slot() {
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_inventory_slot(9, &mut inv, false);
        assert_eq!(inv.slot(9).map(|s| s.count), Some(5), "whole stack placed");
        assert!(ui.cursor_item.is_none(), "cursor emptied");
    }

    #[test]
    fn same_item_single_cursor_into_occupied_grid_cell_stacks_not_swaps() {
        // A 1-count cursor of the SAME item used to swap (a visual no-op that
        // left the item on the cursor). It should now add to the cell so you
        // can click-to-stack one at a time. (Different-item swap still works —
        // see single_count_cursor_still_swaps_into_occupied_cell.)
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 1));
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 1));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), false);
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(2), "stacked to 2");
        assert!(ui.cursor_item.is_none(), "cursor emptied");
    }

    #[test]
    fn stacking_into_full_grid_cell_is_a_no_op_and_loses_nothing() {
        // A full cell (== max_stack) of the same item can't take more: the
        // click is a safe no-op (no swap that would strand the cursor stack).
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 64));
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        ui.click_grid_slot(0, 0, &mut Inventory::new(), false);
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(64), "full cell unchanged");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(5), "cursor keeps all 5");
    }

    #[test]
    fn drag_distribute_drops_one_per_slot_and_conserves_count() {
        // #45 P3 RMB-drag — a carried stack of 5 painted across 3 empty slots
        // leaves one in each + 2 on the cursor; total is conserved.
        let mut ui = CraftingUi::new();
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 5));
        let mut inv = Inventory::new();
        for s in [9usize, 10, 11] {
            assert!(ui.drag_distribute_into_inventory(s, &mut inv));
        }
        for s in [9, 10, 11] {
            assert_eq!(inv.slot(s).map(|x| x.count), Some(1), "one stone in slot {s}");
        }
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(2), "2 left on cursor");
        // Total stone conserved (3 placed + 2 carried = 5).
        let in_slots: u32 = (9..12).filter_map(|s| inv.slot(s)).map(|x| x.count as u32).sum();
        assert_eq!(in_slots + 2, 5);
        // A different item already in a slot is skipped (no overwrite, no loss).
        inv.set_slot(12, Some(ItemStack::new_block(block::DIRT, 1)));
        assert!(!ui.drag_distribute_into_inventory(12, &mut inv));
        assert_eq!(inv.slot(12).map(|x| x.count), Some(1));
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(2), "cursor untouched on skip");
    }

    #[test]
    fn drag_gather_pulls_matching_into_cursor_and_conserves_count() {
        // #45 P3 LMB-drag — gather scattered stone into the cursor.
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        inv.set_slot(9, Some(ItemStack::new_block(block::STONE, 3)));
        inv.set_slot(10, Some(ItemStack::new_block(block::STONE, 4)));
        inv.set_slot(11, Some(ItemStack::new_block(block::DIRT, 9)));
        // Empty cursor adopts the first painted slot's item, then collects.
        assert!(ui.drag_gather_from_inventory(9, &mut inv));
        assert!(ui.drag_gather_from_inventory(10, &mut inv));
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(7), "3+4 gathered");
        assert!(inv.slot(9).is_none() && inv.slot(10).is_none());
        // A non-matching slot is skipped (dirt stays put).
        assert!(!ui.drag_gather_from_inventory(11, &mut inv));
        assert_eq!(inv.slot(11).map(|x| x.count), Some(9));
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(7), "cursor unchanged on skip");
    }

    #[test]
    fn close_with_full_inventory_keeps_items_and_stays_open() {
        let mut ui = CraftingUi::new();
        ui.open = true;
        ui.is_table = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 5));
        ui.cursor_item = Some(ItemStack::new_block(block::DIRT, 3));
        let mut inv = Inventory::new();
        for i in 0..36 {
            inv.set_slot(i, Some(ItemStack::new_tool(
                Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        let closed = ui.close(&mut inv);
        assert!(!closed, "can't close into a full inventory");
        assert!(ui.open, "UI stays open so the items aren't stranded");
        assert_eq!(ui.grid[0][0].as_ref().map(|s| s.count), Some(5), "grid item preserved");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(3), "cursor item preserved");
    }

    #[test]
    fn close_with_room_returns_items_and_closes() {
        let mut ui = CraftingUi::new();
        ui.open = true;
        ui.grid[0][0] = Some(ItemStack::new_block(block::STONE, 5));
        ui.cursor_item = Some(ItemStack::new_block(block::DIRT, 3));
        let mut inv = Inventory::new();
        let closed = ui.close(&mut inv);
        assert!(closed);
        assert!(!ui.open);
        assert!(ui.grid[0][0].is_none() && ui.cursor_item.is_none());
        let total: u32 = inv.slots_iter().flatten().map(|s| s.count as u32).sum();
        assert_eq!(total, 8, "all items landed in the inventory");
    }

    #[test]
    fn click_inventory_slot_does_not_overflow_on_overfull_slot() {
        // A slot holding more than max_stack (legacy save, or the old chest
        // 255-cap bug) merged with a cursor used `existing + cursor` on u8 —
        // 250 + 50 overflows and panics in debug (engine audit A). Must be a
        // safe no-op: no room, cursor unchanged.
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        let mut overfull = ItemStack::new_block(block::STONE, 64);
        overfull.count = 250;
        inv.set_slot(9, Some(overfull));
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 50));
        ui.click_inventory_slot(9, &mut inv, false);
        assert_eq!(inv.slot(9).map(|s| s.count), Some(250), "over-max slot unchanged");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(50), "cursor keeps its stack");
    }

    #[test]
    fn click_inventory_slot_merge_caps_at_max_and_keeps_remainder() {
        let mut ui = CraftingUi::new();
        let mut inv = Inventory::new();
        inv.set_slot(9, Some(ItemStack::new_block(block::STONE, 60)));
        ui.cursor_item = Some(ItemStack::new_block(block::STONE, 20));
        ui.click_inventory_slot(9, &mut inv, false);
        assert_eq!(inv.slot(9).map(|s| s.count), Some(64), "merged up to the 64 cap");
        assert_eq!(ui.cursor_item.as_ref().map(|s| s.count), Some(16), "remainder stays on cursor");
    }

    // ── Controller slot-cursor: pad_move / pad_activate ─────────────────
    use super::{pad_activate, pad_move, PadDir, PadSlot};
    use PadDir::{Down, Left, Right, Up};
    use PadSlot::{Armour, Grid, Inv, Result as ResultSlot};

    #[test]
    fn armour_column_up_down_clamps_at_helmet() {
        assert_eq!(pad_move(Armour(0), Up, true), Armour(0));
        assert_eq!(pad_move(Armour(0), Down, true), Armour(1));
        assert_eq!(pad_move(Armour(2), Down, true), Armour(3));
        assert_eq!(pad_move(Armour(0), Left, true), Armour(0), "left edge clamps");
    }

    #[test]
    fn armour_down_from_boots_enters_main_inventory() {
        assert_eq!(pad_move(Armour(3), Down, true), Inv(9));
        assert_eq!(pad_move(Armour(3), Down, false), Inv(9));
    }

    #[test]
    fn armour_right_enters_grid_row_clamped() {
        assert_eq!(pad_move(Armour(0), Right, true), Grid(0, 0));
        assert_eq!(pad_move(Armour(2), Right, true), Grid(2, 0));
        // 2×2 grid: boots row clamps to grid row 1.
        assert_eq!(pad_move(Armour(3), Right, false), Grid(1, 0));
    }

    #[test]
    fn grid_right_edge_reaches_result() {
        assert_eq!(pad_move(Grid(1, 2), Right, true), ResultSlot);
        assert_eq!(pad_move(Grid(0, 1), Right, false), ResultSlot, "2×2 right edge is col 1");
        assert_eq!(pad_move(Grid(0, 0), Right, true), Grid(0, 1), "interior moves stay in grid");
    }

    #[test]
    fn grid_left_edge_returns_to_armour() {
        assert_eq!(pad_move(Grid(0, 0), Left, true), Armour(0));
        assert_eq!(pad_move(Grid(2, 0), Left, true), Armour(2));
        assert_eq!(pad_move(Grid(1, 1), Left, true), Grid(1, 0));
    }

    #[test]
    fn grid_down_from_bottom_row_lands_in_main_inventory_column_mapped() {
        // Grid col c maps to inventory col 3+c (grid sits centre-left above).
        assert_eq!(pad_move(Grid(2, 0), Down, true), Inv(9 + 3));
        assert_eq!(pad_move(Grid(2, 2), Down, true), Inv(9 + 5));
        assert_eq!(pad_move(Grid(1, 1), Down, false), Inv(9 + 4), "2×2 bottom row is row 1");
        assert_eq!(pad_move(Grid(0, 0), Down, true), Grid(1, 0), "interior moves stay in grid");
    }

    #[test]
    fn result_left_returns_to_grid_and_down_to_inventory() {
        assert_eq!(pad_move(ResultSlot, Left, true), Grid(1, 2));
        assert_eq!(pad_move(ResultSlot, Left, false), Grid(0, 1));
        assert_eq!(pad_move(ResultSlot, Down, true), Inv(9 + 8));
        assert_eq!(pad_move(ResultSlot, Up, true), ResultSlot, "up/right clamp");
        assert_eq!(pad_move(ResultSlot, Right, true), ResultSlot);
    }

    #[test]
    fn main_rows_move_up_down_within_inventory() {
        assert_eq!(pad_move(Inv(9 + 4), Down, true), Inv(9 + 13));
        assert_eq!(pad_move(Inv(9 + 13), Down, true), Inv(9 + 22));
        assert_eq!(pad_move(Inv(9 + 22), Up, true), Inv(9 + 13));
        assert_eq!(pad_move(Inv(9 + 13), Up, true), Inv(9 + 4));
    }

    #[test]
    fn main_row2_down_reaches_hotbar_same_column() {
        assert_eq!(pad_move(Inv(9 + 18 + 4), Down, true), Inv(4));
        assert_eq!(pad_move(Inv(9 + 18), Down, true), Inv(0));
        assert_eq!(pad_move(Inv(9 + 18 + 8), Down, true), Inv(8));
    }

    #[test]
    fn hotbar_up_returns_to_main_row2_same_column() {
        assert_eq!(pad_move(Inv(4), Up, true), Inv(9 + 18 + 4));
        assert_eq!(pad_move(Inv(0), Up, true), Inv(9 + 18));
    }

    #[test]
    fn hotbar_left_right_clamp_and_down_stays() {
        assert_eq!(pad_move(Inv(0), Left, true), Inv(0));
        assert_eq!(pad_move(Inv(8), Right, true), Inv(8));
        assert_eq!(pad_move(Inv(3), Right, true), Inv(4));
        assert_eq!(pad_move(Inv(3), Down, true), Inv(3));
    }

    #[test]
    fn main_row0_up_routes_to_armour_grid_or_result_by_column() {
        // Left edge (cols 0-1) → armour column.
        assert_eq!(pad_move(Inv(9), Up, true), Armour(3));
        assert_eq!(pad_move(Inv(9 + 1), Up, true), Armour(3));
        // Middle (cols 2-6) → grid bottom row, column-mapped + clamped.
        assert_eq!(pad_move(Inv(9 + 2), Up, true), Grid(2, 0));
        assert_eq!(pad_move(Inv(9 + 4), Up, true), Grid(2, 1));
        assert_eq!(pad_move(Inv(9 + 6), Up, true), Grid(2, 2));
        assert_eq!(pad_move(Inv(9 + 6), Up, false), Grid(1, 1), "2×2 clamps col");
        // Right edge (cols 7-8) → result.
        assert_eq!(pad_move(Inv(9 + 7), Up, true), ResultSlot);
        assert_eq!(pad_move(Inv(9 + 8), Up, true), ResultSlot);
    }

    #[test]
    fn main_rows_left_right_clamp_at_row_edges() {
        assert_eq!(pad_move(Inv(9), Left, true), Inv(9), "row 0 left edge clamps");
        assert_eq!(pad_move(Inv(9 + 8), Right, true), Inv(9 + 8), "row 0 right edge clamps");
        assert_eq!(pad_move(Inv(9 + 9), Left, true), Inv(9 + 9), "row 1 left edge clamps (no wrap)");
    }

    #[test]
    fn every_move_stays_in_bounds() {
        // Exhaustive sweep: from every reachable slot, every direction, both
        // grid sizes — the cursor must always land on a valid slot.
        let mut all: Vec<PadSlot> = Vec::new();
        for i in 0..4 {
            all.push(Armour(i));
        }
        for r in 0..3 {
            for c in 0..3 {
                all.push(Grid(r, c));
            }
        }
        all.push(ResultSlot);
        for i in 0..36 {
            all.push(Inv(i));
        }
        let valid = |s: PadSlot, g: usize| match s {
            Armour(i) => i < 4,
            Grid(r, c) => r < g && c < g,
            ResultSlot => true,
            Inv(i) => i < 36,
        };
        for &is_table in &[false, true] {
            let g = if is_table { 3 } else { 2 };
            for &slot in &all {
                // Skip start positions that aren't valid for this grid size.
                if !valid(slot, 3) {
                    continue;
                }
                for &dir in &[Up, Down, Left, Right] {
                    let next = pad_move(slot, dir, is_table);
                    assert!(
                        valid(next, g.max(if valid(slot, g) { g } else { 3 })),
                        "pad_move({slot:?}, {dir:?}, is_table={is_table}) → {next:?} out of bounds"
                    );
                }
            }
        }
    }

    #[test]
    fn pad_activate_maps_to_click_targets() {
        assert_eq!(pad_activate(Grid(1, 2), false), ClickTarget::GridSlot(1, 2, false));
        assert_eq!(pad_activate(Grid(0, 0), true), ClickTarget::GridSlot(0, 0, true));
        assert_eq!(pad_activate(Inv(13), false), ClickTarget::InventorySlot(13, false));
        assert_eq!(pad_activate(Inv(5), true), ClickTarget::InventorySlot(5, true));
        assert_eq!(pad_activate(ResultSlot, false), ClickTarget::ResultSlot);
        assert_eq!(pad_activate(ResultSlot, true), ClickTarget::ResultSlot, "no right-click semantics on result");
        assert_eq!(pad_activate(Armour(2), false), ClickTarget::ArmourSlot(2));
        assert_eq!(pad_activate(Armour(2), true), ClickTarget::ArmourSlot(2));
    }

    #[test]
    fn pad_focus_resets_on_open_and_close() {
        let mut ui = CraftingUi::new();
        assert_eq!(ui.pad_focus, None);
        ui.pad_focus = Some(PadSlot::start());
        ui.open_player_crafting();
        assert_eq!(ui.pad_focus, None, "opening resets the pad cursor");
        ui.pad_focus = Some(Grid(0, 0));
        let mut inv = Inventory::new();
        assert!(ui.close(&mut inv));
        assert_eq!(ui.pad_focus, None, "closing resets the pad cursor");
    }

    #[test]
    fn pad_start_is_hotbar_slot_zero() {
        assert_eq!(PadSlot::start(), Inv(0));
    }

    // ── Recipe-book autofill (2026-06-12) ──────────────────────────────────
    use crate::crafting_catalogue;

    /// Find a catalogue card by name (tests rely on stable generated names).
    fn card_named(name: &str) -> &'static crafting_catalogue::RecipeCard {
        crafting_catalogue::all_cards()
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no card named {name}"))
    }

    #[test]
    fn autofill_lays_the_grid_and_produces_the_cards_output() {
        // Iron Pickaxe: 3 IronIngot + 2 Stick. Stock exactly that.
        let card = card_named("Iron Pickaxe");
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(crate::item::MaterialId::IronIngot, 3)));
        inv.set_slot(1, Some(ItemStack::new_material(crate::item::MaterialId::Stick, 2)));

        assert!(ui.autofill_from_example(&card.example_grid, &mut inv));
        // Grid now equals the example → the live matcher yields the output.
        let mut craft_grid = [[CraftSlot::Empty; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                if let Some(s) = &ui.grid[r][c] {
                    craft_grid[r][c] = CraftSlot::from_item(&s.item);
                }
            }
        }
        assert_eq!(
            format!("{:?}", crate::crafting::match_recipe(&craft_grid)),
            format!("{:?}", Some(card.output.clone())),
            "autofilled grid must produce the card output",
        );
        // Ingredients consumed.
        assert_eq!(inv.count_material(crate::item::MaterialId::IronIngot), 0);
        assert_eq!(inv.count_material(crate::item::MaterialId::Stick), 0);
    }

    #[test]
    fn autofill_fails_and_takes_nothing_when_short() {
        let card = card_named("Iron Pickaxe");
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        let mut inv = Inventory::new();
        // Only 1 iron — not enough (needs 3).
        inv.set_slot(0, Some(ItemStack::new_material(crate::item::MaterialId::IronIngot, 1)));
        inv.set_slot(1, Some(ItemStack::new_material(crate::item::MaterialId::Stick, 2)));

        assert!(!ui.autofill_from_example(&card.example_grid, &mut inv));
        // Nothing taken — items still in inventory, grid empty.
        assert_eq!(inv.count_material(crate::item::MaterialId::IronIngot), 1);
        assert_eq!(inv.count_material(crate::item::MaterialId::Stick), 2);
        assert!(ui.grid.iter().flatten().all(|c| c.is_none()), "grid must be empty on failure");
    }

    #[test]
    fn autofill_returns_prior_grid_contents_to_inventory() {
        let card = card_named("Crafting Table"); // 4 oak planks
        let mut ui = CraftingUi::new();
        ui.is_table = true;
        // Pre-existing junk in the grid that must be returned, not lost.
        ui.grid[2][2] = Some(ItemStack::new_block(crate::block::DIRT, 5));
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::OAK_PLANKS, 4)));

        assert!(ui.autofill_from_example(&card.example_grid, &mut inv));
        // The 5 dirt came back into the inventory.
        let dirt: u32 = (0..36)
            .filter_map(|i| inv.slot(i))
            .filter(|s| matches!(s.item, Item::Block(b) if b == crate::block::DIRT))
            .map(|s| s.count as u32)
            .sum();
        assert_eq!(dirt, 5, "prior grid contents must return to inventory");
    }
}
