//! The shield through the real input and frame path, and on the GPU (ignored:
//! needs a GPU adapter).
//!
//! `game_harness_shield_shots` stands the player on a marble pad and holds the
//! use button (the mouse's right button) the way a player does: the frame path
//! latches the raise, the tick counts it and sets `PlayerIntent::guarding`, and
//! the viewmodel and the self-avatar lift the shield. It asserts that chain, then
//! shoots (first person, a sword main and a shield off, lowered and raised; a
//! shield main, raised; third person from the front, lowered and raised) and the
//! inventory icon at the hotbar's size and large. The window size is `SH_SIZE`
//! (`1280x720` by default); the folder `SH_SHOTS_OUT`.
//!
//! Run: SH_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_shield_shots
use super::torch_diag::{ground_y, place, set_time, settle};
use super::*;
use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::ItemStack;
use crate::shield::RAISE_TICKS;

fn hold(hg: &mut HeadlessGame, main: Option<ItemStack>, off: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, main);
    p.hotbar_slot = 0;
    p.offhand = off;
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
    eprintln!("SHSHOT {path}");
}

/// Frames until `done` holds, pausing so the wall-clock ticks come.
fn frames_until(hg: &mut HeadlessGame, what: &str, done: impl Fn(&HeadlessGame) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !done(hg) {
        hg.frames(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(std::time::Instant::now() < deadline, "never reached: {what}");
    }
}

/// Hold the use button until the shield has been raised for the whole delay.
fn raise(hg: &mut HeadlessGame) {
    hg.state.input.cursor_captured = true;
    hg.state.input.mouse_button_pressed(winit::event::MouseButton::Right);
    frames_until(hg, "the shield is raised", |hg| hg.state.players[0].shield_ticks >= RAISE_TICKS);
    // Past the eased-in part of the pose, still held.
    hg.frames(2);
}

fn lower(hg: &mut HeadlessGame) {
    hg.state.input.mouse_button_released(winit::event::MouseButton::Right);
    frames_until(hg, "the shield is lowered", |hg| hg.state.players[0].shield_ticks == 0);
}

fn guarding_flag(hg: &HeadlessGame) -> bool {
    hg.state.local_player_state(0).flags & crate::protocol::player_flags::GUARDING != 0
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_shield_shots() {
    isolate_saves();
    let out = std::env::var("SH_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-sh-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("SH_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("SH_SIZE is <width>x<height>");
    let mut hg =
        HeadlessGame::boot_into_world_sized("harness-shield-shots", w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    hold(&mut hg, None, None);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let gy = ground_y(&hg, tx, tz, p.y.floor() as i32 + 12);
    for dx in -8..=8 {
        for dz in -8..=8 {
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
    // Stand on the pad (survival: not flying), looking along -Z, a little down.
    let stand = |hg: &mut HeadlessGame, yaw: f32, pitch: f32| {
        let p = &mut hg.state.players[0];
        p.player.pos = glam::Vec3::new(fx, fy, fz);
        p.player.velocity = glam::Vec3::ZERO;
        p.player.flying = false;
        p.camera.yaw = yaw;
        p.camera.pitch = pitch;
        p.camera.position = p.player.eye_pos();
    };

    // First person, a sword main and a shield off: lowered, then raised through
    // the real input chain.
    hold(&mut hg, sword(), shield());
    stand(&mut hg, 0.0, -0.25);
    hg.frames(3);
    assert_eq!(hg.state.players[0].shield_ticks, 0, "nothing held: lowered");
    snap(&mut hg, &format!("{out}/{size}-fp-shield-lowered-sword.png"));
    let raised_at = std::time::Instant::now();
    raise(&mut hg);
    assert!(hg.state.players[0].shield_ticks >= RAISE_TICKS, "the held use button raised it ({:?})", raised_at.elapsed());
    assert!(guarding_flag(&hg), "and the avatar's state says GUARDING");
    assert!(hg.state.local_guard.contains(&0), "the frame path latched it");
    snap(&mut hg, &format!("{out}/{size}-fp-shield-raised-offhand.png"));
    lower(&mut hg);
    assert!(!guarding_flag(&hg), "released: lowered again");

    // First person, a shield main, raised.
    hold(&mut hg, shield(), None);
    stand(&mut hg, 0.0, -0.25);
    raise(&mut hg);
    snap(&mut hg, &format!("{out}/{size}-fp-shield-raised-main.png"));
    lower(&mut hg);

    // Third person from the front: a sword main, the shield off.
    hold(&mut hg, sword(), shield());
    stand(&mut hg, 0.0, -0.1);
    hg.state.players[0].camera.mode = crate::camera::CameraMode::OrbitBehind;
    hg.state.players[0].camera.orbit_yaw_offset = std::f32::consts::PI;
    hg.frames(4);
    snap(&mut hg, &format!("{out}/{size}-tp-shield-lowered.png"));
    raise(&mut hg);
    snap(&mut hg, &format!("{out}/{size}-tp-shield-raised.png"));
    lower(&mut hg);

    // The inventory icon, at the hotbar's size and large, over the pad.
    hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
    hold(&mut hg, shield(), None);
    stand(&mut hg, 0.0, -0.25);
    hg.frames(2);
    let path = format!("{out}/{size}-inventory-icon.png");
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(&path, 0, |ctx, ei| {
            egui::Area::new(egui::Id::new("shield-icon-shot")).fixed_pos(egui::pos2(60.0, 60.0)).show(ctx, |ui| {
                let slot = |ui: &egui::Ui, at: egui::Pos2, side: f32| {
                    let rect = egui::Rect::from_min_size(at, egui::vec2(side, side));
                    ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgba_unmultiplied(20, 20, 24, 230));
                    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, egui::Color32::GRAY), egui::StrokeKind::Inside);
                    crate::hud_ui::paint_shield_icon(ui, rect.shrink(4.0), ei);
                };
                slot(ui, egui::pos2(60.0, 60.0), 280.0);
                slot(ui, egui::pos2(380.0, 60.0), 48.0);
                slot(ui, egui::pos2(440.0, 60.0), 48.0);
            });
        });
    }
    eprintln!("SHSHOT {path}");
}
