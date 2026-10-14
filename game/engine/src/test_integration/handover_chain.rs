//! Original-invitation continuation through checked online admission, actual
//! transfer controllers and native loading. Relay delivery and the final game
//! channel are local injected transports; authority checks remain production.
use super::handover_live::BINDING;
use crate::rendezvous::relay_client::{FakeRelayHub, RendezvousRelay};
use crate::{
    handover::{
        durable::{MarkerKind, MarkerStore},
        live::{
            bearer::Bearer,
            continuation::{self, Resolved},
            recipient::RecipientIdentity,
            source::SourceStage,
        },
    },
    handover_game::{Fixture, Handover, RecipientEnv, SourceEnv, UiAction},
    protocol::{self, PacketType},
    remote_client::RemoteClient,
    test_game_harness::HeadlessGame,
    transport::ClientTransport,
};
use nostr::{Keys, SecretKey, Timestamp, ToBech32};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
struct Actor {
    persona: u8,
    runtime: u8,
}
impl Actor {
    fn keys(self) -> Keys {
        keys(self.runtime)
    }
    fn pubkey(self) -> [u8; 32] {
        keys(self.persona).public_key().to_bytes()
    }
    fn attestation(self) -> nostr::Event {
        pollster::block_on(crate::runtime_identity::mint_player_attestation(
            &keys(self.persona),
            &self.keys().public_key(),
            Timestamp::from(now()),
            1,
        ))
        .unwrap()
    }
    fn recipient(self) -> RecipientIdentity {
        RecipientIdentity {
            runtime: self.keys(),
            attestation: self.attestation(),
        }
    }
    fn identity(self) -> crate::runtime_identity::RuntimeIdentity {
        crate::runtime_identity::RuntimeIdentity::from_parts(self.keys(), Some(self.attestation()))
    }
    fn contact(self) -> crate::contacts::Contact {
        crate::contacts::Contact {
            pubkey: self.pubkey(),
            display_name: None,
            tier: crate::comms::Tier::Kith,
            is_child: false,
            runtime_pubkey: Some(self.keys().public_key().to_bytes()),
            added_via: crate::contacts::AddedVia::Invite,
            added_at: now(),
            last_joined: None,
        }
    }
}
fn keys(seed: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[seed; 32]).unwrap())
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn recipient_fixture(game: &mut HeadlessGame, actor: Actor, imports: PathBuf) {
    game.state.handover_fixture = Some(Fixture {
        signed_in: Some(actor.pubkey()),
        recipient: Some(Box::new(move || {
            Ok(RecipientEnv {
                identity: actor.recipient(),
                imports: imports.clone(),
            })
        })),
    });
}
fn tick(source: &mut HeadlessGame, peer: &mut HeadlessGame) {
    source.hosted_ticks(1);
    source.frames(1);
    peer.frames(1);
}
fn resolve(link: &str) -> Resolved {
    let invite = crate::invite::Invite::parse(link, now()).unwrap();
    let root = crate::handover::context::RootContext::new(
        invite.host_runtime,
        &invite.host_persona,
        invite.expires_at,
        &invite.relays,
        now(),
    )
    .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(continuation::lookup(
            root,
            Bearer::new(invite.bearer),
            None,
            now(),
            &AtomicBool::new(false),
        ))
        .unwrap()
}
fn prepare(intent: &crate::online_prep::OnlineIntent) -> Option<Resolved> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(crate::online_prep::resolve_intent(
            intent,
            now(),
            &AtomicBool::new(false),
        ))
        .unwrap()
}
fn host(
    game: &mut HeadlessGame,
    actor: Actor,
    peer: Actor,
    relays: Vec<String>,
    hub: &FakeRelayHub,
) {
    let folder = game.state.world_name.clone();
    // This is the same worker authority resolution, before socket preparation.
    let continuation = prepare(&crate::online_prep::OnlineIntent::Host {
        folder: folder.clone(),
    });
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let prep = crate::online_prep::OnlinePrep {
        punch_socket: socket.try_clone().unwrap(),
        socket,
        candidates: Vec::new(),
        mapping: None,
        continuation,
    };
    game.state
        .start_online_host_with(
            folder,
            prep,
            crate::online_prep::HostInputs {
                identity: actor.identity(),
                persona: actor.pubkey(),
                relays,
                book: vec![peer.contact()],
                blocked: Vec::new(),
            },
            |_| Box::new(hub.client()),
        )
        .unwrap();
}
fn admission(
    source: &mut HeadlessGame,
    peer: Actor,
    original: &crate::invite::Invite,
    hub: &FakeRelayHub,
    expected: &Resolved,
) -> crate::online_join::OnlineJoin {
    let intent = crate::online_prep::OnlineIntent::Join {
        host_runtime: original.host_runtime,
        bearer: Some(original.bearer),
        invite: Some(Box::new(original.clone())),
        display_name: original.world_name.clone(),
        relays: original.relays.clone(),
    };
    let prepared = prepare(&intent).unwrap();
    assert_eq!(prepared.admission().unwrap(), expected.admission().unwrap());
    assert_eq!(prepared.prefix, expected.prefix);
    // A valid peer already in Kith still cannot present a conflicting context.
    let mut wrong = prepared.admission().unwrap();
    wrong.committed[0] ^= 1;
    let offer = crate::rendezvous::payload::Offer {
        v: crate::rendezvous::payload::PAYLOAD_VERSION,
        session: crate::rendezvous::payload::new_session_id(),
        persona: keys(peer.persona).public_key().to_bech32().unwrap(),
        attestation: peer.attestation(),
        bearer: Some(hex::encode(original.bearer)),
        protocol: protocol::PROTOCOL_VERSION,
        candidates: Vec::new(),
        sent_at: now(),
        handover: Some(wrong),
    };
    let target = nostr::PublicKey::from_slice(&prepared.authority().unwrap().runtime()).unwrap();
    let event = pollster::block_on(crate::rendezvous::payload::seal_offer(
        &peer.keys(),
        &target,
        &offer,
    ))
    .unwrap();
    hub.client().publish(&event).unwrap();
    let dropped = source.state.online_host.as_mut().unwrap().poll(now(), 1, 5);
    assert!(dropped.iter().any(|e| matches!(e, crate::online_host::HostEvent::Dropped(why) if why.contains("authority differs"))));
    assert!(
        !dropped
            .iter()
            .any(|e| matches!(e, crate::online_host::HostEvent::Admitted { .. })),
        "contacts bypassed the original authority context"
    );
    let mut join = crate::online_join::OnlineJoin::start_resolved(
        peer.identity(),
        peer.pubkey(),
        Box::new(hub.client()),
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
        Vec::new(),
        original.host_runtime,
        Some(original.bearer),
        original.world_name.clone(),
        Some(prepared),
        now(),
    )
    .map_err(|(why, _)| why)
    .unwrap();
    // The actual host loop consumes the encrypted Offer, gates it before Kith,
    // answers with its current authority and updates the game allowlist.
    source.state.poll_online();
    assert!(
        matches!(
            join.poll(now(), Duration::ZERO),
            crate::online_join::JoinStep::Connecting { .. }
        ),
        "the current original-link Answer was not verified"
    );
    assert_eq!(join.host_runtime(), expected.authority().unwrap().runtime());
    assert_eq!(
        join.host_contact().unwrap().pubkey,
        expected.authority().unwrap().persona()
    );
    assert_eq!(
        join.continuation().unwrap().admission().unwrap(),
        expected.admission().unwrap()
    );
    join
}
fn source_stage(game: &HeadlessGame) -> Option<SourceStage> {
    match &game.state.handover {
        Some(Handover::Source { session, .. }) => Some(session.stage()),
        _ => None,
    }
}
fn open(game: &mut HeadlessGame, folder: &str) {
    game.state.pending_menu_action = Some((
        crate::menu::MenuAction::LoadWorld(folder.into(), crate::spawn_pref::SpawnPref::Default),
        web_time::Instant::now(),
    ));
    let deadline = Instant::now() + Duration::from_secs(180);
    while !matches!(game.state.mode, crate::GameMode::Playing) {
        game.frames(1);
        if let crate::GameMode::Menu(menu) = &game.state.mode {
            assert!(
                menu.notice.is_none(),
                "continued native load refused: {:?}",
                menu.notice
            );
        }
        assert!(Instant::now() < deadline, "continued world never opened");
    }
}
fn own(game: &HeadlessGame, actor: Actor, counts: &[(u8, u32)]) {
    let slot = &game.state.players[0];
    let (bag, pig) = counts
        .iter()
        .find_map(|&(p, n)| (p == actor.persona).then_some((n, n)))
        .unwrap();
    assert_eq!(
        slot.inventory
            .slot(if actor.persona == 0x82 { 4 } else { 5 })
            .unwrap()
            .count as u32,
        bag,
        "the current own bag was reset or donated"
    );
    assert_eq!(
        slot.kill_counter
            .get(&crate::mob::MobType::Pig)
            .copied()
            .unwrap_or(0),
        pig,
        "the current own progress was reset or donated"
    );
}
/// Each hop uses the common host-start action and original-link preparation,
/// encrypted online admission, explicit consent, frozen capture, retirement,
/// activation, lobby promotion and native loading. Only transport delivery is
/// injected; the production checks verify every issued epoch and original link.
#[test]
#[ignore = "needs two GPU adapters holding the 506-layer atlas"]
fn game_harness_handover_original_link_full_transfer_handback_and_later_epoch() {
    let _worlds = crate::save::WorldsRootGuard::new("original-full-chain");
    let relay = crate::handover::relay_fixture::Relay::start();
    let a = Actor {
        persona: 0x82,
        runtime: 0x81,
    };
    let b = Actor {
        persona: 0x71,
        runtime: 0x72,
    };
    let mut source = HeadlessGame::boot_into_world("original-chain-source");
    let mut peer = HeadlessGame::boot_into_world("original-chain-peer");
    let mut meta = crate::save::WorldMeta::new(&source.state.world_name);
    meta.genesis_block_found = true;
    meta.description = "Exact original authored history".into();
    meta.seed = source.state.biome_gen.seed;
    crate::save::save_world_meta(&source.state.world_name, &meta).unwrap();
    source.state.players[0].inventory.set_slot(
        4,
        Some(crate::item::ItemStack::new_block(crate::block::GLASS, 9)),
    );
    source.state.players[0]
        .kill_counter
        .insert(crate::mob::MobType::Pig, 9);
    source.state.handover_fixture = Some(Fixture {
        signed_in: Some(a.pubkey()),
        recipient: None,
    });
    let mut hub = FakeRelayHub::new();
    host(&mut source, a, b, vec![relay.url()], &hub);
    let link = source
        .state
        .online_host
        .as_ref()
        .unwrap()
        .invite_link(now())
        .unwrap();
    let original = crate::invite::Invite::parse(&link, now()).unwrap();
    let root = source
        .state
        .online_host
        .as_ref()
        .unwrap()
        .root_context(now())
        .unwrap();
    let bearer = source
        .state
        .online_host
        .as_ref()
        .unwrap()
        .live_bearer(now())
        .unwrap();
    let staging = crate::data_dir::data_root().join("handover");
    let imports = staging.join("imports");
    std::fs::create_dir_all(&imports).unwrap();
    let scratch = staging.join("chain-scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut counts = vec![(a.persona, 9u32), (b.persona, 11u32)];
    for (epoch, source_actor, peer_actor) in [(1, a, b), (2, b, a), (3, a, b)] {
        let resolved = resolve(&link);
        assert_eq!(resolved.authority().unwrap().epoch(), epoch - 1);
        assert_eq!(
            resolved.authority().unwrap().runtime(),
            source_actor.keys().public_key().to_bytes()
        );
        assert_eq!(resolved.root, root);
        if epoch > 1 {
            let installed = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(continuation::installed_world(
                    &crate::save::world_dir(&source.state.world_name),
                    now(),
                    &AtomicBool::new(false),
                ))
                .unwrap()
                .unwrap();
            assert_eq!(
                installed.admission().unwrap(),
                resolved.admission().unwrap()
            );
            hub = FakeRelayHub::new();
            host(
                &mut source,
                source_actor,
                peer_actor,
                root.relays().to_vec(),
                &hub,
            );
            // Repeated Host online uses the actual copy-link action branch.
            for _ in 0..2 {
                source
                    .state
                    .start_online_prep(crate::online_prep::OnlineIntent::Host {
                        folder: source.state.world_name.clone(),
                    });
                assert!(source.state.online_prep.is_none());
                let copy = crate::invite::Invite::parse(
                    source.state.pending_clipboard.as_ref().unwrap(),
                    now(),
                )
                .unwrap();
                assert_eq!(
                    (
                        copy.host_runtime,
                        copy.host_persona,
                        copy.bearer,
                        copy.expires_at,
                        copy.relays
                    ),
                    (
                        original.host_runtime,
                        original.host_persona.clone(),
                        original.bearer,
                        original.expires_at,
                        original.relays.clone()
                    )
                );
            }
        }
        source.state.handover_fixture = Some(Fixture {
            signed_in: Some(source_actor.pubkey()),
            recipient: None,
        });
        let folder = source.state.world_name.clone();
        let joined = admission(&mut source, peer_actor, &original, &hub, &resolved);
        let checked_host = joined.host_contact().unwrap().pubkey;
        let checked_runtime = joined.host_runtime();
        let checked_continuation = joined.continuation().cloned();
        // Local channel injection follows production Answer verification. The
        // same signed channel challenge and live JoinAccept still run below.
        crate::online_prep::retire(joined);
        let client = source
            .state
            .hosted_server
            .as_mut()
            .unwrap()
            .attach_test_remote_with_binding(Some(BINDING));
        let challenge = loop {
            let packet = client.try_recv_from_server().unwrap();
            if let Some((PacketType::Challenge, bytes)) = protocol::deserialize_header(&packet) {
                break protocol::safe_deserialize::<protocol::ChallengePacket>(bytes).unwrap();
            }
        };
        let mut request = crate::remote_client::build_join_request_guest("Original participant", 0);
        request.auth_event = Some(
            pollster::block_on(crate::signet::native_signer::build_join_auth_event(
                &keys(peer_actor.persona),
                &challenge.nonce_hex,
                &crate::signet::join_origin(Some(BINDING)),
            ))
            .unwrap(),
        );
        recipient_fixture(&mut peer, peer_actor, imports.clone());
        peer.state.remote_client = Some(RemoteClient::from_transport(
            Box::new(client),
            request,
            None,
        ));
        peer.state.set_joined_host(checked_host, checked_runtime);
        peer.state.joined_continuation = checked_continuation;
        // A lobby peer uses the same ordinary joined game flow as existing
        // controller fixtures, with one local body and rendering kept alive.
        if !matches!(peer.state.mode, crate::GameMode::Playing) {
            peer.state.mode = crate::GameMode::Playing;
        }
        let deadline = Instant::now() + Duration::from_secs(180);
        let seat = loop {
            tick(&mut source, &mut peer);
            let hs = source.state.hosted_server.as_ref().unwrap();
            if let Some(seat) = hs.handover_seat(hs.server.players.len() - 1) {
                let slot = hs.server.players.len() - 1;
                if peer.state.remote_client.as_ref().unwrap().is_connected()
                    && peer.state.remote_client.as_ref().unwrap().personal_ready()
                    && hs.server.players[slot].personal.as_ref().is_none_or(|p| {
                        hs.server.players[slot]
                            .personal_delivery
                            .quiescent(p, hs.server.players[slot].respawns)
                    })
                {
                    break (slot, seat.0, seat.1);
                }
            }
            assert!(
                Instant::now() < deadline,
                "original-link participant never settled its actual join/personal boundary"
            );
        };
        if epoch == 1 {
            // The initial participant owns a different confirmed window.
            let sp = &mut source.state.hosted_server.as_mut().unwrap().server.players[seat.0];
            sp.inventory.set_slot(
                5,
                Some(crate::item::ItemStack::new_block(crate::block::DIRT, 11)),
            );
            peer.state.players[0].inventory.set_slot(
                5,
                Some(crate::item::ItemStack::new_block(crate::block::DIRT, 11)),
            );
        } else {
            own(&peer, peer_actor, &counts);
        }
        source
            .state
            .begin_handover_with(
                seat,
                SourceEnv {
                    runtime: source_actor.keys(),
                    root: root.clone(),
                    scratch: scratch.clone(),
                    bearer: Bearer::new(bearer),
                    attestation: source_actor.attestation(),
                    continuation: (epoch > 1).then_some(resolved),
                },
                now(),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(180);
        while !matches!(&peer.state.handover,Some(Handover::Recipient(session)) if session.stage()==crate::handover::live::recipient::RecipientStage::AwaitingConsent)
        {
            tick(&mut source, &mut peer);
            assert!(
                Instant::now() < deadline,
                "actual continued proposal never reached consent"
            );
        }
        peer.state.handover_consent_shown = Some(web_time::Instant::now() - Duration::from_secs(1));
        peer.state.handover_click = Some(UiAction::Accept);
        peer.frames(1);
        let transfer = match &source.state.handover {
            Some(Handover::Source { session, .. }) => session.transfer(),
            _ => panic!("source action disappeared"),
        };
        let activated = format!("activated-{}.dat", hex::encode(transfer));
        let mut retained = None;
        while !matches!(
            source_stage(&source),
            Some(SourceStage::Retired)
        ) {
            tick(&mut source, &mut peer);
            assert!(
                Instant::now() < deadline,
                "actual continued transfer failed: {:?}",
                source_stage(&source)
            );
        }
        if let Some(Handover::Source { session, .. }) = &source.state.handover {
            retained = session.retained_folder().map(str::to_string);
        }
        let working = match &peer.state.handover {
            Some(Handover::Recipient(session)) => session.checkpoint().unwrap().working_world(),
            _ => panic!("recipient action disappeared"),
        };
        let store = MarkerStore::open_existing(working.join("handover")).unwrap();
        while store
            .read(MarkerKind::Activated, transfer)
            .unwrap()
            .is_none()
        {
            tick(&mut source, &mut peer);
            assert!(
                Instant::now() < deadline,
                "actual continued import never activated"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let core = crate::handover::live::personal_adapter::proof(&working)
            .unwrap()
            .unwrap()
            .core;
        assert_eq!((core.epoch, core.generation), (epoch, u64::from(epoch)));
        assert_eq!(core.context, root.digest());
        assert!(
            crate::handover::live::authority::open_guard(&crate::save::world_dir(&folder)).is_err(),
            "the old source folder reopened"
        );
        let retained = retained
            .as_ref()
            .expect("the source's distinct retained copy was not released");
        assert_ne!(retained, &folder);
        let retained_dir = crate::save::world_dir(retained);
        let retained_proof = crate::handover::live::personal_adapter::proof(&retained_dir)
            .unwrap()
            .unwrap();
        assert!(retained_proof.is_retained());
        assert_eq!(retained_proof.local_owner(), source_actor.pubkey());
        assert_eq!(retained_proof.core, core);
        assert!(
            crate::handover::phase::Installed::read(
                &MarkerStore::open_existing(retained_dir.join("handover")).unwrap(),
                transfer
            )
            .unwrap()
            .is_none(),
            "retained personal proof became original routing authority"
        );
        assert!(source.state.leave_world(
            crate::world_exit::SaveChoice::Discard,
            crate::world_exit::ExitTo::Lobby
        ));
        assert!(peer.state.leave_world(
            crate::world_exit::SaveChoice::Discard,
            crate::world_exit::ExitTo::Lobby
        ));
        // Only ordinary lobby frames promote/list the activated checkpoint.
        let imported = loop {
            peer.frames(1);
            let crate::GameMode::Menu(menu) = &peer.state.mode else {
                panic!("recipient left the lobby")
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
                "the actual lobby did not list the activated hop"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        recipient_fixture(&mut peer, peer_actor, imports.clone());
        open(&mut peer, &imported);
        let idx = if peer_actor.persona == a.persona {
            4
        } else {
            5
        };
        assert_eq!(
            peer.state.players[0].inventory.slot(idx).unwrap().count as u32,
            counts
                .iter()
                .find(|(p, _)| *p == peer_actor.persona)
                .unwrap()
                .1,
            "native continuation reset the bag"
        );
        if epoch == 1 {
            assert!(
                peer.state.players[0].kill_counter.is_empty(),
                "the first recipient inherited source progress"
            );
        } else {
            own(&peer, peer_actor, &counts);
        }
        let meta = crate::save::try_load_world_meta(&imported).unwrap();
        assert!(meta.genesis_block_found);
        assert_eq!(meta.description, "Exact original authored history");
        // Save new native state before the next actual transfer, so the v3
        // contains an old import binding that must be freshly rebound.
        let next = if peer_actor.persona == a.persona {
            17
        } else {
            13
        };
        counts
            .iter_mut()
            .find(|(p, _)| *p == peer_actor.persona)
            .unwrap()
            .1 = next;
        let block = if peer_actor.persona == a.persona {
            crate::block::GLASS
        } else {
            crate::block::DIRT
        };
        peer.state.players[0].inventory.set_slot(
            idx,
            Some(crate::item::ItemStack::new_block(block, next as u8)),
        );
        peer.state.players[0]
            .kill_counter
            .insert(crate::mob::MobType::Pig, next);
        peer.state.save_live_world().unwrap();
        assert!(peer.state.leave_world(
            crate::world_exit::SaveChoice::Save,
            crate::world_exit::ExitTo::Lobby
        ));
        open(&mut peer, &imported);
        own(&peer, peer_actor, &counts);
        let cold = resolve(&link);
        assert_eq!(
            (
                cold.authority().unwrap().epoch(),
                cold.authority().unwrap().runtime()
            ),
            (epoch, peer_actor.keys().public_key().to_bytes())
        );
        assert_eq!(cold.root, root);
        std::mem::swap(&mut source, &mut peer);
    }
    assert_eq!(resolve(&link).authority().unwrap().epoch(), 3);
}
