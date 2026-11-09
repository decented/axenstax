//! Living world Tier 2 S3 through the real game (single-player): the seat loop calls
//! the shared use-record phase (a lane of grass wears into a Worn Path under a seat
//! pacing it, and a lived day is counted), and the local bed arm adds a night to
//! the bed's region before the clock jumps to morning.
use super::*;
use crate::block;
use crate::local_routing::LocalRouting;
use crate::play_mode::PlayMode;

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_a_seat_wears_a_path_and_a_bed_counts_a_night_in_single_player() {
    isolate_saves();
    let name = format!("use-record-local-{:?}", std::thread::current().id());
    let mut game = HeadlessGame::boot_into_world_routed(&name, LocalRouting::Standalone);
    game.state.ecs.clear();
    game.state.set_play_mode(PlayMode::Survival);
    let feet = clear_pad(&mut game, None);
    let floor_y = feet[1] - 1;
    for dx in 0..=1 {
        game.state.world.set_block(feet[0] + dx, floor_y, feet[2], block::GRASS);
    }

    // Pace two cells: each tick enters the other one. 24 footfalls each by tick 48.
    for i in 0..60 {
        let x = feet[0] as f32 + if i % 2 == 0 { 0.5 } else { 1.5 };
        let p = &mut game.state.players[0].player;
        p.pos = glam::Vec3::new(x, feet[1] as f32, feet[2] as f32 + 0.5);
        p.velocity = glam::Vec3::ZERO;
        game.ticks(1);
    }
    for dx in 0..=1 {
        assert_eq!(game.state.world.get_block(feet[0] + dx, floor_y, feet[2]), block::WORN_PATH, "cell {dx}");
        assert!(!game.state.world.is_placed(feet[0] + dx, floor_y, feet[2]));
    }
    assert_eq!(game.state.world.use_record.lived_day(), 1, "one lived day, however many ticks");
    let region = crate::use_record::region_of(feet[0], feet[2]);
    assert_eq!(game.state.world.use_record.get(region).map(|r| r.days), Some(1));

    // Creative wears nothing.
    game.state.set_play_mode(PlayMode::Creative);
    game.state.world.set_block(feet[0] + 2, floor_y, feet[2], block::GRASS);
    game.state.world.set_block(feet[0] + 3, floor_y, feet[2], block::GRASS);
    for i in 0..100 {
        let x = feet[0] as f32 + if i % 2 == 0 { 2.5 } else { 3.5 };
        game.state.players[0].player.pos = glam::Vec3::new(x, feet[1] as f32, feet[2] as f32 + 0.5);
        game.ticks(1);
    }
    assert_eq!(game.state.world.get_block(feet[0] + 2, floor_y, feet[2]), block::GRASS);
    game.state.set_play_mode(PlayMode::Survival);

    // The local bed arm: a night slept in a bed counts for the bed's region.
    let bed = [feet[0], feet[1], feet[2] - 2];
    game.state.world.set_block(bed[0], bed[1], bed[2], block::BED);
    game.state.write_clock(0, game.state.world_time_step);
    game.state.players[0].player.pos = glam::Vec3::new(bed[0] as f32 + 0.5, feet[1] as f32, bed[2] as f32 + 2.5);
    aim_at(&mut game, glam::Vec3::new(bed[0] as f32 + 0.5, bed[1] as f32 + 0.5, bed[2] as f32 + 0.5));
    right_click(&mut game);
    let bed_region = crate::use_record::region_of(bed[0], bed[2]);
    let night = game.state.world.use_record.get(bed_region).map(|r| r.nights);
    assert_eq!(night, Some(1), "the night counted once for the bed's region");
    assert!(game.state.world_time > 6000, "and the night was skipped as before");
}
