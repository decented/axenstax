//! Retained joined actions and recovery parsing. Application stays in world intake.

use super::{ConnectionState, RemoteClient};
use crate::protocol::{self, PacketType};

/// Shown when a Strict client finds an inventory change of its own that it
/// can't send to the world (C3d run 4): it leaves rather than keep a
/// window the world never saw.
pub const STRICT_STRAY_CHANGE_REASON: &str =
    "This game made an inventory change it couldn't send to this world, so we left. Anything this world saved for you is kept.";

/// C3d (run 4) — player 0's window flush on a Strict connection: its
/// auto-refill setting goes as a reserved action, and every other window
/// change already went through a reserving entry point, so nothing is
/// logged for the legacy path. A logged legacy op here is a client
/// invariant breach — the window may already show its prediction — so it is
/// never silently discarded: with no whole-window resync request to ask
/// for, the client leaves the world with a plain reason
/// ([`RemoteClient::leave_for_strict_stray_ops`]). Returns whether it left.
pub fn flush_strict_window_ops(
    client: &mut RemoteClient,
    ui: &mut crate::craft_ui::CraftingUi,
    inv: &crate::inventory::Inventory,
    armour: &[Option<crate::armour::ArmourItem>; 4],
) -> bool {
    // Run 4 (run-3 review M1) — the session-start setting goes only once
    // the join's window sync (always event 1 when there is one) is applied;
    // before that, the first window action carries it (entry points).
    if client.applied_window_boundary() >= 1
        && let Err(error) = ui.sync_auto_refill_joined(client, inv, armour) {
        log::debug!("Strict auto-refill setting could not be reserved: {error}");
    }
    let stray = ui.ops.take_before(None);
    if stray.is_empty() { return false; }
    client.leave_for_strict_stray_ops(stray.len());
    true
}

impl RemoteClient {
    /// C3d (run 4) — a Strict client found `count` legacy window ops in its
    /// flush (see [`flush_strict_window_ops`]): logged, a graceful
    /// `Disconnect` sent, and the session ended with
    /// [`STRICT_STRAY_CHANGE_REASON`]; the world exit then discards the
    /// joined copy. Nothing is sent as a `WindowOp`.
    pub fn leave_for_strict_stray_ops(&mut self, count: usize) {
        log::error!("Strict client invariant breached: {count} legacy window op(s) in its flush; leaving the world");
        let packet = protocol::serialize_packet(PacketType::Disconnect, &());
        self.transport.send_to_server(&packet);
        self.respawn_resend_from = None;
        self.state = ConnectionState::Failed(STRICT_STRAY_CHANGE_REASON.to_string());
    }

    pub fn applied_window_boundary(&self) -> u32 { self.events_applied }
    /// C3d run 6 — a Respawn this client sent that the server hasn't answered.
    pub fn respawn_unanswered(&self) -> bool { self.respawn_resend_from.is_some() }
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
        // C3d run 12 (survey §4.6) — Mirror never sees recovery: fail closed.
        if self.inventory_mode != crate::action_identity::InventoryMode::Strict { return Err("Recovery on a Mirror connection".into()); }
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

    fn flush_applied_action_ack(&mut self) {
        if let Some(ack) = self.pending_action_ack.take() {
            self.transport.send_to_server(&protocol::serialize_packet(PacketType::AppliedActionAck, &ack));
        }
    }

    /// C3d run 16 — does the journal hold an unretired `SetAutoRefill`
    /// (after `seq`, when given)?
    pub(super) fn set_auto_refill_journaled_after(&self, seq: Option<u64>) -> bool {
        self.action_journal.as_ref().is_some_and(|journal| {
            journal.actions().any(|a| seq.is_none_or(|s| a.header.seq > s) && is_set_auto_refill(&a.intent))
        })
    }

    pub(super) fn retire_applied_receipts(&mut self) {
        while let Some(receipt) = self.action_receipts.front().copied() {
            if receipt.included_events > self.events_applied { break; }
            match self.pack_receipt_ready(receipt) {
                Ok(false) => break,
                Err(reason) => { self.state = ConnectionState::Failed(reason); break; }
                Ok(true) => {}
            }
            let Some(journal) = &mut self.action_journal else {
                self.state = ConnectionState::Failed("Receipt has no joined action session".into());
                break;
            };
            if journal.retired_receipt_is_duplicate(receipt) {
                self.action_receipts.pop_front();
                self.queued_action_receipts -= 1;
                continue;
            }
            // Run 16 — unless a later setting is already journaled (the
            // `Replace` that came with this refusal sent it again).
            if receipt.fate == crate::action_identity::ActionFate::Refused
                && journal.actions().find(|a| a.header == receipt.action).is_some_and(|a| is_set_auto_refill(&a.intent))
                && !journal.actions().any(|a| a.header.seq > receipt.action.seq && is_set_auto_refill(&a.intent))
            {
                self.refill_refused = true;
            }
            // Run 17 — a refused or superseded attack or shot is never
            // answered: its request record goes (review LOW-2).
            let released = if matches!(receipt.fate, crate::action_identity::ActionFate::Refused | crate::action_identity::ActionFate::Superseded) {
                journal.actions().find(|a| a.header == receipt.action).and_then(|a| unanswered_request(&a.intent))
            } else {
                None
            };
            if let Err(error) = journal.retire_receipt(receipt, self.events_applied, receipt.action.seq) {
                self.state = ConnectionState::Failed(format!("Invalid applied action receipt: {error:?}"));
                break;
            }
            if let Some(seq) = released {
                if self.released_requests.len() >= crate::joiner_actions::MAX_PENDING { self.released_requests.remove(0); }
                self.released_requests.push(seq);
            }
            let ack = crate::action_wire::AppliedActionAck { session: receipt.action.session, epoch: receipt.action.epoch, life: receipt.action.life, through: receipt.action.seq, included_events: receipt.included_events };
            self.pending_action_ack = Some(ack);
            self.retire_pack_flight(receipt);
            self.action_receipts.pop_front();
            self.queued_action_receipts -= 1;
        }
        // Event arrival also calls this path: no subsequent gameplay input is
        // needed to send the one coalesced, actually-applied metadata frontier.
        self.flush_applied_action_ack();
    }

}

/// Is a journaled intent (its encoded packet) a `SetAutoRefill`?
/// C3d run 17 — the request `seq` of a journaled typed attack or shot (the
/// two typed classes recorded in `JoinerActions`), else `None`. C3d run 23 —
/// and of a typed interaction (a corrected refusal sends no outcome; a
/// world-fact one sends its note too, and a second release is a no-op).
fn unanswered_request(intent: &[u8]) -> Option<u32> {
    match crate::action_wire::ActionPacket::decode(intent).ok()?.intent {
        crate::action_wire::ActionIntent::Attack(p) => Some(p.seq),
        crate::action_wire::ActionIntent::Interact(p) => Some(p.seq),
        // C3d run 24 — and of a typed eat or creative give.
        crate::action_wire::ActionIntent::Item(p) if matches!(p.action, protocol::ItemAction::Shoot { .. } | protocol::ItemAction::Eat { .. } | protocol::ItemAction::CreativeGive { .. }) => Some(p.seq),
        _ => None,
    }
}

fn is_set_auto_refill(intent: &[u8]) -> bool {
    crate::action_wire::ActionPacket::decode(intent)
        .is_ok_and(|p| matches!(p.intent, crate::action_wire::ActionIntent::Window(op) if matches!(op.op, protocol::WireWindowOp::SetAutoRefill { .. })))
}
