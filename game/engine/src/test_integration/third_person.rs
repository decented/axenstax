//! Third-person camera — the **eye-anchored-aim invariant** guard (L3).
//!
//! The one rule a third-person camera must never break: aim (mining, placing,
//! combat, item-drops) keeps originating from the TRUE eye + look direction,
//! never the pulled-back render eye. So the crosshair's target cell is identical
//! in every `CameraMode` for the same position + yaw/pitch. If a future phase
//! ever wires an aim ray to `render_eye`, this fails immediately — and because
//! `check.sh` re-runs every test on every phase, it can't silently regress.
//!
//! Spec: `docs/foundations/2026-06-09-third-person-camera.md`
//! (§"The invariant we must not lose").

use glam::Vec3;

use crate::block::STONE;
use crate::camera::{Camera, CameraMode};
use crate::test_harness::{TestConfig, TestHost};

/// The third-person self-avatar must face the SAME horizontal direction the
/// camera looks — so you see the back of the head and it tracks your look rather
/// than counter-rotating. Regression for the 2026-06-09 playtest bug (avatar
/// faced the opposite/perpendicular way because camera-yaw was fed straight into
/// the entity-yaw builder). The fix routes through `entity_model::yaw_facing`.
#[test]
fn self_avatar_faces_where_the_camera_looks() {
    for deg in [0.0f32, 30.0, 90.0, 150.0, 210.0, 300.0] {
        let mut cam = Camera::new(Vec3::ZERO, 1.0);
        cam.yaw = deg.to_radians();
        let f = cam.forward();
        // The yaw the self-avatar is built with (see game_loop self-avatar block).
        let phi = crate::entity_model::yaw_facing(f.x, f.z);
        let (front_x, front_z) = crate::entity_model::avatar_front_dir(phi);
        let look = glam::vec2(f.x, f.z).normalize();
        assert!(
            (front_x - look.x).abs() < 1e-4 && (front_z - look.y).abs() < 1e-4,
            "avatar front ({front_x},{front_z}) must equal the look dir ({},{}) at yaw {deg}",
            look.x,
            look.y
        );
    }
}

/// The same yaw/pitch/position must produce the same crosshair target cell in
/// first-person, over-the-shoulder, and orbit-behind. A render-eye leak would
/// shift the ray sideways/back and strike a different cell — caught here.
#[test]
fn crosshair_target_is_invariant_across_camera_modes() {
    let mut host = TestHost::start_with(TestConfig::default());

    // Stand the player; look straight ahead (-Z, level pitch).
    host.teleport_player(0, Vec3::new(0.5, 10.0, 0.5));
    host.set_player_look(0, 0.0, 0.0);

    // A stone wall a few blocks ahead — tall and wide enough that a sideways or
    // vertical render-eye leak (the over-shoulder shoulder/up offset) would hit a
    // DIFFERENT cell rather than miss entirely. z = -3 is within reach (≈3.5 m).
    for x in -2..=2 {
        for y in 8..=14 {
            host.set_block(x, y, -3, STONE);
        }
    }

    let fp = {
        host.set_camera_mode(0, CameraMode::FirstPerson);
        host.crosshair_target(0)
    };
    let os = {
        host.set_camera_mode(0, CameraMode::OverShoulder);
        host.crosshair_target(0)
    };
    let ob = {
        host.set_camera_mode(0, CameraMode::OrbitBehind);
        host.crosshair_target(0)
    };

    // The ray genuinely hits the wall — guards against the degenerate
    // "all three are None, trivially equal" pass.
    assert!(fp.is_some(), "first-person crosshair must hit the wall");
    // The mode must not move the aim ray: same hit cell in all three.
    assert_eq!(fp, os, "over-shoulder aim diverged from first-person");
    assert_eq!(fp, ob, "orbit-behind aim diverged from first-person");
}

/// Switching camera mode does not change which block the crosshair targets even
/// when looking at an angle (turned-and-down) — the yaw/down directions the
/// render eye is offset along are exactly where a leak would show up.
#[test]
fn crosshair_target_invariant_when_aiming_off_axis() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.teleport_player(0, Vec3::new(0.5, 10.0, 0.5));
    // Turned ~34° off the -Z axis and tilted slightly down.
    host.set_player_look(0, 0.6, -0.2);

    // Enclose the player in a stone room so a turned, slightly-downward ray
    // reliably strikes a wall (or floor) within the 5-block reach.
    for a in -3..=3 {
        for y in 7..=13 {
            host.set_block(3, y, a, STONE); // +x wall
            host.set_block(-3, y, a, STONE); // -x wall
            host.set_block(a, y, 3, STONE); // +z wall
            host.set_block(a, y, -3, STONE); // -z wall
        }
        for b in -3..=3 {
            host.set_block(a, 7, b, STONE); // floor
        }
    }

    let fp = {
        host.set_camera_mode(0, CameraMode::FirstPerson);
        host.crosshair_target(0)
    };
    let os = {
        host.set_camera_mode(0, CameraMode::OverShoulder);
        host.crosshair_target(0)
    };
    let ob = {
        host.set_camera_mode(0, CameraMode::OrbitBehind);
        host.crosshair_target(0)
    };

    assert!(fp.is_some(), "angled crosshair must hit the room");
    assert_eq!(fp, os);
    assert_eq!(fp, ob);
}

/// Phase 2 — no-snap collision. In orbit-behind with a wall right behind the
/// player, the render eye clamps to BEFORE the wall (never inside it); remove the
/// wall and the camera EASES back out smoothly over several ticks — never a snap.
/// The aim-anchored invariant above still holds because collision only moves the
/// render eye, which `check.sh` re-verifies on every run.
#[test]
fn third_person_camera_clamps_before_a_wall_then_eases_back() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.teleport_player(0, Vec3::new(0.5, 10.0, 0.5));
    host.set_player_look(0, 0.0, 0.0); // face -Z → orbit camera pulls toward +Z
    host.set_camera_mode(0, CameraMode::OrbitBehind);

    let eye = host.camera_eye(0);

    // Clear path: after enough ticks the render eye reaches ~full pull-back.
    for _ in 0..200 {
        host.update_camera_collision(0, 0.05);
    }
    let clear_back = (host.camera_render_eye(0) - eye).length();
    assert!(
        clear_back > crate::camera::ORBIT_BEHIND_DISTANCE * 0.9,
        "clear path → near-full pull-back, got {clear_back}"
    );

    // Drop a wall right behind the player (the camera pulls toward +Z).
    for x in -1..=1 {
        for y in 8..=14 {
            host.set_block(x, y, 2, STONE);
        }
    }
    for _ in 0..60 {
        host.update_camera_collision(0, 0.05); // settle in to the wall
    }
    let walled = host.camera_render_eye(0);
    let walled_back = (walled - eye).length();
    assert!(
        walled_back < clear_back - 0.5,
        "camera retracted toward the wall ({walled_back} vs clear {clear_back})"
    );
    assert!(walled.z < 2.0, "render eye sits BEFORE the wall face (z={})", walled.z);

    // Remove the wall; the camera eases back out — smoothly, not in one snap.
    for x in -1..=1 {
        for y in 8..=14 {
            host.set_block(x, y, 2, crate::block::AIR);
        }
    }
    let before = (host.camera_render_eye(0) - eye).length();
    host.update_camera_collision(0, 0.05); // a single step
    let one_step = (host.camera_render_eye(0) - eye).length();
    assert!(one_step > before, "the camera eases back out once clear");
    assert!(
        (one_step - before) < (clear_back - walled_back) * 0.5,
        "single-step recovery is a smooth lerp, not a snap (step {} of total {})",
        one_step - before,
        clear_back - walled_back
    );

    // Given time it fully recovers to the clear pull-back.
    for _ in 0..200 {
        host.update_camera_collision(0, 0.05);
    }
    let recovered = (host.camera_render_eye(0) - eye).length();
    assert!(
        (recovered - clear_back).abs() < 0.1,
        "recovers to full extension, got {recovered} (clear {clear_back})"
    );
}

/// Phase 4 — per-block camera occlusion. A GLASS wall behind the player (solid
/// but transparent → PassThrough) must NOT clamp the camera — you can see through
/// it — whereas the same wall in STONE (Squeeze) does. The camera keys off
/// registry INTENT, not the visual box: Minecraft's Glass-vs-Barrier fix.
#[test]
fn glass_does_not_clamp_the_camera_but_stone_does() {
    let mut host = TestHost::start_with(TestConfig::default());
    host.teleport_player(0, Vec3::new(0.5, 10.0, 0.5));
    host.set_player_look(0, 0.0, 0.0); // face -Z → orbit pulls toward +Z
    host.set_camera_mode(0, CameraMode::OrbitBehind);
    let eye = host.camera_eye(0);

    // A GLASS wall right behind: see-through, so the camera passes through it.
    for x in -1..=1 {
        for y in 8..=14 {
            host.set_block(x, y, 2, crate::block::GLASS);
        }
    }
    for _ in 0..200 {
        host.update_camera_collision(0, 0.05);
    }
    let glass_back = (host.camera_render_eye(0) - eye).length();
    assert!(
        glass_back > crate::camera::ORBIT_BEHIND_DISTANCE * 0.9,
        "glass must NOT clamp the camera (see-through), got {glass_back}"
    );

    // Swap the same cells to STONE: now the camera clamps in (opaque → Squeeze).
    for x in -1..=1 {
        for y in 8..=14 {
            host.set_block(x, y, 2, STONE);
        }
    }
    for _ in 0..60 {
        host.update_camera_collision(0, 0.05);
    }
    let stone_back = (host.camera_render_eye(0) - eye).length();
    assert!(
        stone_back < glass_back - 0.5,
        "stone clamps the camera ({stone_back}) vs see-through glass ({glass_back})"
    );
}
