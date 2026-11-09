//! Crops grow after a world is reloaded at any tick phase, through the real game:
//! single-player standalone and the local lending server. The machine phase runs
//! on every fourth beat of a counter that restarts at 0 on a load, and the
//! growth cadence reads the restored world clock; the two must keep one phase
//! (`sim_clock::beat_counter_for`).
use super::*;
use crate::{block, local_routing::LocalRouting};

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_a_crop_grows_after_a_reload_at_a_tick_off_the_machine_beat() {
    isolate_saves();
    for route in [LocalRouting::Standalone, LocalRouting::SoloHosted] {
        for saved_tick in [90_000u64, 90_001, 90_003] {
            let name = format!("growth-reload-{route:?}-{saved_tick}-{:?}", std::thread::current().id());
            let mut game = HeadlessGame::boot_into_world_routed(&name, route);
            game.state.tick_counter = saved_tick;
            assert!(game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
            game.reopen_world(&name);
            assert!(
                game.state.tick_counter >= saved_tick,
                "{route:?}: the saved clock is restored ({} < {saved_tick})",
                game.state.tick_counter
            );
            game.state.ecs.clear();
            let at = game.state.players[0].player.pos.as_ivec3();
            let cell = (at.x + 3, at.y, at.z);
            game.state.world.set_block(cell.0, cell.1 - 1, cell.2, block::TILLED_SOIL);
            game.state.world.set_block(cell.0, cell.1, cell.2, block::WHEAT_STAGE_0);
            game.state.world.set_sky_light_at(cell.0, cell.1, cell.2, 15);
            game.logical_ticks(2 * crate::growth::CROP_GROWTH_TICKS_PER_STAGE as u32 + 8);
            let now = game.state.world.get_block(cell.0, cell.1, cell.2);
            assert!(
                now != block::WHEAT_STAGE_0,
                "{route:?}, saved at tick {saved_tick} (mod 4 = {}): the crop never advanced",
                saved_tick % 4
            );
        }
    }
}
