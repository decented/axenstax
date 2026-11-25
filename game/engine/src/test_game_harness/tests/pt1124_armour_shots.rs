//! Armour on the GPU: worn on your own body in third person, held in the hand,
//! and dropped on the ground (ignored: needs a GPU adapter).
//!
//! `pt1124_armour_shots` stands the player on a marble pad and shoots:
//! - third person (F5), from behind and from the front, idle and mid-walk, in a
//!   full diamond set and in a mixed set (leather helmet, iron chestplate,
//!   diamond leggings, satori boots);
//! - first person holding a diamond chestplate;
//! - third person holding an iron helmet, from behind and from the front;
//! - four dropped pieces on the ground (helmet, chestplate, leggings, boots).
//!
//! The walk is driven by the real input (W held) so the pose reads the real local
//! state; each shot asserts the pose took before it is saved. The folder is
//! `PT_ARMOUR_OUT` (a temp dir by default); the window `PT_ARMOUR_SIZE`
//! (`1280x720` by default).
//!
//! Run: PT_ARMOUR_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture pt1124_armour_shots
use super::torch_diag::{ground_y, place, pose_eye, set_time, settle};
use super::*;
use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
use crate::block;
use crate::item::ItemStack;

type Worn = [Option<ArmourMaterial>; 4];

const SLOTS: [ArmourSlot; 4] = [ArmourSlot::Helmet, ArmourSlot::Chestplate, ArmourSlot::Leggings, ArmourSlot::Boots];

fn wear(hg: &mut HeadlessGame, set: Worn) {
    let p = &mut hg.state.players[0];
    for (i, slot) in SLOTS.iter().enumerate() {
        p.armour_slots[i] = set[i].map(|m| ArmourItem::new(*slot, m));
    }
}

fn hold(hg: &mut HeadlessGame, main: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, main);
    p.hotbar_slot = 0;
    p.offhand = None;
}

fn snap(hg: &mut HeadlessGame, path: &str) {
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(path, 0, |_, _| {});
    }
    eprintln!("ARMSHOT {path}");
}

/// The world time with the brightest sun (the fixed times other shots use read as
/// dusk here, which hides plate colours).
fn noon(hg: &HeadlessGame) -> u32 {
    (0..48_000u32)
        .step_by(250)
        .max_by(|a, b| {
            let sun = |t: &u32| crate::camera::compute_sun(hg.state.world.effective_world_time(*t)).1;
            sun(a).total_cmp(&sun(b))
        })
        .unwrap_or(6000)
}

#[derive(Clone, Copy)]
enum Motion {
    Idle,
    Walk,
}

#[test]
#[ignore = "needs a GPU adapter"]
fn pt1124_armour_shots() {
    isolate_saves();
    let out = std::env::var("PT_ARMOUR_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-pt1124-armour-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("PT_ARMOUR_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("PT_ARMOUR_SIZE is <width>x<height>");
    let mut hg =
        HeadlessGame::boot_into_world_sized("harness-pt1124-armour", w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    hold(&mut hg, None);
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
    let day = noon(&hg);
    eprintln!("ARMSHOT noon is world time {day}");
    set_time(&mut hg, day);
    let (fx, fy, fz) = (tx as f32 + 0.5, (gy + 1) as f32, tz as f32 + 0.5);

    // Third person: the look direction is -Z (yaw 0); `view_yaw` swings the orbit
    // camera round to the front. The body is driven by real input.
    let shoot = |hg: &mut HeadlessGame, name: &str, motion: Motion, view_yaw: f32| {
        use winit::keyboard::KeyCode;
        set_time(hg, day);
        let start_z = match motion {
            Motion::Idle => fz,
            Motion::Walk => fz + 8.0,
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
            Motion::Walk => Some(KeyCode::KeyW),
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut frames = 0;
        loop {
            {
                let p = &mut hg.state.players[0];
                p.camera.yaw = 0.0;
                p.camera.pitch = -0.1;
                p.camera.mode = crate::camera::CameraMode::OrbitBehind;
                p.camera.orbit_yaw_offset = view_yaw;
            }
            if let Some(k) = key {
                hg.state.input.key_pressed(k);
            }
            hg.frames(1);
            frames += 1;
            std::thread::sleep(std::time::Duration::from_millis(30));
            assert!(std::time::Instant::now() < deadline, "{name}: the pose never came");
            let state = hg.state.local_player_state(0);
            let t = hg.state.falling_tick_counter as f32 / 20.0;
            let done = match motion {
                Motion::Idle => frames >= 8,
                // Arms and legs near their largest swing.
                Motion::Walk => frames >= 15 && state.anim_state == 1 && (t * 2.5).sin().abs() > 0.93,
            };
            if done {
                break;
            }
        }
        if let Some(k) = key {
            hg.state.input.key_released(k);
        }
        let state = hg.state.local_player_state(0);
        let wanted = match motion {
            Motion::Idle => 0,
            Motion::Walk => 1,
        };
        eprintln!("ARMSHOT state {name}: anim_state {} (wanted {wanted})", state.anim_state);
        assert_eq!(state.anim_state, wanted, "{name}: the pose did not take");
        snap(hg, &format!("{out}/{size}-{name}.png"));
        let idle_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while hg.state.local_player_state(0).anim_state != 0 && std::time::Instant::now() < idle_deadline {
            hg.frames(1);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    };

    let full_diamond: Worn = [Some(ArmourMaterial::Diamond); 4];
    let mixed: Worn = [
        Some(ArmourMaterial::Leather),
        Some(ArmourMaterial::Iron),
        Some(ArmourMaterial::Diamond),
        Some(ArmourMaterial::Satori),
    ];

    // Worn: a full diamond set and a mixed set, behind and in front, idle and walking.
    hold(&mut hg, None);
    for (set_name, set) in [("diamond", full_diamond), ("mixed", mixed)] {
        wear(&mut hg, set);
        for (view, view_yaw) in [("rear", 0.0), ("front", std::f32::consts::PI)] {
            for (mname, motion) in [("idle", Motion::Idle), ("walk", Motion::Walk)] {
                shoot(&mut hg, &format!("worn-{set_name}-{view}-{mname}"), motion, view_yaw);
            }
        }
    }

    // Held in third person: an iron helmet in the main hand, nothing worn.
    wear(&mut hg, [None; 4]);
    hold(&mut hg, Some(ItemStack::new_armour(ArmourSlot::Helmet, ArmourMaterial::Iron)));
    for (view, view_yaw) in [("rear", 0.0), ("front", std::f32::consts::PI)] {
        shoot(&mut hg, &format!("held-3p-helmet-{view}"), Motion::Idle, view_yaw);
    }

    // Held in first person: a diamond chestplate. Looking along -Z, a little down.
    hold(&mut hg, Some(ItemStack::new_armour(ArmourSlot::Chestplate, ArmourMaterial::Diamond)));
    hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
    hg.state.players[0].player.flying = true;
    let eye = [fx, fy + 1.7, fz];
    pose_eye(&mut hg, eye, 0.0, -0.25);
    hg.state.local_swing.remove(&0);
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-held-1p-chestplate.png"));

    // Dropped: helmet, chestplate, leggings and boots in a row on the pad.
    hold(&mut hg, None);
    let drops = [
        (ArmourSlot::Helmet, ArmourMaterial::Iron, -1.5),
        (ArmourSlot::Chestplate, ArmourMaterial::Diamond, -0.5),
        (ArmourSlot::Leggings, ArmourMaterial::Leather, 0.5),
        (ArmourSlot::Boots, ArmourMaterial::Satori, 1.5),
    ];
    let spot = |dx: f32| glam::Vec3::new(fx + dx, fy + 0.3, fz - 3.5);
    for (slot, mat, dx) in drops {
        crate::entity::spawn_item(&mut hg.state.ecs, spot(dx), ItemStack::new_armour(slot, mat), 0);
    }
    for (_, (pos, vel, it)) in hg
        .state
        .ecs
        .query::<(&mut crate::entity::Position, &mut crate::entity::Velocity, &crate::entity::ItemEntity)>()
        .iter()
    {
        if let crate::item::Item::Armour(a) = &it.stack.item {
            if let Some((_, _, dx)) = drops.iter().find(|d| d.0 == a.slot) {
                pos.0 = spot(*dx);
                vel.0 = glam::Vec3::ZERO;
            }
        }
    }
    hg.ticks(60);
    // A drop spins with `falling_tick_counter`: pin it so the pieces turn about
    // 0.3 rad from facing the camera.
    let c0 = hg.state.falling_tick_counter;
    hg.state.falling_tick_counter = (c0..c0 + 200)
        .min_by(|a, b| {
            let off = |c: &u32| ((*c as f32 / 20.0 * 0.9).rem_euclid(std::f32::consts::TAU) - 0.3).abs();
            off(a).total_cmp(&off(b))
        })
        .unwrap();
    // Look at the row from the south (+Z), a little down.
    // Farther than the pickup magnet (2.4 blocks), or the pieces fly into the player.
    let drop_eye = [fx, fy + 1.8, fz - 3.5 + 4.2];
    let (dx, dy, dz) = (0.0_f32, fy + 0.3 - drop_eye[1], -4.2_f32);
    let (yaw, pitch) = ((-dx).atan2(-dz), dy.atan2((dx * dx + dz * dz).sqrt()));
    pose_eye(&mut hg, drop_eye, yaw, pitch);
    hg.frames(1);
    snap(&mut hg, &format!("{out}/{size}-dropped-row.png"));
}
