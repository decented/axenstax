//! Guided build-along **step sequencer** — the pure core of the "paint by
//! numbers / Lego instructions" upgrade to the blueprint build-guide.
//!
//! `build_guide.rs` already answers *"is this cell right?"* (`verify` →
//! Correct / Missing / Wrong) and *"what am I still short of?"*
//! (`remaining_materials`). This module answers the two questions a **guided**
//! build adds on top:
//!
//! 1. **What should I place *now*?** — split the plan's cells into ordered
//!    steps (one block per step, or one horizontal layer per step) and hold a
//!    `current_step` cursor.
//! 2. **Am I done with this step?** — a step advances only when every cell in
//!    it verifies `Correct`. A wrong block reads `Wrong` and the step stands.
//!
//! Pure core: no egui, no renderer, no world access — callers pass in the
//! verified statuses. The ghost render (`game_loop::refresh_build_guide`) and
//! the HUD panel (`hud_ui::draw_build_guide_panel`) call in.
//!
//! Spec: `docs/superpowers/specs/2026-06-23-guided-build-along-schematics-spec.md`.

use crate::block::BlockId;
use crate::build_guide::CellStatus;
use crate::plan::CapturedCell;
use std::collections::BTreeMap;

/// How a laid plan is broken into build steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepMode {
    /// The classic reference ghost — the whole plan at once, with the lowest
    /// unfinished layer glowing. No step cursor.
    #[default]
    Whole,
    /// One cell per step, in buildable order (floor upward). The most guided.
    BlockByBlock,
    /// One horizontal layer (`ry`) per step — the "Lego instructions" reading.
    Layers,
}

impl StepMode {
    /// Player-facing name for the mode (UK English).
    pub fn label(self) -> &'static str {
        match self {
            StepMode::Whole => "whole plan",
            StepMode::BlockByBlock => "block by block",
            StepMode::Layers => "layer by layer",
        }
    }

    /// Parse a mode word off a command argument. `None` for anything else.
    pub fn parse(word: &str) -> Option<StepMode> {
        match word {
            "whole" | "all" | "plan" => Some(StepMode::Whole),
            "block" | "blocks" | "blockbyblock" => Some(StepMode::BlockByBlock),
            "layer" | "layers" | "step" | "steps" => Some(StepMode::Layers),
            _ => None,
        }
    }

    /// Does this mode walk a step cursor? `Whole` does not.
    pub fn is_stepped(self) -> bool {
        !matches!(self, StepMode::Whole)
    }
}

/// Where a cell sits relative to the step the player is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepFocus {
    /// A step already walked past (its cells should all be placed).
    Done,
    /// The step the player is being asked to build right now.
    Current,
    /// Still to come — previewed faintly so the shape reads without shouting.
    Future,
}

/// Split a plan's cells into ordered steps. Each step is a list of **indices
/// into `cells`**, so the caller can index the status list returned by
/// `build_guide::verify` (which is index-aligned with `cells`) directly.
///
/// Ordering mirrors `plan::order_cells_for_build` — `(ry, rx, rz)`, i.e. floor
/// first, then each layer up — so the guided walk-through and the animated
/// auto-builder lay the same plan in the same order.
pub fn plan_steps(cells: &[CapturedCell], mode: StepMode) -> Vec<Vec<usize>> {
    if cells.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..cells.len()).collect();
    order.sort_by_key(|&i| (cells[i].ry, cells[i].rx, cells[i].rz));
    match mode {
        StepMode::Whole => vec![order],
        StepMode::BlockByBlock => order.into_iter().map(|i| vec![i]).collect(),
        StepMode::Layers => {
            let mut steps: Vec<Vec<usize>> = Vec::new();
            let mut current_y: Option<u8> = None;
            for i in order {
                let y = cells[i].ry;
                if current_y != Some(y) {
                    steps.push(Vec::new());
                    current_y = Some(y);
                }
                steps
                    .last_mut()
                    .expect("a step was just pushed")
                    .push(i);
            }
            steps
        }
    }
}

/// Reverse index: cell index → the step it belongs to. Cells not covered by
/// any step (can't happen for steps built by `plan_steps`, but the render path
/// shouldn't panic on a stale list) map to `usize::MAX`, which reads as
/// "future" against any real cursor.
pub fn step_of_cell(steps: &[Vec<usize>], cell_count: usize) -> Vec<usize> {
    let mut out = vec![usize::MAX; cell_count];
    for (step_index, step) in steps.iter().enumerate() {
        for &cell in step {
            if cell < cell_count {
                out[cell] = step_index;
            }
        }
    }
    out
}

/// Is every cell of `step` verified `Correct`? An empty step counts as done.
pub fn step_complete(step: &[usize], statuses: &[([i32; 3], CellStatus)]) -> bool {
    step.iter().all(|&i| {
        statuses
            .get(i)
            .is_some_and(|(_, s)| matches!(s, CellStatus::Correct))
    })
}

/// Advance the cursor past every completed step, starting at `current`.
/// Returns the new cursor. Equal to `steps.len()` when the whole plan is done.
///
/// A step with a `Wrong` (or still `Missing`) cell stops the walk — that's the
/// "place the right block to move on" rule, and it needs no new validation:
/// it's the same `verify` the static guide already uses.
pub fn advance_step(
    steps: &[Vec<usize>],
    statuses: &[([i32; 3], CellStatus)],
    current: usize,
) -> usize {
    let mut cursor = current.min(steps.len());
    while cursor < steps.len() && step_complete(&steps[cursor], statuses) {
        cursor += 1;
    }
    cursor
}

/// Where `cell_step` sits relative to the cursor.
pub fn focus_of(cell_step: usize, current: usize) -> StepFocus {
    if cell_step == usize::MAX {
        return StepFocus::Future;
    }
    match cell_step.cmp(&current) {
        std::cmp::Ordering::Less => StepFocus::Done,
        std::cmp::Ordering::Equal => StepFocus::Current,
        std::cmp::Ordering::Greater => StepFocus::Future,
    }
}

/// The blocks the **current step** still wants: the plan block of every cell in
/// the step that isn't already `Correct`. Sorted count-desc, then block id —
/// the same ordering `build_guide::remaining_materials` uses for the overall
/// list, so the two read consistently in the panel.
pub fn step_materials(
    step: &[usize],
    cells: &[CapturedCell],
    statuses: &[([i32; 3], CellStatus)],
) -> Vec<(BlockId, u32)> {
    let mut map: BTreeMap<BlockId, u32> = BTreeMap::new();
    for &i in step {
        let Some(cell) = cells.get(i) else { continue };
        let done = statuses
            .get(i)
            .is_some_and(|(_, s)| matches!(s, CellStatus::Correct));
        if !done {
            *map.entry(cell.block_id).or_default() += 1;
        }
    }
    crate::build_guide::sort_counts(map)
}

/// What the player is short of: `needed` minus what `owned(block)` reports.
/// Only genuine shortfalls come back, ordered like the input. Used in Survival
/// so the guide doubles as a "go and gather this" list; in Creative the caller
/// simply doesn't ask.
pub fn shortfall(
    needed: &[(BlockId, u32)],
    owned: impl Fn(BlockId) -> u32,
) -> Vec<(BlockId, u32)> {
    needed
        .iter()
        .filter_map(|&(block, want)| {
            let have = owned(block);
            // `then` (lazy), not `then_some` — the subtraction must not run
            // when the player already has enough (u32 underflow).
            (want > have).then(|| (block, want - have))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::build_guide;

    fn cell(rx: u8, ry: u8, rz: u8, b: BlockId) -> CapturedCell {
        CapturedCell { rx, ry, rz, block_id: b }
    }

    /// A 2×2 floor + a single block on the layer above (an L of 5 cells).
    fn sample() -> Vec<CapturedCell> {
        vec![
            cell(1, 1, 0, block::STONE), // deliberately first in the source list
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(0, 0, 1, block::DIRT),
            cell(1, 0, 1, block::DIRT),
        ]
    }

    #[test]
    fn block_by_block_order_matches_the_animated_builder() {
        let cells = sample();
        let steps = plan_steps(&cells, StepMode::BlockByBlock);
        assert_eq!(steps.len(), cells.len(), "one cell per step");
        let stepped: Vec<CapturedCell> = steps.iter().map(|s| cells[s[0]]).collect();
        // Same order the auto-builder places in — guided and automatic agree.
        assert_eq!(stepped, crate::plan::order_cells_for_build(&cells));
    }

    #[test]
    fn layers_group_by_ry_floor_first() {
        let cells = sample();
        let steps = plan_steps(&cells, StepMode::Layers);
        assert_eq!(steps.len(), 2, "two distinct Y layers");
        assert_eq!(steps[0].len(), 4, "the 2×2 floor is one step");
        assert!(steps[0].iter().all(|&i| cells[i].ry == 0));
        assert_eq!(steps[1].len(), 1);
        assert_eq!(cells[steps[1][0]].ry, 1, "the upper layer comes second");
    }

    #[test]
    fn whole_mode_is_a_single_step_and_empty_plans_have_none() {
        assert_eq!(plan_steps(&sample(), StepMode::Whole).len(), 1);
        assert!(plan_steps(&[], StepMode::Layers).is_empty());
        assert!(plan_steps(&[], StepMode::BlockByBlock).is_empty());
    }

    #[test]
    fn step_of_cell_maps_every_cell_and_tolerates_gaps() {
        let cells = sample();
        let steps = plan_steps(&cells, StepMode::Layers);
        let map = step_of_cell(&steps, cells.len());
        assert_eq!(map.len(), cells.len());
        assert_eq!(map[0], 1, "the ry=1 cell is in the second step");
        assert!(map[1..].iter().all(|&s| s == 0));
        // A stale step list referencing a trimmed plan must not panic.
        let map = step_of_cell(&steps, 2);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn advance_walks_only_past_fully_correct_steps() {
        let cells = sample();
        let steps = plan_steps(&cells, StepMode::Layers);
        // Floor: three of four placed → the step stands.
        let mut statuses = vec![
            ([0, 1, 0], CellStatus::Missing), // cells[0] — upper layer
            ([0, 0, 0], CellStatus::Correct),
            ([1, 0, 0], CellStatus::Correct),
            ([0, 0, 1], CellStatus::Correct),
            ([1, 0, 1], CellStatus::Missing),
        ];
        assert_eq!(advance_step(&steps, &statuses, 0), 0, "floor unfinished");
        // Last floor cell placed → advance to the upper layer.
        statuses[4].1 = CellStatus::Correct;
        assert_eq!(advance_step(&steps, &statuses, 0), 1);
        // Upper layer done → the cursor runs off the end = complete.
        statuses[0].1 = CellStatus::Correct;
        assert_eq!(advance_step(&steps, &statuses, 1), steps.len());
    }

    #[test]
    fn a_wrong_block_does_not_advance_the_step() {
        let cells = sample();
        let steps = plan_steps(&cells, StepMode::BlockByBlock);
        // First step = the (0,0,0) stone. Put dirt there instead.
        let wrong_index = steps[0][0];
        let mut statuses: Vec<([i32; 3], CellStatus)> =
            cells.iter().map(|_| ([0, 0, 0], CellStatus::Missing)).collect();
        statuses[wrong_index].1 = CellStatus::Wrong;
        assert_eq!(advance_step(&steps, &statuses, 0), 0, "Wrong holds the step");
        assert!(!step_complete(&steps[0], &statuses));
        statuses[wrong_index].1 = CellStatus::Correct;
        assert_eq!(advance_step(&steps, &statuses, 0), 1, "fixed → advances");
    }

    #[test]
    fn focus_splits_done_current_and_future() {
        assert_eq!(focus_of(0, 2), StepFocus::Done);
        assert_eq!(focus_of(2, 2), StepFocus::Current);
        assert_eq!(focus_of(5, 2), StepFocus::Future);
        assert_eq!(focus_of(usize::MAX, 2), StepFocus::Future);
    }

    #[test]
    fn step_materials_lists_only_what_this_step_still_needs() {
        let cells = sample();
        let steps = plan_steps(&cells, StepMode::Layers);
        let statuses = vec![
            ([0, 1, 0], CellStatus::Missing),
            ([0, 0, 0], CellStatus::Correct), // stone already down
            ([1, 0, 0], CellStatus::Missing), // stone still wanted
            ([0, 0, 1], CellStatus::Missing), // dirt
            ([1, 0, 1], CellStatus::Missing), // dirt
        ];
        let mats = step_materials(&steps[0], &cells, &statuses);
        assert_eq!(mats, vec![(block::DIRT, 2), (block::STONE, 1)]);
    }

    #[test]
    fn shortfall_reports_only_what_is_actually_missing() {
        let needed = vec![(block::STONE, 5), (block::DIRT, 2)];
        let short = shortfall(&needed, |b| if b == block::STONE { 3 } else { 9 });
        assert_eq!(short, vec![(block::STONE, 2)], "dirt is covered");
        assert!(shortfall(&needed, |_| 99).is_empty());
    }

    /// End-to-end walk of the guided flow against a stand-in world: lay the
    /// plan, place each step's blocks (one wrong turn included), and check the
    /// cursor tracks exactly as a player would see it in the HUD.
    #[test]
    fn guided_walkthrough_advances_step_by_step_against_a_world() {
        use std::collections::HashMap;
        let cells = sample();
        let origin = [10, 64, 5];
        let steps = plan_steps(&cells, StepMode::BlockByBlock);
        let mut world: HashMap<(i32, i32, i32), BlockId> = HashMap::new();
        let mut cursor = 0usize;

        let verify = |world: &HashMap<(i32, i32, i32), BlockId>| {
            build_guide::verify(&cells, origin, |x, y, z| {
                *world.get(&(x, y, z)).unwrap_or(&block::AIR)
            })
        };

        // Nothing placed → step 1 of 5, nothing done.
        let statuses = verify(&world);
        assert_eq!(advance_step(&steps, &statuses, cursor), 0);
        assert_eq!(steps.len(), 5);

        // Place the wrong block on step 1 → Wrong, no advance.
        let first = cells[steps[0][0]];
        let first_pos = (
            origin[0] + first.rx as i32,
            origin[1] + first.ry as i32,
            origin[2] + first.rz as i32,
        );
        world.insert(first_pos, block::DIRT);
        let statuses = verify(&world);
        assert_eq!(statuses[steps[0][0]].1, CellStatus::Wrong);
        assert_eq!(advance_step(&steps, &statuses, cursor), 0);

        // Clear the mistake, then walk the whole plan a step at a time.
        world.remove(&first_pos);
        for (i, step) in steps.iter().enumerate() {
            let statuses = verify(&world);
            cursor = advance_step(&steps, &statuses, cursor);
            // The cursor always points at the first not-yet-built step.
            assert_eq!(cursor, i, "cursor should sit on step {i}");
            assert!(
                !step_materials(step, &cells, &statuses).is_empty(),
                "the current step still wants its block"
            );
            let c = cells[step[0]];
            world.insert(
                (
                    origin[0] + c.rx as i32,
                    origin[1] + c.ry as i32,
                    origin[2] + c.rz as i32,
                ),
                c.block_id,
            );
        }
        let statuses = verify(&world);
        cursor = advance_step(&steps, &statuses, cursor);
        assert_eq!(cursor, steps.len(), "walking every step completes the build");
        assert!(step_materials(&steps[0], &cells, &statuses).is_empty());
    }
}
