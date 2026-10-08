//! Historical Pivot Sub-Foundation 2 (HP-2) — Chest UI.
//!
//! 27-slot egui dialog opened on right-click. Click moves stacks between
//! the player's inventory and the chest; shift-click bulk-transfers (one
//! direction at a time per click) per the Spec 29 furnace UX. Close on
//! Escape clears `PlayerSlot.open_chest`.
//!
//! C3b-1 — the dialog only draws and reports what was clicked, as
//! `container_window::ContainerClick`s; the caller applies each through the
//! one shared rule (`container_window::apply_container`, which calls the
//! pure functions below), on the real chest or on a joiner's mirror of the
//! server's.

use egui::{Color32, RichText};

use crate::chest::ChestData;
use crate::container_window::ContainerClick;
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};

const SLOT_SIZE: f32 = 36.0;

/// Side-effect output of a single dialog render. Caller applies these
/// to `PlayerSlot.open_chest` after the egui pass.
#[derive(Clone, Debug, Default)]
pub struct ChestUiResult {
    pub close_requested: bool,
    /// C3b-1 — what the player clicked this frame, in order: the caller
    /// applies each (`container_window::apply_container`).
    pub clicks: Vec<ContainerClick>,
    /// Task 20 (2026-07-07) — the donkey/mule pack variant only. Set when
    /// the player clicks the in-dialog "Remove pack" button (enabled only
    /// when the pack is empty). The caller (game_loop pack UI branch)
    /// unequips the pack + returns the chest. Always `false` for real
    /// chests/dispensers, which never render the button.
    pub remove_pack_requested: bool,
}

/// Render the chest dialog over `chest` and the player's `inventory`; the
/// clicks come back in the result. Pos is purely informational (window title).
pub fn show_chest_dialog(
    ctx: &egui::Context,
    chest: &ChestData,
    inventory: &Inventory,
    pos: (i32, i32, i32),
) -> ChestUiResult {
    // #15 — tier drives the title; the grid + interactions are shared with
    // every other ChestData-backed container (dispenser/dropper).
    let title = chest.tier.display_name();
    show_container_dialog(ctx, title, chest, inventory, pos)
}

/// Whether a container dialog should render the donkey/mule-pack "Remove
/// pack" button, and if so whether it's enabled. `None` = not a pack (no
/// button at all — chests/dispensers). `Some(true/false)` = a pack, button
/// enabled when the pack is empty. Split out so the enabled decision is a
/// pure, testable function rather than buried in the egui closure.
pub fn pack_removal_enabled(chest: &ChestData) -> bool {
    chest.slots.iter().all(|s| s.is_none())
}

/// The container dialog engine: any `ChestData`-backed inventory (chest
/// tiers, dispenser, dropper) with a caller-supplied title. Rows derive from
/// the slot count (2026-07-04 — extracted from `show_chest_dialog` so the
/// dispenser's 9-slot dialog reuses the move/sort/dump/restock machinery).
pub fn show_container_dialog(
    ctx: &egui::Context,
    title: &str,
    chest: &ChestData,
    inventory: &Inventory,
    pos: (i32, i32, i32),
) -> ChestUiResult {
    show_container_dialog_ext(ctx, title, chest, inventory, pos, false)
}

/// As [`show_container_dialog`], plus `show_remove_pack`: when `true`, the
/// dialog renders a donkey/mule-only "Remove pack" button (Task 20). Every
/// non-pack caller (chest tiers, dispenser, dropper) goes through the
/// 5-arg wrapper above with the flag `false`, so real containers never
/// grow the button.
pub fn show_container_dialog_ext(
    ctx: &egui::Context,
    title: &str,
    chest: &ChestData,
    inventory: &Inventory,
    pos: (i32, i32, i32),
    show_remove_pack: bool,
) -> ChestUiResult {
    let mut result = ChestUiResult::default();
    let mut window_open = true;
    let tier_name = title;
    let tier_rows = chest.slots.len().div_ceil(9);

    egui::Window::new(format!("{} ({}, {}, {})", tier_name, pos.0, pos.1, pos.2))
        .open(&mut window_open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(RichText::new(tier_name).size(16.0).strong());
            ui.add_space(4.0);
            ui.label("Click moves one. Shift-click moves the whole stack. Scroll over a slot to transfer.");
            ui.add_space(8.0);

            ui.label(RichText::new(format!("{tier_name} — {} slots", chest.slots.len())).strong());
            for row in 0..tier_rows {
                ui.horizontal(|ui| {
                    for col in 0..9 {
                        let idx = row * 9 + col;
                        if idx >= chest.slots.len() { continue; }
                        let label = slot_label(chest.slots.get(idx).and_then(|s| s.as_ref()));
                        let shift = ui.input(|i| i.modifiers.shift);
                        let resp = ui.add_sized([SLOT_SIZE, SLOT_SIZE], egui::Button::new(label));
                        if resp.clicked() {
                            result.clicks.push(ContainerClick::Withdraw { slot: idx, all: shift });
                        }
                        // #45 P3 — scroll over a chest slot pulls one item out to
                        // the inventory per tick (shift = whole stack).
                        if resp.hovered() {
                            for _ in 0..scroll_ticks(ui) {
                                result.clicks.push(ContainerClick::Withdraw { slot: idx, all: shift });
                            }
                        }
                    }
                });
            }

            ui.add_space(8.0);
            ui.label(RichText::new("Inventory").strong());
            for row in 0..4 {
                ui.horizontal(|ui| {
                    for col in 0..9 {
                        let idx = row * 9 + col;
                        if idx >= 36 { continue; }
                        let label = slot_label(inventory.slot(idx));
                        let shift = ui.input(|i| i.modifiers.shift);
                        let resp = ui.add_sized([SLOT_SIZE, SLOT_SIZE], egui::Button::new(label));
                        if resp.clicked() {
                            result.clicks.push(ContainerClick::Deposit { slot: idx, all: shift });
                        }
                        // #45 P3 — scroll over an inventory slot stashes one item
                        // into the chest per tick (shift = whole stack).
                        if resp.hovered() {
                            for _ in 0..scroll_ticks(ui) {
                                result.clicks.push(ContainerClick::Deposit { slot: idx, all: shift });
                            }
                        }
                    }
                });
            }

            // #45 P1/P2 — container QoL buttons (mouse + touch + gamepad-cursor
            // reachable, since they're ordinary egui buttons in the dialog).
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Sort").clicked() {
                    result.clicks.push(ContainerClick::Sort);
                }
                if ui.button("Dump matching").clicked() {
                    result.clicks.push(ContainerClick::DumpMatching);
                }
                if ui.button("Restock").clicked() {
                    result.clicks.push(ContainerClick::Restock);
                }
                if ui.button("Take all").clicked() {
                    result.clicks.push(ContainerClick::TakeAll);
                }
            });

            // Task 20 (2026-07-07) — donkey/mule pack only. Discoverable
            // in-dialog unequip: enabled only when the pack is empty (so
            // filling still works — the old sneak-gesture idea unequipped a
            // freshly-equipped, empty-by-construction pack on the first
            // click). Disabled + hover-hinted otherwise so the player learns
            // to empty it first.
            if show_remove_pack {
                ui.add_space(4.0);
                let enabled = pack_removal_enabled(chest);
                let btn = ui.add_enabled(enabled, egui::Button::new("Remove pack"));
                let btn = btn.on_disabled_hover_text("Empty the pack first.");
                if btn.clicked() {
                    result.remove_pack_requested = true;
                    result.close_requested = true;
                }
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    result.close_requested = true;
                }
            });
        });

    if !window_open {
        result.close_requested = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result.close_requested = true;
    }
    result
}

/// Withdraw from chest slot `idx` into the inventory. `shift` takes the whole
/// stack; otherwise one item. Returns true if anything moved.
///
/// The recovery path re-inserts **only the genuinely-unplaced remainder** of
/// the withdrawal. `Inventory::add_item` is non-atomic — it places what fits
/// and hands back the rest — so re-inserting the *whole* withdrawn stack (the
/// old behaviour) duplicated whatever had already reached the inventory.
pub fn withdraw_chest_slot(
    chest: &mut ChestData,
    idx: usize,
    inventory: &mut Inventory,
    shift: bool,
) -> bool {
    let taken = chest.slots.get_mut(idx).and_then(|s| s.take());
    let Some(stack) = taken else { return false; };
    let (to_inv, leftover_in_chest) = split_for_click(stack, shift);
    if let Some(slot) = chest.slots.get_mut(idx) {
        *slot = leftover_in_chest;
    }
    let want = to_inv.count;
    match inventory.add_item(to_inv) {
        None => true, // everything fit
        Some(remainder) => {
            // Re-insert ONLY what didn't land — never the whole withdrawn
            // stack (that was the dupe). If some units did land, that's still
            // a mutation worth flagging.
            let placed_some = remainder.count < want;
            reinsert_into_chest(chest, idx, remainder);
            placed_some
        }
    }
}

/// Deposit from inventory slot `idx` into the chest. `shift` moves the whole
/// stack; otherwise one item. Returns true if anything moved. Mirrors
/// `withdraw_chest_slot`'s non-atomic recovery: only the genuinely-unplaced
/// remainder goes back to the inventory, so the count is conserved (no dupe).
pub fn deposit_to_chest(
    inventory: &mut Inventory,
    idx: usize,
    chest: &mut ChestData,
    shift: bool,
) -> bool {
    let Some(stack) = inventory.take_slot(idx) else { return false; };
    let (to_chest, leftover_in_inv) = split_for_click(stack, shift);
    if let Some(s) = leftover_in_inv {
        inventory.set_slot(idx, Some(s));
    }
    let want = to_chest.count;
    let leftover = chest.try_insert(to_chest);
    if leftover.count == 0 {
        return true;
    }
    let placed_some = leftover.count < want;
    // Put back only what didn't land — into the original slot if free, else
    // anywhere it fits (never destroyed).
    if inventory.slot(idx).is_none() {
        inventory.set_slot(idx, Some(leftover));
    } else {
        let _ = inventory.add_item(leftover);
    }
    placed_some
}

/// #45 P3 — how many one-item transfers this frame's scroll over a slot should
/// drive. Capped at one per frame so a single flick never dumps a stack; the
/// player scrolls repeatedly to move more.
fn scroll_ticks(ui: &egui::Ui) -> u32 {
    let d = ui.input(|i| i.smooth_scroll_delta.y);
    if d.abs() < 0.5 {
        0
    } else {
        1
    }
}

/// #45 P1 — sort the chest's contents (merge partial stacks, deterministic
/// order). Chests have no locked slots, so the mask is all-false.
pub fn sort_chest(chest: &mut ChestData) {
    let no_locks = vec![false; chest.slots.len()];
    chest.slots = crate::inventory::sort_slots(&chest.slots, &no_locks);
}

/// #45 P2 — "Dump matching": move every (unlocked) player stack whose item the
/// chest **already holds** into the chest. Returns true if anything moved. Count
/// is conserved — only the genuinely-unplaced remainder stays in the inventory.
pub fn dump_matching(inventory: &mut Inventory, chest: &mut ChestData) -> bool {
    let mut moved = false;
    for i in 0..36 {
        if inventory.is_locked(i) {
            continue;
        }
        let Some(stack) = inventory.slot(i).cloned() else { continue };
        let chest_holds = chest
            .slots
            .iter()
            .flatten()
            .any(|c| c.item.can_stack_with(&stack.item));
        if !chest_holds {
            continue;
        }
        let taken = inventory.take_slot(i).expect("slot was Some");
        let want = taken.count;
        let leftover = chest.try_insert(taken);
        if leftover.count < want {
            moved = true;
        }
        if leftover.count > 0 {
            // The slot was emptied by take_slot; put the remainder back.
            inventory.set_slot(i, Some(leftover));
        }
    }
    moved
}

/// #45 P2 — "Restock": pull from the chest back into the inventory every chest
/// stack whose item the player **already holds**. Count-conserving.
pub fn restock_from_chest(inventory: &mut Inventory, chest: &mut ChestData) -> bool {
    let mut moved = false;
    for ci in 0..chest.slots.len() {
        let Some(stack) = chest.slots[ci].clone() else { continue };
        let player_holds = (0..36)
            .any(|i| inventory.slot(i).is_some_and(|s| s.item.can_stack_with(&stack.item)));
        if !player_holds {
            continue;
        }
        let taken = chest.slots[ci].take().expect("slot was Some");
        let want = taken.count;
        match inventory.add_item(taken) {
            None => moved = true,
            Some(remainder) => {
                if remainder.count < want {
                    moved = true;
                }
                reinsert_into_chest(chest, ci, remainder);
            }
        }
    }
    moved
}

/// #45 P2 — "Take all": move the entire chest into the inventory, best-effort.
/// Anything that doesn't fit stays in the chest (count-conserving).
#[cfg(test)]
pub fn take_all(inventory: &mut Inventory, chest: &mut ChestData) -> bool {
    take_all_where(inventory, chest, |_| true)
}

/// [`take_all`] of the stacks `take` accepts; the rest stay where they are
/// (C3b-1: a shared container's Take all leaves a Plan in place).
pub fn take_all_where(inventory: &mut Inventory, chest: &mut ChestData, take: impl Fn(&ItemStack) -> bool) -> bool {
    let mut moved = false;
    for ci in 0..chest.slots.len() {
        if !chest.slots[ci].as_ref().is_some_and(&take) {
            continue;
        }
        let Some(taken) = chest.slots[ci].take() else { continue };
        let want = taken.count;
        match inventory.add_item(taken) {
            None => moved = true,
            Some(remainder) => {
                if remainder.count < want {
                    moved = true;
                }
                reinsert_into_chest(chest, ci, remainder);
            }
        }
    }
    moved
}

/// Put `stack` back: merge into chest slot `idx` if it still holds a
/// compatible stack, drop into it if empty, else spill elsewhere.
fn reinsert_into_chest(chest: &mut ChestData, idx: usize, stack: ItemStack) {
    match chest.slots.get_mut(idx) {
        Some(Some(existing)) if existing.item.can_stack_with(&stack.item) => {
            existing.count = existing.count.saturating_add(stack.count);
        }
        Some(slot @ None) => {
            *slot = Some(stack);
        }
        _ => {
            let _ = chest.try_insert(stack);
        }
    }
}

/// Split a stack for a click — `shift=true` takes the whole stack and
/// leaves no remainder; `shift=false` takes one and leaves the rest.
fn split_for_click(stack: ItemStack, shift: bool) -> (ItemStack, Option<ItemStack>) {
    if shift || stack.count <= 1 {
        return (stack, None);
    }
    let mut one = stack.clone();
    one.count = 1;
    let mut rest = stack;
    rest.count -= 1;
    (one, Some(rest))
}

fn slot_label(stack: Option<&ItemStack>) -> RichText {
    match stack {
        None => RichText::new(" ").color(Color32::DARK_GRAY),
        Some(s) => {
            let name = match &s.item {
                Item::Block(_) => "blk",
                Item::Material(m) => material_short(*m),
                Item::Tool(_) => "tool",
                Item::Plan(_) => "plan",
                Item::Armour(_) => "arm",
            };
            RichText::new(format!("{name}\nx{}", s.count)).size(10.0)
        }
    }
}

fn material_short(m: crate::item::MaterialId) -> &'static str {
    use crate::item::MaterialId as M;
    match m {
        M::Bread => "brd",
        M::Wheat => "wht",
        M::Carrot => "crt",
        M::Potato => "pot",
        M::RawBeef => "bef",
        M::RawPorkchop => "pig",
        M::RawChicken => "chk",
        M::RawMutton => "mtn",
        M::Bone => "bne",
        M::Leather => "lth",
        M::String => "str",
        M::IronIngot => "iro",
        _ => "mat",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ItemStack, MaterialId};

    #[test]
    fn shift_click_takes_full_stack() {
        let stack = ItemStack::new_material(MaterialId::Bread, 5);
        let (taken, remainder) = split_for_click(stack, true);
        assert_eq!(taken.count, 5);
        assert!(remainder.is_none());
    }

    #[test]
    fn regular_click_takes_one_leaves_rest() {
        let stack = ItemStack::new_material(MaterialId::Bread, 5);
        let (taken, remainder) = split_for_click(stack, false);
        assert_eq!(taken.count, 1);
        assert_eq!(remainder.unwrap().count, 4);
    }

    #[test]
    fn click_on_singleton_takes_whole_thing() {
        let stack = ItemStack::new_material(MaterialId::Bread, 1);
        let (taken, remainder) = split_for_click(stack, false);
        assert_eq!(taken.count, 1);
        assert!(remainder.is_none());
    }

    #[test]
    fn pack_removal_enabled_only_when_all_slots_empty() {
        // Task 20 — a freshly-equipped pack is empty (default ChestData), so
        // the "Remove pack" button IS enabled straight away; putting even one
        // item in disables it, so filling the pack works and removal is a
        // deliberate empty-then-click action rather than a first-click trap.
        let mut pack = crate::chest::ChestData::new();
        assert!(
            pack_removal_enabled(&pack),
            "a fresh/empty pack must allow removal"
        );
        pack.slots[7] = Some(ItemStack::new_material(MaterialId::Stick, 1));
        assert!(
            !pack_removal_enabled(&pack),
            "a pack with any item must NOT allow removal (empty it first)"
        );
        pack.slots[7] = None;
        assert!(
            pack_removal_enabled(&pack),
            "clearing the last item re-enables removal"
        );
    }

    use crate::chest::ChestData;
    use crate::inventory::Inventory;
    use crate::block;
    use crate::crafting::{Tool, ToolMaterial, ToolType};

    fn stone_units(slots: impl Iterator<Item = ItemStack>) -> u32 {
        slots
            .filter(|s| matches!(s.item, Item::Block(b) if b == block::STONE))
            .map(|s| s.count as u32)
            .sum()
    }

    #[test]
    fn withdraw_partial_fit_conserves_total_no_dupe() {
        // Chest holds 64 stone. Inventory has room for exactly 4 more (one
        // 60-stack with 4 headroom; every other slot is a non-stacking tool).
        let mut chest = ChestData::new();
        chest.slots[0] = Some(ItemStack::new_block(block::STONE, 64));
        let mut inv = Inventory::new();
        for i in 0..35 {
            inv.set_slot(i, Some(ItemStack::new_tool(
                Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        inv.set_slot(35, Some(ItemStack::new_block(block::STONE, 60)));

        let total_before = 64 + 60;
        withdraw_chest_slot(&mut chest, 0, &mut inv, true); // shift: whole stack

        let inv_stone = stone_units(inv.slots_iter().flatten().cloned());
        let chest_stone = stone_units(chest.slots.iter().flatten().cloned());
        assert_eq!(inv_stone + chest_stone, total_before, "no dupe and no loss");
        assert_eq!(inv_stone, 64, "the 60-stack topped up to its 64 cap");
        assert_eq!(chest_stone, 60, "the unplaced remainder stays in the chest");
    }

    #[test]
    fn deposit_one_and_stack_conserve_total_no_dupe() {
        // #45 P3 — depositing (the scroll-into-chest path) conserves the item
        // count whether moving one or the whole stack.
        let mut chest = ChestData::new();
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 10)));

        assert!(deposit_to_chest(&mut inv, 0, &mut chest, false)); // one item
        let inv_stone = stone_units(inv.slots_iter().flatten().cloned());
        let chest_stone = stone_units(chest.slots.iter().flatten().cloned());
        assert_eq!(inv_stone + chest_stone, 10, "no dupe/loss after one");
        assert_eq!(chest_stone, 1);
        assert_eq!(inv_stone, 9);

        assert!(deposit_to_chest(&mut inv, 0, &mut chest, true)); // whole stack
        let inv_stone = stone_units(inv.slots_iter().flatten().cloned());
        let chest_stone = stone_units(chest.slots.iter().flatten().cloned());
        assert_eq!(inv_stone + chest_stone, 10, "no dupe/loss after stack");
        assert_eq!(inv_stone, 0, "whole stack deposited");
        assert_eq!(chest_stone, 10);
    }

    #[test]
    fn withdraw_into_full_inventory_keeps_everything_in_chest() {
        let mut chest = ChestData::new();
        chest.slots[0] = Some(ItemStack::new_block(block::STONE, 64));
        let mut inv = Inventory::new();
        for i in 0..36 {
            inv.set_slot(i, Some(ItemStack::new_tool(
                Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
        }
        let moved = withdraw_chest_slot(&mut chest, 0, &mut inv, true);
        assert!(!moved, "nothing moves into a full inventory");
        let chest_stone = stone_units(chest.slots.iter().flatten().cloned());
        assert_eq!(chest_stone, 64, "the whole stack is preserved in the chest");
    }

    // ── #45 Phase 2 — quick-stack / dump / restock / take-all ────────

    fn total_stone(chest: &ChestData, inv: &Inventory) -> u32 {
        stone_units(chest.slots.iter().flatten().cloned())
            + stone_units(inv.slots_iter().flatten().cloned())
    }

    #[test]
    fn dump_matching_moves_only_kinds_the_chest_already_holds() {
        // Chest holds stone; inventory has stone + dirt. Dump should move stone
        // (a matching kind) but leave the dirt.
        let mut chest = ChestData::new();
        chest.slots[0] = Some(ItemStack::new_block(block::STONE, 1));
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 20)));
        inv.set_slot(1, Some(ItemStack::new_block(block::DIRT, 20)));
        let before = total_stone(&chest, &inv);
        let moved = dump_matching(&mut inv, &mut chest);
        assert!(moved);
        assert_eq!(total_stone(&chest, &inv), before, "stone conserved");
        // All stone now in chest; dirt untouched in inventory.
        assert_eq!(stone_units(inv.slots_iter().flatten().cloned()), 0, "stone dumped");
        assert!(matches!(inv.slot(1).map(|s| &s.item), Some(Item::Block(b)) if *b == block::DIRT));
    }

    #[test]
    fn dump_matching_skips_locked_slots() {
        let mut chest = ChestData::new();
        chest.slots[0] = Some(ItemStack::new_block(block::STONE, 1));
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 20)));
        inv.toggle_lock(0);
        dump_matching(&mut inv, &mut chest);
        assert_eq!(inv.slot(0).map(|s| s.count), Some(20), "locked stone stays put");
    }

    #[test]
    fn take_all_moves_everything_and_conserves() {
        let mut chest = ChestData::new();
        chest.slots[0] = Some(ItemStack::new_block(block::STONE, 40));
        chest.slots[5] = Some(ItemStack::new_block(block::STONE, 24));
        let mut inv = Inventory::new();
        let before = total_stone(&chest, &inv);
        let moved = take_all(&mut inv, &mut chest);
        assert!(moved);
        assert_eq!(total_stone(&chest, &inv), before, "no loss");
        assert_eq!(stone_units(chest.slots.iter().flatten().cloned()), 0, "chest emptied");
    }

    #[test]
    fn restock_pulls_only_kinds_the_player_already_holds() {
        // Player holds a little stone; chest has stone + dirt. Restock pulls
        // stone back, leaves dirt in the chest.
        let mut chest = ChestData::new();
        chest.slots[0] = Some(ItemStack::new_block(block::STONE, 30));
        chest.slots[1] = Some(ItemStack::new_block(block::DIRT, 30));
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 5)));
        let moved = restock_from_chest(&mut inv, &mut chest);
        assert!(moved);
        assert_eq!(stone_units(inv.slots_iter().flatten().cloned()), 35, "stone pulled back");
        // Dirt stays in chest (player held none).
        let chest_dirt: u32 = chest.slots.iter().flatten()
            .filter(|s| matches!(s.item, Item::Block(b) if b == block::DIRT))
            .map(|s| s.count as u32).sum();
        assert_eq!(chest_dirt, 30, "non-held kind left in chest");
    }

    #[test]
    fn sort_chest_merges_and_conserves() {
        let mut chest = ChestData::new();
        chest.slots[0] = Some(ItemStack::new_block(block::STONE, 30));
        chest.slots[10] = Some(ItemStack::new_block(block::STONE, 30));
        sort_chest(&mut chest);
        assert_eq!(stone_units(chest.slots.iter().flatten().cloned()), 60, "conserved");
        assert_eq!(chest.slots[0].as_ref().map(|s| s.count), Some(60), "merged to front");
    }
}
