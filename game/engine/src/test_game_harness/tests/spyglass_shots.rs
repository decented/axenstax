//! The spyglass through the real input and frame path, and on the GPU (ignored:
//! needs a GPU adapter).
//!
//! `game_harness_spyglass_shots` stands the player on a marble pad and drives the
//! look the way a player does: a held right-click with a spyglass, the Zoom key
//! (C) with and without one, and the things that end a look (a hotbar change, F,
//! the inventory screen, the pause). It asserts that chain, then shoots: first
//! person holding it, first person looking (the lens painted as the HUD paints
//! it), third person raising it to the eye from the side and from behind, the
//! inventory icon, one lying on the ground, and its recipe card. The window size
//! is `SG_SIZE` (`1280x720` by default); the folder `SG_SHOTS_OUT`.
//!
//! Run: SG_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_spyglass_shots
use super::torch_diag::{ground_y, place, set_time, settle};
use super::*;
use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::ItemStack;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

fn spyglass() -> Option<ItemStack> {
    Some(ItemStack::new_tool(Tool::new(ToolType::Spyglass, ToolMaterial::Wood)))
}

fn sword() -> Option<ItemStack> {
    Some(ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron)))
}

fn hold(hg: &mut HeadlessGame, main: Option<ItemStack>, off: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, main);
    p.hotbar_slot = 0;
    p.offhand = off;
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

fn looking(hg: &HeadlessGame) -> bool {
    hg.state.players[0].sighting
}

fn zooming(hg: &HeadlessGame) -> bool {
    hg.state.players[0].camera.zooming
}

fn sighting_flag(hg: &HeadlessGame) -> bool {
    hg.state.local_player_state(0).flags & crate::protocol::player_flags::SIGHTING != 0
}

fn release_all(hg: &mut HeadlessGame) {
    hg.state.input.mouse_button_released(MouseButton::Right);
    hg.state.input.key_released(KeyCode::KeyC);
    hg.frames(2);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_spyglass_shots() {
    isolate_saves();
    let out = std::env::var("SG_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-sg-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("SG_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("SG_SIZE is <width>x<height>");
    let mut hg =
        HeadlessGame::boot_into_world_sized("harness-spyglass-shots", w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    {
        let textures = crate::texture_gen::generate_textures();
        let r = &mut hg.state.renderer;
        r.egui.upload_block_textures(&r.device, &r.queue, &textures, 16);
    }
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
    // Something to look at down the pad: a pillar of logs 7 blocks ahead.
    for dy in 1..=4 {
        place(&mut hg, tx, gy + dy, tz - 7, block::OAK_LOG);
    }
    settle(&mut hg);
    set_time(&mut hg, 6000);
    let (fx, fy, fz) = (tx as f32 + 0.5, (gy + 1) as f32, tz as f32 + 0.5);
    let stand = |hg: &mut HeadlessGame, yaw: f32, pitch: f32| {
        let p = &mut hg.state.players[0];
        p.player.pos = glam::Vec3::new(fx, fy, fz);
        p.player.velocity = glam::Vec3::ZERO;
        p.player.flying = false;
        p.camera.yaw = yaw;
        p.camera.pitch = pitch;
        p.camera.position = p.player.eye_pos();
    };
    let snap = |hg: &mut HeadlessGame, name: &str, lens: bool| {
        let path = format!("{out}/{size}-{name}.png");
        for _ in 0..2 {
            hg.state.renderer.render_view_to_png(&path, 0, |ctx, _| {
                if lens {
                    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32) / ctx.pixels_per_point().max(1.0));
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("spyglass-lens-shot")));
                    crate::hud_ui::paint_spyglass_lens(&painter, rect);
                }
            });
        }
        eprintln!("SGSHOT {path}");
    };
    hg.state.input.cursor_captured = true;

    // ── The rules through the real input chain ──────────────────────────────
    // C with nothing in a hand: no look, no zoom.
    hold(&mut hg, sword(), None);
    stand(&mut hg, 0.0, 0.0);
    hg.state.input.key_pressed(KeyCode::KeyC);
    hg.frames(3);
    assert!(!looking(&hg) && !zooming(&hg), "C without a spyglass does nothing");
    release_all(&mut hg);
    // C with a spyglass in the off-hand: looking, zoomed, slowed by the tick.
    hold(&mut hg, sword(), spyglass());
    hg.state.input.key_pressed(KeyCode::KeyC);
    hg.frames(2);
    assert!(looking(&hg) && zooming(&hg), "C with a spyglass off-hand looks");
    assert!(sighting_flag(&hg), "and the avatar's state says SIGHTING");
    // A hotbar change ends it, and holding C on does not bring it back.
    hg.state.players[0].hotbar_slot = 1;
    hg.frames(2);
    assert!(!looking(&hg) && !zooming(&hg), "a slot change ends the look");
    release_all(&mut hg);
    // Right-click with a spyglass in the main hand: looking.
    hold(&mut hg, spyglass(), None);
    hg.state.input.mouse_button_pressed(MouseButton::Right);
    frames_until(&mut hg, "the right-click look", looking);
    assert!(zooming(&hg));
    assert!(hg.state.local_sight.contains(&0) && !hg.state.local_guard.contains(&0), "a look, not a guard");
    // F ends it.
    hg.state.input.key_pressed(KeyCode::KeyF);
    hg.state.input.key_released(KeyCode::KeyF);
    hg.frames(2);
    assert!(!looking(&hg) && !zooming(&hg), "the swap ends the look");
    hg.frames(3);
    assert!(!looking(&hg), "the held button stays down until it is let go");
    release_all(&mut hg);
    // The inventory screen ends it.
    hold(&mut hg, spyglass(), None);
    hg.state.input.key_pressed(KeyCode::KeyC);
    hg.frames(2);
    assert!(looking(&hg));
    hg.state.input.key_pressed(KeyCode::KeyE);
    hg.state.input.key_released(KeyCode::KeyE);
    hg.frames(2);
    assert!(!looking(&hg) && !zooming(&hg), "a screen ends the look");
    hg.state.input.key_pressed(KeyCode::KeyE);
    hg.state.input.key_released(KeyCode::KeyE);
    hg.frames(2);
    release_all(&mut hg);
    hg.state.input.cursor_captured = true;
    // The pause ends it.
    hg.state.input.key_pressed(KeyCode::KeyC);
    hg.frames(2);
    assert!(looking(&hg));
    hg.state.mode = crate::GameMode::Paused { confirm_quit: false, confirm_creative: false };
    std::thread::sleep(std::time::Duration::from_millis(60));
    hg.frames(1);
    assert!(!looking(&hg) && !zooming(&hg), "the pause ends the look");
    hg.state.mode = crate::GameMode::Playing;
    hg.state.reset_tick_timing();
    release_all(&mut hg);
    hg.state.input.cursor_captured = true;

    // Looking places, mines and attacks nothing, and the tick walks at sneak speed.
    hold(&mut hg, Some(ItemStack::new_block(block::DIRT, 16)), spyglass());
    stand(&mut hg, 0.0, -0.9);
    let solid = |hg: &HeadlessGame| -> usize {
        let mut n = 0;
        for dx in -3..=3 {
            for dz in -4..=2 {
                for dy in 0..=3 {
                    if hg.state.world.get_block(tx + dx, gy + dy, tz + dz) != block::AIR {
                        n += 1;
                    }
                }
            }
        }
        n
    };
    let before = solid(&hg);
    hg.state.input.key_pressed(KeyCode::KeyC);
    hg.frames(2);
    assert!(looking(&hg));
    hg.state.input.mouse_button_pressed(MouseButton::Right);
    hg.state.input.mouse_button_pressed(MouseButton::Left);
    frames_until(&mut hg, "a tick while looking", |hg| hg.state.net_local_intent.sighting);
    hg.frames(6);
    assert!(looking(&hg), "still looking with both buttons held");
    assert!(!hg.state.net_local_intent.break_block && !hg.state.net_local_intent.place_block, "the tick acts on nothing");
    assert!(!hg.state.net_local_intent.guarding, "a look is not a guard");
    assert_eq!(solid(&hg), before, "nothing placed or broken");
    hg.state.input.mouse_button_released(MouseButton::Left);
    release_all(&mut hg);

    // ── The shots ───────────────────────────────────────────────────────────
    // First person, held, not looking.
    hold(&mut hg, spyglass(), None);
    stand(&mut hg, 0.0, -0.05);
    hg.frames(3);
    snap(&mut hg, "fp-held", false);
    // First person, looking through it (the lens is the HUD's own painter).
    hg.state.input.mouse_button_pressed(MouseButton::Right);
    frames_until(&mut hg, "looking for the shot", looking);
    hg.frames(2);
    snap(&mut hg, "fp-looking", true);
    release_all(&mut hg);

    // Third person: raised to the eye, from the side and from behind.
    hg.state.players[0].camera.mode = crate::camera::CameraMode::OrbitBehind;
    for (name, offset) in [("tp-raised-side", std::f32::consts::FRAC_PI_2), ("tp-raised-rear", 0.0)] {
        stand(&mut hg, 0.0, 0.0);
        hg.state.players[0].camera.orbit_yaw_offset = offset;
        hg.state.input.mouse_button_pressed(MouseButton::Right);
        frames_until(&mut hg, "looking in third person", looking);
        assert!(!zooming(&hg), "third person shows the avatar, not the lens");
        hg.frames(3);
        snap(&mut hg, name, false);
        release_all(&mut hg);
    }
    // Third person from the front, held lowered (for comparison).
    stand(&mut hg, 0.0, 0.0);
    hg.state.players[0].camera.orbit_yaw_offset = std::f32::consts::PI;
    hg.frames(3);
    snap(&mut hg, "tp-held-front", false);
    hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;

    // One on the ground, looked down at.
    hold(&mut hg, None, None);
    stand(&mut hg, 0.0, -0.75);
    crate::entity::spawn_item(&mut hg.state.ecs, glam::Vec3::new(fx, fy, fz - 1.6), spyglass().unwrap(), 0);
    hg.frames(4);
    snap(&mut hg, "dropped", false);

    // The inventory icon at the hotbar's size and large, and the recipe card.
    stand(&mut hg, 0.0, -0.25);
    hg.frames(2);
    let path = format!("{out}/{size}-icon-and-card.png");
    let tool = Tool::new(ToolType::Spyglass, ToolMaterial::Wood);
    let inventory = hg.state.players[0].inventory.clone();
    let mut book = crate::craft_ui::CraftingUi::new();
    book.open = true;
    book.book_open = true;
    book.book_search = "Spyglass".to_string();
    let vp = crate::screen::ViewportRect { x: 0, y: 0, width: w, height: h };
    let st = &mut hg.state;
    let registry = &st.registry;
    for _ in 0..2 {
        st.renderer.render_view_to_png(&path, 0, |ctx, _| {
            let _ = crate::recipe_book_ui::draw_recipe_book(ctx, &vp, 0, &mut book, &inventory, registry);
            egui::Area::new(egui::Id::new("spyglass-icon-shot")).order(egui::Order::Foreground).fixed_pos(egui::pos2(20.0, 20.0)).show(ctx, |ui| {
                let slot = |ui: &egui::Ui, at: egui::Pos2, side: f32| {
                    let rect = egui::Rect::from_min_size(at, egui::vec2(side, side));
                    ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgba_unmultiplied(20, 20, 24, 230));
                    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, egui::Color32::GRAY), egui::StrokeKind::Inside);
                    crate::item_icon::paint_tool_icon(ui.painter(), rect.shrink(4.0), tool);
                };
                slot(ui, egui::pos2(20.0, 20.0), 200.0);
                slot(ui, egui::pos2(230.0, 20.0), 48.0);
            });
        });
    }
    eprintln!("SGSHOT {path}");
}
