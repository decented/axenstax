//! A death's grave is drawn at once (ignored: needs a GPU adapter holding the atlas). The
//! death handler wrote the grave with a raw `World::set_block` and queued no remesh, so the
//! block existed, collided and was outlined, but the chunk's mesh never had it.
//! Run: cargo test --lib -- --ignored game_harness_grave
use super::*;
use crate::block;
use crate::item::ItemStack;
use crate::play_mode::PlayMode;
use crate::world::World;

/// The index count of the mesh the renderer holds for the chunk, 0 for none.
fn uploaded(hg: &HeadlessGame, chunk: (i32, i32, i32)) -> u32 {
    hg.state.renderer.chunk_meshes.get(&chunk).map_or(0, |m| m.index_count)
}

/// The index count the world would mesh to right now.
fn wanted(hg: &HeadlessGame, chunk: (i32, i32, i32)) -> u32 {
    crate::mesh::build_chunk_meshes(chunk.0, chunk.1, chunk.2, &hg.state.world, &hg.state.registry).opaque.indices.len() as u32
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test --lib -- --ignored game_harness_grave"]
fn game_harness_grave_a_death_grave_is_in_the_chunk_mesh() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-grave-mesh");
    hg.state.set_play_mode(PlayMode::Survival);
    hg.state.players[0].inventory = crate::inventory::Inventory::new();
    hg.state.players[0].inventory.set_slot(3, Some(ItemStack::new_block(block::DIRT, 4)));
    let feet = clear_pad(&mut hg, None);
    // The pad was cleared with raw writes: bring the mesh up to date first, so only the
    // grave can make a difference.
    hg.state.rebuild_chunk_at(feet[0], feet[1], feet[2]);
    let before_death = uploaded(&hg, World::block_to_chunk(feet[0], feet[1], feet[2]));

    hg.state.players[0].combat.take_damage(1000.0);
    assert!(hg.state.players[0].combat.dead);
    hg.ticks(3);

    let at = hg.state.players[0].last_grave.expect("a grave was placed");
    assert_eq!(hg.state.world.get_block(at[0], at[1], at[2]), block::GRAVE);
    let chunk = World::block_to_chunk(at[0], at[1], at[2]);
    assert_eq!(
        uploaded(&hg, chunk),
        wanted(&hg, chunk),
        "the chunk's mesh does not have the grave: it was {before_death} indices before the death"
    );
    assert_ne!(uploaded(&hg, chunk), before_death, "the grave added no faces to the mesh");
}
