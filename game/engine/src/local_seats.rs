//! Split-screen seats leaving (and, in tests, joining) a world in progress.
//!
//! A seat's index is its `PlayerSlot`, its GPU resources and its screen. A
//! hosting or solo-hosted server keeps its own slot indices stable instead
//! (`HostedServer::sync_local_slots` marks a departed seat's slot out of the
//! world), so nothing here touches the server: the next tick re-syncs it.

impl crate::GameState {
    /// The pause menu's "Player N leaves": the seat gets off whatever it
    /// rides (its mount is free again, and nothing keeps steering it), its
    /// slot and GPU resources go, a departed P1's pad passes on, and the
    /// screens are laid out again.
    pub(crate) fn remove_local_seat(&mut self, leave_idx: usize) {
        if leave_idx >= self.players.len() || self.players.len() <= 1 {
            return;
        }
        if let Some(mount) = self.players[leave_idx].riding {
            self.dismount(leave_idx, mount);
        }
        self.players.remove(leave_idx);
        if leave_idx < self.renderer.player_gpu.len() {
            self.renderer.player_gpu.remove(leave_idx);
        }
        // P1 left: the remaining player keeps their controller; P1 is now
        // whichever pad is still connected (keyboard + mouse when none).
        if leave_idx == 0 {
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.p1_gamepad = self
                    .gamepad
                    .gamepads()
                    .iter()
                    .enumerate()
                    .find(|(_, gp)| gp.connected)
                    .map(|(idx, _)| idx);
            }
            #[cfg(target_arch = "wasm32")]
            {
                self.p1_gamepad = None;
            }
        }
        self.relayout_seats();
    }

    /// Lay the screens out for the current seats and give each camera its
    /// viewport's aspect.
    pub(crate) fn relayout_seats(&mut self) {
        self.screens = crate::screen::compute_screen_layout(
            self.players.len(),
            self.renderer.width,
            self.renderer.height,
        );
        for screen in &self.screens {
            let crate::screen::ScreenContent::LocalPlayer(pidx) = screen.content;
            if let Some(p) = self.players.get_mut(pidx) {
                p.camera.aspect = screen.viewport.aspect();
            }
        }
    }

    /// Seat another local player at `spawn` (the split-screen join's steps:
    /// a slot, its GPU resources, the screens). Returns its index.
    #[cfg(test)]
    pub(crate) fn add_local_seat(&mut self, spawn: glam::Vec3) -> usize {
        let idx = self.players.len();
        let mut slot = crate::player_slot::PlayerSlot::new(idx, spawn, 1.0);
        slot.player.flying = self.play_mode.flies();
        self.players.push(slot);
        let gpu = self.renderer.create_player_resources();
        self.renderer.player_gpu.push(gpu);
        self.relayout_seats();
        idx
    }
}
