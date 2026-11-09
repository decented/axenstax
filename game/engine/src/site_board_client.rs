//! Living world Tier 2 S7 — the client half of the Site Board: where a window's
//! op goes. A local seat (single-player, a host's seat, split-screen) applies it
//! through the one rule, `site_board::use_board`, in the world it plays in (the
//! lent world on a lending host, so the authority and its joiners see the same
//! list); a joined client asks the authority and is shown the answer as a block
//! view. The surge a tick lifts rides `World::note_feast`, drained by
//! `feast_client::see_feasts` for the confetti and the bloom field.

use crate::GameState;
use crate::site_board::Op;

impl GameState {
    /// Player `pidx` asked for `op` on the Site Board at `cell`.
    pub(crate) fn apply_site_board_op(&mut self, pidx: usize, cell: (i32, i32, i32), op: Op) {
        if self.joined() {
            self.send_site_board_op(pidx, [cell.0, cell.1, cell.2], op);
            return;
        }
        let tonight = self.night_calendar.tonight();
        // A refusal (a full board, a task gone) is a race with another seat and
        // changes nothing; the window redraws the list as it now stands.
        let _ = crate::site_board::use_board(&mut self.world, cell, op, tonight);
    }
}
