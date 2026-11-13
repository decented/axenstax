//! The off-hand and the tool shapes on the GPU (ignored: needs a GPU adapter).
//!
//! Stands the player on a marble pad and shoots, in first person: an iron
//! pickaxe main with a torch off, an iron sword main with bread off, an empty
//! main with a torch off; in third person (the orbit camera swung round to the
//! front, F5 then free-look) a pickaxe main with a torch off; and every tool
//! type held on its own, first person, plus a contact sheet of the five
//! everyday tools (pickaxe, axe, sword, shovel, hoe). The window size comes
//! from `OH_SIZE` (`1280x720` by default, `720x1280` for phone portrait).
//!
//! Run: OH_SHOTS_OUT=<dir> OH_SIZE=1280x720 cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_offhand_shots
use super::torch_diag::{ground_y, place, pose_eye, set_time, settle};
use super::*;
use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{ItemStack, MaterialId};

fn hold(hg: &mut HeadlessGame, main: Option<ItemStack>, off: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, main);
    p.hotbar_slot = 0;
    p.offhand = off;
}

fn tool(t: ToolType) -> Option<ItemStack> {
    Some(ItemStack::new_tool(Tool::new(t, ToolMaterial::Iron)))
}

fn snap(hg: &mut HeadlessGame, path: &str) {
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(path, 0, |_, _| {});
    }
    eprintln!("OHSHOT {path}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_offhand_shots() {
    isolate_saves();
    let out = std::env::var("OH_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-oh-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("OH_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("OH_SIZE is <width>x<height>");
    let mut hg = HeadlessGame::boot_into_world_sized(
        "harness-offhand-shots",
        w,
        h,
        crate::local_routing::LocalRouting::Standalone,
    );
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    hg.state.players[0].player.flying = true;
    hold(&mut hg, None, None);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let top = p.y.floor() as i32 + 12;

    // A 17x17 marble pad, everything cleared 9 high above it.
    let gy = ground_y(&hg, tx, tz, top);
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
    set_time(&mut hg, 12000);
    let (fx, fy, fz) = (tx as f32, (gy + 1) as f32, tz as f32);
    let eye = [fx + 0.5, fy + 1.7, fz + 0.5];
    // Look along -Z, a little down: pitch -0.25 rad.
    let (yaw, pitch) = (0.0_f32, -0.25_f32);

    let torch = || Some(ItemStack::new_block(block::TORCH, 1));
    let bread = || Some(ItemStack::new_material(MaterialId::Bread, 1));

    // First person.
    for (name, main, off) in [
        ("fp-pickaxe-torch", tool(ToolType::Pickaxe), torch()),
        ("fp-sword-bread", tool(ToolType::Sword), bread()),
        ("fp-empty-torch", None, torch()),
    ] {
        hold(&mut hg, main, off);
        pose_eye(&mut hg, eye, yaw, pitch);
        hg.frames(3);
        snap(&mut hg, &format!("{out}/{size}-{name}.png"));
    }

    // Every tool type on its own, first person (the row of five is a contact
    // sheet of the first five).
    let types: [(&str, ToolType); 12] = [
        ("pickaxe", ToolType::Pickaxe),
        ("axe", ToolType::Axe),
        ("sword", ToolType::Sword),
        ("shovel", ToolType::Shovel),
        ("hoe", ToolType::Hoe),
        ("bow", ToolType::Bow),
        ("flint-and-steel", ToolType::FlintAndSteel),
        ("shears", ToolType::Shears),
        ("fishing-rod", ToolType::FishingRod),
        ("slingshot", ToolType::Slingshot),
        ("eraser", ToolType::Eraser),
        ("drafting-stamp", ToolType::DraftingStamp),
    ];
    for (name, t) in types {
        hold(&mut hg, tool(t), None);
        pose_eye(&mut hg, eye, yaw, pitch);
        hg.frames(3);
        snap(&mut hg, &format!("{out}/{size}-tool-{name}.png"));
    }
    // The row: the lower-right 45% x 60% of each of the five.
    let crops: Vec<image::RgbaImage> = types[..5]
        .iter()
        .map(|(name, _)| {
            let img = image::open(format!("{out}/{size}-tool-{name}.png")).unwrap().to_rgba8();
            let (cw, ch) = (img.width() * 45 / 100, img.height() * 60 / 100);
            image::imageops::crop_imm(&img, img.width() - cw, img.height() - ch, cw, ch).to_image()
        })
        .collect();
    let (cw, ch) = (crops[0].width(), crops[0].height());
    let mut row = image::RgbaImage::new(cw * crops.len() as u32, ch);
    for (i, c) in crops.iter().enumerate() {
        image::imageops::replace(&mut row, c, (i as u32 * cw) as i64, 0);
    }
    row.save(format!("{out}/{size}-tool-row.png")).unwrap();
    eprintln!("OHSHOT {out}/{size}-tool-row.png");

    // Third person, from the front: the orbit camera swung round the player.
    hold(&mut hg, tool(ToolType::Pickaxe), torch());
    hg.state.players[0].camera.mode = crate::camera::CameraMode::OrbitBehind;
    pose_eye(&mut hg, eye, yaw, -0.1);
    hg.state.players[0].camera.orbit_yaw_offset = std::f32::consts::PI;
    hg.frames(4);
    snap(&mut hg, &format!("{out}/{size}-tp-pickaxe-torch.png"));
    hg.state.players[0].camera.orbit_yaw_offset = 0.0;
    hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
}
