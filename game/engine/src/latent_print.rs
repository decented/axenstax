//! Spec 38 (Blueprint / Cyanotype, 2026-05-27) — Latent Print block-entity.
//!
//! A LATENT_PRINT is the world-side state of a captured-but-not-yet-developed
//! Plan. The player right-clicks the ground holding a Latent
//! `Item::Plan(_)` to lay it flat; the engine places the LATENT_PRINT
//! block and stamps a `BlockEntityData::LatentPrint(LatentPrintData)`
//! carrying the PlanData. From then on, the per-tick `tick_develop`
//! driver advances the embedded PlanData's `develop_state` whenever the
//! block sees full sky-light during daytime — no roof, no shade, no
//! night. After `DEVELOP_THRESHOLD_TICKS` of accumulated direct sun the
//! state flips `Latent → Developed` and the Plan is ready to use. The
//! player retrieves the Plan by right-clicking the block again.
//!
//! Pure-function-testable; no egui surface. Wires into the game-loop
//! via the block-entity machinery in `world.rs` and the place/retrieve
//! handlers in `game_loop.rs`.

use serde::{Deserialize, Serialize};

use crate::plan::PlanData;
use crate::world::World;

/// Block-entity state for a LATENT_PRINT block. The develop_state lives
/// on `PlanData` itself (see Spec 38 data model) — this wrapper just
/// nests the Plan so the block-entity machinery can serialise it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatentPrintData {
    pub plan_data: PlanData,
}

impl LatentPrintData {
    /// No production caller found (only this file's own tests) — the live
    /// Latent-Plan placement path may construct `LatentPrintData` via a
    /// struct literal directly, or may not exist yet.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(plan_data: PlanData) -> Self {
        Self { plan_data }
    }
}

/// Pure helper — does the block at `(x, y, z)` currently catch enough
/// direct sun to develop a cyanotype? Spec 38 §"Develop": effective
/// sky-light must be full (no roof) at the cell directly above AND it
/// must be daytime per the existing `brigand::is_night_at` threshold
/// (shared so cyanotype + brigand AI + mob-spawn agree on when night
/// begins). Returns true iff both gates pass.
pub fn is_developing_condition(world: &World, world_time: u32, x: i32, y: i32, z: i32) -> bool {
    if crate::brigand::is_night_at(world_time) {
        return false;
    }
    // Sky light is propagated from the topmost open cell downward and
    // attenuates by 1 per opaque block crossed. The cell directly above
    // the Latent Print is the one the sun would actually strike. 15 =
    // unobstructed sky (no roof above the column).
    world.sky_light_at(x, y + 1, z) >= 15
}

/// Per-tick driver — walks every LATENT_PRINT block-entity in the world
/// and advances its embedded `PlanData.develop_state` by one sun-tick
/// when conditions allow. Returns the positions where this tick caused
/// a `Latent → Developed` transition, so the caller can react (emit a
/// particle, broadcast a state-update, log it).
///
/// Snapshots the key set first to avoid borrowing `world.block_entities`
/// twice — same pattern as the furnace + auction sweeps.
pub fn tick_develop(world: &mut World, world_time: u32) -> Vec<(i32, i32, i32)> {
    let mut transitions = Vec::new();
    let keys: Vec<(i32, i32, i32)> = world
        .block_entities
        .iter()
        .filter_map(|(&pos, be)| match be {
            crate::world::BlockEntityData::LatentPrint(_) => Some(pos),
            _ => None,
        })
        .collect();
    for pos in keys {
        if !is_developing_condition(world, world_time, pos.0, pos.1, pos.2) {
            continue;
        }
        if let Some(crate::world::BlockEntityData::LatentPrint(data)) =
            world.block_entities.get_mut(&pos)
            && data.plan_data.develop_state.advance_sun_tick() {
                transitions.push(pos);
            }
    }
    transitions
}

/// Per-tick driver — advances every laid Blueprint face-attachment whose
/// embedded plan is still `Latent` and which currently catches full direct
/// sun (via `is_developing_condition` on the floor cell the attachment sits
/// on). Returns the `(pos, face_idx)` of attachments that flipped
/// `Latent → Developed` this tick (so the caller can react / rebuild the
/// chunk so the decal recolours pale → blue). Mirrors `tick_develop`.
///
/// Snapshots the candidate `(pos, face_idx)` keys in one immutable pass to
/// avoid borrowing `world.face_attachments` while mutating it — same
/// snapshot-then-mutate pattern as `tick_develop`.
pub fn tick_develop_attachments(
    world: &mut World,
    world_time: u32,
) -> Vec<((i32, i32, i32), usize)> {
    let mut transitions = Vec::new();
    // Pass 1 (immutable): collect every laid Blueprint attachment still Latent.
    let candidates: Vec<((i32, i32, i32), usize)> = world
        .iter_face_attachments()
        .flat_map(|(pos, faces)| {
            faces.iter().enumerate().filter_map(move |(face_idx, slot)| match slot {
                Some(crate::world::FaceAttachment::Blueprint(plan))
                    if matches!(plan.develop_state, crate::plan::DevelopState::Latent { .. }) =>
                {
                    Some((pos, face_idx))
                }
                _ => None,
            })
        })
        .collect();
    // Pass 2 (mutable): gate each on direct sun + advance one sun-tick.
    for (pos, face_idx) in candidates {
        if !is_developing_condition(world, world_time, pos.0, pos.1, pos.2) {
            continue;
        }
        if let Some(crate::world::FaceAttachment::Blueprint(plan)) =
            world.face_attachment_at_mut(pos, face_idx)
            && plan.develop_state.advance_sun_tick() {
                transitions.push((pos, face_idx));
            }
    }
    transitions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Face;
    use crate::plan::{DevelopState, PlanData, DEVELOP_THRESHOLD_TICKS};
    use crate::world::{BlockEntityData, FaceAttachment};

    // World-time helpers (cf. `brigand::is_night_at` + `camera::compute_sun`):
    // tick 0 is midnight, tick 6000 is sunrise (elevation = 0, brightness
    // = 0.15 = below the 0.3 night threshold!), tick 12000 is peak day
    // (brightness = 1.0). 18000 is sunset (back to the night band).
    const DAYTIME: u32 = 12000;
    const NIGHT: u32 = 0;

    /// Latent-print fixture: pre-stamp the block-entity at a position
    /// and fill the column above with full sky-light so
    /// `is_developing_condition` returns true at daytime. Returns the
    /// world for further mutation.
    fn world_with_open_sky_latent_at(pos: (i32, i32, i32)) -> World {
        let mut world = World::new();
        // Sky-light propagation happens on world-gen / lighting passes;
        // for these unit tests we directly set the value at the cell
        // above the latent print.
        world.set_sky_light_at(pos.0, pos.1 + 1, pos.2, 15);
        let plan = {
            let mut p = PlanData::debug_3x3_stone();
            p.develop_state = DevelopState::Latent { exposure_ticks: 0 };
            p
        };
        world
            .block_entities
            .insert(pos, BlockEntityData::LatentPrint(LatentPrintData::new(plan)));
        world
    }

    #[test]
    fn is_developing_condition_true_at_full_sky_during_daytime() {
        let world = world_with_open_sky_latent_at((0, 70, 0));
        assert!(is_developing_condition(&world, DAYTIME, 0, 70, 0));
    }

    #[test]
    fn is_developing_condition_false_at_night_even_with_full_sky() {
        let world = world_with_open_sky_latent_at((0, 70, 0));
        assert!(!is_developing_condition(&world, NIGHT, 0, 70, 0));
    }

    #[test]
    fn is_developing_condition_false_with_roof() {
        let mut world = World::new();
        // Set sky-light at the cell above to 0 — proxy for a roof
        // blocking the column.
        world.set_sky_light_at(0, 71, 0, 0);
        assert!(!is_developing_condition(&world, DAYTIME, 0, 70, 0));
    }

    #[test]
    fn is_developing_condition_false_at_partial_sky_light() {
        let mut world = World::new();
        // 14 = something dims the column slightly (e.g. a leaf canopy).
        // The spec requires FULL daylight.
        world.set_sky_light_at(0, 71, 0, 14);
        assert!(!is_developing_condition(&world, DAYTIME, 0, 70, 0));
    }

    #[test]
    fn tick_develop_advances_exposure_during_daytime() {
        let mut world = world_with_open_sky_latent_at((0, 70, 0));
        let transitions = tick_develop(&mut world, DAYTIME);
        assert!(transitions.is_empty(), "single tick should not flip");
        let data = world
            .block_entities
            .get(&(0, 70, 0))
            .and_then(BlockEntityData::as_latent_print)
            .expect("latent print still present");
        assert_eq!(
            data.plan_data.develop_state,
            DevelopState::Latent { exposure_ticks: 1 }
        );
    }

    #[test]
    fn tick_develop_no_op_at_night() {
        let mut world = world_with_open_sky_latent_at((0, 70, 0));
        tick_develop(&mut world, NIGHT);
        let data = world
            .block_entities
            .get(&(0, 70, 0))
            .and_then(BlockEntityData::as_latent_print)
            .expect("latent print still present");
        assert_eq!(
            data.plan_data.develop_state,
            DevelopState::Latent { exposure_ticks: 0 },
            "night tick must not progress develop"
        );
    }

    #[test]
    fn tick_develop_flips_at_threshold_and_reports_position() {
        let mut world = world_with_open_sky_latent_at((0, 70, 0));
        // Force exposure to one tick below the threshold so this tick
        // is the transition.
        if let Some(BlockEntityData::LatentPrint(d)) = world.block_entities.get_mut(&(0, 70, 0)) {
            d.plan_data.develop_state =
                DevelopState::Latent { exposure_ticks: DEVELOP_THRESHOLD_TICKS - 1 };
        }
        let transitions = tick_develop(&mut world, DAYTIME);
        assert_eq!(transitions, vec![(0, 70, 0)],
            "transition tick should be reported back to the caller");
        let data = world
            .block_entities
            .get(&(0, 70, 0))
            .and_then(BlockEntityData::as_latent_print)
            .expect("latent print still present after develop");
        assert_eq!(data.plan_data.develop_state, DevelopState::Developed);
    }

    #[test]
    fn tick_develop_idempotent_once_developed() {
        let mut world = World::new();
        world.set_sky_light_at(0, 71, 0, 15);
        let mut plan = PlanData::debug_3x3_stone();
        plan.develop_state = DevelopState::Developed;
        world.block_entities.insert(
            (0, 70, 0),
            BlockEntityData::LatentPrint(LatentPrintData::new(plan)),
        );
        let transitions = tick_develop(&mut world, DAYTIME);
        assert!(transitions.is_empty(), "already-developed prints must not re-transition");
    }

    #[test]
    fn tick_develop_pauses_during_night_segment() {
        // Drive a Latent print part-way through, then pretend the day
        // rolls into night — exposure must NOT advance. This mirrors
        // the spec's "drag it to the sun, then carry it home overnight"
        // story.
        let mut world = world_with_open_sky_latent_at((0, 70, 0));
        for _ in 0..50 {
            tick_develop(&mut world, DAYTIME);
        }
        let daytime_exposure = {
            let d = world
                .block_entities
                .get(&(0, 70, 0))
                .and_then(BlockEntityData::as_latent_print)
                .unwrap();
            match d.plan_data.develop_state {
                DevelopState::Latent { exposure_ticks } => exposure_ticks,
                _ => panic!("not developed yet"),
            }
        };
        assert_eq!(daytime_exposure, 50);
        for _ in 0..200 {
            tick_develop(&mut world, NIGHT);
        }
        let night_exposure = {
            let d = world
                .block_entities
                .get(&(0, 70, 0))
                .and_then(BlockEntityData::as_latent_print)
                .unwrap();
            match d.plan_data.develop_state {
                DevelopState::Latent { exposure_ticks } => exposure_ticks,
                _ => panic!("not developed yet"),
            }
        };
        assert_eq!(night_exposure, daytime_exposure,
            "night ticks must not progress develop");
    }

    // --- Phase E: laid-attachment develop driver ---------------------------

    /// Lay a Blueprint attachment on the Top face of a floor block at `pos`
    /// with the given develop_state, and open the sky above it so
    /// `is_developing_condition` passes at daytime.
    fn world_with_open_sky_attachment_at(
        pos: (i32, i32, i32),
        state: DevelopState,
    ) -> World {
        let mut world = World::new();
        world.set_sky_light_at(pos.0, pos.1 + 1, pos.2, 15);
        let plan = {
            let mut p = PlanData::debug_3x3_stone();
            p.develop_state = state;
            Box::new(p)
        };
        world.set_face_attachment(pos, Face::Top.index(), FaceAttachment::Blueprint(plan));
        world
    }

    fn attachment_state(world: &World, pos: (i32, i32, i32)) -> DevelopState {
        match world.face_attachment_at(pos, Face::Top.index()) {
            Some(FaceAttachment::Blueprint(plan)) => plan.develop_state.clone(),
            other => panic!("expected a Blueprint attachment, got {other:?}"),
        }
    }

    #[test]
    fn tick_develop_attachments_advances_under_open_sky_daytime() {
        let mut world =
            world_with_open_sky_attachment_at((0, 70, 0), DevelopState::Latent { exposure_ticks: 0 });
        let transitions = tick_develop_attachments(&mut world, DAYTIME);
        assert!(transitions.is_empty(), "single tick should not flip");
        assert_eq!(
            attachment_state(&world, (0, 70, 0)),
            DevelopState::Latent { exposure_ticks: 1 }
        );
    }

    #[test]
    fn tick_develop_attachments_no_op_at_night() {
        let mut world =
            world_with_open_sky_attachment_at((0, 70, 0), DevelopState::Latent { exposure_ticks: 0 });
        let transitions = tick_develop_attachments(&mut world, NIGHT);
        assert!(transitions.is_empty());
        assert_eq!(
            attachment_state(&world, (0, 70, 0)),
            DevelopState::Latent { exposure_ticks: 0 },
            "night ticks must not progress develop"
        );
    }

    #[test]
    fn tick_develop_attachments_flips_at_threshold_and_reports() {
        let mut world = world_with_open_sky_attachment_at(
            (0, 70, 0),
            DevelopState::Latent { exposure_ticks: DEVELOP_THRESHOLD_TICKS - 1 },
        );
        let transitions = tick_develop_attachments(&mut world, DAYTIME);
        assert_eq!(
            transitions,
            vec![((0, 70, 0), Face::Top.index())],
            "transition tick should be reported back to the caller"
        );
        assert_eq!(attachment_state(&world, (0, 70, 0)), DevelopState::Developed);
    }

    #[test]
    fn tick_develop_attachments_ignores_developed() {
        let mut world =
            world_with_open_sky_attachment_at((0, 70, 0), DevelopState::Developed);
        let transitions = tick_develop_attachments(&mut world, DAYTIME);
        assert!(transitions.is_empty(), "developed attachments must not re-transition");
        assert_eq!(attachment_state(&world, (0, 70, 0)), DevelopState::Developed);
    }
}
