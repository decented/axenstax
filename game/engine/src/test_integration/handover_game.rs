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
    let generation = hg.state.hosted_server.as_ref().unwrap().handover_seat(slot).unwrap().0;
    let drawn = (slot, generation, key(RECIPIENT_PERSONA));
    assert_eq!(hg.state.handover_candidates(), vec![drawn], "the present signed joiner");
    let env = || SourceEnv { runtime: keys(SOURCE_RUNTIME), root: root(), scratch: dir.0.join("scratch") };
    assert!(hg.state.begin_handover_with((0, 0, key(SOURCE_PERSONA)), env(), now()).is_err(), "the host's own seat");
    // L6: a row drawn for an earlier attach of that seat starts nothing.
    assert_eq!(
        hg.state.begin_handover_with((slot, generation.wrapping_add(1), drawn.2), env(), now()),
        Err(crate::handover::live::copy::SEAT_CHANGED.to_string()),
        "a changed seat started a handover"
    );
    assert!(hg.state.handover.is_none());
    hg.state.begin_handover_with(drawn, env(), now()).expect("eligible");
    let rc = RemoteClient::from_transport(Box::new(client), crate::remote_client::build_join_request_guest("far", 0), None);
    (hg, Far { rc, dst: None }, slot)
}

/// The far end reads the Propose the ordinary way and consents; the host
/// game's own pump then freezes it.
fn frozen(tag: &str, dir: &Tmp) -> (HeadlessGame, Far) {
    frozen_after(tag, dir, |_| {})
}

/// [`frozen`], with `prep` run on the host game just before the far end
/// consents (dialogs it opens are open at the freeze).
fn frozen_after(tag: &str, dir: &Tmp, prep: impl FnOnce(&mut HeadlessGame)) -> (HeadlessGame, Far) {
    let (mut hg, mut far, _) = proposed(tag, dir);
    prep(&mut hg);
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
    // D6: the cancel is said in the lobby, where it is drawn.
    let crate::GameMode::Menu(menu) = &hg.state.mode else { panic!("not in the lobby") };
    assert_eq!(menu.notice.as_deref(), Some(copy::LEFT), "the lobby doesn't say leaving cancelled the handover");
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
    // L7: a click on the frame the window appears is not consent.
    j.hg.state.handover_click = Some(UiAction::Accept);
    j.hg.frames(1);
    assert_eq!(recipient_stage(&j.hg), Some(RecipientStage::AwaitingConsent), "an unarmed Accept consented");
    assert!(!j.hg.state.remote_client.as_ref().unwrap().is_parked());
    arm_consent(&mut j.hg);
    // L5: the joined session's router lease moves into the session.
    j.hg.state.upnp_lease = Some(crate::online_prep::LeaseKeeper::lost_for_test());
    j.hg.state.handover_click = Some(UiAction::Accept);
    j.hg.frames(1);
    assert!(j.hg.state.remote_client.as_ref().unwrap().is_parked(), "consent took the connection");
    assert!(j.hg.state.upnp_lease.is_none(), "the lease stayed with the joined session");
    assert!(matches!(&j.hg.state.handover, Some(Handover::Recipient(s)) if s.holds_lease()), "the session holds the lease");
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
    src.settle_pack(&mut j.hs, live(), now()).unwrap();
    src
}

/// The consent window has been up past `CONSENT_ARM` (L7).
fn arm_consent(hg: &mut HeadlessGame) {
    hg.state.handover_consent_shown = Some(web_time::Instant::now() - std::time::Duration::from_secs(1));
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
    assert!(
        matches!(&j.hg.state.handover, Some(Handover::Recipient(s)) if s.holds_lease()),
        "leaving the world released the lease the session holds"
    );
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
    let (sent, ticks) = (j.sent.load(Ordering::Relaxed), j.hg.state.tick_counter);
    for _ in 0..5 {
        j.hg.frames(1);
        j.hg.ticks(1);
        j.hg.state.network_send_input();
    }
    assert_eq!(j.hg.state.tick_counter, ticks, "a paused joined game ticked");
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

/// D5: a joined game whose join verified no host declines with a typed
/// reason, and the source shows that reason (no silent decline).
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_unverified_join_declines_with_a_reason_the_host_shows() {
    let mut j = joined("dst-unverified");
    j.hg.state.joined_host = None;
    let mut src = source(&mut j);
    let mut refused = None;
    for _ in 0..10 {
        step(&mut j);
        if let Err(r) = src.pump(&mut j.hs, live(), now()) {
            refused = Some(r);
            break;
        }
    }
    assert_eq!(
        refused,
        Some(crate::handover::live::eligibility::Refusal(copy::host_declined(
            crate::handover::live::wire::DeclineReason::UnverifiedHost
        ))),
        "the host wasn't told why the player declined"
    );
    assert!(j.hg.state.handover.is_none(), "an unverified join kept a handover");
}

/// D6: the consent window frees the cursor mid-play (it joins the cursor
/// predicate), and Decline gives it back.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_consent_window_frees_the_cursor_mid_play() {
    let mut j = joined("dst-cursor");
    assert!(!j.hg.state.cursor_must_be_free(), "the cursor was free before any proposal");
    let _src = source(&mut j);
    for _ in 0..10 {
        step(&mut j);
        if recipient_stage(&j.hg) == Some(RecipientStage::AwaitingConsent) {
            break;
        }
    }
    assert_eq!(recipient_stage(&j.hg), Some(RecipientStage::AwaitingConsent));
    assert!(j.hg.state.cursor_must_be_free(), "the consent window left the cursor grabbed");
    arm_consent(&mut j.hg);
    j.hg.state.handover_click = Some(UiAction::Decline);
    j.hg.frames(1);
    assert!(!j.hg.state.cursor_must_be_free(), "the cursor stayed free after Decline");
}

/// CP2 review M-A: the freeze closes the host's HUD dialogs before its
/// capture, and the ONE central gate holds any dialog still open: a chest
/// move and a bounty claim during the freeze leave the world, the seat's
/// items and its progression unchanged. A pre-Retired cancel lifts the gate
/// (the same injected chest click then lands: the injection is real).
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_freeze_closes_and_gates_hud_dialogs() {
    use crate::container_window::ContainerClick;
    use crate::handover_game::TestHudClick;
    let dir = Tmp::new("hud");
    let body = std::cell::Cell::new((0, 0, 0));
    let (mut hg, _far) = frozen_after("hud", &dir, |hg| {
        let p = hg.state.players[0].player.pos;
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        body.set((x, y, z));
        hg.state.world.set_block(x + 2, y, z, crate::block::CHEST);
        let mut chest = crate::chest::ChestData::new();
        chest.slots[2] = Some(crate::item::ItemStack::new_block(crate::block::STONE, 6));
        hg.state.world.insert_chest((x + 2, y, z), chest);
        hg.state.world.set_block(x, y, z + 2, crate::block::BOUNTY_BOARD);
        hg.state.world.bounties = vec![crate::bounty::ActiveBounty { id: 77, template_idx: 0, issued_tick: 0 }];
        let t = crate::bounty::ActiveBounty { id: 77, template_idx: 0, issued_tick: 0 }.template().unwrap();
        hg.state.players[0].kill_counter.insert(t.mob_kind, t.required_count);
        hg.state.players[0].open_chest = Some((x + 2, y, z));
    });
    let (x, y, z) = body.get();
    let (chest, board) = ((x + 2, y, z), (x, y, z + 2));
    assert_eq!(hg.state.players[0].open_chest, None, "the freeze left the chest dialog open");
    let chest_before = hg.state.world.chest_at(chest).cloned().unwrap().slots;
    let inv_before: Vec<_> = (0..36).map(|i| hg.state.players[0].inventory.slot(i).cloned()).collect();
    let (kills_before, claimed_before) =
        (hg.state.players[0].kill_counter.clone(), hg.state.players[0].bounties_claimed.clone());
    // A chest dialog open during the freeze anyway: its move doesn't land.
    hg.state.players[0].open_chest = Some(chest);
    hg.state.test_hud_click = Some(TestHudClick::Chest(ContainerClick::Withdraw { slot: 2, all: true }));
    hg.frames(2);
    assert_eq!(hg.state.world.chest_at(chest).unwrap().slots, chest_before, "a chest move landed while frozen");
    let inv_now: Vec<_> = (0..36).map(|i| hg.state.players[0].inventory.slot(i).cloned()).collect();
    assert_eq!(inv_now, inv_before, "the seat's items changed while frozen");
    // A bounty claim while frozen: no claim, no drained kills, no payout.
    hg.state.test_hud_click = None;
    hg.state.players[0].open_chest = None;
    hg.state.players[0].open_bounty_board = Some(board);
    hg.state.test_hud_click = Some(TestHudClick::Bounty(77));
    hg.frames(2);
    assert_eq!(hg.state.players[0].kill_counter, kills_before, "a bounty claim drained kills while frozen");
    assert_eq!(hg.state.players[0].bounties_claimed, claimed_before, "a bounty was claimed while frozen");
    // Pre-Retired cancel: the surfaces work again.
    hg.state.test_hud_click = None;
    hg.state.players[0].open_bounty_board = None;
    hg.state.handover_click = Some(UiAction::Cancel);
    hg.frames(1);
    assert!(!hg.state.handover_holds_play());
    hg.state.players[0].open_chest = Some(chest);
    hg.state.test_hud_click = Some(TestHudClick::Chest(ContainerClick::Withdraw { slot: 2, all: true }));
    hg.frames(1);
    assert_eq!(hg.state.world.chest_at(chest).unwrap().slots[2], None, "the released chest dialog didn't work");
}

/// M-A controls with a headless path: the pause-menu world-setting gate,
/// and a request on a parked connection (settled unsent, never written into
/// the closed stand-in). The disabled pause notice drawn in place of a HUD
/// dialog is egui-only (the harness has no pointer to read it back).
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_frozen_pause_settings_gate_and_parked_requests_settle_unsent() {
    let dir = Tmp::new("pausegate");
    let (mut hg, _far) = frozen("pausegate", &dir);
    assert_eq!(hg.state.handover_gate_pause_click(crate::menu::PAUSE_DIFFICULTY_CHANGED), -1, "a world setting landed while frozen");
    assert_eq!(hg.state.handover_gate_pause_click(crate::menu::PAUSE_SWITCH_CREATIVE), -1);
    drop(hg);
    let mut j = joined("dst-parked-request");
    let _src = consented(&mut j);
    use crate::item::{ItemStack, MaterialId};
    j.hg.state.players[0].inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 1)));
    j.hg.state.players[0].hotbar_slot = 0;
    j.hg.state.send_eat_request(0);
    assert!(!j.hg.state.joiner_actions.eat_in_flight(), "a request was written into a parked connection");
    assert_eq!(j.hg.state.joiner_actions.len(), 0);
}
