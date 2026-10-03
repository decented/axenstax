//! Spec 28d.nostrich — The Nostrich's Vow curse mechanic.
//!
//! Triggered when a player kills (or eats meat from) a Nostrich.
//! Blocks vendor trade, zeroes village reputation, and suppresses
//! sats payouts for 24,000 ticks (1 in-game day @ default 1× time
//! speed). Purified by building a Nostrich Memorial (Phase G).
//!
//! Data layer: `NostrichVow` struct on PlayerSlot.
//! Decay: `tick_vow` runs once per game tick from `GameState::tick`.
//! Checks: `is_vow_active` is the single source of truth for any
//! gameplay code that needs to know "is this player cursed right now".

use serde::{Deserialize, Serialize};

/// Default vow duration in ticks. 24,000 ticks = 1 in-game day @
/// the default 1× time speed. Tunable in playtest if it feels too
/// harsh or too lenient.
pub const VOW_DURATION_TICKS: u32 = 24_000;

/// State of an active vow on a player. `None` on PlayerSlot means
/// no vow; `Some` means the player is currently cursed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NostrichVow {
    /// Ticks remaining before the vow naturally decays.
    pub ticks_remaining: u32,
    /// World tick the vow was triggered. Audit trail.
    pub origin_tick: u64,
    /// Why the vow was triggered. Used by toasts.
    pub reason: VowReason,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum VowReason {
    /// Player killed a Nostrich (most common trigger).
    KilledNostrich,
    /// Player ate Nostrich meat (raw or cooked).
    AteNostrichMeat,
}

impl NostrichVow {
    /// New vow with the standard duration.
    pub fn new(origin_tick: u64, reason: VowReason) -> Self {
        Self {
            ticks_remaining: VOW_DURATION_TICKS,
            origin_tick,
            reason,
        }
    }

    /// Real-minutes remaining at the default 1× time speed (20 TPS).
    /// 24,000 ticks ÷ 20 TPS ÷ 60 s/min = 20 real-minutes.
    pub fn minutes_remaining(&self) -> u32 {
        self.ticks_remaining / (20 * 60)
    }

    /// Decay one tick. Returns true iff the vow just expired this
    /// tick (so the caller can fire a "Vow lifted" toast).
    pub fn tick(&mut self) -> bool {
        if self.ticks_remaining > 1 {
            self.ticks_remaining -= 1;
            false
        } else {
            // Expired (or already at 0 — treat as expired idempotently).
            self.ticks_remaining = 0;
            true
        }
    }

    /// Convenience: is the vow expired? The live cleanup pass (game_loop.rs)
    /// instead reads the bool `tick()` itself returns, so this has no caller.
    #[allow(dead_code)]
    pub fn is_expired(&self) -> bool {
        self.ticks_remaining == 0
    }
}

/// Single source of truth — convenience for "should this gameplay
/// path bail because of an active vow?" checks. Takes `Option<&NostrichVow>`
/// so the gameplay code can pass `slot.nostrich_vow.as_ref()` directly.
pub fn is_vow_active(vow: Option<&NostrichVow>) -> bool {
    matches!(vow, Some(v) if v.ticks_remaining > 0)
}

/// Trigger text for the on-trigger flavour toast.
pub fn trigger_toast(reason: VowReason) -> &'static str {
    match reason {
        VowReason::KilledNostrich => "The Nostriches will remember this…",
        VowReason::AteNostrichMeat => "The meat is sour. The Nostriches notice.",
    }
}

/// Sub-toast on trigger explaining the effects. `trigger_toast` (the main
/// flavour toast) is live, but nothing shows this follow-up explanation yet.
#[allow(dead_code)]
pub fn effects_toast() -> &'static str {
    "Vow active. Vendors + villages + sats temporarily blocked. \
     A purple-wool memorial purifies; time also heals."
}

/// Toast on purification.
pub fn purified_toast() -> &'static str {
    "The Nostriches' Vow is lifted. Welcome back."
}

/// Toast on a vow-blocked vendor / sats / quest interaction. Caller
/// rate-limits to avoid spam.
pub fn blocked_toast() -> &'static str {
    "The Nostriches will not aid your dealings."
}

/// Pure detection — given a placement position + a sampling function
/// `is_memorial_block(x, y, z)`, return true iff the 3×3 wool memorial pattern
/// is complete with the placed block as one of the nine tiles. The
/// pattern is a flat 3×3 of wool blocks on a single y-level.
///
/// Searches all 9 possible 3×3 anchors that could contain `pos`:
/// pos as top-left, top-centre, top-right, centre-left, centre,
/// centre-right, bottom-left, bottom-centre, bottom-right.
pub fn detect_memorial<F>(pos: (i32, i32, i32), is_memorial_block: F) -> bool
where
    F: Fn(i32, i32, i32) -> bool,
{
    let (px, py, pz) = pos;
    if !is_memorial_block(px, py, pz) {
        return false;
    }
    // For each offset (dx, dz) where pos could sit inside a 3×3,
    // check the full 3×3 anchored at (px - dx, py, pz - dz).
    for dx in 0..3 {
        for dz in 0..3 {
            let base_x = px - dx;
            let base_z = pz - dz;
            let mut all_wool = true;
            'inner: for ox in 0..3 {
                for oz in 0..3 {
                    if !is_memorial_block(base_x + ox, py, base_z + oz) {
                        all_wool = false;
                        break 'inner;
                    }
                }
            }
            if all_wool {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod memorial_tests {
    use super::*;

    fn block_set(positions: &[(i32, i32, i32)]) -> impl Fn(i32, i32, i32) -> bool + '_ {
        move |x, y, z| positions.iter().any(|&(px, py, pz)| px == x && py == y && pz == z)
    }

    #[test]
    fn detect_returns_false_for_single_block() {
        let blocks = vec![(5, 70, 5)];
        let is_memorial_block = block_set(&blocks);
        assert!(!detect_memorial((5, 70, 5), is_memorial_block));
    }

    #[test]
    fn detect_returns_true_for_complete_3x3() {
        let mut blocks = Vec::new();
        for x in 5..8 {
            for z in 5..8 {
                blocks.push((x, 70, z));
            }
        }
        let is_memorial_block = block_set(&blocks);
        // Any block in the 3x3 should detect the memorial.
        for (x, y, z) in blocks.iter().copied() {
            assert!(detect_memorial((x, y, z), &is_memorial_block),
                "expected memorial detection from {:?}", (x, y, z));
        }
    }

    #[test]
    fn detect_returns_false_for_3x3_with_a_hole() {
        let mut blocks = Vec::new();
        for x in 5..8 {
            for z in 5..8 {
                if (x, z) == (6, 6) { continue; }  // hole in the middle
                blocks.push((x, 70, z));
            }
        }
        let is_memorial_block = block_set(&blocks);
        // (5, 70, 5) corner should not detect because the 3×3 anchored
        // there has a hole at (6, 70, 6).
        assert!(!detect_memorial((5, 70, 5), is_memorial_block));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_vow_starts_at_full_duration() {
        let v = NostrichVow::new(1000, VowReason::KilledNostrich);
        assert_eq!(v.ticks_remaining, VOW_DURATION_TICKS);
        assert_eq!(v.origin_tick, 1000);
        assert_eq!(v.reason, VowReason::KilledNostrich);
    }

    #[test]
    fn minutes_remaining_matches_24000_tick_window() {
        // 24,000 / (20 * 60) = 20 minutes.
        let v = NostrichVow::new(0, VowReason::KilledNostrich);
        assert_eq!(v.minutes_remaining(), 20);
    }

    #[test]
    fn tick_decays_one_per_call() {
        let mut v = NostrichVow::new(0, VowReason::KilledNostrich);
        let just_expired = v.tick();
        assert_eq!(v.ticks_remaining, VOW_DURATION_TICKS - 1);
        assert!(!just_expired);
    }

    #[test]
    fn tick_reports_expiration_on_last_tick() {
        let mut v = NostrichVow::new(0, VowReason::KilledNostrich);
        v.ticks_remaining = 1;
        let just_expired = v.tick();
        assert!(just_expired, "should report expiration when transitioning to 0");
        assert!(v.is_expired());
    }

    #[test]
    fn tick_at_zero_reports_expired_idempotently() {
        let mut v = NostrichVow::new(0, VowReason::KilledNostrich);
        v.ticks_remaining = 0;
        let just_expired = v.tick();
        assert!(just_expired);
        assert_eq!(v.ticks_remaining, 0);
    }

    #[test]
    fn is_vow_active_handles_none() {
        assert!(!is_vow_active(None));
    }

    #[test]
    fn is_vow_active_true_while_positive() {
        let v = NostrichVow::new(0, VowReason::AteNostrichMeat);
        assert!(is_vow_active(Some(&v)));
    }

    #[test]
    fn is_vow_active_false_when_expired() {
        let mut v = NostrichVow::new(0, VowReason::KilledNostrich);
        v.ticks_remaining = 0;
        assert!(!is_vow_active(Some(&v)));
    }

    #[test]
    fn trigger_toast_distinguishes_reasons() {
        assert!(trigger_toast(VowReason::KilledNostrich).contains("remember"));
        assert!(trigger_toast(VowReason::AteNostrichMeat).contains("sour"));
    }
}
