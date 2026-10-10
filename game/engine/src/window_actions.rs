//! Actual playing-screen window entry points with pre-prediction reservation.
use crate::{GameState, window::{ClickCtx, ClickResult, WindowClick}, action_identity::InventoryMode};

impl GameState {
    fn strict_window_connection(&self) -> bool {
        self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict)
    }
    fn window_action_failed(&mut self, error: String) {
        log::warn!("Joined window action could not be retained: {error}");
        self.toast = Some(("Inventory action is waiting; try again.".into(), web_time::Instant::now() + web_time::Duration::from_secs(2)));
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
    /// Open player `pidx`'s 2×2 grid (E) or the crafting table at `table`:
    /// in Strict the open is a reserved action before the screen changes.
    pub(crate) fn open_crafting(&mut self, pidx: usize, table: Option<[i32; 3]>) {
        let p = &mut self.players[pidx];
        if !self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict) {
            match table {
                Some(cell) => p.crafting_ui.open_table_crafting(cell, &p.inventory, &p.armour_slots),
                None => p.crafting_ui.open_player_crafting(&p.inventory, &p.armour_slots),
            }
            return;
        }
        let result = p.crafting_ui.open_joined(self.remote_client.as_mut().expect("strict connection"), table, &p.inventory, &p.armour_slots);
        if let Err(error) = result { self.window_action_failed(error); }
    }
    /// A joiner asks to open the shared container at `cell` (logged in
    /// Mirror, a reserved action in Strict).
    pub(crate) fn ask_open_container(&mut self, pidx: usize, cell: [i32; 3]) {
        let p = &mut self.players[pidx];
        if !self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict) {
            p.crafting_ui.log_open_container(cell, &p.inventory, &p.armour_slots);
            return;
        }
        let result = p.crafting_ui.open_container_joined(self.remote_client.as_mut().expect("strict connection"), cell, &p.inventory, &p.armour_slots);
        if let Err(error) = result { self.window_action_failed(error); }
    }
    /// One click on player `pidx`'s shared container mirror. In Strict the
    /// action is reserved before the mirror or the window changes; a failed
    /// reservation changes nothing (`Refused`).
    pub(crate) fn apply_shared_container_click(&mut self, pidx: usize, mirror: &mut crate::container_window::SharedContainer, click: &crate::container_window::ContainerClick, ctx: &ClickCtx) -> crate::container_window::ContainerApplied {
        let p = &mut self.players[pidx];
        if !self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict) {
            return p.crafting_ui.apply_container_click(&mut p.inventory, &mut p.armour_slots, mirror.as_mut(), click, ctx);
        }
        let cell = mirror.cell;
        let result = p.crafting_ui.apply_container_click_joined(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, mirror.as_mut(), cell, click, ctx, &self.registry);
        match result {
            Ok(applied) => applied,
            Err(error) => { self.window_action_failed(error); crate::container_window::ContainerApplied { result: ClickResult::Refused, touched: Vec::new() } }
        }
    }
    /// Player `pidx`'s shared container screen closed: a `Close` click (a
    /// reserved action in Strict).
    pub(crate) fn close_shared_container(&mut self, pidx: usize) {
        let p = &mut self.players[pidx];
        if !self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict) {
            p.crafting_ui.close_container(&mut p.inventory, &mut p.armour_slots);
            return;
        }
        let result = p.crafting_ui.close_container_joined(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, &self.registry);
        if let Err(error) = result { self.window_action_failed(error); }
    }
}

#[cfg(test)]
mod tests {
    /// C3d review M1 — a crafting window mutation on the playing screen
    /// (click, close, Esc close, forced close) enters ONLY through the
    /// reserving entry points above, which allocate the Strict action
    /// before predicting and send no legacy `WindowOp` in Strict. The Esc
    /// chain used to call `crafting_ui.close` directly. Since C3d shared
    /// containers the opens, the container open, its clicks and its close
    /// are covered too.
    #[test]
    fn c3d_crafting_window_mutations_enter_only_through_reserving_entry_points() {
        let direct = [".crafting_ui.close(", ".crafting_ui.force_close(", ".crafting_ui.apply_click("];
        // lib.rs's `AXENSTAX_AUTO_INVENTORY` pre-open runs at boot, before
        // any connection, so the open guard reads the in-play files only.
        let opens = [
            ".crafting_ui.open_player_crafting(", ".crafting_ui.open_table_crafting(", ".crafting_ui.log_open_container(",
            ".crafting_ui.close_container(",
        ];
        // game_loop.rs's `apply_container_click` sites act on the world's OWN
        // containers (single-player, a host: `ctx.shared == false`), never a
        // joiner's mirror; the shared container's clicks live in
        // container_client.rs, guarded below.
        let shared = [".crafting_ui.apply_container_click("];
        for (file, source, guarded) in [
            ("lib.rs", include_str!("lib.rs"), &direct[..]),
            ("game_loop.rs", include_str!("game_loop.rs"), &[&direct[..], &opens[..]].concat()[..]),
            ("container_client.rs", include_str!("container_client.rs"), &[&direct[..], &opens[..], &shared[..]].concat()[..]),
            ("player_slot.rs", include_str!("player_slot.rs"), &[&direct[..], &opens[..]].concat()[..]),
        ] {
            for call in guarded {
                assert!(!source.contains(call), "{file} mutates the crafting window directly ({call}); use GameState::apply_window_click / close_crafting / force_close_crafting");
            }
        }
        let entry = include_str!("window_actions.rs");
        let production = &entry[..entry.find("#[cfg(test)]").expect("this test module")];
        assert!(production.contains("crafting_ui.close_joined(") && production.contains("crafting_ui.force_close_joined(") && production.contains("crafting_ui.apply_joined_click(") && production.contains("crafting_ui.open_joined(") && production.contains("crafting_ui.apply_container_click_joined(") && production.contains("crafting_ui.close_container_joined(") && production.contains("crafting_ui.open_container_joined("), "the Strict arms reserve through the joined entry points");
    }
}
