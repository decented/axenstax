//! Rubber feature — Slingshot projectile + stun-on-hit helpers.
//!
//! Single Wood-tier ranged tool. Fires Rubber Ball entities with a
//! ballistic arc (lower starting velocity than arrows). Stuns passive
//! + neutral mobs on hit by setting `AiState::Wander` with a short
//!   timer; carnivores (Bear / Wolf / Hyena) and brigand-family humans
//!   (Brigand / Marauder / Berserker / Knight) shrug off rubber-ball hits.
//!
//! Spec: `docs/foundations/2026-05-23-rubber.md`.

use crate::mob::MobType;

/// Maximum charge ticks the slingshot accepts (~1 second hold).
/// Beyond this, released projectile uses MAX_CHARGE.
pub const SLINGSHOT_MAX_CHARGE_TICKS: u32 = 20;

/// Base projectile damage at minimum charge.
pub const SLINGSHOT_MIN_DAMAGE: f32 = 1.0;

/// Maximum projectile damage at full charge.
pub const SLINGSHOT_MAX_DAMAGE: f32 = 3.0;

/// Stun duration in ticks applied to passive/neutral mob targets
/// when the projectile hits at at-least half charge.
pub const STUN_TICKS: u32 = 20;

/// Pure: damage scaled by charge. Linearly maps charge_ticks 0..MAX
/// to MIN..MAX damage. Caller clamps charge before passing or relies
/// on the min() inside.
pub fn slingshot_damage(charge_ticks: u32) -> f32 {
    let charge = charge_ticks.min(SLINGSHOT_MAX_CHARGE_TICKS) as f32;
    let max = SLINGSHOT_MAX_CHARGE_TICKS as f32;
    let t = charge / max;
    SLINGSHOT_MIN_DAMAGE + (SLINGSHOT_MAX_DAMAGE - SLINGSHOT_MIN_DAMAGE) * t
}

/// Pure: stun-tick duration for the target species + charge. Returns
/// 0 if the charge was below half, or the target isn't stunnable.
/// Otherwise STUN_TICKS. No production caller — `entity.rs`'s blunt-hit
/// resolver applies the same stun rule inline (checks `is_stunnable` +
/// half-damage threshold, sets `STUN_TICKS` directly) rather than calling
/// this composed helper, so the two paths could drift. Exercised by tests
/// here and in `test_integration/rubber.rs`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn slingshot_stun_ticks(target: MobType, charge_ticks: u32) -> u32 {
    if charge_ticks < SLINGSHOT_MAX_CHARGE_TICKS / 2 {
        return 0;
    }
    if !is_stunnable(target) {
        return 0;
    }
    STUN_TICKS
}

/// True for mob species the slingshot can stun. Excludes carnivores
/// (Bear / Hyena / Wolf) and brigand-family humans (Brigand / Marauder /
/// Berserker / Knight) — those shrug off rubber-ball hits.
pub fn is_stunnable(kind: MobType) -> bool {
    !matches!(kind,
        MobType::Bear | MobType::Hyena | MobType::Wolf
            | MobType::Brigand | MobType::Marauder | MobType::Berserker
            | MobType::Knight)
}

/// Initial velocity of a slingshot projectile, scaled by charge.
/// Returns blocks/tick magnitude; direction is the caller's
/// responsibility (player look direction).
pub fn slingshot_velocity(charge_ticks: u32) -> f32 {
    let charge = charge_ticks.min(SLINGSHOT_MAX_CHARGE_TICKS) as f32;
    let max = SLINGSHOT_MAX_CHARGE_TICKS as f32;
    let t = charge / max;
    // 0.4 -> 0.9 blocks/tick. Arrow starts ~1.5 b/tick so slingshot
    // feels more arc-y; rubber balls are lobbed, not snapped.
    0.4 + 0.5 * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_scales_with_charge() {
        let d_min = slingshot_damage(0);
        let d_max = slingshot_damage(SLINGSHOT_MAX_CHARGE_TICKS);
        let d_over = slingshot_damage(SLINGSHOT_MAX_CHARGE_TICKS * 10);
        assert!((d_min - SLINGSHOT_MIN_DAMAGE).abs() < 1e-3);
        assert!((d_max - SLINGSHOT_MAX_DAMAGE).abs() < 1e-3);
        assert!((d_over - SLINGSHOT_MAX_DAMAGE).abs() < 1e-3, "overcharge clamps");
    }

    #[test]
    fn stun_requires_at_least_half_charge() {
        assert_eq!(slingshot_stun_ticks(MobType::Cow, 0), 0);
        assert_eq!(slingshot_stun_ticks(MobType::Cow, SLINGSHOT_MAX_CHARGE_TICKS / 2 - 1), 0);
        assert_eq!(slingshot_stun_ticks(MobType::Cow, SLINGSHOT_MAX_CHARGE_TICKS / 2), STUN_TICKS);
        assert_eq!(slingshot_stun_ticks(MobType::Cow, SLINGSHOT_MAX_CHARGE_TICKS), STUN_TICKS);
    }

    #[test]
    fn carnivores_and_brigands_are_not_stunnable() {
        let full = SLINGSHOT_MAX_CHARGE_TICKS;
        for mob in [
            MobType::Bear, MobType::Wolf, MobType::Hyena, MobType::Brigand,
            MobType::Marauder, MobType::Berserker, MobType::Knight,
        ] {
            assert_eq!(slingshot_stun_ticks(mob, full), 0,
                "{mob:?} should not be stunnable");
        }
    }

    #[test]
    fn passives_are_stunnable() {
        let full = SLINGSHOT_MAX_CHARGE_TICKS;
        for mob in [
            MobType::Cow, MobType::Sheep, MobType::Pig, MobType::Horse,
            MobType::Goat, MobType::Chicken, MobType::Rabbit, MobType::Nostrich,
        ] {
            assert_eq!(slingshot_stun_ticks(mob, full), STUN_TICKS,
                "{mob:?} should be stunnable");
        }
    }

    #[test]
    fn velocity_scales_with_charge() {
        let v_min = slingshot_velocity(0);
        let v_max = slingshot_velocity(SLINGSHOT_MAX_CHARGE_TICKS);
        assert!(v_min < v_max);
        assert!(v_min > 0.3 && v_min < 0.5);
        assert!(v_max > 0.8 && v_max < 1.0);
    }
}
