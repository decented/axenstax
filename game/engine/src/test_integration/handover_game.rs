//! World handover through the REAL GameState (headless GPU harness, so
//! `#[ignore]`d like every harness test; run with `--ignored`). The source
//! tests host the game's own world on a lent server and drive the far
//! recipient by hand; the recipient tests join the game to a non-GPU lent
//! server whose source session is driven by hand. One GPU game per test.
use super::handover_live::{BINDING, signed_join_lent};
use super::joiner_authority::{accepted, settle_chunk_push};
use super::lent_world::start_lent;
use crate::handover::{
    context::RootContext,
    durable::MarkerStore,
    live::{
        capture, copy,
        eligibility::Seat,
        recipient::{JoinedHost, RecipientConsent, RecipientIdentity, RecipientSession, RecipientStage},
        source::{Capture, HostAction, Live, SourceConfig, SourceSession, SourceStage},
        wire::{Frame, root_lineage},
    },
    personal::PersonalRecord,
    phase::Authority,
};
use crate::handover_game::{Fixture, Handover, RecipientEnv, SourceEnv, UiAction};
use crate::hosted_server::HostedServer;
use crate::protocol::{self, PacketType};
use crate::remote_client::RemoteClient;
use crate::sim_lend::OwnedSimParts;
use crate::test_game_harness::HeadlessGame;
use crate::transport::{ChannelClientTransport, ClientTransport};
use nostr::{Keys, SecretKey, Timestamp, ToBech32};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SOURCE_RUNTIME: u8 = 0x81;
const SOURCE_PERSONA: u8 = 0x82;
const RECIPIENT_PERSONA: u8 = 0x71;
const RECIPIENT_RUNTIME: u8 = 0x72;
const ROUNDS: usize = 20_000;

fn keys(b: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[b; 32]).unwrap())
}
fn key(b: u8) -> [u8; 32] {
    keys(b).public_key().to_bytes()
}
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}
fn npub(b: u8) -> String {
    keys(b).public_key().to_bech32().unwrap()
}
fn root() -> RootContext {
    RootContext::new(key(SOURCE_RUNTIME), &npub(SOURCE_PERSONA), now() + 3600, &["wss://relay.example".into()], now())
        .unwrap()
}
fn joined_host() -> JoinedHost {
    JoinedHost { persona: key(SOURCE_PERSONA), runtime: key(SOURCE_RUNTIME) }
}
fn identity() -> RecipientIdentity {
    let runtime = keys(RECIPIENT_RUNTIME);
    let attestation = pollster::block_on(crate::runtime_identity::mint_player_attestation(
        &keys(RECIPIENT_PERSONA),
        &runtime.public_key(),
        Timestamp::from(now()),
        1,
    ))
    .unwrap();
    RecipientIdentity { runtime, attestation }
}

/// The same pid-scoped worlds dir every harness test uses.
fn isolate_saves() {
    let tmp = std::env::temp_dir().join(format!("axenstax-harness-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    // SAFETY: every harness test sets the same value.
    unsafe { std::env::set_var("AXENSTAX_WORLDS_DIR", &tmp) };
}

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("handover-game-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        for d in ["imports", "markers", "scratch"] {
            std::fs::create_dir_all(p.join(d)).unwrap();
        }
        Self(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn recipient_env(dir: &Path) -> RecipientEnv {
    RecipientEnv { identity: identity(), imports: dir.join("imports"), markers: MarkerStore::new(dir.join("markers")).unwrap() }
}

// ── Source: the game hosts; the far recipient is driven by hand ─────────────

/// The far end: a joined client's RemoteClient and its session.
struct Far {
    rc: RemoteClient,
    dst: Option<RecipientSession>,
}

/// One main-loop tick of a hosting game, then one frame.
fn round(hg: &mut HeadlessGame) {
    hg.hosted_ticks(1);
    hg.frames(1);
}

/// The game hosting its own world on a lent server, one signed joiner on a
/// bound channel, and the host action taken for that joiner's seat.
fn proposed(tag: &str, dir: &Tmp) -> (HeadlessGame, Far, usize) {
    isolate_saves();
    let name = format!("harness-handover-{tag}");
    let mut hg = HeadlessGame::boot_into_world(&name);
    let seed = hg.state.biome_gen.seed;
    let hs = HostedServer::start_host(
        1,
        name,
        seed,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        crate::hosted_server::HostWorld::Lent,
    )
    .unwrap();
    hg.state.hosted_server = Some(hs);
    // The real seat 0 carries its own progress and inventory.
    let p0 = &mut hg.state.players[0];
    p0.kill_counter.insert(crate::mob::MobType::Pig, 7);
    p0.inventory.set_slot(4, Some(crate::item::ItemStack::new_block(crate::block::GLASS, 9)));
    let client = hg.state.hosted_server.as_mut().unwrap().attach_test_remote_with_binding(Some(BINDING));
    let chal = loop {
        let pkt = client.try_recv_from_server().expect("challenge issued on attach");
        if let Some((PacketType::Challenge, payload)) = protocol::deserialize_header(&pkt) {
            break protocol::safe_deserialize::<protocol::ChallengePacket>(payload).unwrap();
        }
    };
    let origin = crate::signet::join_origin(Some(BINDING));
    let ev = crate::hosted_server::tests::signed_auth_event([RECIPIENT_PERSONA; 32], &chal.nonce_hex, &origin, now() as u32);
    client.send_to_server(&protocol::serialize_packet(PacketType::JoinRequest, &crate::hosted_server::tests::join_with_auth(ev, None)));
    hg.hosted_ticks(1);
    let slot = accepted(&client).expect("signed join accepted").player_index as usize;
    settle_chunk_push(&mut hg, &client, |hg| {
        hg.hosted_ticks(1);
    });
    hg.state.handover_fixture = Some(Fixture { signed_in: Some(key(SOURCE_PERSONA)), recipient: None });
    assert_eq!(hg.state.handover_candidates(), vec![(slot, key(RECIPIENT_PERSONA))], "the present signed joiner");
    let env = SourceEnv { runtime: keys(SOURCE_RUNTIME), root: root(), scratch: dir.0.join("scratch") };
    assert!(hg.state.begin_handover_with(0, SourceEnv { runtime: keys(SOURCE_RUNTIME), root: root(), scratch: dir.0.join("scratch") }, now()).is_err(), "the host's own seat");
    hg.state.begin_handover_with(slot, env, now()).expect("eligible");
    let rc = RemoteClient::from_transport(Box::new(client), crate::remote_client::build_join_request_guest("far", 0), None);
    (hg, Far { rc, dst: None }, slot)
}

/// The far end reads the Propose the ordinary way and consents; the host
/// game's own pump then freezes it.
fn frozen(tag: &str, dir: &Tmp) -> (HeadlessGame, Far) {
    let (mut hg, mut far, _) = proposed(tag, dir);
    round(&mut hg);
    far.rc.poll();
    let propose = far.rc.take_handover_frames().into_iter().find(|b| matches!(Frame::decode(b), Ok(Frame::Propose { .. })));
    let mut dst = RecipientSession::propose(
        &propose.expect("the Propose reached the joined client"),
        joined_host(),
        identity(),
        dir.0.join("imports"),
        MarkerStore::new(dir.0.join("markers")).unwrap(),
        now(),
    )
    .unwrap();
    let p = dst.proposal().clone();
    let c = RecipientConsent { transfer: p.transfer, world_name: p.world_name, source_npub: p.source_npub };
    dst.consent(c, std::net::UdpSocket::bind("127.0.0.1:0").unwrap(), far.rc.park_transport(), now()).unwrap();
    far.dst = Some(dst);
    for _ in 0..8 {
        round(&mut hg);
        if hg.state.handover_freezes_world() {
            break;
        }
    }
    assert!(hg.state.handover_freezes_world(), "the host game froze on consent");
    (hg, far)
}

fn source_stage(hg: &HeadlessGame) -> Option<SourceStage> {
    match &hg.state.handover {
        Some(Handover::Source { session, .. }) => Some(session.stage()),
        _ => None,
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_frozen_host_runs_no_tick_captures_the_real_seat_and_cancel_resumes() {
    let dir = Tmp::new("tick");
    let (mut hg, mut far) = frozen("tick", &dir);
    let (time, tally, counter) = (hg.state.world_time, hg.state.world.sim_tally, hg.state.tick_counter);
    hg.state.autosave_counter = 5999;
    for _ in 0..ROUNDS {
        far.dst.as_mut().unwrap().pump(now()).unwrap();
        round(&mut hg);
        if source_stage(&hg) == Some(SourceStage::ReadyVerified) {
            break;
        }
    }
    assert_eq!(source_stage(&hg), Some(SourceStage::ReadyVerified));
    assert_eq!(far.dst.as_ref().unwrap().stage(), RecipientStage::Ready);
    assert_eq!(
        (hg.state.world_time, hg.state.world.sim_tally, hg.state.tick_counter),
        (time, tally, counter),
        "a frozen host game ticked"
    );
    assert!(!crate::save::has_autosave(&hg.state.world_name), "a frozen host autosaved");
    // The copy carries the REAL seat 0: its own record and its sidecar.
    let run_id = hg.state.hosted_server.as_ref().unwrap().server.run_id;
    let Some(Handover::Source { session, .. }) = &hg.state.handover else { panic!("no source session") };
    let context = session.record_context(key(SOURCE_PERSONA));
    let cp = far.dst.as_ref().unwrap().checkpoint().unwrap().received_world().to_path_buf();
    let rec = std::fs::read(cp.join(format!("participants/{}.dat", npub(SOURCE_PERSONA)))).unwrap();
    assert_eq!(PersonalRecord::decode(&rec, &context).unwrap().kills, vec![(crate::mob::MobType::Pig, 7)]);
    let sidecar = std::fs::read(cp.join(format!("joiners/{}.dat", npub(SOURCE_PERSONA)))).unwrap();
    assert_eq!(sidecar, capture::local0_sidecar(&hg.state.players[0], run_id).unwrap(), "not the real seat 0");
    // Cancel: the host game plays again (and the far end is told).
    hg.state.handover_click = Some(UiAction::Cancel);
    hg.frames(1);
    assert!(hg.state.handover.is_none() && !hg.state.handover_freezes_world());
    hg.hosted_ticks(3);
    assert_ne!(hg.state.world_time, time, "the released host game ticks again");
    assert!(far.dst.as_mut().unwrap().pump(now()).is_err(), "the far end heard the cancel");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_frozen_host_refuses_saves_and_leaving_cancels_first() {
    let dir = Tmp::new("save");
    let (mut hg, mut far) = frozen("save", &dir);
    let dat = crate::save::world_dir(&hg.state.world_name).join("world.dat");
    let before = std::fs::read(&dat).ok();
    assert_eq!(hg.state.save_live_world(), Err(copy::SAVE_PAUSED.to_string()), "a frozen world was saved");
    assert_eq!(std::fs::read(&dat).ok(), before, "the frozen world's save changed");
    // Leaving cancels the handover first, then saves the released world.
    assert!(hg.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    assert!(hg.state.handover.is_none());
    assert!(std::fs::read(&dat).is_ok(), "the world saved after the cancel");
    let mut heard = false;
    for _ in 0..4 {
        heard |= far.dst.as_mut().unwrap().pump(now()).is_err();
    }
    assert!(heard, "the far end heard the cancel");
}

// ── Recipient: the game is joined; the source is driven by hand ─────────────

/// A client transport that counts what the game sends.
struct Counting {
    inner: ChannelClientTransport,
    sent: Arc<AtomicUsize>,
}
impl ClientTransport for Counting {
    fn send_to_server(&self, data: &[u8]) {
        self.sent.fetch_add(1, Ordering::Relaxed);
        self.inner.send_to_server(data);
    }
    fn try_recv_from_server(&self) -> Option<crate::transport::Packet> {
        self.inner.try_recv_from_server()
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.inner.channel_binding()
    }
}

struct Joined {
    hg: HeadlessGame,
    hs: HostedServer,
    host: OwnedSimParts,
    slot: usize,
    sent: Arc<AtomicUsize>,
    _dir: Tmp,
}

/// One logical step: the lent server, then a frame and a tick of the game.
fn step(j: &mut Joined) {
    j.host.lend_tick(&mut j.hs);
    j.hg.frames(1);
    j.hg.ticks(1);
    j.hg.state.network_send_input();
}

/// The game joined, signed, to a non-GPU lent server whose host it verified.
fn joined(tag: &str) -> Joined {
    isolate_saves();
    let dir = Tmp::new(tag);
    let mut hg = HeadlessGame::boot_into_world(&format!("harness-handover-{tag}"));
    let (mut hs, host) = start_lent(&format!("handover-game-{tag}"));
    let sent = Arc::new(AtomicUsize::new(0));
    let inner = hs.attach_test_remote_with_binding(Some(BINDING));
    let pkt = inner.try_recv_from_server().expect("challenge issued on attach");
    let Some((PacketType::Challenge, bytes)) = protocol::deserialize_header(&pkt) else { panic!("no challenge") };
    let challenge: protocol::ChallengePacket = protocol::safe_deserialize(bytes).unwrap();
    let mut request = crate::remote_client::build_join_request_guest("Recipient", 0);
    request.auth_event = Some(
        pollster::block_on(crate::signet::native_signer::build_join_auth_event(
            &keys(RECIPIENT_PERSONA),
            &challenge.nonce_hex,
            &crate::signet::join_origin(Some(BINDING)),
        ))
        .unwrap(),
    );
    let transport = Counting { inner, sent: sent.clone() };
    hg.state.remote_client = Some(RemoteClient::from_transport(Box::new(transport), request, None));
    hg.state.set_joined_host(key(SOURCE_PERSONA), key(SOURCE_RUNTIME));
    let env_dir = dir.0.clone();
    hg.state.handover_fixture =
        Some(Fixture { signed_in: None, recipient: Some(Box::new(move || Ok(recipient_env(&env_dir)))) });
    let mut j = Joined { hg, hs, host, slot: usize::MAX, sent, _dir: dir };
    for _ in 0..400 {
        step(&mut j);
        let slot = j.hs.server.players.len().saturating_sub(1);
        if j.hg.state.remote_client.as_ref().is_some_and(|rc| rc.is_connected()) && j.hs.handover_seat(slot).is_some() {
            j.slot = slot;
            break;
        }
    }
    assert_ne!(j.slot, usize::MAX, "the game never joined, signed");
    for _ in 0..40 {
        step(&mut j);
    }
    j
}

/// The hand-driven source session for the joined game's seat.
fn source(j: &mut Joined) -> SourceSession {
    let (generation, k) = j.hs.handover_seat(j.slot).unwrap();
    let root = root();
    let lineage = root_lineage(&root);
    let cfg = SourceConfig {
        runtime: keys(SOURCE_RUNTIME),
        root: root.clone(),
        authority: Authority::root(&root, lineage).unwrap(),
        lineage,
        world: [6; 16],
        generation: 1,
        scratch: j._dir.0.join("scratch"),
        validity: 600,
        world_dir: None,
    };
    let action = HostAction { seat: Seat { slot: j.slot, generation, key: k }, source_key: key(SOURCE_PERSONA), world_name: "Far".into() };
    SourceSession::begin(&mut j.hs, action, cfg, live(), now()).expect("eligible")
}
fn live() -> Live {
    Live { signed_in: Some(key(SOURCE_PERSONA)), local_seats: 1 }
}

fn recipient_stage(hg: &HeadlessGame) -> Option<RecipientStage> {
    match &hg.state.handover {
        Some(Handover::Recipient(s)) => Some(s.stage()),
        _ => None,
    }
}

/// The game sees the proposal through its real receive path and consents
/// with a click; the source freezes and makes its frozen capture.
fn consented(j: &mut Joined) -> SourceSession {
    let mut src = source(j);
    for _ in 0..10 {
        step(j);
        if recipient_stage(&j.hg) == Some(RecipientStage::AwaitingConsent) {
            break;
        }
    }
    assert_eq!(recipient_stage(&j.hg), Some(RecipientStage::AwaitingConsent), "the joined game never saw the proposal");
    j.hg.state.handover_click = Some(UiAction::Accept);
    j.hg.frames(1);
    assert!(j.hg.state.remote_client.as_ref().unwrap().is_parked(), "consent took the connection");
    assert!(j.hg.state.handover_holds_play());
    for _ in 0..4 {
        j.host.lend_tick(&mut j.hs);
        src.pump(&mut j.hs, live(), now()).unwrap();
    }
    assert_eq!(src.stage(), SourceStage::Frozen);
    let seat0 = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(3.0, 70.0, 3.0), 1.0);
    let context = src.record_context(key(SOURCE_PERSONA));
    let sidecar = capture::local0_sidecar(&seat0, [9; 16]).unwrap();
    let progress = capture::local0_progress(&seat0, context).unwrap();
    let meta = crate::handover::checkpoint::transfer_metadata(&crate::save::WorldMeta::new("Far")).unwrap();
    let save = crate::save::build_world_save(&j.host.world, std::slice::from_ref(&seat0), 42, &[], &[]).unwrap();
    let cap = Capture {
        meta: &meta,
        save,
        world: &j.host.world,
        disk_chunks: None,
        exhibit_dir: None,
        source_sidecar: &sidecar,
        source_progress: &progress,
    };
    src.offer(&mut j.hs, live(), cap, now()).unwrap();
    src
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_recipient_consents_while_joined_and_keeps_its_session_after_leaving() {
    let mut j = joined("dst-leave");
    let mut src = consented(&mut j);
    // The player leaves the joined world mid-transfer: the RemoteClient and
    // the world go, the session (and its connection) carry on to Ready.
    assert!(j.hg.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
    assert!(j.hg.state.remote_client.is_none());
    for _ in 0..ROUNDS {
        j.hg.frames(1);
        if src.pump(&mut j.hs, live(), now()).is_err() || src.stage() == SourceStage::ReadyVerified {
            break;
        }
    }
    assert_eq!(src.stage(), SourceStage::ReadyVerified, "the session didn't survive leaving the world");
    assert_eq!(recipient_stage(&j.hg), Some(RecipientStage::Ready));
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_recipient_cancel_before_ready_resumes_the_joined_session() {
    let mut j = joined("dst-cancel");
    let mut src = consented(&mut j);
    let tally = j.host.world.sim_tally;
    j.hg.state.handover_click = Some(UiAction::Cancel);
    j.hg.frames(1);
    assert!(j.hg.state.handover.is_none());
    assert!(src.pump(&mut j.hs, live(), now()).is_err(), "the source hears the cancel");
    assert!(!j.hs.handover_frozen(), "the source resumes");
    for _ in 0..6 {
        step(&mut j);
    }
    assert!(j.host.world.sim_tally != tally, "the source simulates again");
    let rc = j.hg.state.remote_client.as_ref().expect("still in the joined world");
    assert!(!rc.is_parked() && rc.is_connected(), "the connection came back to the joined game");
    assert!(!j.hg.state.handover_holds_play(), "play resumed on the joined game");
    assert!(crate::world_exit::remote_session_end_reason(rc).is_none());
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_joiners_pause_on_the_hosts_notice_and_send_nothing() {
    let mut j = joined("pause");
    // Another joiner is the recipient; the source freezes for it.
    let (_other, slot, generation, k) = signed_join_lent(&mut j.hs, &mut j.host, [0x5a; 32]);
    let _lease = j.hs.handover_bind(slot, generation, k, [0x2b; 16]).unwrap();
    j.hs.handover_freeze().unwrap();
    for _ in 0..3 {
        j.hg.frames(1);
    }
    assert!(j.hg.state.host_paused && j.hg.state.handover_holds_play(), "the joined game didn't pause");
    let sent = j.sent.load(Ordering::Relaxed);
    for _ in 0..5 {
        j.hg.frames(1);
        j.hg.ticks(1);
        j.hg.state.network_send_input();
    }
    assert_eq!(j.sent.load(Ordering::Relaxed), sent, "a paused joiner kept sending");
    j.hs.handover_release();
    for _ in 0..3 {
        j.hg.frames(1);
    }
    assert!(!j.hg.state.host_paused && !j.hg.state.handover_holds_play(), "the joined game didn't resume");
    j.hg.state.network_send_input();
    assert!(j.sent.load(Ordering::Relaxed) > sent, "a resumed joiner sends again");
}

/// The frozen host answers no online call (no admission at all); once the
/// handover is cancelled the same call is answered.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_frozen_host_answers_no_online_call_until_released() {
    use crate::online_join::{JoinStep, OnlineJoin};
    use crate::rendezvous::payload::Candidate;
    use crate::runtime_identity::{RuntimeIdentity, mint_player_attestation};
    let dir = Tmp::new("admit");
    let (mut hg, _far) = frozen("admit", &dir);
    let hub = crate::rendezvous::relay_client::FakeRelayHub::new();
    let attested = |persona: &Keys, runtime: Keys| {
        let att = pollster::block_on(mint_player_attestation(persona, &runtime.public_key(), Timestamp::from(now() - 10), 90))
            .unwrap();
        RuntimeIdentity::from_parts(runtime, Some(att))
    };
    let lan = || vec![Candidate { kind: "lan".to_string(), addr: "127.0.0.1:9".to_string() }];
    let host = crate::online_host::OnlineHost::start(
        attested(&keys(SOURCE_PERSONA), keys(SOURCE_RUNTIME)),
        key(SOURCE_PERSONA),
        Box::new(hub.client()),
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
        lan(),
        "Loop".into(),
        vec!["wss://relay.example".into()],
        Vec::new(),
        now(),
    )
    .map_err(|(e, _)| e)
    .unwrap();
    let bearer = host.invite().bearer;
    hg.state.online_host = Some(host);
    let caller = keys(0x91);
    let mut join = OnlineJoin::start(
        attested(&caller, keys(0x92)),
        caller.public_key().to_bytes(),
        Box::new(hub.client()),
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
        lan(),
        key(SOURCE_RUNTIME),
        Some(bearer),
        "Rowan".into(),
        now(),
    )
    .map_err(|(e, _)| e)
    .unwrap();
    let wait = std::time::Duration::from_secs(1);
    for _ in 0..5 {
        round(&mut hg);
        assert_eq!(join.poll(now(), wait), JoinStep::Waiting, "a frozen host answered a call");
    }
    hg.state.handover_click = Some(UiAction::Cancel);
    hg.frames(1);
    assert!(!hg.state.handover_freezes_world());
    let mut answered = false;
    for _ in 0..5 {
        round(&mut hg);
        answered |= join.poll(now(), wait) != JoinStep::Waiting;
    }
    assert!(answered, "the released host never answered the call");
}
