//! The hand lantern on the GPU, before and after the look work (ignored: needs a
//! GPU adapter).
//!
//! `game_harness_lantern_shots` stands the player on a marble pad and shoots the
//! lantern four ways: its inventory icon (the real hotbar-slot painter, at slot
//! size and large, beside a torch and a stone block); a dropped lantern on the
//! ground beside a dropped torch and stone; the lantern held in first person (main
//! hand and off-hand); and the avatar carrying it in third person (F5, from behind
//! and from the front, main hand and off-hand, idle and mid-walk). The body is
//! driven by the real input (W held) so the avatar's pose reads the real state.
//! The window size is `LT_SIZE` (`1280x720` by default); the folder `LT_SHOTS_OUT`.
//!
//! Run: LT_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_lantern_shots
use super::torch_diag::{ground_y, place, pose_eye, set_time, settle};
use super::*;
use crate::block;
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

fn lantern() -> Option<ItemStack> {
    Some(ItemStack::new_block(block::LANTERN, 1))
}

fn snap(hg: &mut HeadlessGame, path: &str) {
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(path, 0, |_, _| {});
    }
    eprintln!("LTSHOT {path}");
}

fn aim(eye: [f32; 3], target: [f32; 3]) -> (f32, f32) {
    let d = glam::Vec3::from_array(target) - glam::Vec3::from_array(eye);
    let h = (d.x * d.x + d.z * d.z).sqrt();
    ((-d.x).atan2(-d.z), d.y.atan2(h))
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_lantern_shots() {
    use winit::keyboard::KeyCode;
    isolate_saves();
    let out = std::env::var("LT_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-lt-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("LT_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("LT_SIZE is <width>x<height>");
    let mut hg =
        HeadlessGame::boot_into_world_sized("harness-lantern-shots", w, h, crate::local_routing::LocalRouting::Standalone);
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

    // ── The inventory icon: the real hotbar-slot painter ─────────────────────
    // The headless game never uploads the block icons the HUD draws from.
    {
        let textures = crate::texture_gen::generate_textures();
        let r = &mut hg.state.renderer;
        r.egui.upload_block_textures(&r.device, &r.queue, &textures, 16);
    }
    hold(&mut hg, lantern(), None);
    pose_eye(&mut hg, [fx, fy + 1.62, fz], 0.0, -0.25);
    hg.frames(2);
    let path = format!("{out}/{size}-inventory-icons.png");
    let stacks = [
        ItemStack::new_block(block::LANTERN, 1),
        ItemStack::new_block(block::TORCH, 1),
        ItemStack::new_block(block::STONE, 1),
        ItemStack::new_block(block::PAPER_LANTERN_YELLOW, 1),
    ];
    for _ in 0..2 {
        let reg = &hg.state.registry;
        hg.state.renderer.render_view_to_png(&path, 0, |ctx, ei| {
            egui::Area::new(egui::Id::new("lantern-icon-shot")).fixed_pos(egui::pos2(40.0, 40.0)).show(ctx, |ui| {
                let slot = |ui: &egui::Ui, at: egui::Pos2, side: f32, stack: &ItemStack| {
                    let rect = egui::Rect::from_min_size(at, egui::vec2(side, side));
                    ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgba_unmultiplied(20, 20, 24, 230));
                    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, egui::Color32::GRAY), egui::StrokeKind::Inside);
                    crate::hud_ui::draw_stack_in_slot(ui, rect, stack, reg, ei, 1280.0);
                };
                // Large, then the hotbar's size, one of each.
                slot(ui, egui::pos2(40.0, 40.0), 280.0, &stacks[0]);
                for (i, s) in stacks.iter().enumerate() {
                    slot(ui, egui::pos2(360.0 + 60.0 * i as f32, 40.0), 48.0, s);
                }
            });
        });
    }
    eprintln!("LTSHOT {path}");

    // ── Dropped: a lantern, a torch and a stone block on the pad ─────────────
    hold(&mut hg, None, None);
    // Far from the drops while they land (a near player would pick them up).
    let away = [fx, fy + 1.62, fz + 8.0];
    pose_eye(&mut hg, away, 0.0, 0.0);
    hg.frames(2);
    let drop_eye = [fx - 1.0, fy + 1.0, fz - 5.0 + 2.2];
    let drop_target = [fx - 1.0, fy + 0.3, fz - 5.0];
    let spots = [
        (block::LANTERN, glam::Vec3::new(fx - 1.4, fy + 0.3, fz - 5.0)),
        (block::TORCH, glam::Vec3::new(fx - 1.0, fy + 0.3, fz - 5.0)),
        (block::STONE, glam::Vec3::new(fx - 0.6, fy + 0.3, fz - 5.0)),
    ];
    for (id, at) in spots {
        crate::entity::spawn_item(&mut hg.state.ecs, at, ItemStack::new_block(id, 1), 0);
    }
    let spot_of = |item: &crate::item::Item| match item {
        crate::item::Item::Block(id) => spots.iter().find(|(b, _)| b == id).map(|s| s.1),
        _ => None,
    };
    for (_, (pos, vel, it)) in hg
        .state
        .ecs
        .query::<(&mut crate::entity::Position, &mut crate::entity::Velocity, &crate::entity::ItemEntity)>()
        .iter()
    {
        if let Some(at) = spot_of(&it.stack.item) {
            pos.0 = at;
            vel.0 = glam::Vec3::ZERO;
        }
    }
    hg.ticks(60);
    // Pin the spin so each drop is turned about 0.3 rad from facing the camera.
    let c0 = hg.state.falling_tick_counter;
    hg.state.falling_tick_counter = (c0..c0 + 200)
        .min_by(|a, b| {
            let off = |c: &u32| ((*c as f32 / 20.0 * 0.9).rem_euclid(std::f32::consts::TAU) - 0.3).abs();
            off(a).total_cmp(&off(b))
        })
        .unwrap();
    let (yaw, pitch) = aim(drop_eye, drop_target);
    pose_eye(&mut hg, drop_eye, yaw, pitch);
    hg.frames(1);
    snap(&mut hg, &format!("{out}/{size}-drops.png"));
    // Everything the pad dropped is gone from the world before the body shots.
    let ids: Vec<hecs::Entity> = hg.state.ecs.query::<&crate::entity::ItemEntity>().iter().map(|(e, _)| e).collect();
    for e in ids {
        let _ = hg.state.ecs.despawn(e);
    }

    // ── First person, the lantern in a hand ──────────────────────────────────
    for (hand, main, off) in [("main", lantern(), None), ("off", None, lantern())] {
        hold(&mut hg, main, off);
        hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
        pose_eye(&mut hg, [fx, fy + 1.62, fz], 0.0, -0.25);
        hg.state.players[0].player.flying = false;
        hg.frames(3);
        snap(&mut hg, &format!("{out}/{size}-fp-{hand}.png"));
    }

    // ── Third person: idle and mid-walk, from behind and from the front ──────
    for (hand, main, off) in [("main", lantern(), None), ("off", None, lantern())] {
        hold(&mut hg, main, off);
        for (view, view_yaw) in [("rear", 0.0), ("front", std::f32::consts::PI)] {
            for walk in [false, true] {
                set_time(&mut hg, 6000);
                {
                    let p = &mut hg.state.players[0];
                    p.player.pos = glam::Vec3::new(fx, fy, if walk { fz + 8.0 } else { fz });
                    p.player.velocity = glam::Vec3::ZERO;
                    p.player.flying = false;
                }
                hg.state.input.cursor_captured = true;
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
                    if walk {
                        hg.state.input.key_pressed(KeyCode::KeyW);
                    }
                    hg.frames(1);
                    frames += 1;
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    assert!(std::time::Instant::now() < deadline, "{view} {hand}: the pose never came");
                    let state = hg.state.local_player_state(0);
                    let t = hg.state.falling_tick_counter as f32 / 20.0;
                    let done = if walk {
                        frames >= 15 && state.anim_state == 1 && (t * 2.5).sin().abs() > 0.93
                    } else {
                        frames >= 8
                    };
                    if done {
                        break;
                    }
                }
                hg.state.input.key_released(KeyCode::KeyW);
                let want = u8::from(walk);
                assert_eq!(hg.state.local_player_state(0).anim_state, want, "{view} {hand}: the pose did not take");
                snap(&mut hg, &format!("{out}/{size}-tp-{view}-{hand}-{}.png", if walk { "walk" } else { "idle" }));
                let idle_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while hg.state.local_player_state(0).anim_state != 0 && std::time::Instant::now() < idle_deadline {
                    hg.frames(1);
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
            }
        }
    }
}
