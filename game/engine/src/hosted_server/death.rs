//! The server-owned death window and shared grave retrieval.
use super::*;
impl HostedServer {
    /// Handle every death once, after inbound reports and the simulation tick.
    pub(super) fn handle_joiner_deaths(&mut self) {
        for i in 0..self.server.players.len() {
            let sp = &self.server.players[i];
            if !sp.server_simulated || !sp.is_in_world() || !sp.combat.dead || sp.death_handled {
                continue;
            }
            self.settle_terminal_grants(i);
            let sp = &mut self.server.players[i];
            sp.death_handled = true;
            sp.combat.just_died = false;
            let cause = damage_cause_to_wire(sp.combat.last_damage);
            let pos = sp.player.pos;
            sp.open_container = None;
            sp.container_sent.open_kind = None;
            sp.container_sent.seen = None;
            let mut window = crate::window_events::effective_window(sp);
            let mut grave_at = None;
            if !self.server.world.keep_inventory && !self.server.play_mode.is_creative() {
                let (grave, kept) = crate::death::split_window(&window.inv, true);
                for (at, stack) in kept.into_iter().enumerate() {
                    window.inv.set_slot(at, stack);
                }
                if grave.iter().any(Option::is_some) {
                    let cell = crate::grave::find_safe_grave_pos(
                        |x, y, z| {
                            (0..96).contains(&y)
                                && self
                                    .server
                                    .loaded_columns
                                    .contains(&(x.div_euclid(16), z.div_euclid(16)))
                                && self.server.world.get_block(x, y, z) == crate::block::AIR
                        },
                        pos.x.floor() as i32,
                        pos.y.floor() as i32,
                        pos.z.floor() as i32,
                    );
                    if let Some([x, y, z]) = cell {
                        self.server.world.set_block(x, y, z, crate::block::GRAVE);
                        self.server
                            .pending_block_changes
                            .push(crate::game_loop::broadcast_change(
                                &self.server.world,
                                x,
                                y,
                                z,
                                crate::block::GRAVE,
                            ));
                        self.server.world.insert_grave(
                            (x, y, z),
                            crate::grave::GraveData::from_snapshot(
                                grave.into(),
                                self.server.world_time,
                            ),
                        );
                        grave_at = cell;
                    } else {
                        for (k, stack) in grave.into_iter().flatten().enumerate() {
                            crate::entity::spawn_item(
                                &mut self.server.ecs,
                                pos,
                                stack,
                                self.server.world_time.wrapping_add(k as u32),
                            );
                        }
                    }
                }
            }
            self.send_to_joined_slot(
                i,
                &protocol::serialize_packet(
                    protocol::PacketType::PlayerEvent,
                    &protocol::PlayerEventPacket {
                        player_index: i as u32,
                        event: protocol::PlayerEventType::DiedOf { cause },
                        window_event: 0,
                    },
                ),
            );
            if let Some([x, y, z]) = grave_at {
                self.send_to_joined_slot(
                    i,
                    &protocol::serialize_packet(
                        protocol::PacketType::PlayerEvent,
                        &protocol::PlayerEventPacket {
                            player_index: i as u32,
                            event: protocol::PlayerEventType::GraveAt { x, y, z },
                            window_event: 0,
                        },
                    ),
                );
            }
            // OC — death clears the correction debt (accepted Mirror
            // behaviour since v86): what the old life owed dies with it, on
            // both copies. The replacement carries an empty debt, explicitly.
            let n = self.send_inventory_sync(i, window, crate::container_window::CorrectionDebt::default());
            self.server.players[i].window_events.death_replace = Some(n);
        }
    }

    /// Replace joiner `i`'s whole window with `window` and its correction
    /// debt with `debt` (OC, v88), on both copies: one numbered
    /// `WindowEvent::Replace` for the server's and one `InventorySync`
    /// carrying the same window and debt for the client's. The debt is
    /// carried as a carrier holds it (`CorrectionDebt::carried`: at most
    /// `MAX_CARRIED_DEBT` entries; `CorrectionDebt::wire_safe`: only entries
    /// the client decodes back exactly), the same on both copies; anything
    /// left out marks the session's obligations incomplete. Returns the
    /// event's number.
    pub(super) fn send_inventory_sync(
        &mut self,
        i: usize,
        window: crate::window_events::EffectiveWindow,
        debt: crate::container_window::CorrectionDebt,
    ) -> u32 {
        let (debt, over) = debt.carried();
        let (debt, unsafe_on_wire) = debt.wire_safe(&self.server.registry);
        let dropped = over + unsafe_on_wire;
        if dropped > 0 {
            // A validated save holds at most the bound, and a death or a
            // fresh joiner none; what can fail the wire is an owed item it
            // can't carry (a record's tool or armour piece at durability 0 or
            // above its maximum, or an unregistered block). Either way the
            // session can no longer prove its debt (`Overflow`).
            log::warn!(
                "Inventory sync for {}: {dropped} correction-debt entries left out (past the carried bound, \
                 or not decodable by the client)",
                self.server.players[i].display_name
            );
            self.server.players[i].obligations.mark(crate::joiner_save::IncompleteCause::Overflow);
        }
        let n = crate::window_events::queue(
            &mut self.server.players[i],
            crate::window_events::WindowEvent::Replace { window: Box::new(window.clone()), debt: debt.clone() },
            self.server.tick_counter,
        );
        let mut pkt = crate::inventory_sync::packet(&window, &debt, n);
        pkt.station = crate::inventory_sync::station_wire(self.server.players[i].station);
        self.send_to_joined_slot(
            i,
            &protocol::serialize_packet(protocol::PacketType::InventorySync, &pkt),
        );
        n
    }

    pub(super) fn joiner_requests_blocked(&self, i: usize) -> bool {
        self.server.players.get(i).is_some_and(|sp| {
            sp.server_simulated && (sp.combat.dead || sp.window_events.death_replace.is_some())
        })
    }

    /// Retrieve only the stacks that fit the effective window. Graves are ownerless.
    pub(super) fn serve_open_grave(&mut self, i: usize, seq: u32, cell: (i32, i32, i32)) {
        use crate::item_actions::ItemNote;
        let Some(sp) = self.server.players.get(i) else {
            return;
        };
        if !sp.is_present_and_alive() {
            return self.refuse_item_action(i, seq, ItemNote::NotNow);
        }
        if !crate::container_window::container_in_server_reach(
            sp.player.eye_pos(),
            [cell.0, cell.1, cell.2],
        ) {
            return self.refuse_item_action(i, seq, ItemNote::OutOfReach);
        }
        if self.server.world.get_block(cell.0, cell.1, cell.2) != crate::block::GRAVE {
            return self.refuse_item_action(i, seq, ItemNote::NotThatBlock);
        }
        let Some(mut grave) = self.server.world.grave_at(cell).cloned() else {
            return self.refuse_item_action(i, seq, ItemNote::NotThatBlock);
        };
        let mut inv = crate::window_events::effective_inventory(sp);
        let mut grants = Vec::new();
        for slot in &mut grave.slots {
            let Some(stack) = slot.take() else { continue };
            // A host's grave can contain a Plan whose body cannot be granted.
            if matches!(stack.item, crate::item::Item::Plan(_)) {
                *slot = Some(stack);
                continue;
            }
            let rest = inv.add_item(stack.clone());
            let fitted = stack
                .count
                .saturating_sub(rest.as_ref().map_or(0, |s| s.count));
            *slot = rest;
            if fitted > 0 {
                grants.push(crate::item::ItemStack {
                    item: stack.item,
                    count: fitted,
                });
            }
        }
        let empty = grave.is_empty();
        if empty {
            self.server.world.remove_block_entity(cell);
            self.server
                .world
                .set_block(cell.0, cell.1, cell.2, crate::block::AIR);
            self.server
                .pending_block_changes
                .push(crate::game_loop::broadcast_change(
                    &self.server.world,
                    cell.0,
                    cell.1,
                    cell.2,
                    crate::block::AIR,
                ));
        } else {
            self.server.world.insert_grave(cell, grave);
        }
        for stack in grants {
            self.grant_to_joiner(i, [stack]);
            let events = &mut self.server.players[i].window_events;
            if events.grave_grants.len() >= crate::window_events::MAX_PENDING_EVENTS + crate::window_events::RECENT_GRANTS {
                events.grave_grants.pop_front();
            }
            events.grave_grants.push_back((events.last_number(), cell));
        }
        self.send_item_outcome(
            i,
            &protocol::ItemActionOutcomePacket {
                seq,
                accepted: true,
                consume_held: 0,
                note: if empty {
                    ItemNote::None
                } else {
                    ItemNote::InventoryFull
                }
                .to_wire(),
                window_event: 0,
                wear_held: false,
                bite_after: 0,
            },
        );
    }
}
