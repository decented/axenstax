//! Blueprint build-guide (#9) — the **reverse** of plan capture: project a
//! captured `PlanData` as a ghost at a chosen origin so you can rebuild it
//! block-by-block, with a **material list** (what blocks it needs) and a
//! **verifier** (per-cell correct / wrong / missing vs the live world).
//!
//! Pure core here (counts + verification); the render (ghost markers coloured by
//! status) + the `/buildguide` command + the HUD panel call in.
//!
//! Spec: `docs/foundations/2026-06-16-blueprint-build-guide.md`.

use crate::block;
use crate::block::BlockId;
use crate::plan::CapturedCell;
use std::collections::BTreeMap;

/// How a placed cell compares to the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellStatus {
    /// The world block matches the plan.
    Correct,
    /// The world cell is air — still to place.
    Missing,
    /// The world holds a *different* (non-air) block than the plan wants.
    Wrong,
}

/// Active build-guide state (kept on `GameState`): the plan's cells + the world
/// origin they're anchored to, plus the guided-build step cursor.
///
/// Transient by design — nothing here is saved. A guide re-lays in one click
/// from the Plan item still in the player's bag, so there's no reason to spend
/// a `WorldSave` field on it.
#[derive(Clone)]
pub struct BuildGuide {
    pub name: String,
    pub origin: [i32; 3],
    pub cells: Vec<CapturedCell>,
    /// How the plan is broken into steps (whole plan / block / layer).
    pub mode: crate::build_steps::StepMode,
    /// Cell indices per step, in build order. Derived from `cells` + `mode`.
    pub steps: Vec<Vec<usize>>,
    /// Which step the player is on. `steps.len()` = every step done.
    pub current_step: usize,
}

impl BuildGuide {
    /// Lay a plan as a guide in the given step mode, computing the step split
    /// once (the cells never change for the life of a guide).
    pub fn new(
        name: String,
        origin: [i32; 3],
        cells: Vec<CapturedCell>,
        mode: crate::build_steps::StepMode,
    ) -> Self {
        let steps = crate::build_steps::plan_steps(&cells, mode);
        BuildGuide { name, origin, cells, mode, steps, current_step: 0 }
    }

    /// Switch step mode on an active guide (the mode picker). Recomputes the
    /// steps and rewinds the cursor; the next verify pass walks it straight
    /// back to the first unfinished step, so no progress is lost.
    pub fn set_mode(&mut self, mode: crate::build_steps::StepMode) {
        self.mode = mode;
        self.steps = crate::build_steps::plan_steps(&self.cells, mode);
        self.current_step = 0;
    }
}

/// The blocks a plan needs, `(block, count)` sorted by count desc then id.
/// No production caller — tested directly.
#[cfg_attr(not(test), allow(dead_code))]
pub fn material_list(cells: &[CapturedCell]) -> Vec<(BlockId, u32)> {
    let mut map: BTreeMap<BlockId, u32> = BTreeMap::new();
    for c in cells {
        *map.entry(c.block_id).or_default() += 1;
    }
    sort_counts(map)
}

/// Compare one cell's wanted block to what's actually in the world.
pub fn verify_cell(plan_block: BlockId, world_block: BlockId) -> CellStatus {
    if world_block == plan_block {
        CellStatus::Correct
    } else if world_block == block::AIR {
        CellStatus::Missing
    } else {
        CellStatus::Wrong
    }
}

/// Verify every cell against the world via `get(x,y,z)`. Returns each cell's
/// world position + status (cells are anchored at `origin`).
pub fn verify(
    cells: &[CapturedCell],
    origin: [i32; 3],
    get: impl Fn(i32, i32, i32) -> BlockId,
) -> Vec<([i32; 3], CellStatus)> {
    cells
        .iter()
        .map(|c| {
            let pos = [
                origin[0] + c.rx as i32,
                origin[1] + c.ry as i32,
                origin[2] + c.rz as i32,
            ];
            let status = verify_cell(c.block_id, get(pos[0], pos[1], pos[2]));
            (pos, status)
        })
        .collect()
}

/// (correct, missing, wrong) counts over a verified cell list.
pub fn summarize(statuses: &[([i32; 3], CellStatus)]) -> (u32, u32, u32) {
    let mut out = (0, 0, 0);
    for (_, s) in statuses {
        match s {
            CellStatus::Correct => out.0 += 1,
            CellStatus::Missing => out.1 += 1,
            CellStatus::Wrong => out.2 += 1,
        }
    }
    out
}

/// Build-along walk-through: the lowest Y layer that still has a missing block —
/// the "current step" you should be placing now. `None` when nothing is missing
/// (complete, or only Wrong blocks remain to fix). Wrong blocks don't count as
/// the current layer; they're flagged separately as mistakes to correct.
pub fn current_layer(statuses: &[([i32; 3], CellStatus)]) -> Option<i32> {
    statuses
        .iter()
        .filter(|(_, s)| matches!(s, CellStatus::Missing))
        .map(|(pos, _)| pos[1])
        .min()
}

/// `(1-based index, total)` of the current build layer among all distinct Y
/// layers in the guide — feeds the "Layer N of M" step indicator. `None` once
/// there's nothing left to place.
pub fn layer_progress(statuses: &[([i32; 3], CellStatus)]) -> Option<(usize, usize)> {
    let current = current_layer(statuses)?;
    let mut ys: Vec<i32> = statuses.iter().map(|(p, _)| p[1]).collect();
    ys.sort_unstable();
    ys.dedup();
    let idx = ys.iter().position(|&y| y == current).map(|i| i + 1)?;
    Some((idx, ys.len()))
}

/// The blocks still needed (plan block of every not-yet-Correct cell), sorted.
pub fn remaining_materials(
    cells: &[CapturedCell],
    origin: [i32; 3],
    get: impl Fn(i32, i32, i32) -> BlockId,
) -> Vec<(BlockId, u32)> {
    let mut map: BTreeMap<BlockId, u32> = BTreeMap::new();
    for c in cells {
        let w = get(
            origin[0] + c.rx as i32,
            origin[1] + c.ry as i32,
            origin[2] + c.rz as i32,
        );
        if w != c.block_id {
            *map.entry(c.block_id).or_default() += 1;
        }
    }
    sort_counts(map)
}

/// `(block, count)` map → vec sorted by count desc, then block id asc.
/// Shared with `build_steps::step_materials` so the per-step list and the
/// overall "still need" list read in the same order.
pub(crate) fn sort_counts(map: BTreeMap<BlockId, u32>) -> Vec<(BlockId, u32)> {
    let mut v: Vec<(BlockId, u32)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(rx: u8, ry: u8, rz: u8, b: BlockId) -> CapturedCell {
        CapturedCell { rx, ry, rz, block_id: b }
    }

    #[test]
    fn material_list_counts_by_block_sorted_desc() {
        let cells = vec![
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(2, 0, 0, block::DIRT),
            cell(0, 1, 0, block::STONE),
        ];
        let ml = material_list(&cells);
        assert_eq!(ml, vec![(block::STONE, 3), (block::DIRT, 1)]);
    }

    #[test]
    fn verify_cell_classifies_correct_missing_wrong() {
        assert_eq!(verify_cell(block::STONE, block::STONE), CellStatus::Correct);
        assert_eq!(verify_cell(block::STONE, block::AIR), CellStatus::Missing);
        assert_eq!(verify_cell(block::STONE, block::DIRT), CellStatus::Wrong);
    }

    #[test]
    fn verify_anchors_cells_at_origin_and_reads_world() {
        let cells = vec![cell(0, 0, 0, block::STONE), cell(1, 0, 0, block::STONE)];
        // World: the first cell placed correctly, the second still air.
        let get = |x: i32, y: i32, z: i32| -> BlockId {
            if (x, y, z) == (10, 64, 5) {
                block::STONE
            } else {
                block::AIR
            }
        };
        let v = verify(&cells, [10, 64, 5], get);
        assert_eq!(v[0], ([10, 64, 5], CellStatus::Correct));
        assert_eq!(v[1], ([11, 64, 5], CellStatus::Missing));
    }

    #[test]
    fn summarize_counts_each_status() {
        let v = vec![
            ([0, 0, 0], CellStatus::Correct),
            ([1, 0, 0], CellStatus::Missing),
            ([2, 0, 0], CellStatus::Missing),
            ([3, 0, 0], CellStatus::Wrong),
        ];
        assert_eq!(summarize(&v), (1, 2, 1));
    }

    #[test]
    fn current_layer_is_lowest_missing_y_and_walks_up() {
        // Layer 64 fully placed, layer 65 still missing → current step is 65.
        let v = vec![
            ([0, 64, 0], CellStatus::Correct),
            ([1, 64, 0], CellStatus::Correct),
            ([0, 65, 0], CellStatus::Missing),
            ([0, 66, 0], CellStatus::Missing),
        ];
        assert_eq!(current_layer(&v), Some(65));
        assert_eq!(layer_progress(&v), Some((2, 3))); // layer 2 of 3 distinct Ys
    }

    #[test]
    fn current_layer_none_when_only_wrong_or_complete() {
        // Wrong blocks are mistakes, not the next step — they don't set a layer.
        let only_wrong = vec![([0, 64, 0], CellStatus::Wrong)];
        assert_eq!(current_layer(&only_wrong), None);
        assert_eq!(layer_progress(&only_wrong), None);
        let complete = vec![([0, 64, 0], CellStatus::Correct)];
        assert_eq!(current_layer(&complete), None);
    }

    /// The guided walk-through against a **real `World`** — the closest cheap
    /// seam to `game_loop::refresh_build_guide` (the guide lives on the client
    /// `GameState`, which has no headless harness that isn't GPU-gated).
    #[test]
    fn guide_steps_advance_against_a_real_world_and_survive_a_mode_switch() {
        use crate::build_steps::{StepMode, advance_step};
        use crate::world::World;
        let cells = vec![
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(0, 1, 0, block::DIRT),
        ];
        let origin = [4, 70, 9];
        let mut world = World::new();
        let mut guide =
            BuildGuide::new("Hut".to_string(), origin, cells, StepMode::Layers);
        assert_eq!(guide.steps.len(), 2, "floor + one course above it");

        let mut walk = |world: &World, guide: &mut BuildGuide| {
            let statuses =
                verify(&guide.cells, guide.origin, |x, y, z| world.get_block(x, y, z));
            guide.current_step = advance_step(&guide.steps, &statuses, guide.current_step);
            statuses
        };

        walk(&world, &mut guide);
        assert_eq!(guide.current_step, 0, "nothing placed → still on the floor");

        // Lay the floor → the layer step completes and the cursor moves up.
        world.set_block(4, 70, 9, block::STONE);
        world.set_block(5, 70, 9, block::STONE);
        walk(&world, &mut guide);
        assert_eq!(guide.current_step, 1);

        // Mode picker mid-build: the cursor rewinds, then the very next verify
        // pass walks it back past the work already done — no progress lost.
        guide.set_mode(StepMode::BlockByBlock);
        assert_eq!(guide.current_step, 0);
        assert_eq!(guide.steps.len(), 3);
        walk(&world, &mut guide);
        assert_eq!(guide.current_step, 2, "both floor blocks done");

        // A wrong block reads Wrong and holds the step.
        world.set_block(4, 71, 9, block::STONE);
        let statuses = walk(&world, &mut guide);
        assert_eq!(statuses[2].1, CellStatus::Wrong);
        assert_eq!(guide.current_step, 2, "a wrong block does not advance");

        // Put it right → the walk finishes.
        world.set_block(4, 71, 9, block::DIRT);
        walk(&world, &mut guide);
        assert_eq!(guide.current_step, guide.steps.len(), "build complete");
    }

    #[test]
    fn remaining_materials_counts_not_yet_correct_cells() {
        let cells = vec![
            cell(0, 0, 0, block::STONE), // correct
            cell(1, 0, 0, block::STONE), // missing
            cell(2, 0, 0, block::DIRT),  // wrong (dirt wanted, stone placed)
        ];
        let get = |x: i32, _y: i32, _z: i32| -> BlockId {
            match x {
                0 => block::STONE, // correct
                2 => block::STONE, // wrong block present
                _ => block::AIR,   // missing
            }
        };
        let rem = remaining_materials(&cells, [0, 0, 0], get);
        // Still need: 1 stone (the missing one) + 1 dirt (the wrong cell).
        assert_eq!(rem, vec![(block::STONE, 1), (block::DIRT, 1)]);
    }
}
