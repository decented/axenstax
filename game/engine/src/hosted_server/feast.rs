//! Living world S3 — the authority's half of the Feast
//! (`docs/superpowers/specs/2026-11-08-living-world-tier1.md` §3.2, "Guest plumbing").
//!
//! A joiner right-clicked a Feast Table (`ItemAction::UseBlock`, judged by
//! `judge_block_use`: alive, in reach, the play mode and plots allow it). The
//! shared rule is `feast::hold`; this module is what only the authority can
//! do, the same on a lent host and a dedicated server:
//!
//! - **The guests** are `GameServer.players` that are present and alive,
//!   except the holder (a lent host's own seats are in that list, so they
//!   count), and the ECS villagers, gathered by `feast::gather`.
//! - **The stack** is what the server's copy of the joiner's window holds in
//!   the hand slot it named; the whole of it is the goods. What that copy
//!   can't cover is believed within the joiner's believed-pay bound
//!   (`window_ops::believe_pay`) and the feast is refused past it, exactly as
//!   any other `UseBlock` pay.
//! - **The authority spawns every player guest's share** as a real ground item
//!   at their feet, and turns the table into a Feast Marker (meta = guests) in
//!   the world it serves; the block change goes out like any other.
//!
//! The holder's toast and the confetti come from the clients seeing the block
//! change (`feast_client`); a joiner fires no trial events.
use super::*;
use crate::item_actions::ItemNote;

impl HostedServer {
    fn feast_outcome(&self, i: usize, seq: u32, accepted: bool, consume_held: u8, note: ItemNote, window_event: u32) {
        let pkt = protocol::serialize_packet(
            protocol::PacketType::ItemActionOutcome,
            &protocol::ItemActionOutcomePacket {
                seq,
                accepted,
                consume_held,
                note: note.to_wire(),
                window_event,
                wear_held: false,
                bite_after: 0,
            },
        );
        self.send_to_joined_slot(i, &pkt);
    }

    /// Joiner `i` used the Feast Table at `cell` holding `held` in hotbar slot
    /// `hotbar_slot`. Refusals change nothing and take nothing.
    pub(super) fn serve_feast(&mut self, i: usize, seq: u32, cell: [i32; 3], hotbar_slot: u8, held: Option<crate::item::Item>) {
        let pos = (cell[0], cell[1], cell[2]);
        let creative = self.server.play_mode.is_creative();
        let now = self.server.tick_counter;
        let Some(item) = held else {
            return self.feast_outcome(i, seq, false, 0, ItemNote::NothingToShare, 0);
        };
        let slot = {
            let Some(sp) = self.server.players.get(i) else { return };
            if hotbar_slot < 9 { usize::from(hotbar_slot) } else { sp.hotbar_slot }
        };
        // The goods: the server's copy of the hand slot (as it will be once its
        // waiting events land), else 1 believed.
        let count = {
            let Some(sp) = self.server.players.get(i) else { return };
            let w = crate::window_events::effective_window(sp);
            w.inv.slot(slot).filter(|s| s.item == item).map_or(1, |s| s.count).max(1)
        };
        let stack = crate::item::ItemStack { item: item.clone(), count };
        let table = crate::feast::cell_centre(pos);
        let players = self
            .server
            .players
            .iter()
            .enumerate()
            .filter(|(j, sp)| *j != i && sp.is_present_and_alive())
            .map(|(j, sp)| (j, sp.player.pos));
        let guests = crate::feast::gather(table, players, crate::feast::villagers_in(&self.server.ecs));
        let tonight = self.server.night_calendar.tonight();
        let feasted = crate::feast::feasted_today(&self.server.world, pos, tonight);
        let feast = match crate::feast::hold(pos, Some(&stack), &guests, feasted) {
            Ok(f) => f,
            Err(crate::feast::Refusal::NeedsGuests) => {
                return self.feast_outcome(i, seq, false, 0, ItemNote::NeedsGuests, 0);
            }
            Err(crate::feast::Refusal::NothingToShare) => {
                return self.feast_outcome(i, seq, false, 0, ItemNote::NothingToShare, 0);
            }
            Err(crate::feast::Refusal::NeedsFood) => {
                return self.feast_outcome(i, seq, false, 0, ItemNote::NeedsFood, 0);
            }
            Err(crate::feast::Refusal::FeastedToday) => {
                return self.feast_outcome(i, seq, false, 0, ItemNote::FeastedToday, 0);
            }
        };
        // Pay: believed within the joiner's bound, refused past it. A creative
        // joiner pays nothing (and is unbounded), as its other uses.
        let mut believed = 0u32;
        if !creative {
            let Some(sp) = self.server.players.get_mut(i) else { return };
            match crate::window_ops::believe_pay(sp, &item, u32::from(feast.pay), now) {
                Ok(n) => believed = n,
                Err(over) => {
                    self.note_believed_use(i, 0, over, now);
                    return self.feast_outcome(i, seq, false, 0, ItemNote::NothingToTake, 0);
                }
            }
        }
        if believed > 0 {
            self.note_believed_use(i, believed, 0, now);
        }
        crate::feast::spawn_drops(&mut self.server.ecs, &stack, &feast);
        let change = crate::feast::convert(&mut self.server.world, pos, feast.guests, tonight);
        self.pending_block_changes.push(change);
        if self.lends_host_world() {
            self.lent_edit_cells.push(pos);
        }
        let consume = if creative { 0 } else { feast.pay };
        let owed = consume.saturating_sub(u8::try_from(believed).unwrap_or(u8::MAX));
        let window_event = if owed > 0 { self.shadow_take_owed(i, slot, crate::hands::Hand::Main, &item, owed, "at a feast") } else { 0 };
        self.feast_outcome(i, seq, true, consume, ItemNote::None, window_event);
    }
}
