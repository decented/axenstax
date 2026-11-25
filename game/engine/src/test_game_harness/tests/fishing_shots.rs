//! A cast fishing line, on the GPU (ignored: needs a GPU adapter).
//!
//! `pt1124_fishing_shots` stands the player on a marble pad with a pond ahead,
//! holds a Fishing Rod, casts it through the real right-click onto the water and
//! shoots the first-person view and the third-person view (F5, from behind and
//! from the side), with the line settled and then with a bite (the bobber dips).
//! A last shot holds the rod uncast. The window size is `FISH_SIZE` (`1280x720`
//! by default); the folder `FISH_SHOTS_OUT`.
//!
//! Run: FISH_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture pt1124_fishing_shots
use super::torch_diag::{ground_y, place, set_time, settle};
use super::*;
use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::ItemStack;

fn snap(hg: &mut HeadlessGame, path: &str) {
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(path, 0, |_, _| {});
    }
    eprintln!("FISHSHOT {path}");
}

#[test]
#[ignore = "needs a GPU adapter"]
fn pt1124_fishing_shots() {
    isolate_saves();
    let out = std::env::var("FISH_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-fish-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("FISH_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("FISH_SIZE is <width>x<height>");
    let mut hg =
        HeadlessGame::boot_into_world_sized("harness-fishing-shots", w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let gy = ground_y(&hg, tx, tz, p.y.floor() as i32 + 12);
    // A marble pad with a pond (marble beneath, water in the top layer) ahead,
    // along -Z, 3 to 8 blocks from the feet and 9 wide.
    for dx in -9..=9 {
        for dz in -12..=9 {
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
    for dx in -4..=4 {
        for dz in -8..=-3 {
            place(&mut hg, tx + dx, gy, tz + dz, block::WATER);
        }
    }
    settle(&mut hg);
    set_time(&mut hg, 6000);
    let (fx, fy, fz) = (tx as f32 + 0.5, (gy + 1) as f32, tz as f32 + 0.5);
    let pond = glam::Vec3::new(fx, gy as f32 + 0.8, fz - 6.0);

    let rod = Some(ItemStack::new_tool(Tool::new(ToolType::FishingRod, ToolMaterial::Wood)));
    {
        let slot = &mut hg.state.players[0];
        for i in 0..9 {
            slot.inventory.set_slot(i, None);
        }
        slot.inventory.set_slot(0, rod);
        slot.hotbar_slot = 0;
        slot.fishing = None;
        slot.player.pos = glam::Vec3::new(fx, fy, fz);
        slot.player.velocity = glam::Vec3::ZERO;
        slot.player.flying = false;
        slot.camera.yaw = 0.0;
        slot.camera.mode = crate::camera::CameraMode::FirstPerson;
    }
    hg.frames(3);

    // The orbit view: yaw offset 0 = from behind, PI/2 = from the side.
    let view = |hg: &mut HeadlessGame, third: Option<f32>| {
        let c = &mut hg.state.players[0].camera;
        match third {
            Some(off) => {
                c.mode = crate::camera::CameraMode::OrbitBehind;
                c.orbit_yaw_offset = off;
            }
            None => {
                c.mode = crate::camera::CameraMode::FirstPerson;
                c.orbit_yaw_offset = 0.0;
            }
        }
    };

    // Uncast, first person: the rod as it is held with no line out.
    view(&mut hg, None);
    aim_at(&mut hg, pond);
    hg.frames(2);
    snap(&mut hg, &format!("{out}/{size}-fp-uncast.png"));

    // Cast through the real click.
    right_click(&mut hg);
    assert!(hg.state.players[0].fishing.is_some(), "the cast took");
    // The fly-out takes a few ticks; the settled line is a second on.
    hg.ticks(30);
    hg.frames(2);
    snap(&mut hg, &format!("{out}/{size}-fp-cast.png"));
    view(&mut hg, Some(0.0));
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-f5-rear-cast.png"));
    view(&mut hg, Some(std::f32::consts::FRAC_PI_2));
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-f5-side-cast.png"));

    // A bite: the line is hooked and the bobber dips.
    if let Some(line) = hg.state.players[0].fishing.as_mut() {
        line.catch_at_tick = 0;
    }
    hg.ticks(2);
    assert!(hg.state.players[0].fishing.is_some_and(|l| l.hooked), "the bite came");
    hg.frames(2);
    snap(&mut hg, &format!("{out}/{size}-f5-side-hooked.png"));
    view(&mut hg, Some(0.0));
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-f5-rear-hooked.png"));
    view(&mut hg, None);
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-fp-hooked.png"));
}
