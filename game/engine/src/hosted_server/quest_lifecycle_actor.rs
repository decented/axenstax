//! One real attached client; each physical and personal frontier is independent.
use super::*;
use crate::{
    action_identity::ActionFrontier,
    action_wire::{ActionPacket, AppliedActionAck},
    remote_client::RequestOutcome,
    transport::ClientTransport as _,
    window_events::{InboxItem, WindowInbox},
};

pub(crate) struct Actor {
    pub client: ChannelClientTransport,
    pub accepted: protocol::JoinAcceptPacket,
    pub slot: crate::player_slot::PlayerSlot,
    pub personal: crate::personal_sync::Inbox,
    pub physical: WindowInbox,
    pub applied: u32,
    pub initial_carriers: Vec<(u32, &'static str)>,
    pub frontier: ActionFrontier,
    pub results: Vec<crate::quest_wire::QuestResult>,
    pub kills: Vec<protocol::KillEventPacket>,
    pub outcomes: Vec<protocol::InteractOutcomePacket>,
    pub view: Option<crate::quest_wire::QuestView>,
    pub rejects: Vec<String>,
    receipts: Vec<crate::action_wire::ActionReceipt>,
    sent: Vec<ActionPacket>,
    pub active: bool,
    pub legacy_seq: u32,
    pub input_seq: u64,
}
impl Actor {
    pub fn new(
        client: ChannelClientTransport,
        accepted: protocol::JoinAcceptPacket,
        slot: crate::player_slot::PlayerSlot,
        personal: crate::personal_sync::Inbox,
        applied: u32,
    ) -> Self {
        Self {
            frontier: ActionFrontier::new(accepted.action_session, accepted.action_life),
            client,
            accepted,
            slot,
            personal,
            physical: Default::default(),
            applied,
            initial_carriers: Vec::new(),
            results: Vec::new(),
            kills: Vec::new(),
            outcomes: Vec::new(),
            view: None,
            rejects: Vec::new(),
            receipts: Vec::new(),
            sent: Vec::new(),
            active: true,
            legacy_seq: 0,
            input_seq: 0,
        }
    }
    pub fn i(&self) -> usize {
        self.accepted.player_index as usize
    }
    pub fn send(
        &mut self,
        intent: crate::action_wire::ActionIntent,
    ) -> crate::action_identity::ActionHeader {
        assert!(
            self.sent.is_empty(),
            "previous actual receipt must settle first"
        );
        let action = self.frontier.header(
            self.frontier.through().checked_add(1).unwrap(),
            self.applied,
        );
        let packet = ActionPacket { action, intent };
        self.client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::Action,
            &packet,
        ));
        self.sent.push(packet);
        action
    }
    fn ack(&self, value: protocol::PersonalAck) {
        self.client.send_to_server(&protocol::serialize_packet(
            protocol::PacketType::PersonalSync,
            &protocol::PersonalSyncPacket::Applied(value),
        ));
    }
    pub fn drain(&mut self, registry: &crate::block::BlockRegistry) {
        let mut outcomes = Vec::new();
        let mut grants = Vec::new();
        while let Some(bytes) = self.client.try_recv_from_server() {
            match protocol::deserialize_header(&bytes) {
                Some((protocol::PacketType::InventorySync, body)) => outcomes.push(
                    RequestOutcome::Sync(protocol::safe_deserialize(body).unwrap()),
                ),
                Some((protocol::PacketType::QuestResult, body)) => {
                    let result = crate::quest_wire::QuestResult::decode(body).unwrap();
                    assert!(
                        self.sent
                            .iter()
                            .any(|packet| packet.action == result.action)
                    );
                    self.results.push(result.clone());
                    outcomes.push(RequestOutcome::Quest(Box::new(result)));
                }
                Some((protocol::PacketType::QuestView, body)) => {
                    let view: crate::quest_wire::QuestViewPush =
                        protocol::safe_deserialize(body).unwrap();
                    view.validate().unwrap();
                    assert_eq!(view.session, self.accepted.action_session);
                    assert_eq!(view.epoch, self.frontier.header(0, 0).epoch);
                    assert_eq!(view.life, self.accepted.action_life);
                    assert_eq!(view.actor_respawns, self.accepted.action_life);
                    outcomes.push(RequestOutcome::QuestView(Box::new(view)));
                }
                Some((protocol::PacketType::PersonalSync, body)) => {
                    if let protocol::PersonalSyncPacket::Snapshot(snapshot) =
                        protocol::safe_deserialize(body).unwrap()
                    {
                        assert_eq!(snapshot.session, self.accepted.action_session);
                        assert_eq!(snapshot.life, self.accepted.action_life);
                        let event = snapshot.window_event;
                        if let Some(ack) = self.personal.push(snapshot).unwrap() {
                            self.ack(ack);
                        }
                        outcomes.push(RequestOutcome::PersonalSync {
                            window_event: event,
                            actor_respawns: self.accepted.action_life,
                        });
                    }
                }
                Some((protocol::PacketType::InventoryGrant, body)) => {
                    grants.push(protocol::safe_deserialize(body).unwrap())
                }
                Some((protocol::PacketType::InteractOutcome, body)) => {
                    let result: protocol::InteractOutcomePacket =
                        protocol::safe_deserialize(body).unwrap();
                    // These clients really use an empty hand: no wear or cost is omitted.
                    assert_eq!(result.window_event, 0);
                    assert_eq!(result.consume_held, 0);
                    self.outcomes.push(result);
                }
                Some((protocol::PacketType::ActionReceipt, body)) => self
                    .receipts
                    .push(protocol::safe_deserialize(body).unwrap()),
                Some((protocol::PacketType::KillEvent, body)) => {
                    self.kills.push(protocol::safe_deserialize(body).unwrap())
                }
                Some((protocol::PacketType::JoinReject, body)) => self.rejects.push(
                    protocol::safe_deserialize::<protocol::JoinRejectPacket>(body)
                        .unwrap()
                        .reason,
                ),
                _ => {}
            }
        }
        self.physical.add(outcomes, grants, Vec::new());
        let (items, _) = self.physical.take_ordered();
        for item in items {
            let event = item.event();
            if event <= self.applied {
                continue;
            }
            assert_eq!(
                event,
                self.applied + 1,
                "an actual physical carrier cannot skip a missing event"
            );
            let p = &mut self.slot;
            match item {
                InboxItem::Outcome(RequestOutcome::Sync(sync)) => {
                    if event == 1 {
                        self.initial_carriers.push((event, "sync"));
                    }
                    let station = crate::inventory_sync::apply(
                        &mut crate::window::WindowMut {
                            inv: &mut p.inventory,
                            armour: &mut p.armour_slots,
                            cursor: &mut p.crafting_ui.cursor_item,
                            grid: &mut p.crafting_ui.grid,
                            container: None,
                        },
                        &sync,
                        registry,
                        &mut Default::default(),
                        &mut self.physical.debt,
                        self.accepted.inventory_mode,
                    )
                    .unwrap();
                    if let Some(station) = station {
                        p.crafting_ui.adopt_station(station);
                    }
                }
                InboxItem::Outcome(RequestOutcome::Quest(result)) => {
                    crate::pack_client::apply_canonical_window(
                        &mut crate::window::WindowMut {
                            inv: &mut p.inventory,
                            armour: &mut p.armour_slots,
                            cursor: &mut p.crafting_ui.cursor_item,
                            grid: &mut p.crafting_ui.grid,
                            container: None,
                        },
                        &result.window,
                        registry,
                        &Default::default(),
                        &mut self.physical.debt,
                        self.accepted.inventory_mode,
                    )
                    .unwrap();
                    self.view = result.view.clone();
                }
                InboxItem::Outcome(RequestOutcome::QuestView(view)) => {
                    if let Some(current) = &self.view {
                        assert_eq!(
                            (view.target, view.incarnation),
                            (current.target, current.incarnation)
                        );
                    }
                    self.view = view.view;
                }
                InboxItem::Grant(grant) => assert_eq!(
                    crate::remote_entities::apply_inventory_grant(
                        &mut p.inventory,
                        &grant,
                        registry
                    ),
                    Some(0),
                    "real small grants fit this actual bag"
                ),
                InboxItem::Outcome(RequestOutcome::PersonalSync { window_event, .. }) => {
                    let acknowledgements = self.personal.apply(window_event, p).unwrap();
                    if event == 2 {
                        self.initial_carriers.push((event, "personal"));
                    }
                    for ack in acknowledgements {
                        self.ack(ack);
                    }
                }
                _ => panic!("unexpected physical carrier in this actual bare-hand lifecycle"),
            }
            self.applied = event;
        }
        let mut waiting = Vec::new();
        for receipt in std::mem::take(&mut self.receipts) {
            if receipt.included_events > self.applied {
                waiting.push(receipt);
                continue;
            }
            let at = self
                .sent
                .iter()
                .position(|packet| packet.action == receipt.action)
                .expect("actual own receipt");
            self.sent.remove(at);
            self.frontier
                .terminal(receipt.action, receipt.fate)
                .unwrap();
            self.client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::AppliedActionAck,
                &AppliedActionAck {
                    session: receipt.action.session,
                    epoch: receipt.action.epoch,
                    life: receipt.action.life,
                    through: receipt.action.seq,
                    included_events: receipt.included_events,
                },
            ));
        }
        self.receipts = waiting;
    }
    pub fn idle(&self) -> bool {
        self.sent.is_empty() && self.receipts.is_empty() && self.physical.is_empty()
    }
}
