//! Living world Tier 2 S7 — the authority's half of the Site Board
//! (`docs/superpowers/specs/2026-11-09-living-world-tier2.md` §7).
//!
//! A joiner's `ItemAction::SiteBoard` is judged here: its body must be in the
//! world and alive, the cell within the server body's reach and a Site Board on
//! THIS world. Then the one rule runs, `site_board::use_board` (the function a
//! local seat calls too): it applies the op and, for a task ticked to done, lifts
//! the board's region once per board per world day (`World::note_feast`, drained
//! by the host client or the dedicated server's bloom).
//!
//! Anyone may do any op, so there is no plot or Commons gate, no hand claim and
//! no cost. The board's new list reaches every client that holds the chunk as a
//! `BlockView::SiteBoard` on the next tick's `block_views` pass (the asker's
//! too, so the outcome carries nothing but accepted or why not).
use super::*;
use crate::item_actions::ItemNote;
use crate::site_board::{Refusal, use_board};

impl HostedServer {
    /// Joiner `i` asked for `op` on the Site Board at `pos`. Refusals change
    /// nothing. A full board and a missing task are silent (the window the joiner
    /// sees already disables them; they only follow a race with another player).
    pub(super) fn serve_site_board(&mut self, i: usize, seq: u32, pos: [i32; 3], op: crate::site_board::Op) {
        let Some(sp) = self.server.players.get(i) else { return };
        if !sp.is_present_and_alive() {
            return self.refuse_item_action(i, seq, ItemNote::NotNow);
        }
        if !crate::container_window::container_in_server_reach(sp.player.eye_pos(), pos) {
            return self.refuse_item_action(i, seq, ItemNote::OutOfReach);
        }
        let cell = (pos[0], pos[1], pos[2]);
        let tonight = self.server.night_calendar.tonight();
        match use_board(&mut self.server.world, cell, op, tonight) {
            Ok(_) => {
                let out = protocol::ItemActionOutcomePacket {
                    seq,
                    accepted: true,
                    consume_held: 0,
                    note: ItemNote::None.to_wire(),
                    window_event: 0,
                    wear_held: false,
                    bite_after: 0,
                };
                self.send_item_outcome(i, &out);
            }
            Err(Refusal::NotABoard) => self.refuse_item_action(i, seq, ItemNote::NotThatBlock),
            Err(Refusal::Full | Refusal::NoSuchTask | Refusal::UnknownJob) => {
                self.refuse_item_action(i, seq, ItemNote::NothingToTake)
            }
        }
    }
}
