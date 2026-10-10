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

#[cfg(test)]
mod tests {
    /// C3d review M1 — a crafting window mutation on the playing screen
    /// (click, close, Esc close, forced close) enters ONLY through the
    /// reserving entry points above, which allocate the Strict action
    /// before predicting and send no legacy `WindowOp` in Strict. The Esc
    /// chain used to call `crafting_ui.close` directly. Containers are a
    /// later C3d milestone (`close_container`), not covered here.
    #[test]
    fn c3d_crafting_window_mutations_enter_only_through_reserving_entry_points() {
        let direct = [".crafting_ui.close(", ".crafting_ui.force_close(", ".crafting_ui.apply_click("];
        for (file, source) in [
            ("lib.rs", include_str!("lib.rs")),
            ("game_loop.rs", include_str!("game_loop.rs")),
            ("container_client.rs", include_str!("container_client.rs")),
            ("player_slot.rs", include_str!("player_slot.rs")),
        ] {
            for call in direct {
                assert!(!source.contains(call), "{file} mutates the crafting window directly ({call}); use GameState::apply_window_click / close_crafting / force_close_crafting");
            }
        }
        let entry = include_str!("window_actions.rs");
        let production = &entry[..entry.find("#[cfg(test)]").expect("this test module")];
        assert!(production.contains("crafting_ui.close_joined(") && production.contains("crafting_ui.force_close_joined(") && production.contains("crafting_ui.apply_joined_click("), "the Strict arms reserve through the joined entry points");
    }
}
