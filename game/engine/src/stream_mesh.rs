//! Meshing for a batch of freshly streamed columns (Spec 03 §10.2, as-built).
//!
//! Streaming used to mesh every column on its own, inline: all its sections,
//! then every section of each loaded 4-neighbour (so hidden faces cull). Up to
//! 30 `build_chunk_meshes` calls per column, ~120 a frame, and a neighbour that
//! loaded in the same batch was meshed again each time one of its neighbours
//! arrived. This module turns that into one plan per batch:
//!
//! * each section is meshed at most once per batch (the batch's own columns
//!   are generated and lit first, then meshed);
//! * all-air sections are skipped (they have no geometry; a stale mesh left
//!   at such a cell is removed instead);
//! * at most `now_budget` of the batch's own sections are built this frame;
//!   the rest, and the already-loaded neighbours' seam sections, go onto the
//!   `dirty_mesh_chunks` queue, which `process_dirty_meshes` drains nearest
//!   first under its own per-frame budget. The seam on a neighbour that has
//!   not caught up is a face that now sits inside terrain (invisible), so a
//!   frame or two of lag is harmless — the same trade edits already make.

use crate::world::{World, MAX_CHUNK_Y};
use ahash::AHashSet;

pub type SectionPos = (i32, i32, i32);

/// How much of a batch is meshed in the frame it loads.
#[derive(Clone, Copy, Debug)]
pub struct MeshPolicy {
    /// Most of the batch's own sections built this frame; the rest are queued.
    pub now_budget: usize,
    /// Build the loaded neighbours' seam sections this frame too, instead of
    /// queueing them (a loading screen hides the cost; in play it would be a
    /// hitch).
    pub seams_now: bool,
}

impl MeshPolicy {
    /// In play: three columns' worth of own sections inline (the 4-column
    /// load budget stays the outer bound), seams and the rest queued.
    pub const STREAMING: Self = Self { now_budget: 18, seams_now: false };
    /// Everything inline. Used behind the loading screen, and in play by
    /// `load_one_column` (a local column a server change lands on), where the
    /// whole cost falls in one frame.
    pub const LOADING: Self = Self { now_budget: usize::MAX, seams_now: true };
}

/// What a section holds, as far as meshing cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SectionKind {
    /// No chunk in the world.
    Absent,
    /// A chunk with only air: no geometry.
    Empty,
    /// A chunk with at least one non-air cell.
    Solid,
}

pub fn section_kind(world: &World, pos: SectionPos) -> SectionKind {
    match world.get_chunk(pos.0, pos.1, pos.2) {
        None => SectionKind::Absent,
        Some(c) if c.is_empty() => SectionKind::Empty,
        Some(_) => SectionKind::Solid,
    }
}

/// What to do about meshes after a batch of columns streamed in.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct StreamMeshPlan {
    /// Build and upload these this frame.
    pub now: Vec<SectionPos>,
    /// Queue these for the budgeted dirty-mesh pass.
    pub later: Vec<SectionPos>,
    /// Present but all-air sections of the batch: drop any mesh held for them.
    pub vacated: Vec<SectionPos>,
}

impl StreamMeshPlan {
    /// Total `build_chunk_meshes` calls this plan will cost, now or later.
    #[cfg(test)]
    pub fn builds(&self) -> usize {
        self.now.len() + self.later.len()
    }
}

/// Plan the meshing of `batch` (columns already generated, lit and marked
/// loaded). `loaded(cx, cz)` says whether a column outside the batch is loaded
/// (and so already meshed); `kind` classifies a section.
pub fn plan_stream_meshes(
    batch: &[(i32, i32)],
    policy: MeshPolicy,
    loaded: impl Fn(i32, i32) -> bool,
    kind: impl Fn(SectionPos) -> SectionKind,
) -> StreamMeshPlan {
    let mut plan = StreamMeshPlan::default();
    let in_batch: AHashSet<(i32, i32)> = batch.iter().copied().collect();
    let mut seen: AHashSet<SectionPos> = AHashSet::new();

    for &(cx, cz) in batch {
        for cy in 0..=MAX_CHUNK_Y {
            let pos = (cx, cy, cz);
            if !seen.insert(pos) {
                continue;
            }
            match kind(pos) {
                SectionKind::Absent => {}
                SectionKind::Empty => plan.vacated.push(pos),
                SectionKind::Solid if plan.now.len() < policy.now_budget => plan.now.push(pos),
                SectionKind::Solid => plan.later.push(pos),
            }
        }
    }
    for &(cx, cz) in batch {
        for (nx, nz) in [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
            if in_batch.contains(&(nx, nz)) || !loaded(nx, nz) {
                continue;
            }
            for cy in 0..=MAX_CHUNK_Y {
                let pos = (nx, cy, nz);
                if kind(pos) == SectionKind::Solid && seen.insert(pos) {
                    if policy.seams_now {
                        plan.now.push(pos);
                    } else {
                        plan.later.push(pos);
                    }
                }
            }
        }
    }
    plan
}

/// Build the meshes for `positions` (CPU only) and hand each to `sink`.
/// Returns how many it built. The one place stream meshing calls
/// `build_chunk_meshes`, so tests can count builds exactly.
pub fn build_sections(
    world: &World,
    registry: &crate::block::BlockRegistry,
    positions: &[SectionPos],
    mut sink: impl FnMut(SectionPos, crate::mesh::ChunkMeshes),
) -> usize {
    for &pos in positions {
        sink(pos, crate::mesh::build_chunk_meshes(pos.0, pos.1, pos.2, world, registry));
    }
    positions.len()
}

impl super::GameState {
    /// Mesh a batch of columns that just finished streaming in (generated or
    /// restored, lit, and in `loaded_columns`) — see the module doc.
    pub(crate) fn mesh_stream_batch(&mut self, batch: &[(i32, i32)], policy: MeshPolicy) {
        if batch.is_empty() {
            return;
        }
        let plan = {
            let (world, loaded) = (&self.world, &self.loaded_columns);
            plan_stream_meshes(
                batch,
                policy,
                |cx, cz| loaded.contains(&(cx, cz)),
                |pos| section_kind(world, pos),
            )
        };
        for &pos in &plan.vacated {
            self.drop_section_meshes(pos);
        }
        let renderer = &mut self.renderer;
        build_sections(&self.world, &self.registry, &plan.now, |pos, meshes| {
            renderer.upload_chunk(pos, &meshes);
        });
        self.dirty_mesh_chunks.extend(plan.later);
    }

    /// Remove every mesh held for one section (all kinds, leaves and glass
    /// included: a vacated section must not keep floating geometry).
    pub(crate) fn drop_section_meshes(&mut self, key: SectionPos) {
        self.renderer.drop_section(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::BiomeGenerator;
    use crate::block::BlockRegistry;

    fn all_solid(_: SectionPos) -> SectionKind {
        SectionKind::Solid
    }

    impl MeshPolicy {
        /// In play, own sections unbudgeted, seams queued (isolates the seam rule).
        const STREAMING_UNBOUNDED: Self = Self { now_budget: usize::MAX, seams_now: false };
    }

    #[test]
    fn behind_the_loading_screen_the_seams_are_built_inline_but_still_once() {
        let batch = [(0, 0), (1, 0)];
        let loaded = |x: i32, z: i32| x == -1 && z == 0 || x == 2 && z == 0;
        let plan = plan_stream_meshes(&batch, MeshPolicy::LOADING, loaded, all_solid);
        assert_eq!(plan.now.len(), 4 * (MAX_CHUNK_Y as usize + 1));
        assert!(plan.later.is_empty());
    }

    /// The pre-batch algorithm: per loaded column, mesh its present sections,
    /// then every present section of each loaded 4-neighbour. Counts calls.
    fn legacy_builds(
        batch: &[(i32, i32)],
        loaded_before: impl Fn(i32, i32) -> bool,
        kind: impl Fn(SectionPos) -> SectionKind,
    ) -> usize {
        let mut loaded: AHashSet<(i32, i32)> = AHashSet::new();
        let mut builds = 0;
        for &(cx, cz) in batch {
            loaded.insert((cx, cz));
            for cy in 0..=MAX_CHUNK_Y {
                if kind((cx, cy, cz)) != SectionKind::Absent {
                    builds += 1;
                }
            }
            for (nx, nz) in [(cx - 1, cz), (cx + 1, cz), (cx, cz - 1), (cx, cz + 1)] {
                if loaded.contains(&(nx, nz)) || loaded_before(nx, nz) {
                    for cy in 0..=MAX_CHUNK_Y {
                        if kind((nx, cy, nz)) != SectionKind::Absent {
                            builds += 1;
                        }
                    }
                }
            }
        }
        builds
    }

    #[test]
    fn each_section_is_planned_once_and_the_batch_is_not_remeshed_by_itself() {
        // A 2x2 batch; the ring round it is already loaded.
        let batch = [(0, 0), (1, 0), (0, 1), (1, 1)];
        let in_batch = |x: i32, z: i32| (0..=1).contains(&x) && (0..=1).contains(&z);
        let loaded = |x: i32, z: i32| !in_batch(x, z) && (-1..=2).contains(&x) && (-1..=2).contains(&z);
        let plan = plan_stream_meshes(&batch, MeshPolicy::STREAMING_UNBOUNDED, loaded, all_solid);
        let sections = (MAX_CHUNK_Y + 1) as usize;
        // 4 own columns + the 8 edge-adjacent neighbours (corners do not touch).
        assert_eq!(plan.builds(), (4 + 8) * sections);
        let mut all: Vec<_> = plan.now.iter().chain(&plan.later).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), plan.builds(), "no section planned twice");
        assert_eq!(plan.now.len(), 4 * sections, "own sections now");
        assert_eq!(plan.later.len(), 8 * sections, "seam sections deferred");
        let old = legacy_builds(&batch, loaded, all_solid);
        assert!(plan.builds() < old, "{} < {old}", plan.builds());
    }

    #[test]
    fn the_now_budget_pushes_the_rest_of_the_batch_to_the_queue() {
        let batch = [(0, 0), (1, 0)];
        let plan = plan_stream_meshes(&batch, MeshPolicy { now_budget: 5, seams_now: false }, |_, _| false, all_solid);
        assert_eq!(plan.now.len(), 5);
        assert_eq!(plan.later.len(), 2 * (MAX_CHUNK_Y as usize + 1) - 5);
        assert!(plan.vacated.is_empty());
    }

    #[test]
    fn air_and_absent_sections_cost_nothing() {
        let kind = |p: SectionPos| match p.1 {
            0 | 1 => SectionKind::Solid,
            2 => SectionKind::Empty,
            _ => SectionKind::Absent,
        };
        let plan = plan_stream_meshes(&[(0, 0)], MeshPolicy::STREAMING_UNBOUNDED, |_, _| true, kind);
        assert_eq!(plan.now, vec![(0, 0, 0), (0, 1, 0)]);
        assert_eq!(plan.vacated, vec![(0, 2, 0)]);
        // Loaded neighbours contribute only their solid sections.
        assert_eq!(plan.later.len(), 4 * 2);
        assert!(plan.later.iter().all(|p| p.1 <= 1));
    }

    #[test]
    fn a_duplicate_column_in_the_batch_is_planned_once() {
        let plan = plan_stream_meshes(&[(3, 3), (3, 3)], MeshPolicy::STREAMING_UNBOUNDED, |_, _| false, all_solid);
        assert_eq!(plan.now.len(), MAX_CHUNK_Y as usize + 1);
    }

    /// CPU-only integration: real generated terrain, real mesher. Counts the
    /// `build_chunk_meshes` calls a 2x2 stream batch costs against the old
    /// per-column algorithm, and checks the all-air sections are not built.
    #[test]
    fn a_real_stream_batch_builds_far_fewer_meshes_than_per_column_streaming() {
        let registry = BlockRegistry::new();
        let biome_gen = BiomeGenerator::new(42);
        let mut world = World::new();
        // 4x4 columns: the outer ring is already loaded, the 2x2 middle streams in.
        let area: Vec<(i32, i32)> = (0..4).flat_map(|x| (0..4).map(move |z| (x, z))).collect();
        let batch: Vec<(i32, i32)> = vec![(1, 1), (2, 1), (1, 2), (2, 2)];
        for &(cx, cz) in &area {
            world.generate_column(cx, cz, &biome_gen);
            crate::lighting::run_initial_pass_for_column(&mut world, cx, cz, &registry);
        }
        let loaded = |x: i32, z: i32| area.contains(&(x, z)) && !batch.contains(&(x, z));
        let kind = |p: SectionPos| section_kind(&world, p);

        let plan = plan_stream_meshes(&batch, MeshPolicy::STREAMING, loaded, kind);
        let old = legacy_builds(&batch, loaded, kind);

        let mut built = Vec::new();
        let n_now = build_sections(&world, &registry, &plan.now, |p, _| built.push(p));
        let n_later = build_sections(&world, &registry, &plan.later, |p, _| built.push(p));
        assert_eq!(n_now + n_later, plan.builds());
        assert!(n_now <= MeshPolicy::STREAMING.now_budget, "per-frame inline builds are bounded: {n_now}");
        built.sort();
        built.dedup();
        assert_eq!(built.len(), n_now + n_later, "every section built exactly once");
        assert!(
            built.iter().all(|&p| kind(p) == SectionKind::Solid),
            "no all-air or absent section is built"
        );
        assert!(
            n_now + n_later < old,
            "batched {} (now {n_now}, queued {n_later}) vs per-column {old}",
            n_now + n_later
        );
        eprintln!("stream batch builds: per-column {old}, batched {} (inline {n_now})", n_now + n_later);
    }
}
