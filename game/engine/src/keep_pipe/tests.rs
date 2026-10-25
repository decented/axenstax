//! The Keep pipe: the frame codec, the reader's routing and a peer's
//! transport, driven with in-memory streams. Also the test pipe the
//! `keep_world` tests drive a whole server with.

use super::*;
use crate::protocol::PacketType;

// --- test-only encoding the other way round (what the Keep does) ---

/// Encode a frame as the Keep sends it.
pub(crate) fn encode_to_child(message: &ToChild) -> Vec<u8> {
    let framed = match message {
        ToChild::Hello { proto, max_players } => frame(HELLO, &[&proto.to_le_bytes(), &[*max_players]]),
        ToChild::Joined { peer, binding } => frame(JOINED, &[&peer.to_le_bytes(), binding]),
        ToChild::Reliable { peer, bytes } => frame(RELIABLE, &[&peer.to_le_bytes(), bytes]),
        ToChild::Lossy { peer, bytes } => frame(LOSSY, &[&peer.to_le_bytes(), bytes]),
        ToChild::Left { peer } => frame(LEFT, &[&peer.to_le_bytes()]),
        ToChild::GateAnswer { peer, ok, reason } => {
            frame(GATE_ANSWER, &[&peer.to_le_bytes(), &[u8::from(*ok)], reason.as_bytes()])
        }
        ToChild::Shutdown => frame(SHUTDOWN, &[]),
    };
    framed.expect("a test frame fits")
}

/// Decode a frame the server sent (what the Keep does with its stdout).
pub(crate) fn decode_from_child(body: &[u8]) -> Result<FromChild, FrameError> {
    let (&tag, payload) = body.split_first().ok_or(FrameError::BadLength(0))?;
    let mut c = Cursor { tag, rest: payload };
    let message = match tag {
        READY => FromChild::Ready { protocol: c.u32()? },
        SEND_RELIABLE => FromChild::SendReliable { peer: c.u32()?, bytes: c.bytes() },
        DISCONNECT => FromChild::Disconnect { peer: c.u32()?, reason: c.text()? },
        GATE_ASK => FromChild::GateAsk { peer: c.u32()?, persona: c.key()? },
        SEATED => FromChild::Seated { peer: c.u32()?, persona: c.key()?, name: c.text()? },
        UNSEATED => FromChild::Unseated { peer: c.u32()? },
        SAVED => FromChild::Saved { ok: c.flag()? },
        other => return Err(FrameError::UnknownTag(other)),
    };
    c.end()?;
    Ok(message)
}

/// Every whole frame in `bytes`, decoded.
pub(crate) fn decode_all(bytes: &[u8]) -> Vec<FromChild> {
    let mut stream = bytes;
    let mut frames = Vec::new();
    while let Some(body) = read_frame(&mut stream).expect("whole frames only") {
        frames.push(decode_from_child(&body).expect("a frame the Keep reads"));
    }
    frames
}

/// A stdout the test reads back.
#[derive(Clone, Default)]
pub(crate) struct Capture(pub Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A stdout that is closed (the Keep went away).
struct Closed;

impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// A stdout that does not take a byte until the test lets go of `gate` (drops
/// its sender): the Keep that has stopped reading. Everything written after
/// that lands in `into`.
pub(crate) struct Blocked {
    pub gate: mpsc::Receiver<()>,
    pub into: Capture,
}

impl Write for Blocked {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Blocks until the sender is dropped; then returns at once, every time.
        let _ = self.gate.recv();
        self.into.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A stdin the test writes into while the reader runs; dropping the sender
/// is the Keep closing it.
pub(crate) struct Feed {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    at: usize,
}

impl Read for Feed {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.at == self.buf.len() {
            match self.rx.recv() {
                Ok(bytes) => {
                    self.buf = bytes;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// The Keep's end of a pipe to a server in this process: write frames in,
/// read the server's frames out.
pub(crate) struct TestPipe {
    feed: Option<mpsc::Sender<Vec<u8>>>,
    capture: Capture,
    /// How many of the captured frames [`Self::take`] has handed out.
    seen: usize,
}

impl TestPipe {
    /// A pipe and its two ends for the server: its stdin and its stdout.
    pub(crate) fn new() -> (TestPipe, Feed, Capture) {
        let (tx, rx) = mpsc::channel();
        let capture = Capture::default();
        (TestPipe { feed: Some(tx), capture: capture.clone(), seen: 0 }, Feed { rx, buf: Vec::new(), at: 0 }, capture)
    }
    pub(crate) fn send(&self, message: &ToChild) {
        self.send_raw(encode_to_child(message));
    }
    pub(crate) fn send_raw(&self, bytes: Vec<u8>) {
        if let Some(feed) = &self.feed {
            let _ = feed.send(bytes);
        }
    }
    /// The Keep closes the server's stdin.
    pub(crate) fn close(&mut self) {
        self.feed = None;
    }
    /// Every frame the server has written so far.
    pub(crate) fn all(&self) -> Vec<FromChild> {
        decode_all(&self.capture.0.lock().unwrap())
    }
    /// The frames written since the last call.
    pub(crate) fn take(&mut self) -> Vec<FromChild> {
        let all = self.all();
        let new = all[self.seen.min(all.len())..].to_vec();
        self.seen = all.len();
        new
    }
}

/// The reader run to the end of `input` on this thread: its control messages,
/// the transports it handed over, and what it wrote.
fn run_reader(input: &[u8], cap: usize) -> (Vec<Control>, Vec<Box<dyn ServerTransport>>, Capture, PipeOut) {
    let capture = Capture::default();
    let out = spawn_writer(capture.clone()).unwrap();
    let (jtx, jrx) = mpsc::channel();
    let (ctx, crx) = mpsc::channel();
    read_loop(input, &out, &jtx, &ctx, cap);
    let controls = crx.try_iter().collect();
    let joined = jrx.try_iter().collect();
    (controls, joined, capture, out)
}

fn frames_of(capture: &Capture, out: &PipeOut) -> Vec<FromChild> {
    assert!(out.flush(Duration::from_secs(5)));
    decode_all(&capture.0.lock().unwrap())
}

fn stream(messages: &[ToChild]) -> Vec<u8> {
    messages.iter().flat_map(encode_to_child).collect()
}

fn hello() -> ToChild {
    ToChild::Hello { proto: PROTO, max_players: 8 }
}

const PEER: u32 = 0x0403_0201;

fn decode_one(bytes: &[u8]) -> Result<Option<ToChild>, FrameError> {
    let mut stream = bytes;
    read_frame(&mut stream)?.map(|body| decode_to_child(&body)).transpose()
}

// --- the codec ---

#[test]
fn a_joined_frame_has_the_exact_bytes_of_the_plan() {
    let mut golden = vec![37, 0, 0, 0, 2, 0x01, 0x02, 0x03, 0x04];
    golden.extend_from_slice(&[9; 32]);
    assert_eq!(encode_to_child(&ToChild::Joined { peer: PEER, binding: [9; 32] }), golden);
    assert_eq!(decode_one(&golden).unwrap(), Some(ToChild::Joined { peer: PEER, binding: [9; 32] }));
    // And the other way: Ready, Saved and GateAsk as the Keep reads them.
    assert_eq!(encode_from_child(&FromChild::Ready { protocol: 95 }).unwrap(), vec![5, 0, 0, 0, 64, 95, 0, 0, 0]);
    assert_eq!(encode_from_child(&FromChild::Saved { ok: true }).unwrap(), vec![2, 0, 0, 0, 71, 1]);
    let mut ask = vec![37, 0, 0, 0, 68, 0x01, 0x02, 0x03, 0x04];
    ask.extend_from_slice(&[7; 32]);
    assert_eq!(encode_from_child(&FromChild::GateAsk { peer: PEER, persona: [7; 32] }).unwrap(), ask);
}

#[test]
fn every_frame_to_the_server_decodes_as_it_was_sent() {
    let messages = vec![
        hello(),
        ToChild::Joined { peer: PEER, binding: [3; 32] },
        ToChild::Reliable { peer: PEER, bytes: vec![1, 2, 3] },
        ToChild::Reliable { peer: PEER, bytes: Vec::new() },
        ToChild::Lossy { peer: 0, bytes: vec![0xff; 70] },
        ToChild::Left { peer: u32::MAX },
        ToChild::GateAnswer { peer: PEER, ok: true, reason: String::new() },
        ToChild::GateAnswer { peer: PEER, ok: false, reason: "No entry, \u{1f6ab}".into() },
        ToChild::Shutdown,
    ];
    let bytes = stream(&messages);
    let mut input = &bytes[..];
    let mut got = Vec::new();
    while let Some(body) = read_frame(&mut input).unwrap() {
        got.push(decode_to_child(&body).unwrap());
    }
    assert_eq!(got, messages);
}

#[test]
fn a_length_of_zero_or_over_a_mebibyte_is_refused_before_anything_is_read() {
    let over = ((MAX_FRAME + 1) as u32).to_le_bytes();
    assert!(matches!(decode_one(&over), Err(FrameError::BadLength(n)) if n == MAX_FRAME as u64 + 1));
    assert!(matches!(decode_one(&u32::MAX.to_le_bytes()), Err(FrameError::BadLength(_))));
    assert!(matches!(decode_one(&[0, 0, 0, 0, 7]), Err(FrameError::BadLength(0))));
    // Exactly 1 MiB is a frame.
    let mut biggest = (MAX_FRAME as u32).to_le_bytes().to_vec();
    biggest.push(RELIABLE);
    biggest.extend_from_slice(&PEER.to_le_bytes());
    biggest.resize(4 + MAX_FRAME, 0xab);
    let Ok(Some(ToChild::Reliable { peer, bytes })) = decode_one(&biggest) else { panic!("a 1 MiB frame") };
    assert_eq!((peer, bytes.len()), (PEER, MAX_FRAME - 5));
}

#[test]
fn a_fixed_size_frame_that_is_short_or_long_is_refused() {
    let body = |tag: u8, payload: &[u8]| {
        let mut b = vec![tag];
        b.extend_from_slice(payload);
        b
    };
    // Short.
    assert!(matches!(decode_to_child(&body(JOINED, &[0; 35])), Err(FrameError::Short { tag: JOINED })));
    assert!(matches!(decode_to_child(&body(JOINED, &[0; 3])), Err(FrameError::Short { tag: JOINED })));
    assert!(matches!(decode_to_child(&body(LEFT, &[0; 3])), Err(FrameError::Short { tag: LEFT })));
    assert!(matches!(decode_to_child(&body(GATE_ANSWER, &[0; 4])), Err(FrameError::Short { tag: GATE_ANSWER })));
    assert!(matches!(decode_to_child(&body(GATE_ANSWER, &[0; 2])), Err(FrameError::Short { tag: GATE_ANSWER })));
    assert!(matches!(decode_to_child(&body(HELLO, &[1, 0])), Err(FrameError::Short { tag: HELLO })));
    assert!(matches!(decode_to_child(&body(RELIABLE, &[0; 3])), Err(FrameError::Short { tag: RELIABLE })));
    // Long.
    assert!(matches!(decode_to_child(&body(JOINED, &[0; 37])), Err(FrameError::Trailing { tag: JOINED })));
    assert!(matches!(decode_to_child(&body(LEFT, &[0; 5])), Err(FrameError::Trailing { tag: LEFT })));
    assert!(matches!(decode_to_child(&body(HELLO, &[1, 0, 8, 0])), Err(FrameError::Trailing { tag: HELLO })));
    assert!(matches!(decode_to_child(&body(SHUTDOWN, &[0])), Err(FrameError::Trailing { tag: SHUTDOWN })));
}

#[test]
fn an_unknown_tag_a_bad_flag_or_bad_text_is_refused() {
    for tag in [0u8, 8, 63, READY, SEND_RELIABLE, SAVED, 255] {
        assert!(matches!(decode_to_child(&[tag, 0, 0, 0, 0]), Err(FrameError::UnknownTag(t)) if t == tag), "tag {tag}");
    }
    assert!(matches!(decode_to_child(&[GATE_ANSWER, 1, 0, 0, 0, 2]), Err(FrameError::BadFlag { tag: GATE_ANSWER })));
    assert!(matches!(
        decode_to_child(&[GATE_ANSWER, 1, 0, 0, 0, 0, 0xff, 0xfe]),
        Err(FrameError::BadText { tag: GATE_ANSWER })
    ));
}

#[test]
fn a_stream_that_ends_inside_a_frame_is_truncated_and_one_that_ends_between_is_clean() {
    assert!(matches!(decode_one(&[]), Ok(None)));
    assert!(matches!(decode_one(&[5, 0]), Err(FrameError::Truncated)), "inside the length");
    assert!(matches!(decode_one(&[5, 0, 0, 0, LEFT, 1]), Err(FrameError::Truncated)), "inside the body");
    let whole = encode_to_child(&ToChild::Left { peer: 1 });
    for cut in 1..whole.len() {
        assert!(matches!(decode_one(&whole[..cut]), Err(FrameError::Truncated)), "cut at {cut}");
    }
}

#[test]
fn no_byte_stream_makes_the_decoder_panic() {
    // A small deterministic generator: garbage of every shape, including
    // plausible lengths, through the reader and decoder, as the Keep's stdin.
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..2_000 {
        let len = (next() % 80) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if len >= 5 && next() % 2 == 0 {
            let body = (len - 4) as u32;
            bytes[..4].copy_from_slice(&body.to_le_bytes());
            bytes[4] = (next() % 9) as u8;
        }
        let mut input = &bytes[..];
        while let Ok(Some(body)) = read_frame(&mut input) {
            let _ = decode_to_child(&body);
        }
        // And the reader's whole loop.
        let (controls, _, _, _) = run_reader(&bytes, MAX_INBOUND_BYTES);
        assert!(matches!(controls.last(), Some(Control::Ended | Control::Fatal(_))), "{controls:?}");
    }
}

#[test]
fn a_frame_too_big_for_the_pipe_is_not_encoded() {
    let huge = FromChild::SendReliable { peer: 1, bytes: vec![0; MAX_FRAME] };
    assert!(matches!(encode_from_child(&huge), Err(FrameError::BadLength(_))));
}

// --- the reader ---

#[test]
fn hello_must_come_first_once_and_in_this_protocol() {
    let (controls, joined, _, _) = run_reader(&stream(&[ToChild::Joined { peer: 1, binding: [0; 32] }]), 1 << 20);
    assert!(matches!(&controls[..], [Control::Fatal(why)] if why.contains("first frame must be Hello")), "{controls:?}");
    assert!(joined.is_empty());
    let (controls, _, _, _) = run_reader(&stream(&[ToChild::Hello { proto: 2, max_players: 8 }]), 1 << 20);
    assert!(matches!(&controls[..], [Control::Fatal(why)] if why.contains("protocol 2")), "{controls:?}");
    let (controls, _, _, _) = run_reader(&stream(&[hello(), hello()]), 1 << 20);
    assert!(matches!(&controls[..], [Control::Hello { max_players: 8 }, Control::Fatal(why)] if why.contains("second Hello")));
    let (controls, _, _, _) = run_reader(&stream(&[hello(), ToChild::Shutdown]), 1 << 20);
    assert_eq!(controls, vec![Control::Hello { max_players: 8 }, Control::Shutdown, Control::Ended]);
}

#[test]
fn a_joined_peer_becomes_a_transport_with_exactly_the_binding_the_keep_passed() {
    let binding = [0x5a; 32];
    let input = stream(&[
        hello(),
        ToChild::Joined { peer: 7, binding },
        ToChild::Reliable { peer: 7, bytes: vec![1, 2] },
        ToChild::Reliable { peer: 7, bytes: Vec::new() },
        ToChild::Lossy { peer: 7, bytes: vec![9] },
        ToChild::Reliable { peer: 7, bytes: vec![3] },
    ]);
    let (controls, joined, _, _) = run_reader(&input, 1 << 20);
    assert_eq!(joined.len(), 1);
    let t = &joined[0];
    assert_eq!(t.channel_binding(), Some(binding));
    assert!(!t.is_websocket(), "its origin is the bound one, never a WebSocket host");
    // The empty message and the lossy one are not the game's.
    assert_eq!(t.try_recv_from_client(), Some(vec![1, 2]));
    assert_eq!(t.try_recv_from_client(), Some(vec![3]));
    // The pipe ended: the peer leaves with it, after everything it sent.
    let leave = protocol::serialize_packet(PacketType::Disconnect, &());
    assert_eq!(t.try_recv_from_client(), Some(leave));
    assert!(t.is_closed());
    assert_eq!(controls.last(), Some(&Control::Ended));
}

#[test]
fn left_closes_the_peer_after_its_packets_and_unknown_peers_are_ignored() {
    let input = stream(&[
        hello(),
        ToChild::Left { peer: 3 },
        ToChild::Reliable { peer: 3, bytes: vec![1] },
        ToChild::GateAnswer { peer: 3, ok: true, reason: String::new() },
        ToChild::Joined { peer: 1, binding: [1; 32] },
        ToChild::Reliable { peer: 1, bytes: vec![4] },
        ToChild::Left { peer: 1 },
        ToChild::Left { peer: 1 },
        ToChild::Reliable { peer: 1, bytes: vec![5] },
    ]);
    let (controls, joined, capture, out) = run_reader(&input, 1 << 20);
    assert_eq!(controls, vec![Control::Hello { max_players: 8 }, Control::Ended]);
    let t = &joined[0];
    assert!(t.is_closed());
    assert_eq!(t.try_recv_from_client(), Some(vec![4]));
    assert_eq!(t.try_recv_from_client(), Some(protocol::serialize_packet(PacketType::Disconnect, &())));
    assert_eq!(t.try_recv_from_client(), None, "nothing after the leave");
    drop(joined);
    assert!(frames_of(&capture, &out).is_empty(), "a peer the Keep closed is not disconnected again");
}

#[test]
fn a_joiner_past_hello_max_players_is_disconnected_and_a_second_joined_for_a_live_peer_is_fatal() {
    let input = stream(&[
        ToChild::Hello { proto: PROTO, max_players: 2 },
        ToChild::Joined { peer: 1, binding: [1; 32] },
        ToChild::Joined { peer: 2, binding: [2; 32] },
        ToChild::Joined { peer: 3, binding: [3; 32] },
    ]);
    let (_, joined, capture, out) = run_reader(&input, 1 << 20);
    assert_eq!(joined.iter().map(|t| t.channel_binding()).collect::<Vec<_>>(), vec![Some([1; 32]), Some([2; 32])]);
    assert_eq!(frames_of(&capture, &out), vec![FromChild::Disconnect { peer: 3, reason: WORLD_FULL.to_string() }]);

    // The same id joined again while it is live: a protocol error. The second
    // connection is never given a transport, its packets never reach the
    // first's, and the first leaves with the pipe.
    let input = stream(&[
        ToChild::Hello { proto: PROTO, max_players: 2 },
        ToChild::Joined { peer: 1, binding: [1; 32] },
        ToChild::Reliable { peer: 1, bytes: vec![1] },
        ToChild::Joined { peer: 1, binding: [9; 32] },
        ToChild::Reliable { peer: 1, bytes: vec![2] },
    ]);
    let (controls, joined, _, _) = run_reader(&input, 1 << 20);
    assert!(matches!(controls.last(), Some(Control::Fatal(why)) if why.contains("joined twice")), "{controls:?}");
    assert_eq!(joined.len(), 1, "no transport for the second");
    assert_eq!(joined[0].channel_binding(), Some([1; 32]));
    assert_eq!(joined[0].try_recv_from_client(), Some(vec![1]));
    assert_eq!(joined[0].try_recv_from_client(), Some(protocol::serialize_packet(PacketType::Disconnect, &())));
    assert_eq!(joined[0].try_recv_from_client(), None, "nothing of the second connection arrived");
    assert!(joined[0].is_closed());

    // After a Left, the id is free again.
    let input = stream(&[
        hello(),
        ToChild::Joined { peer: 1, binding: [1; 32] },
        ToChild::Left { peer: 1 },
        ToChild::Joined { peer: 1, binding: [9; 32] },
    ]);
    let (controls, joined, _, _) = run_reader(&input, 1 << 20);
    assert_eq!(controls.last(), Some(&Control::Ended));
    assert_eq!(joined.iter().map(|t| t.channel_binding()).collect::<Vec<_>>(), vec![Some([1; 32]), Some([9; 32])]);
}

#[test]
fn a_hello_with_no_seats_is_a_protocol_error() {
    let (controls, joined, _, _) = run_reader(
        &stream(&[ToChild::Hello { proto: PROTO, max_players: 0 }, ToChild::Joined { peer: 1, binding: [1; 32] }]),
        1 << 20,
    );
    assert!(matches!(&controls[..], [Control::Fatal(why)] if why.contains("max_players is 0")), "{controls:?}");
    assert!(joined.is_empty());
}

#[test]
fn a_malformed_frame_mid_stream_is_fatal_and_every_peer_leaves() {
    let mut input = stream(&[hello(), ToChild::Joined { peer: 1, binding: [1; 32] }]);
    input.extend_from_slice(&[2, 0, 0, 0, LEFT, 0]);
    input.extend_from_slice(&encode_to_child(&ToChild::Joined { peer: 2, binding: [2; 32] }));
    let (controls, joined, _, _) = run_reader(&input, 1 << 20);
    assert!(matches!(controls.last(), Some(Control::Fatal(why)) if why.contains("too short")), "{controls:?}");
    assert_eq!(joined.len(), 1, "nothing after the bad frame is acted on");
    assert!(joined[0].is_closed());
}

#[test]
fn a_peer_past_its_inbound_bound_has_the_rest_dropped_until_the_server_reads() {
    let cap = 3 * charge(10);
    let mut messages = vec![hello(), ToChild::Joined { peer: 1, binding: [1; 32] }];
    for n in 0..6u8 {
        messages.push(ToChild::Reliable { peer: 1, bytes: vec![n; 10] });
    }
    let (_, joined, _, _) = run_reader(&stream(&messages), cap);
    let got: Vec<Packet> = std::iter::from_fn(|| joined[0].try_recv_from_client()).collect();
    // Three fit; the fourth crosses the bound and is still handed over (the
    // server's own queue then disconnects the peer); the rest are dropped.
    assert_eq!(got[..4].iter().map(|p| p[0]).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(got.len(), 5, "then the leave: {got:?}");
}

// --- a peer's transport ---

fn one_peer() -> (Box<dyn ServerTransport>, PeerSlot, Capture, PipeOut) {
    let capture = Capture::default();
    let out = spawn_writer(capture.clone()).unwrap();
    let (slot, transport) = peer_pair(7, [4; 32], out.clone());
    (Box::new(transport), slot, capture, out)
}

#[test]
fn a_transport_the_server_lets_go_of_says_disconnect_with_the_reason_the_player_was_given() {
    let (t, _slot, capture, out) = one_peer();
    let reject = protocol::serialize_packet(
        PacketType::JoinReject,
        &protocol::JoinRejectPacket { reason: "This player's pass was cancelled".into() },
    );
    t.send_to_client(&reject);
    drop(t);
    assert_eq!(
        frames_of(&capture, &out),
        vec![
            FromChild::SendReliable { peer: 7, bytes: reject },
            FromChild::Disconnect { peer: 7, reason: "This player's pass was cancelled".into() },
        ]
    );
    // With no reason of its own, the default one.
    let (t, _slot, capture, out) = one_peer();
    drop(t);
    assert_eq!(frames_of(&capture, &out), vec![FromChild::Disconnect { peer: 7, reason: LET_GO.into() }]);
}

#[test]
fn after_left_the_transport_sends_nothing_and_says_no_disconnect() {
    let (t, slot, capture, out) = one_peer();
    slot.finish();
    assert!(t.is_closed());
    t.send_to_client(&[1, 2, 3]);
    drop(t);
    assert!(frames_of(&capture, &out).is_empty());
}

#[test]
fn the_gate_is_asked_once_and_answered_once() {
    let (t, slot, capture, out) = one_peer();
    assert_eq!(t.gate_verdict(), None, "not asked");
    slot.answer(7, true, String::new());
    assert_eq!(t.gate_verdict(), None, "an answer nobody asked for is ignored");
    assert!(t.ask_gate(&[8; 32]));
    assert!(!t.ask_gate(&[8; 32]), "asked already");
    assert_eq!(t.gate_verdict(), None, "no answer yet");
    slot.answer(7, false, "Blocked".into());
    slot.answer(7, true, String::new());
    assert_eq!(t.gate_verdict(), Some(Err("Blocked".to_string())), "the first answer");
    assert_eq!(t.gate_verdict(), None, "given once");
    assert_eq!(frames_of(&capture, &out), vec![FromChild::GateAsk { peer: 7, persona: [8; 32] }]);
}

#[test]
fn a_seated_peer_is_unseated_when_it_goes() {
    let (t, slot, capture, out) = one_peer();
    t.seated(&[8; 32], "Ann-abcd");
    t.seated(&[8; 32], "Ann-abcd");
    slot.finish();
    drop(t);
    assert_eq!(
        frames_of(&capture, &out),
        vec![FromChild::Seated { peer: 7, persona: [8; 32], name: "Ann-abcd".into() }, FromChild::Unseated { peer: 7 }]
    );
}

#[test]
fn a_packet_over_the_frame_cap_closes_the_link_instead_of_vanishing() {
    let (t, _slot, capture, out) = one_peer();
    t.send_to_client(&vec![0; MAX_FRAME_LEN + 1]);
    t.send_to_client(&[1]);
    drop(t);
    let frames = frames_of(&capture, &out);
    assert_eq!(frames.len(), 1, "one Disconnect, nothing sent after it: {frames:?}");
    assert!(matches!(&frames[0], FromChild::Disconnect { peer: 7, .. }));
    // The biggest packet the game sends goes out whole.
    let (t, slot, capture, out) = one_peer();
    t.send_to_client(&vec![1; MAX_FRAME_LEN]);
    slot.finish();
    drop(t);
    assert!(matches!(&frames_of(&capture, &out)[..], [FromChild::SendReliable { bytes, .. }] if bytes.len() == MAX_FRAME_LEN));
}

#[test]
fn a_keep_that_stops_reading_bounds_the_queue_then_reports_a_stall_and_recovers_when_it_reads() {
    let (open, gate) = mpsc::channel::<()>();
    let into = Capture::default();
    let out = spawn_writer_bounded(Blocked { gate, into: into.clone() }, 1_000, Duration::from_millis(150)).unwrap();
    let frame = FromChild::SendReliable { peer: 1, bytes: vec![7; 400] };
    // A frame is 409 bytes: the queue takes frames until it holds the cap, so
    // three go in (0, 409, 818 bytes held), and every later one is dropped.
    for _ in 0..200 {
        out.send(&frame);
    }
    assert_eq!(out.queued_bytes(), 3 * 409, "memory is bounded, not 200 frames");
    // Over the cap for the stall time, with nothing written: stalled.
    let end = Instant::now() + Duration::from_secs(10);
    while !out.stalled() {
        assert!(Instant::now() < end, "never reported the stall");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(into.0.lock().unwrap().is_empty(), "nothing was written while blocked");
    // The Keep reads again: the three queued frames are written, the queue is
    // under the cap, the stall is over, and new frames go out.
    drop(open);
    assert!(out.flush(Duration::from_secs(10)));
    assert!(!out.stalled());
    assert_eq!(out.queued_bytes(), 0);
    out.send(&FromChild::Saved { ok: true });
    assert!(out.flush(Duration::from_secs(10)));
    let written = decode_all(&into.0.lock().unwrap());
    assert_eq!(written.len(), 4, "the three queued, then the new one: {written:?}");
    assert_eq!(written[3], FromChild::Saved { ok: true });
}

#[test]
fn a_queue_that_keeps_draining_is_never_reported_as_stalled() {
    // Under the cap nothing is dropped and there is no stall, however long.
    let capture = Capture::default();
    let out = spawn_writer_bounded(capture, 1_000_000, Duration::ZERO).unwrap();
    for _ in 0..100 {
        out.send(&FromChild::SendReliable { peer: 1, bytes: vec![1; 400] });
        assert!(!out.stalled());
    }
    assert!(out.flush(Duration::from_secs(10)));
    assert_eq!(out.queued_bytes(), 0);
}

#[test]
fn a_closed_stdout_is_survived_and_reported_as_flushed() {
    let out = spawn_writer(Closed).unwrap();
    out.send(&FromChild::Ready { protocol: 95 });
    out.send(&FromChild::Saved { ok: true });
    assert!(out.flush(Duration::from_secs(5)), "the writer lives on, discarding");
}
