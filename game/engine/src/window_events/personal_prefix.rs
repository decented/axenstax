//! Reserved zero-effect Personal markers. No ordinary window work is drained.
use super::{RequestOutcome, WindowInbox};
use crate::personal_inbox::{Inbox, PersonalAdmission, PersonalFenceIdentity, PreparedPersonalPush};
use std::{fmt, mem::size_of};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PersonalPrefixBlocked {
    Empty,
    Ordinary,
    Gap,
    Life,
    Missing,
    TooLarge,
    Exhausted,
}
impl fmt::Display for PersonalPrefixBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Personal-only prefix waits: {self:?}")
    }
}
pub(crate) struct PreparedPersonalFence<'w> {
    window: &'w mut WindowInbox,
    identity: PersonalFenceIdentity,
}
impl<'w> PreparedPersonalFence<'w> {
    pub(crate) fn identity(&self) -> &PersonalFenceIdentity {
        &self.identity
    }
    /// Own BOTH reservations before publishing either body or marker. The
    /// retained sealed identity is checked while abandoning remains harmless.
    pub(crate) fn pair<'i>(
        self,
        mut push: PreparedPersonalPush<'i>,
    ) -> Result<PairedPersonalAdmission<'w, 'i>, String> {
        let proposed = push.fence_identity(self.identity.base())?.ok_or("Personal marker has no new admitted body")?;
        if !self.identity.same_snapshot(&proposed) {
            return Err("Personal marker belongs to another Inbox or snapshot".into());
        }
        Ok(PairedPersonalAdmission { marker: self, push })
    }
}
pub(crate) struct PairedPersonalAdmission<'w, 'i> {
    marker: PreparedPersonalFence<'w>,
    push: PreparedPersonalPush<'i>,
}
impl PairedPersonalAdmission<'_, '_> {
    pub(crate) fn commit(self) {
        // pair owns the bound pending push and marker; no caller can replace
        // either identity between validation and this allocation-free commit.
        match self.push.commit() {
            PersonalAdmission::Admitted(admitted) => {
                let identity = admitted.identity();
                self.marker.window.personal_fences.push(identity.clone());
                self.marker.window.outcomes.push(RequestOutcome::PersonalSync {
                    window_event: identity.event(),
                    actor_respawns: identity.life(),
                });
            }
            _ => unreachable!("paired pending Personal admission"),
        }
    }
}
pub(crate) struct PreparedPersonalPrefix<'w> {
    window: &'w mut WindowInbox,
    base: u32,
    through: u32,
    life: u32,
    identities: Vec<PersonalFenceIdentity>,
}
impl PreparedPersonalPrefix<'_> {
    pub(crate) fn base(&self) -> u32 {
        self.base
    }
    pub(crate) fn through(&self) -> u32 {
        self.through
    }
    pub(crate) fn life(&self) -> u32 {
        self.life
    }
    pub(crate) fn identities(&self) -> &[PersonalFenceIdentity] {
        &self.identities
    }
    /// Includes the held WindowInbox allocations plus this reserved identity
    /// vector. Caller adds pending bodies/slot/other ownership exactly once.
    pub(crate) fn owned_bytes(&self) -> usize {
        self.window
            .retained_usage()
            .1
            .saturating_add(size_of::<Self>())
            .saturating_add(self.identities.capacity().saturating_mul(size_of::<PersonalFenceIdentity>()))
    }
    pub(crate) fn commit(self) {
        // The exclusive window borrow prevents additions/reordering after
        // preparation. Every retained outcome was proven Personal-only.
        self.window.outcomes.clear();
        self.window.personal_fences.clear();
    }
}
impl WindowInbox {
    pub(crate) fn prepare_personal_fence(
        &mut self,
        identity: &PersonalFenceIdentity,
        external: (usize, usize),
    ) -> Result<PreparedPersonalFence<'_>, String> {
        if identity.event() == 0
            || identity.event() <= identity.base()
            || self.outcomes.iter().any(|outcome| super::outcome_event(outcome) == identity.event())
        {
            return Err("Personal fence has an invalid or repeated numbered identity".into());
        }
        let (count, bytes) = self.retained_usage();
        if count.saturating_add(1).saturating_add(external.0) > crate::action_journal::MAX_EVENT_ENTRIES
            || bytes.saturating_add(size_of::<RequestOutcome>()).saturating_add(external.1)
                > crate::action_journal::MAX_RETAINED_BYTES
        {
            return Err("Personal fence exceeds its retained limit".into());
        }
        self.personal_fences.try_reserve_exact(1).map_err(|_| "Personal seal allocation failed")?;
        self.outcomes.try_reserve_exact(1).map_err(|_| "Personal fence allocation failed")?;
        if self.retained_usage().1.saturating_add(external.1).saturating_add(size_of::<PreparedPersonalFence<'_>>())
            > crate::action_journal::MAX_RETAINED_BYTES
        {
            return Err("Personal fence allocation exceeds its retained limit".into());
        }
        Ok(PreparedPersonalFence { window: self, identity: identity.clone() })
    }
    pub(crate) fn prepare_personal_prefix<'w>(
        &'w mut self,
        applied: u32,
        life: u32,
        personal: &Inbox,
        limit: usize,
    ) -> Result<PreparedPersonalPrefix<'w>, PersonalPrefixBlocked> {
        if !self.grants.is_empty()
            || !self.armour.is_empty()
            || self.ack.is_some()
            || self.outcomes.iter().any(|outcome| !matches!(outcome, RequestOutcome::PersonalSync { .. }))
        {
            return Err(PersonalPrefixBlocked::Ordinary);
        }
        if self.personal_fences.len() != self.outcomes.len() {
            return Err(PersonalPrefixBlocked::Missing);
        }
        if self.outcomes.is_empty() {
            return Err(PersonalPrefixBlocked::Empty);
        }
        if limit == 0 || self.outcomes.len() > limit.min(8) {
            return Err(PersonalPrefixBlocked::TooLarge);
        }
        let mut identities = Vec::new();
        identities.try_reserve_exact(self.outcomes.len()).map_err(|_| PersonalPrefixBlocked::TooLarge)?;
        let mut expected = applied.checked_add(1).ok_or(PersonalPrefixBlocked::Exhausted)?;
        // Stable ordinal selection without mutating the original queue on a
        // failed preparation. Duplicate/stale markers never grant a new ACK.
        for _ in 0..self.outcomes.len() {
            let Some(outcome) = self.outcomes.iter().find(|outcome| super::outcome_event(outcome) == expected) else {
                return Err(PersonalPrefixBlocked::Gap);
            };
            let RequestOutcome::PersonalSync { actor_respawns, .. } = outcome else { unreachable!() };
            if *actor_respawns != life {
                return Err(PersonalPrefixBlocked::Life);
            }
            let identity =
                personal.pending_fence_identity(expected, life, applied).ok_or(PersonalPrefixBlocked::Missing)?;
            if !self.personal_fences.iter().any(|stored| stored.same_snapshot(&identity)) {
                return Err(PersonalPrefixBlocked::Missing);
            }
            if identities.first().is_some_and(|first: &PersonalFenceIdentity| !first.same_namespace(&identity)) {
                return Err(PersonalPrefixBlocked::Missing);
            }
            identities.push(identity);
            expected = expected.checked_add(1).ok_or(PersonalPrefixBlocked::Exhausted)?;
        }
        let through = expected - 1;
        Ok(PreparedPersonalPrefix { window: self, base: applied, through, life, identities })
    }
}

#[cfg(test)]
mod tests;
