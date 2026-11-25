//! The game-state half of eating you can see (`eating.rs` is the pure half):
//! starting an eat when a bite is taken, ageing it each frame, and the crumbs
//! and the munch of each bite. Presentation only: nothing here changes what is
//! eaten, when, or what the server is told (Spec 05 §6.4.2).

use crate::eating::{self, Eating};
use crate::hands::Hand;
use crate::item_model::HeldItem;
use crate::GameState;

impl GameState {
    /// A bite was taken (or, for a joiner, asked for) by player `pidx` from
    /// `held` in `hand`: start showing it. `held` is the food as it was before
    /// the bite took it from its slot. A bite taken in the middle of another
    /// restarts the picture (a held click eats once per cooldown, so the old one
    /// has run its course by then).
    pub(crate) fn begin_eating(&mut self, pidx: usize, hand: Hand, held: Option<HeldItem>) {
        let Some(held) = held.filter(|h| !h.is_empty()) else { return };
        let slot = self.players[pidx].hotbar_slot;
        self.eating.insert(pidx, Eating::start(held, hand, slot));
    }

    /// The frame step of every local player's eat: age it by `dt` seconds, end
    /// it when it has run its course or the player moved on (hand or slot
    /// changed, died, opened a screen), and for each bite that lands throw its
    /// crumbs from the mouth and play the munch.
    pub(crate) fn tick_eating_presentation(&mut self, dt: f32) {
        if self.eating.is_empty() {
            return;
        }
        let modal_p0 = self.p0_ui_modal_open();
        let players: Vec<usize> = self.eating.keys().copied().collect();
        for pidx in players {
            let Some(mut eat) = self.eating.remove(&pidx) else { continue };
            let Some(slot) = self.players.get(pidx) else { continue };
            let screen_open = self.seat_modal_open(pidx) || (pidx == 0 && modal_p0);
            if eating::ends_early(&eat, slot.hotbar_slot, slot.is_dead(), screen_open) {
                continue;
            }
            let landed = eat.advance(dt);
            let first_person = slot.camera.mode.shows_viewmodel();
            let mouth = eating::mouth(
                first_person,
                slot.player.pos,
                slot.player.eye_pos(),
                slot.camera.forward(),
            );
            for bite in landed {
                let colours = eating::crumb_colours(eat.held.item, bite);
                let seed = self.tick_counter.wrapping_add((pidx as u64) << 40) ^ ((bite as u64 + 1) << 20);
                self.particles.burst_crumbs(mouth, &colours, eating::CRUMBS_PER_BITE, seed);
                self.audio.play_munch();
            }
            if !eat.finished() {
                self.eating.insert(pidx, eat);
            }
        }
    }
}
