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

/// Review SF7 — the local place arm judges building over a block exactly as a
/// break: another seat's placed water source in a Commons is not to be built
/// over (the toast, nothing taken); a seat's own is.
#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_a_seat_cannot_build_over_another_seats_source_in_a_commons() {
    isolate_saves();
    let name = format!("authorship-local-over-{:?}", std::thread::current().id());
    let mut game = HeadlessGame::boot_into_world_routed(&name, LocalRouting::Standalone);
    game.state.ecs.clear();
    game.state.set_play_mode(PlayMode::Survival);
    let feet = clear_pad(&mut game, None);
    let centre = |c: [i32; 3]| glam::Vec3::new(c[0] as f32 + 0.5, c[1] as f32 + 0.5, c[2] as f32 + 0.5);
    game.state.world.set_block(feet[0] + 3, feet[1] - 1, feet[2], block::COMMONS_STONE);
    game.state.players[0].hotbar_slot = 0;
    game.state.players[0].inventory.set_slot(0, Some(ItemStack::new_block(block::STONE, 8)));
    let floor = [feet[0], feet[1] - 1, feet[2] - 2];
    let spot = [floor[0], floor[1] + 1, floor[2]];

    // Another seat's source: the ray passes through water to the floor, so the
    // placement cell is the source's.
    game.state.world.place_player_block_as(spot[0], spot[1], spot[2], block::WATER, Some(&AuthorKey::Local(1)));
    game.state.toast = None;
    aim_at(&mut game, centre(floor));
    right_click(&mut game);
    assert_eq!(game.state.world.get_block(spot[0], spot[1], spot[2]), block::WATER, "their source stands");
    assert_eq!(game.state.world.author_at(spot[0], spot[1], spot[2]), Some(&AuthorKey::Local(1)));
    assert_eq!(game.state.toast.as_ref().map(|t| t.0.as_str()), Some("Someone else built this."));
    assert_eq!(game.state.players[0].inventory.slot(0).map(|s| s.count), Some(8), "nothing was taken");

    // My own source: built over, and the stone is mine.
    game.state.world.place_player_block_as(spot[0], spot[1], spot[2], block::WATER, Some(&AuthorKey::Local(0)));
    aim_at(&mut game, centre(floor));
    right_click(&mut game);
    assert_eq!(game.state.world.get_block(spot[0], spot[1], spot[2]), block::STONE);
    assert_eq!(game.state.world.author_at(spot[0], spot[1], spot[2]), Some(&AuthorKey::Local(0)));
}

/// Review SF1 — breaking a door from the local arm (instant in creative, timed
/// in survival) takes its paired half's placed bit and author with it, so no
/// entry is left behind on empty space.
#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_breaking_a_door_leaves_no_author_on_its_other_half() {
    isolate_saves();
    let name = format!("authorship-local-door-{:?}", std::thread::current().id());
    let mut game = HeadlessGame::boot_into_world_routed(&name, LocalRouting::Standalone);
    game.state.ecs.clear();
    let feet = clear_pad(&mut game, None);
    let centre = |c: [i32; 3]| glam::Vec3::new(c[0] as f32 + 0.5, c[1] as f32 + 0.5, c[2] as f32 + 0.5);
    for (creative, dx) in [(true, 1), (false, -1)] {
        game.state.set_play_mode(if creative { PlayMode::Creative } else { PlayMode::Survival });
        let bottom = [feet[0] + dx, feet[1], feet[2] - 2];
        let top = [bottom[0], bottom[1] + 1, bottom[2]];
        let me = AuthorKey::Local(0);
        game.state.world.place_player_block_as(bottom[0], bottom[1], bottom[2], block::OAK_DOOR, Some(&me));
        game.state.world.place_player_block_as(top[0], top[1], top[2], block::OAK_DOOR, Some(&me));
        let top_meta = crate::use_edits::door_top_meta(game.state.world.meta_at(bottom[0], bottom[1], bottom[2]));
        game.state.world.set_meta((top[0], top[1], top[2]), top_meta);
        assert_eq!(game.state.world.author_at(top[0], top[1], top[2]), Some(&me));
        aim_at(&mut game, centre(bottom));
        hold_break(&mut game, bottom, 3000);
        assert_eq!(game.state.world.get_block(bottom[0], bottom[1], bottom[2]), block::AIR, "creative={creative}: the door broke");
        assert_eq!(game.state.world.get_block(top[0], top[1], top[2]), block::AIR, "creative={creative}: and its other half");
        assert!(!game.state.world.is_placed(top[0], top[1], top[2]), "creative={creative}: the other half is not left placed");
        assert!(game.state.world.authors.is_empty(), "creative={creative}: no author is left on empty space");
    }
}
