//! Carried light on the GPU (ignored: needs a GPU adapter).
//!
//! A sealed stone room with no voxel light and no sky: the only light a shot can
//! show is what the holder carries. First person, 1280x720 by default (`CL_SIZE`).
//!
//! `game_harness_carried_light_shots` runs on the stone room: nothing in the hands,
//! a held torch (main and off-hand), a held lantern, and a lantern placed standing
//! and hanging. `game_harness_polish_shots` runs on an open marble pad by day:
//! the polished sword, the fishing rod and held bread, in first and third person.
//! The before shots (the commit before the lantern) have only the torch scenes.
//!
//! Run: CL_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_carried_light_shots game_harness_polish_shots
use super::torch_diag::{ground_y, place, pose_eye, set_time, settle};
use super::*;
use crate::block;
use crate::item::ItemStack;

pub(super) fn hold(hg: &mut HeadlessGame, main: Option<ItemStack>, off: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, main);
    p.hotbar_slot = 0;
    p.offhand = off;
}

pub(super) fn snap(hg: &mut HeadlessGame, path: &str) {
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(path, 0, |_, _| {});
    }
    eprintln!("CLSHOT {path}");
}

/// The window size from `CL_SIZE` (`1280x720` by default).
pub(super) fn size() -> (String, u32, u32) {
    let size = std::env::var("CL_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("CL_SIZE is <width>x<height>");
    (size, w, h)
}

/// Boots a world, builds the sealed room (a marble floor, stone walls and roof,
/// 13 x 13 inside and 4 high) and returns the game and the eye position.
pub(super) fn boot_cave(name: &str) -> (HeadlessGame, [f32; 3], (i32, i32, i32)) {
    isolate_saves();
    let (_, w, h) = size();
    let mut hg = HeadlessGame::boot_into_world_sized(name, w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    hg.state.players[0].player.flying = true;
    hold(&mut hg, None, None);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let top = p.y.floor() as i32 + 12;
    let gy = ground_y(&hg, tx, tz, top);
    // Clear a 15x15 footprint 7 high, lay the floor, then close it in.
    for dx in -7i32..=7 {
        for dz in -7i32..=7 {
            let (x, z) = (tx + dx, tz + dz);
            for dy in 1..=7 {
                if hg.state.world.get_block(x, gy + dy, z) != block::AIR {
                    place(&mut hg, x, gy + dy, z, block::AIR);
                }
            }
            if hg.state.world.get_block(x, gy, z) != block::MARBLE {
                place(&mut hg, x, gy, z, block::MARBLE);
            }
        }
    }
    for dx in -7i32..=7 {
        for dz in -7i32..=7 {
            let wall = dx.abs() == 7 || dz.abs() == 7;
            for dy in 1..=5 {
                let roof = dy == 5;
                if roof || wall {
                    place(&mut hg, tx + dx, gy + dy, tz + dz, block::STONE);
                }
            }
        }
    }
    // The room straddles chunk-column borders, and `update_for_block_change` re-runs
    // the sky pass for the placed block's own column only: light left in a
    // neighbour column before its roof went on spreads back in. Relight every
    // column the room touches from scratch (the torch diagnostic's recipe),
    // clearing all of them before any sky pass so none reseeds from another's
    // stale cells.
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let mut columns: Vec<(i32, i32)> = Vec::new();
    for dx in -7i32..=7 {
        for dz in -7i32..=7 {
            columns.push(((tx + dx).div_euclid(cs), (tz + dz).div_euclid(cs)));
        }
    }
    columns.sort_unstable();
    columns.dedup();
    let tops: Vec<i32> = columns.iter().map(|&(cx, cz)| hg.state.world.light_top(cx, cz)).collect();
    for (&(cx, cz), &top) in columns.iter().zip(&tops) {
        for lx in 0..cs {
            for lz in 0..cs {
                for y in 0..=top {
                    hg.state.world.set_sky_light_at(cx * cs + lx, y, cz * cs + lz, 0);
                }
            }
        }
    }
    for (&(cx, cz), &top) in columns.iter().zip(&tops) {
        crate::lighting::initial_sky_light_for_column(&mut hg.state.world, cx, cz, (0, top), &hg.state.registry);
    }
    for &(cx, cz) in &columns {
        hg.state.rebuild_chunks_for_lighting(cx * cs + cs / 2, gy + 2, cz * cs + cs / 2);
    }
    let leak = (-6i32..=6)
        .flat_map(|dx| (1..=4).flat_map(move |dy| (-6i32..=6).map(move |dz| (dx, dy, dz))))
        .map(|(dx, dy, dz)| hg.state.world.sky_light_at(tx + dx, gy + dy, tz + dz))
        .max()
        .unwrap_or(0);
    assert_eq!(leak, 0, "the sealed room must hold no sky light after the relight");
    settle(&mut hg);
    set_time(&mut hg, 12000);
    let eye = [tx as f32 + 0.5, (gy + 1) as f32 + 1.7, tz as f32 + 0.5];
    (hg, eye, (tx, gy, tz))
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_carried_light_shots() {
    let out = std::env::var("CL_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-cl-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let (size, _, _) = size();
    let (mut hg, eye, _) = boot_cave("harness-carried-light");
    let (yaw, pitch) = (0.0_f32, -0.15_f32);
    let torch = || Some(ItemStack::new_block(block::TORCH, 1));
    let lantern = || Some(ItemStack::new_block(block::LANTERN, 1));
    for (name, main, off) in [
        ("cave-empty", None, None),
        ("cave-held-torch", torch(), None),
        ("cave-offhand-torch", None, torch()),
        ("cave-held-lantern", lantern(), None),
        ("cave-offhand-lantern", None, lantern()),
    ] {
        hold(&mut hg, main, off);
        pose_eye(&mut hg, eye, yaw, pitch);
        hg.frames(3);
        snap(&mut hg, &format!("{out}/{size}-{name}.png"));
    }
}

/// Place a lantern with the meta its placement would stamp, then relight and remesh.
fn place_lantern(hg: &mut HeadlessGame, x: i32, y: i32, z: i32, face: [i32; 3]) {
    place(hg, x, y, z, block::LANTERN);
    hg.state.world.set_meta((x, y, z), crate::lantern::placement_meta(face));
    hg.state.rebuild_chunks_for_lighting(x, y, z);
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_placed_lantern_shots() {
    let out = std::env::var("CL_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-cl-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let (size, _, _) = size();
    let (mut hg, eye, (tx, gy, tz)) = boot_cave("harness-placed-lantern");
    hold(&mut hg, None, None);
    let (yaw, pitch) = (0.0_f32, -0.2_f32);
    // A standing lantern on the floor ahead, then a hanging one under the roof.
    place_lantern(&mut hg, tx, gy + 1, tz - 3, [0, 1, 0]);
    settle(&mut hg);
    pose_eye(&mut hg, eye, yaw, pitch);
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-cave-standing-lantern.png"));
    place(&mut hg, tx, gy + 1, tz - 3, block::AIR);
    place_lantern(&mut hg, tx, gy + 4, tz - 3, [0, -1, 0]);
    settle(&mut hg);
    pose_eye(&mut hg, eye, yaw, 0.25);
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-cave-hanging-lantern.png"));
    // Both, close, nothing else lit: the two shells side by side.
    place_lantern(&mut hg, tx - 1, gy + 1, tz - 3, [0, 1, 0]);
    settle(&mut hg);
    let close = [tx as f32 + 0.0, (gy + 1) as f32 + 1.3, tz as f32 - 0.2];
    pose_eye(&mut hg, close, yaw, 0.0);
    hg.frames(3);
    snap(&mut hg, &format!("{out}/{size}-cave-lanterns-close.png"));
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_polish_shots() {
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::item::MaterialId;
    isolate_saves();
    let out = std::env::var("CL_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-cl-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let (size, w, h) = size();
    let mut hg = HeadlessGame::boot_into_world_sized("harness-polish", w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    hg.state.players[0].player.flying = true;
    hold(&mut hg, None, None);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let gy = ground_y(&hg, tx, tz, p.y.floor() as i32 + 12);
    for dx in -8i32..=8 {
        for dz in -8i32..=8 {
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
    let eye = [tx as f32 + 0.5, (gy + 1) as f32 + 1.7, tz as f32 + 0.5];
    let tool = |t: ToolType| Some(ItemStack::new_tool(Tool::new(t, ToolMaterial::Iron)));
    let bread = || Some(ItemStack::new_material(MaterialId::Bread, 1));
    for (name, main, off) in [
        ("fp-sword", tool(ToolType::Sword), None),
        ("fp-fishing-rod", tool(ToolType::FishingRod), None),
        ("fp-bread", bread(), None),
        ("fp-sword-bread", tool(ToolType::Sword), bread()),
        ("fp-lantern", Some(ItemStack::new_block(block::LANTERN, 1)), None),
    ] {
        hold(&mut hg, main, off);
        pose_eye(&mut hg, eye, 0.0, -0.25);
        hg.frames(3);
        snap(&mut hg, &format!("{out}/{size}-{name}.png"));
    }
    // Third person, from the front: bread in the main hand, a lantern in the off-hand.
    hold(&mut hg, bread(), Some(ItemStack::new_block(block::LANTERN, 1)));
    hg.state.players[0].camera.mode = crate::camera::CameraMode::OrbitBehind;
    pose_eye(&mut hg, eye, 0.0, -0.1);
    hg.state.players[0].camera.orbit_yaw_offset = std::f32::consts::PI;
    hg.frames(4);
    snap(&mut hg, &format!("{out}/{size}-tp-bread-lantern.png"));
}
