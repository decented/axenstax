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
use super::torch_diag::{ground_y, place, pose_eye, set_time, settle};
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

/// The marble pad the avatar shots stand on, and where the shots go.
struct Pad {
    out: String,
    size: String,
    fx: f32,
    fy: f32,
    fz: f32,
}

/// Boot a world, build the 19x19 marble pad with nine blocks of air above it, and
/// say where it is. The window size is `AV_SIZE`, the folder `out_env`.
fn boot_pad(world: &str, out_env: &str) -> (HeadlessGame, Pad) {
    isolate_saves();
    let out = std::env::var(out_env).unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-av-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("AV_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("AV_SIZE is <width>x<height>");
    let mut hg = HeadlessGame::boot_into_world_sized(world, w, h, crate::local_routing::LocalRouting::Standalone);
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
    let pad = Pad { out, size, fx: tx as f32 + 0.5, fy: (gy + 1) as f32, fz: tz as f32 + 0.5 };
    (hg, pad)
}

/// Stand the avatar on the pad in `motion`, with the orbit camera `view_yaw` round
/// the body (0 = from behind, pi = from the front), and shoot it to
/// `<out>/<size>-<name>.png`.
///
/// The look direction is -Z (yaw 0): "forward" is decreasing z. The body is driven
/// the way a player drives it: W or S held (and Space for the jump), so the pose
/// comes from the real physics (`anim_state` from the velocity and `on_ground`; the
/// travel direction from the position change).
fn shoot(hg: &mut HeadlessGame, pad: &Pad, name: &str, motion: Motion, view_yaw: f32) {
    use winit::keyboard::KeyCode;
    let (out, size, fx, fy, fz) = (&pad.out, &pad.size, pad.fx, pad.fy, pad.fz);
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
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_avatar_shots() {
    let (mut hg, pad) = boot_pad("harness-avatar-shots", "AV_SHOTS_OUT");
    for (hand, main, off) in [("main", torch(), None), ("off", None, torch())] {
        hold(&mut hg, main, off);
        for (view, view_yaw) in [("rear", 0.0), ("front", std::f32::consts::PI)] {
            for motion in [Motion::Idle, Motion::Walk, Motion::AirForward, Motion::AirBackward] {
                shoot(&mut hg, &pad, &format!("{view}-torch-{hand}-{}", motion.name()), motion, view_yaw);
            }
        }
    }

    // A lowered shield on a walking avatar (a sword main, the shield off), from the front.
    hold(&mut hg, sword(), shield());
    shoot(&mut hg, &pad, "front-shield-lowered-walk", Motion::Walk, std::f32::consts::PI);
    shoot(&mut hg, &pad, "front-shield-lowered-air-forward", Motion::AirForward, std::f32::consts::PI);

    // From the side, where a leg's trail shows: the look direction is to the left of
    // the picture or the right, whichever way the orbit puts the camera.
    hold(&mut hg, torch(), None);
    for motion in [Motion::AirForward, Motion::AirBackward] {
        shoot(&mut hg, &pad, &format!("side-torch-main-{}", motion.name()), motion, std::f32::consts::FRAC_PI_2);
    }
}

/// One labelled slot of a contact sheet, drawn by the hotbar's own slot painter.
type Sheet<'a> = &'a [(&'a str, ItemStack)];

/// Shoot `rows` of labelled slots (`side` px square, `gap` apart, the first row at
/// `(40, 30)`) over the pad's view, through the real slot painter.
fn shoot_sheet(hg: &mut HeadlessGame, path: &str, side: f32, rows: &[Sheet<'_>]) {
    let gap = 14.0;
    for _ in 0..2 {
        let reg = &hg.state.registry;
        hg.state.renderer.render_view_to_png(path, 0, |ctx, ei| {
            egui::Area::new(egui::Id::new("pt1124-sheet")).fixed_pos(egui::pos2(0.0, 0.0)).show(ctx, |ui| {
                let (w, h) = (ctx.content_rect().width(), ctx.content_rect().height());
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(w, h)),
                    0.0,
                    egui::Color32::from_rgb(52, 56, 64),
                );
                for (r, row) in rows.iter().enumerate() {
                    for (c, (label, stack)) in row.iter().enumerate() {
                        let at = egui::pos2(40.0 + (side + gap) * c as f32, 30.0 + (side + gap + 12.0) * r as f32);
                        let rect = egui::Rect::from_min_size(at, egui::vec2(side, side));
                        ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgba_unmultiplied(20, 20, 24, 230));
                        ui.painter().rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(1.0_f32, egui::Color32::GRAY),
                            egui::StrokeKind::Inside,
                        );
                        crate::hud_ui::draw_stack_in_slot(ui, rect, stack, reg, ei, 1280.0);
                        ui.painter().text(
                            rect.left_bottom() + egui::vec2(0.0, 2.0),
                            egui::Align2::LEFT_TOP,
                            label,
                            egui::FontId::proportional(9.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
            });
        });
    }
    eprintln!("AVSHOT {path}");
}

/// The torch (F5 rear and front, both hands, idle and walking; first person), and the
/// inventory icons of tools, armour, a Plan and shaped blocks, for the pt1124 visual
/// lane: run it before and after the change (`AV_SHOTS_OUT` names the folder).
///
/// Run: AV_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture pt1124_visual_shots
#[test]
#[ignore = "needs a GPU adapter"]
fn pt1124_visual_shots() {
    use crate::armour::{ArmourMaterial, ArmourSlot};
    use crate::building_variants::{self as bv, VShape};
    use crate::item::MaterialId;

    let (mut hg, pad) = boot_pad("harness-pt1124-visual", "AV_SHOTS_OUT");
    let (out, size) = (pad.out.clone(), pad.size.clone());

    // The headless game never uploads the block icons the HUD draws from.
    {
        let textures = crate::texture_gen::generate_textures();
        let r = &mut hg.state.renderer;
        r.egui.upload_block_textures(&r.device, &r.queue, &textures, 16);
    }

    // First person: the torch in the main hand, then the off-hand.
    for (hand, main, off) in [("main", torch(), None), ("off", None, torch())] {
        hold(&mut hg, main, off);
        hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
        pose_eye(&mut hg, [pad.fx, pad.fy + 1.62, pad.fz], 0.0, -0.25);
        hg.state.players[0].player.flying = false;
        hg.frames(3);
        snap(&mut hg, &format!("{out}/{size}-fp-torch-{hand}.png"));
    }

    // Third person (F5): rear and front, either hand, idle and walking.
    for (hand, main, off) in [("main", torch(), None), ("off", None, torch())] {
        hold(&mut hg, main, off);
        for (view, view_yaw) in [("rear", 0.0), ("front", std::f32::consts::PI)] {
            for motion in [Motion::Idle, Motion::Walk] {
                shoot(&mut hg, &pad, &format!("{view}-torch-{hand}-{}", motion.name()), motion, view_yaw);
            }
        }
    }

    // Inventory icons. Tools: each of the five tiered types in every material.
    hold(&mut hg, None, None);
    hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
    pose_eye(&mut hg, [pad.fx, pad.fy + 1.62, pad.fz], 0.0, -0.25);
    hg.frames(2);
    let tool = |t: ToolType, m: ToolMaterial| ItemStack::new_tool(Tool::new(t, m));
    let mats = [
        ("wood", ToolMaterial::Wood),
        ("stone", ToolMaterial::Stone),
        ("iron", ToolMaterial::Iron),
        ("diamond", ToolMaterial::Diamond),
        ("satori", ToolMaterial::Satori),
    ];
    let tiered = [
        ("pickaxe", ToolType::Pickaxe),
        ("axe", ToolType::Axe),
        ("sword", ToolType::Sword),
        ("shovel", ToolType::Shovel),
        ("hoe", ToolType::Hoe),
    ];
    let mut rows: Vec<Vec<(&str, ItemStack)>> = Vec::new();
    for (name, t) in tiered {
        rows.push(mats.iter().map(|(m, mat)| (if *m == "wood" { name } else { *m }, tool(t, *mat))).collect());
    }
    // The single-tier tools and the Plan.
    rows.push(vec![
        ("bow", tool(ToolType::Bow, ToolMaterial::Wood)),
        ("flint&steel", tool(ToolType::FlintAndSteel, ToolMaterial::Iron)),
        ("shears", tool(ToolType::Shears, ToolMaterial::Iron)),
        ("rod", tool(ToolType::FishingRod, ToolMaterial::Wood)),
        ("slingshot", tool(ToolType::Slingshot, ToolMaterial::Wood)),
        ("eraser", tool(ToolType::Eraser, ToolMaterial::Wood)),
        ("stamp", tool(ToolType::DraftingStamp, ToolMaterial::Wood)),
        ("shield", tool(ToolType::Shield, ToolMaterial::Wood)),
        (
            "plan",
            ItemStack {
                item: crate::item::Item::Plan(crate::plan::PlanData::debug_3x3_stone()),
                count: 1,
            },
        ),
    ]);
    // Armour: all four slots in two materials.
    for (name, m) in [("leather", ArmourMaterial::Leather), ("diamond", ArmourMaterial::Diamond)] {
        rows.push(
            [
                ("helmet", ArmourSlot::Helmet),
                ("chest", ArmourSlot::Chestplate),
                ("legs", ArmourSlot::Leggings),
                ("boots", ArmourSlot::Boots),
            ]
            .into_iter()
            .map(|(l, s)| (if l == "helmet" { name } else { l }, ItemStack::new_armour(s, m)))
            .collect(),
        );
    }
    let sheet: Vec<Sheet<'_>> = rows.iter().map(|r| r.as_slice()).collect();
    shoot_sheet(&mut hg, &format!("{out}/{size}-icons-tools-armour.png"), 56.0, &sheet);

    // Shaped blocks: spruce planks beside the spruce fence gate, slab, stairs, door,
    // trapdoor, a birch fence post, and the stone family; then some materials.
    let spruce = 2;
    let id = |m: usize, s: VShape| bv::id_of(m, s).expect("a variant");
    let blk = |id| ItemStack::new_block(id, 1);
    let mat = |m| ItemStack::new_material(m, 1);
    let rows: Vec<Vec<(&str, ItemStack)>> = vec![
        vec![
            ("spruce planks", blk(block::SPRUCE_PLANKS)),
            ("fence gate", blk(id(spruce, VShape::FenceGate))),
            ("slab", blk(id(spruce, VShape::Slab))),
            ("stairs", blk(id(spruce, VShape::Stairs))),
            ("door", blk(id(spruce, VShape::Door))),
            ("trapdoor", blk(id(spruce, VShape::Trapdoor))),
            ("birch post", blk(block::BIRCH_FENCE_POST)),
            ("spruce post", blk(block::SPRUCE_FENCE_POST)),
            ("oak post", blk(block::OAK_FENCE_POST)),
        ],
        vec![
            ("stone", blk(block::STONE)),
            ("stone slab", blk(id(bv::STONE, VShape::Slab))),
            ("stone stairs", blk(id(bv::STONE, VShape::Stairs))),
            ("cobble wall", blk(id(bv::COBBLESTONE, VShape::Wall))),
            ("oak door", blk(block::OAK_DOOR)),
            ("oak gate", blk(block::OAK_FENCE_GATE)),
            ("oak trapdoor", blk(block::OAK_TRAPDOOR)),
            ("torch", blk(block::TORCH)),
            ("lantern", blk(block::LANTERN)),
        ],
        vec![
            ("bread", mat(MaterialId::Bread)),
            ("coal", mat(MaterialId::Coal)),
            ("iron ingot", mat(MaterialId::IronIngot)),
            ("diamond", mat(MaterialId::Diamond)),
            ("satori", mat(MaterialId::Satori)),
            ("raw beef", mat(MaterialId::RawBeef)),
            ("flint", mat(MaterialId::Flint)),
            ("stick", mat(MaterialId::Stick)),
            ("carrot", mat(MaterialId::Carrot)),
        ],
    ];
    let sheet: Vec<Sheet<'_>> = rows.iter().map(|r| r.as_slice()).collect();
    shoot_sheet(&mut hg, &format!("{out}/{size}-icons-blocks.png"), 96.0, &sheet);
}
