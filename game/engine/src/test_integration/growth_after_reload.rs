//! A world saved at a tick that is not a multiple of 4 must still grow its
//! crops after a reload. The machine phase (growth, saplings, blight) runs on
//! every fourth beat of a separate counter that restarts at 0 on every load,
//! while the growth cadence reads the restored world clock; the two must keep
//! the same phase or the cadence never lands on a machine beat.
use crate::block;
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::save::WorldsRootGuard;

/// Ticks in a dry crop's growth stage; the run below covers two of them.
const RUN: u32 = 2 * crate::growth::CROP_GROWTH_TICKS_PER_STAGE as u32 + 8;

fn start(name: &str) -> HostedServer {
    let mut hs = HostedServer::start(0, name.to_string(), 42, 0, RemoteTransport::WebSocket { port: 0 })
        .expect("the server starts");
    hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    hs.server.ecs.clear();
    hs
}

fn plant(hs: &mut HostedServer) -> (i32, i32, i32) {
    let cell = (20, 80, 20);
    hs.server.world.set_block(cell.0, cell.1 - 1, cell.2, block::TILLED_SOIL);
    hs.server.world.set_block(cell.0, cell.1, cell.2, block::WHEAT_STAGE_0);
    hs.server.world.set_sky_light_at(cell.0, cell.1, cell.2, 15);
    cell
}

/// Save a dedicated server at `saved_tick`, restart it on the same folder,
/// plant a crop and run: how many stages did the crop advance?
fn stages_grown_after_reload(tag: &str, saved_tick: u64) -> i32 {
    let _g = WorldsRootGuard::new(&format!("growth-reload-{tag}"));
    let name = format!("growth-reload-{tag}");
    let mut hs = start(&name);
    hs.server.tick_counter = saved_tick;
    hs.server.try_save().unwrap();
    drop(hs);

    let mut hs = start(&name);
    assert_eq!(hs.server.tick_counter, saved_tick, "the clock is restored");
    let cell = plant(&mut hs);
    for _ in 0..RUN {
        hs.tick();
    }
    hs.server.world.get_block(cell.0, cell.1, cell.2) as i32 - block::WHEAT_STAGE_0 as i32
}

#[test]
fn a_dedicated_server_grows_crops_after_a_reload_at_any_tick() {
    // Control: a clock on the beat. Then every phase of the machine counter.
    for (i, saved_tick) in [90_000u64, 90_001, 90_002, 90_003, 90_005].into_iter().enumerate() {
        let grown = stages_grown_after_reload(&format!("ded{i}"), saved_tick);
        assert!(grown >= 1, "saved at tick {saved_tick} (mod 4 = {}): the crop never advanced", saved_tick % 4);
    }
}
