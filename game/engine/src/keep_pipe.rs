//! `--keep-world`: the pipe between this server and its parent, a home Keep
//! (Ingle). The Keep owns the room on its one Fray listener (one forwarded UDP
//! port) and carries each peer's traffic to this process over its stdin and
//! stdout. Native only; nothing here opens a socket.
//!
//! # Frames
//!
//! One frame is a `u32` little-endian length of the rest, then a `u8` tag, then
//! the payload. The length counts the tag and the payload, so it is at least 1
//! and at most [`MAX_FRAME`] (1 MiB). Every integer is little-endian. Peer ids are
//! Fray's `u32`; a persona and a connection binding are 32 raw bytes; text is
//! UTF-8 and runs to the end of the frame.
//!
//! The Keep to this server (stdin):
//!
//! | tag | frame        | payload                                          |
//! |-----|--------------|--------------------------------------------------|
//! | 1   | `Hello`      | `proto: u16` (1), `max_players: u8`              |
//! | 2   | `Joined`     | `peer: u32`, `binding: [u8; 32]`                 |
//! | 3   | `Reliable`   | `peer: u32`, `bytes` (to the end)                |
//! | 4   | `Lossy`      | `peer: u32`, `bytes` (to the end)                |
//! | 5   | `Left`       | `peer: u32`                                      |
//! | 6   | `GateAnswer` | `peer: u32`, `ok: u8` (0 or 1), `reason` (UTF-8) |
//! | 7   | `Shutdown`   | none                                             |
//!
//! This server to the Keep (stdout):
//!
//! | tag | frame          | payload                                          |
//! |-----|----------------|--------------------------------------------------|
//! | 64  | `Ready`        | `protocol: u32` (the engine protocol)            |
//! | 65  | `SendReliable` | `peer: u32`, `bytes` (to the end)                |
//! | 66  | `SendLossy`    | `peer: u32`, `bytes` (never sent: the game is reliable only) |
//! | 67  | `Disconnect`   | `peer: u32`, `reason` (UTF-8)                    |
//! | 68  | `GateAsk`      | `peer: u32`, `persona: [u8; 32]`                 |
//! | 69  | `Seated`       | `peer: u32`, `persona: [u8; 32]`, `name` (UTF-8) |
//! | 70  | `Unseated`     | `peer: u32`                                      |
//! | 71  | `Saved`        | `ok: u8` (0 or 1)                                |
//!
//! The decoder is strict: an unknown tag, a payload that is short, or longer
//! than a frame of fixed size, text that is not UTF-8, a flag byte other than 0
//! or 1, a length of 0 or above 1 MiB, or a stream that ends inside a frame is a
//! fatal protocol error, never a guess. Nothing on the pipe can make it panic.
//!
//! # Threads
//!
//! [`spawn_reader`] reads stdin on its own thread and routes each frame at
//! once (a peer's messages to its transport's queue, the rest to the server's
//! loop as a [`Control`]), so the Keep is never kept waiting by a tick or a
//! save. [`spawn_writer_bounded`] owns stdout: every frame from every thread goes
//! through its one queue and is written whole, one at a time, in the order
//! it was sent.
//!
//! # Peers
//!
//! `Joined { peer, binding }` becomes one [`PipeServerTransport`], handed to
//! the server on its accept channel as Fray's pump hands its peers over
//! (`fray_transport`). Its `channel_binding()` is exactly that binding, so the
//! server's own join check (`signet::join_origin` over the binding, then the
//! signed Signet join) runs unchanged on it. `Left` delivers the synthetic
//! `Disconnect` packet and marks the transport closed, as Fray's `PeerLeft`
//! does. When the server lets the peer go, the transport's drop says
//! `Disconnect` with the reason the server gave the player (its
//! `JoinReject`), and `Unseated` if the peer was seated.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::network::MAX_FRAME_LEN;
use crate::protocol;
use crate::transport::{Packet, ServerTransport, INBOUND_ENTRY_OVERHEAD, MAX_INBOUND_BYTES};

/// The most a frame's length field may say: 1 MiB.
pub const MAX_FRAME: usize = 1 << 20;
/// The pipe protocol this server speaks (the Keep says its own in `Hello`).
pub const PROTO: u16 = 1;

const HELLO: u8 = 1;
const JOINED: u8 = 2;
const RELIABLE: u8 = 3;
const LOSSY: u8 = 4;
const LEFT: u8 = 5;
const GATE_ANSWER: u8 = 6;
const SHUTDOWN: u8 = 7;

const READY: u8 = 64;
const SEND_RELIABLE: u8 = 65;
const DISCONNECT: u8 = 67;
const GATE_ASK: u8 = 68;
const SEATED: u8 = 69;
const UNSEATED: u8 = 70;
const SAVED: u8 = 71;

/// Told to the Keep with a peer's `Disconnect` when the server let the peer go
/// without a reason of its own (a slot freed after its connection closed).
const LET_GO: &str = "The world let this player go.";
/// Told to a joiner past `Hello`'s `max_players`.
pub const WORLD_FULL: &str = "This world is full.";

/// A frame the Keep sends to this server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToChild {
    Hello { proto: u16, max_players: u8 },
    Joined { peer: u32, binding: [u8; 32] },
    Reliable { peer: u32, bytes: Vec<u8> },
    Lossy { peer: u32, bytes: Vec<u8> },
    Left { peer: u32 },
    GateAnswer { peer: u32, ok: bool, reason: String },
    Shutdown,
}

impl ToChild {
    fn name(&self) -> &'static str {
        match self {
            ToChild::Hello { .. } => "Hello",
            ToChild::Joined { .. } => "Joined",
            ToChild::Reliable { .. } => "Reliable",
            ToChild::Lossy { .. } => "Lossy",
            ToChild::Left { .. } => "Left",
            ToChild::GateAnswer { .. } => "GateAnswer",
            ToChild::Shutdown => "Shutdown",
        }
    }
}

/// A frame this server sends to the Keep. (`SendLossy`, tag 66, is never sent.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromChild {
    Ready { protocol: u32 },
    SendReliable { peer: u32, bytes: Vec<u8> },
    Disconnect { peer: u32, reason: String },
    GateAsk { peer: u32, persona: [u8; 32] },
    Seated { peer: u32, persona: [u8; 32], name: String },
    Unseated { peer: u32 },
    Saved { ok: bool },
}

/// Why a frame was refused.
#[derive(Debug)]
pub enum FrameError {
    /// The tag is not one of this direction's.
    UnknownTag(u8),
    /// The payload is shorter than the frame needs.
    Short { tag: u8 },
    /// The payload has bytes after the end of a frame of fixed size.
    Trailing { tag: u8 },
    /// Text that is not UTF-8.
    BadText { tag: u8 },
    /// A flag byte that is neither 0 nor 1.
    BadFlag { tag: u8 },
    /// The length field is 0 (no tag) or above [`MAX_FRAME`].
    BadLength(u64),
    /// The stream ended inside a frame.
    Truncated,
    /// The pipe itself failed.
    Io(io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::UnknownTag(tag) => write!(f, "unknown frame tag {tag}"),
            FrameError::Short { tag } => write!(f, "frame {tag} is too short"),
            FrameError::Trailing { tag } => write!(f, "frame {tag} has bytes left over"),
            FrameError::BadText { tag } => write!(f, "frame {tag} carries text that is not UTF-8"),
            FrameError::BadFlag { tag } => write!(f, "frame {tag} has a flag that is not 0 or 1"),
            FrameError::BadLength(len) => write!(f, "frame length {len} is outside 1..={MAX_FRAME}"),
            FrameError::Truncated => write!(f, "the pipe ended inside a frame"),
            FrameError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// A frame whose tag and payload parts are put together, length first.
fn frame(tag: u8, payload: &[&[u8]]) -> Result<Vec<u8>, FrameError> {
    let rest = 1 + payload.iter().map(|p| p.len()).sum::<usize>();
    if rest > MAX_FRAME {
        return Err(FrameError::BadLength(rest as u64));
    }
    let mut out = Vec::with_capacity(4 + rest);
    out.extend_from_slice(&(rest as u32).to_le_bytes());
    out.push(tag);
    for part in payload {
        out.extend_from_slice(part);
    }
    Ok(out)
}

/// Encode a frame for the Keep: the length, the tag and the payload.
pub fn encode_from_child(message: &FromChild) -> Result<Vec<u8>, FrameError> {
    match message {
        FromChild::Ready { protocol } => frame(READY, &[&protocol.to_le_bytes()]),
        FromChild::SendReliable { peer, bytes } => frame(SEND_RELIABLE, &[&peer.to_le_bytes(), bytes]),
        FromChild::Disconnect { peer, reason } => frame(DISCONNECT, &[&peer.to_le_bytes(), reason.as_bytes()]),
        FromChild::GateAsk { peer, persona } => frame(GATE_ASK, &[&peer.to_le_bytes(), persona]),
        FromChild::Seated { peer, persona, name } => {
            frame(SEATED, &[&peer.to_le_bytes(), persona, name.as_bytes()])
        }
        FromChild::Unseated { peer } => frame(UNSEATED, &[&peer.to_le_bytes()]),
        FromChild::Saved { ok } => frame(SAVED, &[&[u8::from(*ok)]]),
    }
}

/// A payload read front to back.
struct Cursor<'a> {
    tag: u8,
    rest: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], FrameError> {
        let Some((head, tail)) = self.rest.split_first_chunk::<N>() else {
            return Err(FrameError::Short { tag: self.tag });
        };
        self.rest = tail;
        Ok(*head)
    }
    fn u8(&mut self) -> Result<u8, FrameError> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, FrameError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, FrameError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn key(&mut self) -> Result<[u8; 32], FrameError> {
        self.take()
    }
    fn flag(&mut self) -> Result<bool, FrameError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(FrameError::BadFlag { tag: self.tag }),
        }
    }
    /// Everything left, as bytes.
    fn bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.rest).to_vec()
    }
    /// Everything left, as text.
    fn text(&mut self) -> Result<String, FrameError> {
        String::from_utf8(self.bytes()).map_err(|_| FrameError::BadText { tag: self.tag })
    }
    /// The end of a frame of fixed size (or of a fixed header before text).
    fn end(&self) -> Result<(), FrameError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(FrameError::Trailing { tag: self.tag })
        }
    }
}

/// Decode the rest of a frame from the Keep (the tag and the payload, as
/// [`read_frame`] returns it).
pub fn decode_to_child(body: &[u8]) -> Result<ToChild, FrameError> {
    let (&tag, payload) = body.split_first().ok_or(FrameError::BadLength(0))?;
    let mut c = Cursor { tag, rest: payload };
    let message = match tag {
        HELLO => ToChild::Hello { proto: c.u16()?, max_players: c.u8()? },
        JOINED => ToChild::Joined { peer: c.u32()?, binding: c.key()? },
        RELIABLE => ToChild::Reliable { peer: c.u32()?, bytes: c.bytes() },
        LOSSY => ToChild::Lossy { peer: c.u32()?, bytes: c.bytes() },
        LEFT => ToChild::Left { peer: c.u32()? },
        GATE_ANSWER => ToChild::GateAnswer { peer: c.u32()?, ok: c.flag()?, reason: c.text()? },
        SHUTDOWN => ToChild::Shutdown,
        other => return Err(FrameError::UnknownTag(other)),
    };
    c.end()?;
    Ok(message)
}

/// Read one frame's tag and payload. `Ok(None)` is the stream ending cleanly
/// between frames; ending inside one is [`FrameError::Truncated`]. The length
/// is checked before anything is allocated.
pub fn read_frame(from: &mut impl Read) -> Result<Option<Vec<u8>>, FrameError> {
    let mut length = [0u8; 4];
    let mut got = 0;
    while got < length.len() {
        match from.read(&mut length[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(FrameError::BadLength(length as u64));
    }
    let mut body = vec![0u8; length];
    from.read_exact(&mut body).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => FrameError::Truncated,
        _ => FrameError::Io(e),
    })?;
    Ok(Some(body))
}

// --- stdout: the writer ---

enum Outgoing {
    Frame(Vec<u8>),
    Flush(mpsc::Sender<()>),
}

/// What may wait in the writer's queue: 32 MiB, as the Keep's own bound on what
/// it holds for this child. A frame that would take the queue past this is not
/// queued, and the server stops ([`PipeOut::overflowed`]): the Keep has stopped
/// reading, and no frame is ever dropped while the server carries on.
pub const OUT_QUEUE_CAP: usize = 32 << 20;

/// What the senders and the writer share: the bytes waiting, and whether the
/// cap was crossed.
struct OutQueue {
    cap: usize,
    /// Bytes of the frames queued and not yet written.
    queued: AtomicUsize,
    /// A frame would have taken the queue past `cap`. Never reset: the server
    /// is stopping.
    overflowed: AtomicBool,
}

impl OutQueue {
    /// Make room for a frame of `len` bytes: `false` if it would take the queue
    /// past the cap, which is then latched as overflowed (the frame, and every
    /// one after it, is not queued; the server saves and exits).
    fn admit(&self, len: usize) -> bool {
        if self.overflowed.load(Ordering::Relaxed) {
            return false;
        }
        // One step: no sender can slip a frame in between the check and the add.
        let taken = self
            .queued
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |queued| {
                queued.checked_add(len).filter(|total| *total <= self.cap)
            })
            .is_ok();
        if !taken && !self.overflowed.swap(true, Ordering::Relaxed) {
            log::error!(
                "keep pipe: the Keep is not reading ({} bytes waiting, cap {}); the server will save and stop",
                self.queued.load(Ordering::Relaxed),
                self.cap
            );
        }
        taken
    }

    /// The writer is done with a frame of `len` bytes (written, or discarded
    /// because stdout is gone).
    fn written(&self, len: usize) {
        self.queued.fetch_sub(len, Ordering::Relaxed);
    }
}

/// The one way to stdout. A frame sent here from any thread is queued, in the
/// order of the `send` calls, and the writer thread writes it whole.
#[derive(Clone)]
pub struct PipeOut {
    tx: mpsc::Sender<Outgoing>,
    queue: Arc<OutQueue>,
}

impl PipeOut {
    /// Queue `message` for the Keep. Never blocks. A message that would take
    /// the queue past [`OUT_QUEUE_CAP`] is not queued, and [`Self::overflowed`]
    /// is true from then on.
    pub fn send(&self, message: &FromChild) {
        match encode_from_child(message) {
            Ok(frame) => {
                let len = frame.len();
                if !self.queue.admit(len) {
                    return;
                }
                // The writer is gone only when the process is ending.
                if self.tx.send(Outgoing::Frame(frame)).is_err() {
                    self.queue.queued.fetch_sub(len, Ordering::Relaxed);
                }
            }
            Err(e) => log::error!("keep pipe: a frame was not sent: {e}"),
        }
    }

    /// The Keep is not reading: a frame would have taken the queue past its
    /// cap. The loop then saves the world and exits non-zero, so the Keep can
    /// start it again.
    pub fn overflowed(&self) -> bool {
        self.queue.overflowed.load(Ordering::Relaxed)
    }

    /// Bytes of frames queued and not yet written.
    #[cfg(test)]
    pub(crate) fn queued_bytes(&self) -> usize {
        self.queue.queued.load(Ordering::Relaxed)
    }

    /// Wait, up to `wait`, until everything queued before this call has been
    /// written (or found that stdout is gone). `false` if the wait ran out.
    pub fn flush(&self, wait: Duration) -> bool {
        let (done, flushed) = mpsc::channel();
        if self.tx.send(Outgoing::Flush(done)).is_err() {
            return false;
        }
        flushed.recv_timeout(wait).is_ok()
    }
}

/// Start the writer thread on `out` (a test's buffer), with the real queue
/// bounds. The server itself starts it through [`spawn_writer_bounded`].
#[cfg(test)]
pub fn spawn_writer(out: impl Write + Send + 'static) -> io::Result<PipeOut> {
    spawn_writer_bounded(out, OUT_QUEUE_CAP)
}

/// [`spawn_writer`] with its queue's cap given (a small one in tests).
pub fn spawn_writer_bounded(out: impl Write + Send + 'static, cap: usize) -> io::Result<PipeOut> {
    let (tx, rx) = mpsc::channel();
    let queue = Arc::new(OutQueue { cap, queued: AtomicUsize::new(0), overflowed: AtomicBool::new(false) });
    let writer_queue = queue.clone();
    thread::Builder::new().name("keep-pipe-out".into()).spawn(move || write_loop(out, rx, &writer_queue))?;
    Ok(PipeOut { tx, queue })
}

fn write_loop(mut out: impl Write, rx: mpsc::Receiver<Outgoing>, queue: &OutQueue) {
    let mut broken = false;
    for item in rx {
        match item {
            Outgoing::Frame(frame) => {
                if !broken && let Err(e) = out.write_all(&frame).and_then(|()| out.flush()) {
                    // The Keep is gone (a closed stdout is `BrokenPipe`: Rust
                    // ignores SIGPIPE). Nothing more can reach it; the server
                    // still saves on its way out.
                    log::warn!("keep pipe: stdout is closed ({e}); nothing more is sent");
                    broken = true;
                }
                queue.written(frame.len());
            }
            Outgoing::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

// --- one peer ---

/// Where a peer's join stands with the Keep's gate.
#[derive(Debug)]
enum GateState {
    NotAsked,
    Asked,
    Answered(Result<(), String>),
    Taken,
}

/// What the reader and a peer's transport share.
struct PeerShared {
    /// The Keep said `Left` for this peer (or the pipe ended). Set with
    /// `Release` after the synthetic `Disconnect` is queued, read with
    /// `Acquire`: closed implies every packet the peer sent is readable.
    left: AtomicBool,
    /// What the waiting packets are charged, as `InboundQueue` charges them.
    charged: AtomicUsize,
    gate: Mutex<GateState>,
}

/// The reader's side of one peer.
struct PeerSlot {
    tx: mpsc::Sender<Packet>,
    shared: Arc<PeerShared>,
    /// Over the inbound bound, for good: the message that crossed it was still
    /// handed over, so the server's own `InboundQueue::push` overflows and
    /// releases the peer, and everything the peer sends after that is dropped
    /// here. It is never reset: a released peer has no later use for it.
    over: bool,
}

/// What a waiting packet is charged, as `InboundQueue` charges it.
fn charge(len: usize) -> usize {
    len + INBOUND_ENTRY_OVERHEAD
}

impl PeerSlot {
    /// Hand one message from the peer to its transport. As Fray's pump does
    /// (`fray_transport::PeerSlot::deliver`): the message that takes the charge
    /// past `cap` is still handed over, so the server's own queue overflows
    /// and disconnects the peer with the game's reason; later ones are dropped.
    fn deliver(&mut self, peer: u32, bytes: Vec<u8>, cap: usize) {
        if self.over || bytes.is_empty() {
            return;
        }
        let cost = charge(bytes.len());
        let crossing = self.shared.charged.load(Ordering::Relaxed) + cost > cap;
        self.shared.charged.fetch_add(cost, Ordering::Relaxed);
        if self.tx.send(bytes).is_err() {
            // The server let go of this transport already.
            self.shared.charged.fetch_sub(cost, Ordering::Relaxed);
            return;
        }
        if crossing {
            log::warn!("keep pipe: peer {peer} has too many packets waiting; dropping the rest");
            self.over = true;
        }
    }

    /// The Keep's answer about this peer. Taken only if the server asked and
    /// has not been answered yet; anything else is ignored.
    fn answer(&self, peer: u32, ok: bool, reason: String) {
        let mut gate = self.shared.gate.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*gate, GateState::Asked) {
            *gate = GateState::Answered(if ok { Ok(()) } else { Err(reason) });
        } else {
            log::warn!("keep pipe: a gate answer for peer {peer}, who was not waiting for one, is ignored");
        }
    }

    /// The peer is gone: deliver the synthetic `Disconnect` the server reads a
    /// leaver by, then mark the transport closed.
    fn finish(self) {
        let packet = protocol::serialize_packet(protocol::PacketType::Disconnect, &());
        self.shared.charged.fetch_add(charge(packet.len()), Ordering::Relaxed);
        let _ = self.tx.send(packet);
        self.shared.left.store(true, Ordering::Release);
    }
}

/// A new peer's transport and the reader's side of it.
fn peer_pair(peer: u32, binding: [u8; 32], out: PipeOut) -> (PeerSlot, PipeServerTransport) {
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(PeerShared {
        left: AtomicBool::new(false),
        charged: AtomicUsize::new(0),
        gate: Mutex::new(GateState::NotAsked),
    });
    let transport = PipeServerTransport {
        peer,
        binding,
        rx,
        shared: shared.clone(),
        out,
        reason: Mutex::new(None),
        seated: AtomicBool::new(false),
        cut: AtomicBool::new(false),
    };
    (PeerSlot { tx, shared, over: false }, transport)
}

/// One peer of the Keep's room, as the server sees it.
pub struct PipeServerTransport {
    peer: u32,
    /// The room's app binding for this peer's connection, as the Keep passed it.
    binding: [u8; 32],
    rx: mpsc::Receiver<Packet>,
    shared: Arc<PeerShared>,
    out: PipeOut,
    /// The reason the server last refused this peer with (its `JoinReject`),
    /// told to the Keep with the `Disconnect`.
    reason: Mutex<Option<String>>,
    /// Seated: `Unseated` is said when the server lets go of it.
    seated: AtomicBool,
    /// This side has said `Disconnect` already (an over-long packet).
    cut: AtomicBool,
}

impl PipeServerTransport {
    /// Ask the Keep to close the link, once.
    fn cut_off(&self, reason: &str) {
        if !self.cut.swap(true, Ordering::Relaxed) {
            self.out.send(&FromChild::Disconnect { peer: self.peer, reason: reason.to_string() });
        }
    }
}

impl ServerTransport for PipeServerTransport {
    fn send_to_client(&self, data: &[u8]) {
        if data.is_empty() || self.cut.load(Ordering::Relaxed) || self.shared.left.load(Ordering::Relaxed) {
            return;
        }
        if data.len() > MAX_FRAME_LEN {
            // As every other transport: close rather than drop, so the peer
            // sees the end instead of waiting for a packet that never comes.
            log::warn!("keep pipe: a {}-byte packet is over the frame cap; closing peer {}", data.len(), self.peer);
            self.cut_off("A packet was too large to send.");
            return;
        }
        if let Some((protocol::PacketType::JoinReject, payload)) = protocol::deserialize_header(data)
            && let Ok(reject) = protocol::safe_deserialize::<protocol::JoinRejectPacket>(payload)
        {
            *self.reason.lock().unwrap_or_else(|e| e.into_inner()) = Some(reject.reason);
        }
        self.out.send(&FromChild::SendReliable { peer: self.peer, bytes: data.to_vec() });
    }

    fn try_recv_from_client(&self) -> Option<Packet> {
        match self.rx.try_recv() {
            Ok(packet) => {
                self.shared.charged.fetch_sub(charge(packet.len()), Ordering::Relaxed);
                Some(packet)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                // The reader ended without a `Left` for this peer.
                self.shared.left.store(true, Ordering::Release);
                None
            }
        }
    }

    fn channel_binding(&self) -> Option<[u8; 32]> {
        Some(self.binding)
    }

    fn is_closed(&self) -> bool {
        self.shared.left.load(Ordering::Acquire)
    }

    fn ask_gate(&self, persona: &[u8; 32]) -> bool {
        let mut gate = self.shared.gate.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*gate, GateState::NotAsked) {
            return false;
        }
        *gate = GateState::Asked;
        drop(gate);
        self.out.send(&FromChild::GateAsk { peer: self.peer, persona: *persona });
        true
    }

    fn gate_verdict(&self) -> Option<Result<(), String>> {
        let mut gate = self.shared.gate.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*gate, GateState::Answered(_)) {
            return None;
        }
        match std::mem::replace(&mut *gate, GateState::Taken) {
            GateState::Answered(verdict) => Some(verdict),
            _ => None,
        }
    }

    fn seated(&self, persona: &[u8; 32], name: &str) {
        if !self.seated.swap(true, Ordering::Relaxed) {
            self.out.send(&FromChild::Seated { peer: self.peer, persona: *persona, name: name.to_string() });
        }
    }
}

impl Drop for PipeServerTransport {
    /// The server let go of this peer: say so to the Keep, which closes the
    /// link (a `Left` then follows, for a peer this server no longer holds).
    fn drop(&mut self) {
        if self.seated.load(Ordering::Relaxed) {
            self.out.send(&FromChild::Unseated { peer: self.peer });
        }
        if !self.shared.left.load(Ordering::Acquire) {
            let reason = self.reason.lock().unwrap_or_else(|e| e.into_inner()).take();
            self.cut_off(reason.as_deref().unwrap_or(LET_GO));
        }
    }
}

// --- stdin: the reader ---

/// What the reader tells the server's loop.
#[derive(Debug, PartialEq, Eq)]
pub enum Control {
    /// The Keep's `Hello`: always the first, at most once.
    Hello { max_players: u8 },
    /// The Keep asks for a save and an exit.
    Shutdown,
    /// stdin ended between frames: the Keep is gone.
    Ended,
    /// A protocol error (or stdin could not be read), and why. The reader has
    /// stopped.
    Fatal(String),
}

/// Start the reader thread on `input` (stdin, or a test's stream): new peers
/// go to `joiners` (the server's accept channel), everything else that is not
/// a peer's message to the returned channel.
pub fn spawn_reader(
    input: impl Read + Send + 'static,
    out: PipeOut,
    joiners: mpsc::Sender<Box<dyn ServerTransport>>,
) -> io::Result<mpsc::Receiver<Control>> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("keep-pipe-in".into())
        .spawn(move || read_loop(input, &out, &joiners, &tx, MAX_INBOUND_BYTES))?;
    Ok(rx)
}

fn read_loop(
    input: impl Read,
    out: &PipeOut,
    joiners: &mpsc::Sender<Box<dyn ServerTransport>>,
    control: &mpsc::Sender<Control>,
    inbound_cap: usize,
) {
    let mut input = io::BufReader::new(input);
    let mut peers: HashMap<u32, PeerSlot> = HashMap::new();
    let mut max_players: Option<usize> = None;
    let end = loop {
        let body = match read_frame(&mut input) {
            Ok(Some(body)) => body,
            Ok(None) => break Control::Ended,
            Err(e) => break Control::Fatal(e.to_string()),
        };
        let message = match decode_to_child(&body) {
            Ok(message) => message,
            Err(e) => break Control::Fatal(e.to_string()),
        };
        let Some(max) = max_players else {
            match message {
                ToChild::Hello { proto: PROTO, max_players: 0 } => {
                    break Control::Fatal("the Keep's Hello gives the world no seats (max_players is 0)".to_string());
                }
                ToChild::Hello { proto: PROTO, max_players: seats } => {
                    max_players = Some(usize::from(seats));
                    let _ = control.send(Control::Hello { max_players: seats });
                    continue;
                }
                ToChild::Hello { proto, .. } => {
                    break Control::Fatal(format!("the Keep speaks pipe protocol {proto}; this server speaks {PROTO}"));
                }
                other => break Control::Fatal(format!("the first frame must be Hello, not {}", other.name())),
            }
        };
        match message {
            ToChild::Hello { .. } => break Control::Fatal("a second Hello".to_string()),
            ToChild::Joined { peer, binding } => {
                if peers.contains_key(&peer) {
                    // Two live connections under one id would put the second's
                    // packets into the first's transport, binding and seat.
                    // Neither is served: the server saves and exits.
                    break Control::Fatal(format!("peer {peer} joined twice without leaving"));
                } else if peers.len() >= max {
                    log::info!("keep pipe: peer {peer} joined a full world ({max} players)");
                    out.send(&FromChild::Disconnect { peer, reason: WORLD_FULL.to_string() });
                } else {
                    let (slot, transport) = peer_pair(peer, binding, out.clone());
                    // If the server is gone the transport is dropped here,
                    // which tells the Keep.
                    if joiners.send(Box::new(transport)).is_ok() {
                        peers.insert(peer, slot);
                    }
                }
            }
            ToChild::Reliable { peer, bytes } => {
                if let Some(slot) = peers.get_mut(&peer) {
                    slot.deliver(peer, bytes, inbound_cap);
                }
            }
            // The game sends and reads reliable packets only.
            ToChild::Lossy { .. } => {}
            ToChild::Left { peer } => {
                // A peer this server already let go of, or never held, is
                // nothing to do.
                if let Some(slot) = peers.remove(&peer) {
                    slot.finish();
                }
            }
            ToChild::GateAnswer { peer, ok, reason } => match peers.get(&peer) {
                Some(slot) => slot.answer(peer, ok, reason),
                None => log::info!("keep pipe: a gate answer for peer {peer}, who is not here, is ignored"),
            },
            ToChild::Shutdown => {
                let _ = control.send(Control::Shutdown);
            }
        }
    };
    // Every peer still held leaves with the pipe.
    for (_, slot) in peers.drain() {
        slot.finish();
    }
    let _ = control.send(end);
}

#[cfg(test)]
pub(crate) mod tests;
