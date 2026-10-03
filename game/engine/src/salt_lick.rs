//! Salt feature — Salt Lick aura, drop bonus, and wander steering.
//!
//! Pure helpers + a tick driver. Affected mobs (Cow / Sheep / Horse /
//! Pig / Goat) within 8 blocks Chebyshev of a SALT_LICK get:
//!   1. 2× HP regen rate (read by the combat regen helper).
//!   2. Wander direction biased toward the nearest lick.
//!   3. On kill, +1 of their species' primary drop.
//!
//! `World.salt_licks: AHashSet<(i32, i32, i32)>` is the side-table
//! index — populated at place-time, drained at break-time, rebuilt on
//! load. `#[serde(skip)]` because the block placement IS the canonical
//! state; this is a derived cache.
//!
//! Spec: `docs/foundations/2026-05-23-salt.md`.

use crate::item::MaterialId;
use crate::mob::MobType;
use crate::world::World;
use glam::Vec3;

/// Aura radius — Chebyshev distance in blocks for the regen + drop-
/// bonus effect.
pub const SALT_LICK_AURA_BLOCKS: i32 = 8;

/// Detection radius for the wander-steering bias (looser than the aura
/// so livestock drift in from further out).
pub const SALT_LICK_DETECT_BLOCKS: i32 = 16;

/// Returns the HP regen multiplier for a mob at `pos`. Affected
/// species get 2× when within `SALT_LICK_AURA_BLOCKS` (Chebyshev) of
/// any SALT_LICK; everything else returns 1.0.
pub fn regen_multiplier_at_pos(world: &World, pos: Vec3, kind: MobType) -> f32 {
    if !is_livestock_for_salt_lick(kind) {
        return 1.0;
    }
    let ix = pos.x.floor() as i32;
    let iy = pos.y.floor() as i32;
    let iz = pos.z.floor() as i32;
    for &(lx, ly, lz) in world.salt_licks.iter() {
        let dx = (lx - ix).abs();
        let dy = (ly - iy).abs();
        let dz = (lz - iz).abs();
        if dx.max(dy).max(dz) <= SALT_LICK_AURA_BLOCKS {
            return 2.0;
        }
    }
    1.0
}

/// Returns the unit direction the mob should bias its wander toward,
/// or `None` if no lick is within `SALT_LICK_DETECT_BLOCKS` or if the
/// species isn't affected. Linear scan over the index — number of
/// licks is small (player-placed, expect tens).
pub fn wander_bias_for_salt_lick(
    world: &World,
    pos: Vec3,
    kind: MobType,
) -> Option<Vec3> {
    if !is_livestock_for_salt_lick(kind) {
        return None;
    }
    let mut best: Option<(Vec3, f32)> = None;
    for &(lx, ly, lz) in world.salt_licks.iter() {
        let lp = Vec3::new(lx as f32 + 0.5, ly as f32, lz as f32 + 0.5);
        // 3D Chebyshev — matches `regen_multiplier_at_pos` +
        // `within_any_salt_lick_aura`. A 2D-only gate would steer
        // mobs toward licks on cliff tops they can never reach.
        let dx = (lx as f32 - pos.x).abs();
        let dy = (ly as f32 - pos.y).abs();
        let dz = (lz as f32 - pos.z).abs();
        if dx.max(dy).max(dz) > SALT_LICK_DETECT_BLOCKS as f32 {
            continue;
        }
        let to_lick = lp - pos;
        let d = to_lick.length();
        if d < 0.5 {
            continue; // standing on it; no bias needed
        }
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((to_lick.normalize(), d));
        }
    }
    best.map(|(dir, _)| dir)
}

/// True when `kind` is a livestock species that benefits from a Salt
/// Lick aura. Data table — explicit per-species so the surface is
/// audit-able. v1 list: Cow / Sheep / Horse / Pig / Goat. Carnivores
/// (Wolf / Bear / Hyena) + exotics (Bee / Squid / Nostrich / Rabbit /
/// Chicken) explicitly excluded.
pub fn is_livestock_for_salt_lick(kind: MobType) -> bool {
    matches!(kind,
        MobType::Cow | MobType::Sheep | MobType::Horse
            | MobType::Pig | MobType::Goat)
}

/// Per-species "primary product" used for the on-kill drop bonus.
/// Returns the MaterialId to drop one extra of when the mob died
/// inside the SALT_LICK aura.
pub fn salt_lick_primary_drop(kind: MobType) -> Option<MaterialId> {
    match kind {
        MobType::Cow => Some(MaterialId::Leather),
        MobType::Sheep => Some(MaterialId::Wool),
        MobType::Pig => Some(MaterialId::RawPorkchop),
        MobType::Horse => Some(MaterialId::Leather),
        // Goat v1 only drops Wool from `mob::drops_for` (no
        // goat-specific raw meat); RawMutton would be a phantom
        // material the player never sees outside the aura.
        MobType::Goat => Some(MaterialId::Wool),
        _ => None,
    }
}

/// Ticks between +1 HP regen pulses for livestock standing inside a
/// SALT_LICK aura. 100 ticks ≈ 5 s @ 20 TPS — slower than player
/// passive regen so it's a husbandry helper, not a god-bar.
pub const SALT_LICK_REGEN_INTERVAL_TICKS: u64 = 100;

/// Drive HP regen for livestock standing inside any SALT_LICK aura.
/// Pulses every `SALT_LICK_REGEN_INTERVAL_TICKS`; on a pulse tick,
/// every affected mob below max HP gains 1 HP (capped at max). Cheap
/// no-op on non-pulse ticks; the in-aura check is short-circuited by
/// the empty-lick fast path.
pub fn tick_salt_lick_regen(
    ecs: &mut hecs::World,
    world: &World,
    monotonic_tick: u64,
) {
    if world.salt_licks.is_empty() {
        return;
    }
    if !monotonic_tick.is_multiple_of(SALT_LICK_REGEN_INTERVAL_TICKS) {
        return;
    }
    let mut q = ecs.query::<(
        &crate::entity::Position,
        &crate::entity::MobKind,
        &mut crate::combat::Health,
    )>();
    for (_id, (pos, kind, health)) in q.iter() {
        if health.current >= health.max {
            continue;
        }
        if !is_livestock_for_salt_lick(kind.0) {
            continue;
        }
        if !within_any_salt_lick_aura(world, pos.0) {
            continue;
        }
        let multiplier = regen_multiplier_at_pos(world, pos.0, kind.0);
        health.current = (health.current + 1.0 * multiplier).min(health.max);
    }
}

/// True if `mob_pos` is within the SALT_LICK aura of any registered
/// lick. Used by the on-kill drop-bonus arm in `combat.rs` (and by
/// integration tests).
pub fn within_any_salt_lick_aura(world: &World, mob_pos: Vec3) -> bool {
    let ix = mob_pos.x.floor() as i32;
    let iy = mob_pos.y.floor() as i32;
    let iz = mob_pos.z.floor() as i32;
    for &(lx, ly, lz) in world.salt_licks.iter() {
        let dx = (lx - ix).abs();
        let dy = (ly - iy).abs();
        let dz = (lz - iz).abs();
        if dx.max(dy).max(dz) <= SALT_LICK_AURA_BLOCKS {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affected_species_table_is_exact() {
        // Yes set — domestic livestock.
        for kind in [
            MobType::Cow, MobType::Sheep, MobType::Horse,
            MobType::Pig, MobType::Goat,
        ] {
            assert!(is_livestock_for_salt_lick(kind), "{kind:?} should be livestock");
        }
        // Explicit no set — carnivores + exotics.
        for kind in [
            MobType::Wolf, MobType::Bear, MobType::Hyena,
            MobType::Bee, MobType::Squid, MobType::Nostrich,
            MobType::Rabbit, MobType::Chicken,
        ] {
            assert!(!is_livestock_for_salt_lick(kind), "{kind:?} should NOT be livestock");
        }
    }

    #[test]
    fn regen_multiplier_inside_aura_is_2x() {
        let mut w = World::new();
        w.salt_licks.insert((0, 64, 0));
        // 3 blocks away in each axis — Chebyshev 3, well within 8.
        let p = Vec3::new(3.0, 64.0, 3.0);
        assert_eq!(regen_multiplier_at_pos(&w, p, MobType::Cow), 2.0);
    }

    #[test]
    fn regen_multiplier_outside_aura_is_1x() {
        let mut w = World::new();
        w.salt_licks.insert((0, 64, 0));
        // 9 blocks away in x — Chebyshev 9, outside 8.
        let p = Vec3::new(9.0, 64.0, 0.0);
        assert_eq!(regen_multiplier_at_pos(&w, p, MobType::Cow), 1.0);
    }

    #[test]
    fn regen_multiplier_carnivore_is_1x_inside_aura() {
        let mut w = World::new();
        w.salt_licks.insert((0, 64, 0));
        let p = Vec3::new(2.0, 64.0, 2.0);
        assert_eq!(regen_multiplier_at_pos(&w, p, MobType::Wolf), 1.0);
        assert_eq!(regen_multiplier_at_pos(&w, p, MobType::Bear), 1.0);
    }

    #[test]
    fn wander_bias_points_toward_lick() {
        let mut w = World::new();
        w.salt_licks.insert((10, 64, 0));
        let p = Vec3::new(0.0, 64.0, 0.0);
        let dir = wander_bias_for_salt_lick(&w, p, MobType::Cow)
            .expect("should produce a bias direction");
        assert!(dir.x > 0.9, "should point mostly +x; got {dir:?}");
        assert!(dir.z.abs() < 0.1, "should be ~no z component; got {dir:?}");
    }

    #[test]
    fn wander_bias_is_none_outside_detect_radius() {
        let mut w = World::new();
        // 20 blocks away — outside the 16-block detect.
        w.salt_licks.insert((20, 64, 0));
        let p = Vec3::new(0.0, 64.0, 0.0);
        assert!(wander_bias_for_salt_lick(&w, p, MobType::Cow).is_none());
    }

    #[test]
    fn wander_bias_uses_3d_detect_not_2d() {
        // Salt lick 20 blocks above the mob — 2D Chebyshev gives 0
        // (would falsely bias toward the unreachable lick); 3D gives
        // 20 (correctly outside the 16-block detect).
        let mut w = World::new();
        w.salt_licks.insert((0, 84, 0));
        let p = Vec3::new(0.0, 64.0, 0.0);
        assert!(wander_bias_for_salt_lick(&w, p, MobType::Cow).is_none(),
            "wander bias must respect Y distance");
    }

    #[test]
    fn primary_drop_table_matches_spec() {
        assert_eq!(salt_lick_primary_drop(MobType::Cow), Some(MaterialId::Leather));
        assert_eq!(salt_lick_primary_drop(MobType::Sheep), Some(MaterialId::Wool));
        assert_eq!(salt_lick_primary_drop(MobType::Pig), Some(MaterialId::RawPorkchop));
        assert_eq!(salt_lick_primary_drop(MobType::Horse), Some(MaterialId::Leather));
        assert_eq!(salt_lick_primary_drop(MobType::Goat), Some(MaterialId::Wool));
        // Non-livestock returns None.
        assert_eq!(salt_lick_primary_drop(MobType::Wolf), None);
        assert_eq!(salt_lick_primary_drop(MobType::Nostrich), None);
    }

    #[test]
    fn tick_regen_heals_livestock_inside_aura_at_pulse() {
        // Cow at half HP next to a SALT_LICK; after one pulse interval
        // it gains 2 HP (1.0 * 2.0 multiplier). After many pulses it caps at max.
        let mut world = World::new();
        world.salt_licks.insert((0, 64, 0));
        let mut ecs = hecs::World::new();
        let cow_id = ecs.spawn((
            crate::entity::Position(Vec3::new(2.0, 64.0, 2.0)),
            crate::entity::MobKind(MobType::Cow),
            crate::combat::Health {
                current: 5.0, max: 10.0,
                flash_timer: 0, invincible_timer: 0,
            },
        ));

        // Non-pulse tick — no change.
        tick_salt_lick_regen(&mut ecs, &world, SALT_LICK_REGEN_INTERVAL_TICKS - 1);
        let h = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        assert_eq!(h, 5.0, "non-pulse tick must not heal");

        // Pulse tick — +2.0 (1.0 * 2.0x multiplier inside aura).
        tick_salt_lick_regen(&mut ecs, &world, SALT_LICK_REGEN_INTERVAL_TICKS);
        let h = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        assert_eq!(h, 7.0, "pulse tick must heal +2.0 with 2x aura multiplier");

        // Many pulses later — capped at max.
        for n in 2..50u64 {
            tick_salt_lick_regen(&mut ecs, &world, n * SALT_LICK_REGEN_INTERVAL_TICKS);
        }
        let h = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        assert_eq!(h, 10.0, "should cap at max HP");
    }

    #[test]
    fn tick_regen_skips_carnivores_in_aura() {
        let mut world = World::new();
        world.salt_licks.insert((0, 64, 0));
        let mut ecs = hecs::World::new();
        let wolf_id = ecs.spawn((
            crate::entity::Position(Vec3::new(2.0, 64.0, 2.0)),
            crate::entity::MobKind(MobType::Wolf),
            crate::combat::Health {
                current: 5.0, max: 10.0,
                flash_timer: 0, invincible_timer: 0,
            },
        ));
        tick_salt_lick_regen(&mut ecs, &world, SALT_LICK_REGEN_INTERVAL_TICKS);
        let h = ecs.get::<&crate::combat::Health>(wolf_id).unwrap().current;
        assert_eq!(h, 5.0, "carnivores must not regen at SALT_LICK");
    }

    #[test]
    fn tick_regen_skips_livestock_outside_aura() {
        let mut world = World::new();
        world.salt_licks.insert((0, 64, 0));
        let mut ecs = hecs::World::new();
        let cow_id = ecs.spawn((
            crate::entity::Position(Vec3::new(20.0, 64.0, 20.0)),  // outside 8-block aura
            crate::entity::MobKind(MobType::Cow),
            crate::combat::Health {
                current: 5.0, max: 10.0,
                flash_timer: 0, invincible_timer: 0,
            },
        ));
        tick_salt_lick_regen(&mut ecs, &world, SALT_LICK_REGEN_INTERVAL_TICKS);
        let h = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        assert_eq!(h, 5.0, "outside aura must not regen");
    }

    #[test]
    fn tick_regen_is_noop_when_no_licks() {
        // Empty salt_licks → early return. Verifies the fast-path.
        let world = World::new();
        let mut ecs = hecs::World::new();
        let cow_id = ecs.spawn((
            crate::entity::Position(Vec3::new(2.0, 64.0, 2.0)),
            crate::entity::MobKind(MobType::Cow),
            crate::combat::Health {
                current: 5.0, max: 10.0,
                flash_timer: 0, invincible_timer: 0,
            },
        ));
        tick_salt_lick_regen(&mut ecs, &world, SALT_LICK_REGEN_INTERVAL_TICKS);
        let h = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        assert_eq!(h, 5.0);
    }

    #[test]
    fn tick_regen_applies_2x_multiplier_inside_aura() {
        // Cow inside aura should gain 2.0 HP per pulse (1.0 * 2.0 multiplier),
        // not 1.0 HP as the old buggy code did.
        let mut world = World::new();
        world.salt_licks.insert((0, 64, 0));
        let mut ecs = hecs::World::new();

        let cow_id = ecs.spawn((
            crate::entity::Position(Vec3::new(2.0, 64.0, 2.0)),
            crate::entity::MobKind(MobType::Cow),
            crate::combat::Health {
                current: 5.0, max: 10.0,
                flash_timer: 0, invincible_timer: 0,
            },
        ));

        tick_salt_lick_regen(&mut ecs, &world, SALT_LICK_REGEN_INTERVAL_TICKS);

        let hp = ecs.get::<&crate::combat::Health>(cow_id).unwrap().current;
        assert_eq!(hp, 7.0, "inside aura, regen should be 2.0 HP (1.0 * 2.0 multiplier)");
    }

    #[test]
    fn within_any_aura_chebyshev_gate() {
        let mut w = World::new();
        w.salt_licks.insert((0, 64, 0));
        // Inside aura at (8, 64, 8) — Chebyshev 8, boundary case = inside.
        assert!(within_any_salt_lick_aura(&w, Vec3::new(8.0, 64.0, 8.0)));
        // Outside at (9, 64, 0) — Chebyshev 9.
        assert!(!within_any_salt_lick_aura(&w, Vec3::new(9.0, 64.0, 0.0)));
    }
}
