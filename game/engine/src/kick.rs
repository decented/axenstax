//! Kick — find the connected slot for a target npub (Spec B §5). Pure; the
//! actual mid-session disconnect (mark the slot + drop it from broadcasts) lives
//! in `hosted_server` and is verified live (owner boundary).

/// Every LIVE slot whose verified pubkey matches `target`. `players` is
/// `(verified_pubkey, live)` per slot. A kick addresses a person by their
/// stable identity — the npub — resolved to slots at the moment it lands:
/// freed slots from an earlier session are skipped (audit 2026-09-27: the old
/// first-match picked a dead slot after a rejoin, and the player played on),
/// and every live match is returned so no second connection survives.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // used by the native hosted_server kick path; dead on the wasm build
pub fn live_slots_for_pubkey(players: &[(Option<[u8; 32]>, bool)], target: &[u8; 32]) -> Vec<usize> {
    players
        .iter()
        .enumerate()
        .filter(|(_, (pk, live))| *live && pk.as_ref() == Some(target))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 32] = [1u8; 32];
    const B: [u8; 32] = [2u8; 32];

    #[test]
    fn finds_matching_live_slot() {
        let players = vec![(None, true), (Some(A), true), (Some(B), true)];
        assert_eq!(live_slots_for_pubkey(&players, &A), vec![1]);
        assert_eq!(live_slots_for_pubkey(&players, &B), vec![2]);
    }

    #[test]
    fn absent_target_is_empty() {
        let players = vec![(None, true), (Some(A), true)];
        assert!(live_slots_for_pubkey(&players, &B).is_empty());
    }

    #[test]
    fn guests_are_skipped() {
        let players = vec![(None, true), (None, true)];
        assert!(live_slots_for_pubkey(&players, &A).is_empty());
    }

    #[test]
    fn after_a_rejoin_the_live_slot_is_hit_not_the_stale_one() {
        // Alice joined in slot 1, left, rejoined in slot 3.
        let players = vec![(None, true), (Some(A), false), (Some(B), true), (Some(A), true)];
        assert_eq!(live_slots_for_pubkey(&players, &A), vec![3]);
    }
}
