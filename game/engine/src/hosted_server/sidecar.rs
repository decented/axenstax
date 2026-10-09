//! Join persistence boundaries and creative grants to the joiner's own window.
use super::*;
impl HostedServer {
    pub(super) fn serve_creative_give(&mut self, i: usize, seq: u32, wire: &protocol::WireStack) {
        use crate::item_actions::ItemNote;
        if !self.server.play_mode.is_creative() {
            return self.refuse_item_action(i, seq, ItemNote::NotHere);
        }
        let Some(stack) = crate::inventory::stack_from_wire(
            wire,
            &self.server.registry,
            crate::inventory::PlanDecode::Marker,
        ) else {
            return self.refuse_item_action(i, seq, ItemNote::NotNow);
        };
        if matches!(stack.item, crate::item::Item::Plan(_)) {
            if stack.count != 1 {
                return self.refuse_item_action(i, seq, ItemNote::NotNow);
            }
            let mut inv = crate::window_events::effective_inventory(&self.server.players[i]);
            if inv.add_item(stack.clone()).is_some() {
                return self.refuse_item_action(i, seq, ItemNote::InventoryFull);
            }
            // Only the caller's private marker: never a Plan pickup or body transfer.
            let event = crate::window_events::queue(
                &mut self.server.players[i],
                crate::window_events::WindowEvent::Grant(stack),
                self.server.tick_counter,
            );
            let pkt = protocol::InventoryGrantPacket {
                item_kind: protocol::item_kind::PLAN,
                item_id: 0,
                count: 1,
                full_item: wire.full_item,
                window_event: event,
            };
            self.send_to_joined_slot(
                i,
                &protocol::serialize_packet(protocol::PacketType::InventoryGrant, &pkt),
            );
        } else {
            self.grant_to_joiner(i, [stack]);
        }
        self.send_item_outcome(
            i,
            &protocol::ItemActionOutcomePacket {
                seq,
                accepted: true,
                consume_held: 0,
                note: ItemNote::None.to_wire(),
                window_event: 0,
                wear_held: false,
                bite_after: 0,
            },
        );
    }

    pub(super) fn settle_terminal_grants(&mut self, i: usize) {
        let sp = &mut self.server.players[i];
        let overflow = crate::window_events::settle_terminal_grants(sp, self.server.tick_counter);
        for unfit in overflow {
            log::debug!("Settling grant {} overflow for slot {i}", unfit.event);
            self.route_joiner_unfit_at(i, unfit.stack, unfit.grave);
        }
    }

    pub(super) fn flush_retired_overflow(&mut self, i: usize) {
        debug_assert!(!self.lends_host_world() || self.server.lent);
        let Some(sp) = self.server.players.get_mut(i) else {
            return;
        };
        for unfit in crate::window_events::take_retired_overflow(sp) {
            log::debug!(
                "Returning retired grant {} overflow for slot {i}",
                unfit.event
            );
            self.route_joiner_unfit_at(i, unfit.stack, unfit.grave);
        }
    }

    /// A fatal report and a disconnect/reconnect can arrive in the same poll.
    /// Finish the old life before its leave snapshot or slot reuse.
    pub(super) fn settle_leaving_life(&mut self, leaving: usize) {
        debug_assert!(
            !self.lends_host_world() || self.server.lent,
            "death settlement needs the authoritative world"
        );
        let grants = std::mem::take(&mut self.server.pending_item_grants);
        for (slot, stack) in grants {
            self.grant_to_joiner(slot, [stack]);
        }
        self.settle_terminal_grants(leaving);
        self.handle_joiner_deaths();
    }
}
