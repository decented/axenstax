//! Bounded live-session control ownership; existing raw FIFO policy stays separate.
use super::*;
use crate::handover::live::{
    park::{check_incoming_batch, check_incoming_control},
    wire::Frame,
};
use std::mem::size_of;

pub(crate) const SOURCE_CONTROL_FRAMES: usize = 16;

impl HostedServer {
    /// One oldest-first removal limit across routed and frozen raw controls.
    /// The scan still covers the existing bounded raw FIFO, not just16 entries.
    /// No second taken/kept container or unbounded control batch is materialised.
    pub(crate) fn handover_take_source_frames(
        &mut self,
        retained: usize,
    ) -> Result<Vec<Vec<u8>>, LinkError> {
        let (slot, generation, key, frozen) = match &self.handover {
            Some(link) if link.broken.is_some() => {
                return self.handover_check().map(|()| Vec::new());
            }
            Some(link) => (link.slot, link.generation, link.key, link.frozen),
            None => return Err("no handover link"),
        };
        let live = self.handover_seat_live(slot, generation, key);
        // Preserve the final PreparedAck closed-last-frame rule. The source's
        // early park wait separately requires its actual live link first.
        if !live && !(frozen && self.handover_seat_closed(slot, generation, key)) {
            return self.handover_check().map(|()| Vec::new());
        }
        if frozen {
            if self.inbound[slot]
                .fill_from(&*self.transports[slot])
                .is_err()
            {
                self.handover.as_mut().expect("checked link").broken =
                    Some("the recipient's connection overflowed");
                return Err("the recipient's connection overflowed");
            }
            self.handover_fill_guests(slot);
        }
        let mut out = Vec::new();
        out.try_reserve_exact(SOURCE_CONTROL_FRAMES)
            .map_err(|_| "the control batch could not be reserved")?;
        let link = self.handover.as_ref().expect("checked link");
        let routed = link
            .inbox
            .iter()
            .map(|packet| (packet.as_slice(), packet.capacity()));
        let raw = self.inbound[slot]
            .newest(self.inbound[slot].len())
            .filter_map(|packet| {
                if !frozen {
                    return None;
                }
                match protocol::deserialize_header(packet) {
                    Some((PacketType::Handover, payload)) => Some((payload, packet.capacity())),
                    _ => None,
                }
            });
        let mut fixed = 0usize;
        for (payload, capacity) in routed.chain(raw).take(SOURCE_CONTROL_FRAMES) {
            check_incoming_control(payload, capacity).map_err(|r| r.0)?;
            if Frame::is_fixed_park_control(payload) {
                fixed = fixed.saturating_add(capacity);
            }
        }
        check_incoming_batch(
            retained,
            fixed,
            out.capacity().saturating_mul(size_of::<Vec<u8>>()),
        )
        .map_err(|r| r.0)?;
        // All capacity checks precede removal. No fallible allocation remains.
        let link = self.handover.as_mut().expect("checked link");
        while out.len() < SOURCE_CONTROL_FRAMES {
            let Some(packet) = link.inbox.pop_front() else {
                break;
            };
            link.inbox_bytes = link.inbox_bytes.saturating_sub(packet.len());
            out.push(packet);
        }
        if frozen {
            let routed = out.len();
            self.inbound[slot].extract_into(&mut out, SOURCE_CONTROL_FRAMES - routed, |packet| {
                matches!(
                    protocol::deserialize_header(packet),
                    Some((PacketType::Handover, _))
                )
            });
            for packet in &mut out[routed..] {
                packet.remove(0);
            }
        }
        if !live && out.is_empty() {
            return self.handover_check().map(|()| out);
        }
        Ok(out)
    }
}

#[cfg(test)]
struct OverallocatedControl {
    inner: Box<dyn crate::transport::ServerTransport>,
}

#[cfg(test)]
impl crate::transport::ServerTransport for OverallocatedControl {
    fn send_to_client(&self, bytes: &[u8]) {
        self.inner.send_to_client(bytes);
    }
    fn try_recv_from_client(&self) -> Option<crate::transport::Packet> {
        let mut packet = self.inner.try_recv_from_client()?;
        if matches!(protocol::deserialize_header(&packet), Some((PacketType::Handover, payload))
            if Frame::is_fixed_park_control(payload))
        {
            // Preserve the genuine signed bytes; only the actual carrier's
            // allocation is adverse, before source receipt/admission.
            let needed = (crate::handover::live::park::WIRE_CAP + 1).saturating_sub(packet.len());
            packet.reserve_exact(needed);
        }
        Some(packet)
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        self.inner.channel_binding()
    }
    fn is_websocket(&self) -> bool {
        self.inner.is_websocket()
    }
    fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    fn ask_gate(&self, key: &[u8; 32]) -> bool {
        self.inner.ask_gate(key)
    }
    fn gate_verdict(&self) -> Option<Result<(), String>> {
        self.inner.gate_verdict()
    }
    fn seated(&self, key: &[u8; 32], name: &str) {
        self.inner.seated(key, name);
    }
}

#[cfg(test)]
impl HostedServer {
    pub(crate) fn handover_test_overallocate_fixed_controls(&mut self, slot: usize) {
        let inner = std::mem::replace(
            &mut self.transports[slot],
            Box::new(crate::transport::ClosedTransport),
        );
        self.transports[slot] = Box::new(OverallocatedControl { inner });
    }
    pub(crate) fn handover_test_inbound_packets(
        &self,
        slot: usize,
    ) -> impl Iterator<Item = &crate::transport::Packet> {
        self.inbound[slot].newest(self.inbound[slot].len())
    }
}
