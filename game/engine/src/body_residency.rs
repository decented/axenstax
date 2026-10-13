//! Actual client-owned Plan allocations, without cloning bodies into history.
use crate::action_capacity::{CapacityError, MAX_PRIVATE_BODY_BYTES, MAX_RESIDENT_BODY_BYTES};
use crate::plan::PlanData;

/// A borrowed scan deduplicates aliases of the same allocation. Distinct owned
/// copies have distinct addresses and are charged even when their marker agrees.
#[derive(Default)]
pub struct BodyResidency {
    seen: Vec<*const PlanData>,
    bytes: usize,
}

pub fn allocated_body_bytes(body: &PlanData) -> Result<usize, CapacityError> {
    let mut bytes = std::mem::size_of::<PlanData>();
    let mut add = |n| { bytes = bytes.checked_add(n).ok_or(CapacityError::Body)?; Ok::<_, CapacityError>(()) };
    add(body.name.capacity())?;
    add(body.author_npub.capacity())?;
    add(body.authored_in.capacity())?;
    add(body.cells.capacity().checked_mul(std::mem::size_of::<crate::plan::CapturedCell>()).ok_or(CapacityError::Body)?)?;
    add(body.derivation_chain.capacity().checked_mul(std::mem::size_of::<crate::plan::DerivationLink>()).ok_or(CapacityError::Body)?)?;
    for ancestor in &body.derivation_chain { add(ancestor.author_npub.capacity())?; add(ancestor.plan_name.capacity())?; }
    Ok(bytes)
}

impl BodyResidency {
    pub fn bytes(&self) -> usize { self.bytes + self.seen.capacity() * std::mem::size_of::<*const PlanData>() }
    pub fn observe(&mut self, body: &PlanData) -> Result<(), CapacityError> {
        if body.marker.is_some() { return Ok(()); }
        let address = std::ptr::from_ref(body);
        if self.seen.contains(&address) { return Ok(()); }
        let incoming = allocated_body_bytes(body)?;
        if incoming > MAX_PRIVATE_BODY_BYTES { return Err(CapacityError::Body); }
        let next = self.bytes.checked_add(incoming).ok_or(CapacityError::ResidentBodies)?;
        // The scan itself is bounded by the body cap as well. Charge its actual
        // allocated pointer capacity instead of treating metadata as free.
        let metadata = self.seen.capacity().max(self.seen.len() + 1).checked_mul(std::mem::size_of::<*const PlanData>()).ok_or(CapacityError::ResidentBodies)?;
        if next.checked_add(metadata).is_none_or(|n| n > MAX_RESIDENT_BODY_BYTES) { return Err(CapacityError::ResidentBodies); }
        self.seen.try_reserve_exact(1).map_err(|_| CapacityError::ResidentBodies)?;
        if next.checked_add(self.seen.capacity() * std::mem::size_of::<*const PlanData>()).is_none_or(|n| n > MAX_RESIDENT_BODY_BYTES) { return Err(CapacityError::ResidentBodies); }
        self.seen.push(address);
        self.bytes = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn c3d_actual_body_capacity_charges_distinct_owned_copies_and_deduplicates_aliases() {
        let mut body = PlanData::debug_3x3_stone();
        body.cells.reserve_exact(500);
        body.name.reserve_exact(100);
        let copied = body.clone();
        let mut residency = BodyResidency::default();
        residency.observe(&body).unwrap();
        let one = residency.bytes();
        assert_eq!(one, allocated_body_bytes(&body).unwrap() + residency.seen.capacity() * std::mem::size_of::<*const PlanData>());
        assert!(one > body.cells.len() * std::mem::size_of::<crate::plan::CapturedCell>());
        residency.observe(&body).unwrap();
        assert_eq!(residency.bytes(), one);
        residency.observe(&copied).unwrap();
        assert_eq!(residency.bytes(), allocated_body_bytes(&body).unwrap() + allocated_body_bytes(&copied).unwrap() + residency.seen.capacity() * std::mem::size_of::<*const PlanData>());
        let stub = PlanData::marker_placeholder(crate::plan::marker(&body), true);
        residency.observe(&stub).unwrap();
        assert_eq!(residency.bytes(), allocated_body_bytes(&body).unwrap() + allocated_body_bytes(&copied).unwrap() + residency.seen.capacity() * std::mem::size_of::<*const PlanData>());
    }
}
