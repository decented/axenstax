//! C3d run 18 — doors (Decision 20): a Mirror joiner's toggle (item 0, a
//! finding), and the Strict compound door place, break and toggle.
use super::*;
use crate::action_identity::InventoryMode;
use crate::block::{AIR, OAK_DOOR, STONE};
use crate::remote_client::RemoteClient;

/// A Mirror (default) dedicated server whose joiner stands at (40.5, 80,
/// 40.5) on a stone floor, its column held, and the client's own world.
pub(super) struct MirrorJoined {
    _g: crate::save::WorldsRootGuard,
    pub(super) hs: HostedServer,
    pub(super) rc: RemoteClient,
    pub(super) world: crate::world::World,
}

pub(super) fn mirror_joined(tag: &str) -> MirrorJoined {
    let g = crate::save::WorldsRootGuard::new(&format!("c3d-door-{tag}"));
    let mut hs = super::tests::start_room_test_server(&format!("c3d-door-{tag}"), 0);
    hs.server.column_streamer = None;
    hs.server.column_refill_per_tick = 0;
    hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    for x in 32..=48 {
        for z in 32..=48 {
            hs.server.world.set_block(x, 79, z, STONE);
            for y in 80..=84 { hs.server.world.set_block(x, y, z, AIR); }
        }
    }
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let col = (40i32.div_euclid(cs), 40i32.div_euclid(cs));
    hs.server.loaded_columns.insert(col);
    let rc = RemoteClient::from_transport(Box::new(hs.attach_test_remote()), crate::remote_client::build_join_request_guest("Doorman", 0), None);
    let mut m = MirrorJoined { _g: g, hs, rc, world: crate::world::World::new() };
    for _ in 0..20 {
        m.pump();
        if m.rc.player_index().is_some() { break; }
    }
    let i = m.rc.player_index().expect("the guest joined") as usize;
    assert_eq!(m.rc.inventory_mode, InventoryMode::Mirror, "the session is Mirror");
    m.hs.hold_column_for_test(i, col);
    let p = &mut m.hs.server.players[i].player;
    p.pos = glam::Vec3::new(40.5, 80.0, 40.5);
    p.velocity = glam::Vec3::ZERO;
    p.on_ground = true;
    m
}

impl MirrorJoined {
    pub(super) fn pump(&mut self) {
        self.hs.tick();
        self.rc.poll();
        for change in std::mem::take(&mut self.rc.pending_block_changes) { self.world.apply_remote_block_change(&change); }
    }
}

/// A closed door (bottom at `bottom`, facing `f`) standing in `world`.
pub(super) fn stand_door(world: &mut crate::world::World, bottom: [i32; 3], f: crate::meta::Facing) {
    let m = crate::meta::with_facing(0, f);
    world.set_block(bottom[0], bottom[1], bottom[2], OAK_DOOR);
    world.set_meta((bottom[0], bottom[1], bottom[2]), m);
    world.set_block(bottom[0], bottom[1] + 1, bottom[2], OAK_DOOR);
    world.set_meta((bottom[0], bottom[1] + 1, bottom[2]), crate::use_edits::door_top_meta(m));
}

/// Item 0 (a finding, no behaviour change): a Mirror joiner opens a door;
/// does the server (the world every other player is served from) see it?
/// The joiner's toggle is the game loop's toggle arm (`game_loop.rs`, the
/// `is_toggleable` branch of the use click): it writes its own world's meta
/// and re-meshes, and pushes nothing onto `pending_block_changes`, so the
/// joiner's next movement packet carries no change for it.
#[test]
#[ignore = "Mirror multiplayer gap (C3d run 18 item 0): a door/gate/trapdoor toggle is client-local in every mode — the game loop's toggle arm (game_loop.rs `is_toggleable` branch) only writes its own world's meta and sends nothing, so the server and every other player keep the door shut. The coordinator decides the fix; Mirror is unchanged in run 18."]
fn c3d_mirror_joiner_door_toggle_reaches_the_server() {
    let mut m = mirror_joined("mirror-toggle");
    let bottom = [41, 80, 40];
    stand_door(&mut m.hs.server.world, bottom, crate::meta::Facing::North);
    stand_door(&mut m.world, bottom, crate::meta::Facing::North);
    // The joiner's toggle, as the game loop's arm does it: both halves of
    // its own door flipped open...
    for y in [80, 81] {
        let meta = m.world.meta_at(41, y, 40);
        m.world.set_meta((41, y, 40), crate::block_shape::toggled_open(meta));
    }
    assert!(crate::block_shape::is_open(m.world.meta_at(41, 80, 40)), "the joiner sees its door open");
    // ...then its ordinary movement packet, which carries no change for it.
    m.rc.send_input(&protocol::InputPacket::default());
    for _ in 0..4 { m.pump(); }
    for y in [80, 81] {
        assert!(crate::block_shape::is_open(m.hs.server.world.meta_at(41, y, 40)), "the server (and so every other player) sees the door at y={y} open");
    }
}
