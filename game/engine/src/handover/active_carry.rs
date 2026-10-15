//! One current cut of the SAME two participants. Staging owns temporary
//! encodings/maps only; a ticket retains provenance, never a second holder.
use super::{
    Hash, Id,
    live::{
        eligibility::{self, Refusal, Seat},
        source::Live,
    },
};
use crate::{
    action_identity::ActionHeader,
    hosted_server::{HostedServer, PreparedCarryPersonal},
    player_slot::PlayerSlot,
    progression::{GameplayBinding, GameplayId, GameplayRecord, install::PreparedProgressionInstall},
    protocol::PersonalAck,
};

const REFUSED: Refusal = Refusal("Current personal state couldn't be prepared without loss. Nothing was changed.");
const CHANGED: Refusal = Refusal("The current handover participants changed. Play has resumed.");

pub(crate) struct CutIdentity {
    transfer: Id,
    run: Id,
    scope: Option<GameplayId>,
    source: [u8; 32],
    seat: Seat,
    tick: u64,
    header: ActionHeader,
}
impl CutIdentity {
    pub(in crate::handover) fn checked(
        transfer: Id,
        hs: &HostedServer,
        live: Live,
        seat: Seat,
        tick: u64,
        scope: Option<GameplayId>,
    ) -> Result<Self, Refusal> {
        let source = live.signed_in.ok_or(CHANGED)?;
        if !hs.handover_frozen()
            || tick != hs.server.tick_counter
            || live.local_seats != 1
            || !hs.handover_seat_live(seat.slot, seat.generation, seat.key)
        {
            return Err(CHANGED);
        }
        eligibility::seats(&hs.server, seat)?;
        let sp = &hs.server.players[seat.slot];
        let header =
            sp.action_frontier.as_ref().ok_or(CHANGED)?.header(sp.action_frontier.as_ref().unwrap().through(), 0);
        Ok(Self { transfer, run: hs.server.run_id, scope, source, seat, tick, header })
    }
}
/// A current committed boundary, not an ACK or timer ledger.
pub(crate) struct CutTicket {
    identity: CutIdentity,
    native_binding: Option<GameplayBinding>,
    native_hash: Option<Hash>,
    remote_binding: Option<GameplayBinding>,
    remote_hash: Option<Hash>,
    expected: Option<PersonalAck>,
    settled: bool,
    started: u64,
    last_sent: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurrentPersonalProgress {
    Waiting,
    Settled,
}

/// Holding both live borrows makes invalidating the checked commit boundary
/// impossible between participant writes. Drop before commit abandons cleanly.
pub(crate) struct PreparedCarryCut<'a> {
    local: &'a mut PlayerSlot,
    hs: &'a mut HostedServer,
    ticket: Option<CutTicket>,
    native: Option<PreparedProgressionInstall>,
    remote_bytes: Option<Vec<u8>>,
    delivery: Option<PreparedCarryPersonal>,
}
fn expired(old: &crate::progression::QuestProgress, next: &crate::progression::QuestProgress) -> bool {
    old.declines.len() != next.declines.len() || (old.penalty_left > 0 && next.penalty_left == 0)
}
impl PreparedCarryCut<'_> {
    pub(crate) fn commit(mut self) -> (CutTicket, Option<Vec<u8>>) {
        let ticket = self.ticket.take().expect("prepared cut ticket");
        // No fallible work, transport callback or allocation between these
        // two SAME-holder commits. Every reservation was checked beforehand.
        if let Some(install) = self.native.take() {
            install.commit(self.local);
            self.local.gameplay_clock.committed(ticket.identity.tick);
        }
        let sp = &mut self.hs.server.players[ticket.identity.seat.slot];
        let output = match self.delivery.take() {
            Some(delivery) => Some(delivery.commit(
                sp,
                ticket.identity.tick,
                self.remote_bytes.take().expect("prepared remote encoding"),
            )),
            None => {
                if self.remote_bytes.take().is_some() {
                    sp.gameplay_clock.committed(ticket.identity.tick);
                }
                None
            }
        };
        // The caller stores this ticket before invoking any transport. This
        // ephemeral, preencoded output owns no second gameplay holder.
        (ticket, output)
    }
}
impl Drop for PreparedCarryCut<'_> {
    fn drop(&mut self) {
        if let Some(delivery) = self.delivery.take() {
            let slot = self.ticket.as_ref().expect("uncommitted cut ticket").identity.seat.slot;
            delivery.abandon(&mut self.hs.server.players[slot]);
        }
    }
}

pub(crate) fn prepare_cut<'a>(
    local: &'a mut PlayerSlot,
    hs: &'a mut HostedServer,
    identity: CutIdentity,
    now: u64,
) -> Result<PreparedCarryCut<'a>, Refusal> {
    eligibility::local_progress(local, identity.tick)?;
    eligibility::seats(&hs.server, identity.seat)?;
    let mut native = None;
    let mut native_hash = None;
    let mut retained = PreparedProgressionInstall::slot_owned_bytes(local);
    if let Some(binding) = &local.personal_binding {
        binding.validate().map_err(|_| REFUSED)?;
        if binding.context().participant() != Some(identity.source)
            || matches!(binding, GameplayBinding::WorldLocal { scope, .. } if Some(*scope) != identity.scope)
        {
            return Err(CHANGED);
        }
        let mut record = GameplayRecord::from_slot(binding.context(), local).map_err(|_| REFUSED)?;
        if expired(&local.quest_progress, &record.quests) {
            record.revision = record.revision.checked_add(1).ok_or(REFUSED)?;
        }
        let bytes = record.encode().map_err(|_| REFUSED)?;
        retained = retained.saturating_add(PreparedProgressionInstall::record_owned_bytes(&record));
        native_hash = Some(super::hash(&bytes));
        let install = PreparedProgressionInstall::prepare(record).map_err(|_| REFUSED)?;
        retained = retained.saturating_add(bytes.capacity()).saturating_add(install.owned_bytes());
        native = Some(install);
    }
    // An absent holder is not a current ACK or an AlreadyApplied proof.
    // Legacy low-level zero-state protocol fixtures retain their independent
    // offer path; the real two-participant cut requires actual current bodies.
    if local.personal_binding.is_none() && identity.scope.is_some() {
        return Err(REFUSED);
    }
    let sp = &hs.server.players[identity.seat.slot];
    if sp.personal.is_none() {
        return Err(REFUSED);
    }
    let mut remote_bytes = None;
    let mut remote_hash = None;
    let remote_binding = sp.personal.as_ref().map(|personal| personal.binding.clone());
    let mut expected = None;
    let mut changed = false;
    if let Some(personal) = &sp.personal {
        if personal.binding.context().participant() != Some(identity.seat.key) {
            return Err(CHANGED);
        }
        let record = personal.record().map_err(|_| REFUSED)?;
        let (mut projected, did_expire) = sp.gameplay_clock.project(&record, identity.tick);
        if did_expire {
            projected.revision = projected.revision.checked_add(1).ok_or(REFUSED)?;
        }
        let bytes = projected.encode().map_err(|_| REFUSED)?;
        retained = retained
            .saturating_add(PreparedProgressionInstall::record_owned_bytes(&record))
            .saturating_add(PreparedProgressionInstall::record_owned_bytes(&projected));
        changed = bytes != personal.bytes;
        if !changed && !sp.personal_delivery.current(personal, identity.header, &sp.window_events) {
            return Err(REFUSED);
        }
        remote_hash = Some(super::hash(&bytes));
        expected = sp.personal_delivery.last;
        retained = retained.saturating_add(personal.bytes.capacity()).saturating_add(bytes.capacity());
        remote_bytes = Some(bytes);
    }
    if retained > crate::action_journal::MAX_RETAINED_BYTES {
        return Err(REFUSED);
    }
    // This is the LAST fallible reservation. Its empty prefix/last number
    // is validated under the SAME held server borrow before native commit.
    let delivery = if changed {
        Some(
            hs.prepare_carry_personal(identity.seat.slot, remote_bytes.as_ref().unwrap(), retained)
                .map_err(|_| REFUSED)?,
        )
    } else {
        None
    };
    if let Some(delivery) = &delivery {
        expected = Some(delivery.ack());
    }
    let ready = delivery.as_ref().is_none_or(|delivery| delivery.ready(&hs.server.players[identity.seat.slot]));
    if !ready {
        if let Some(delivery) = delivery {
            delivery.abandon(&mut hs.server.players[identity.seat.slot]);
        }
        return Err(REFUSED);
    }
    let ticket = CutTicket {
        identity,
        native_binding: local.personal_binding.clone(),
        native_hash,
        remote_binding,
        remote_hash,
        expected,
        settled: false,
        started: now,
        last_sent: now,
    };
    Ok(PreparedCarryCut { local, hs, ticket: Some(ticket), native, remote_bytes, delivery })
}
impl CutTicket {
    pub(crate) fn check_deadline(&self, now: u64) -> Result<(), Refusal> {
        if now.saturating_sub(self.started) > 60 {
            Err(Refusal("Current personal state wasn't applied in time. Play has resumed."))
        } else {
            Ok(())
        }
    }
    pub(crate) fn retry_due(&self, now: u64) -> bool {
        !self.settled && now.saturating_sub(self.last_sent) >= 5
    }
    pub(crate) fn sent(&mut self, now: u64) {
        self.last_sent = now;
    }
    pub(crate) fn transfer(&self) -> Id {
        self.identity.transfer
    }
    pub(crate) fn seat(&self) -> Seat {
        self.identity.seat
    }
    pub(crate) fn expected(&self) -> Option<PersonalAck> {
        self.expected
    }
    pub(crate) fn settled(&self) -> bool {
        self.settled
    }
    pub(crate) fn check(
        &mut self,
        local: &PlayerSlot,
        hs: &HostedServer,
        live: Live,
        tick: u64,
        scope: Option<GameplayId>,
    ) -> Result<(), Refusal> {
        self.settled = false;
        let id = &self.identity;
        if hs.server.run_id != id.run
            || tick != id.tick
            || hs.server.tick_counter != id.tick
            || scope != id.scope
            || live.signed_in != Some(id.source)
            || live.local_seats != 1
            || !hs.handover_seat_live(id.seat.slot, id.seat.generation, id.seat.key)
            || local.personal_binding != self.native_binding
            || !local.gameplay_carry_complete()
        {
            return Err(CHANGED);
        }
        let sp = &hs.server.players[id.seat.slot];
        if !sp.progression_tracked
            || sp.personal_read_failed
            || sp.combat.dead
            || sp.attach_gen != id.seat.generation
            || sp.verified_pubkey != Some(id.seat.key)
            || sp.action_frontier.as_ref().map(|f| f.header(f.through(), 0)) != Some(id.header)
            || hs.server.gameplay_preparation.is_some()
            || sp.pack_flight.is_some()
            || sp.action_queue.as_ref().is_some_and(|q| !q.is_empty())
            || sp.action_results.as_ref().is_some_and(|r| !r.is_quiescent())
            || sp.personal.as_ref().map(|p| &p.binding) != self.remote_binding.as_ref()
            || sp.personal.as_ref().map(|p| super::hash(&p.bytes)) != self.remote_hash
        {
            return Err(CHANGED);
        }
        if let Some(binding) = &self.native_binding {
            let bytes = GameplayRecord::from_slot(binding.context(), local)
                .and_then(|record| record.encode())
                .map_err(|_| CHANGED)?;
            if Some(super::hash(&bytes)) != self.native_hash {
                return Err(CHANGED);
            }
        }
        eligibility::local_progress(local, tick)?;
        Ok(())
    }
    pub(crate) fn confirm(
        &mut self,
        local: &PlayerSlot,
        hs: &HostedServer,
        live: Live,
        tick: u64,
        scope: Option<GameplayId>,
        now: u64,
    ) -> Result<CurrentPersonalProgress, Refusal> {
        // A previous success is not a permanent capability if any current
        // proof changes. Waiting must never leave capture enabled.
        self.settled = false;
        self.check(local, hs, live, tick, scope)?;
        self.check_deadline(now)?;
        let sp = &hs.server.players[self.identity.seat.slot];
        if let Some(personal) = &sp.personal {
            if sp.personal_delivery.last != self.expected
                || !sp.personal_delivery.current(personal, self.identity.header, &sp.window_events)
            {
                return Ok(CurrentPersonalProgress::Waiting);
            }
        } else if self.expected.is_some() {
            return Err(CHANGED);
        }
        eligibility::seats(&hs.server, self.identity.seat)?;
        self.settled = true;
        Ok(CurrentPersonalProgress::Settled)
    }
}

#[cfg(test)]
mod tests;
