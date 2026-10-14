//! One pack flight; decode and actual world/window application are distinct.
use super::{ConnectionState, RemoteClient};
use crate::{action_identity::{ActionFate, ActionHeader}, action_wire::ActionReceipt, pack_wire::{PackRequest, PackResult}};
use sha2::{Digest, Sha256};

/// Reserved at decode and released only at actual window application.
/// It survives every FIFO/inbox ownership transfer and actor-life change.
#[derive(Default)]
pub(super) struct PackBuffer { count: usize, bytes: usize }

impl PackBuffer {
    pub(super) fn reserve(&mut self, bytes: usize) -> Result<(), String> {
        let count = self.count.checked_add(1).ok_or("Pack carrier count overflow")?;
        let total = self.bytes.checked_add(bytes).ok_or("Pack carrier bytes overflow")?;
        if count > crate::action_journal::MAX_EVENT_ENTRIES || total > crate::action_journal::MAX_RETAINED_BYTES { return Err("Pack carriers exceed their retained limit".into()); }
        self.count = count;
        self.bytes = total;
        Ok(())
    }
}

pub(super) struct PackFlight {
    pub(super) action: ActionHeader,
    pub(super) actor_respawns: u32,
    pub(super) received: Option<(u32, ActionFate, [u8; 32])>,
    pub(super) world_ready: bool,
    pub(super) requires_personal: bool,
}

impl RemoteClient {
    pub fn pack_buffers_live(&self) -> bool {
        // Imported initial state uses the same aggregate owned-byte/count
        // reservations while its genuine first whole window is held.
        self.pack_request_pending() || self.pack_buffer.count != 0 || self.initial_personal_window_waits()
    }

    /// Cross-frame reservation, intentionally conservative while a pack
    /// carrier is also present in the separately measured held window inbox.
    pub fn pack_retained_usage(&self) -> (usize, usize) {
        let count = self.pack_buffer.count;
        let bytes = self.pack_buffer.bytes.saturating_add(self.chunk_queue.capacity().saturating_mul(std::mem::size_of::<(usize, crate::chunk_intake::StreamItem)>()));
        let (personal_count, personal_bytes) = self.personal_inbox.as_ref().map_or((0, 0), |inbox| (inbox.pending_count(), inbox.owned_bytes()));
        (count.saturating_add(personal_count), bytes.saturating_add(personal_bytes))
    }

    pub fn pack_carrier_applied(&mut self, owned_bytes: usize) {
        self.pack_buffer.count = self.pack_buffer.count.saturating_sub(1);
        self.pack_buffer.bytes = self.pack_buffer.bytes.saturating_sub(owned_bytes);
    }

    pub(super) fn check_pack_poll_buffer(&self) -> Result<(), String> {
        if !self.pack_buffers_live() { return Ok(()); }
        let external = self.pack_retained_usage();
        let count = external.0.saturating_add(self.pending_outcomes.len())
            .saturating_add(self.pending_grants.len()).saturating_add(self.pending_life_events.len());
        // Conservatively charge the pack heap again while a poll still owns
        // it. Include actual outer Vec metadata in both transport/FIFO paths.
        let bytes = crate::window_events::window_carrier_bytes(&self.pending_outcomes, &self.pending_grants, 0)
            .saturating_add(external.1)
            .saturating_add(self.pending_life_events.capacity().saturating_mul(std::mem::size_of::<super::OwnLifeEvent>()))
            ;
        if count > crate::action_journal::MAX_EVENT_ENTRIES || bytes > crate::action_journal::MAX_RETAINED_BYTES { return Err("Held pack poll exceeds its retained limit".into()); }
        Ok(())
    }

    pub fn pack_view_namespace_matches(&self, packet: &crate::pack_wire::PackViewPush) -> bool {
        self.action_journal.as_ref().and_then(|journal| journal.next_header(self.events_applied).ok())
            .is_some_and(|header| header.session == packet.session && header.epoch == packet.epoch && header.life == packet.life)
    }

    pub(super) fn receive_pack_view(&mut self, packet: crate::pack_wire::PackViewPush) -> Result<(), String> {
        if !self.pack_view_namespace_matches(&packet) { return Err("Pack view names another action namespace".into()); }
        self.pack_buffer.reserve(packet.owned_bytes())?;
        self.pending_outcomes.try_reserve_exact(1).map_err(|_| "Pack view allocation failed")?;
        self.pending_outcomes.push(super::RequestOutcome::PackView(Box::new(packet)));
        self.check_pack_poll_buffer()
    }

    pub fn pack_request_pending(&self) -> bool { self.pack_flight.is_some() }
    /// This gates only window application. The world FIFO/control intake
    /// stays live, so the result can reach its own real application boundary.
    pub fn pack_window_inbox_held(&self) -> bool { self.pack_flight.as_ref().is_some_and(|flight| !flight.world_ready) }

    pub fn reserve_pack_request(&mut self, request: PackRequest) -> Result<ActionHeader, String> {
        if !self.is_connected() || self.pack_request_pending() || self.respawn_unanswered() { return Err("Pack action is waiting".into()); }
        if request.actor_respawns != self.applied_actor_respawns() { return Err("Pack action names an old body".into()); }
        let actor_respawns = request.actor_respawns;
        let action = self.reserve_action(crate::action_wire::ActionIntent::Pack(request), Box::new([]), Box::new([]))?;
        self.pack_flight = Some(PackFlight { action, actor_respawns, received: None, world_ready: false, requires_personal: false });
        Ok(action)
    }

    fn retired_pack_result(&self, packet: &PackResult) -> bool {
        self.action_journal.as_ref().is_some_and(|journal| journal.retired_receipt_is_duplicate(ActionReceipt { action: packet.action, fate: packet.fate, included_events: packet.window_event }))
    }

    pub(super) fn receive_pack_result(&mut self, payload: &[u8]) -> Result<bool, String> {
        let packet = PackResult::decode(payload)?;
        if self.pack_flight.as_ref().is_none_or(|flight| flight.action != packet.action || flight.requires_personal) {
            return if self.retired_pack_result(&packet) { Ok(false) } else { Err("Pack result has no matching request".into()) };
        }
        let hash: [u8; 32] = Sha256::digest(payload).into();
        let flight = self.pack_flight.as_mut().expect("matching flight was checked");
        if packet.actor_respawns != flight.actor_respawns { return Err("Pack result changed its actor life".into()); }
        let identity = (packet.window_event, packet.fate, hash);
        if let Some(received) = flight.received {
            return if received == identity { Ok(false) } else { Err("A repeated pack result changed its outcome".into()) };
        }
        self.pack_buffer.reserve(packet.owned_bytes())?;
        self.chunk_queue.try_reserve_exact(1).map_err(|_| "Pack result FIFO allocation failed")?;
        flight.received = Some(identity);
        self.chunk_queue.push((self.pending_block_changes.len(), crate::chunk_intake::StreamItem::PackResult(Box::new(packet))));
        Ok(true)
    }

    /// GameState's actual world FIFO intake calls this, even while its
    /// window inbox is held. A stale body's state is skipped later at actual
    /// window application; its exact terminal metadata still retires.
    pub fn apply_pack_result_intake(&mut self, packet: &PackResult) -> Result<bool, String> {
        if self.pack_flight.as_ref().is_none_or(|flight| flight.action != packet.action || flight.requires_personal) {
            return if self.retired_pack_result(packet) { Ok(false) } else { Err("Pack intake has no matching request".into()) };
        }
        let hash: [u8; 32] = Sha256::digest(packet.encode()?.as_ref()).into();
        let flight = self.pack_flight.as_mut().expect("matching flight was checked");
        if flight.received != Some((packet.window_event, packet.fate, hash)) { return Err("Pack intake differs from its received carrier".into()); }
        if flight.world_ready { return Ok(false); }
        flight.world_ready = true;
        Ok(true)
    }

    pub(super) fn pack_receipt_ready(&self, receipt: ActionReceipt) -> Result<bool, String> {
        let Some(flight) = self.pack_flight.as_ref().filter(|flight| flight.action == receipt.action) else { return Ok(true) };
        let Some((event, fate, _)) = flight.received else { return Ok(false) };
        if receipt.included_events != event || receipt.fate != fate { return Err("Pack receipt differs from its exact result".into()); }
        let ready = flight.world_ready && self.events_applied >= event;
        if ready && flight.requires_personal {
            return Ok(self.personal_inbox.as_ref().is_some_and(|inbox| inbox.applied_through(event, self.applied_actor_respawns())));
        }
        Ok(ready && !flight.requires_personal)
    }

    pub(super) fn retire_pack_flight(&mut self, receipt: ActionReceipt) {
        if self.pack_flight.as_ref().is_some_and(|flight| flight.action == receipt.action) { self.pack_flight = None; }
    }

    pub(super) fn pack_carrier_failed(&mut self, reason: String) {
        log::warn!("Invalid pack carrier: {reason}");
        self.state = ConnectionState::Failed(super::HOST_BAD_WORLD_DATA.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{action_identity::InventoryMode, container_window::ContainerTarget, pack_wire::{PackOperation, PackViewPush}, transport::{channel_pair, ServerTransport}};

    fn client(mode: InventoryMode) -> (crate::transport::ChannelServerTransport, RemoteClient) {
        let (server, transport) = channel_pair();
        let mut remote = RemoteClient::from_transport(Box::new(transport), super::super::build_join_request_guest("Guest", 0), None);
        remote.state = ConnectionState::Connected { player_index: 1, seed: 0 };
        remote.inventory_mode = mode;
        remote.action_journal = Some(crate::action_journal::ActionJournal::new([7; 16], 1));
        (server, remote)
    }

    fn request(remote: &mut RemoteClient) -> ActionHeader {
        remote.reserve_pack_request(PackRequest { actor_respawns: remote.applied_actor_respawns(), target: ContainerTarget::Entity(9), hotbar_slot: 0, before: 0, operation: PackOperation::Open }).unwrap()
    }

    fn result(action: ActionHeader, life: u32) -> PackResult {
        let registry = crate::block::BlockRegistry::new();
        let window = crate::window_events::EffectiveWindow { inv: crate::inventory::Inventory::new(), armour: [None; 4], cursor: None, grid: Default::default() };
        PackResult { actor_respawns: life, action, fate: ActionFate::Accepted, note: crate::pack_wire::PackNote::Done, window_event: 2, window: crate::pack_wire::PackWindow::capture(&window, &Default::default(), &registry).unwrap(), view: None }
    }

    #[test]
    fn d2c2_pack_held_flight_flood_fails_with_reason_in_both_modes() {
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let (server, mut remote) = client(mode);
            request(&mut remote);
            for event in 1..=crate::action_journal::MAX_EVENT_ENTRIES as u32 + 1 {
                let push = PackViewPush { actor_respawns: remote.applied_actor_respawns(), session: [7; 16], epoch: 0, life: 1, window_event: event, target: ContainerTarget::Entity(9), incarnation: 1, view: None };
                server.send_to_client(&crate::protocol::serialize_packet(crate::protocol::PacketType::PackView, &push));
            }
            remote.poll();
            assert!(matches!(&remote.state, ConnectionState::Failed(reason) if reason == super::super::HOST_BAD_WORLD_DATA));
            assert!(remote.pack_window_inbox_held(), "flood cannot release its own flight");
            assert!(remote.pack_buffer.count <= crate::action_journal::MAX_EVENT_ENTRIES);
            assert!(remote.pack_buffer.bytes <= crate::action_journal::MAX_RETAINED_BYTES);
        }
    }

    #[test]
    fn d2c2_pack_reservation_survives_decode_fifo_and_inbox_until_application() {
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let (server, mut remote) = client(mode);
            let action = request(&mut remote);
            let grant = |event| crate::protocol::InventoryGrantPacket { item_kind: crate::protocol::item_kind::BLOCK, item_id: crate::block::STONE, count: 1, full_item: crate::protocol::WireItem::None, window_event: event };
            server.send_to_client(&crate::protocol::serialize_packet(crate::protocol::PacketType::InventoryGrant, &grant(1)));
            remote.poll();
            let packet = result(action, remote.applied_actor_respawns());
            let bytes = packet.encode().unwrap();
            assert!(remote.receive_pack_result(&bytes).unwrap());
            assert!(!remote.receive_pack_result(&bytes).unwrap(), "identical result reserves no second body");
            assert!(remote.pack_window_inbox_held(), "decode is not world intake");
            server.send_to_client(&crate::protocol::serialize_packet(crate::protocol::PacketType::InventoryGrant, &grant(4)));
            server.send_to_client(&crate::protocol::serialize_packet(crate::protocol::PacketType::PlayerEvent, &crate::protocol::PlayerEventPacket { player_index: 1, event: crate::protocol::PlayerEventType::ArmourWorn { hits: 1 }, window_event: 3 }));
            remote.poll();
            assert!(remote.pack_window_inbox_held(), "later stock and wear do not turn decode into intake");
            assert_eq!(remote.pack_buffer.count, 1);
            remote.queued_action_receipts = 1;
            remote.apply_action_receipt(ActionReceipt { action, fate: packet.fate, included_events: packet.window_event });
            remote.note_window_event(1);
            assert!(remote.pack_request_pending(), "an early receipt cannot release its guard");
            let (_, crate::chunk_intake::StreamItem::PackResult(actual)) = remote.chunk_queue.pop().unwrap() else { panic!("actual FIFO carrier") };
            assert!(remote.apply_pack_result_intake(&actual).unwrap());
            assert!(!remote.pack_window_inbox_held(), "world intake opens window ordering");
            assert!(remote.pack_request_pending(), "physical mutation still waits for its applied fence");
            let mut inbox = crate::window_events::WindowInbox::default();
            let grants = std::mem::take(&mut remote.pending_grants);
            let armour = std::mem::take(&mut remote.pending_life_events).into_iter().filter_map(|event| match event { super::super::OwnLifeEvent::ArmourWorn(hits, number) => Some((hits, number)), _ => None }).collect();
            inbox.add_bounded(vec![super::super::RequestOutcome::Pack(actual)], grants, armour, remote.pack_retained_usage()).unwrap();
            assert_eq!(remote.pack_buffer.count, 1, "moving to the inbox retains ownership accounting");
            let (items, _) = inbox.take_ordered();
            assert_eq!(items.iter().map(crate::window_events::InboxItem::event).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
            let crate::window_events::InboxItem::Outcome(super::super::RequestOutcome::Pack(actual)) = &items[1] else { panic!("canonical fence precedes later wear") };
            remote.pack_carrier_applied(actual.owned_bytes());
            remote.note_window_event(2);
            assert!(!remote.pack_request_pending(), "only the actual applied fence retires its receipt");
            assert_eq!((remote.pack_buffer.count, remote.pack_buffer.bytes), (0, 0));
        }
    }
}
