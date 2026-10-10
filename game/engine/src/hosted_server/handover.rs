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
    /// player is told play is paused.
    pub fn handover_freeze(&mut self) -> Result<(), LinkError> {
        self.handover_check()?;
        let mut transfer = None;
        if let Some(l) = &mut self.handover {
            if !l.frozen {
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
        if let Ok(frame) = (crate::handover::live::wire::Frame::Abort { transfer }).encode() {
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
        Ok(())
    }

    /// Control frames the bound seat sent, oldest first. While frozen they
    /// are read straight out of that seat's queue (no tick runs); before
    /// that, `process_inbound_packets` routes them into the link's inbox.
    pub fn handover_take_frames(&mut self) -> Result<Vec<Vec<u8>>, LinkError> {
        self.handover_check()?;
        let (slot, frozen) = match &self.handover {
            Some(l) => (l.slot, l.frozen),
            None => return Err("no handover link"),
        };
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
        Ok(out)
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
