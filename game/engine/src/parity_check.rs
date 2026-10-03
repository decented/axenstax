//! Phase 0 of `docs/foundations/2026-04-20-singleplayer-hostedserver-routing.md`:
//! tripwire that fires when the client-side and server-side simulations of
//! single-player drift apart.
//!
//! Today single-player runs the same simulation functions on both sides
//! (mob spawning, falling blocks, entity AI) — the client because it always
//! has, the server (when started under flag-on) because it ticks the same
//! authoritative loop. They share inputs, so they should produce the same
//! outputs. If they don't, the cutover plan in phases 1–4 (which deletes the
//! client-side caller in favour of the server) is unsafe.
//!
//! `WorldParityHash` is intentionally cheap to compute — counts plus a
//! mixed-in float-position digest so trivial cases (mob count drift,
//! falling-block displacement drift) trip immediately while a full
//! per-block hash is left to a later phase if it's ever needed.
//!
//! Comparing two `WorldParityHash` values that disagree returns a
//! `Divergence` describing the first delta found, in priority order. The
//! caller decides whether to log, warn, or panic — the spec asks for panic
//! in debug builds, but Phase 0 ships with logging only so the autonomy
//! boundary is preserved (a noisy panic on launch is worse than a noisy log).

use std::hash::Hasher;

use crate::entity::{MobKind, Position};

/// Cheap one-tick world signature.
///
/// All fields are aggregates, not hashes of individual entities — comparing
/// two signatures from the same tick should agree byte-for-byte if both
/// simulations produced the same outputs. The purpose is to catch any drift
/// early in the cutover, not to provide full state attestation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldParityHash {
    /// Total entities tracked by the ECS this tick — includes mobs and any
    /// other ECS-resident entity (items, projectiles, …).
    pub entity_count: u32,
    /// Subset of `entity_count` that are mobs (kind matters: a cow drift
    /// distinct from a zombie drift will look the same in entity_count).
    pub mob_count: u32,
    /// 64-bit hash of mob positions discretised to integer block units. Cheap
    /// enough to recompute every tick; sensitive enough to trip on most kinds
    /// of physics drift. See `mix_position` for the discretisation.
    pub mob_position_digest: u64,
    /// World tick the signature was taken at; mismatched ticks should never
    /// be compared, so the comparator yields `TickMismatch` immediately.
    pub world_time: u32,
}

impl WorldParityHash {
    /// Build a signature from raw counts and an iterator over mob positions
    /// `(x, y, z)`. Caller is responsible for collecting positions in a
    /// stable order — the same order on both sides — so the digest is
    /// reproducible. The natural order is "ECS query iteration over the
    /// MobKind component", which both sides perform once per tick.
    pub fn from_state<I: Iterator<Item = (f32, f32, f32)>>(
        entity_count: u32,
        mob_count: u32,
        positions: I,
        world_time: u32,
    ) -> Self {
        let mut hasher = ahash::AHasher::default();
        for pos in positions {
            mix_position(&mut hasher, pos);
        }
        Self {
            entity_count,
            mob_count,
            mob_position_digest: hasher.finish(),
            world_time,
        }
    }

    /// Sample a `WorldParityHash` from a live ECS at a known `world_time`.
    /// Encapsulates the "stable iteration order" requirement so the call
    /// site doesn't have to think about it: positions are pulled by sorting
    /// mobs on `(MobKind discriminant, x, y, z)`, which is reproducible
    /// across the two ECS instances of phases 1–4 (client vs. server). The
    /// caller is still responsible for synchronising sample points — call
    /// after both sides have ticked.
    pub fn sample_from_ecs(ecs: &hecs::World, world_time: u32) -> Self {
        let entity_count = ecs.iter().count() as u32;

        let mut mobs: Vec<(u32, f32, f32, f32)> = ecs
            .query::<(&MobKind, &Position)>()
            .iter()
            .map(|(_, (kind, pos))| {
                // Discriminant only — MobType is repr-default, so as-cast to
                // u32 yields a stable per-variant id within a given build.
                let discriminant = kind.0 as u32;
                (discriminant, pos.0.x, pos.0.y, pos.0.z)
            })
            .collect();
        mobs.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.partial_cmp(&b.1).unwrap_or(core::cmp::Ordering::Equal))
                .then(a.2.partial_cmp(&b.2).unwrap_or(core::cmp::Ordering::Equal))
                .then(a.3.partial_cmp(&b.3).unwrap_or(core::cmp::Ordering::Equal))
        });

        let mob_count = mobs.len() as u32;
        let positions = mobs.iter().map(|(_, x, y, z)| (*x, *y, *z));
        Self::from_state(entity_count, mob_count, positions, world_time)
    }

    /// Compare two signatures from the same tick and return the first
    /// divergence found, in fixed priority order. `None` is the happy path.
    pub fn diff(&self, other: &Self) -> Option<Divergence> {
        if self.world_time != other.world_time {
            return Some(Divergence::TickMismatch {
                client: self.world_time,
                server: other.world_time,
            });
        }
        if self.entity_count != other.entity_count {
            return Some(Divergence::EntityCount {
                client: self.entity_count,
                server: other.entity_count,
            });
        }
        if self.mob_count != other.mob_count {
            return Some(Divergence::MobCount {
                client: self.mob_count,
                server: other.mob_count,
            });
        }
        if self.mob_position_digest != other.mob_position_digest {
            return Some(Divergence::MobPositions {
                client: self.mob_position_digest,
                server: other.mob_position_digest,
            });
        }
        None
    }
}

/// First divergence found between two `WorldParityHash` signatures, in fixed
/// priority order. `EntityCount` is checked before `MobCount` because the
/// former subsumes the latter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Divergence {
    /// The two signatures came from different ticks. Always investigate —
    /// it means the caller failed to sync the sample point.
    TickMismatch { client: u32, server: u32 },
    EntityCount { client: u32, server: u32 },
    MobCount { client: u32, server: u32 },
    /// Counts agree but positions don't — physics drift.
    MobPositions { client: u64, server: u64 },
}

impl core::fmt::Display for Divergence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Divergence::TickMismatch { client, server } => write!(
                f,
                "world_time mismatch (client={client}, server={server}) — sample points are not synchronised"
            ),
            Divergence::EntityCount { client, server } => {
                write!(f, "entity count differs (client={client}, server={server})")
            }
            Divergence::MobCount { client, server } => {
                write!(f, "mob count differs (client={client}, server={server})")
            }
            Divergence::MobPositions { client, server } => write!(
                f,
                "mob position digest differs (client=0x{client:016x}, server=0x{server:016x})"
            ),
        }
    }
}

/// Discretise a (sub-block-precision) position into integer block coordinates
/// before mixing it into the digest. Catches physics drift at block
/// granularity — finer drift (sub-block) is intentionally tolerated because
/// f32 ordering can vary across simulation orderings even when behaviour is
/// equivalent. Phase 1 may switch to fixed-point if that turns out to be too
/// loose.
fn mix_position(hasher: &mut ahash::AHasher, (x, y, z): (f32, f32, f32)) {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let zi = z.floor() as i32;
    hasher.write_i32(xi);
    hasher.write_i32(yi);
    hasher.write_i32(zi);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        (x, y, z)
    }

    #[test]
    fn identical_state_produces_identical_hashes() {
        let positions = [pos(1.0, 2.0, 3.0), pos(4.0, 5.0, 6.0)];
        let a = WorldParityHash::from_state(2, 2, positions.iter().copied(), 100);
        let b = WorldParityHash::from_state(2, 2, positions.iter().copied(), 100);
        assert_eq!(a, b);
        assert_eq!(a.diff(&b), None, "matching state must report no divergence");
    }

    #[test]
    fn entity_count_drift_reported_first() {
        let positions = [pos(1.0, 2.0, 3.0)];
        let a = WorldParityHash::from_state(2, 1, positions.iter().copied(), 100);
        let b = WorldParityHash::from_state(3, 1, positions.iter().copied(), 100);
        match a.diff(&b) {
            Some(Divergence::EntityCount { client: 2, server: 3 }) => {}
            other => panic!("expected EntityCount divergence, got {other:?}"),
        }
    }

    #[test]
    fn mob_count_drift_reported_when_entities_match() {
        let positions = [pos(1.0, 2.0, 3.0)];
        let a = WorldParityHash::from_state(2, 1, positions.iter().copied(), 100);
        let b = WorldParityHash::from_state(2, 2, positions.iter().copied(), 100);
        match a.diff(&b) {
            Some(Divergence::MobCount { client: 1, server: 2 }) => {}
            other => panic!("expected MobCount divergence, got {other:?}"),
        }
    }

    #[test]
    fn mob_position_drift_caught_at_block_granularity() {
        let a = WorldParityHash::from_state(1, 1, [pos(1.0, 2.0, 3.0)].iter().copied(), 100);
        let b = WorldParityHash::from_state(1, 1, [pos(2.0, 2.0, 3.0)].iter().copied(), 100);
        match a.diff(&b) {
            Some(Divergence::MobPositions { .. }) => {}
            other => panic!("expected MobPositions divergence, got {other:?}"),
        }
    }

    #[test]
    fn sub_block_movement_within_same_block_is_tolerated() {
        // Both positions floor to (1, 2, 3) — drift below block granularity
        // is intentionally invisible (see `mix_position` rationale).
        let a = WorldParityHash::from_state(1, 1, [pos(1.0, 2.0, 3.0)].iter().copied(), 100);
        let b = WorldParityHash::from_state(1, 1, [pos(1.4, 2.7, 3.9)].iter().copied(), 100);
        assert_eq!(a.diff(&b), None);
    }

    #[test]
    fn tick_mismatch_takes_priority_over_state_drift() {
        // Even with totally divergent counts, world_time mismatch is the
        // first thing reported because it means the caller mis-sampled.
        let a = WorldParityHash::from_state(1, 1, [pos(1.0, 2.0, 3.0)].iter().copied(), 100);
        let b = WorldParityHash::from_state(99, 99, [pos(99.0, 99.0, 99.0)].iter().copied(), 101);
        match a.diff(&b) {
            Some(Divergence::TickMismatch { client: 100, server: 101 }) => {}
            other => panic!("expected TickMismatch, got {other:?}"),
        }
    }

    #[test]
    fn empty_world_hashes_match() {
        let a = WorldParityHash::from_state(0, 0, std::iter::empty(), 100);
        let b = WorldParityHash::from_state(0, 0, std::iter::empty(), 100);
        assert_eq!(a, b);
    }

    #[test]
    fn position_order_matters_so_callers_must_use_a_stable_iterator() {
        // Documenting the requirement via test: out-of-order positions
        // produce different digests. The caller (game_loop.rs) is responsible
        // for iterating mob positions the same way on both sides.
        let a = WorldParityHash::from_state(
            2,
            2,
            [pos(1.0, 2.0, 3.0), pos(4.0, 5.0, 6.0)].iter().copied(),
            100,
        );
        let b = WorldParityHash::from_state(
            2,
            2,
            [pos(4.0, 5.0, 6.0), pos(1.0, 2.0, 3.0)].iter().copied(),
            100,
        );
        assert_ne!(
            a, b,
            "position order must matter — caller must use a stable iterator"
        );
    }

    #[test]
    fn divergence_display_is_human_readable() {
        let d = Divergence::MobCount { client: 3, server: 5 };
        assert_eq!(format!("{d}"), "mob count differs (client=3, server=5)");
    }

    // ── sample_from_ecs ──────────────────────────────────────────────────────
    //
    // Wiring acceptance: the helper that the cutover plan will call from
    // game_loop.rs each tick. Drive against a handcrafted ECS so the test
    // doesn't need a full GameServer.

    use crate::entity;
    use crate::mob::MobType;
    use glam::Vec3;

    #[test]
    fn sample_from_ecs_matches_when_two_ecs_have_identical_mobs() {
        let mut a = hecs::World::new();
        let mut b = hecs::World::new();
        entity::spawn_mob(&mut a, MobType::Cow, Vec3::new(1.0, 2.0, 3.0));
        entity::spawn_mob(&mut a, MobType::Brigand, Vec3::new(10.0, 20.0, 30.0));
        entity::spawn_mob(&mut b, MobType::Brigand, Vec3::new(10.0, 20.0, 30.0));
        entity::spawn_mob(&mut b, MobType::Cow, Vec3::new(1.0, 2.0, 3.0));

        // Same mobs, different spawn order: the sample's stable sort means
        // their hashes still agree.
        let h_a = WorldParityHash::sample_from_ecs(&a, 100);
        let h_b = WorldParityHash::sample_from_ecs(&b, 100);
        assert_eq!(h_a, h_b);
        assert_eq!(h_a.diff(&h_b), None);
        assert_eq!(h_a.mob_count, 2);
    }

    #[test]
    fn sample_from_ecs_diverges_when_one_side_is_missing_a_mob() {
        let mut client = hecs::World::new();
        let mut server = hecs::World::new();
        entity::spawn_mob(&mut client, MobType::Cow, Vec3::new(0.0, 0.0, 0.0));
        entity::spawn_mob(&mut client, MobType::Brigand, Vec3::new(5.0, 5.0, 5.0));
        entity::spawn_mob(&mut server, MobType::Cow, Vec3::new(0.0, 0.0, 0.0));

        let h_client = WorldParityHash::sample_from_ecs(&client, 100);
        let h_server = WorldParityHash::sample_from_ecs(&server, 100);
        match h_client.diff(&h_server) {
            Some(Divergence::EntityCount { .. }) | Some(Divergence::MobCount { .. }) => {}
            other => panic!("expected entity- or mob-count divergence, got {other:?}"),
        }
    }

    #[test]
    fn sample_from_ecs_diverges_when_one_mob_drifts_a_block() {
        let mut client = hecs::World::new();
        let mut server = hecs::World::new();
        entity::spawn_mob(&mut client, MobType::Cow, Vec3::new(0.0, 0.0, 0.0));
        entity::spawn_mob(&mut server, MobType::Cow, Vec3::new(1.0, 0.0, 0.0));

        let h_client = WorldParityHash::sample_from_ecs(&client, 100);
        let h_server = WorldParityHash::sample_from_ecs(&server, 100);
        match h_client.diff(&h_server) {
            Some(Divergence::MobPositions { .. }) => {}
            other => panic!("expected MobPositions divergence, got {other:?}"),
        }
    }
}
