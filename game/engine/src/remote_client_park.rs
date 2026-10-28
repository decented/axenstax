//! Normal client remains the only reader until the checked ordered fence.
use super::*;
use crate::handover::{
    Id,
    live::{
        park::{Fence, WIRE_CAP},
        wire::Frame,
    },
    phase::Authority,
};
use std::mem::size_of;

pub(super) struct Drain {
    transfer: Id,
    authority: Authority,
    participant: [u8; 32],
    runtime: [u8; 32],
    life: u32,
    fence: Option<Fence>,
    world_pending: bool,
    failure: Option<&'static str>,
    read_credit: bool,
}

impl RemoteClient {
    pub(crate) fn begin_handover_drain(
        &mut self,
        transfer: Id,
        authority: Authority,
        participant: [u8; 32],
        runtime: [u8; 32],
        world_pending: bool,
    ) -> Result<(), &'static str> {
        if transfer == [0; 16]
            || self.parked
            || self.handover_drain.is_some()
            || !self.is_connected()
            || self.transport.is_closed()
            || !self.handover_authority_settled()
        {
            return Err("This joined game is not ready for handover.");
        }
        self.handover_drain = Some(Drain {
            transfer,
            authority,
            participant,
            runtime,
            life: self.actor_respawns,
            fence: None,
            world_pending,
            failure: None,
            read_credit: true,
        });
        Ok(())
    }

    pub(crate) fn cancel_handover_drain(&mut self, transfer: Id) {
        if self
            .handover_drain
            .as_ref()
            .is_some_and(|d| d.transfer == transfer)
        {
            // Fixed control metadata only. Admitted work and raw FIFO stay owned.
            self.handover_drain = None;
        }
    }

    pub(crate) fn handover_world_pending(&mut self, pending: bool) {
        if let Some(d) = &mut self.handover_drain {
            d.world_pending = pending;
        }
    }

    pub(super) fn park_poll_can_read(&self, consumed: usize) -> bool {
        self.handover_drain.as_ref().is_none_or(|d| {
            consumed == 0
                && d.read_credit
                && !d.world_pending
                && d.fence.is_none()
                && d.failure.is_none()
        })
    }

    pub(super) fn handover_carrier_read(&mut self) {
        if let Some(d) = &mut self.handover_drain {
            d.read_credit = false;
        }
    }

    pub(crate) fn handover_application_completed(&mut self, world_pending: bool) {
        if let Some(d) = &mut self.handover_drain {
            d.world_pending = world_pending;
            // Another raw carrier needs an actual ordinary application pass,
            // even if another caller polls the same client in this frame.
            d.read_credit = !world_pending && d.fence.is_none() && d.failure.is_none();
        }
    }

    pub(super) fn receive_park_fence(
        &mut self,
        payload: &[u8],
        capacity: usize,
    ) -> Result<bool, &'static str> {
        let Some(d) = &mut self.handover_drain else {
            return Ok(false);
        };
        // Only the new fixed variant is decoded here. Other controls retain
        // their existing single inbox path, without copying archived bodies.
        if !Frame::is_freeze_fence(payload) {
            return Ok(false);
        }
        crate::handover::live::park::check_incoming_control(payload, capacity).map_err(|r| r.0)?;
        let Frame::FreezeFence { fence } =
            Frame::decode(payload).map_err(|_| "The host sent an invalid handover fence.")?
        else {
            return Err("The host sent an invalid handover fence.");
        };
        fence
            .check(d.transfer, &d.authority, d.participant, d.runtime)
            .map_err(|r| r.0)?;
        if !self.replay.is_empty() {
            return Err("Gameplay is still waiting after the handover fence. Try again.");
        }
        d.fence = Some(fence);
        Ok(true)
    }

    pub(super) fn refuse_park_drain(
        &mut self,
        why: &'static str,
        packet: crate::transport::Packet,
    ) {
        if let Some(d) = &mut self.handover_drain {
            d.failure = Some(why);
        }
        if self.replay.try_reserve_exact(1).is_err() {
            self.state =
                ConnectionState::Failed("The handover replay could not be retained.".into());
            return;
        }
        self.replay.push_front(packet);
    }

    pub(crate) fn handover_drain_failure(&self, transfer: Id) -> Option<&'static str> {
        self.handover_drain
            .as_ref()
            .filter(|d| d.transfer == transfer)
            .and_then(|d| d.failure)
    }

    pub(crate) fn handover_drain_active(&self, transfer: Id) -> bool {
        self.handover_drain
            .as_ref()
            .is_some_and(|d| d.transfer == transfer)
    }

    pub(crate) fn handover_drain_live(&self, transfer: Id) -> bool {
        self.handover_drain.as_ref().is_some_and(|drain| {
            drain.transfer == transfer
                && drain.life == self.actor_respawns
                && drain.life == self.received_actor_respawns
        }) && self.is_connected()
            && !self.transport.is_closed()
    }

    pub(crate) fn handover_drain_fixed_bytes(&self) -> usize {
        size_of::<Option<Drain>>()
    }

    pub(crate) fn handover_drain_fence(&self, transfer: Id) -> Option<Fence> {
        self.handover_drain
            .as_ref()
            .filter(|d| d.transfer == transfer && d.failure.is_none())
            .and_then(|d| d.fence)
    }

    pub(crate) fn handover_drain_application_ready(&self, transfer: Id) -> bool {
        self.handover_drain.as_ref().is_some_and(|d| {
            d.transfer == transfer && d.fence.is_some() && !d.world_pending && d.failure.is_none()
        }) && self.personal_control_quiescent()
            && self.latest_state.is_none()
            && self.undecodable_chunks == 0
            && self.pending_refused_uses.is_empty()
            && self.pending_play_mode.is_none()
            && self.pending_difficulty.is_none()
            && self.pending_world_time.is_none()
            && self.pending_exhibits.is_none()
            && self.pending_resource_pack.is_none()
            && self.pending_operator_snapshot_json.is_none()
            && self.pending_chat.is_empty()
    }
}

const _: () = assert!(size_of::<Option<Drain>>() <= WIRE_CAP);
