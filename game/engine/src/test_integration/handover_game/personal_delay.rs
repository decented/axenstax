//! Actual signed imported-world client initialisation in both delivery orders.
//! Only the injected channel delays genuine server packets; no ready flag,
//! personal holder, window namespace or acknowledgement is fabricated.
use super::*;
use crate::action_identity::InventoryMode;
use crate::window::{ClickResult, WindowClick};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Delayed {
    replace_allowed: bool,
    snapshot_allowed: bool,
    replace: Option<Vec<u8>>,
    snapshot: Option<Vec<u8>>,
    injected: VecDeque<Vec<u8>>,
    sent: Vec<Vec<u8>>,
    accept: Option<protocol::JoinAcceptPacket>,
    first_replace: Option<protocol::InventorySyncPacket>,
    first_snapshot: Option<protocol::PersonalSnapshot>,
}

struct DelayTransport {
    inner: ChannelClientTransport,
    delayed: Arc<Mutex<Delayed>>,
}
impl ClientTransport for DelayTransport {
    fn send_to_server(&self, data: &[u8]) {
        self.delayed.lock().unwrap().sent.push(data.to_vec());
        self.inner.send_to_server(data);
    }
    fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
        let mut delayed = self.delayed.lock().unwrap();
        if let Some(packet) = delayed.injected.pop_front() {
            return Some(packet);
        }
        if delayed.replace_allowed
            && let Some(packet) = delayed.replace.take()
        {
            return Some(packet);
        }
        if delayed.snapshot_allowed
            && let Some(packet) = delayed.snapshot.take()
        {
            return Some(packet);
        }
        while let Some(packet) = self.inner.try_recv_from_server() {
            match protocol::deserialize_header(&packet).unwrap() {
                (PacketType::JoinAccept, bytes) => {
                    delayed.accept = Some(protocol::safe_deserialize(bytes).unwrap());
                }
                (PacketType::InventorySync, bytes) => {
                    let replace: protocol::InventorySyncPacket =
                        protocol::safe_deserialize(bytes).unwrap();
                    if delayed.first_replace.is_none() {
                        delayed.first_replace = Some(replace);
                    }
                    if !delayed.replace_allowed {
                        assert!(
                            delayed.replace.is_none(),
                            "unexpected second Replace before initial installation"
                        );
                        delayed.replace = Some(packet);
                        continue;
                    }
                }
                (PacketType::PersonalSync, bytes) => {
                    let protocol::PersonalSyncPacket::Snapshot(snapshot) =
                        protocol::safe_deserialize(bytes).unwrap()
                    else {
                        panic!("server sent a personal acknowledgement")
                    };
                    if let Some(first) = &delayed.first_snapshot {
                        assert_eq!(
                            snapshot, *first,
                            "initial snapshot changed before its exact Applied"
                        );
                    } else {
                        delayed.first_snapshot = Some(snapshot);
                    }
                    if !delayed.snapshot_allowed {
                        // Reliable retries carry the same exact tuple/body.
                        if let Some(held) = &delayed.snapshot {
                            assert_eq!(held, &packet);
                        } else {
                            delayed.snapshot = Some(packet);
                        }
                        continue;
                    }
                }
                _ => (),
            }
            return Some(packet);
        }
        None
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.inner.channel_binding()
    }
}

fn authorises(kind: PacketType) -> bool {
    matches!(
        kind,
        PacketType::ClientInput
            | PacketType::Action
            | PacketType::WindowOp
            | PacketType::ItemAction
            | PacketType::EntityAttack
            | PacketType::EntityInteract
            | PacketType::DeviceInteract
            | PacketType::Respawn
    )
}
fn holdings(hg: &HeadlessGame) -> (Vec<u8>, crate::window::Station) {
    let p = &hg.state.players[0];
    let slots: Vec<_> = p.inventory.slots_iter().collect();
    let locks: Vec<_> = (0..crate::window::SLOTS)
        .map(|i| p.inventory.is_locked(i))
        .collect();
    let bytes = bincode::serialize(&(
        slots,
        locks,
        p.inventory.auto_refill,
        p.armour_slots,
        &p.crafting_ui.cursor_item,
        &p.crafting_ui.grid,
    ))
    .unwrap();
    (bytes, p.crafting_ui.station())
}
fn next_action(hg: &HeadlessGame) -> crate::action_identity::ActionHeader {
    let rc = hg.state.remote_client.as_ref().unwrap();
    rc.action_journal
        .as_ref()
        .unwrap()
        .next_header(rc.applied_window_boundary())
        .unwrap()
}
fn attempts_wait_without_prediction(
    hg: &mut HeadlessGame,
    delayed: &Arc<Mutex<Delayed>>,
    cell: [i32; 3],
) {
    let before = holdings(hg);
    let action = next_action(hg);
    let next_input = hg.state.remote_client.as_ref().unwrap().next_input_seq();
    let tick = hg.state.tick_counter;
    let pos = hg.state.players[0].player.pos;
    let block = hg.state.world.get_block(cell[0], cell[1], cell[2]);
    let sent_at = delayed.lock().unwrap().sent.len();
    delayed
        .lock()
        .unwrap()
        .injected
        .push_back(protocol::serialize_packet(PacketType::Ping, &()));
    // Actual click entry and actual per-frame/tick input entry, with a real
    // occupied authoritative window in the Replace-first case.
    assert_eq!(
        hg.state.apply_window_click(
            0,
            &WindowClick::Slot {
                slot: 4,
                right: false
            }
        ),
        ClickResult::Refused
    );
    hg.state.input.cursor_captured = true;
    hg.state.input.key_pressed(winit::keyboard::KeyCode::KeyW);
    hg.state.input.key_pressed(winit::keyboard::KeyCode::KeyQ);
    hg.state.input.right_held = true;
    hg.state.players[0].place_cooldown = 0;
    hg.frames(1);
    hg.ticks(2);
    hg.state.network_send_input();
    hg.state.input.right_held = false;
    hg.state.input.key_released(winit::keyboard::KeyCode::KeyW);
    hg.state.input.key_released(winit::keyboard::KeyCode::KeyQ);
    assert_eq!(
        holdings(hg),
        before,
        "an initial held click predicted or spent personal inventory"
    );
    assert_eq!(
        next_action(hg),
        action,
        "a held initial action allocated an identity"
    );
    assert_eq!(
        hg.state.remote_client.as_ref().unwrap().next_input_seq(),
        next_input,
        "held input advanced its sequence"
    );
    assert_eq!(
        hg.state.tick_counter, tick,
        "held client simulation advanced"
    );
    assert_eq!(
        hg.state.players[0].player.pos, pos,
        "held movement predicted before personal installation"
    );
    assert_eq!(
        hg.state.world.get_block(cell[0], cell[1], cell[2]),
        block,
        "held placement predicted a block"
    );
    assert!(hg.state.pending_block_changes.is_empty());
    assert!(hg.state.pending_mined.is_empty());
    assert!(hg.state.players[0].crafting_ui.ops.first_stamp().is_none());
    assert_eq!(
        hg.state.joiner_actions.len(),
        0,
        "held request reserved a spend claim"
    );
    let delayed = delayed.lock().unwrap();
    let sent = &delayed.sent[sent_at..];
    assert!(
        sent.iter()
            .any(|p| protocol::deserialize_header(p).unwrap().0 == PacketType::Pong),
        "control polling stopped behind personal initialisation"
    );
    assert!(
        sent.iter()
            .all(|p| !authorises(protocol::deserialize_header(p).unwrap().0)),
        "held client sent authority before its baseline was applied"
    );
    assert!(
        sent.iter()
            .all(|p| protocol::deserialize_header(p).unwrap().0 != PacketType::PersonalSync),
        "a partial baseline sent Applied"
    );
}

/// The imported world's real canonical source participant supplies Glass9 /
/// Pig7. Signed admission loads that record; GameState runs the ordinary joined
/// Loading/Playing flow, then input and window entry points, in both modes.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_delayed_initial_personal_client_blocks_prediction_and_orders_applied() {
    let _worlds = crate::save::WorldsRootGuard::new("personal-delayed-client");
    let (mut j, _src, _working, transfer) = leave_while_prepared(
        "dst-opens-activated-personal-delayed",
        Some(lobby_imports()),
    );
    let activated = format!("activated-{}.dat", hex::encode(transfer));
    let deadline = Instant::now() + Duration::from_secs(90);
    let folder = loop {
        j.hg.frames(1);
        let crate::GameMode::Menu(menu) = &j.hg.state.mode else {
            panic!("recipient left lobby")
        };
        if let Some(world) = menu.worlds.iter().find(|world| {
            crate::save::world_dir(&world.folder_name)
                .join("handover")
                .join(&activated)
                .is_file()
        }) {
            break world.folder_name.clone();
        }
        assert!(
            Instant::now() < deadline,
            "actual import did not activate/promote"
        );
        std::thread::sleep(Duration::from_millis(1));
    };
    let proof = crate::handover::live::personal_adapter::proof(&crate::save::world_dir(&folder))
        .unwrap()
        .unwrap();
    assert!(
        !proof.is_retained(),
        "fixture substituted retained provenance for activated import"
    );
    assert_eq!(proof.core.source_persona, key(SOURCE_PERSONA));
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for snapshot_first in [false, true] {
            let mut hs = HostedServer::start(
                0,
                folder.clone(),
                42,
                0,
                crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
            )
            .unwrap();
            hs.select_inventory_mode(mode).unwrap();
            hs.server.difficulty = crate::survival::Difficulty::Peaceful;
            let inner = hs.attach_test_remote_with_binding(Some(BINDING));
            let challenge = loop {
                let packet = inner.try_recv_from_server().unwrap();
                if let Some((PacketType::Challenge, bytes)) = protocol::deserialize_header(&packet)
                {
                    break protocol::safe_deserialize::<protocol::ChallengePacket>(bytes).unwrap();
                }
            };
            let mut request =
                crate::remote_client::build_join_request_guest("Source participant", 0);
            request.auth_event = Some(
                pollster::block_on(crate::signet::native_signer::build_join_auth_event(
                    &keys(SOURCE_PERSONA),
                    &challenge.nonce_hex,
                    &crate::signet::join_origin(Some(BINDING)),
                ))
                .unwrap(),
            );
            let delayed = Arc::new(Mutex::new(Delayed::default()));
            j.hg.state.remote_client = Some(RemoteClient::from_transport(
                Box::new(DelayTransport {
                    inner,
                    delayed: delayed.clone(),
                }),
                request,
                None,
            ));
            j.hg.state.world_name = crate::remote_client::JOINED_WORLD_FOLDER.into();
            j.hg.state.mode = crate::GameMode::Loading(crate::loading_screen::LoadingState::new(
                crate::remote_client::JOINED_WORLD_FOLDER.into(),
            ));
            let deadline = Instant::now() + Duration::from_secs(25);
            let mut server_tick = Instant::now() - Duration::from_millis(50);
            loop {
                if server_tick.elapsed() >= Duration::from_millis(50) {
                    hs.tick();
                    server_tick = Instant::now();
                }
                j.hg.frames(1);
                let payloads = delayed.lock().unwrap();
                if matches!(j.hg.state.mode, crate::GameMode::Playing)
                    && payloads.replace.is_some()
                    && payloads.snapshot.is_some()
                {
                    break;
                }
                drop(payloads);
                assert!(
                    Instant::now() < deadline,
                    "joined loading/payload receipt never completed: mode={mode:?} snapshot_first={snapshot_first}"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            let (accept, replace, snapshot) = {
                let delayed = delayed.lock().unwrap();
                (
                    delayed.accept.clone().unwrap(),
                    delayed.first_replace.clone().unwrap(),
                    delayed.first_snapshot.clone().unwrap(),
                )
            };
            let i = accept.player_index as usize;
            assert_eq!(accept.inventory_mode, mode);
            assert!(accept.has_personal_state);
            assert_ne!(accept.action_session, [0; 16]);
            assert_eq!(snapshot.session, accept.action_session);
            assert_eq!(snapshot.life, accept.action_life);
            assert_eq!(
                snapshot.window_event,
                replace.window_event + 1,
                "personal snapshot lacks its genuinely numbered linked fence"
            );
            assert_eq!(snapshot.context.participant, key(SOURCE_PERSONA));
            assert_eq!(snapshot.context.transfer, transfer);
            assert_eq!(snapshot.context.lineage, proof.core.lineage);
            assert_eq!(snapshot.context.generation, proof.core.generation);
            assert!(hs.server.players[i].personal.is_some());
            assert!(!hs.server.players[i].personal_delivery.initialised);
            assert!(j.hg.state.handover_holds_play());
            assert!(!j.hg.state.remote_client.as_ref().unwrap().personal_ready());
            assert!(
                delayed
                    .lock()
                    .unwrap()
                    .sent
                    .iter()
                    .all(|p| !authorises(protocol::deserialize_header(p).unwrap().0)),
                "join loading sent authority before the initial state"
            );
            // A clear reachable target for the real blocked right-click. The
            // pad is fixture world setup; no player holdings are supplied here.
            let p = j.hg.state.players[0].player.pos;
            let cell = [
                p.x.floor() as i32,
                p.y.floor() as i32,
                p.z.floor() as i32 - 2,
            ];
            for world in [&mut j.hg.state.world, &mut hs.server.world] {
                for x in cell[0] - 3..=cell[0] + 3 {
                    for z in cell[2] - 3..=cell[2] + 3 {
                        world.set_block(x, cell[1] - 1, z, crate::block::STONE);
                        for y in cell[1]..=cell[1] + 4 {
                            world.set_block(x, y, z, crate::block::AIR);
                        }
                    }
                }
            }
            j.hg.state.players[0].hotbar_slot = 4;
            let direction =
                (glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32, cell[2] as f32 + 0.5)
                    - j.hg.state.players[0].player.eye_pos())
                .normalize();
            j.hg.state.players[0].camera.yaw = (-direction.x).atan2(-direction.z);
            j.hg.state.players[0].camera.pitch = direction.y.asin();
            let hit = crate::raycast::cast_ray(
                j.hg.state.players[0].player.eye_pos(),
                j.hg.state.players[0].camera.forward(),
                crate::REACH_DISTANCE,
                &j.hg.state.world,
                &j.hg.state.registry,
            )
            .expect("blocked placement has a real reachable floor");
            assert_eq!(hit.block_pos, [cell[0], cell[1] - 1, cell[2]]);
            assert_eq!(hit.face_normal, [0, 1, 0]);
            attempts_wait_without_prediction(&mut j.hg, &delayed, cell);
            {
                let mut delayed = delayed.lock().unwrap();
                delayed.snapshot_allowed = snapshot_first;
                delayed.replace_allowed = !snapshot_first;
            }
            j.hg.frames(1);
            assert!(
                !j.hg.state.remote_client.as_ref().unwrap().personal_ready(),
                "one half of the initial baseline released authority"
            );
            let applied =
                j.hg.state
                    .remote_client
                    .as_ref()
                    .unwrap()
                    .applied_window_boundary();
            if snapshot_first {
                assert!(
                    applied < replace.window_event,
                    "snapshot skipped its missing Replace"
                );
                assert!(
                    j.hg.state
                        .remote_client
                        .as_ref()
                        .unwrap()
                        .personal_inbox
                        .as_ref()
                        .unwrap()
                        .has_pending_fence(snapshot.window_event, snapshot.life)
                );
            } else {
                assert_eq!(applied, replace.window_event);
                let slot = j.hg.state.players[0].inventory.slot(4).unwrap();
                assert_eq!(
                    (slot.item.clone(), slot.count),
                    (crate::item::Item::Block(crate::block::GLASS), 9),
                    "Replace-first test lacks real authoritative inventory"
                );
            }
            attempts_wait_without_prediction(&mut j.hg, &delayed, cell);
            let sent_before_full = delayed.lock().unwrap().sent.len();
            {
                let mut delayed = delayed.lock().unwrap();
                delayed.replace_allowed = true;
                delayed.snapshot_allowed = true;
            }
            // No held key/button is replayed when the missing half arrives.
            j.hg.frames(1);
            assert!(j.hg.state.remote_client.as_ref().unwrap().personal_ready());
            assert_eq!(
                j.hg.state
                    .remote_client
                    .as_ref()
                    .unwrap()
                    .applied_window_boundary(),
                snapshot.window_event
            );
            assert_eq!(
                j.hg.state.players[0]
                    .kill_counter
                    .get(&crate::mob::MobType::Pig),
                Some(&7)
            );
            assert_eq!(j.hg.state.players[0].inventory.slot(4).unwrap().count, 9);
            assert!(j.hg.state.players[0].crafting_ui.cursor_item.is_none());
            assert!(j.hg.state.pending_block_changes.is_empty());
            assert_eq!(
                j.hg.state.world.get_block(cell[0], cell[1], cell[2]),
                crate::block::AIR
            );
            j.hg.state.network_send_input();
            let expected_ack = protocol::PersonalAck {
                session: snapshot.session,
                life: snapshot.life,
                revision: snapshot.revision,
                window_event: snapshot.window_event,
                snapshot: crate::handover::hash(&bincode::serialize(&snapshot).unwrap()),
            };
            {
                let delayed = delayed.lock().unwrap();
                let packets = &delayed.sent[sent_before_full..];
                let ack_index = packets.iter().position(|p| {
                    let (kind, bytes) = protocol::deserialize_header(p).unwrap();
                    kind == PacketType::PersonalSync && matches!(protocol::safe_deserialize::<protocol::PersonalSyncPacket>(bytes), Ok(protocol::PersonalSyncPacket::Applied(ack)) if ack==expected_ack)
                }).expect("actual client never sent the exact installed Applied");
                let first = packets
                    .iter()
                    .position(|p| authorises(protocol::deserialize_header(p).unwrap().0))
                    .expect("newly initialised client never sent authority");
                assert!(
                    ack_index < first,
                    "authority overtook its actual initial Applied"
                );
                for packet in packets
                    .iter()
                    .filter(|p| authorises(protocol::deserialize_header(p).unwrap().0))
                {
                    let (kind, bytes) = protocol::deserialize_header(packet).unwrap();
                    match kind {
                        PacketType::ClientInput => {
                            let input: protocol::InputPacket =
                                protocol::safe_deserialize(bytes).unwrap();
                            assert_eq!(input.events_applied, snapshot.window_event);
                            assert!(
                                !input.place_block && !input.drop_item && !input.toggle_inventory
                            );
                            assert!(
                                input.block_changes.is_empty(),
                                "a blocked click replayed as an authorised edit"
                            );
                        }
                        PacketType::Action => {
                            let action: crate::action_wire::ActionPacket =
                                protocol::safe_deserialize(bytes).unwrap();
                            assert_eq!(action.action.session, snapshot.session);
                            assert_eq!(action.action.life, accept.action_life);
                            assert_eq!(action.action.events_applied, snapshot.window_event);
                        }
                        PacketType::WindowOp => {
                            let op: protocol::WindowOpPacket =
                                protocol::safe_deserialize(bytes).unwrap();
                            assert_eq!(op.events_applied, snapshot.window_event);
                            assert!(
                                matches!(op.op, protocol::WireWindowOp::SetAutoRefill { .. }),
                                "held window click was replayed"
                            );
                        }
                        _ => panic!("blocked authorisation replayed after installation"),
                    }
                }
            }
            hs.tick();
            assert!(
                hs.server.players[i].is_in_world(),
                "honest client authority behind its Applied was released"
            );
            assert!(hs.server.players[i].personal_delivery.initialised);
            assert!(hs.server.players[i].personal_delivery.pending.is_none());
            // A fresh click now actually predicts, allocates in Strict, sends
            // behind the same frontier and reaches the server's own window.
            assert_eq!(
                j.hg.state.apply_window_click(
                    0,
                    &WindowClick::Slot {
                        slot: 4,
                        right: false
                    }
                ),
                ClickResult::Done
            );
            j.hg.state.network_send_input();
            for _ in 0..20 {
                hs.tick();
                j.hg.frames(1);
            }
            assert!(hs.server.players[i].is_in_world());
            assert!(hs.server.players[i].inventory.slot(4).is_none());
            assert_eq!(hs.server.players[i].cursor.as_ref().unwrap().count, 9);
            assert_eq!(
                j.hg.state.apply_window_click(
                    0,
                    &WindowClick::Slot {
                        slot: 4,
                        right: false
                    }
                ),
                ClickResult::Done
            );
            j.hg.state.network_send_input();
            for _ in 0..20 {
                hs.tick();
                j.hg.frames(1);
            }
            assert_eq!(hs.server.players[i].inventory.slot(4).unwrap().count, 9);
            assert!(hs.server.players[i].cursor.is_none());
            assert!(j.hg.state.leave_world(
                crate::world_exit::SaveChoice::Discard,
                crate::world_exit::ExitTo::Lobby
            ));
            hs.tick();
            assert!(
                !hs.server.players[i].is_in_world(),
                "fixture did not release its real signed participant"
            );
        }
    }
}
