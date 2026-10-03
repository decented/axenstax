//! Rubber feature integration tests.
//!
//! End-to-end coverage for the tap cycle, save/reload semantics,
//! slingshot stun helpers, sprint multiplier, and Eraser Plan wipe.

use crate::block;
use crate::mob::MobType;
use crate::world::World;

#[test]
fn full_tap_cycle_world_level() {
    let mut world = World::new();
    world.set_block(0, 64, 0, block::RUBBER_LOG);
    // Tap.
    let ok = crate::rubber::apply_tap(&mut world, (0, 64, 0), 0);
    assert!(ok);
    assert_eq!(world.get_block(0, 64, 0), block::RUBBER_LOG_TAPPED);
    // Mid-cooldown — no restoration.
    let restored = crate::rubber::tick_rubber_cooldowns(&mut world, 1_000);
    assert_eq!(restored, 0);
    assert_eq!(world.get_block(0, 64, 0), block::RUBBER_LOG_TAPPED);
    // Cooldown expired — restored.
    let restored = crate::rubber::tick_rubber_cooldowns(
        &mut world, crate::rubber::TAP_COOLDOWN_TICKS,
    );
    assert_eq!(restored, 1);
    assert_eq!(world.get_block(0, 64, 0), block::RUBBER_LOG);
    assert!(!world.tapped_rubber_logs.contains_key(&(0, 64, 0)));
    // Re-tap works after restoration.
    let ok = crate::rubber::apply_tap(
        &mut world, (0, 64, 0), crate::rubber::TAP_COOLDOWN_TICKS + 1,
    );
    assert!(ok);
}

#[test]
fn felled_tapped_log_drops_index_entry_without_restoration() {
    let mut world = World::new();
    world.set_block(0, 64, 0, block::RUBBER_LOG);
    let _ = crate::rubber::apply_tap(&mut world, (0, 64, 0), 0);
    // Player fells the tapped tree mid-cooldown.
    world.set_block(0, 64, 0, block::AIR);
    // Cooldown driver: no restoration; index dropped.
    let restored = crate::rubber::tick_rubber_cooldowns(
        &mut world, crate::rubber::TAP_COOLDOWN_TICKS,
    );
    assert_eq!(restored, 0);
    assert!(!world.tapped_rubber_logs.contains_key(&(0, 64, 0)));
    assert_eq!(world.get_block(0, 64, 0), block::AIR);
}

#[test]
fn rebuild_tapped_rubber_logs_index_after_load() {
    let mut world = World::new();
    world.set_block(3, 64, 7, block::RUBBER_LOG_TAPPED);
    world.set_block(-5, 64, 2, block::RUBBER_LOG_TAPPED);
    assert!(world.tapped_rubber_logs.is_empty());
    world.rebuild_tapped_rubber_logs_index(5_000);
    assert!(world.tapped_rubber_logs.contains_key(&(3, 64, 7)));
    assert!(world.tapped_rubber_logs.contains_key(&(-5, 64, 2)));
    assert_eq!(world.tapped_rubber_logs.get(&(3, 64, 7)), Some(&5_000));
    assert_eq!(world.tapped_rubber_logs.len(), 2);
}

#[test]
fn slingshot_helpers_pure_contract() {
    let damage = crate::slingshot::slingshot_damage(
        crate::slingshot::SLINGSHOT_MAX_CHARGE_TICKS,
    );
    assert!((damage - crate::slingshot::SLINGSHOT_MAX_DAMAGE).abs() < 1e-3);
    let stun_cow = crate::slingshot::slingshot_stun_ticks(
        MobType::Cow, crate::slingshot::SLINGSHOT_MAX_CHARGE_TICKS,
    );
    assert_eq!(stun_cow, crate::slingshot::STUN_TICKS);
    let stun_brigand = crate::slingshot::slingshot_stun_ticks(
        MobType::Brigand, crate::slingshot::SLINGSHOT_MAX_CHARGE_TICKS,
    );
    assert_eq!(stun_brigand, 0);
}

#[test]
fn rubber_boots_sprint_multiplier_helper_pure() {
    use crate::armour::{ArmourMaterial, sprint_multiplier, has_rubber_boots};
    assert_eq!(sprint_multiplier(None), 1.0);
    assert_eq!(sprint_multiplier(Some(ArmourMaterial::Leather)), 1.0);
    // Task 15 (2026-07-07): capped at 1.4, down from the originally-designed
    // 2.0, to stay conservatively clear of the server anti-cheat speed gate
    // (server.rs MAX_HORIZONTAL_PER_TICK ≈ 1.6335 b/tick — a constant that is
    // FLY_SPRINT_SPEED-derived, not grounded-SPRINT_SPEED-derived as its label
    // implies; if it's ever corrected to SPRINT_SPEED × 1.5 the boots
    // multiplier must be revisited). Chosen instead of raising the server
    // cap. Full rationale on `armour::sprint_multiplier`'s doc comment.
    assert_eq!(sprint_multiplier(Some(ArmourMaterial::Rubber)), 1.4);
    assert!(sprint_multiplier(Some(ArmourMaterial::Rubber)) <= 1.5);
    assert!(has_rubber_boots(Some(ArmourMaterial::Rubber)));
    assert!(!has_rubber_boots(Some(ArmourMaterial::Iron)));
    assert!(!has_rubber_boots(None));
}

#[test]
fn eraser_wipe_preserves_name() {
    let mut p = crate::plan::PlanData::debug_3x3_stone();
    p.name = "My House".to_string();
    p.wipe_captured_cells();
    assert_eq!(p.name, "My House",
        "Eraser-driven wipe should preserve the Plan's chosen name");
}

#[test]
fn projectile_is_blunt_default_for_arrow_false() {
    // The ProjectileEntity::is_blunt field defaults to false for
    // arrow spawns, so the slingshot stun logic doesn't accidentally
    // fire on every arrow hit.
    let mut ecs = hecs::World::new();
    crate::entity::spawn_arrow(
        &mut ecs,
        glam::Vec3::new(0.0, 70.0, 0.0),
        glam::Vec3::new(1.0, 0.0, 0.0),
        crate::entity::ARROW_DAMAGE,
        None,
    );
    let mut found_arrow = 0;
    for (_, proj) in ecs.query::<&crate::entity::ProjectileEntity>().iter() {
        assert!(!proj.is_blunt, "spawn_arrow must leave is_blunt false");
        found_arrow += 1;
    }
    assert_eq!(found_arrow, 1);
}

#[test]
fn blunt_projectile_marks_is_blunt_true() {
    let mut ecs = hecs::World::new();
    crate::entity::spawn_blunt_projectile(
        &mut ecs,
        glam::Vec3::new(0.0, 70.0, 0.0),
        glam::Vec3::new(1.0, 0.0, 0.0),
        crate::slingshot::SLINGSHOT_MAX_DAMAGE,
        None,
    );
    let mut found_blunt = 0;
    for (_, proj) in ecs.query::<&crate::entity::ProjectileEntity>().iter() {
        assert!(proj.is_blunt, "spawn_blunt_projectile must stamp is_blunt true");
        found_blunt += 1;
    }
    assert_eq!(found_blunt, 1);
}
