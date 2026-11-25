//! Eating you can see, on the GPU (ignored: needs a GPU adapter).
//!
//! `pt1124_eating_shots` stands the player on a marble pad and shoots, in first
//! person: cooked beef held to the mouth at 0%, ~35%, ~70% and ~95% of the bite
//! (with a burst of crumbs at 35%), a carrot, bread and a stew at ~70%; and in
//! third person from the front (the orbit camera swung round) the beef at ~50%.
//! `EAT_BEFORE=1` takes the same frames with no eating state set, which is what
//! the game drew before this feature. The folder is `EAT_SHOTS_OUT`, the window
//! `EAT_SIZE` (`1280x720` by default).
//!
//! Run: EAT_SHOTS_OUT=<dir> [EAT_BEFORE=1] cargo test --lib -- --ignored --test-threads=1 --nocapture pt1124_eating_shots
use super::torch_diag::{ground_y, place, pose_eye, set_time, settle};
use super::*;
use crate::block;
use crate::eating::Eating;
use crate::hands::Hand;
use crate::item::{ItemStack, MaterialId};
use crate::item_model::HeldItem;

fn hold(hg: &mut HeadlessGame, m: MaterialId) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, Some(ItemStack::new_material(m, 5)));
    p.hotbar_slot = 0;
    p.offhand = None;
    hg.state.eating.clear();
}

fn eat_at(hg: &mut HeadlessGame, m: MaterialId, progress: f32) {
    if std::env::var("EAT_BEFORE").is_ok() {
        return;
    }
    let held = HeldItem::of(&crate::item::Item::Material(m));
    hg.state.eating.insert(0, Eating::at_progress(held, Hand::Main, 0, progress));
}

fn snap(hg: &mut HeadlessGame, path: &str) {
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(path, 0, |_, _| {});
    }
    eprintln!("EATSHOT {path}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn pt1124_eating_shots() {
    isolate_saves();
    let out = std::env::var("EAT_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-eat-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("EAT_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("EAT_SIZE is <width>x<height>");
    let mut hg = HeadlessGame::boot_into_world_sized(
        "harness-eating-shots",
        w,
        h,
        crate::local_routing::LocalRouting::Standalone,
    );
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    hg.state.players[0].player.flying = true;
    hold(&mut hg, MaterialId::CookedBeef);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let top = p.y.floor() as i32 + 12;
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
    let (yaw, pitch) = (0.0_f32, -0.25_f32);

    for (name, m, progress) in [
        ("fp-beef-000", MaterialId::CookedBeef, 0.0),
        ("fp-beef-035", MaterialId::CookedBeef, 0.35),
        ("fp-beef-070", MaterialId::CookedBeef, 0.70),
        ("fp-beef-095", MaterialId::CookedBeef, 0.95),
        ("fp-carrot-070", MaterialId::Carrot, 0.70),
        ("fp-bread-070", MaterialId::Bread, 0.70),
        ("fp-stew-070", MaterialId::Stew, 0.70),
    ] {
        hold(&mut hg, m);
        eat_at(&mut hg, m, progress);
        pose_eye(&mut hg, eye, yaw, pitch);
        hg.frames(3);
        snap(&mut hg, &format!("{out}/{size}-{name}.png"));
    }

    // Crumbs at the first bite: the burst the game throws, then one frame.
    hold(&mut hg, MaterialId::CookedBeef);
    eat_at(&mut hg, MaterialId::CookedBeef, 0.35);
    pose_eye(&mut hg, eye, yaw, pitch);
    hg.frames(2);
    if !hg.state.eating.is_empty() {
        let slot = &hg.state.players[0];
        let mouth = crate::eating::mouth(true, slot.player.pos, slot.player.eye_pos(), slot.camera.forward());
        let colours = crate::eating::crumb_colours(crate::item_model::HeldItem::of(&crate::item::Item::Material(MaterialId::CookedBeef)).item, 0);
        hg.state.particles.burst_crumbs(mouth, &colours, 24, 1);
    }
    hg.frames(1);
    snap(&mut hg, &format!("{out}/{size}-fp-beef-035-crumbs.png"));

    // Third person, from the front.
    hold(&mut hg, MaterialId::CookedBeef);
    eat_at(&mut hg, MaterialId::CookedBeef, 0.50);
    hg.state.players[0].camera.mode = crate::camera::CameraMode::OrbitBehind;
    pose_eye(&mut hg, eye, yaw, -0.1);
    hg.state.players[0].camera.orbit_yaw_offset = std::f32::consts::PI;
    hg.frames(4);
    snap(&mut hg, &format!("{out}/{size}-tp-beef-050.png"));
    hg.state.players[0].camera.orbit_yaw_offset = 0.0;
    hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
}
