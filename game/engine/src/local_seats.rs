//! Split-screen seats leaving (and, in tests, joining) a world in progress.
//!
//! A seat's index is its `PlayerSlot`, its GPU resources and its screen. A
//! hosting or solo-hosted server keeps its own slot indices stable instead
//! (`HostedServer::sync_local_slots` marks a departed seat's slot out of the
//! world), so nothing here touches the server: the next tick re-syncs it.

/// The arm shape the body of seat `seat` is drawn with: only seat 0 has a wardrobe
/// (`GameState::worn_arm_model`), every other local body is Classic. One answer for
/// the seat's own third-person body, its first-person hand and the copy of it drawn
/// in another seat's viewport, so two viewports never fit its armour to different
/// arm widths.
pub(crate) fn seat_arm_model(seat: usize, wardrobe: crate::skin_uv::ArmModel) -> crate::skin_uv::ArmModel {
    if seat == 0 { wardrobe } else { crate::skin_uv::ArmModel::Classic }
}

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
        // The seats after it shift down: an eat belongs to its seat, not its index.
        crate::eating::reindex_after_seat_left(&mut self.eating, leave_idx);
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

#[cfg(test)]
mod tests {
    use super::seat_arm_model;
    use crate::skin_uv::ArmModel;

    #[test]
    fn a_seats_body_has_the_same_arms_in_every_viewport() {
        // Seat 0 wears its wardrobe's arms wherever it is drawn: its own F5 body,
        // its hand, and the copy another seat's viewport shows.
        assert_eq!(seat_arm_model(0, ArmModel::Slim), ArmModel::Slim);
        assert_eq!(seat_arm_model(0, ArmModel::Classic), ArmModel::Classic);
        // The others have no wardrobe of their own: never seat 0's arms.
        assert_eq!(seat_arm_model(1, ArmModel::Slim), ArmModel::Classic);
        assert_eq!(seat_arm_model(3, ArmModel::Slim), ArmModel::Classic);
    }
}
