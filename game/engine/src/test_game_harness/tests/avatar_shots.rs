//! The third-person avatar's held item and airborne legs, on the GPU (ignored:
//! needs a GPU adapter).
//!
//! `game_harness_avatar_shots` stands the player on a marble pad in third person
//! (F5, from behind and from the front) holding a torch in the main hand, then in
//! the off-hand, and shoots it idle, mid-walk, airborne travelling forwards and
//! airborne travelling backwards, plus a lowered shield on a walking avatar. The
//! body is driven by the real input (W or S held, Space for the jump) because the
//! avatar's pose reads the real local state: `anim_state` from the velocity and
//! `on_ground`, and the travel direction from the position change. It asserts each
//! pose took (`anim_state`) before the shot.
//! The window size is `AV_SIZE` (`1280x720` by default); the folder `AV_SHOTS_OUT`.
//!
//! Run: AV_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_avatar_shots
use super::torch_diag::{ground_y, place, set_time, settle};
use super::*;
use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::ItemStack;

fn hold(hg: &mut HeadlessGame, main: Option<ItemStack>, off: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, main);
    p.hotbar_slot = 0;
    p.offhand = off;
}

fn torch() -> Option<ItemStack> {
    Some(ItemStack::new_block(block::TORCH, 1))
}

fn sword() -> Option<ItemStack> {
    Some(ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron)))
}

fn shield() -> Option<ItemStack> {
    Some(ItemStack::new_tool(Tool::new(ToolType::Shield, ToolMaterial::Wood)))
}

fn snap(hg: &mut HeadlessGame, path: &str) {
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(path, 0, |_, _| {});
    }
    eprintln!("AVSHOT {path}");
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    Idle,
    Walk,
    /// Airborne, travelling along the look direction.
    AirForward,
    /// Airborne, travelling against the look direction.
    AirBackward,
}

impl Motion {
    fn name(self) -> &'static str {
        match self {
            Motion::Idle => "idle",
            Motion::Walk => "walk",
            Motion::AirForward => "air-forward",
            Motion::AirBackward => "air-backward",
        }
    }
    fn anim_state(self) -> u8 {
        match self {
            Motion::Idle => 0,
            Motion::Walk => 1,
            Motion::AirForward | Motion::AirBackward => 2,
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_avatar_shots() {
    isolate_saves();
    let out = std::env::var("AV_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-av-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("AV_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("AV_SIZE is <width>x<height>");
    let mut hg =
        HeadlessGame::boot_into_world_sized("harness-avatar-shots", w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    hold(&mut hg, None, None);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let gy = ground_y(&hg, tx, tz, p.y.floor() as i32 + 12);
    for dx in -9..=9 {
        for dz in -9..=9 {
            let (x, z) = (tx + dx, tz + dz);
            if hg.state.world.get_block(x, gy, z) != block::MARBLE {
                place(&mut hg, x, gy, z, block::MARBLE);
            }
            for dy in 1..=9 {
                if hg.state.world.get_block(x, gy + dy, z) != block::AIR {
                    place(&mut hg, x, gy + dy, z, block::AIR);
                }
            }
        }
    }
    settle(&mut hg);
    set_time(&mut hg, 6000);
    let (fx, fy, fz) = (tx as f32 + 0.5, (gy + 1) as f32, tz as f32 + 0.5);

    // The look direction is -Z (yaw 0): "forward" is decreasing z. The body is
    // driven the way a player drives it: W or S held (and Space for the jump), so
    // the pose comes from the real physics (`anim_state` from the velocity and
    // `on_ground`; the travel direction from the position change).
    let shoot = |hg: &mut HeadlessGame, name: &str, motion: Motion, view_yaw: f32| {
        use winit::keyboard::KeyCode;
        set_time(hg, 6000);
        let start_z = match motion {
            Motion::Idle => fz,
            Motion::Walk | Motion::AirForward => fz + 8.0,
            Motion::AirBackward => fz - 8.0,
        };
        {
            let p = &mut hg.state.players[0];
            p.player.pos = glam::Vec3::new(fx, fy, start_z);
            p.player.velocity = glam::Vec3::ZERO;
            p.player.flying = false;
        }
        hg.state.input.cursor_captured = true;
        let key = match motion {
            Motion::Idle => None,
            Motion::Walk | Motion::AirForward => Some(KeyCode::KeyW),
            Motion::AirBackward => Some(KeyCode::KeyS),
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut frames = 0;
        let mut jumped = false;
        loop {
            {
                let p = &mut hg.state.players[0];
                p.camera.yaw = 0.0;
                p.camera.pitch = -0.1;
                p.camera.mode = crate::camera::CameraMode::OrbitBehind;
                // The free-look offset eases back to zero while moving: hold it.
                p.camera.orbit_yaw_offset = view_yaw;
            }
            if let Some(k) = key {
                hg.state.input.key_pressed(k);
            }
            // Run up for a second, then jump while still moving.
            if matches!(motion, Motion::AirForward | Motion::AirBackward) && frames >= 14 && !jumped {
                hg.state.input.key_pressed(KeyCode::Space);
                jumped = true;
            }
            hg.frames(1);
            frames += 1;
            std::thread::sleep(std::time::Duration::from_millis(30));
            assert!(std::time::Instant::now() < deadline, "{name}: the pose never came");
            let state = hg.state.local_player_state(0);
            let rise = hg.state.players[0].player.pos.y - fy;
            let t = hg.state.falling_tick_counter as f32 / 20.0;
            let done = match motion {
                Motion::Idle => frames >= 8,
                // Arms near their largest swing.
                Motion::Walk => frames >= 15 && state.anim_state == 1 && (t * 2.5).sin().abs() > 0.93,
                // Well off the ground, near the top of the jump.
                Motion::AirForward | Motion::AirBackward => jumped && state.anim_state == 2 && rise > 0.6,
            };
            if done {
                break;
            }
        }
        if let Some(k) = key {
            hg.state.input.key_released(k);
        }
        hg.state.input.key_released(KeyCode::Space);
        let state = hg.state.local_player_state(0);
        let (vel, pos) = (hg.state.players[0].player.velocity, hg.state.players[0].player.pos);
        eprintln!(
            "AVSHOT state {name}: anim_state {} (wanted {}) flags {:#04x} vel {vel:?} pos {pos:?}",
            state.anim_state,
            motion.anim_state(),
            state.flags,
        );
        assert_eq!(state.anim_state, motion.anim_state(), "{name}: the pose did not take");
        snap(hg, &format!("{out}/{size}-{name}.png"));
        // Stop, and let the body come to rest before the next shot.
        let idle_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while hg.state.local_player_state(0).anim_state != 0 && std::time::Instant::now() < idle_deadline {
            hg.frames(1);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    };

    for (hand, main, off) in [("main", torch(), None), ("off", None, torch())] {
        hold(&mut hg, main, off);
        for (view, view_yaw) in [("rear", 0.0), ("front", std::f32::consts::PI)] {
            for motion in [Motion::Idle, Motion::Walk, Motion::AirForward, Motion::AirBackward] {
                shoot(&mut hg, &format!("{view}-torch-{hand}-{}", motion.name()), motion, view_yaw);
            }
        }
    }

    // A lowered shield on a walking avatar (a sword main, the shield off), from the front.
    hold(&mut hg, sword(), shield());
    shoot(&mut hg, "front-shield-lowered-walk", Motion::Walk, std::f32::consts::PI);
    shoot(&mut hg, "front-shield-lowered-air-forward", Motion::AirForward, std::f32::consts::PI);

    // From the side, where a leg's trail shows: the look direction is to the left of
    // the picture or the right, whichever way the orbit puts the camera.
    hold(&mut hg, torch(), None);
    for motion in [Motion::AirForward, Motion::AirBackward] {
        shoot(&mut hg, &format!("side-torch-main-{}", motion.name()), motion, std::f32::consts::FRAC_PI_2);
    }
}
