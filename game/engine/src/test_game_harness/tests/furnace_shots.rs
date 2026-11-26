//! The furnace screen on the GPU (ignored: needs a GPU adapter).
//!
//! `pt1124_furnace_shots` draws the open furnace screen over a live world view,
//! twice: an empty furnace above an inventory that holds ore, fuel and things the
//! furnace has no use for (bag and hotbar both), and a furnace part-way through a
//! smelt. It is the picture the owner asked for before and after the redesign:
//! the old screen showed no inventory at all.
//! The window size is `FU_SIZE` (`1280x720` by default); the folder `FU_SHOTS_OUT`.
//!
//! Run: FU_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture pt1124_furnace_shots
//! The block icons are uploaded as the windowed game does at start.
use super::*;
use crate::block;
use crate::furnace::FurnaceData;
use crate::item::{ItemStack, MaterialId};

#[test]
#[ignore = "needs a GPU adapter"]
fn pt1124_furnace_shots() {
    isolate_saves();
    let out = std::env::var("FU_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-fu-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let size = std::env::var("FU_SIZE").unwrap_or_else(|_| "1280x720".to_string());
    let (w, h) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("FU_SIZE is <width>x<height>");
    let mut hg =
        HeadlessGame::boot_into_world_sized("harness-furnace-shots", w, h, crate::local_routing::LocalRouting::Standalone);
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    // The headless renderer skips the block-icon upload the windowed game makes
    // at start (`Renderer::new`), so without this every block and material in a
    // slot would draw as a bare count or a flat swatch.
    {
        let textures = crate::texture_gen::generate_textures();
        let r = &mut hg.state.renderer;
        r.egui.upload_block_textures(&r.device, &r.queue, &textures, 16);
    }
    hg.frames(2);

    // Ore and fuel in the bag, fuel on the hotbar, and things a furnace has no use for.
    {
        let inv = &mut hg.state.players[0].inventory;
        for i in 0..36 {
            inv.set_slot(i, None);
        }
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Coal, 12)));
        inv.set_slot(1, Some(ItemStack::new_block(block::STONE, 40)));
        inv.set_slot(2, Some(ItemStack::new_material(MaterialId::Stick, 8)));
        inv.set_slot(11, Some(ItemStack::new_material(MaterialId::RawIron, 9)));
        inv.set_slot(12, Some(ItemStack::new_material(MaterialId::Copper, 4)));
        inv.set_slot(20, Some(ItemStack::new_block(block::OAK_PLANKS, 32)));
        inv.set_slot(21, Some(ItemStack::new_block(block::OAK_LOG, 6)));
        inv.set_slot(30, Some(ItemStack::new_block(block::DIRT, 64)));
    }
    let snap = |hg: &mut HeadlessGame, name: &str, data: &FurnaceData| {
        let path = format!("{out}/{name}.png");
        let st = &mut hg.state;
        let keys = st.input.bindings().clone();
        let registry = &st.registry;
        let inventory = &st.players[0].inventory;
        let vp = crate::screen::ViewportRect { x: 0, y: 0, width: w, height: h };
        // Twice: egui settles its sizing pass on the first.
        for _ in 0..2 {
            st.renderer.render_view_to_png(&path, 0, |ctx, egui| {
                let _ = crate::furnace_ui::draw_furnace_ui(ctx, &vp, 0, data, registry, egui, &keys, false, inventory);
            });
        }
        eprintln!("FUSHOT {path}");
    };

    snap(&mut hg, "furnace-empty", &FurnaceData::default());
    let mid_smelt = FurnaceData {
        input: Some(ItemStack::new_material(MaterialId::RawIron, 3)),
        fuel: Some(ItemStack::new_material(MaterialId::Coal, 2)),
        output: Some(ItemStack::new_material(MaterialId::IronIngot, 2)),
        smelt_progress: 80,
        smelt_total: 200,
        fuel_ticks_remaining: 2400,
        lit: true,
    };
    snap(&mut hg, "furnace-smelting", &mid_smelt);
}
