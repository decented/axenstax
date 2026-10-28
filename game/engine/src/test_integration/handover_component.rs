//! Renderer-free session fixture. Real admission/window/Personal/transport owners;
//! no GameState/UI/current-carry acceptance is attributed to this component driver.
use crate::{
    chunk_intake::{ChunkIntake, Delta},
    handover::live::{
        recipient::{RecipientConsent, RecipientSession, RecipientStage},
        wire::Frame,
    },
    player_slot::PlayerSlot,
    protocol::{self, PacketType},
    remote_client::{RemoteClient, RequestOutcome},
    transport::ClientTransport,
    window_events::{InboxItem, WindowInbox},
};
use std::collections::VecDeque;

pub(super) fn signed_request(
    client: &dyn ClientTransport,
    secret: [u8; 32],
) -> protocol::JoinRequestPacket {
    let bytes = client
        .try_recv_from_server()
        .expect("actual attach challenge");
    let Some((PacketType::Challenge, body)) = protocol::deserialize_header(&bytes) else {
        panic!("no challenge")
    };
    let challenge: protocol::ChallengePacket = protocol::safe_deserialize(body).unwrap();
    let mut request = crate::remote_client::build_join_request_guest("Session component", 0);
    request.auth_event = Some(
        pollster::block_on(crate::signet::native_signer::build_join_auth_event(
            &nostr::Keys::new(nostr::SecretKey::from_slice(&secret).unwrap()),
            &challenge.nonce_hex,
            &crate::signet::join_origin(client.channel_binding()),
        ))
        .unwrap(),
    );
    request
}

pub(super) struct Application {
    pub slot: PlayerSlot,
    pub window: WindowInbox,
    world: crate::world::World,
    intake: ChunkIntake,
    loaded: ahash::AHashSet<(i32, i32)>,
    items: crate::remote_entities::RemoteItems,
    projectiles: crate::remote_entities::RemoteProjectiles,
    mobs: crate::remote_mobs::RemoteMobs,
    mode: crate::play_mode::PlayMode,
    difficulty: String,
    clock: u32,
}

impl Application {
    pub(super) fn new() -> Self {
        Self {
            slot: PlayerSlot::new(0, glam::Vec3::ZERO, 1.0),
            window: WindowInbox::default(),
            world: crate::world::World::new(),
            intake: ChunkIntake::default(),
            loaded: Default::default(),
            items: Default::default(),
            projectiles: Default::default(),
            mobs: Default::default(),
            mode: crate::play_mode::PlayMode::default(),
            difficulty: String::new(),
            clock: 0,
        }
    }

    pub(super) fn ready(&self, remote: &RemoteClient) -> bool {
        remote.personal_ready()
            && remote.handover_authority_settled()
            && self.window.is_empty()
            && !self.intake.has_carried_world_work()
    }

    /// Only this fixture's declared passive session traffic is supported.
    /// Unexpected action/life/feedback work fails instead of being drained away.
    pub(super) fn receive(
        &mut self,
        remote: &mut RemoteClient,
        registry: &crate::block::BlockRegistry,
    ) {
        remote.handover_world_pending(self.intake.has_carried_world_work());
        remote.poll();
        assert!(
            remote.is_connected(),
            "actual signed admission failed: {:?}",
            remote.state
        );
        if let Some(joined) = remote.pending_joined_world.take() {
            self.intake.expect_notes(
                joined.chunk_note_radius,
                joined
                    .spawn
                    .map(crate::chunk_stream::column_of)
                    .unwrap_or((0, 0)),
            );
        }
        let chunks = std::mem::take(&mut remote.chunk_queue);
        let changes = std::mem::take(&mut remote.pending_block_changes);
        assert_eq!(
            remote.undecodable_chunks, 0,
            "no malformed world data in a positive fixture"
        );
        for step in self.intake.plan_deltas(chunks, &changes, 4) {
            match step {
                Delta::Chunk(packet) => {
                    self.intake
                        .apply(&mut self.world, &mut self.loaded, registry, &packet);
                }
                Delta::Local(column, hash) => self.intake.note_local(column, hash),
                Delta::Change(change) => {
                    self.world.apply_remote_block_change(&change);
                }
                Delta::View(view) => {
                    crate::block_views::apply_view(&mut self.world, registry, &view);
                }
                Delta::Attachment(change) => {
                    crate::chunk_intake::apply_attachment_change(&mut self.world, &change);
                }
                _ => panic!("undeclared action or recovery in passive session component"),
            }
        }
        crate::remote_entities::apply_entity_batches(
            &std::mem::take(&mut remote.pending_entity_batches),
            &mut self.items,
            &mut self.projectiles,
            Some(&mut self.mobs),
        );
        if let Some(mode) = remote.pending_play_mode.take() {
            self.mode = mode;
        }
        if let Some(difficulty) = remote.pending_difficulty.take() {
            self.difficulty = difficulty;
        }
        if let Some(clock) = remote.pending_world_time.take() {
            self.clock = clock;
        }
        if let Some(exhibits) = remote.pending_exhibits.take() {
            self.world.exhibits = exhibits;
        }
        if let Some(state) = remote.latest_state.take() {
            self.clock = state.world_time;
            crate::health_sync::apply_own_hunger(&mut self.slot.combat, true, state.own_hunger);
            self.intake.confirm_drops(state.last_acked_input);
            self.window.acknowledged(state.last_acked_input);
        }
        assert!(
            remote.pending_life_events.is_empty()
                && remote.pending_kills.is_empty()
                && remote.pending_refused_uses.is_empty()
                && remote.pending_resource_pack.is_none()
                && remote.pending_operator_snapshot_json.is_none()
                && remote.pending_chat.is_empty(),
            "undeclared life/action/presentation traffic in passive session component"
        );
        self.window.add(
            std::mem::take(&mut remote.pending_outcomes),
            std::mem::take(&mut remote.pending_grants),
            Vec::new(),
        );
        if !remote.initial_personal_window_waits()
            || self.window.prioritise_initial_personal_replace()
        {
            let (items, _) = self.window.take_ordered();
            for item in items {
                let event = item.event();
                match item {
                    InboxItem::Outcome(RequestOutcome::Sync(sync)) => {
                        let slot = &mut self.slot;
                        crate::inventory_sync::apply(
                            &mut crate::window::WindowMut {
                                inv: &mut slot.inventory,
                                armour: &mut slot.armour_slots,
                                cursor: &mut slot.crafting_ui.cursor_item,
                                grid: &mut slot.crafting_ui.grid,
                                container: None,
                            },
                            &sync,
                            registry,
                            &mut Default::default(),
                            &mut self.window.debt,
                            remote.inventory_mode,
                        )
                        .unwrap();
                        remote.note_initial_personal_window_applied(event, sync.locks);
                    }
                    InboxItem::Outcome(RequestOutcome::PersonalSync { .. }) => {}
                    InboxItem::Grant(grant) => assert_eq!(
                        crate::remote_entities::apply_inventory_grant(
                            &mut self.slot.inventory,
                            &grant,
                            registry
                        ),
                        Some(0),
                        "declared passive bag fits its actual grant"
                    ),
                    _ => panic!("undeclared window action in passive session component"),
                }
                remote.note_window_event(event); // Only after this actual item applied.
            }
        }
        if !remote.initial_personal_window_waits() {
            let events = remote.applied_window_event();
            if let Some(inbox) = remote.personal_inbox.as_mut() {
                let batch = inbox
                    .prepare_apply(
                        events,
                        &mut self.slot,
                        (
                            0,
                            self.window.retained_usage().1 + self.intake.carried_metadata_bytes(),
                        ),
                    )
                    .unwrap()
                    .commit();
                for wire in batch.wires() {
                    remote.send_raw(wire);
                }
            }
            remote.retire_applied_receipts();
        }
        remote.handover_application_completed(self.intake.has_carried_world_work());
    }

    pub(super) fn consent(
        &mut self,
        remote: &mut RemoteClient,
        session: &mut RecipientSession,
        now: u64,
    ) {
        let p = session.proposal();
        let consent = RecipientConsent {
            transfer: p.transfer,
            world_name: p.world_name.clone(),
            source_npub: p.source_npub.clone(),
        };
        let wire = session
            .consent_normal(
                consent,
                std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
                now,
            )
            .unwrap();
        let (participant, runtime) = session.park_identity();
        remote
            .begin_handover_drain(
                session.proposal().transfer,
                session.park_authority().clone(),
                participant,
                runtime,
                self.intake.has_carried_world_work(),
            )
            .unwrap();
        remote.send_raw(&wire);
        assert_eq!(session.stage(), RecipientStage::Draining);
        assert!(!remote.is_parked());
    }

    pub(super) fn progress(
        &mut self,
        remote: &mut RemoteClient,
        session: &mut RecipientSession,
        registry: &crate::block::BlockRegistry,
        now: u64,
    ) {
        if session.stage() == RecipientStage::Draining {
            session.check_normal_wait(now).unwrap();
            self.receive(remote, registry);
            for bytes in remote.take_handover_frames() {
                assert!(
                    matches!(Frame::decode(&bytes), Ok(Frame::Notice { .. })),
                    "unexpected pre-park control"
                );
            }
            let transfer = session.proposal().transfer;
            assert!(remote.handover_drain_failure(transfer).is_none());
            if let Some(fence) = remote.handover_drain_fence(transfer)
                && remote.handover_drain_application_ready(transfer)
                && self.window.is_empty()
                && !self.intake.has_carried_world_work()
            {
                let mut link = Some(remote.park_for_personal_control(transfer).unwrap());
                let wire = match session.prepare_parked(&fence, &mut link, remote, now) {
                    Ok(prepared) => prepared.commit(),
                    Err(error) => {
                        let (transport, _) =
                            link.take().expect("retained actual owner").into_parts();
                        remote.restore_transport(transport, VecDeque::new());
                        panic!("actual component park proof refused: {error:?}");
                    }
                };
                remote.cancel_handover_drain(transfer);
                session.send_parked_wire(&wire);
            }
        } else {
            session.pump(now).unwrap();
            session
                .pump_personal_control(
                    remote,
                    &mut self.window,
                    &mut self.slot,
                    (0, self.intake.carried_metadata_bytes()),
                    now,
                )
                .unwrap();
        }
    }
}
