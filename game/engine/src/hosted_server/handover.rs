//! World handover, source side: the control link to ONE selected joiner and
//! the source freeze.
//!
//! The link is bound to an exact authenticated seat — slot, attach generation
//! and verified key — and every boundary re-checks that triple; an npub from
//! UI text or an old disconnected seat cannot satisfy it. While frozen the
//! server runs no tick at all (`HostedServer::tick` returns at once): no
//! admission, no inbound gameplay, no simulation, grants, autosave or
//! broadcast. Control frames from the selected seat are read out of its queue
//! by `handover_take_frames`; everything else that seat sent stays queued in
//! its original order, and every other seat's packets stay in its transport.
//! A pre-retirement release resumes them through the ordinary path only while
//! the seat is still that exact live seat (a reused, departed or closed seat's
//! queue is discarded, never replayed into a world its client can't see).
//!
//! Every joined player is told when play pauses (freeze) and resumes
//! (release), so their clients stop sending input instead of queueing it.
//! The link holds only a weak lease on its session: a session dropped without
//! an explicit abort is released (and the recipient told) at the next tick.
use super::HostedServer;
use crate::protocol::{self, PacketType};
use std::collections::VecDeque;

/// Frames waiting for the session before it is frozen (consent phase).
pub const INBOX_FRAMES: usize = 32;
pub const INBOX_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub struct HandoverLink {
    slot: usize,
    generation: u32,
    key: [u8; 32],
    frozen: bool,
    inbox: VecDeque<Vec<u8>>,
    inbox_bytes: usize,
    broken: Option<&'static str>,
    transfer: crate::handover::Id,
    lease: std::sync::Weak<()>,
    /// L1: other seats whose bounded inbound queue overflowed while frozen.
    /// They are read no further and disconnected at release.
    overflowed: Vec<usize>,
    /// CP7 review HIGH-1: the session reached Retired (or an uncertain
    /// retirement). Nothing unfreezes this world any more: release and reap
    /// do nothing; only tearing the HostedServer down ends the freeze.
    retired: bool,
    /// CP14: the world folder whose late-save fence this freeze raised
    /// (`save_fence`). Lifted only by a pre-retirement release.
    fence: Option<std::path::PathBuf>,
}

/// Why a link refused or broke. Short reasons, no keys or frame text.
pub type LinkError = &'static str;

impl HostedServer {
    /// Is `slot` exactly this authenticated live seat right now?
    pub fn handover_seat_live(&self, slot: usize, generation: u32, key: [u8; 32]) -> bool {
        self.handshake_done.get(slot).copied().unwrap_or(false)
            && !self.disconnected.get(slot).copied().unwrap_or(true)
            && !self.transports[slot].is_closed()
            && self.server.players.get(slot).is_some_and(|sp| {
                sp.connected
                    && sp.server_simulated
                    && sp.attach_gen == generation
                    && sp.verified_pubkey == Some(key)
            })
    }

    /// The live seat's generation and verified key, for the host action.
    pub fn handover_seat(&self, slot: usize) -> Option<(u32, [u8; 32])> {
        let sp = self.server.players.get(slot)?;
        let key = sp.verified_pubkey?;
        self.handover_seat_live(slot, sp.attach_gen, key)
            .then_some((sp.attach_gen, key))
    }

    /// Bind the control link to one authenticated seat for one transfer.
    /// One link at a time. The returned lease keeps it bound: drop it and the
    /// next tick releases the link.
    pub fn handover_bind(
        &mut self,
        slot: usize,
        generation: u32,
        key: [u8; 32],
        transfer: crate::handover::Id,
    ) -> Result<std::sync::Arc<()>, LinkError> {
        if self.handover.is_some() {
            return Err("a handover is already in progress");
        }
        if !self.handover_seat_live(slot, generation, key) {
            return Err("that player is no longer signed in to this world");
        }
        let lease = std::sync::Arc::new(());
        self.handover = Some(HandoverLink {
            slot,
            generation,
            key,
            frozen: false,
            inbox: VecDeque::new(),
            inbox_bytes: 0,
            broken: None,
            transfer,
            lease: std::sync::Arc::downgrade(&lease),
            overflowed: Vec::new(),
            retired: false,
            fence: None,
        });
        Ok(lease)
    }

    /// Re-check the bound seat; a changed or departed seat breaks the link.
    pub fn handover_check(&mut self) -> Result<(), LinkError> {
        let Some(l) = &self.handover else { return Err("no handover link") };
        if let Some(why) = l.broken {
            return Err(why);
        }
        let (slot, generation, key) = (l.slot, l.generation, l.key);
        if !self.handover_seat_live(slot, generation, key) {
            let why = "the recipient left or reconnected";
            if let Some(l) = &mut self.handover {
                l.broken = Some(why);
            }
            return Err(why);
        }
        Ok(())
    }

    /// Freeze the source. Only after the seat re-check passes. Every joined
    /// player is told play is paused. CP14: the freeze first raises the
    /// late-save fence over `world_dir` (`save_fence::raise`, which waits
    /// for a commit under way), so the capture taken next reflects the last
    /// save that landed and no later save commits into that folder.
    pub fn handover_freeze(&mut self, world_dir: Option<&std::path::Path>) -> Result<(), LinkError> {
        self.handover_check()?;
        let mut transfer = None;
        if let Some(l) = &mut self.handover {
            if !l.frozen {
                if let Some(dir) = world_dir {
                    crate::save_fence::raise(dir);
                    l.fence = Some(dir.to_path_buf());
                }
                transfer = Some(l.transfer);
            }
            l.frozen = true;
        }
        if let Some(t) = transfer {
            self.handover_notice_all(t, true);
        }
        Ok(())
    }

    /// Tell every joined, live player that play paused or resumed.
    fn handover_notice_all(&self, transfer: crate::handover::Id, paused: bool) {
        let Ok(frame) = crate::handover::live::wire::Frame::Notice { transfer, paused }.encode() else { return };
        let mut pkt = vec![PacketType::Handover as u8];
        pkt.extend_from_slice(&frame);
        for j in 0..self.transports.len() {
            self.send_to_joined_slot(j, &pkt);
        }
    }

    /// A link whose session was dropped without an explicit abort: tell the
    /// recipient and release. Called at the top of every tick (frozen or not).
    pub(super) fn handover_reap(&mut self) {
        let Some(l) = &self.handover else { return };
        // HIGH-1: a retired world stays frozen (no Abort, no release) until
        // it is torn down, even once its session is gone.
        if l.retired || l.lease.strong_count() > 0 {
            return;
        }
        let (slot, transfer) = (l.slot, l.transfer);
        if let Ok(frame) = (crate::handover::live::wire::Frame::Abort { transfer, aborted: None }).encode() {
            let mut pkt = vec![PacketType::Handover as u8];
            pkt.extend_from_slice(&frame);
            self.send_to_joined_slot(slot, &pkt);
        }
        self.handover_release();
    }

    /// HIGH-1: the bound session retired this world (or may have): from now
    /// on neither `handover_release` nor `handover_reap` unfreezes it.
    pub fn handover_mark_retired(&mut self) {
        if let Some(l) = self.handover.as_mut() {
            l.retired = true;
            // CP14: a retired world's late-save fence never lifts.
            if let Some(dir) = &l.fence {
                crate::save_fence::retire(dir);
            }
        }
    }

    pub fn handover_frozen(&self) -> bool {
        self.handover.as_ref().is_some_and(|l| l.frozen)
    }

    /// The bound seat (slot, generation, key), if any.
    pub fn handover_link(&self) -> Option<(usize, u32, [u8; 32])> {
        self.handover.as_ref().map(|l| (l.slot, l.generation, l.key))
    }

    /// Send one control frame to the bound seat only.
    pub fn handover_send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        self.handover_check()?;
        let slot = self.handover.as_ref().map(|l| l.slot).unwrap_or(usize::MAX);
        let mut pkt = Vec::with_capacity(frame.len() + 1);
        pkt.push(PacketType::Handover as u8);
        pkt.extend_from_slice(frame);
        self.send_to_joined_slot(slot, &pkt);
        // CP8 review MEDIUM-A: this buffer may carry the bearer frame; it is
        // ours, so it is wiped (unconditionally: frames are small and this
        // never sees the frame type).
        zeroize::Zeroize::zeroize(&mut pkt);
        Ok(())
    }

    /// Control frames the bound seat sent, oldest first. While frozen they
    /// are read straight out of that seat's queue (no tick runs); before
    /// that, `process_inbound_packets` routes them into the link's inbox.
    ///
    /// CP13 (CP12 review MEDIUM-1): a frozen link drains the seat's queue
    /// BEFORE the liveness refusal. The transport guarantees "closed => its
    /// last frame is in `rx`", so a PreparedAck the recipient sent before it
    /// left (CP12's leave closes the link) is still read; verified and
    /// bearer-proven, it retires the world (`eligibility::seats` still
    /// requires that exact seat, generation and key). Only the same seat
    /// whose transport alone has closed: a broken link, a disconnect or a
    /// reconnect is refused first, and a closed seat with nothing queued is
    /// refused as before.
    pub fn handover_take_frames(&mut self) -> Result<Vec<Vec<u8>>, LinkError> {
        let (slot, generation, key, frozen) = match &self.handover {
            Some(l) if l.broken.is_some() => return self.handover_check().map(|()| Vec::new()),
            Some(l) => (l.slot, l.generation, l.key, l.frozen),
            None => return Err("no handover link"),
        };
        let live = self.handover_seat_live(slot, generation, key);
        if !live && !(frozen && self.handover_seat_closed(slot, generation, key)) {
            return self.handover_check().map(|()| Vec::new());
        }
        let mut out: Vec<Vec<u8>> = Vec::new();
        if let Some(l) = &mut self.handover {
            out.extend(l.inbox.drain(..));
            l.inbox_bytes = 0;
        }
        if frozen {
            let fill = self.inbound[slot].fill_from(&*self.transports[slot]);
            if fill.is_err() {
                if let Some(l) = &mut self.handover {
                    l.broken = Some("the recipient's connection overflowed");
                }
                return Err("the recipient's connection overflowed");
            }
            let frames = self.inbound[slot].extract(|p| {
                matches!(protocol::deserialize_header(p), Some((PacketType::Handover, _)))
            });
            out.extend(frames.into_iter().map(|mut p| {
                p.remove(0);
                p
            }));
            self.handover_fill_guests(slot);
        }
        if !live && out.is_empty() {
            return self.handover_check().map(|()| out);
        }
        Ok(out)
    }

    /// Test only (CP13): the seat's connection closes the way the QUIC
    /// transport's does (network.rs: "closed => its last frame is in `rx`"):
    /// `is_closed` turns true at once, and what it already sent stays queued.
    /// (The in-process channel only reports closed once its queue is empty.)
    #[cfg(test)]
    pub(crate) fn test_close_seat_transport(&mut self, slot: usize) {
        use crate::transport::{ClosedTransport, Packet, ServerTransport};
        struct ClosedAtOnce(Box<dyn ServerTransport>);
        impl ServerTransport for ClosedAtOnce {
            fn send_to_client(&self, data: &[u8]) {
                self.0.send_to_client(data)
            }
            fn try_recv_from_client(&self) -> Option<Packet> {
                self.0.try_recv_from_client()
            }
            fn channel_binding(&self) -> Option<[u8; 32]> {
                self.0.channel_binding()
            }
            fn is_websocket(&self) -> bool {
                self.0.is_websocket()
            }
            fn is_closed(&self) -> bool {
                true
            }
        }
        let inner = std::mem::replace(&mut self.transports[slot], Box::new(ClosedTransport));
        self.transports[slot] = Box::new(ClosedAtOnce(inner));
    }

    /// CP13: `slot` is exactly the bound seat, still authenticated and not
    /// disconnected, and only its transport has closed (the leave).
    fn handover_seat_closed(&self, slot: usize, generation: u32, key: [u8; 32]) -> bool {
        self.handshake_done.get(slot).copied().unwrap_or(false)
            && !self.disconnected.get(slot).copied().unwrap_or(true)
            && self.transports.get(slot).is_some_and(|t| t.is_closed())
            && self.server.players.get(slot).is_some_and(|sp| {
                sp.connected && sp.server_simulated && sp.attach_gen == generation && sp.verified_pubkey == Some(key)
            })
    }

    /// L1: while frozen, every other seat's packets move into its own
    /// bounded inbound queue, unprocessed (they wait for release in order).
    /// A seat that overflows its bound is read no further and is
    /// disconnected at release by the ordinary overflow kick.
    fn handover_fill_guests(&mut self, bound: usize) {
        for j in 0..self.transports.len() {
            let over = self.handover.as_ref().is_some_and(|l| l.overflowed.contains(&j));
            if j == bound || over || self.disconnected[j] {
                continue;
            }
            if self.inbound[j].fill_from(&*self.transports[j]).is_err() {
                if let Some(l) = &mut self.handover {
                    l.overflowed.push(j);
                }
                // Review LOW-1: close it now. Dropping the real transport ends
                // its reader and the connection, so nothing keeps buffering
                // for the rest of the freeze; the slot bookkeeping (leave,
                // save) still waits for release.
                self.transports[j] = Box::new(crate::transport::ClosedTransport);
            }
        }
    }

    /// Pretend this host runs `--no-lend` (tests; L3).
    #[cfg(test)]
    pub fn handover_test_set_owned(&mut self) {
        self.host_world = super::HostWorld::Owned;
    }

    /// `slot`'s server-side transport is closed (tests; review LOW-1).
    #[cfg(test)]
    pub fn handover_test_transport_closed(&self, slot: usize) -> bool {
        self.transports[slot].is_closed()
    }

    /// Bytes waiting in `slot`'s bounded inbound queue (tests).
    #[cfg(test)]
    pub fn handover_test_inbound_bytes(&self, slot: usize) -> usize {
        self.inbound[slot].charged_bytes()
    }

    /// Route a control frame that arrived through ordinary processing.
    /// Frames from any seat but the bound one are dropped.
    pub(super) fn route_handover_frame(&mut self, i: usize, payload: &[u8]) {
        let Some(l) = &mut self.handover else { return };
        if l.slot != i || l.broken.is_some() {
            return;
        }
        if l.inbox.len() >= INBOX_FRAMES || l.inbox_bytes + payload.len() > INBOX_BYTES {
            l.broken = Some("the recipient sent too many control frames");
            return;
        }
        l.inbox_bytes += payload.len();
        l.inbox.push_back(payload.to_vec());
    }

    /// Pre-retirement abort: unfreeze and drop the link. The bound seat's
    /// waiting gameplay resumes through the ordinary path only while it is
    /// still that exact live seat (its client restored the connection and
    /// sees the world); a departed, reused or closed seat's queue is
    /// discarded. Joined players are told play resumes.
    pub fn handover_release(&mut self) {
        // HIGH-1: past retirement nothing resumes this world.
        if self.handover.as_ref().is_some_and(|l| l.retired) {
            return;
        }
        let Some(l) = self.handover.take() else { return };
        // CP14: the one place the late-save fence lifts: an abort that
        // unfreezes before retirement.
        if let Some(dir) = &l.fence {
            crate::save_fence::lift(dir);
        }
        if !self.handover_seat_live(l.slot, l.generation, l.key) {
            self.inbound[l.slot] = Default::default();
        }
        if l.frozen {
            self.handover_notice_all(l.transfer, false);
        }
        for j in l.overflowed {
            // Review LOW-2: a release outside the lend (a cancel from the
            // frame's control pump) defers the kick to the next lent tick,
            // so the guest's leave settles (grants, window, death) first.
            if self.lends_host_world() && !self.server.lent {
                if !self.deferred_overflow.contains(&j) {
                    self.deferred_overflow.push(j);
                }
                continue;
            }
            self.handover_overflow_kick(j);
        }
    }

    /// The ordinary overflow kick for a guest that overflowed while frozen.
    pub(super) fn handover_overflow_kick(&mut self, j: usize) {
        log::warn!("Player {j} overflowed its inbound bound during a handover freeze — disconnecting it");
        if let Some(left) = self.release_slot(j, Some(super::INBOUND_OVERFLOW_REASON)) {
            for k in 0..self.transports.len() {
                if k != j && self.handshake_done[k] && !self.disconnected[k] {
                    self.transports[k].send_to_client(&left);
                }
            }
        }
    }

    /// `slot` has been released (tests; review LOW-2).
    #[cfg(test)]
    pub fn handover_test_slot_released(&self, slot: usize) -> bool {
        self.disconnected[slot]
    }

    /// Kicks waiting for the next lent tick (tests; review LOW-2).
    #[cfg(test)]
    pub fn handover_test_deferred_overflow(&self) -> usize {
        self.deferred_overflow.len()
    }
}
