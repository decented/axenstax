//! WorldEdit-style region editing (#7) — region selection + mass-edit ops over
//! the `World`, plus a per-player selection/clipboard session. The command
//! surface is `/we` (the parser strips a single `/`, so WorldEdit's `//` syntax
//! can't be used — `/we set …` etc. instead). The mutation ops are free
//! functions over `&mut World`, so they unit-test headless.
//!
//! Spec: `docs/foundations/2026-06-16-worldedit-region-editing.md`.

use crate::block::BlockId;
use crate::world::World;

/// Hard cap on a single op's block volume — refuses edits that would hang/OOM.
pub const MAX_REGION_VOLUME: u64 = 2_000_000;

/// Per-player WorldEdit state (selection corners + clipboard). Lives on
/// `PlayerSlot`; transient (not persisted).
#[derive(Default, Clone)]
pub struct WorldEditSession {
    pub pos1: Option<[i32; 3]>,
    pub pos2: Option<[i32; 3]>,
    pub clipboard: Option<Clipboard>,
}

/// A captured cuboid: block ids relative to the copy's min corner, row-major
/// with x fastest (`idx = x + y*dx + z*dx*dy`).
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard {
    pub dims: [i32; 3],
    pub blocks: Vec<BlockId>,
}

/// Inclusive (min, max) corners of the cuboid spanned by two points.
pub fn bounds(p1: [i32; 3], p2: [i32; 3]) -> ([i32; 3], [i32; 3]) {
    (
        [p1[0].min(p2[0]), p1[1].min(p2[1]), p1[2].min(p2[2])],
        [p1[0].max(p2[0]), p1[1].max(p2[1]), p1[2].max(p2[2])],
    )
}

/// Block count in the inclusive cuboid `min..=max`.
pub fn volume(min: [i32; 3], max: [i32; 3]) -> u64 {
    let dx = (max[0] - min[0] + 1) as i64;
    let dy = (max[1] - min[1] + 1) as i64;
    let dz = (max[2] - min[2] + 1) as i64;
    if dx <= 0 || dy <= 0 || dz <= 0 {
        0
    } else {
        (dx * dy * dz) as u64
    }
}

/// Chunk coords (16³) touched by the inclusive cuboid `min..=max` — for re-mesh.
pub fn affected_chunks(min: [i32; 3], max: [i32; 3]) -> Vec<(i32, i32, i32)> {
    let t = crate::chunk::CHUNK_SIZE as i32;
    let lo = [min[0].div_euclid(t), min[1].div_euclid(t), min[2].div_euclid(t)];
    let hi = [max[0].div_euclid(t), max[1].div_euclid(t), max[2].div_euclid(t)];
    let mut out = Vec::new();
    for cx in lo[0]..=hi[0] {
        for cy in lo[1]..=hi[1] {
            for cz in lo[2]..=hi[2] {
                out.push((cx, cy, cz));
            }
        }
    }
    out
}

/// Write one cell the way a gameplay edit would, not the way worldgen does.
/// Returns true if the cell changed (and records it in `changed`).
///
/// The raw `World::set_block` left the old block's state behind: `/we set air`
/// over a battery kept its `PowerDevice` as a ghost source still powering the
/// network, a chest's contents stayed orphaned under whatever came next, and a
/// pasted lamp-and-lever build had no devices behind it, so it was inert
/// (audit 2026-09-27, P7). Here the old block's meta and block-entity go with
/// it, a power block gets its device registered (as the place handler does),
/// and any power cell wakes its network (Spec 48 §2.3).
///
/// Only power cells notify their neighbours: a region is up to
/// `MAX_REGION_VOLUME` cells, and queuing six neighbours for every one of them
/// would flood the update scheduler for nothing.
pub fn write_cell(world: &mut World, pos: [i32; 3], new: BlockId, changed: &mut Vec<[i32; 3]>) -> bool {
    let [x, y, z] = pos;
    let old = world.get_block(x, y, z);
    if old == new {
        return false;
    }
    let key = (x, y, z);
    world.set_block(x, y, z, new);
    // The old block's meta and block-entity belong to the old block.
    world.set_meta(key, 0);
    world.block_entities.remove(&key);
    if let Some(kind) = crate::power::device_kind_for_block(new) {
        world.insert_power_device(
            key,
            crate::power::PowerDeviceData::new(kind, crate::meta::facing(0)),
        );
    }
    if crate::block::is_power_block(old) || crate::block::is_power_block(new) {
        world.mark_dirty(key);
        world.notify_neighbours(key);
    }
    changed.push(pos);
    true
}

/// Fill the cuboid spanned by `p1,p2` with `block`. Returns the number of cells
/// actually changed; each changed cell is appended to `changed`.
pub fn region_set(
    world: &mut World,
    p1: [i32; 3],
    p2: [i32; 3],
    block: BlockId,
    changed: &mut Vec<[i32; 3]>,
) -> u32 {
    let (min, max) = bounds(p1, p2);
    let mut n = 0;
    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                if write_cell(world, [x, y, z], block, changed) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Replace `from` with `to` inside the cuboid. Returns cells changed.
pub fn region_replace(
    world: &mut World,
    p1: [i32; 3],
    p2: [i32; 3],
    from: BlockId,
    to: BlockId,
    changed: &mut Vec<[i32; 3]>,
) -> u32 {
    let (min, max) = bounds(p1, p2);
    let mut n = 0;
    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                if world.get_block(x, y, z) == from && write_cell(world, [x, y, z], to, changed) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Set only the cuboid's outer shell (the four vertical walls + the top/bottom
/// rings are NOT included — this is WorldEdit `//walls`: the four side faces).
pub fn region_walls(
    world: &mut World,
    p1: [i32; 3],
    p2: [i32; 3],
    block: BlockId,
    changed: &mut Vec<[i32; 3]>,
) -> u32 {
    let (min, max) = bounds(p1, p2);
    let mut n = 0;
    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                let is_wall = x == min[0] || x == max[0] || z == min[2] || z == max[2];
                if is_wall && write_cell(world, [x, y, z], block, changed) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Capture the cuboid into a clipboard (relative to its min corner).
pub fn region_copy(world: &World, p1: [i32; 3], p2: [i32; 3]) -> Clipboard {
    let (min, max) = bounds(p1, p2);
    let dims = [max[0] - min[0] + 1, max[1] - min[1] + 1, max[2] - min[2] + 1];
    let mut blocks = Vec::with_capacity((dims[0] * dims[1] * dims[2]).max(0) as usize);
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                blocks.push(world.get_block(min[0] + x, min[1] + y, min[2] + z));
            }
        }
    }
    Clipboard { dims, blocks }
}

/// Paste a clipboard with `at` as the min corner. Returns cells changed.
pub fn clipboard_paste(
    world: &mut World,
    clip: &Clipboard,
    at: [i32; 3],
    changed: &mut Vec<[i32; 3]>,
) -> u32 {
    let [dx, dy, dz] = clip.dims;
    let mut n = 0;
    for z in 0..dz {
        for y in 0..dy {
            for x in 0..dx {
                let b = clip.blocks[(x + y * dx + z * dx * dy) as usize];
                if write_cell(world, [at[0] + x, at[1] + y, at[2] + z], b, changed) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Copy the cuboid then paste it `count` times, each offset by the region's own
/// size along `axis` (0=x,1=y,2=z). Returns (cells_changed, new_min, new_max)
/// covering the original + all copies (for the re-mesh range).
pub fn region_stack(
    world: &mut World,
    p1: [i32; 3],
    p2: [i32; 3],
    axis: usize,
    count: i32,
    changed: &mut Vec<[i32; 3]>,
) -> (u32, [i32; 3], [i32; 3]) {
    let clip = region_copy(world, p1, p2);
    let (min, max) = bounds(p1, p2);
    let axis = axis.min(2);
    let size = clip.dims[axis];
    let mut total = 0;
    for i in 1..=count.max(0) {
        let mut at = min;
        at[axis] += size * i;
        total += clipboard_paste(world, &clip, at, changed);
    }
    let mut new_max = max;
    if count > 0 {
        new_max[axis] = max[axis] + size * count;
    }
    (total, min, new_max)
}

/// Most `/we` cells fed into one host tick's broadcast. A `BlockChange` is 15
/// bytes on the wire. This batch was first the only thing keeping a region
/// edit's StateUpdate under `protocol::MAX_PACKET_SIZE` (review W3 B2); since
/// gap-audit T1-5 every client's `state_outbox` splits and paces StateUpdates
/// itself, so the batch is now pacing, not protection: it keeps a huge region
/// from landing in one tick, and each batch is valued from the server's world
/// when it is taken, so a later edit to a queued cell is never undone.
pub const REGION_BROADCAST_BATCH: usize = 1024;

/// Host-side `/we` cells waiting to be broadcast, oldest first. The region
/// is applied to the server's world at once; joiners get it in batches of at
/// most [`REGION_BROADCAST_BATCH`] over successive ticks. Each batch reads
/// the cell's CURRENT value from the world it is handed (the server's), so a
/// later edit to a queued cell is never overwritten by a stale snapshot.
#[derive(Default)]
pub struct RegionBroadcastQueue {
    cells: std::collections::VecDeque<[i32; 3]>,
}

impl RegionBroadcastQueue {
    /// Queue a region's changed cells behind any still waiting.
    pub fn push(&mut self, changed: &[[i32; 3]]) {
        self.cells.extend(changed.iter().copied());
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The next batch, in queue order: at most `budget` (itself capped at
    /// [`REGION_BROADCAST_BATCH`]) changes, valued from `world`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn next_batch(&mut self, world: &World, budget: usize) -> Vec<crate::protocol::BlockChange> {
        let n = budget.min(REGION_BROADCAST_BATCH).min(self.cells.len());
        let cells: Vec<[i32; 3]> = self.cells.drain(..n).collect();
        crate::world_exit::region_broadcast(world, &cells)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    #[test]
    fn bounds_orders_corners() {
        assert_eq!(bounds([5, 2, 9], [1, 8, 3]), ([1, 2, 3], [5, 8, 9]));
    }

    #[test]
    fn volume_counts_inclusive_cells() {
        assert_eq!(volume([0, 0, 0], [0, 0, 0]), 1);
        assert_eq!(volume([0, 0, 0], [1, 1, 1]), 8);
        assert_eq!(volume([0, 0, 0], [2, 0, 0]), 3);
    }

    #[test]
    fn affected_chunks_covers_the_span() {
        // A cuboid from (0,0,0) to (16,0,0) spans chunk columns cx 0 and 1.
        let cs = affected_chunks([0, 0, 0], [16, 0, 0]);
        assert!(cs.contains(&(0, 0, 0)));
        assert!(cs.contains(&(1, 0, 0)));
        // Negative coords use floor division.
        let cs2 = affected_chunks([-1, 0, 0], [-1, 0, 0]);
        assert_eq!(cs2, vec![(-1, 0, 0)]);
    }

    #[test]
    fn region_set_fills_and_counts_changes() {
        let mut w = World::new();
        let n = region_set(&mut w, [0, 4, 0], [1, 5, 1], block::STONE, &mut Vec::new()); // 2×2×2
        assert_eq!(n, 8);
        assert_eq!(w.get_block(0, 4, 0), block::STONE);
        assert_eq!(w.get_block(1, 5, 1), block::STONE);
        // Re-running is a no-op (already stone).
        assert_eq!(region_set(&mut w, [0, 4, 0], [1, 5, 1], block::STONE, &mut Vec::new()), 0);
    }

    #[test]
    fn region_replace_only_touches_matching_cells() {
        let mut w = World::new();
        region_set(&mut w, [0, 4, 0], [2, 4, 0], block::STONE, &mut Vec::new());
        w.set_block(1, 4, 0, block::DIRT); // a non-matching cell in the row
        let n = region_replace(&mut w, [0, 4, 0], [2, 4, 0], block::STONE, block::GLASS, &mut Vec::new());
        assert_eq!(n, 2, "only the two STONE cells became GLASS");
        assert_eq!(w.get_block(1, 4, 0), block::DIRT, "the DIRT cell is untouched");
    }

    #[test]
    fn region_walls_sets_only_the_side_faces() {
        let mut w = World::new();
        // 3×1×3 footprint: walls = the outer ring (8 cells), centre hollow.
        let n = region_walls(&mut w, [0, 4, 0], [2, 4, 2], block::STONE, &mut Vec::new());
        assert_eq!(n, 8);
        assert_eq!(w.get_block(1, 4, 1), block::AIR, "centre stays hollow");
        assert_eq!(w.get_block(0, 4, 0), block::STONE, "corner is a wall");
    }

    #[test]
    fn copy_then_paste_reproduces_the_region() {
        let mut w = World::new();
        w.set_block(0, 4, 0, block::STONE);
        w.set_block(1, 4, 0, block::DIRT);
        let clip = region_copy(&w, [0, 4, 0], [1, 4, 0]);
        assert_eq!(clip.dims, [2, 1, 1]);
        // Paste 10 blocks away.
        let n = clipboard_paste(&mut w, &clip, [10, 4, 0], &mut Vec::new());
        assert_eq!(n, 2);
        assert_eq!(w.get_block(10, 4, 0), block::STONE);
        assert_eq!(w.get_block(11, 4, 0), block::DIRT);
    }

    #[test]
    fn stack_repeats_the_region_along_an_axis() {
        let mut w = World::new();
        w.set_block(0, 4, 0, block::STONE); // a 1×1×1 region
        let (n, min, max) = region_stack(&mut w, [0, 4, 0], [0, 4, 0], 0, 2, &mut Vec::new());
        assert_eq!(n, 2, "two copies written");
        assert_eq!(w.get_block(1, 4, 0), block::STONE, "copy 1");
        assert_eq!(w.get_block(2, 4, 0), block::STONE, "copy 2");
        assert_eq!(min, [0, 4, 0]);
        assert_eq!(max, [2, 4, 0], "bounds cover original + both copies");
    }

    #[test]
    fn set_air_over_a_battery_drops_its_power_device() {
        // Audit P7: `/we set air` left the battery's PowerDevice as a ghost.
        let mut w = World::new();
        let mut changed = Vec::new();
        region_set(&mut w, [0, 4, 0], [0, 4, 0], block::BATTERY, &mut changed);
        assert!(w.power_device_at((0, 4, 0)).is_some(), "a written battery gets its device");
        changed.clear();
        let n = region_set(&mut w, [0, 4, 0], [0, 4, 0], block::AIR, &mut changed);
        assert_eq!(n, 1);
        assert_eq!(changed, vec![[0, 4, 0]]);
        assert!(w.power_device_at((0, 4, 0)).is_none(), "no ghost device left behind");
    }

    #[test]
    fn pasted_power_blocks_are_live_devices() {
        // Audit P7: a pasted lamp-and-lever build had no devices behind it.
        let mut w = World::new();
        region_set(&mut w, [0, 4, 0], [0, 4, 0], block::LEVER, &mut Vec::new());
        region_set(&mut w, [1, 4, 0], [1, 4, 0], block::ELECTRIC_LAMP, &mut Vec::new());
        let clip = region_copy(&w, [0, 4, 0], [1, 4, 0]);
        let mut changed = Vec::new();
        let n = clipboard_paste(&mut w, &clip, [10, 4, 0], &mut changed);
        assert_eq!(n, 2);
        assert!(w.power_device_at((10, 4, 0)).is_some(), "pasted lever has a device");
        assert!(w.power_device_at((11, 4, 0)).is_some(), "pasted lamp has a device");
    }

    #[test]
    fn overwriting_a_cell_drops_the_old_blocks_meta_and_entity() {
        let mut w = World::new();
        w.set_block(0, 4, 0, block::STONE);
        w.set_meta((0, 4, 0), 5);
        w.block_entities.insert(
            (0, 4, 0),
            crate::world::BlockEntityData::Chest(Default::default()),
        );
        region_set(&mut w, [0, 4, 0], [0, 4, 0], block::DIRT, &mut Vec::new());
        assert_eq!(w.meta_at(0, 4, 0), 0);
        assert!(!w.block_entities.contains_key(&(0, 4, 0)), "no orphaned chest");
    }
    #[test]
    fn a_huge_region_broadcast_goes_out_in_ordered_packets_under_the_cap() {
        // Review W3 B2: a host `/we` over ~4,300 cells made one StateUpdate
        // over 64 KiB, which every joiner dropped whole.
        use crate::protocol::{self, BlockChange, StateUpdatePacket};
        let mut host = World::new();
        let mut changed = Vec::new();
        // 10,000 cells: 25 × 20 × 20.
        region_set(&mut host, [0, 1, 0], [24, 20, 19], crate::block::STONE, &mut changed);
        assert_eq!(changed.len(), 10_000);
        let mut q = RegionBroadcastQueue::default();
        q.push(&changed);
        let mut sent: Vec<BlockChange> = Vec::new();
        let mut packets = 0;
        while !q.is_empty() {
            let batch = q.next_batch(&host, usize::MAX);
            assert!(!batch.is_empty() && batch.len() <= REGION_BROADCAST_BATCH);
            let pkt = StateUpdatePacket {
                tick: 0,
                players: Vec::new(),
                block_changes: batch.clone(),
                world_time: 0,
                last_acked_input: 0,
                entity_spawns: Vec::new(),
                entity_updates: Vec::new(),
                entity_despawns: Vec::new(),
                reserve_richness: 0.0,
                reserve_target_sats: 0,
                reserve_current_sats: 0,
                rain_ticks_left: 0,
                storm_ticks_left: 0,
                own_hunger: 0,
            };
            let bytes = protocol::serialize_packet(protocol::PacketType::StateUpdate, &pkt);
            assert!((bytes.len() as u64) < protocol::MAX_PACKET_SIZE, "{} bytes", bytes.len());
            let (_, payload) = protocol::deserialize_header(&bytes).unwrap();
            let back: StateUpdatePacket = protocol::safe_deserialize(payload).expect("joiner reads it");
            sent.extend(back.block_changes);
            packets += 1;
        }
        assert!(packets >= 10, "10k cells need several updates, got {packets}");
        let order: Vec<[i32; 3]> = sent.iter().map(|b| [b.x, b.y, b.z]).collect();
        assert_eq!(order, changed, "every change, in order");
        // The joiner ends with exactly the host's blocks.
        let mut joiner = World::new();
        for bc in &sent {
            joiner.apply_remote_block_change(bc);
        }
        for &[x, y, z] in &changed {
            assert_eq!(joiner.get_block(x, y, z), host.get_block(x, y, z));
        }
    }

    #[test]
    fn a_queued_cell_edited_again_broadcasts_its_latest_value() {
        let mut server = World::new();
        let mut changed = Vec::new();
        region_set(&mut server, [0, 1, 0], [3, 1, 0], crate::block::STONE, &mut changed);
        let mut q = RegionBroadcastQueue::default();
        q.push(&changed);
        // A later break of a still-queued cell must not be undone by the queue.
        server.set_block(2, 1, 0, crate::block::AIR);
        let batch = q.next_batch(&server, 2);
        assert_eq!(batch.len(), 2);
        assert_eq!(q.len(), 2);
        let rest = q.next_batch(&server, 10);
        assert_eq!((rest[0].x, rest[0].new_block), (2, crate::block::AIR));
        assert_eq!((rest[1].x, rest[1].new_block), (3, crate::block::STONE));
    }
}
