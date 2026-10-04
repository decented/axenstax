//! Leaf decay system — unsupported leaves decay after their log is removed.
//!
//! When a log is broken, nearby leaves are queued for support checks.
//! A leaf is supported if it can reach a log within 4 blocks via other leaves.
//! Unsupported leaves decay with a random delay for natural appearance.

use std::collections::VecDeque;
use ahash::AHashSet;
use crate::block;
use crate::world::World;

/// Max BFS distance from leaf to log for support.
const MAX_SUPPORT_DIST: u8 = 4;
/// Max leaves to support-check per tick.
const CHECK_BUDGET: usize = 16;
/// Max leaves to decay per tick.
const DECAY_BUDGET: usize = 8;
/// Hard ceiling on each queue. A burst of log destruction (clear-cut, TNT)
/// queues leaves far faster than the per-tick drain; without a cap the queues
/// grow unbounded (engine audit 2026-06-04, E). De-dup keeps it near the count
/// of distinct affected leaves; this backstops the pathological case.
const MAX_QUEUE: usize = 8192;

/// Spawn the sapling drops a [`LeafDecaySystem::tick`] rolled (drained with
/// [`LeafDecaySystem::take_sapling_drops`]) as item entities. Shared by the
/// client loop and `GameServer::tick` (T1-3, 2026-10-05) so a decayed leaf on a
/// hosted or dedicated world drops the same sapling a single-player one does.
pub fn spawn_sapling_drops(
    ecs: &mut hecs::World,
    drops: Vec<(i32, i32, i32, crate::item::MaterialId)>,
) {
    for (x, y, z, mat) in drops {
        crate::entity::spawn_item(
            ecs,
            glam::Vec3::new(x as f32 + 0.5, y as f32 + 0.2, z as f32 + 0.5),
            crate::item::ItemStack::new_material(mat, 1),
            (x ^ z) as u32,
        );
    }
}

pub struct LeafDecaySystem {
    /// Leaves pending a support check.
    check_queue: Vec<(i32, i32, i32)>,
    /// Membership set for `check_queue` — de-dups overlapping `on_log_broken`
    /// radii so the same leaf isn't queued (and re-BFS'd) many times.
    check_set: AHashSet<(i32, i32, i32)>,
    /// Unsupported leaves waiting to decay: (x, y, z, tick_to_decay).
    decay_queue: Vec<(i32, i32, i32, u64)>,
    /// Membership set for `decay_queue` — a leaf is only scheduled once.
    decay_set: AHashSet<(i32, i32, i32)>,
    /// Monotonic tick counter (incremented each call to tick).
    tick: u64,
    /// Sapling drops rolled this tick (2026-07-04): (x, y, z, sapling
    /// material). Drained by the caller via [`take_sapling_drops`] — this
    /// module has no ECS access, so the item entity spawn happens there.
    pending_sapling_drops: Vec<(i32, i32, i32, crate::item::MaterialId)>,
}

impl LeafDecaySystem {
    /// Drain the sapling drops rolled since the last call (2026-07-04).
    pub fn take_sapling_drops(&mut self) -> Vec<(i32, i32, i32, crate::item::MaterialId)> {
        std::mem::take(&mut self.pending_sapling_drops)
    }

    pub fn new() -> Self {
        Self {
            check_queue: Vec::new(),
            check_set: AHashSet::new(),
            decay_queue: Vec::new(),
            decay_set: AHashSet::new(),
            tick: 0,
            pending_sapling_drops: Vec::new(),
        }
    }

    /// Called when any log is broken. Scans nearby leaves (of every species)
    /// and queues them for support checks. Was OAK-only — breaking a Birch /
    /// Spruce / Jungle / etc. log left its leaves floating forever (#14).
    pub fn on_log_broken(&mut self, x: i32, y: i32, z: i32, world: &World) {
        let radius = 5;
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let bx = x + dx;
                    let by = y + dy;
                    let bz = z + dz;
                    if block::is_any_leaves(world.get_block(bx, by, bz))
                        && self.check_queue.len() < MAX_QUEUE
                        && self.check_set.insert((bx, by, bz))
                    {
                        self.check_queue.push((bx, by, bz));
                    }
                }
            }
        }
    }

    /// Run one tick: check leaf support, then decay expired leaves.
    /// Returns dirty block positions.
    pub fn tick(&mut self, world: &mut World) -> Vec<(i32, i32, i32)> {
        self.tick += 1;
        let mut dirty: Vec<(i32, i32, i32)> = Vec::new();

        // Phase 1: Support checks
        let mut checked = 0;
        while checked < CHECK_BUDGET && !self.check_queue.is_empty() {
            let (lx, ly, lz) = self.check_queue.swap_remove(0);
            self.check_set.remove(&(lx, ly, lz));
            checked += 1;

            // Skip if already decayed or replaced
            if !block::is_any_leaves(world.get_block(lx, ly, lz)) {
                continue;
            }

            if !self.is_supported(lx, ly, lz, world) {
                // Random delay: 27-108 ticks at 5 Hz = 5.4-21.6 seconds.
                // Schedule each leaf for decay at most once.
                let delay = 27 + simple_hash(lx, ly, lz) % 82;
                if self.decay_set.insert((lx, ly, lz)) {
                    self.decay_queue.push((lx, ly, lz, self.tick + delay as u64));
                }
            }
        }

        // Phase 2: Decay expired leaves
        let mut decayed = 0;
        let mut i = 0;
        while i < self.decay_queue.len() && decayed < DECAY_BUDGET {
            if self.decay_queue[i].3 <= self.tick {
                let (dx, dy, dz, _) = self.decay_queue.swap_remove(i);
                self.decay_set.remove(&(dx, dy, dz));
                // Only decay if still a leaf (might have been broken by player)
                let leaf_id = world.get_block(dx, dy, dz);
                if block::is_any_leaves(leaf_id) {
                    world.set_block(dx, dy, dz, block::AIR);
                    // Owner-inbox #1/2/3 — a decayed leaf drops any wallpaper
                    // overlays on its cell, so no orphan resurfaces later.
                    world.remove_face_attachments_at((dx, dy, dz));
                    // Saplings (2026-07-04): ~1 in 20 decayed leaves drops the
                    // species' sapling — the renewable tree loop. Deterministic
                    // per position+tick; the caller spawns the item entity.
                    if let Some(species) = block::species_for_leaves(leaf_id) {
                        let h = (dx as u64 & 0xFFFF)
                            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                            .wrapping_add((dz as u64 & 0xFFFF) << 24)
                            .wrapping_add(dy as u64)
                            ^ self.tick.wrapping_mul(31);
                        if h.is_multiple_of(20) {
                            self.pending_sapling_drops.push((
                                dx,
                                dy,
                                dz,
                                crate::block::sapling_material_for(species),
                            ));
                        }
                    }
                    dirty.push((dx, dy, dz));
                    decayed += 1;
                }
                // Don't increment i — swap_remove moved last element here
            } else {
                i += 1;
            }
        }

        dirty
    }

    /// BFS from a leaf block through other leaves. Returns true if a log is
    /// reachable within MAX_SUPPORT_DIST steps.
    fn is_supported(&self, x: i32, y: i32, z: i32, world: &World) -> bool {
        let mut visited: AHashSet<(i32, i32, i32)> = AHashSet::new();
        let mut frontier: VecDeque<(i32, i32, i32, u8)> = VecDeque::new();
        visited.insert((x, y, z));
        frontier.push_back((x, y, z, 0));

        while let Some((cx, cy, cz, dist)) = frontier.pop_front() {
            for &(dx, dy, dz) in &[(1,0,0),(-1,0,0),(0,1,0),(0,-1,0),(0,0,1),(0,0,-1)] {
                let nx = cx + dx;
                let ny = cy + dy;
                let nz = cz + dz;

                if !visited.insert((nx, ny, nz)) {
                    continue;
                }

                let blk = world.get_block(nx, ny, nz);
                if block::is_any_log_block(blk) {
                    return true; // Found a supporting log (any species)
                }
                if block::is_any_leaves(blk) && dist + 1 < MAX_SUPPORT_DIST {
                    frontier.push_back((nx, ny, nz, dist + 1));
                }
            }
        }
        false
    }
}

/// Simple deterministic hash for random delay variation.
fn simple_hash(x: i32, y: i32, z: i32) -> u32 {
    let mut h = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263) ^ z.wrapping_mul(1274126177);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    (h ^ (h >> 16)).unsigned_abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::FaceAttachment;

    #[test]
    fn repeated_log_breaks_do_not_duplicate_queue_entries() {
        // Overlapping on_log_broken radii covered the same leaves repeatedly,
        // growing check_queue unbounded faster than the 16/tick drain (engine
        // audit 2026-06-04, E). De-dup keeps each leaf queued once.
        let mut world = World::new();
        let mut sys = LeafDecaySystem::new();
        world.set_block(0, 70, 0, block::OAK_LEAVES);
        world.set_block(1, 70, 0, block::OAK_LEAVES);
        for _ in 0..5 {
            sys.on_log_broken(0, 70, 0, &world); // same spot, overlapping radii
        }
        assert_eq!(sys.check_queue.len(), 2, "2 unique leaves, not 10 (5 breaks × 2)");
        assert_eq!(sys.check_set.len(), 2, "membership set tracks the unique leaves");
    }

    #[test]
    fn decayed_leaf_clears_its_face_attachments() {
        // Owner-inbox #1/2/3 — when an unsupported leaf decays to AIR it must
        // drop any wallpaper overlay on its cell, or the overlay orphans and
        // resurfaces as phantom wallpaper + a dupe-on-break when the cell is
        // re-occupied. (Sibling of falling_blocks::falling_block_clears_its_face_overlays.)
        let mut world = World::new();
        let mut sys = LeafDecaySystem::new();
        let leaf = (0, 70, 0);
        // A lone leaf with no log within range is unsupported -> decays.
        world.set_block(leaf.0, leaf.1, leaf.2, block::OAK_LEAVES);
        world.set_face_attachment(leaf, 0, FaceAttachment::Wallpaper(block::WALLPAPER_GREEN));

        sys.on_log_broken(leaf.0, leaf.1, leaf.2, &world);
        // Tick well past the 27-108-tick decay delay.
        for _ in 0..200 {
            sys.tick(&mut world);
        }

        assert_eq!(
            world.get_block(leaf.0, leaf.1, leaf.2),
            block::AIR,
            "unsupported leaf must have decayed"
        );
        assert!(
            world.face_attachment_at(leaf, 0).is_none(),
            "a decayed leaf must not leave an orphan wallpaper overlay"
        );
    }

    /// #14 — leaf decay must handle EVERY species, not just oak. A birch leaf
    /// supported only by a birch log must survive while that log stands (proving
    /// `is_supported`'s BFS recognises a birch log as support, and
    /// `on_log_broken`/`tick` recognise birch leaves) and decay once the log is
    /// gone. Before the fix these hard-checked OAK_LOG/OAK_LEAVES, so non-oak
    /// leaves floated forever.
    #[test]
    fn non_oak_leaves_decay_when_their_log_is_removed() {
        let mut world = World::new();
        let mut sys = LeafDecaySystem::new();
        let log = (0, 70, 0);
        let leaf = (1, 70, 0); // adjacent → supported while the log stands

        world.set_block(log.0, log.1, log.2, block::BIRCH_LOG);
        world.set_block(leaf.0, leaf.1, leaf.2, block::BIRCH_LEAVES);

        // Phase 1 — while the birch log stands, a support check leaves the birch
        // leaf in place (it reaches the log within MAX_SUPPORT_DIST).
        sys.on_log_broken(leaf.0, leaf.1, leaf.2, &world);
        for _ in 0..200 {
            sys.tick(&mut world);
        }
        assert_eq!(
            world.get_block(leaf.0, leaf.1, leaf.2),
            block::BIRCH_LEAVES,
            "a birch leaf supported by a standing birch log must NOT decay"
        );

        // Phase 2 — remove the birch log and notify: the now-unsupported birch
        // leaf must decay to AIR.
        world.set_block(log.0, log.1, log.2, block::AIR);
        sys.on_log_broken(log.0, log.1, log.2, &world);
        for _ in 0..200 {
            sys.tick(&mut world);
        }
        assert_eq!(
            world.get_block(leaf.0, leaf.1, leaf.2),
            block::AIR,
            "a birch leaf must decay once its only birch-log support is removed (#14)"
        );
    }
}
