//! `--keep-world`: the world's folder and lock, the served loop end to end
//! over an in-memory pipe, and the gate: a signed join over the pipe is
//! verified over the binding the Keep passed, waits for the gate with nothing
//! sent, and is seated, refused or let go as the gate and the Keep say.

use super::*;
use crate::keep_pipe::tests::{Capture, Feed, TestPipe};
use crate::keep_pipe::{FromChild, ToChild, PROTO};
use crate::protocol::{self, PacketType};

// --- the world's folder ---

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "axenstax-keep-world-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("axenstax").join("worlds")).unwrap();
    dir
}

#[test]
fn the_world_must_be_named_alike_by_the_flag_and_the_environment() {
    let dir = scratch("names");
    let worlds = dir.join("axenstax").join("worlds");
    let env = |world: Option<&'static str>, worlds: PathBuf| {
        move |key: &str| match key {
            "AXENSTAX_WORLD" => world.map(str::to_string),
            "AXENSTAX_WORLDS_DIR" => Some(worlds.to_string_lossy().into_owned()),
            _ => None,
        }
    };
    let ok = keep_world_folder(&args(&["--keep-world", "--world", "keep"]), env(Some("keep"), worlds.clone())).unwrap();
    assert_eq!(ok, KeepWorld { name: "keep".into(), data_root: dir.join("axenstax") });
    for (list, world) in [
        (&["--keep-world"][..], Some("keep")),
        (&["--keep-world", "--world"][..], Some("keep")),
        (&["--keep-world", "--world", "keep"][..], None),
        (&["--keep-world", "--world", "keep"][..], Some("other")),
        (&["--keep-world", "--world", "../keep"][..], Some("../keep")),
        (&["--keep-world", "--world", "a/b"][..], Some("a/b")),
        (&["--keep-world", "--world", ".."][..], Some("..")),
    ] {
        assert!(keep_world_folder(&args(list), env(world, worlds.clone())).is_err(), "{list:?} {world:?}");
    }
    let missing = keep_world_folder(&args(&["--world", "keep"]), env(Some("keep"), dir.join("nowhere")));
    assert!(missing.unwrap_err().contains("is not a folder"));
    let unset = keep_world_folder(&args(&["--world", "keep"]), |k: &str| (k == "AXENSTAX_WORLD").then(|| "keep".to_string()));
    assert!(unset.unwrap_err().contains("AXENSTAX_WORLDS_DIR"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_process_on_the_same_world_is_refused_and_touches_nothing() {
    let dir = scratch("lock");
    let worlds = dir.join("axenstax").join("worlds");
    let first = lock_world(&worlds, "keep").expect("the first lock");
    let before: Vec<_> = std::fs::read_dir(worlds.join("keep")).unwrap().map(|e| e.unwrap().file_name()).collect();
    // A second open of the lock file conflicts even in one process (`flock`
    // is per open file), as a second server process would.
    match lock_world(&worlds, "keep") {
        Err(LockFailure::Held(path)) => assert_eq!(path, worlds.join("keep").join(LOCK_FILE)),
        other => panic!("expected Held, got {other:?}"),
    }
    let after: Vec<_> = std::fs::read_dir(worlds.join("keep")).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after);
    assert_eq!(after, vec![std::ffi::OsString::from(LOCK_FILE)]);
    drop(first);
    lock_world(&worlds, "keep").expect("free again once the first is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- the served loop ---

/// `serve` on its own thread, with the worlds root at `root`.
fn serve_on(root: &Path, world: &str, feed: Feed, capture: Capture) -> std::thread::JoinHandle<i32> {
    let (root, world) = (root.to_path_buf(), world.to_string());
    std::thread::spawn(move || {
        crate::save::with_worlds_root(&root, || serve(&world, feed, capture, Duration::from_secs(3_600)))
    })
}

/// Wait (up to 60 s) for a frame `want` picks, keeping every frame seen.
fn wait_for(pipe: &mut TestPipe, seen: &mut Vec<FromChild>, want: impl Fn(&FromChild) -> bool) -> FromChild {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        for frame in pipe.take() {
            let hit = want(&frame);
            seen.push(frame.clone());
            if hit {
                return frame;
            }
        }
        assert!(Instant::now() < end, "never came; saw {seen:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn world_dat(root: &Path, world: &str) -> PathBuf {
    root.join(world).join("world.dat")
}

#[test]
fn hello_gets_ready_and_shutdown_saves_the_world_says_saved_and_exits_zero() {
    let dir = scratch("serve");
    let root = dir.join("axenstax").join("worlds");
    let (mut pipe, feed, capture) = TestPipe::new();
    let server = serve_on(&root, "keep", feed, capture);
    pipe.send(&ToChild::Hello { proto: PROTO, max_players: 8 });
    let mut seen = Vec::new();
    let ready = wait_for(&mut pipe, &mut seen, |f| matches!(f, FromChild::Ready { .. }));
    assert_eq!(ready, FromChild::Ready { protocol: protocol::PROTOCOL_VERSION });
    assert_eq!(seen.len(), 1, "Ready is the first frame");
    // The new world was created and saved at tick 0, before Ready.
    let dat = world_dat(&root, "keep");
    let at_ready = std::fs::metadata(&dat).expect("world.dat from tick 0").modified().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    pipe.send(&ToChild::Shutdown);
    assert_eq!(server.join().unwrap(), 0);
    let all = pipe.all();
    assert_eq!(all.last(), Some(&FromChild::Saved { ok: true }), "{all:?}");
    let at_stop = std::fs::metadata(&dat).unwrap().modified().unwrap();
    assert!(at_stop > at_ready, "Shutdown wrote world.dat again ({at_ready:?} -> {at_stop:?})");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_end_of_stdin_saves_and_exits_zero_and_a_restart_opens_the_same_world() {
    let dir = scratch("eof");
    let root = dir.join("axenstax").join("worlds");
    let seed_of = |root: &Path| crate::save::with_worlds_root(root, || crate::save::load_world_meta("keep").seed);
    let mut stamps = Vec::new();
    for run in 0..2 {
        let (mut pipe, feed, capture) = TestPipe::new();
        let server = serve_on(&root, "keep", feed, capture);
        pipe.send(&ToChild::Hello { proto: PROTO, max_players: 8 });
        wait_for(&mut pipe, &mut Vec::new(), |f| matches!(f, FromChild::Ready { .. }));
        let dat = world_dat(&root, "keep");
        let at_ready = std::fs::metadata(&dat).unwrap().modified().unwrap();
        stamps.push(seed_of(&root));
        std::thread::sleep(Duration::from_millis(50));
        pipe.close();
        assert_eq!(server.join().unwrap(), 0, "run {run}");
        assert!(std::fs::metadata(&dat).unwrap().modified().unwrap() > at_ready, "run {run} saved on the way out");
        assert_eq!(pipe.all().last(), Some(&FromChild::Saved { ok: true }));
    }
    assert_eq!(stamps[0], stamps[1], "the second start opened the first one's world, not a new one");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pipe_that_does_not_start_with_hello_in_this_protocol_exits_non_zero_before_the_world_opens() {
    for first in [ToChild::Shutdown, ToChild::Hello { proto: 2, max_players: 8 }] {
        let dir = scratch("nohello");
        let root = dir.join("axenstax").join("worlds");
        let (pipe, feed, capture) = TestPipe::new();
        let server = serve_on(&root, "keep", feed, capture);
        pipe.send(&first);
        assert_eq!(server.join().unwrap(), EXIT_PROTOCOL, "{first:?}");
        assert!(pipe.all().is_empty(), "no Ready");
        assert!(!root.join("keep").exists(), "nothing of the world was written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn an_oversize_frame_after_ready_is_a_clean_exit_non_zero() {
    let dir = scratch("oversize");
    let root = dir.join("axenstax").join("worlds");
    let (mut pipe, feed, capture) = TestPipe::new();
    let server = serve_on(&root, "keep", feed, capture);
    pipe.send(&ToChild::Hello { proto: PROTO, max_players: 8 });
    wait_for(&mut pipe, &mut Vec::new(), |f| matches!(f, FromChild::Ready { .. }));
    pipe.send_raw(((crate::keep_pipe::MAX_FRAME + 1) as u32).to_le_bytes().to_vec());
    assert_eq!(server.join().unwrap(), EXIT_PROTOCOL);
    assert!(!pipe.all().contains(&FromChild::Saved { ok: true }), "no Saved after a protocol error");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_joiner_that_comes_before_ready_waits_and_is_challenged_once_the_world_is_open() {
    let dir = scratch("early");
    let root = dir.join("axenstax").join("worlds");
    let (mut pipe, feed, capture) = TestPipe::new();
    let server = serve_on(&root, "keep", feed, capture);
    pipe.send(&ToChild::Hello { proto: PROTO, max_players: 8 });
    pipe.send(&ToChild::Joined { peer: 5, binding: [5; 32] });
    let mut seen = Vec::new();
    wait_for(&mut pipe, &mut seen, |f| challenge_nonce(f, 5).is_some());
    assert!(matches!(seen.first(), Some(FromChild::Ready { .. })), "Ready came first: {seen:?}");
    pipe.send(&ToChild::Shutdown);
    assert_eq!(server.join().unwrap(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- the gate, over a server ticked here ---

/// A Keep world's server, ticked on this thread, and the Keep's end of its pipe.
struct Rig {
    hs: HostedServer,
    pipe: TestPipe,
    out: keep_pipe::PipeOut,
    frames: Vec<FromChild>,
    _worlds: crate::save::WorldsRootGuard,
}

fn rig(tag: &str) -> Rig {
    let worlds = crate::save::WorldsRootGuard::new(&format!("keep-gate-{tag}"));
    let (pipe, feed, capture) = TestPipe::new();
    let out = keep_pipe::spawn_writer(capture).unwrap();
    let (joiners, joined) = mpsc::channel();
    let control = keep_pipe::spawn_reader(feed, out.clone(), joiners).unwrap();
    pipe.send(&ToChild::Hello { proto: PROTO, max_players: 8 });
    assert_eq!(control.recv().unwrap(), Control::Hello { max_players: 8 });
    let hs = open_world("keep", 8, joined).expect("the world opens");
    Rig { hs, pipe, out, frames: Vec::new(), _worlds: worlds }
}

impl Rig {
    /// Tick until a frame `want` picks has come (at most `ticks` ticks), keeping
    /// every frame in `self.frames`.
    fn tick_until(&mut self, ticks: usize, want: impl Fn(&FromChild) -> bool) -> FromChild {
        for _ in 0..ticks {
            self.hs.tick();
            assert!(self.out.flush(Duration::from_secs(5)));
            for frame in self.pipe.take() {
                let hit = want(&frame);
                self.frames.push(frame.clone());
                if hit {
                    return frame;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("never came in {ticks} ticks; saw {:?}", self.frames.iter().map(short).collect::<Vec<_>>());
    }

    /// Tick `ticks` times and return the frames that came meanwhile.
    fn tick_for(&mut self, ticks: usize) -> Vec<FromChild> {
        let from = self.frames.len();
        for _ in 0..ticks {
            self.hs.tick();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(self.out.flush(Duration::from_secs(5)));
        self.frames.extend(self.pipe.take());
        self.frames[from..].to_vec()
    }

    /// Peer `peer` connects with `binding` and is sent its challenge's nonce.
    fn connect(&mut self, peer: u32, binding: [u8; 32]) -> String {
        self.pipe.send(&ToChild::Joined { peer, binding });
        let frame = self.tick_until(100, |f| challenge_nonce(f, peer).is_some());
        challenge_nonce(&frame, peer).unwrap()
    }

    /// Peer `peer` sends a game packet.
    fn says(&self, peer: u32, packet: Vec<u8>) {
        self.pipe.send(&ToChild::Reliable { peer, bytes: packet });
    }

    /// Peer `peer` connects over `binding` and joins as `keys`, signing over
    /// `signed_over`; the server's question to the gate, if it asks one.
    fn join(&mut self, peer: u32, binding: [u8; 32], keys: &nostr::Keys, signed_over: [u8; 32]) {
        let nonce = self.connect(peer, binding);
        self.says(peer, signed_join(keys, &nonce, signed_over));
    }

    /// The game packets sent to `peer` in `frames`, by type.
    fn packets_to(frames: &[FromChild], peer: u32) -> Vec<PacketType> {
        frames
            .iter()
            .filter_map(|f| match f {
                FromChild::SendReliable { peer: p, bytes } if *p == peer => {
                    protocol::deserialize_header(bytes).map(|(t, _)| t)
                }
                _ => None,
            })
            .collect()
    }

    /// Fully seat `keys` on `peer` (gate says yes).
    fn seat(&mut self, peer: u32, binding: [u8; 32], keys: &nostr::Keys) {
        self.join(peer, binding, keys, binding);
        self.tick_until(100, |f| matches!(f, FromChild::GateAsk { peer: p, .. } if *p == peer));
        self.pipe.send(&ToChild::GateAnswer { peer, ok: true, reason: String::new() });
        self.tick_until(100, |f| matches!(f, FromChild::Seated { peer: p, .. } if *p == peer));
    }
}

/// A frame, briefly, for a failing test's message.
fn short(frame: &FromChild) -> String {
    match frame {
        FromChild::SendReliable { peer, bytes } => {
            format!("SendReliable({peer}, {:?})", protocol::deserialize_header(bytes).map(|(t, _)| t))
        }
        other => format!("{other:?}"),
    }
}

/// The nonce of a `Challenge` sent to `peer`, if `frame` is one.
fn challenge_nonce(frame: &FromChild, peer: u32) -> Option<String> {
    let FromChild::SendReliable { peer: p, bytes } = frame else { return None };
    let (PacketType::Challenge, payload) = protocol::deserialize_header(bytes)? else { return None };
    (*p == peer).then(|| protocol::safe_deserialize::<protocol::ChallengePacket>(payload).ok().map(|c| c.nonce_hex))?
}

/// The reason of a `JoinReject` sent to `peer` in `frames`.
fn reject_reason(frames: &[FromChild], peer: u32) -> Option<String> {
    frames.iter().find_map(|f| {
        let FromChild::SendReliable { peer: p, bytes } = f else { return None };
        let (PacketType::JoinReject, payload) = protocol::deserialize_header(bytes)? else { return None };
        (*p == peer).then(|| protocol::safe_deserialize::<protocol::JoinRejectPacket>(payload).ok().map(|r| r.reason))?
    })
}

/// A `JoinRequest` signed by `keys` over the connection binding `binding`.
fn signed_join(keys: &nostr::Keys, nonce_hex: &str, binding: [u8; 32]) -> Vec<u8> {
    let mut request = crate::remote_client::build_join_request_guest("Tester", 0);
    request.auth_event = Some(
        pollster::block_on(crate::signet::native_signer::build_join_auth_event(
            keys,
            nonce_hex,
            &crate::signet::join_origin(Some(binding)),
        ))
        .unwrap(),
    );
    protocol::serialize_packet(PacketType::JoinRequest, &request)
}

fn keys(n: u8) -> nostr::Keys {
    nostr::Keys::new(nostr::SecretKey::from_slice(&[n; 32]).unwrap())
}

fn persona(keys: &nostr::Keys) -> [u8; 32] {
    keys.public_key().to_bytes()
}

fn about(frames: &[FromChild], peer: u32) -> Vec<FromChild> {
    frames
        .iter()
        .filter(|f| match f {
            FromChild::SendReliable { peer: p, .. }
            | FromChild::Disconnect { peer: p, .. }
            | FromChild::GateAsk { peer: p, .. }
            | FromChild::Seated { peer: p, .. }
            | FromChild::Unseated { peer: p } => *p == peer,
            _ => false,
        })
        .cloned()
        .collect()
}

#[test]
fn a_signed_join_over_the_pipe_verifies_over_the_keeps_binding_and_fails_over_any_other() {
    let mut rig = rig("binding");
    let ann = keys(41);
    // Signed over the binding of its own connection: verified, and the gate is
    // asked about exactly the persona that signed.
    rig.join(1, [0xa1; 32], &ann, [0xa1; 32]);
    let ask = rig.tick_until(100, |f| matches!(f, FromChild::GateAsk { peer: 1, .. }));
    assert_eq!(ask, FromChild::GateAsk { peer: 1, persona: persona(&ann) });
    // Signed over another connection's binding (a relayed signature): refused
    // by the server's own check, and the gate is never asked.
    rig.join(2, [0xb2; 32], &ann, [0xa1; 32]);
    rig.tick_until(100, |f| matches!(f, FromChild::Disconnect { peer: 2, .. }));
    let reason = reject_reason(&rig.frames, 2).expect("the joiner is told");
    assert!(reason.contains("origin"), "{reason}");
    assert!(!rig.frames.iter().any(|f| matches!(f, FromChild::GateAsk { peer: 2, .. })));
    assert!(!Rig::packets_to(&rig.frames, 2).contains(&PacketType::JoinAccept));
}

#[test]
fn a_held_join_is_sent_nothing_until_the_gate_says_yes_then_joins_and_is_seated() {
    let mut rig = rig("hold");
    let (ann, bea) = (keys(41), keys(42));
    // Ann plays, so the world is busy sending world data to someone.
    rig.seat(1, [0xa1; 32], &ann);
    rig.join(2, [0xb2; 32], &bea, [0xb2; 32]);
    rig.tick_until(100, |f| matches!(f, FromChild::GateAsk { peer: 2, .. }));
    let challenge_only = Rig::packets_to(&rig.frames, 2);
    assert_eq!(challenge_only, vec![PacketType::Challenge], "before the question");
    let held = rig.tick_for(60);
    assert!(about(&held, 2).is_empty(), "nothing to the held joiner: {:?}", about(&held, 2).iter().map(short).collect::<Vec<_>>());
    assert!(!Rig::packets_to(&held, 1).is_empty(), "while Ann is sent the world");
    assert!(!rig.hs.slot_is_free(1), "the slot is held, not freed");
    // Yes: the join goes on, JoinAccept first, then the world; then seated.
    rig.pipe.send(&ToChild::GateAnswer { peer: 2, ok: true, reason: String::new() });
    let seated = rig.tick_until(100, |f| matches!(f, FromChild::Seated { peer: 2, .. }));
    let FromChild::Seated { persona: seated_as, name, .. } = seated else { unreachable!() };
    assert_eq!(seated_as, persona(&bea));
    assert!(!name.is_empty());
    let to_bea = Rig::packets_to(&rig.frames, 2);
    assert_eq!(to_bea[..2], [PacketType::Challenge, PacketType::JoinAccept], "{to_bea:?}");
    let after = rig.tick_for(10);
    assert!(Rig::packets_to(&after, 2).contains(&PacketType::StateUpdate), "Bea is sent the world now");
    // Bea leaves: unseated, and no Disconnect for a peer the Keep closed.
    rig.pipe.send(&ToChild::Left { peer: 2 });
    rig.tick_until(100, |f| matches!(f, FromChild::Unseated { peer: 2 }));
    let rest = rig.tick_for(5);
    assert!(!rig.frames.iter().any(|f| matches!(f, FromChild::Disconnect { peer: 2, .. })), "{rest:?}");
}

#[test]
fn a_gate_refusal_frees_the_slot_tells_the_player_and_evicts_no_one() {
    let mut rig = rig("refuse");
    let ann = keys(41);
    rig.seat(1, [0xa1; 32], &ann);
    // Ann again, from a second connection: verified, held, refused.
    rig.join(2, [0xb2; 32], &ann, [0xb2; 32]);
    rig.tick_until(100, |f| matches!(f, FromChild::GateAsk { peer: 2, .. }));
    rig.pipe.send(&ToChild::GateAnswer { peer: 2, ok: false, reason: "This player's pass was cancelled".into() });
    let gone = rig.tick_until(100, |f| matches!(f, FromChild::Disconnect { peer: 2, .. }));
    assert_eq!(gone, FromChild::Disconnect { peer: 2, reason: "This player's pass was cancelled".into() });
    assert_eq!(reject_reason(&rig.frames, 2).as_deref(), Some("This player's pass was cancelled"));
    assert!(!Rig::packets_to(&rig.frames, 2).contains(&PacketType::JoinAccept));
    assert!(rig.hs.slot_is_free(1), "the slot was released");
    // The Keep closes the link and says Left for a peer the server let go: fine.
    rig.pipe.send(&ToChild::Left { peer: 2 });
    let later = rig.tick_for(20);
    assert!(about(&later, 2).is_empty());
    // The seated Ann was not evicted by the refused twin.
    assert!(!rig.frames.iter().any(|f| matches!(f, FromChild::Unseated { peer: 1 } | FromChild::Disconnect { peer: 1, .. })));
    assert!(!rig.hs.slot_is_free(0));
}

#[test]
fn a_guest_is_refused_before_any_question_to_the_gate() {
    let mut rig = rig("guest");
    rig.connect(1, [0xa1; 32]);
    let guest = crate::remote_client::build_join_request_guest("Guest", 0);
    rig.says(1, protocol::serialize_packet(PacketType::JoinRequest, &guest));
    rig.tick_until(100, |f| matches!(f, FromChild::Disconnect { peer: 1, .. }));
    assert!(reject_reason(&rig.frames, 1).is_some(), "the guest is told");
    assert!(!rig.frames.iter().any(|f| matches!(f, FromChild::GateAsk { .. })));
    assert!(rig.hs.slot_is_free(0));
}

#[test]
fn a_gate_answer_for_a_peer_that_left_or_was_never_here_is_ignored() {
    let mut rig = rig("stale");
    rig.pipe.send(&ToChild::GateAnswer { peer: 99, ok: true, reason: String::new() });
    rig.join(1, [0xa1; 32], &keys(41), [0xa1; 32]);
    rig.tick_until(100, |f| matches!(f, FromChild::GateAsk { peer: 1, .. }));
    // Gone before the answer: the held slot is let go, and the late yes seats no one.
    rig.pipe.send(&ToChild::Left { peer: 1 });
    rig.tick_for(5);
    assert!(rig.hs.slot_is_free(0));
    rig.pipe.send(&ToChild::GateAnswer { peer: 1, ok: true, reason: String::new() });
    rig.pipe.send(&ToChild::Reliable { peer: 1, bytes: vec![1, 2, 3] });
    let later = rig.tick_for(20);
    assert!(about(&later, 1).is_empty(), "{later:?}");
    assert!(!Rig::packets_to(&rig.frames, 1).contains(&PacketType::JoinAccept));
}
