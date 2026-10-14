//! Actual playing-screen window entry points with pre-prediction reservation.
use crate::{GameState, window::{ClickCtx, ClickResult, WindowClick}, action_identity::InventoryMode};

/// What a crafting-screen close did ([`GameState::close_crafting`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CraftClose {
    /// The grid and cursor went back; the screen closed.
    Closed,
    /// A full bag kept some of them; the screen stays open.
    Refused,
    /// C3d run 8 — a frozen Strict window (dead body, unanswered Respawn):
    /// nothing was sent and nothing changed.
    Frozen,
}

impl GameState {
    fn strict_window_connection(&self) -> bool {
        self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict)
    }
    /// C3d run 6 — a Strict joiner whose body is dead, or whose Respawn is
    /// still unanswered: its crafting screen sends no `Close` (the server's
    /// correction `Replace` is the guarantee; this avoids flicker).
    ///
    /// C3d run 8 (review LOW-1) — THE one gate: read only by
    /// [`Self::close_crafting`], [`Self::force_close_crafting`] and the
    /// return-the-grid `Close` an open sends first ([`Self::open_crafting`]),
    /// so Esc, E, the touch/gamepad pause and the table force-close all reach
    /// it. Esc while frozen pauses ([`Self::esc_crafting`]): a never-answered
    /// Respawn can't trap the player in a modal with no pause menu.
    fn strict_window_frozen(&self, pidx: usize) -> bool {
        if self.pack_window_frozen(pidx) { return true; }
        if pidx == 0 && self.remote_client.as_ref().is_some_and(|r| !r.personal_ready()) { return true; }
        pidx == 0
            && self.strict_window_connection()
            && (self.players[0].is_dead() || self.remote_client.as_ref().is_some_and(|r| r.respawn_unanswered()))
    }
    fn window_action_failed(&mut self, error: String) {
        log::warn!("Joined window action could not be retained: {error}");
        self.toast = Some(("Inventory action is waiting; try again.".into(), web_time::Instant::now() + web_time::Duration::from_secs(2)));
    }
    pub(crate) fn apply_window_click(&mut self, pidx: usize, click: &WindowClick) -> ClickResult {
        if self.pack_window_frozen(pidx) { return ClickResult::Refused; }
        if pidx == 0 && self.remote_client.as_ref().is_some_and(|r| !r.personal_ready()) { return ClickResult::Refused; }
        if !self.strict_window_connection() {
            let p = &mut self.players[pidx];
            return p.crafting_ui.apply_click(&mut p.inventory, &mut p.armour_slots, click, self.is_creative, p.player.eye_pos(), |cell| self.world.get_block(cell[0], cell[1], cell[2]));
        }
        let p = &mut self.players[pidx];
        let ctx = ClickCtx::new(self.is_creative, p.crafting_ui.station(), p.player.eye_pos(), |cell| self.world.get_block(cell[0], cell[1], cell[2]));
        let result = p.crafting_ui.apply_joined_click(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, click, &ctx, &self.registry);
        match result { Ok(result) => result, Err(error) => { self.window_action_failed(error); ClickResult::Refused } }
    }
    /// Close player `pidx`'s crafting screen (Esc, E, a pause): the grid and
    /// cursor go back to the bag. In Strict a reserved action, and nothing
    /// at all while the window is frozen (the one gate).
    pub(crate) fn close_crafting(&mut self, pidx: usize) -> CraftClose {
        if self.strict_window_frozen(pidx) { return CraftClose::Frozen; }
        let closed = if !self.strict_window_connection() {
            let p = &mut self.players[pidx];
            p.crafting_ui.close(&mut p.inventory, &mut p.armour_slots)
        } else {
            let p = &mut self.players[pidx];
            let result = p.crafting_ui.close_joined(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, &self.registry);
            match result { Ok(closed) => closed, Err(error) => { self.window_action_failed(error); false } }
        };
        if closed { CraftClose::Closed } else { CraftClose::Refused }
    }
    /// Esc on an open crafting screen: it closes (a full bag leaves it open),
    /// or, while the window is frozen, the game pauses instead — never the
    /// generic panel close, which would reach the same `Close`.
    pub(crate) fn esc_crafting(&mut self) {
        match self.close_crafting(0) {
            CraftClose::Closed | CraftClose::Refused => self.capture_cursor(),
            CraftClose::Frozen => {
                self.release_cursor();
                self.skin_panel.open = false;
                self.mode = crate::GameMode::Paused { confirm_quit: false, confirm_creative: false };
            }
        }
    }
    pub(crate) fn force_close_crafting(&mut self, pidx: usize, new_tick: bool) -> bool {
        // The one gate, before the pacing: a frozen window's tries are not refusals.
        if self.strict_window_frozen(pidx) { return false; }
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
    ///
    /// C3d run 8 (review H1) — a grid holding items goes back to the bag
    /// first, by a real `Close` over the real grid (`CraftingUi::open_crafting`
    /// / `open_joined`); that `Close` passes the one gate, so a frozen window
    /// opens nothing.
    pub(crate) fn open_crafting(&mut self, pidx: usize, table: Option<[i32; 3]>) {
        if self.pack_window_frozen(pidx) || self.players[pidx].crafting_ui.grid_holds_items() && self.strict_window_frozen(pidx) { return; }
        let p = &mut self.players[pidx];
        if !self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict) {
            p.crafting_ui.open_crafting(table, &mut p.inventory, &mut p.armour_slots);
            return;
        }
        let result = p.crafting_ui.open_joined(self.remote_client.as_mut().expect("strict connection"), table, &mut p.inventory, &mut p.armour_slots, &self.registry);
        if let Err(error) = result { self.window_action_failed(error); }
    }
    /// A joiner asks to open the shared container at `cell` (logged in
    /// Mirror, a reserved action in Strict).
    pub(crate) fn ask_open_container(&mut self, pidx: usize, cell: [i32; 3]) {
        if self.pack_window_frozen(pidx) { return; }
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
        let refused = || crate::container_window::ContainerApplied { result: ClickResult::Refused, touched: Vec::new() };
        if self.pack_window_frozen(pidx) { return refused(); }
        if let Some(session) = mirror.pack_session {
            let operation = crate::pack_wire::PackOperation::Click { incarnation: session.incarnation, revision: session.revision, click: click.clone() };
            return if self.request_pack_action(pidx, mirror.target, operation) { crate::container_window::ContainerApplied { result: ClickResult::Done, touched: Vec::new() } } else { refused() };
        }
        let Some(cell) = mirror.block_cell() else { return refused() };
        let p = &mut self.players[pidx];
        if !self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == InventoryMode::Strict) {
            return p.crafting_ui.apply_container_click(&mut p.inventory, &mut p.armour_slots, mirror.as_mut(), click, ctx);
        }
        let result = p.crafting_ui.apply_container_click_joined(self.remote_client.as_mut().expect("strict connection"), &mut p.inventory, &mut p.armour_slots, mirror.as_mut(), cell, click, ctx, &self.registry);
        match result {
            Ok(applied) => applied,
            Err(error) => { self.window_action_failed(error); crate::container_window::ContainerApplied { result: ClickResult::Refused, touched: Vec::new() } }
        }
    }
    /// Player `pidx`'s shared container screen closed: a `Close` click (a
    /// reserved action in Strict).
    pub(crate) fn close_shared_container(&mut self, pidx: usize) {
        if self.players[pidx].shared_container.as_ref().is_some_and(|mirror| mirror.pack_session.is_some()) { self.request_pack_close(pidx); return; }
        if self.pack_window_frozen(pidx) { return; }
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
