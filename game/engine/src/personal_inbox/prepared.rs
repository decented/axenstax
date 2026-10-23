//! Shared normal/parked preparation. No pending body is popped before decoding.
use super::{Inbox, PersonalAck, PersonalSnapshot};
use crate::{
    player_slot::PlayerSlot,
    progression::{GameplayContext, GameplayRecord, install::PreparedProgressionInstall},
};
use std::mem::size_of;
#[cfg(any(test, not(target_arch = "wasm32")))]
use std::sync::Arc;

#[cfg(any(test, not(target_arch = "wasm32")))]
#[derive(Clone, Debug)]
struct Attachment {
    origin: Arc<()>,
    session: [u8; 16],
    context: Option<GameplayContext>,
    life: u32,
    observed_life: u32,
}
/// A sealed identity, never an authority to acknowledge application.
#[derive(Clone, Debug)]
pub(crate) struct PersonalFenceIdentity {
    #[cfg(any(test, not(target_arch = "wasm32")))]
    attachment: Attachment,
    base: u32,
    ack: PersonalAck,
}
impl PersonalFenceIdentity {
    pub(crate) fn event(&self) -> u32 {
        self.ack.window_event
    }
    pub(crate) fn life(&self) -> u32 {
        self.ack.life
    }
    pub(crate) fn base(&self) -> u32 {
        self.base
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn same_namespace(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.attachment.origin, &other.attachment.origin)
            && self.attachment.session == other.attachment.session
            && self.attachment.context == other.attachment.context
            && self.attachment.life == other.attachment.life
            && self.attachment.observed_life == other.attachment.observed_life
            && self.base == other.base
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn same_snapshot(&self, other: &Self) -> bool {
        self.same_namespace(other) && self.ack == other.ack
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn matches_inbox(&self, inbox: &Inbox, base: u32) -> bool {
        self.base == base
            && inbox
                .pending_fence_identity(self.event(), self.ack.life, base)
                .is_some_and(|actual| self.same_snapshot(&actual))
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn participant(&self) -> Option<[u8; 32]> {
        self.attachment
            .context
            .and_then(|context| context.participant())
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn same_attachment(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.attachment.origin, &other.attachment.origin)
            && self.attachment.session == other.attachment.session
            && self.attachment.context == other.attachment.context
            && self.attachment.life == other.attachment.life
            && self.attachment.observed_life == other.attachment.observed_life
    }
}

pub(crate) struct AdmittedPersonalFence {
    identity: PersonalFenceIdentity,
}
impl AdmittedPersonalFence {
    pub(crate) fn identity(&self) -> &PersonalFenceIdentity {
        &self.identity
    }
}
pub(crate) enum PersonalAdmission {
    Admitted(AdmittedPersonalFence),
    DuplicateApplied(AppliedPersonalBatch),
    Ignored,
}

struct AppliedAck {
    ack: PersonalAck,
    wire: Vec<u8>,
}
/// Created only by committed SAME-Inbox application or its exact installed duplicate.
pub(crate) struct AppliedPersonalBatch {
    identity: PersonalFenceIdentity,
    values: Vec<AppliedAck>,
}
impl AppliedPersonalBatch {
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn identity(&self) -> &PersonalFenceIdentity {
        &self.identity
    }
    #[cfg(test)]
    pub(crate) fn acks(&self) -> impl Iterator<Item = PersonalAck> + '_ {
        self.values.iter().map(|value| value.ack)
    }
    pub(crate) fn wires(&self) -> impl Iterator<Item = &[u8]> {
        self.values.iter().map(|value| value.wire.as_slice())
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn count(&self) -> usize {
        self.values.len()
    }
    pub(crate) fn last_ack(&self) -> Option<PersonalAck> {
        self.values.last().map(|v| v.ack)
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn matches_inbox(&self, inbox: &Inbox) -> bool {
        inbox
            .installed_identity(self.identity.base)
            .is_some_and(|current| current.same_attachment(&self.identity))
            && self.last_ack() == inbox.last
    }
    pub(crate) fn owned_bytes(&self) -> usize {
        std::mem::size_of_val(&self.identity).saturating_add(
            self.values
                .capacity()
                .saturating_mul(size_of::<AppliedAck>())
                .saturating_add(self.values.iter().map(|v| v.wire.capacity()).sum::<usize>()),
        )
    }
}

fn wire(ack: PersonalAck) -> Vec<u8> {
    crate::protocol::serialize_packet(
        crate::protocol::PacketType::PersonalSync,
        &crate::protocol::PersonalSyncPacket::Applied(ack),
    )
}
fn check_usage(count: usize, bytes: usize) -> Result<(), String> {
    if count > crate::action_journal::MAX_EVENT_ENTRIES
        || bytes > crate::action_journal::MAX_RETAINED_BYTES
    {
        Err("Personal preparation exceeds its retained limit".into())
    } else {
        Ok(())
    }
}
/// Include nested reward vector allocations; accepted rewards contain no private bodies.
fn decoded_bytes(record: &GameplayRecord) -> usize {
    record.reputation.capacity() * size_of::<((i32, i32), i16)>()
        + record.kills.capacity() * size_of::<(crate::mob::MobType, u32)>()
        + record.claims.capacity() * size_of::<(u32, u32)>()
        + record.quests.accepted.capacity() * size_of::<crate::progression::AcceptedQuest>()
        + record.quests.declines.capacity() * size_of::<([u8; 16], u32)>()
        + record
            .quests
            .accepted
            .iter()
            .map(|q| q.rewards.capacity() * size_of::<crate::saved_item_slot::SavedItemSlot>())
            .sum::<usize>()
}

enum PushKind {
    Pending(PersonalSnapshot),
    Duplicate(AppliedPersonalBatch),
    Ignored,
}
pub(crate) struct PreparedPersonalPush<'a> {
    inbox: &'a mut Inbox,
    identity: Option<PersonalFenceIdentity>,
    kind: PushKind,
    bound_base: Option<u32>,
}
impl PreparedPersonalPush<'_> {
    pub(crate) fn fence_identity(
        &mut self,
        base: u32,
    ) -> Result<Option<PersonalFenceIdentity>, String> {
        if self.bound_base.is_some_and(|current| current != base)
            || (self.bound_base.is_some()
                && self
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.base() != base))
        {
            return Err("Personal admission was rebound to another frontier".into());
        }
        self.bound_base = Some(base);
        if let Some(identity) = &mut self.identity {
            identity.base = base;
        }
        Ok(self.identity.clone())
    }
    /// Actual newly owned body and queue capacity, for joint marker admission.
    pub(crate) fn retained_usage(&self) -> (usize, usize) {
        let body = match &self.kind {
            PushKind::Pending(value) => value.bytes.capacity(),
            _ => 0,
        };
        let duplicate = match &self.kind {
            PushKind::Duplicate(batch) => batch.owned_bytes(),
            _ => 0,
        };
        (
            self.inbox.pending_count() + usize::from(matches!(&self.kind, PushKind::Pending(_))),
            self.inbox
                .owned_bytes()
                .saturating_add(body)
                .saturating_add(duplicate),
        )
    }
    pub(crate) fn commit(self) -> PersonalAdmission {
        match self.kind {
            PushKind::Pending(value) => {
                self.inbox.context = Some(value.context);
                self.inbox.pending.push_back(value);
                PersonalAdmission::Admitted(AdmittedPersonalFence {
                    identity: self.identity.expect("prepared pending identity"),
                })
            }
            PushKind::Duplicate(batch) => PersonalAdmission::DuplicateApplied(batch),
            PushKind::Ignored => PersonalAdmission::Ignored,
        }
    }
}

pub(crate) struct PreparedInboxApply<'i, 's> {
    inbox: &'i mut Inbox,
    slot: &'s mut PlayerSlot,
    installs: Vec<PreparedProgressionInstall>,
    batch: AppliedPersonalBatch,
    remove: usize,
}
impl PreparedInboxApply<'_, '_> {
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn count(&self) -> usize {
        self.installs.len()
    }
    pub(crate) fn commit(self) -> AppliedPersonalBatch {
        for install in self.installs {
            install.commit(self.slot);
        }
        for _ in 0..self.remove {
            self.inbox.pending.pop_front();
        }
        if let Some(ack) = self.batch.last_ack() {
            self.inbox.last = Some(ack);
        }
        self.batch
    }
}

impl Inbox {
    fn identity(&self, ack: PersonalAck, base: u32) -> PersonalFenceIdentity {
        PersonalFenceIdentity {
            #[cfg(any(test, not(target_arch = "wasm32")))]
            attachment: Attachment {
                origin: self.origin.clone(),
                session: self.session,
                context: self.context,
                life: self.life,
                observed_life: self.observed_life,
            },
            base,
            ack,
        }
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn installed_identity(&self, base: u32) -> Option<PersonalFenceIdentity> {
        self.last.map(|ack| self.identity(ack, base))
    }
    #[cfg(any(test, not(target_arch = "wasm32")))]
    pub(crate) fn pending_fence_identity(
        &self,
        event: u32,
        life: u32,
        base: u32,
    ) -> Option<PersonalFenceIdentity> {
        self.pending
            .iter()
            .find(|value| value.life == life && value.window_event == event)
            .map(|value| self.identity(super::acknowledgement(value), base))
    }
    #[cfg(test)]
    pub(crate) fn prepare_push(
        &mut self,
        value: PersonalSnapshot,
    ) -> Result<PreparedPersonalPush<'_>, String> {
        self.prepare_push_bounded(value, (0, 0))
    }
    pub(crate) fn prepare_push_bounded(
        &mut self,
        value: PersonalSnapshot,
        external: (usize, usize),
    ) -> Result<PreparedPersonalPush<'_>, String> {
        if value.session != self.session
            || value.life > self.observed_life
            || value.revision == 0
            || value.window_event == 0
        {
            return Err("foreign or future personal snapshot".into());
        }
        if value.life < self.life {
            return Ok(PreparedPersonalPush {
                bound_base: None,
                inbox: self,
                identity: None,
                kind: PushKind::Ignored,
            });
        }
        let decoded = GameplayRecord::decode(&value.bytes, value.context)?;
        check_usage(
            external
                .0
                .saturating_add(self.pending_count())
                .saturating_add(1),
            external
                .1
                .saturating_add(self.owned_bytes())
                .saturating_add(value.bytes.capacity())
                .saturating_add(decoded_bytes(&decoded)),
        )?;
        drop(decoded);
        if matches!(value.context, GameplayContext::Guest { session, .. } if session != self.session)
        {
            return Err("foreign guest progression attachment".into());
        }
        if self.context.is_some_and(|context| context != value.context) {
            return Err("personal snapshot identity changed".into());
        }
        let ack = super::acknowledgement(&value);
        if let Some(last) = self.last
            && value.revision <= last.revision
        {
            let kind = if ack == last {
                let mut values = Vec::new();
                values
                    .try_reserve_exact(1)
                    .map_err(|_| "Personal ACK allocation failed")?;
                values.push(AppliedAck {
                    ack,
                    wire: wire(ack),
                });
                let batch = AppliedPersonalBatch {
                    identity: self.identity(ack, 0),
                    values,
                };
                check_usage(
                    external.0.saturating_add(self.pending_count()),
                    external
                        .1
                        .saturating_add(self.owned_bytes())
                        .saturating_add(value.bytes.capacity())
                        .saturating_add(batch.owned_bytes()),
                )?;
                PushKind::Duplicate(batch)
            } else {
                PushKind::Ignored
            };
            return Ok(PreparedPersonalPush {
                bound_base: None,
                inbox: self,
                identity: None,
                kind,
            });
        }
        if self
            .pending
            .iter()
            .any(|snapshot| super::acknowledgement(snapshot) == ack)
        {
            return Ok(PreparedPersonalPush {
                bound_base: None,
                inbox: self,
                identity: None,
                kind: PushKind::Ignored,
            });
        }
        if self.pending.len() >= 8 {
            return Err("personal snapshot queue full".into());
        }
        if self
            .pending
            .back()
            .is_some_and(|snapshot| value.revision <= snapshot.revision)
        {
            return Err("personal snapshot order changed".into());
        }
        check_usage(
            external
                .0
                .saturating_add(self.pending_count())
                .saturating_add(1),
            external
                .1
                .saturating_add(self.owned_bytes())
                .saturating_add(value.bytes.capacity())
                .saturating_add(size_of::<PersonalSnapshot>()),
        )?;
        self.pending
            .try_reserve_exact(1)
            .map_err(|_| "Personal snapshot allocation failed")?;
        check_usage(
            external
                .0
                .saturating_add(self.pending_count())
                .saturating_add(1),
            external
                .1
                .saturating_add(self.owned_bytes())
                .saturating_add(value.bytes.capacity()),
        )?;
        let identity = self.identity(ack, 0);
        #[cfg(any(test, not(target_arch = "wasm32")))]
        let identity = {
            let mut identity = identity;
            identity.attachment.context = Some(value.context);
            identity
        };
        Ok(PreparedPersonalPush {
            bound_base: None,
            inbox: self,
            identity: Some(identity),
            kind: PushKind::Pending(value),
        })
    }
    pub(crate) fn prepare_apply<'i, 's>(
        &'i mut self,
        events: u32,
        slot: &'s mut PlayerSlot,
        external: (usize, usize),
    ) -> Result<PreparedInboxApply<'i, 's>, String> {
        let identity = self
            .last
            .map(|ack| self.identity(ack, events))
            .or_else(|| {
                self.pending
                    .front()
                    .map(|value| self.identity(super::acknowledgement(value), events))
            })
            .ok_or("No personal snapshot to apply")?;
        let mut installs = Vec::new();
        let mut values = Vec::new();
        installs
            .try_reserve_exact(self.pending.len())
            .map_err(|_| "Personal install allocation failed")?;
        values
            .try_reserve_exact(self.pending.len())
            .map_err(|_| "Personal ACK allocation failed")?;
        let mut batch = AppliedPersonalBatch { identity, values };
        let base_bytes = external
            .1
            .saturating_add(self.owned_bytes())
            .saturating_add(PreparedProgressionInstall::slot_owned_bytes(slot))
            .saturating_add(installs.capacity() * size_of::<PreparedProgressionInstall>());
        check_usage(
            external.0.saturating_add(self.pending_count()),
            base_bytes.saturating_add(batch.owned_bytes()),
        )?;
        let mut staged = 0usize;
        let mut remove = 0;
        for value in &self.pending {
            if value.life < self.life {
                remove += 1;
                continue;
            }
            if value.life != self.life || value.window_event > events {
                break;
            }
            let record = GameplayRecord::decode(&value.bytes, value.context)?;
            let decoded = decoded_bytes(&record);
            check_usage(
                external.0.saturating_add(self.pending_count()),
                base_bytes
                    .saturating_add(staged)
                    .saturating_add(decoded)
                    .saturating_add(batch.owned_bytes()),
            )?;
            let install = PreparedProgressionInstall::prepare(record)?;
            // Conservatively retain the pre-conversion decoded charge whilst checking peak overlap.
            let install_bytes = install
                .owned_bytes()
                .saturating_sub(size_of::<PreparedProgressionInstall>());
            let ack = super::acknowledgement(value);
            let encoded = wire(ack);
            check_usage(
                external.0.saturating_add(self.pending_count()),
                base_bytes
                    .saturating_add(staged)
                    .saturating_add(decoded)
                    .saturating_add(install_bytes)
                    .saturating_add(encoded.capacity())
                    .saturating_add(batch.owned_bytes()),
            )?;
            staged = staged.saturating_add(install_bytes);
            installs.push(install);
            batch.values.push(AppliedAck { ack, wire: encoded });
            remove += 1;
        }
        Ok(PreparedInboxApply {
            inbox: self,
            slot,
            installs,
            batch,
            remove,
        })
    }
}
