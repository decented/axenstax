//! Actual playing-screen window entry points with pre-prediction reservation.
use crate::{GameState, window::{ClickCtx, ClickResult, WindowClick}, action_identity::InventoryMode};

impl GameState {
    fn strict_window_connection(&self) -> bool {
        self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict)
    }
    fn window_action_failed(&mut self, error: String) {
        log::warn!("Joined window action could not be retained: {error}");
        self.toast = Some(("Inventory action is waiting; try again.".into(), std::time::Instant::now() + std::time::Duration::from_secs(2)));
    }
    pub(crate) fn apply_window_click(&mut self, pidx: usize, click: &WindowClick) -> ClickResult {
        if !self.strict_window_connection() {
            let p = &mut self.players[pidx];
            return p.crafting_ui.apply_click(&mut p.inventory, &mut p.armour_slots, click, self.is_creative, p.player.eye_pos(), |cell| self.world.get_block(cell[0], cell[1], cell[2]));
        }
        let p = &mut self.players[pidx];
        let ctx = ClickCtx::new(self.is_creative, p.crafting_ui.station(), p.player.eye_pos(), |cell| self.world.get_block(cell[0], cell[1], cell[2]));
        let result = p.crafting_ui.apply_joined_click(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, click, &ctx, &self.registry);
        match result { Ok(result) => result, Err(error) => { self.window_action_failed(error); ClickResult::Refused } }
    }
    pub(crate) fn close_crafting(&mut self, pidx: usize) -> bool {
        if !self.strict_window_connection() {
            let p = &mut self.players[pidx];
            return p.crafting_ui.close(&mut p.inventory, &mut p.armour_slots);
        }
        let p = &mut self.players[pidx];
        let result = p.crafting_ui.close_joined(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, &self.registry);
        match result { Ok(closed) => closed, Err(error) => { self.window_action_failed(error); false } }
    }
    pub(crate) fn force_close_crafting(&mut self, pidx: usize, new_tick: bool) -> bool {
        if !self.strict_window_connection() {
            let p = &mut self.players[pidx];
            return p.crafting_ui.force_close(&mut p.inventory, &mut p.armour_slots, new_tick);
        }
        let p = &mut self.players[pidx];
        let result = p.crafting_ui.force_close_joined(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, &self.registry, new_tick);
        match result { Ok(closed) => closed, Err(error) => { self.window_action_failed(error); false } }
    }
}
