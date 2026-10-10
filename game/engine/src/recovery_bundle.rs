//! One owned, bounded FIFO bundle. No later world step enters its middle.

use crate::action_identity::RecoveryCut;
use crate::action_journal::{MAX_BUNDLE_BYTES, MAX_PAGE_BYTES};
use crate::action_wire::{BoundedList, RebasePage, RecoveryPacket};
use crate::protocol::{self, PacketType};
use crate::rebase_assembly::{RebaseManifest, MAX_REBASE_PARTS};

#[derive(Debug)]
pub struct RecoveryBundle {
    cut: RecoveryCut,
    packets: Box<[Box<[u8]>]>,
    next: usize,
    wire_bytes: usize,
}

impl RecoveryBundle {
    /// Takes already frozen owned pages; all validation precedes queue commit.
    /// `owner_overhead` charges the actual enclosing queue entry as well.
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d recovery milestone (RebasePrepare, Describe, capture, applied fence and replay)"))]
    pub fn new(manifest: RebaseManifest, pages: Vec<Box<[u8]>>, owner_overhead: usize) -> Result<Self, &'static str> {
        if pages.is_empty() || pages.len() > MAX_REBASE_PARTS || pages.len() != usize::from(manifest.parts) {
            return Err("Recovery part count mismatch");
        }
        let mut body_bytes = 0usize;
        for page in &pages {
            if page.is_empty() || page.len() > MAX_PAGE_BYTES { return Err("Recovery page exceeds its byte limit"); }
            body_bytes = body_bytes.checked_add(page.len()).ok_or("Recovery byte count overflow")?;
        }
        if body_bytes != manifest.encoded_bytes as usize { return Err("Recovery byte count mismatch"); }
        let mut packets = Vec::with_capacity(pages.len() + 3);
        let encode = |packet| protocol::serialize_packet(PacketType::Recovery, &packet).into_boxed_slice();
        packets.push(encode(RecoveryPacket::Begin(manifest)));
        for (index, bytes) in pages.into_iter().enumerate() {
            packets.push(encode(RecoveryPacket::Page(RebasePage {
                cut: manifest.cut,
                index: index as u16,
                bytes: BoundedList::new(bytes.into_vec())?,
            })));
        }
        packets.push(encode(RecoveryPacket::End(manifest.cut)));
        packets.push(encode(RecoveryPacket::Fence(manifest.cut)));
        let packets = packets.into_boxed_slice();
        let wire_bytes = packets.iter().try_fold(0usize, |sum, page| sum.checked_add(page.len())).ok_or("Recovery byte count overflow")?;
        let retained = wire_bytes.checked_add(std::mem::size_of::<Self>())
            .and_then(|n| packets.len().checked_mul(std::mem::size_of::<Box<[u8]>>()).and_then(|slots| n.checked_add(slots)))
            .and_then(|n| n.checked_add(owner_overhead)).ok_or("Recovery byte count overflow")?;
        if retained > MAX_BUNDLE_BYTES { return Err("Recovery bundle exceeds its byte limit"); }
        Ok(Self { cut: manifest.cut, packets, next: 0, wire_bytes })
    }

    pub fn cut(&self) -> RecoveryCut { self.cut }
    pub fn wire_bytes(&self) -> usize { self.wire_bytes }
    pub fn front(&self) -> Option<&[u8]> { self.packets.get(self.next).map(AsRef::as_ref) }
    pub fn pop_front(&mut self) -> Option<Vec<u8>> {
        let packet = self.packets.get_mut(self.next)?;
        self.next += 1;
        let packet = std::mem::take(packet);
        self.wire_bytes -= packet.len();
        Some(packet.into_vec())
    }
    pub fn is_empty(&self) -> bool { self.next == self.packets.len() }
}
