//! Spec 36 Lead tether mechanic (2026-05-28).
//!
//! The Lead's gameplay — right-click on a passive mob attaches it to the
//! player (it follows); right-click on a fence post anchors it (it stays
//! near the post); right-click with an empty hand on a tethered mob
//! detaches it (the Lead returns to the player). Out-of-range
//! tethers snap, dropping the Lead at the mob's position so the player
//! can recover it.
//!
//! Pure helpers + a single `tick_tethers` ECS sweep. The right-click
//! handlers in `game_loop.rs` call the pure helpers + apply the ECS
//! mutations they describe.

use glam::Vec3;

use crate::entity::{Position, Velocity};

/// What a tether's "other end" is fastened to. Two variants:
/// `Player(pidx)` — mob follows the player; `Post(pos)` — mob stays
/// near a FENCE_POST block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TetherTarget {
    Player(usize),
    Post([i32; 3]),
}

/// ECS component on a tethered mob. The lone field is the target;
/// distance + tick state are derived per-frame in `tick_tethers` so
/// adding the component is a zero-cost ECS insert.
#[derive(Clone, Copy, Debug)]
pub struct Tethered {
    pub target: TetherTarget,
}

/// Anchor-position resolution for a tether target. Returns the world-
/// space Vec3 the mob should converge toward, or `None` if the player
/// index is out of range (defensive — caller drops the tether in that
/// case).
pub fn anchor_position(target: TetherTarget, player_positions: &[Vec3]) -> Option<Vec3> {
    match target {
        TetherTarget::Player(pidx) => player_positions.get(pidx).copied(),
        TetherTarget::Post([x, y, z]) => {
            // The mob anchors to the air cell directly above the post
            // (the post itself is a solid cube). +0.5 centres horizontally
            // so the mob doesn't try to walk into the post wall.
            Some(Vec3::new(x as f32 + 0.5, y as f32 + 1.0, z as f32 + 0.5))
        }
    }
}

/// Below this distance from the anchor the mob stops being pulled —
/// the tether is slack. Inside this radius the mob's normal AI runs
/// unhindered.
pub const TETHER_SLACK_DISTANCE: f32 = 3.0;

/// Above this distance the tether snaps and the Lead drops at the
/// mob's position. The spec doesn't pin the exact value; 8 blocks is
/// roughly two stride-clusters, so a player running away will leave
/// the mob behind but a normal-pace walk keeps the mob in tow.
pub const TETHER_SNAP_DISTANCE: f32 = 8.0;

/// Per-tick override speed when actively pulling — a tethered mob
/// walking toward its anchor moves at this many blocks/second.
/// 4.0 b/s ≈ a brisk player walk; tuned so the mob keeps up with a
/// non-sprinting player.
pub const TETHER_PULL_SPEED: f32 = 4.0;

/// Outcome of a per-mob tether check this tick. The caller applies
/// each variant: `Snap` removes the Tethered component + drops a
/// Lead item entity at the snap position; `Velocity` overrides the
/// mob's velocity vector for this tick (the mob's normal AI tick
/// stays alongside it but the tether wins).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TetherStepOutcome {
    /// Anchor within slack distance — no pull needed.
    Slack,
    /// Pull the mob toward anchor with the given velocity (b/tick).
    Pull { velocity: Vec3 },
    /// Tether snapped; caller removes the component + drops a Lead.
    Snap { drop_pos: Vec3 },
}

/// Pure helper — compute what a tether wants to do this tick from
/// `mob_pos` + `anchor`. Returns `TetherStepOutcome`. The 20-TPS tick
/// math (`speed / 20`) is applied here so the caller just assigns.
pub fn step_for(mob_pos: Vec3, anchor: Vec3) -> TetherStepOutcome {
    let delta = anchor - mob_pos;
    // Horizontal-only distance — y differences don't trigger snap so
    // a player jumping or standing on a block doesn't lose every
    // tether they're holding.
    let dx = delta.x;
    let dz = delta.z;
    let horiz_dist = (dx * dx + dz * dz).sqrt();
    if horiz_dist > TETHER_SNAP_DISTANCE {
        return TetherStepOutcome::Snap { drop_pos: mob_pos };
    }
    if horiz_dist <= TETHER_SLACK_DISTANCE {
        return TetherStepOutcome::Slack;
    }
    // Pull toward anchor at TETHER_PULL_SPEED b/s, converted to b/tick.
    let dir_xz = Vec3::new(dx, 0.0, dz).normalize_or_zero();
    let speed_per_tick = TETHER_PULL_SPEED / 20.0;
    TetherStepOutcome::Pull { velocity: dir_xz * speed_per_tick }
}

/// Per-tick driver — for every Tethered mob, resolve its anchor +
/// apply the [`TetherStepOutcome`]. Returns the list of mob entities
/// whose tethers snapped this tick + the world-space positions to
/// drop a Lead item; the caller spawns ItemEntities + removes the
/// `Tethered` component.
pub fn tick_tethers(
    ecs: &mut hecs::World,
    player_positions: &[Vec3],
) -> Vec<(hecs::Entity, Vec3)> {
    let mut snapped = Vec::new();
    for (id, (pos, vel, tether)) in ecs
        .query_mut::<(&Position, &mut Velocity, &Tethered)>()
    {
        let Some(anchor) = anchor_position(tether.target, player_positions) else {
            // Out-of-range pidx — drop the tether at the mob.
            snapped.push((id, pos.0));
            continue;
        };
        match step_for(pos.0, anchor) {
            TetherStepOutcome::Slack => { /* mob's own AI keeps moving */ }
            TetherStepOutcome::Pull { velocity } => {
                // Override horizontal velocity; leave y alone so gravity
                // still pulls the mob down per the physics tick.
                vel.0.x = velocity.x;
                vel.0.z = velocity.z;
            }
            TetherStepOutcome::Snap { drop_pos } => {
                snapped.push((id, drop_pos));
            }
        }
    }
    snapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_position_returns_player_pos_for_valid_index() {
        let players = [Vec3::new(1.0, 70.0, -2.0)];
        assert_eq!(
            anchor_position(TetherTarget::Player(0), &players),
            Some(Vec3::new(1.0, 70.0, -2.0))
        );
    }

    #[test]
    fn anchor_position_returns_none_for_out_of_range_pidx() {
        let players = [Vec3::new(0.0, 0.0, 0.0)];
        assert_eq!(anchor_position(TetherTarget::Player(5), &players), None);
    }

    #[test]
    fn anchor_position_centres_horizontally_above_post() {
        // A post at block (3, 70, 4) anchors at (3.5, 71.0, 4.5).
        // The +1.0 puts the loop ABOVE the solid post, so the mob
        // can stand next to it without trying to walk inside.
        let players = [];
        assert_eq!(
            anchor_position(TetherTarget::Post([3, 70, 4]), &players),
            Some(Vec3::new(3.5, 71.0, 4.5))
        );
    }

    #[test]
    fn step_for_slack_inside_slack_distance() {
        // Mob right next to the player — no pull, AI runs normally.
        let mob = Vec3::new(0.0, 0.0, 0.0);
        let anchor = Vec3::new(2.0, 0.0, 0.0);
        assert_eq!(step_for(mob, anchor), TetherStepOutcome::Slack);
    }

    #[test]
    fn step_for_pull_above_slack_below_snap() {
        // Mob 5 blocks away — pull toward anchor at the pull speed.
        let mob = Vec3::new(0.0, 0.0, 0.0);
        let anchor = Vec3::new(5.0, 0.0, 0.0);
        match step_for(mob, anchor) {
            TetherStepOutcome::Pull { velocity } => {
                // Direction should be +x; magnitude = PULL_SPEED / 20.
                assert!(velocity.x > 0.0);
                assert!((velocity.x - TETHER_PULL_SPEED / 20.0).abs() < 1e-3);
                assert!(velocity.z.abs() < 1e-3);
            }
            other => panic!("expected Pull, got {other:?}"),
        }
    }

    #[test]
    fn step_for_snap_above_snap_distance() {
        // Mob 9 blocks away — tether snaps.
        let mob = Vec3::new(0.0, 0.0, 0.0);
        let anchor = Vec3::new(9.0, 0.0, 0.0);
        match step_for(mob, anchor) {
            TetherStepOutcome::Snap { drop_pos } => assert_eq!(drop_pos, mob),
            other => panic!("expected Snap, got {other:?}"),
        }
    }

    #[test]
    fn step_for_ignores_vertical_distance() {
        // Player on a tall tower, mob on the ground — y delta is huge
        // but the horizontal distance is 2; that's slack, not snap.
        let mob = Vec3::new(0.0, 64.0, 0.0);
        let anchor = Vec3::new(2.0, 90.0, 0.0);
        assert_eq!(step_for(mob, anchor), TetherStepOutcome::Slack);
    }

    #[test]
    fn step_for_at_exact_snap_distance_does_not_snap() {
        // Boundary check: distance == TETHER_SNAP_DISTANCE is a Pull,
        // not a Snap (strictly greater-than triggers the break).
        let mob = Vec3::new(0.0, 0.0, 0.0);
        let anchor = Vec3::new(TETHER_SNAP_DISTANCE, 0.0, 0.0);
        match step_for(mob, anchor) {
            TetherStepOutcome::Pull { .. } => {}
            other => panic!("at exactly snap distance expected Pull, got {other:?}"),
        }
    }

    #[test]
    fn tick_tethers_snaps_when_player_too_far() {
        use crate::entity::{Position, Velocity};
        let mut ecs = hecs::World::new();
        let mob = ecs.spawn((
            Position(Vec3::new(0.0, 0.0, 0.0)),
            Velocity(Vec3::ZERO),
            Tethered { target: TetherTarget::Player(0) },
        ));
        // Player 9.5 blocks away — beyond snap distance.
        let snapped = tick_tethers(&mut ecs, &[Vec3::new(9.5, 0.0, 0.0)]);
        assert_eq!(snapped.len(), 1);
        assert_eq!(snapped[0].0, mob);
        // The drop position is the mob's position at snap time.
        assert_eq!(snapped[0].1, Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn tick_tethers_pulls_mob_velocity_toward_player() {
        use crate::entity::{Position, Velocity};
        let mut ecs = hecs::World::new();
        let _mob = ecs.spawn((
            Position(Vec3::new(0.0, 0.0, 0.0)),
            Velocity(Vec3::ZERO),
            Tethered { target: TetherTarget::Player(0) },
        ));
        // 5 blocks away on +x — should pull mob in that direction.
        let snapped = tick_tethers(&mut ecs, &[Vec3::new(5.0, 0.0, 0.0)]);
        assert!(snapped.is_empty(), "5 b away is well inside snap");
        let vel: Vec<_> = ecs
            .query::<&Velocity>()
            .iter()
            .map(|(_, v)| v.0)
            .collect();
        assert!(vel[0].x > 0.0, "expected positive-x pull, got {:?}", vel[0]);
    }

    #[test]
    fn tick_tethers_leaves_slack_mob_velocity_untouched() {
        use crate::entity::{Position, Velocity};
        let mut ecs = hecs::World::new();
        // Mob has a non-zero velocity from its AI; the tether is
        // slack so we shouldn't override it.
        let _mob = ecs.spawn((
            Position(Vec3::new(0.0, 0.0, 0.0)),
            Velocity(Vec3::new(0.1, 0.0, 0.2)),
            Tethered { target: TetherTarget::Player(0) },
        ));
        tick_tethers(&mut ecs, &[Vec3::new(1.5, 0.0, 0.0)]);
        let vel: Vec<_> = ecs
            .query::<&Velocity>()
            .iter()
            .map(|(_, v)| v.0)
            .collect();
        assert!((vel[0].x - 0.1).abs() < 1e-6);
        assert!((vel[0].z - 0.2).abs() < 1e-6);
    }

    #[test]
    fn tick_tethers_post_anchor_pulls_toward_post() {
        use crate::entity::{Position, Velocity};
        let mut ecs = hecs::World::new();
        let _mob = ecs.spawn((
            Position(Vec3::new(0.0, 0.0, 0.0)),
            Velocity(Vec3::ZERO),
            Tethered { target: TetherTarget::Post([5, 0, 0]) },
        ));
        tick_tethers(&mut ecs, &[]);
        let vel: Vec<_> = ecs
            .query::<&Velocity>()
            .iter()
            .map(|(_, v)| v.0)
            .collect();
        // Post at (5.5, 1.0, 0.5) → horizontal direction is +x dominant.
        assert!(vel[0].x > 0.0);
    }
}
