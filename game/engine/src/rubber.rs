//! Rubber feature — tap state machine.
//!
//! `RUBBER_LOG` is the living tree that produces latex when tapped
//! with an empty Bucket. On tap, the log converts to
//! `RUBBER_LOG_TAPPED` and the position is stamped in
//! `World.tapped_rubber_logs` with the current tick. After
//! `TAP_COOLDOWN_TICKS` (24 000 ≈ 1 in-game day), the cooldown
//! driver flips the block back to RUBBER_LOG and drops the index
//! entry.
//!
//! Mining either variant yields a GreenLog timber drop (the species-
//! neutral wood-log convention) — the cooldown state is lost when
//! the player fells the tree. The player chooses between ongoing
//! tapping or one-shot timber.
//!
//! Spec: `docs/foundations/2026-05-23-rubber.md`.

use crate::block::{self, BlockId};
use crate::world::World;

/// Ticks the tapped state persists before the cooldown driver
/// restores the live RUBBER_LOG. 24 000 ticks @ 20 TPS ≈ 1 in-game
/// day (matches the brigand-hideout replenisher cadence).
pub const TAP_COOLDOWN_TICKS: u64 = 24_000;

/// True for the live, tappable variant only. Tapped trees return
/// false because they're already in cooldown.
pub fn is_tappable(id: BlockId) -> bool {
    id == block::RUBBER_LOG
}

/// Apply a tap at `pos`. Returns `true` on success (block was a live
/// RUBBER_LOG and got converted); `false` otherwise (already tapped,
/// felled, or not a rubber log). On success: flips the block to
/// RUBBER_LOG_TAPPED + stamps the cooldown index. Caller is
/// responsible for spawning the Rubber material into the player's
/// inventory.
pub fn apply_tap(world: &mut World, pos: (i32, i32, i32), current_tick: u64) -> bool {
    if !is_tappable(world.get_block(pos.0, pos.1, pos.2)) {
        return false;
    }
    world.set_block(pos.0, pos.1, pos.2, block::RUBBER_LOG_TAPPED);
    world.tapped_rubber_logs.insert(pos, current_tick);
    true
}

/// Walk the cooldown index; for every entry whose
/// `tap_tick + TAP_COOLDOWN_TICKS <= current_tick`, restore the
/// block at that position to RUBBER_LOG (only if the block is still
/// RUBBER_LOG_TAPPED — the player may have felled it). Drops the
/// entry from the index regardless of whether the restoration
/// actually happened.
///
/// Returns the count of restorations (useful for tests).
pub fn tick_rubber_cooldowns(world: &mut World, current_tick: u64) -> u32 {
    // Snapshot the entries first so we don't borrow tapped_rubber_logs
    // mutably while reading World.
    let expired: Vec<(i32, i32, i32)> = world
        .tapped_rubber_logs
        .iter()
        .filter(|&(_, &tap_tick)| {
            current_tick.saturating_sub(tap_tick) >= TAP_COOLDOWN_TICKS
        })
        .map(|(&pos, _)| pos)
        .collect();
    let mut restored = 0u32;
    for pos in expired {
        // Only restore the block if it's still in the tapped state;
        // the player may have felled it during cooldown.
        if world.get_block(pos.0, pos.1, pos.2) == block::RUBBER_LOG_TAPPED {
            world.set_block(pos.0, pos.1, pos.2, block::RUBBER_LOG);
            restored += 1;
        }
        world.tapped_rubber_logs.remove(&pos);
    }
    restored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_tappable_returns_true_only_for_live_log() {
        assert!(is_tappable(block::RUBBER_LOG));
        assert!(!is_tappable(block::RUBBER_LOG_TAPPED));
        assert!(!is_tappable(block::OAK_LOG));
        assert!(!is_tappable(block::AIR));
    }

    #[test]
    fn apply_tap_flips_block_and_stamps_index() {
        let mut w = World::new();
        w.set_block(0, 64, 0, block::RUBBER_LOG);
        let ok = apply_tap(&mut w, (0, 64, 0), 1_000);
        assert!(ok);
        assert_eq!(w.get_block(0, 64, 0), block::RUBBER_LOG_TAPPED);
        assert_eq!(w.tapped_rubber_logs.get(&(0, 64, 0)), Some(&1_000));
    }

    #[test]
    fn apply_tap_noops_on_already_tapped() {
        let mut w = World::new();
        w.set_block(0, 64, 0, block::RUBBER_LOG_TAPPED);
        w.tapped_rubber_logs.insert((0, 64, 0), 500);
        let ok = apply_tap(&mut w, (0, 64, 0), 1_000);
        assert!(!ok);
        // Index unchanged.
        assert_eq!(w.tapped_rubber_logs.get(&(0, 64, 0)), Some(&500));
    }

    #[test]
    fn apply_tap_noops_on_non_rubber_block() {
        let mut w = World::new();
        w.set_block(0, 64, 0, block::OAK_LOG);
        let ok = apply_tap(&mut w, (0, 64, 0), 1_000);
        assert!(!ok);
        assert_eq!(w.get_block(0, 64, 0), block::OAK_LOG);
    }

    #[test]
    fn cooldown_driver_restores_after_full_cooldown() {
        let mut w = World::new();
        w.set_block(0, 64, 0, block::RUBBER_LOG);
        let _ = apply_tap(&mut w, (0, 64, 0), 0);
        // Mid-cooldown — no restoration.
        let restored = tick_rubber_cooldowns(&mut w, TAP_COOLDOWN_TICKS - 1);
        assert_eq!(restored, 0);
        assert_eq!(w.get_block(0, 64, 0), block::RUBBER_LOG_TAPPED);
        // Cooldown reached — restored.
        let restored = tick_rubber_cooldowns(&mut w, TAP_COOLDOWN_TICKS);
        assert_eq!(restored, 1);
        assert_eq!(w.get_block(0, 64, 0), block::RUBBER_LOG);
        assert!(!w.tapped_rubber_logs.contains_key(&(0, 64, 0)));
    }

    #[test]
    fn cooldown_driver_drops_index_when_player_felled_tapped_log() {
        let mut w = World::new();
        w.set_block(0, 64, 0, block::RUBBER_LOG);
        let _ = apply_tap(&mut w, (0, 64, 0), 0);
        // Player fells the tapped tree mid-cooldown.
        w.set_block(0, 64, 0, block::AIR);
        let restored = tick_rubber_cooldowns(&mut w, TAP_COOLDOWN_TICKS);
        assert_eq!(restored, 0, "felled tree should not be resurrected");
        assert!(!w.tapped_rubber_logs.contains_key(&(0, 64, 0)),
            "cooldown entry should be dropped regardless of restoration");
    }
}
