//! World handover through the REAL GameState (headless GPU harness, so
//! `#[ignore]`d like every harness test; run with `--ignored`). The source
//! tests host the game's own world on a lent server and drive the far
//! recipient by hand; the recipient tests join the game to a non-GPU lent
//! server whose source session is driven by hand. One GPU game per test.
use super::handover_live::{BINDING, signed_join_lent};
use super::joiner_authority::{accepted, settle_chunk_push};
use crate::handover::{
    context::RootContext,
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
use nostr::{JsonUtil, Keys, SecretKey, Timestamp, ToBech32};
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
/// H5: the loopback fixture relay, never a real one.
fn relay() -> &'static crate::handover::relay_fixture::Relay {
    static RELAY: std::sync::OnceLock<crate::handover::relay_fixture::Relay> = std::sync::OnceLock::new();
    RELAY.get_or_init(crate::handover::relay_fixture::Relay::start)
}
fn root() -> RootContext {
    let url = relay().url();
    RootContext::new(key(SOURCE_RUNTIME), &npub(SOURCE_PERSONA), now() + 3600, &[url], now()).unwrap()
}
fn bearer() -> crate::handover::live::bearer::Bearer {
    crate::handover::live::bearer::Bearer::new([0x5A; 16])
}
fn source_attestation() -> nostr::Event {
    pollster::block_on(crate::runtime_identity::mint_player_attestation(
        &keys(SOURCE_PERSONA),
        &keys(SOURCE_RUNTIME).public_key(),
        nostr::Timestamp::from(now()),
        1,
    ))
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
fn recipient_env_at(imports: PathBuf) -> RecipientEnv {
    RecipientEnv { identity: identity(), imports }
}
/// CP20 (CP18 review LOW-5): the imports folder the lobby itself promotes
/// from (`handover_game::promote_activated_imports`), pid-scoped in tests
/// and shared by every harness test in the process.
fn lobby_imports() -> PathBuf {
    let imports = crate::data_dir::data_root().join("handover").join("imports");
    std::fs::create_dir_all(&imports).unwrap();
    imports
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
    // Establish the fresh fixture's native metadata before the transfer.
    // Production capture never invents missing saved history/settings.
    if !crate::save::world_dir(&name).join("world_meta.json").exists() {
        let mut meta = crate::save::WorldMeta::new(&name);
        meta.seed = seed;
        crate::save::save_world_meta(&name, &meta).unwrap();
    }
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
    let env = || SourceEnv {
        runtime: keys(SOURCE_RUNTIME),
        root: root(),
        scratch: dir.0.join("scratch"),
        bearer: bearer(),
        attestation: source_attestation(),
        continuation: None,
    };
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
    joined_into(tag, None)
}

/// [`joined`], its recipient importing into `imports` (default: the test's
/// own folder).
fn joined_into(tag: &str, imports: Option<PathBuf>) -> Joined {
    isolate_saves();
    let dir = Tmp::new(tag);
    let mut hg = HeadlessGame::boot_into_world(&format!("harness-handover-{tag}"));
    let (mut hs, host) = super::lent_world::start_prepared_lent(&format!("handover-game-{tag}"));
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
    let imports = imports.unwrap_or_else(|| dir.0.join("imports"));
    hg.state.handover_fixture =
        Some(Fixture { signed_in: None, recipient: Some(Box::new(move || Ok(recipient_env_at(imports.clone())))) });
    let mut j = Joined { hg, hs, host, slot: usize::MAX, sent, _dir: dir };
    for _ in 0..400 {
        step(&mut j);
        let slot = j.hs.server.players.len().saturating_sub(1);
        if j.hg.state.remote_client.as_ref().is_some_and(|rc| rc.is_connected() && rc.personal_ready())
            && j.hs.handover_seat(slot).is_some()
            && j.hs.server.players[slot].window_events.is_quiescent()
        {
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
        // CP12: a world folder, so the source can retire (its record).
        world_dir: Some(j._dir.0.join("world")),
        bearer: bearer(),
        attestation: source_attestation(),
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
    if j._dir.0.file_name().unwrap().to_string_lossy().contains("dst-opens-activated") {
        let sp = &mut j.hs.server.players[j.slot];
        sp.inventory.set_slot(5, Some(crate::item::ItemStack::new_block(crate::block::DIRT, 11)));
        let mut plan = crate::plan::PlanData::debug_3x3_stone();
        plan.name = "Recipient latent exposure".into();
        plan.develop_state = crate::plan::DevelopState::Latent { exposure_ticks: 12 };
        crate::plan_store::remember_checked(&plan, crate::body_residency::allocated_body_bytes(&plan).unwrap()).unwrap();
        sp.cursor = Some(crate::item::ItemStack { item: crate::item::Item::Plan(plan), count: 1 });
        sp.craft_grid[1][2] = Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Diamond, 2));
        sp.armour[3] = Some(crate::armour::ArmourItem::new(crate::armour::ArmourSlot::Boots, crate::armour::ArmourMaterial::Iron));
        sp.combat.health = 8.0; sp.combat.hunger = 9;
    }
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
    let mut seat0 = crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(3.0, 70.0, 3.0), 1.0);
    seat0.inventory.set_slot(4, Some(crate::item::ItemStack::new_block(crate::block::GLASS, 9)));
    seat0.kill_counter.insert(crate::mob::MobType::Pig, 7);
    let context = src.record_context(key(SOURCE_PERSONA));
    let sidecar = capture::local0_sidecar(&seat0, [9; 16]).unwrap();
    let progress = capture::local0_progress(&seat0, context).unwrap();
    let meta = crate::handover::checkpoint::transfer_metadata(&crate::save::WorldMeta::new("Far")).unwrap();
    let save = crate::save::build_world_save(&j.host.world, std::slice::from_ref(&seat0), 42, &[], &[], None).unwrap();
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
    // CP21: a wall-clock bound (on the slow ecryptfs test folder the
    // import outlasted ROUNDS frames).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while std::time::Instant::now() < deadline {
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
    j.hs.handover_freeze(None).unwrap();
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

/// Review LOW-5: the trial "Try again" and closing the crafting screen sit
/// outside the HUD's frozen branch; while frozen both are dropped through
/// the central gate (nothing teleports, nothing moves into the inventory),
/// and once play resumes the crafting screen closes as normal.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_frozen_try_again_and_crafting_close_change_nothing() {
    let dir = Tmp::new("tryagain");
    let (mut hg, _far) = frozen("tryagain", &dir);
    let pos = hg.state.players[0].player.pos;
    hg.state.trial_outcome = Some(crate::trials::TrialOutcome {
        trial_id: "race".into(),
        name: "Race".into(),
        headline: "done".into(),
        is_best: false,
    });
    hg.state.trial_try_again("race");
    assert!(hg.state.trial_outcome.is_some(), "a trial restarted while frozen");
    assert_eq!(hg.state.players[0].player.pos, pos, "the seat moved while frozen");
    {
        let p = &mut hg.state.players[0];
        p.crafting_ui.open_player_crafting(&p.inventory, &p.armour_slots);
        p.crafting_ui.grid[0][0] = Some(crate::item::ItemStack::new_block(crate::block::GLASS, 3));
    }
    let inv_before: Vec<_> = (0..36).map(|i| hg.state.players[0].inventory.slot(i).cloned()).collect();
    assert!(hg.state.close_topmost_ui_panel());
    assert!(hg.state.players[0].crafting_ui.open, "the crafting screen closed while frozen");
    let inv_now: Vec<_> = (0..36).map(|i| hg.state.players[0].inventory.slot(i).cloned()).collect();
    assert_eq!(inv_now, inv_before, "closing crafting moved items while frozen");
    hg.state.handover_click = Some(crate::handover_game::UiAction::Cancel);
    round(&mut hg);
    assert!(!hg.state.handover_holds_play(), "the cancel released play");
    assert!(hg.state.close_topmost_ui_panel());
    assert!(!hg.state.players[0].crafting_ui.open, "crafting didn't close once play resumed");
}

/// CP8 step 1: the rotation the game runs after a disclosed abort
/// (`GameState::rotate_handover_invite`, called by the source pump) changes
/// only the bearer: the old invite string stops admitting, the new one
/// admits, and the invitation's root context is unchanged.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_disclosed_abort_rotation_keeps_the_root_and_retires_the_old_link() {
    use crate::online_admission::Admission;
    use crate::runtime_identity::{RuntimeIdentity, mint_player_attestation};
    let mut hg = HeadlessGame::boot_into_world("harness-handover-rotate");
    let hub = crate::rendezvous::relay_client::FakeRelayHub::new();
    let runtime = keys(SOURCE_RUNTIME);
    let att = pollster::block_on(mint_player_attestation(&keys(SOURCE_PERSONA), &runtime.public_key(), Timestamp::from(now() - 10), 90))
        .unwrap();
    let host = crate::online_host::OnlineHost::start(
        RuntimeIdentity::from_parts(runtime, Some(att)),
        key(SOURCE_PERSONA),
        Box::new(hub.client()),
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
        Vec::new(),
        "Loop".into(),
        vec!["wss://relay.example".into()],
        Vec::new(),
        now(),
    )
    .map_err(|(e, _)| e)
    .unwrap();
    let (old_link, root) = (host.invite_link(now()).unwrap(), host.root_context(now()).unwrap().digest());
    hg.state.online_host = Some(host);
    hg.state.rotate_handover_invite();
    let host = hg.state.online_host.as_ref().unwrap();
    let stranger = [0x44; 32];
    assert_ne!(host.invite_link(now()).unwrap(), old_link, "the invitation wasn't rotated");
    assert!(matches!(host.admission_for_link(&stranger, &old_link, now()), Admission::Refuse(_)), "the old invite still admits");
    assert!(matches!(host.admission_for_link(&stranger, &host.invite_link(now()).unwrap(), now()), Admission::Accept(_)), "the new invite doesn't admit");
    assert_eq!(host.root_context(now()).unwrap().digest(), root, "rotation changed the root");
}

// ── CP9: a retired world leaves without a save; its Commit still publishes ──

/// Every file under `dir` except the authority records in `handover/`.
fn snapshot(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, d: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(d) else { return };
        for e in entries {
            let p = e.unwrap().path();
            if p == crate::handover::live::authority::dir(base) {
                continue;
            }
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(p.strip_prefix(base).unwrap().to_path_buf(), std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// CP7 review HIGH-1 rest + CP9 step 1: the game hosting a world drives the
/// handover to Retired, then the player picks Save & Quit and closes the
/// window. The leave writes nothing (no save, no autosave), the
/// HostedServer and the online host are gone, the recipient never hears an
/// Abort, the open guard still says retired, and the background resumer
/// (handed the publication at Retired) publishes the record's exact Commit
/// bytes on the loopback relay and lands the "Committed published" record.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_retired_world_leaves_without_saving_and_its_commit_still_publishes() {
    use crate::handover::live::resumer::Resumer;
    use crate::handover::phase::RetiredCommit;
    use crate::runtime_identity::{RuntimeIdentity, mint_player_attestation};
    let dir = Tmp::new("retired-leave");
    let (mut hg, mut far) = frozen("retired-leave", &dir);
    let world_dir = crate::save::world_dir(&hg.state.world_name);
    // The online host the leave must stop (fake rendezvous hub, never a relay).
    let hub = crate::rendezvous::relay_client::FakeRelayHub::new();
    let runtime = keys(SOURCE_RUNTIME);
    let att = pollster::block_on(mint_player_attestation(&keys(SOURCE_PERSONA), &runtime.public_key(), Timestamp::from(now() - 10), 90))
        .unwrap();
    let host = crate::online_host::OnlineHost::start(
        RuntimeIdentity::from_parts(runtime, Some(att)),
        key(SOURCE_PERSONA),
        Box::new(hub.client()),
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
        Vec::new(),
        "Loop".into(),
        vec!["wss://relay.example".into()],
        Vec::new(),
        now(),
    )
    .map_err(|(e, _)| e)
    .unwrap();
    hg.state.online_host = Some(host);
    let abort = RecipientStage::Quarantined("The host cancelled the handover.");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while source_stage(&hg) != Some(SourceStage::Retired) && std::time::Instant::now() < deadline {
        let _ = far.dst.as_mut().unwrap().pump(now());
        if let Some(Handover::Source { session, .. }) = hg.state.handover.as_mut()
            && session.stage() == SourceStage::Publishing
        {
            session.join_publish_worker();
        }
        round(&mut hg);
    }
    assert_eq!(source_stage(&hg), Some(SourceStage::Retired), "the host game never retired");
    let Some(Handover::Source { session, .. }) = &hg.state.handover else { panic!("no source session") };
    let (t, commit) = (session.transfer(), session.commit_event().expect("no Commit").as_json());
    let handed = session.publication_handed();
    // CP8 review LOW-3: a Cancel that slips through past retirement cancels
    // nothing: the session (and the freeze) stay.
    hg.state.handover_click = Some(UiAction::Cancel);
    hg.frames(1);
    assert_eq!(source_stage(&hg), Some(SourceStage::Retired), "a Cancel after Retired ended the source session");
    assert!(hg.state.handover_freezes_world(), "a Cancel after Retired unfroze the world");
    let before = snapshot(&world_dir);
    // Save & Quit, then the window's close button.
    assert!(hg.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby), "the leave failed");
    assert!(hg.state.handover.is_none(), "the session outlived the leave");
    assert!(hg.state.hosted_server.is_none(), "the HostedServer outlived the leave");
    assert!(hg.state.online_host.is_none(), "the online host wasn't stopped");
    assert!(hg.state.close_window(), "the window didn't close");
    for _ in 0..6 {
        hg.frames(1);
        let _ = far.dst.as_mut().unwrap().pump(now());
        assert_ne!(far.dst.as_ref().unwrap().stage(), abort, "the recipient got an Abort after Retired");
    }
    assert_eq!(snapshot(&world_dir), before, "the retired world's files changed (a save was written)");
    assert!(!crate::save::has_autosave(&hg.state.world_name), "a retired world autosaved");
    assert_eq!(
        crate::handover::live::authority::open_guard(&world_dir).unwrap_err().0,
        copy::RETIRED,
        "the retired world opens again"
    );
    let store = crate::handover::live::authority::store(&world_dir).unwrap();
    let retired = RetiredCommit::read(&store, t).unwrap().expect("no retirement record");
    assert_eq!(retired.exact_bytes(), commit.as_bytes());
    assert!(Resumer::global().holds(t) || retired.is_published(&store), "nothing holds the publication");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !retired.is_published(&store) {
        assert!(std::time::Instant::now() < deadline, "the Committed published record never landed");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        relay().stored().iter().any(|e| e.as_json().as_bytes() == retired.exact_bytes()),
        "the relay doesn't hold the record's exact Commit bytes"
    );
    assert_eq!(crate::handover::live::authority::open_guard(&world_dir).unwrap_err().0, copy::RETIRED, "publication reopened it");
    assert!(handed, "the publication wasn't handed to the resumer at Retired");
}

// ── CP12 (CP10 review MEDIUM-1): the one exit function ──

/// The lobby's Quit ends the process only through the game's one exit
/// function (`exit_app`, where the handover grace runs), never a bare
/// `process::exit`.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_lobby_quit_goes_through_the_exit_function() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-handover-quit");
    assert!(hg.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby));
    assert!(matches!(hg.state.mode, crate::GameMode::Menu(_)), "the leave didn't reach the lobby");
    hg.frames(1);
    assert_eq!(hg.state.exit_calls, 0, "the lobby ran the exit function unasked");
    hg.state.test_menu_action = Some(crate::menu::MenuAction::Quit);
    // The lobby draws at most every 50 ms (its frame throttle).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while hg.state.test_menu_action.is_some() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(60));
        hg.frames(1);
    }
    assert!(hg.state.test_menu_action.is_none(), "the lobby never drew a frame");
    assert_eq!(hg.state.exit_calls, 1, "the lobby Quit didn't go through the exit function");
    assert!(hg.state.exit_now, "the lobby Quit didn't end the process");
}

// ── CP12 (CP10 review LOW-2): leaving while Prepared ends the session ──

/// A router lease that records its release.
struct FlagLease(Arc<std::sync::atomic::AtomicBool>);
impl crate::handover::live::recipient::HeldLease for FlagLease {
    fn renew(&mut self) {}
    fn give_back(self: Box<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
    fn give_back_within(self: Box<Self>, _grace: std::time::Duration) {
        self.0.store(true, Ordering::SeqCst);
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

/// The joined game consents; its session is then pumped BY HAND to
/// Prepared (the ack has left), so the pump's own hand-off never runs, as
/// when it failed and is retrying. The source reads the ack, retires and
/// publishes Committed. The player leaves the joined world: the session
/// ends, its lease goes back and its link closes, and the wait is the
/// resumer's: the import activates on the Committed.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_recipient_leaving_while_prepared_ends_the_session_and_still_activates() {
    let (_j, _src, working, t) = leave_while_prepared("dst-prepared-leave", None);
    let store = crate::handover::live::authority::store(&working).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while store.read(crate::handover::durable::MarkerKind::Activated, t).unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline, "the recipient never activated after leaving while Prepared");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// The body of the test above, up to the leave: the recipient leaves at
/// Prepared with Committed already published, and its import's wait is the
/// resumer's. Returns the joined game (now in the lobby), the source
/// session, the import's working world and the transfer.
fn leave_while_prepared(tag: &str, imports: Option<PathBuf>) -> (Joined, SourceSession, PathBuf, crate::handover::Id) {
    use crate::handover::live::resumer::{Backoff, Resumer};
    let mut j = joined_into(tag, imports);
    let mut src = consented(&mut j);
    let t = src.transfer();
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let Some(Handover::Recipient(s)) = j.hg.state.handover.as_mut() else { panic!("no recipient session") };
    drop(s.take_lease());
    s.hold_lease(Box::new(FlagLease(released.clone())));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !(recipient_stage(&j.hg) == Some(RecipientStage::Prepared) && src.past_retirement()) && std::time::Instant::now() < deadline {
        j.host.lend_tick(&mut j.hs);
        if let Some(Handover::Recipient(s)) = j.hg.state.handover.as_mut() {
            let _ = s.pump(now());
        }
        if src.stage() == SourceStage::Publishing {
            src.join_publish_worker();
        }
        if src.pump(&mut j.hs, live(), now()).is_err() {
            break;
        }
    }
    assert_eq!(recipient_stage(&j.hg), Some(RecipientStage::Prepared), "the recipient never reached Prepared (source {:?})", src.stage());
    assert!(src.past_retirement(), "the source never retired (stage {:?})", src.stage());
    let Some(Handover::Recipient(s)) = &j.hg.state.handover else { unreachable!() };
    assert!(!s.activation_handed() && !Resumer::global().holds_import(t), "the wait was handed before the leave");
    let working = s.checkpoint().unwrap().working_world();
    // The source's Committed is on the loopback relay before the leave.
    let fast = Resumer::new(Backoff { first: std::time::Duration::from_millis(100), ceiling: std::time::Duration::from_millis(400) });
    assert!(src.hand_publication(&fast), "the publication wasn't taken");
    let src_store = crate::handover::live::authority::store(&j._dir.0.join("world")).unwrap();
    let retired = crate::handover::phase::RetiredCommit::read(&src_store, t).unwrap().expect("no retirement record");
    while !retired.is_published(&src_store) {
        assert!(std::time::Instant::now() < deadline, "Committed was never published");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(j.hg.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby), "the leave failed");
    assert!(j.hg.state.handover.is_none(), "the Prepared session outlived leaving the world");
    assert!(released.load(Ordering::SeqCst), "leaving didn't give the session's router lease back");
    assert!(j.hg.state.upnp_lease.is_none(), "the lease came back to a joined session that has gone");
    let store = crate::handover::live::authority::store(&working).unwrap();
    let activated = || store.read(crate::handover::durable::MarkerKind::Activated, t).unwrap().is_some();
    assert!(Resumer::global().holds_import(t) || activated(), "nothing holds the wait for Committed after the leave");
    (j, src, working, t)
}

/// CP18: once Activated, the handed-over world moves into the world list;
/// Play opens it through the lobby's own load path (the open guard passes
/// the Activated record it carries), it saves, it hosts, and a joiner
/// joins it.
///
/// CP20 (CP18 review MEDIUM-1, LOW-5): the recipient is already in the
/// lobby (it left at Prepared, its import under the lobby's own imports
/// folder) when the import activates in the background. Only lobby frames
/// run: no promotion or refresh by hand. The "Waiting for…" line goes and
/// the card appears without leaving the lobby.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_recipient_opens_and_hosts_the_activated_world_from_the_lobby() {
    let (mut j, _src, working, t) = leave_while_prepared("dst-opens-activated", Some(lobby_imports()));
    let activated_record = format!("activated-{}.dat", hex::encode(t));
    let ours = |ms: &crate::menu::MenuState| {
        ms.worlds
            .iter()
            .find(|e| crate::handover::live::authority::dir(&crate::save::worlds_root().join(&e.folder_name)).join(&activated_record).exists())
            .map(|e| e.folder_name.clone())
    };
    {
        let crate::GameMode::Menu(ms) = &j.hg.state.mode else { panic!("the recipient isn't in the lobby") };
        assert!(ours(ms).is_none(), "the world was listed before it activated");
        assert!(ms.awaiting_imports.iter().any(|l| l.starts_with("Waiting for")), "no waiting line in the lobby: {:?}", ms.awaiting_imports);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let folder = loop {
        j.hg.frames(1);
        let crate::GameMode::Menu(ms) = &j.hg.state.mode else { panic!("the recipient left the lobby") };
        if let Some(f) = ours(ms)
            && !ms.awaiting_imports.iter().any(|l| l.starts_with("Waiting for"))
        {
            break f;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the lobby never listed the handed-over world (waiting lines {:?})",
            ms.awaiting_imports
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(!working.exists(), "the import's working world is still there");
    let crate::GameMode::Menu(ms) = &j.hg.state.mode else { unreachable!() };
    assert!(ms.worlds.iter().any(|e| e.folder_name == folder), "the lobby doesn't list the handed-over world");
    assert_eq!(crate::save::world_open_refusal(&folder), None, "the handed-over world is refused");
    // A signed remote attachment does not silently select the native local
    // persona after leaving it. The ordinary loader must refuse signed-out
    // play without changing any imported bytes.
    assert_eq!(j.hg.state.handover_signed_in(), None);
    let before_unsigned = snapshot(&crate::save::world_dir(&folder));
    j.hg.state.pending_menu_action =
        Some((crate::menu::MenuAction::LoadWorld(folder.clone(), crate::spawn_pref::SpawnPref::Default), web_time::Instant::now()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        j.hg.frames(1);
        assert!(!matches!(j.hg.state.mode, crate::GameMode::Playing), "signed-out native play received an imported player's bag");
        if let crate::GameMode::Menu(ms) = &mut j.hg.state.mode
            && let Some(notice) = &ms.notice
        {
            assert!(notice.contains("Sign in as this world's saved host"), "unexpected unsigned refusal: {notice}");
            ms.notice = None;
            break;
        }
        assert!(std::time::Instant::now() < deadline, "signed-out native open did not finish with a refusal");
    }
    assert_eq!(snapshot(&crate::save::world_dir(&folder)), before_unsigned, "unsigned native refusal changed imported bytes");
    // Select the same actual persona that signed this transfer's recipient
    // attachment; keep its recipient backend and all own-state assertions.
    let fixture = j.hg.state.handover_fixture.as_mut().expect("recipient identity fixture disappeared");
    fixture.signed_in = Some(key(RECIPIENT_PERSONA));
    assert_eq!(j.hs.server.players[j.slot].verified_pubkey, fixture.signed_in);
    // Play, from the lobby.
    j.hg.state.pending_menu_action =
        Some((crate::menu::MenuAction::LoadWorld(folder.clone(), crate::spawn_pref::SpawnPref::Default), web_time::Instant::now()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !matches!(j.hg.state.mode, crate::GameMode::Playing) {
        j.hg.frames(1);
        if let crate::GameMode::Menu(ms) = &j.hg.state.mode {
            assert!(ms.notice.is_none(), "the lobby refused the world: {:?}", ms.notice);
        }
        assert!(std::time::Instant::now() < deadline, "the handed-over world never opened");
    }
    assert_eq!(j.hg.state.world_name, folder);
    let own = &j.hg.state.players[0];
    assert_eq!(own.inventory.slot(5).unwrap().count, 11, "the promoted world lost its recipient's own bag");
    assert!(own.inventory.slot(4).is_none(), "the source owner's bag was donated to the recipient");
    assert_eq!(own.crafting_ui.grid[1][2].as_ref().unwrap().count, 2, "the grid was lost");
    assert!(own.armour_slots[3].is_some(), "the recipient's armour was lost");
    let crate::item::Item::Plan(plan) = &own.crafting_ui.cursor_item.as_ref().unwrap().item else { panic!("recipient cursor Plan lost") };
    assert_eq!(plan.develop_state, crate::plan::DevelopState::Latent { exposure_ticks: 12 }, "resolving a marker changed its private body's identity");
    assert!(plan.marker.is_none(), "the locally retained body was not resolved");
    assert_eq!((own.combat.health, own.combat.hunger), (8.0,9));
    assert!(own.kill_counter.is_empty(), "the source owner's progression was donated");
    let chest = (3, 69, 3);
    j.hg.state.world.set_block(chest.0,chest.1,chest.2,crate::block::CHEST);
    j.hg.state.world.insert_chest(chest, crate::chest::ChestData::default());
    j.hg.state.save_live_world().unwrap();
    // Deposit, autosave, explicit discard: both bag and chest select primary.
    let stack=j.hg.state.players[0].inventory.slot(5).cloned().unwrap();
    j.hg.state.players[0].inventory.set_slot(5,None);
    j.hg.state.world.chest_at_mut(chest).unwrap().slots[0]=Some(stack);
    j.hg.state.players[0].kill_counter.insert(crate::mob::MobType::Pig,23);
    j.hg.state.autosave_counter=5999; j.hg.ticks(1);
    assert!(crate::save::has_autosave(&folder));
    assert!(j.hg.state.leave_world(crate::world_exit::SaveChoice::Discard,crate::world_exit::ExitTo::Lobby));
    open_from_lobby(&mut j.hg,&folder);
    assert_eq!(j.hg.state.players[0].inventory.slot(5).unwrap().count,11,"discard kept an autosaved personal bag");
    assert!(j.hg.state.world.chest_at(chest).unwrap().slots[0].is_none(),"discard kept the autosaved chest");
    assert!(j.hg.state.players[0].kill_counter.is_empty(),"discard kept autosaved progress");
    // The same deposit with Save & Quit selects the complete new pair.
    let stack=j.hg.state.players[0].inventory.slot(5).cloned().unwrap();
    j.hg.state.players[0].inventory.set_slot(5,None);
    j.hg.state.world.chest_at_mut(chest).unwrap().slots[0]=Some(stack);
    j.hg.state.players[0].kill_counter.insert(crate::mob::MobType::Pig,23);
    assert!(j.hg.state.leave_world(crate::world_exit::SaveChoice::Save,crate::world_exit::ExitTo::Lobby));
    open_from_lobby(&mut j.hg,&folder);
    assert!(j.hg.state.players[0].inventory.slot(5).is_none(),"save replayed the frozen bag");
    assert_eq!(j.hg.state.world.chest_at(chest).unwrap().slots[0].as_ref().unwrap().count,11);
    assert_eq!(j.hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],23,"save/reopen lost current progression");
    // Withdraw, autosave, discard selects the saved deposit without duplication.
    let stack=j.hg.state.world.chest_at_mut(chest).unwrap().slots[0].take().unwrap();
    j.hg.state.players[0].inventory.set_slot(5,Some(stack));
    j.hg.state.players[0].kill_counter.insert(crate::mob::MobType::Pig,24);
    j.hg.state.autosave_counter=5999; j.hg.ticks(1);
    assert!(j.hg.state.leave_world(crate::world_exit::SaveChoice::Discard,crate::world_exit::ExitTo::Lobby));
    open_from_lobby(&mut j.hg,&folder);
    assert!(j.hg.state.players[0].inventory.slot(5).is_none(),"discard duplicated the withdrawn bag");
    assert_eq!(j.hg.state.world.chest_at(chest).unwrap().slots[0].as_ref().unwrap().count,11);
    assert_eq!(j.hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],23);
    // CP20 (CP18 review LOW-5): a real save of the moved world.
    assert_eq!(j.hg.state.save_live_world(), Ok(()), "the handed-over world didn't save");
    // A known imported local holder can predate the appended world scope.
    // Exercise that preparation boundary on the genuinely activated persona,
    // preserving its proof and selected body through a failed save and retry.
    let imported = j.hg.state.players[0].personal_binding.clone().unwrap();
    assert!(matches!(imported, crate::progression::GameplayBinding::Imported(_)));
    j.hg.state.world.quest_scope = None;
    j.hg.state.save_live_world().unwrap();
    let imported_before = snapshot(&crate::save::world_dir(&folder));
    crate::save_fence::raise(&crate::save::world_dir(&folder));
    let refused = j.hg.state.prepare_local_gameplay();
    crate::save_fence::lift(&crate::save::world_dir(&folder));
    assert!(refused.is_err());
    assert!(j.hg.state.world.quest_scope.is_none());
    assert_eq!(j.hg.state.players[0].personal_binding.as_ref(), Some(&imported));
    assert_eq!(j.hg.state.local_personal.as_ref(), Some(&imported));
    assert_eq!(snapshot(&crate::save::world_dir(&folder)), imported_before, "failed imported scope preparation changed selected authority or proofs");
    j.hg.state.prepare_local_gameplay().unwrap();
    assert!(j.hg.state.world.quest_scope.is_some());
    assert_eq!(j.hg.state.players[0].personal_binding.as_ref(), Some(&imported));
    assert_eq!(j.hg.state.local_personal.as_ref(), Some(&imported));
    let prepared = crate::save::read_world_save(&std::fs::read(crate::save::world_dir(&folder).join("world.dat")).unwrap()).unwrap();
    let local = crate::joiner_save::JoinerSave::decode(&prepared.local_personal.unwrap().0).unwrap();
    assert_eq!(local.personal.unwrap().binding, imported);
    assert_eq!(prepared.quest_world.unwrap().scope, j.hg.state.world.quest_scope.unwrap());
    // Host it (a lent host on the in-process transport), and a joiner joins.
    let hs = HostedServer::start_host(
        1,
        folder.clone(),
        j.hg.state.biome_gen.seed,
        1,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        crate::hosted_server::HostWorld::Lent,
    )
    .expect("the handed-over world hosts");
    j.hg.state.hosted_server = Some(hs);
    let client = j.hg.state.hosted_server.as_mut().unwrap().attach_test_remote();
    client.send_to_server(&protocol::serialize_packet(
        PacketType::JoinRequest,
        &crate::remote_client::build_join_request_guest("Visitor", 0),
    ));
    j.hg.hosted_ticks(2);
    assert!(accepted(&client).is_some(), "the joiner wasn't accepted into the handed-over world");
}


/// CP13 (CP12 review MEDIUM-1): the recipient leaves at Prepared BEFORE the
/// source has pumped past its ack (a slow or minimised source). The leave
/// closes the link; the source, pumped only after it, still reads the
/// queued, verified ack and retires; Committed is published and the
/// recipient's import activates.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_recipient_leaving_before_the_source_reads_the_ack_still_activates() {
    use crate::handover::live::resumer::{Backoff, Resumer};
    let mut j = joined("dst-leave-before-ack");
    let mut src = consented(&mut j);
    let t = src.transfer();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        assert!(std::time::Instant::now() < deadline, "the recipient never reached Prepared (source {:?})", src.stage());
        j.host.lend_tick(&mut j.hs);
        if let Some(Handover::Recipient(s)) = j.hg.state.handover.as_mut() {
            let _ = s.pump(now());
        }
        // Stop the moment the recipient has acked: the source hasn't read it.
        if recipient_stage(&j.hg) == Some(RecipientStage::Prepared) {
            break;
        }
        if src.stage() == SourceStage::Publishing {
            src.join_publish_worker();
        }
        src.pump(&mut j.hs, live(), now()).expect("the source refused before Prepared");
    }
    assert!(!src.past_retirement(), "the source read the ack before the leave");
    let Some(Handover::Recipient(s)) = &j.hg.state.handover else { unreachable!() };
    let working = s.checkpoint().unwrap().working_world();
    assert!(j.hg.state.leave_world(crate::world_exit::SaveChoice::Discard, crate::world_exit::ExitTo::Lobby), "the leave failed");
    assert!(j.hg.state.handover.is_none(), "the Prepared session outlived leaving the world");
    // The link closes as QUIC's does: closed at once, the ack still queued.
    j.hs.test_close_seat_transport(j.slot);
    // Only now does the source pump: the link has closed.
    while !src.past_retirement() {
        assert!(std::time::Instant::now() < deadline, "the source never retired (stage {:?})", src.stage());
        if let Err(r) = src.pump(&mut j.hs, live(), now()) {
            panic!("the source refused after the leave instead of reading the queued ack: {}", r.0);
        }
    }
    let fast = Resumer::new(Backoff { first: std::time::Duration::from_millis(100), ceiling: std::time::Duration::from_millis(400) });
    assert!(src.hand_publication(&fast), "the publication wasn't taken");
    let store = crate::handover::live::authority::store(&working).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while store.read(crate::handover::durable::MarkerKind::Activated, t).unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline, "the recipient never activated after leaving before the ack was read");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

// ── CP9 review MEDIUM-1 (CP12 step 2, the CP9 step 3 game test): Cancel after the bearer left rotates ──

/// The host game hosting online, driven to `at` (the bearer disclosed),
/// then the player clicks Cancel: the session ends, `live_bearer()` changed,
/// the old invite refuses and the new one admits, and the invitation's root
/// context is unchanged.
fn cancel_after_disclosure_rotates_the_invite(tag: &str, at: SourceStage) {
    use crate::online_admission::Admission;
    use crate::runtime_identity::{RuntimeIdentity, mint_player_attestation};
    let dir = Tmp::new(tag);
    let (mut hg, mut far) = frozen(tag, &dir);
    // The online host whose invitation Cancel must rotate (fake rendezvous
    // hub, never a relay).
    let hub = crate::rendezvous::relay_client::FakeRelayHub::new();
    let runtime = keys(SOURCE_RUNTIME);
    let att = pollster::block_on(mint_player_attestation(&keys(SOURCE_PERSONA), &runtime.public_key(), Timestamp::from(now() - 10), 90))
        .unwrap();
    let host = crate::online_host::OnlineHost::start(
        RuntimeIdentity::from_parts(runtime, Some(att)),
        key(SOURCE_PERSONA),
        Box::new(hub.client()),
        std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
        Vec::new(),
        "Loop".into(),
        vec!["wss://relay.example".into()],
        Vec::new(),
        now(),
    )
    .map_err(|(e, _)| e)
    .unwrap();
    let (old_bearer, old_link, root) = (host.live_bearer(now()), host.invite_link(now()).unwrap(), host.root_context(now()).unwrap().digest());
    assert!(old_bearer.is_some(), "no live invitation to rotate");
    hg.state.online_host = Some(host);
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(120);
    while source_stage(&hg) != Some(at) && std::time::Instant::now() < deadline {
        let _ = far.dst.as_mut().unwrap().pump(now());
        if at == SourceStage::Prepared
            && let Some(Handover::Source { session, .. }) = hg.state.handover.as_mut()
            && session.stage() == SourceStage::Publishing
        {
            session.join_publish_worker();
        }
        round(&mut hg);
    }
    // The click is handled before the next pump, so Cancel lands in `at`.
    // CP22 (CP21 review LOW-6): where the far recipient was, and how long.
    assert_eq!(
        source_stage(&hg),
        Some(at),
        "the host game never reached {at:?} (far recipient at {:?} after {:?})",
        far.dst.as_ref().map(|d| d.stage()),
        started.elapsed()
    );
    hg.state.handover_click = Some(UiAction::Cancel);
    hg.frames(1);
    assert!(hg.state.handover.is_none(), "Cancel in {at:?} didn't end the source session");
    let host = hg.state.online_host.as_ref().expect("Cancel stopped the online host");
    let stranger = [0x44; 32];
    assert_ne!(host.live_bearer(now()), old_bearer, "Cancel in {at:?} didn't rotate the live bearer");
    assert!(host.live_bearer(now()).is_some(), "Cancel in {at:?} left no live invitation");
    assert!(
        matches!(host.admission_for_link(&stranger, &old_link, now()), Admission::Refuse(_)),
        "the old invite still admits after Cancel in {at:?}"
    );
    assert!(
        matches!(host.admission_for_link(&stranger, &host.invite_link(now()).unwrap(), now()), Admission::Accept(_)),
        "the new invite doesn't admit after Cancel in {at:?}"
    );
    assert_eq!(host.root_context(now()).unwrap().digest(), root, "Cancel in {at:?} changed the root context");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_cancel_in_publishing_rotates_the_invite_and_keeps_the_root() {
    cancel_after_disclosure_rotates_the_invite("cancel-publishing", SourceStage::Publishing);
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_cancel_in_prepared_rotates_the_invite_and_keeps_the_root() {
    cancel_after_disclosure_rotates_the_invite("cancel-prepared", SourceStage::Prepared);
}

/// CP14b (CP13b review LOW-2): a recipient's Cancel past its ack says the
/// wait goes on and leaves the joined world: the link is never handed back
/// to the frozen host.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_recipient_cancel_after_the_ack_leaves_with_the_waiting_line() {
    let mut j = joined("dst-cancel-after-ack");
    let mut src = consented(&mut j);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        assert!(std::time::Instant::now() < deadline, "the recipient never reached Prepared (source {:?})", src.stage());
        j.host.lend_tick(&mut j.hs);
        if let Some(Handover::Recipient(s)) = j.hg.state.handover.as_mut() {
            let _ = s.pump(now());
        }
        if recipient_stage(&j.hg) == Some(RecipientStage::Prepared) {
            break;
        }
        if src.stage() == SourceStage::Publishing {
            src.join_publish_worker();
        }
        if src.pump(&mut j.hs, live(), now()).is_err() {
            break;
        }
    }
    let Some(Handover::Recipient(s)) = &j.hg.state.handover else { panic!("no recipient session") };
    assert!(s.ack_started(), "the ack wasn't sent at Prepared");
    let npub = s.proposal().source_npub.clone();
    j.hg.state.handover_click = Some(UiAction::Cancel);
    j.hg.frames(1);
    assert!(j.hg.state.handover.is_none(), "the session outlived the Cancel");
    assert!(
        matches!(j.hg.state.mode, crate::GameMode::Menu(_)),
        "a Cancel after the ack put the player back into the frozen world"
    );
    let crate::GameMode::Menu(menu) = &j.hg.state.mode else { unreachable!() };
    assert_eq!(menu.notice.as_deref(), Some(copy::waiting_after_ack(&npub).as_str()), "the Cancel after the ack has the wrong line");
    drop(src);
}

/// CP16 (CP14 review LOW-5, decided; LOW-4: the pump's Quarantined arm):
/// past the ack, the source aborts with its live Abort carrying the sealed
/// Aborted. The recipient verifies it and quarantines the import; the source
/// has unfrozen, so the link goes back and the player stays in the joined
/// world (not pushed to the lobby), with the quarantine line.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_recipient_after_a_verified_abort_keeps_playing_the_resumed_world() {
    let mut j = joined("dst-verified-abort");
    let mut src = consented(&mut j);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        assert!(std::time::Instant::now() < deadline, "the recipient never reached Prepared (source {:?})", src.stage());
        j.host.lend_tick(&mut j.hs);
        if let Some(Handover::Recipient(s)) = j.hg.state.handover.as_mut() {
            let _ = s.pump(now());
        }
        if recipient_stage(&j.hg) == Some(RecipientStage::Prepared) {
            break;
        }
        if src.stage() == SourceStage::Publishing {
            src.join_publish_worker();
        }
        if src.pump(&mut j.hs, live(), now()).is_err() {
            break;
        }
    }
    let Some(Handover::Recipient(s)) = &j.hg.state.handover else { panic!("no recipient session") };
    assert!(s.ack_started(), "the ack wasn't sent at Prepared");
    src.abort(&mut j.hs, copy::LEFT);
    assert!(!j.hs.handover_frozen(), "the source's abort didn't unfreeze its host");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while j.hg.state.handover.is_some() {
        assert!(std::time::Instant::now() < deadline, "the recipient never ended on the verified Aborted ({:?})", recipient_stage(&j.hg));
        j.host.lend_tick(&mut j.hs);
        j.hg.frames(1);
    }
    assert!(
        !matches!(j.hg.state.mode, crate::GameMode::Menu(_)),
        "a verified Aborted pushed the player out of a world that has resumed"
    );
    assert!(
        j.hg.state.remote_client.as_ref().is_some_and(|rc| !rc.is_parked()),
        "the link wasn't handed back to the joined world"
    );
    drop(src);
}

fn open_from_lobby(hg: &mut HeadlessGame, folder: &str) {
    hg.state.pending_menu_action=Some((crate::menu::MenuAction::LoadWorld(folder.into(),crate::spawn_pref::SpawnPref::Default),web_time::Instant::now()));
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(180);
    while !matches!(hg.state.mode,crate::GameMode::Playing) {
        hg.frames(1);
        if let crate::GameMode::Menu(menu)=&hg.state.mode { assert!(menu.notice.is_none(),"promoted load refused: {:?}",menu.notice); }
        assert!(std::time::Instant::now()<deadline,"promoted load never completed");
    }
}

/// The real GameState frozen capture must never transfer display defaults or
/// recovered metadata when the saved authored history/settings cannot be read.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_frozen_capture_refuses_torn_or_missing_metadata_without_repair() {
    for missing in [false, true] {
        let tag = if missing { "capture-metadata-missing" } else { "capture-metadata-torn" };
        let dir = Tmp::new(tag);
        let (mut hg, mut far, _) = proposed(tag, &dir);
        let world = crate::save::world_dir(&hg.state.world_name);
        let path = world.join("world_meta.json");
        let original = std::fs::read(&path).unwrap();
        assert!(crate::handover::checkpoint::source_metadata(&original).is_ok());
        round(&mut hg);
        far.rc.poll();
        let proposal = far.rc.take_handover_frames().into_iter()
            .find(|bytes| matches!(Frame::decode(bytes), Ok(Frame::Propose { .. }))).unwrap();
        let mut recipient = RecipientSession::propose(&proposal, joined_host(), identity(), dir.0.join("imports"), now()).unwrap();
        if missing { std::fs::remove_file(&path).unwrap(); }
        else { std::fs::write(&path, &original[..original.len()/2]).unwrap(); }
        let before = snapshot(&world);
        let proposal = recipient.proposal().clone();
        recipient.consent(RecipientConsent { transfer: proposal.transfer, world_name: proposal.world_name, source_npub: proposal.source_npub },
            std::net::UdpSocket::bind("127.0.0.1:0").unwrap(), far.rc.park_transport(), now()).unwrap();
        far.dst = Some(recipient);
        // Accept follows the actual hosted FIFO, then GameState freezes and
        // invokes its real capture callback in the next frame.
        round(&mut hg);
        assert!(hg.state.handover.is_none(), "unreadable metadata entered packing instead of refusing capture");
        assert!(!hg.state.hosted_server.as_ref().unwrap().handover_frozen());
        assert_eq!(snapshot(&world), before, "capture repaired, quarantined or changed source bytes");
        assert_eq!(path.exists(), !missing, "capture invented missing source metadata");
        assert!(crate::handover::live::authority::open_guard(&world).is_ok(), "a refused capture retired the original world");
        assert!(std::fs::read_dir(&dir.0.join("scratch")).unwrap().next().is_none(), "capture packed default metadata");
        assert!(far.dst.as_mut().unwrap().pump(now()).is_err(), "the recipient was not told capture refused");
        // The refusal preserves the live source; restoring its exact original
        // file is an explicit fixture operation, not the transfer path.
        std::fs::write(&path, &original).unwrap();
        let counter = hg.state.tick_counter;
        hg.ticks(1);
        assert!(hg.state.tick_counter > counter, "ordinary source play never resumed");
    }
}

/// A real lobby load action must return a new refusal without becoming live.
fn refused_from_lobby(hg: &mut HeadlessGame, folder: &str) {
    if let crate::GameMode::Menu(menu) = &mut hg.state.mode {
        menu.notice = None;
    }
    hg.state.pending_menu_action = Some((
        crate::menu::MenuAction::LoadWorld(folder.into(), crate::spawn_pref::SpawnPref::Default),
        web_time::Instant::now(),
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        hg.frames(1);
        assert!(
            !matches!(hg.state.mode, crate::GameMode::Playing),
            "a refused selected world became live"
        );
        if let crate::GameMode::Menu(menu) = &hg.state.mode
            && menu.notice.is_some()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "failed native open never returned an honest refusal"
        );
    }
}

/// The distinct released copy opens through the actual native loader with the
/// source's identity-selected state. Saved/autosaved bag and chest choose one
/// snapshot, and a complete directory copy remains portable without a ledger.
/// Private held bodies stay installation-local; a fresh reader preserves their
/// exact unresolved markers while real world-embedded Plans remain complete.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_retained_source_native_save_discard_and_complete_directory_portability() {
    let _worlds = crate::save::WorldsRootGuard::new("retained-native");
    let dir = Tmp::new("retained-native");
    // Isolate this complete source installation before its actual transfer
    // writes. Another harness thread may append to its own installation;
    // the complete original-byte comparison below remains meaningful.
    let source_ledger = dir.0.join("source-install").join("ledger");
    let _source_install = crate::handover::ledger::fixture_install_path(&source_ledger);
    assert_eq!(crate::handover::ledger::Ledger::install().path(), source_ledger.as_path());
    assert!(!source_ledger.exists());
    let shared_cell = (2, 69, 2);
    let mut shared_body = crate::plan::PlanData::debug_3x3_stone();
    shared_body.name = "Retained explicitly embedded world blueprint".into();
    shared_body.develop_state = crate::plan::DevelopState::Developed;
    let (mut hg, mut far) = frozen_after("retained-native", &dir, |hg| {
        let mut plan = crate::plan::PlanData::debug_3x3_stone();
        plan.name = "Retained source latent exposure".into();
        plan.develop_state = crate::plan::DevelopState::Latent { exposure_ticks: 12 };
        crate::plan_store::remember_checked(
            &plan,
            crate::body_residency::allocated_body_bytes(&plan).unwrap(),
        )
        .unwrap();
        hg.state.players[0].crafting_ui.cursor_item = Some(crate::item::ItemStack {
            item: crate::item::Item::Plan(plan),
            count: 1,
        });
        let mut developed = crate::plan::PlanData::debug_3x3_stone();
        developed.name = "Retained source developed body".into();
        developed.develop_state = crate::plan::DevelopState::Developed;
        crate::plan_store::remember_checked(
            &developed,
            crate::body_residency::allocated_body_bytes(&developed).unwrap(),
        )
        .unwrap();
        hg.state.players[0].crafting_ui.grid[1][2] = Some(crate::item::ItemStack {
            item: crate::item::Item::Plan(developed),
            count: 1,
        });
        hg.state.world.set_block(
            shared_cell.0,
            shared_cell.1,
            shared_cell.2,
            crate::block::STONE,
        );
        hg.state.world.set_face_attachment(
            shared_cell,
            0,
            crate::world::FaceAttachment::Blueprint(Box::new(shared_body.clone())),
        );
        let mut meta = crate::save::try_load_world_meta(&hg.state.world_name).unwrap();
        meta.genesis_block_found = true;
        meta.description = "Exact retained authored history".into();
        crate::save::save_world_meta(&hg.state.world_name, &meta).unwrap();
    });
    let original_folder = hg.state.world_name.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while !matches!(source_stage(&hg), Some(SourceStage::Retired)) {
        let _ = far.dst.as_mut().unwrap().pump(now());
        round(&mut hg);
        assert!(
            std::time::Instant::now() < deadline,
            "retained native source did not retire: {:?}",
            source_stage(&hg)
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let Some(Handover::Source { session, .. }) = &hg.state.handover else {
        panic!("source disappeared")
    };
    let folder = session
        .retained_folder()
        .expect("the retained copy was not durably released")
        .to_string();
    assert_ne!(folder, original_folder);
    let world = crate::save::world_dir(&folder);
    let proof = crate::handover::live::personal_adapter::proof(&world)
        .unwrap()
        .unwrap();
    assert!(proof.is_retained());
    assert_eq!(proof.local_owner(), key(SOURCE_PERSONA));
    let metadata = crate::save::try_load_world_meta(&folder).unwrap();
    assert!(metadata.genesis_block_found);
    assert_eq!(metadata.description, "Exact retained authored history");
    assert!(metadata.published_to.is_none() && !metadata.cloud_save);
    assert!(hg.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    hg.state.handover_fixture = Some(Fixture {
        signed_in: Some(key(SOURCE_PERSONA)),
        recipient: None,
    });
    open_from_lobby(&mut hg, &folder);
    assert_eq!(hg.state.players[0].inventory.slot(4).unwrap().count, 9);
    assert_eq!(
        hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],
        7
    );
    assert!(
        hg.state.players[0].inventory.slot(5).is_none(),
        "recipient bag donated to source"
    );
    let crate::item::Item::Plan(plan) = &hg.state.players[0]
        .crafting_ui
        .cursor_item
        .as_ref()
        .unwrap()
        .item
    else {
        panic!("retained source Plan lost")
    };
    assert_eq!(
        plan.develop_state,
        crate::plan::DevelopState::Latent { exposure_ticks: 12 }
    );
    assert!(
        plan.marker.is_none(),
        "retained private body did not resolve"
    );
    assert_eq!(
        hg.state.world.face_attachment_at(shared_cell, 0),
        Some(&crate::world::FaceAttachment::Blueprint(Box::new(
            shared_body.clone()
        )))
    );
    let retained_plan = plan.clone();
    let crate::item::Item::Plan(developed) = &hg.state.players[0].crafting_ui.grid[1][2]
        .as_ref()
        .unwrap()
        .item
    else {
        panic!("retained developed body lost")
    };
    assert_eq!(
        developed.develop_state,
        crate::plan::DevelopState::Developed
    );
    assert!(
        developed.marker.is_none(),
        "developed private body did not resolve"
    );
    let retained_developed = developed.clone();
    let chest = (3, 69, 3);
    hg.state
        .world
        .set_block(chest.0, chest.1, chest.2, crate::block::CHEST);
    hg.state
        .world
        .insert_chest(chest, crate::chest::ChestData::default());
    hg.state.save_live_world().unwrap();
    let stack = hg.state.players[0].inventory.slot(4).cloned().unwrap();
    hg.state.players[0].inventory.set_slot(4, None);
    hg.state.world.chest_at_mut(chest).unwrap().slots[0] = Some(stack);
    hg.state.players[0]
        .kill_counter
        .insert(crate::mob::MobType::Pig, 23);
    hg.state.autosave_counter = 5999;
    hg.ticks(1);
    assert!(crate::save::has_autosave(&folder));
    assert!(hg.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    open_from_lobby(&mut hg, &folder);
    assert_eq!(
        hg.state.players[0].inventory.slot(4).unwrap().count,
        9,
        "discard selected later personal autosave"
    );
    assert!(
        hg.state.world.chest_at(chest).unwrap().slots[0].is_none(),
        "discard retained later chest"
    );
    assert_eq!(
        hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],
        7
    );
    let stack = hg.state.players[0].inventory.slot(4).cloned().unwrap();
    hg.state.players[0].inventory.set_slot(4, None);
    hg.state.world.chest_at_mut(chest).unwrap().slots[0] = Some(stack);
    hg.state.players[0]
        .kill_counter
        .insert(crate::mob::MobType::Pig, 23);
    assert!(hg.state.leave_world(
        crate::world_exit::SaveChoice::Save,
        crate::world_exit::ExitTo::Lobby
    ));
    open_from_lobby(&mut hg, &folder);
    assert!(
        hg.state.players[0].inventory.slot(4).is_none(),
        "save replayed frozen source bag"
    );
    assert_eq!(
        hg.state.world.chest_at(chest).unwrap().slots[0]
            .as_ref()
            .unwrap()
            .count,
        9
    );
    assert_eq!(
        hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],
        23
    );
    let stack = hg.state.world.chest_at_mut(chest).unwrap().slots[0]
        .take()
        .unwrap();
    hg.state.players[0].inventory.set_slot(4, Some(stack));
    hg.state.players[0]
        .kill_counter
        .insert(crate::mob::MobType::Pig, 24);
    hg.state.autosave_counter = 5999;
    hg.ticks(1);
    assert!(hg.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    open_from_lobby(&mut hg, &folder);
    assert!(
        hg.state.players[0].inventory.slot(4).is_none(),
        "discard duplicated withdrawn source bag"
    );
    assert_eq!(
        hg.state.world.chest_at(chest).unwrap().slots[0]
            .as_ref()
            .unwrap()
            .count,
        9
    );
    assert_eq!(
        hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],
        23
    );
    assert!(hg.state.leave_world(
        crate::world_exit::SaveChoice::Save,
        crate::world_exit::ExitTo::Lobby
    ));
    let portable = "retained-native-full-directory-copy";
    let reader_worlds = dir.0.join("fresh-reader-worlds");
    std::fs::create_dir(&reader_worlds).unwrap();
    let portable_dir = reader_worlds.join(portable);
    // A complete directory copy, not generic archive/profile export. Include
    // the signed retained proof, exact manifest, participants and joiners.
    std::fs::create_dir(&portable_dir).unwrap();
    let mut pending = vec![world.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let output = portable_dir.join(path.strip_prefix(&world).unwrap());
            assert!(!entry.file_type().unwrap().is_symlink());
            if entry.file_type().unwrap().is_dir() {
                std::fs::create_dir(&output).unwrap();
                pending.push(path);
            } else {
                std::fs::copy(path, output).unwrap();
            }
        }
    }
    let source_world = crate::save::world_dir(&original_folder);
    let source_store =
        crate::handover::durable::MarkerStore::open_existing(source_world.join("handover"))
            .unwrap();
    let retirement =
        crate::handover::phase::RetiredCommit::read(&source_store, proof.core.transfer)
            .unwrap()
            .unwrap();
    let original_ledger = crate::handover::ledger::Ledger::install();
    assert!(original_ledger.lists(&retirement.ledger_entry()));
    let original_ledger_bytes = std::fs::read(original_ledger.path()).unwrap();
    let fresh_ledger = dir.0.join("fresh-reader-install").join("ledger");
    let missing = crate::handover::ledger::fixture_install_path(&fresh_ledger);
    assert_eq!(
        crate::handover::ledger::Ledger::install().path(),
        fresh_ledger.as_path()
    );
    assert!(!fresh_ledger.exists());
    assert!(!crate::handover::ledger::Ledger::install().lists(&retirement.ledger_entry()));
    drop(hg);
    // A new game reader under an empty installation ledger must accept the
    // released signed personal proof; old jobs/threads cannot give this copy
    // adoption rights, since it carries no original activation/receipt.
    crate::save::with_worlds_root(&reader_worlds, || {
        let private_marker = crate::plan::marker(&retained_plan);
        let developed_marker = crate::plan::marker(&retained_developed);
        assert!(crate::plan_store::resolve(private_marker).is_none());
        assert!(crate::plan_store::resolve(developed_marker).is_none());
        let mut reader = HeadlessGame::boot_into_world("retained-portable-fresh-reader");
        assert!(reader.state.leave_world(
            crate::world_exit::SaveChoice::Discard,
            crate::world_exit::ExitTo::Lobby
        ));
        reader.state.handover_fixture = Some(Fixture {
            signed_in: Some(key(SOURCE_PERSONA)),
            recipient: None,
        });
        open_from_lobby(&mut reader, portable);
        assert!(
            !fresh_ledger.exists(),
            "opening personal provenance tried to adopt original authority"
        );
        assert!(!crate::handover::ledger::Ledger::install().lists(&retirement.ledger_entry()));
        assert!(
            reader.state.players[0].inventory.slot(4).is_none(),
            "portable copy replayed frozen bag"
        );
        assert_eq!(
            reader.state.world.chest_at(chest).unwrap().slots[0]
                .as_ref()
                .unwrap()
                .count,
            9
        );
        assert_eq!(
            reader.state.players[0].kill_counter[&crate::mob::MobType::Pig],
            23
        );
        let crate::item::Item::Plan(plan) = &reader.state.players[0]
            .crafting_ui
            .cursor_item
            .as_ref()
            .unwrap()
            .item
        else {
            panic!("portable private Plan lost")
        };
        assert_eq!(
            plan.marker,
            Some(private_marker),
            "portable load lost or invented a private body"
        );
        assert!(!plan.develop_state.is_developed());
        let crate::item::Item::Plan(developed) = &reader.state.players[0].crafting_ui.grid[1][2]
            .as_ref()
            .unwrap()
            .item
        else {
            panic!("portable developed body lost")
        };
        assert_eq!(
            developed.marker,
            Some(developed_marker),
            "portable load lost or invented a developed private body"
        );
        assert!(developed.develop_state.is_developed());
        assert_eq!(
            reader.state.world.face_attachment_at(shared_cell, 0),
            Some(&crate::world::FaceAttachment::Blueprint(Box::new(
                shared_body.clone()
            ))),
            "portable world lost its deliberately embedded real Plan"
        );
        let metadata = crate::save::try_load_world_meta(portable).unwrap();
        assert!(metadata.genesis_block_found);
        assert_eq!(metadata.description, "Exact retained authored history");
        assert!(crate::handover::live::authority::open_guard(&source_world).is_err());
        assert!(reader.state.leave_world(
            crate::world_exit::SaveChoice::Discard,
            crate::world_exit::ExitTo::Lobby
        ));
        // A different known participant and an unknown persona cannot receive
        // the selected source-owner bag, even with complete proof files present.
        let selected_before = snapshot(&portable_dir);
        for actor in [RECIPIENT_PERSONA, 0x55] {
            reader.state.handover_fixture = Some(Fixture {
                signed_in: Some(key(actor)),
                recipient: None,
            });
            refused_from_lobby(&mut reader, portable);
            assert_eq!(
                snapshot(&portable_dir),
                selected_before,
                "wrong-identity load changed portable current state"
            );
        }
        reader.state.handover_fixture = Some(Fixture {
            signed_in: Some(key(SOURCE_PERSONA)),
            recipient: None,
        });
        let selected_path = portable_dir.join("world.dat");
        let selected_bytes = std::fs::read(&selected_path).unwrap();
        let (mut selected, defaulted) =
            crate::save::read_world_save_reporting(&selected_bytes).unwrap();
        assert!(!defaulted);
        selected.local_personal.as_mut().unwrap().0[0] = 255;
        crate::handover::live::personal_adapter::write_world_snapshot(
            &portable_dir,
            &portable_dir,
            &selected,
        )
        .unwrap();
        let corrupt = snapshot(&portable_dir);
        refused_from_lobby(&mut reader, portable);
        assert_eq!(
            snapshot(&portable_dir),
            corrupt,
            "corrupt selected state was repaired or replaced by a frozen bag"
        );
        std::fs::write(&selected_path, &selected_bytes).unwrap();
        if let crate::GameMode::Menu(menu) = &mut reader.state.mode {
            menu.notice = None;
        }
        open_from_lobby(&mut reader, portable);
        assert_eq!(
            reader.state.players[0].kill_counter[&crate::mob::MobType::Pig],
            23
        );
        assert!(reader.state.players[0].inventory.slot(4).is_none());
        assert!(reader.state.leave_world(
            crate::world_exit::SaveChoice::Discard,
            crate::world_exit::ExitTo::Lobby
        ));
        drop(reader);
    });
    drop(missing);
    assert_eq!(crate::handover::ledger::Ledger::install(), original_ledger);
    assert_eq!(
        std::fs::read(original_ledger.path()).unwrap(),
        original_ledger_bytes
    );
}

/// A visible copy is still inactive after a failed parent sync; after inert
/// removal, the actual native loader must confirm its directory durability.
#[cfg(unix)]
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_retained_directory_failures_refuse_native_play_and_retry_without_reset() {
    use crate::handover::live::{personal_adapter, retained};
    use crate::handover::{
        durable::{MarkerKind, MarkerStore},
        phase::RetiredCommit,
    };
    for after_unlink in [false, true] {
        let tag = if after_unlink {
            "retained-post-unlink"
        } else {
            "retained-post-rename"
        };
        let _worlds = crate::save::WorldsRootGuard::new(tag);
        let dir = Tmp::new(tag);
        let (mut hg, mut far) = frozen(tag, &dir);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        while source_stage(&hg) != Some(SourceStage::ReadyVerified) {
            far.dst.as_mut().unwrap().pump(now()).unwrap();
            round(&mut hg);
            assert!(
                std::time::Instant::now() < deadline,
                "source copy never became ready"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let Some(Handover::Source { session, .. }) = &hg.state.handover else {
            panic!("source disappeared")
        };
        let (transfer, copy) = (session.transfer(), session.retained_copy());
        let folder = format!("retained-{}", hex::encode(copy));
        let world = crate::save::world_dir(&folder);
        let original = crate::save::world_dir(&hg.state.world_name);
        let before_source = snapshot(&original);
        let path = if after_unlink {
            world.join("handover")
        } else {
            crate::save::worlds_root()
        };
        let fault =
            crate::fsync::tests::fail_directory_on_match(&path, if after_unlink { 2 } else { 1 });
        while source_stage(&hg) != Some(SourceStage::Retired) {
            let _ = far.dst.as_mut().unwrap().pump(now());
            round(&mut hg);
            assert!(
                std::time::Instant::now() < deadline,
                "source never retired at directory boundary"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            fault.failures() > 0,
            "the executing source thread escaped the injected directory boundary"
        );
        assert!(
            world.is_dir(),
            "rename did not precede the injected boundary"
        );
        let copy_store = MarkerStore::open_existing(world.join("handover")).unwrap();
        assert_eq!(
            copy_store.exists(MarkerKind::RetainedInert, copy).unwrap(),
            !after_unlink,
            "the fault did not land after the intended rename/unlink"
        );
        assert_eq!(
            snapshot(&original),
            before_source,
            "a retained release failure changed original source data"
        );
        assert!(
            crate::handover::live::authority::open_guard(&original).is_err(),
            "the retired original resumed play"
        );
        let source_store = MarkerStore::open_existing(original.join("handover")).unwrap();
        let retired = RetiredCommit::read(&source_store, transfer)
            .unwrap()
            .unwrap();
        assert!(hg.state.leave_world(
            crate::world_exit::SaveChoice::Discard,
            crate::world_exit::ExitTo::Lobby
        ));
        hg.state.handover_fixture = Some(Fixture {
            signed_in: Some(key(SOURCE_PERSONA)),
            recipient: None,
        });
        let before_copy = snapshot(&world);
        let prior_failures = fault.failures();
        refused_from_lobby(&mut hg, &folder);
        if after_unlink {
            assert!(
                fault.failures() > prior_failures,
                "native durable confirmation ran outside the scoped executing thread"
            );
        }
        assert_eq!(
            snapshot(&world),
            before_copy,
            "a refused native open rewrote selected personal/world data"
        );
        drop(fault);
        assert_eq!(
            retained::finish_retired(
                &original,
                &retired,
                &crate::handover::ledger::Ledger::install()
            )
            .unwrap(),
            folder
        );
        assert!(!copy_store.exists(MarkerKind::RetainedInert, copy).unwrap());
        if let crate::GameMode::Menu(menu) = &mut hg.state.mode {
            menu.notice = None;
        }
        open_from_lobby(&mut hg, &folder);
        assert_eq!(hg.state.players[0].inventory.slot(4).unwrap().count, 9);
        assert_eq!(
            hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],
            7
        );
        hg.state.players[0].inventory.set_slot(
            4,
            Some(crate::item::ItemStack::new_block(crate::block::GLASS, 17)),
        );
        hg.state.players[0]
            .kill_counter
            .insert(crate::mob::MobType::Pig, 23);
        assert!(hg.state.leave_world(
            crate::world_exit::SaveChoice::Save,
            crate::world_exit::ExitTo::Lobby
        ));
        let played = snapshot(&world);
        retained::finish_retired(
            &original,
            &retired,
            &crate::handover::ledger::Ledger::install(),
        )
        .unwrap();
        assert_eq!(
            snapshot(&world),
            played,
            "retirement retry replaced the played copy with its old prepared snapshot"
        );
        open_from_lobby(&mut hg, &folder);
        assert_eq!(hg.state.players[0].inventory.slot(4).unwrap().count, 17);
        assert_eq!(
            hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],
            23
        );
        assert!(
            personal_adapter::proof(&world)
                .unwrap()
                .unwrap()
                .is_retained()
        );
        assert!(hg.state.leave_world(
            crate::world_exit::SaveChoice::Discard,
            crate::world_exit::ExitTo::Lobby
        ));
        assert_eq!(snapshot(&original), before_source);
    }
}

/// Even forcing an inactive prepared directory into the ordinary world list
/// cannot open it. Cancelling before retirement keeps the original playable.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_handover_retained_inactive_copy_refuses_native_open_before_original_retirement() {
    let _worlds = crate::save::WorldsRootGuard::new("retained-native-inactive");
    let dir = Tmp::new("retained-native-inactive");
    let (mut hg, mut far) = frozen("retained-native-inactive", &dir);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while source_stage(&hg) != Some(SourceStage::ReadyVerified) {
        far.dst.as_mut().unwrap().pump(now()).unwrap();
        round(&mut hg);
        assert!(
            std::time::Instant::now() < deadline,
            "source inactive copy never reached Ready"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let Some(Handover::Source { session, .. }) = &hg.state.handover else {
        panic!("source disappeared")
    };
    assert!(!session.past_retirement());
    let copy = session.retained_copy();
    let stage = crate::save::worlds_root()
        .join(".handover-retained")
        .join(hex::encode(copy));
    let working = std::fs::read_dir(&stage)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("working"))
        .find(|path| path.is_dir())
        .unwrap();
    let forced = "retained-native-forced-inactive";
    let destination = crate::save::world_dir(forced);
    // Fixture-only relocation makes the candidate reach the real menu loader;
    // it never changes its inert marker or gives it retirement provenance.
    std::fs::rename(working, &destination).unwrap();
    let before_copy = snapshot(&destination);
    let copy_store =
        crate::handover::durable::MarkerStore::open_existing(destination.join("handover")).unwrap();
    assert!(
        copy_store
            .exists(crate::handover::durable::MarkerKind::RetainedInert, copy)
            .unwrap()
    );
    let original_folder = hg.state.world_name.clone();
    let original = crate::save::world_dir(&original_folder);
    let before_source = snapshot(&original);
    hg.state.handover_click = Some(UiAction::Cancel);
    hg.frames(1);
    assert!(hg.state.handover.is_none());
    assert!(!hg.state.hosted_server.as_ref().unwrap().handover_frozen());
    assert!(
        crate::handover::live::authority::open_guard(&original).is_ok(),
        "cancelling the inactive copy retired the source"
    );
    assert_eq!(snapshot(&original), before_source);
    let tick = hg.state.tick_counter;
    hg.hosted_ticks(1);
    assert!(
        hg.state.tick_counter > tick,
        "the original stopped playing before retirement"
    );
    assert_eq!(hg.state.players[0].inventory.slot(4).unwrap().count, 9);
    assert_eq!(
        hg.state.players[0].kill_counter[&crate::mob::MobType::Pig],
        7
    );
    assert!(hg.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
    hg.state.handover_fixture = Some(Fixture {
        signed_in: Some(key(SOURCE_PERSONA)),
        recipient: None,
    });
    refused_from_lobby(&mut hg, forced);
    assert_eq!(
        snapshot(&destination),
        before_copy,
        "native opening changed an inactive prepared copy"
    );
    assert!(
        copy_store
            .exists(crate::handover::durable::MarkerKind::RetainedInert, copy)
            .unwrap()
    );
    assert!(crate::handover::live::authority::open_guard(&original).is_ok());
    if let crate::GameMode::Menu(menu) = &mut hg.state.mode {
        menu.notice = None;
    }
    open_from_lobby(&mut hg, &original_folder);
    assert!(matches!(hg.state.mode, crate::GameMode::Playing));
    assert!(hg.state.leave_world(
        crate::world_exit::SaveChoice::Discard,
        crate::world_exit::ExitTo::Lobby
    ));
}

mod personal_delay;
