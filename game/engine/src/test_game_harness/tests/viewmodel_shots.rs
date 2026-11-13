//! First-person hand and held item on the GPU (ignored: needs a GPU adapter).
//!
//! Stands the player on a marble pad, looking a little down at it, and shoots
//! the first-person view holding a torch, an iron sword, a stone block, an oak
//! sapling and nothing, each at rest and at the swing peak. The window size
//! comes from `VM_SIZE` (`1280x720` by default, `720x1280` for phone portrait).
//! The projected numbers for the same poses are logged by the CPU test
//! `viewmodel::layout_tests::viewmodel_layout_numbers`.
//!
//! Run: VM_SHOTS_OUT=<dir> VM_SIZE=1280x720 cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_viewmodel_shots
use super::torch_diag::{ground_y, place, pose_eye, settle, set_time};
use super::*;
use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::ItemStack;

fn hold(hg: &mut HeadlessGame, stack: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, stack);
    p.hotbar_slot = 0;
}

#[test]
#[ignore = "needs a GPU adapter"]
fn game_harness_viewmodel_shots() {
    isolate_saves();
    let out = std::env::var("VM_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-vm-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("VM_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("VM_SIZE is <width>x<height>");
    let mut hg = HeadlessGame::boot_into_world_sized(
        "harness-viewmodel-shots",
        w,
        h,
        crate::local_routing::LocalRouting::Standalone,
    );
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    hg.state.players[0].player.flying = true;
    hold(&mut hg, None);
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

    let things: [(&str, Option<ItemStack>); 5] = [
        ("empty", None),
        ("torch", Some(ItemStack::new_block(block::TORCH, 1))),
        ("sword", Some(ItemStack::new_tool(Tool::new(ToolType::Sword, ToolMaterial::Iron)))),
        ("stone", Some(ItemStack::new_block(block::STONE, 1))),
        ("sapling", Some(ItemStack::new_block(block::SAPLING_OAK, 1))),
    ];
    for (what, stack) in things {
        for pose in ["rest", "swing"] {
            hold(&mut hg, stack.clone());
            pose_eye(&mut hg, eye, yaw, pitch);
            hg.state.local_swing.remove(&0);
            hg.frames(2);
            if pose == "swing" {
                // Part-way down the window: progress 0.4, 0.69 rad of the 0.70 peak.
                hg.state.local_swing.insert(0, 4);
            }
            hg.frames(1);
            let path = format!("{out}/{size}-{what}-{pose}.png");
            for _ in 0..2 {
                hg.state.renderer.render_view_to_png(&path, 0, |_, _| {});
            }
            eprintln!("VMSHOT {path}");
        }
    }
}
