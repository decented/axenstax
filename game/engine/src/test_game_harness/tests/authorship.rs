//! Living world Tier 2 S2 through the real game (single-player): the place arm
//! records the seat that placed a block, and the break arm refuses another's
//! block inside a Commons with "Someone else built this." (creative bypasses,
//! an unrecorded or natural block is free, and breaking your own clears the record).
use super::*;
use crate::authorship::AuthorKey;
use crate::item::ItemStack;
use crate::local_routing::LocalRouting;
use crate::play_mode::PlayMode;
use crate::block;

/// Hold the left button on the aimed block for up to `frames` frames (one logical tick each;
/// a hand takes a few hundred to dig dirt in the headless frame clock).
fn hold_break(hg: &mut HeadlessGame, cell: [i32; 3], frames: u32) {
    hg.state.input.cursor_captured = true;
    hg.state.input.left_held = true;
    for _ in 0..frames {
        hg.state.tick_accumulator = crate::TICK_DURATION;
        hg.frames(1);
        if hg.state.world.get_block(cell[0], cell[1], cell[2]) == block::AIR {
            break;
        }
    }
    hg.state.input.left_held = false;
    hg.frames(1);
}

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_a_seat_places_as_itself_and_cannot_break_anothers_block_in_a_commons() {
    isolate_saves();
    let name = format!("authorship-local-{:?}", std::thread::current().id());
    let mut game = HeadlessGame::boot_into_world_routed(&name, LocalRouting::Standalone);
    game.state.ecs.clear();
    game.state.set_play_mode(PlayMode::Survival);
    let feet = clear_pad(&mut game, None);
    let centre = |c: [i32; 3]| glam::Vec3::new(c[0] as f32 + 0.5, c[1] as f32 + 0.5, c[2] as f32 + 0.5);

    // A Commons Stone in the floor three blocks over: the pad is inside its footprint.
    game.state.world.set_block(feet[0] + 3, feet[1] - 1, feet[2], block::COMMONS_STONE);
    assert_eq!(game.state.world.commons_areas().len(), 1);

    // The place arm: a block dirt placed on the floor is recorded under seat 0.
    game.state.players[0].hotbar_slot = 0;
    game.state.players[0].inventory.set_slot(0, Some(ItemStack::new_block(block::DIRT, 8)));
    let floor = [feet[0], feet[1] - 1, feet[2] - 2];
    let above = [floor[0], floor[1] + 1, floor[2]];
    aim_at(&mut game, centre(floor));
    right_click(&mut game);
    assert_eq!(game.state.world.get_block(above[0], above[1], above[2]), block::DIRT, "the block was placed");
    assert_eq!(game.state.world.author_at(above[0], above[1], above[2]), Some(&AuthorKey::Local(0)));

    // Another seat's block next to it (and a natural one, and one nobody is recorded for).
    let theirs = [feet[0] - 1, feet[1], feet[2] - 2];
    let natural = [feet[0] - 2, feet[1], feet[2] - 2];
    let unknown = [feet[0] + 1, feet[1], feet[2] - 2];
    game.state.world.place_player_block_as(theirs[0], theirs[1], theirs[2], block::DIRT, Some(&AuthorKey::Local(1)));
    game.state.world.set_block(natural[0], natural[1], natural[2], block::DIRT);
    game.state.world.place_player_block(unknown[0], unknown[1], unknown[2], block::DIRT);

    // Breaking another seat's block: refused, with the toast, and the block stands.
    game.state.toast = None;
    aim_at(&mut game, centre(theirs));
    hold_break(&mut game, theirs, 600);
    assert_eq!(game.state.world.get_block(theirs[0], theirs[1], theirs[2]), block::DIRT, "their block stands");
    assert_eq!(game.state.world.author_at(theirs[0], theirs[1], theirs[2]), Some(&AuthorKey::Local(1)));
    assert_eq!(game.state.toast.as_ref().map(|t| t.0.as_str()), Some("Someone else built this."));

    // Clear theirs out of the line of sight (a world edit, not a break), then: a
    // natural block and an unrecorded one are free; so is my own (its record goes with it).
    game.state.world.set_block(theirs[0], theirs[1], theirs[2], block::AIR);
    game.state.world.set_placed(theirs[0], theirs[1], theirs[2], false);
    for cell in [natural, unknown, above] {
        aim_at(&mut game, centre(cell));
        hold_break(&mut game, cell, 600);
        assert_eq!(game.state.world.get_block(cell[0], cell[1], cell[2]), block::AIR, "{cell:?} breaks");
    }
    assert_eq!(game.state.world.author_at(above[0], above[1], above[2]), None, "a break clears the author");

    // Creative bypasses: the other seat's block comes out.
    game.state.world.place_player_block_as(theirs[0], theirs[1], theirs[2], block::DIRT, Some(&AuthorKey::Local(1)));
    game.state.set_play_mode(PlayMode::Creative);
    aim_at(&mut game, centre(theirs));
    hold_break(&mut game, theirs, 600);
    assert_eq!(game.state.world.get_block(theirs[0], theirs[1], theirs[2]), block::AIR, "creative bypasses");
}
