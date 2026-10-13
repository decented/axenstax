//! Complete immediate-effect reservation, before any irreversible action effect.
//! These bounds supplement existing tighter per-packet and per-tick limits.

use crate::action_journal::{MAX_EVENT_ENTRIES, MAX_RETAINED_BYTES, MAX_RESULT_ENTRIES, MAX_PAGE_BYTES};

pub const MAX_PRIVATE_BODY_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RESIDENT_BODY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "C3d compound-action milestone (complete event/result/view reservation across one action)"))]
pub struct Occupancy {
    pub entries: usize,
    pub bytes: usize,
}

impl Occupancy {
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d compound-action milestone (complete event/result/view reservation across one action)"))]
    fn plus(self, other: Self, entries: usize, bytes: usize) -> Option<Self> {
        let result = Self { entries: self.entries.checked_add(other.entries)?, bytes: self.bytes.checked_add(other.bytes)? };
        (result.entries <= entries && result.bytes <= bytes).then_some(result)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "C3d compound-action milestone (complete event/result/view reservation across one action)"))]
pub struct ImmediateEffects {
    pub events: Occupancy,
    pub results: Occupancy,
    /// Count actual retained receipt/view bytes, including tombstones.
    pub views: Occupancy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "C3d compound-action milestone (complete event/result/view reservation across one action)"))]
pub enum CapacityError { Events, Results, Views, Body, ResidentBodies }

/// The returned reservation is owned by the staged transaction. Replacing
/// occupancy is the commit step; failed staging leaves the original untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "C3d compound-action milestone (complete event/result/view reservation across one action)"))]
pub struct EffectCapacity {
    pub events: Occupancy,
    pub results: Occupancy,
    pub views: Occupancy,
}

impl EffectCapacity {
    #[cfg_attr(not(test), expect(dead_code, reason = "C3d compound-action milestone (complete event/result/view reservation across one action)"))]
    pub fn reserve(&self, all: ImmediateEffects) -> Result<Self, CapacityError> {
        Ok(Self {
            events: self.events.plus(all.events, MAX_EVENT_ENTRIES, MAX_RETAINED_BYTES).ok_or(CapacityError::Events)?,
            results: self.results.plus(all.results, MAX_RESULT_ENTRIES, MAX_PAGE_BYTES).ok_or(CapacityError::Results)?,
            views: self.views.plus(all.views, crate::action_journal::MAX_CONTEXT_CELLS, MAX_RETAINED_BYTES).ok_or(CapacityError::Views)?,
        })
    }
}

/// Count every unique resident private body and owned copy; sharing an Arc
/// does not make 2048 distinct bodies one allocation. Decode at most one new
/// body while deciding this reservation, before admitting it to the pool.
pub fn reserve_body(resident_bytes: usize, incoming_bytes: usize) -> Result<usize, CapacityError> {
    if incoming_bytes > MAX_PRIVATE_BODY_BYTES { return Err(CapacityError::Body); }
    let total = resident_bytes.checked_add(incoming_bytes).ok_or(CapacityError::ResidentBodies)?;
    if total > MAX_RESIDENT_BODY_BYTES { return Err(CapacityError::ResidentBodies); }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c3d_a_two_event_shot_reserves_both_before_projectile_commit() {
        let pending = EffectCapacity { events: Occupancy { entries: 1023, bytes: 100 }, ..Default::default() };
        let shot = ImmediateEffects { events: Occupancy { entries: 2, bytes: 100 }, ..Default::default() };
        let mut projectiles = 0;
        if pending.reserve(shot).is_ok() { projectiles += 1; }
        assert_eq!(projectiles, 0);
        assert_eq!(pending.events.entries, 1023);
        let legal = EffectCapacity { events: Occupancy { entries: 1022, bytes: 100 }, ..Default::default() }.reserve(shot).unwrap();
        assert_eq!(legal.events.entries, 1024);
    }

    #[test]
    fn c3d_result_and_view_bytes_can_refuse_a_fully_paid_action_before_effects() {
        let capacity = EffectCapacity::default();
        let oversized = ImmediateEffects { results: Occupancy { entries: 1, bytes: MAX_PAGE_BYTES + 1 }, ..Default::default() };
        assert_eq!(capacity.reserve(oversized), Err(CapacityError::Results));
        let oversized = ImmediateEffects { views: Occupancy { entries: 1, bytes: MAX_RETAINED_BYTES + 1 }, ..Default::default() };
        assert_eq!(capacity.reserve(oversized), Err(CapacityError::Views));
        let overflow = ImmediateEffects { events: Occupancy { entries: usize::MAX, bytes: usize::MAX }, ..Default::default() };
        assert_eq!(capacity.reserve(overflow), Err(CapacityError::Events));
    }

    #[test]
    fn c3d_distinct_private_bodies_and_owned_copies_count_against_resident_cap() {
        let mut resident = 0;
        for _ in 0..4 { resident = reserve_body(resident, MAX_PRIVATE_BODY_BYTES).unwrap(); }
        assert_eq!(resident, MAX_RESIDENT_BODY_BYTES);
        assert_eq!(reserve_body(resident, 1), Err(CapacityError::ResidentBodies));
        assert_eq!(reserve_body(0, MAX_PRIVATE_BODY_BYTES + 1), Err(CapacityError::Body));
        assert_eq!(reserve_body(usize::MAX, 1), Err(CapacityError::ResidentBodies));
    }
}
