//! SAME normal/parked Personal intake; parked output is typed application only.
#[cfg(not(target_arch = "wasm32"))]
use super::ConnectionState;
use super::{RemoteClient, RequestOutcome};
use crate::{
    personal_inbox::{AppliedPersonalBatch, PersonalAdmission},
    protocol::{self, PersonalSnapshot, PersonalSyncPacket},
};

fn decode(payload: &[u8]) -> Result<PersonalSnapshot, String> {
    if payload.len() > protocol::MAX_PACKET_SIZE as usize {
        return Err("Personal packet exceeds its wire limit".into());
    }
    match protocol::safe_deserialize::<PersonalSyncPacket>(payload).map_err(|e| e.to_string())? {
        PersonalSyncPacket::Snapshot(value) => Ok(value),
        PersonalSyncPacket::Applied(_) => Err("Unexpected personal applied packet".into()),
    }
}
fn budget(usage: (usize, usize)) -> Result<(), String> {
    if usage.0 > crate::action_journal::MAX_EVENT_ENTRIES
        || usage.1 > crate::action_journal::MAX_RETAINED_BYTES
    {
        return Err("Personal control exceeds its retained limit".into());
    }
    Ok(())
}
fn add(a: (usize, usize), b: (usize, usize)) -> (usize, usize) {
    (a.0.saturating_add(b.0), a.1.saturating_add(b.1))
}
impl RemoteClient {
    /// Actual queued carriers stay charged when abort moves the raw FIFO
    /// back from RecipientSession into this client's normal replay queue.
    pub(crate) fn personal_control_retained_usage(&self) -> (usize, usize) {
        use std::mem::size_of;
        let retained = self.pack_retained_usage();
        let count = self
            .pending_outcomes
            .len()
            .saturating_add(self.pending_grants.len())
            .saturating_add(self.pending_life_events.len())
            .saturating_add(self.pending_kills.len())
            .saturating_add(self.pending_block_changes.len())
            .saturating_add(self.pending_entity_batches.len())
            .saturating_add(self.replay.len());
        let mut bytes = crate::window_events::window_carrier_bytes(
            &self.pending_outcomes,
            &self.pending_grants,
            0,
        )
        .saturating_add(self.pending_life_events.capacity() * size_of::<super::OwnLifeEvent>())
        .saturating_add(self.pending_kills.capacity() * size_of::<protocol::KillEventPacket>())
        .saturating_add(self.pending_block_changes.capacity() * size_of::<protocol::BlockChange>())
        .saturating_add(
            self.pending_entity_batches.capacity()
                * size_of::<crate::remote_entities::EntityDeltas>(),
        )
        .saturating_add(self.replay.capacity() * size_of::<Vec<u8>>());
        for batch in &self.pending_entity_batches {
            bytes = bytes
                .saturating_add(batch.spawns.capacity() * size_of::<protocol::EntitySpawn>())
                .saturating_add(batch.updates.capacity() * size_of::<protocol::EntityUpdate>())
                .saturating_add(batch.despawns.capacity() * size_of::<u32>());
        }
        for packet in &self.replay {
            bytes = bytes.saturating_add(packet.capacity());
        }
        add(retained, (count, bytes))
    }
    pub(super) fn receive_personal_normal(
        &mut self,
        payload: &[u8],
        raw_capacity: usize,
    ) -> Result<(bool, Option<AppliedPersonalBatch>), String> {
        let value = decode(payload)?;
        // Reserve the real normal destination BEFORE publishing a body.
        self.pending_outcomes
            .try_reserve_exact(1)
            .map_err(|_| "Personal fence allocation failed")?;
        let retained = self.personal_control_retained_usage();
        let own = self
            .personal_inbox
            .as_ref()
            .ok_or("Unexpected personal snapshot")?;
        let external = (
            retained.0.saturating_sub(own.pending_count()),
            retained
                .1
                .saturating_sub(own.owned_bytes())
                .saturating_add(raw_capacity),
        );
        let mut push = self
            .personal_inbox
            .as_mut()
            .expect("checked Inbox")
            .prepare_push_bounded(value, external)?;
        budget(add(external, push.retained_usage()))?;
        push.fence_identity(self.events_applied)?;
        match push.commit() {
            PersonalAdmission::Admitted(proof) => {
                self.pending_outcomes.push(RequestOutcome::PersonalSync {
                    window_event: proof.identity().event(),
                    actor_respawns: proof.identity().life(),
                });
                Ok((true, None))
            }
            PersonalAdmission::DuplicateApplied(batch) => Ok((false, Some(batch))),
            PersonalAdmission::Ignored => Ok((false, None)),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod parked {
    use super::*;
    use crate::{
        personal_inbox::{PersonalFenceIdentity, PreparedInboxApply},
        player_slot::PlayerSlot,
        transport::ClientTransport,
        window_events::{WindowInbox, personal_prefix::PreparedPersonalPrefix},
    };

    pub(crate) struct PersonalParkBinding {
        transfer: crate::handover::Id,
        player_index: u32,
        identity: PersonalFenceIdentity,
    }
    pub(crate) struct ParkedPersonalLink {
        transport: Box<dyn ClientTransport>,
        binding: PersonalParkBinding,
    }
    impl ParkedPersonalLink {
        pub(crate) fn binding(&self) -> &PersonalParkBinding {
            &self.binding
        }
        pub(crate) fn is_closed(&self) -> bool {
            self.transport.is_closed()
        }
        pub(crate) fn into_parts(self) -> (Box<dyn ClientTransport>, PersonalParkBinding) {
            (self.transport, self.binding)
        }
    }
    impl PersonalParkBinding {
        pub(crate) fn transfer(&self) -> crate::handover::Id {
            self.transfer
        }
        pub(crate) fn participant(&self) -> Option<[u8; 32]> {
            self.identity.participant()
        }
        pub(crate) fn matches(&self, other: &Self) -> bool {
            self.transfer == other.transfer
                && self.player_index == other.player_index
                && self.identity.same_attachment(&other.identity)
        }
    }
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum PersonalControlProgress {
        Idle,
        Waiting,
        Applied { through: u32, count: usize },
    }

    pub(crate) struct PreparedParkedApply<'r, 's, 'w> {
        frontier: &'r mut u32,
        prefix: PreparedPersonalPrefix<'w>,
        application: PreparedInboxApply<'r, 's>,
        through: u32,
    }
    impl PreparedParkedApply<'_, '_, '_> {
        pub(crate) fn commit(self) -> AppliedPersonalBatch {
            self.prefix.commit();
            let batch = self.application.commit();
            *self.frontier = self.through;
            batch
        }
    }
    impl RemoteClient {
        pub(crate) fn park_for_personal_control(
            &mut self,
            transfer: crate::handover::Id,
        ) -> Result<ParkedPersonalLink, String> {
            if transfer == [0; 16]
                || self.parked
                || !self.handover_authority_settled()
                || self.transport.is_closed()
                || !self.personal_control_quiescent()
            {
                return Err("Personal handover boundary is not settled".into());
            }
            let ConnectionState::Connected { player_index, .. } = self.state else {
                return Err("Personal handover needs the joined client".into());
            };
            let inbox = self
                .personal_inbox
                .as_ref()
                .ok_or("Personal handover has no Inbox")?;
            if inbox.pending_count() != 0 || inbox.life != self.actor_respawns {
                return Err("Personal handover has pending state or life".into());
            }
            let identity = inbox
                .installed_identity(self.events_applied)
                .ok_or("Personal handover is not initialised")?;
            let binding = PersonalParkBinding {
                transfer,
                player_index,
                identity: identity.clone(),
            };
            self.personal_park = Some(PersonalParkBinding {
                transfer,
                player_index,
                identity,
            });
            Ok(ParkedPersonalLink {
                transport: self.park_transport(),
                binding,
            })
        }
        fn personal_control_quiescent(&self) -> bool {
            self.handover_authority_settled()
                && !self.pack_request_pending()
                && !self.pack_buffers_live()
                && !self.respawn_unanswered()
                && self.received_actor_respawns == self.actor_respawns
                && self.pending_outcomes.is_empty()
                && self.pending_grants.is_empty()
                && self.pending_life_events.is_empty()
                && self.pending_kills.is_empty()
                && self.pending_block_changes.is_empty()
                && self.pending_entity_batches.is_empty()
                && self.chunk_queue.is_empty()
                && self.replay.is_empty()
        }
        pub(crate) fn validate_personal_control(
            &self,
            binding: &PersonalParkBinding,
        ) -> Result<(), String> {
            if !self.parked
                || !self.personal_control_quiescent()
                || self
                    .personal_park
                    .as_ref()
                    .is_none_or(|current| !current.matches(binding))
            {
                return Err("Parked personal client changed or ordinary work is waiting".into());
            }
            let ConnectionState::Connected { player_index, .. } = self.state else {
                return Err("Parked personal client is no longer joined".into());
            };
            let inbox = self
                .personal_inbox
                .as_ref()
                .ok_or("Parked personal Inbox vanished")?;
            let current = inbox
                .installed_identity(self.events_applied)
                .ok_or("Parked personal state is not initialised")?;
            if player_index != binding.player_index
                || inbox.life != self.actor_respawns
                || !binding.identity.same_attachment(&current)
            {
                return Err("Parked personal attachment changed".into());
            }
            Ok(())
        }
        pub(crate) fn personal_batch_matches(&self, batch: &AppliedPersonalBatch) -> bool {
            self.personal_park
                .as_ref()
                .is_some_and(|binding| binding.identity.same_attachment(batch.identity()))
                && self
                    .personal_inbox
                    .as_ref()
                    .is_some_and(|inbox| batch.matches_inbox(inbox))
        }
        pub(crate) fn receive_personal_parked(
            &mut self,
            payload: &[u8],
            window: &mut WindowInbox,
            external: (usize, usize),
            allow_new: bool,
        ) -> Result<Option<AppliedPersonalBatch>, String> {
            let binding = self
                .personal_park
                .as_ref()
                .ok_or("Missing parked personal binding")?;
            self.validate_personal_control(binding)?;
            let value = decode(payload)?;
            if value.life != self.actor_respawns {
                return Err("Personal control cannot cross a physical life boundary".into());
            }
            let retained = self.personal_control_retained_usage();
            let own = self
                .personal_inbox
                .as_ref()
                .ok_or("Parked personal Inbox vanished")?;
            let external = add(
                external,
                add(
                    window.retained_usage(),
                    (
                        retained.0.saturating_sub(own.pending_count()),
                        retained.1.saturating_sub(own.owned_bytes()),
                    ),
                ),
            );
            let mut push = self
                .personal_inbox
                .as_mut()
                .expect("checked Inbox")
                .prepare_push_bounded(value, external)?;
            budget(add(external, push.retained_usage()))?;
            if let Some(identity) = push.fence_identity(self.events_applied)? {
                if !allow_new {
                    return Err("Personal state arrived after handover capture began".into());
                }
                let combined = add(external, push.retained_usage());
                let window_usage = window.retained_usage();
                let marker_external = (
                    combined.0.saturating_sub(window_usage.0),
                    combined.1.saturating_sub(window_usage.1),
                );
                let marker = window.prepare_personal_fence(&identity, marker_external)?;
                if !marker.identity().same_snapshot(&identity) {
                    return Err("Personal marker reservation changed identity".into());
                }
                marker.pair(push)?.commit();
                Ok(None)
            } else {
                Ok(match push.commit() {
                    PersonalAdmission::DuplicateApplied(batch) => Some(batch),
                    PersonalAdmission::Ignored => None,
                    PersonalAdmission::Admitted(_) => unreachable!("no proposed fence"),
                })
            }
        }
        pub(crate) fn prepare_personal_apply<'r, 's, 'w>(
            &'r mut self,
            slot: &'s mut PlayerSlot,
            prefix: PreparedPersonalPrefix<'w>,
            external: (usize, usize),
        ) -> Result<PreparedParkedApply<'r, 's, 'w>, String> {
            let binding = self
                .personal_park
                .as_ref()
                .ok_or("Missing parked personal binding")?;
            self.validate_personal_control(binding)?;
            let inbox = self
                .personal_inbox
                .as_ref()
                .ok_or("Missing parked personal Inbox")?;
            if slot.index != 0
                || prefix.base() != self.events_applied
                || prefix.life() != self.actor_respawns
                || prefix.identities().is_empty()
                || prefix.identities().iter().any(|identity| {
                    !identity.matches_inbox(inbox, self.events_applied)
                        || !identity.same_attachment(&binding.identity)
                })
            {
                return Err("Personal prefix does not belong to the current joined holder".into());
            }
            let through = prefix.through();
            let retained = self.personal_control_retained_usage();
            let external = add(
                external,
                (
                    retained
                        .0
                        .saturating_sub(inbox.pending_count())
                        .saturating_add(prefix.identities().len()),
                    retained
                        .1
                        .saturating_sub(inbox.owned_bytes())
                        .saturating_add(prefix.owned_bytes()),
                ),
            );
            let application = self
                .personal_inbox
                .as_mut()
                .expect("checked Inbox")
                .prepare_apply(through, slot, external)?;
            if application.count() != prefix.identities().len() {
                return Err("Personal prefix/body count differs".into());
            }
            Ok(PreparedParkedApply {
                frontier: &mut self.events_applied,
                prefix,
                application,
                through,
            })
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use parked::{ParkedPersonalLink, PersonalControlProgress, PersonalParkBinding};
#[cfg(test)]
#[path = "remote_client_personal_control/tests.rs"]
mod tests;
