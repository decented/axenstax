//! Pick block (Spec 05 §8.2) applied to a live [`GameState`]: the half of the
//! feature that touches the player's inventory. The decisions are the pure
//! [`crate::pick_block`] plan; every change here goes through a path the
//! inventory screen already uses, so a joiner's pick is exactly what the same
//! moves would be by hand:
//!
//! - Survival swap: three left clicks on the inventory window
//!   ([`GameState::apply_window_click`], the entry point the screen's slot
//!   clicks use): pick up the bag stack, click the selected hotbar slot (it
//!   lands there and the old hotbar item rides the cursor), click the bag
//!   slot again (the old item goes home). In Strict that is three reserved
//!   window actions; in Mirror three logged ops.
//! - Creative take: the Inventory Explorer's give. A joiner asks the server
//!   ([`GameState::request_creative_give`], granted by an `InventoryGrant`);
//!   everyone else writes the slot directly, as the Explorer's local branch
//!   does.

use crate::pick_block::{plan_pick_block, PickPlan};
use crate::window::WindowClick;
use crate::GameState;

impl GameState {
    /// Middle-click for player `pidx`: pick the block under the crosshair
    /// (`target_block`, the break/place ray, reach included). The caller has
    /// already checked the cursor is captured and no inventory screen is up.
    pub(crate) fn pick_block_at_target(&mut self, pidx: usize) {
        if self.pack_window_frozen(pidx) {
            return;
        }
        let Some(cell) = self.players[pidx].target_block else { return };
        let aimed = self.world.get_block(cell[0], cell[1], cell[2]);
        let selected = self.players[pidx].hotbar_slot;
        let plan = plan_pick_block(self.play_mode, &self.players[pidx].inventory, selected, aimed);
        match plan {
            PickPlan::Nothing => {}
            PickPlan::Select(slot) => self.players[pidx].hotbar_slot = slot,
            PickPlan::Swap { bag_slot } => self.pick_swap_from_bag(pidx, selected, bag_slot),
            PickPlan::Give { slot, stack, .. } => {
                if self.joined() {
                    // The server's grant lands in the lowest empty slot, so
                    // that is the hand to move to; with a full hotbar it lands
                    // in the bag and the selection stays put (replacing the
                    // selected slot would need a new wire operation).
                    if let Some(empty) = (0..crate::window::BAG_START).find(|&i| self.players[pidx].inventory.slot(i).is_none()) {
                        self.players[pidx].hotbar_slot = empty;
                    }
                    self.request_creative_give(pidx, &stack);
                } else {
                    crate::plan_store::remember_item(&stack.item);
                    let p = &mut self.players[pidx];
                    p.inventory.set_slot(slot, Some(stack));
                    p.hotbar_slot = slot;
                }
            }
        }
    }

    /// The survival swap: the bag stack at `bag_slot` trades places with the
    /// hotbar slot `selected`, by the inventory screen's own three clicks.
    fn pick_swap_from_bag(&mut self, pidx: usize, selected: usize, bag_slot: usize) {
        let click = |slot: usize| WindowClick::Slot { slot, right: false };
        if !self.apply_window_click(pidx, &click(bag_slot)).ok() {
            return;
        }
        self.apply_window_click(pidx, &click(selected));
        // The old hotbar item (or, if the swap was refused, the picked stack)
        // is on the cursor: it goes back to where the picked one came from.
        // An empty selected slot took the stack outright and leaves nothing.
        if self.players[pidx].crafting_ui.cursor_item.is_some() {
            self.apply_window_click(pidx, &click(bag_slot));
        }
        // Never leave an item riding the cursor of a closed window.
        if self.players[pidx].crafting_ui.cursor_item.is_some() {
            self.close_crafting(pidx);
        }
    }
}
