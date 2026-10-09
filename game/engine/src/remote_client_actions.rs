//! Retained joined actions and recovery parsing. Application stays in world intake.

use super::{ConnectionState, RemoteClient};
use crate::protocol::{self, PacketType};

impl RemoteClient {
    /// Must succeed BEFORE the caller changes holdings, world or local ECS.
    /// Retains the complete packet; sender/resubmission never allocates an id.
    pub fn reserve_action(&mut self, intent: crate::action_wire::ActionIntent, recipe: Box<[u8]>, cells: Box<[[i32; 3]]>) -> Result<crate::action_identity::ActionHeader, String> {
        if self.recovery_prepare.is_some() { return Err("Inventory recovery is pending".into()); }
        let journal = self.action_journal.as_mut().ok_or("No joined action session")?;
        let action = journal.next_header(self.events_applied).map_err(|e| format!("Action allocation failed: {e:?}"))?;
        let packet = crate::action_wire::ActionPacket { action, intent };
        let bytes = packet.encode()?;
        let retained = journal.append(bytes, recipe, cells, self.events_applied).map_err(|e| format!("Action reservation failed: {e:?}"))?;
        debug_assert_eq!(retained, action);
        Ok(action)
    }

    /// Sends a retained identity, never allocates or retires history.
    pub fn send_reserved_action(&mut self, action: crate::action_identity::ActionHeader) -> Result<(), String> {
        if self.recovery_prepare.is_some() { return Err("Inventory recovery is pending".into()); }
        let journal = self.action_journal.as_mut().ok_or("No joined action session")?;
        let retained = journal.actions().find(|a| a.header == action).ok_or("Action was not reserved")?;
        let mut packet = Vec::with_capacity(retained.intent.len() + 1);
        packet.push(PacketType::Action as u8);
        packet.extend_from_slice(&retained.intent);
        self.transport.send_to_server(&packet);
        journal.mark_sent(action.seq);
        Ok(())
    }

    pub(super) fn receive_recovery(&mut self, packet: crate::action_wire::RecoveryPacket) -> Result<(), String> {
        use crate::action_wire::RecoveryPacket;
        match packet {
            RecoveryPacket::Prepare(cut) => {
                if self.recovery_prepare == Some(cut) { return Ok(()); }
                let journal = self.action_journal.as_ref().ok_or("No joined action session")?;
                if !journal.accepts_cut(cut) || cut.id <= self.last_recovery_id { return Err("Superseded or foreign recovery".into()); }
                self.last_recovery_id = cut.id;
                self.recovery_prepare = Some(cut);
                self.recovery_assembly = None;
                self.queued_recovery_fence = None;
                let cells: Vec<_> = journal.actions().flat_map(|a| a.cells.iter().copied()).collect();
                let parts = cells.len().div_ceil(crate::action_wire::DESCRIBE_PAGE_CELLS);
                let manifest = crate::action_wire::DescribeManifest { cut, cells: cells.len() as u16, parts: parts as u16 };
                let mut packets = vec![protocol::serialize_packet(PacketType::Recovery, &RecoveryPacket::DescribeBegin(manifest))];
                for (index, cells) in cells.chunks(crate::action_wire::DESCRIBE_PAGE_CELLS).enumerate() {
                    let describe = crate::action_wire::RebaseDescribe { cut, index: index as u16, cells: crate::action_wire::BoundedList::new(cells.to_vec())? };
                    packets.push(protocol::serialize_packet(PacketType::Recovery, &RecoveryPacket::Describe(describe)));
                }
                packets.push(protocol::serialize_packet(PacketType::Recovery, &RecoveryPacket::DescribeEnd(cut)));
                let retained = packets.iter().map(Vec::capacity).sum::<usize>() + packets.capacity() * std::mem::size_of::<Vec<u8>>() + cells.capacity() * std::mem::size_of::<[i32; 3]>();
                if retained > crate::action_journal::MAX_CONTEXT_BYTES || packets.iter().any(|p| p.len() > crate::action_journal::MAX_PAGE_BYTES) {
                    return Err("Recovery context exceeds its byte limit".into());
                }
                for packet in packets { self.transport.send_to_server(&packet); }
            }
            RecoveryPacket::Begin(manifest) => {
                if self.recovery_prepare != Some(manifest.cut) { return Err("Recovery Begin has no matching Prepare".into()); }
                if let Some(old) = &self.recovery_assembly {
                    return if old.manifest() == manifest { Ok(()) } else { Err("Conflicting recovery Begin".into()) };
                }
                self.recovery_assembly = Some(crate::rebase_assembly::RebaseAssembly::new(manifest, manifest.cut.session, manifest.cut.life).map_err(|e| format!("Invalid recovery Begin: {e:?}"))?);
            }
            RecoveryPacket::Page(page) => {
                self.recovery_assembly.as_mut().ok_or("Recovery page has no Begin")?.page(page.cut, page.index, page.bytes.into_vec().into_boxed_slice()).map_err(|e| format!("Invalid recovery page: {e:?}"))?;
            }
            RecoveryPacket::End(cut) => {
                self.recovery_assembly.as_mut().ok_or("Recovery End has no Begin")?.end(cut).map_err(|e| format!("Invalid recovery End: {e:?}"))?;
            }
            RecoveryPacket::Fence(cut) => {
                if self.recovery_prepare != Some(cut) { return Err("Foreign recovery fence".into()); }
                if self.queued_recovery_fence == Some(cut) { return Ok(()); }
                self.queued_recovery_fence = Some(cut);
                // Same offset as views/attachments; carried chunks hold this
                // fence and every later step. No activation at socket parse.
                self.chunk_queue.push((self.pending_block_changes.len(), crate::chunk_intake::StreamItem::RebaseFence(cut)));
            }
            RecoveryPacket::DescribeBegin(_) | RecoveryPacket::Describe(_) | RecoveryPacket::DescribeEnd(_) | RecoveryPacket::Ack(_) | RecoveryPacket::DeathDescribe { .. } => return Err("Client-only recovery carrier received from server".into()),
        }
        Ok(())
    }

    /// Called only from GameState's actual ordered world intake.
    pub fn apply_recovery_fence(&mut self, cut: crate::action_identity::RecoveryCut) {
        if self.recovery_prepare.is_some_and(|current| current.session == cut.session && current.life == cut.life && current.id > cut.id) { return; }
        if let Some(assembly) = &mut self.recovery_assembly {
            if let Err(error) = assembly.apply_fence(cut) {
                self.state = ConnectionState::Failed(format!("Recovery fence could not apply: {error:?}"));
            }
        } else { self.state = ConnectionState::Failed("Recovery fence has no assembled bundle".into()); }
    }

    pub fn apply_action_receipt(&mut self, receipt: crate::action_wire::ActionReceipt) {
        if self.queued_action_receipts == 0 || self.action_journal.as_ref().is_none_or(|journal| !journal.receipt_frontier_is_valid(receipt)) {
            self.state = ConnectionState::Failed("Invalid action receipt frontier".into());
            return;
        }
        self.action_receipts.push_back(receipt);
        self.retire_applied_receipts();
    }

    pub(super) fn retire_applied_receipts(&mut self) {
        while let Some(receipt) = self.action_receipts.front().copied() {
            if receipt.included_events > self.events_applied { break; }
            let Some(journal) = &mut self.action_journal else {
                self.state = ConnectionState::Failed("Receipt has no joined action session".into());
                break;
            };
            if journal.retired_receipt_is_duplicate(receipt) {
                self.action_receipts.pop_front();
                self.queued_action_receipts -= 1;
                continue;
            }
            if let Err(error) = journal.retire_receipt(receipt, self.events_applied, receipt.action.seq) {
                self.state = ConnectionState::Failed(format!("Invalid applied action receipt: {error:?}"));
                break;
            }
            self.action_receipts.pop_front();
            self.queued_action_receipts -= 1;
        }
    }

}
