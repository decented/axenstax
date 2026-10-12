//! C3d run 20 — a Mirror joiner's typed `ToggleBlock` (run 19) keeps the
//! Mirror window order exactly as before: it neither depends on nor moves
//! the window boundary (run-19 review HIGH-1).
use super::c3d_door_tests::{mirror_joined, stand_door, MirrorJoined};
use super::*;
use crate::action_identity::ActionFate;
use crate::block::OAK_DOOR;
use crate::meta::Facing;

/// The game loop's Mirror toggle arm, without pumping: one reserved
/// `ToggleBlock` (hotbar slot 0), `FlipEach` predicted in its own world.
fn click(m: &mut MirrorJoined, cell: [i32; 3]) {
    let expected = m.world.get_block(cell[0], cell[1], cell[2]);
    crate::strict_doors::reserve_toggle(&mut m.rc, &m.world, cell, expected, 0).expect("reserved and sent");
    for (c, meta) in crate::strict_doors::toggle_metas(&m.world, cell, crate::strict_doors::OpenRule::FlipEach) {
        m.world.set_meta((c[0], c[1], c[2]), meta);
    }
}

fn open(world: &crate::world::World, c: [i32; 3]) -> bool { crate::block_shape::is_open(world.meta_at(c[0], c[1], c[2])) }

/// Run-19 review HIGH-1: five toggles in one burst (the item-action budget
/// is four, so the fifth waits a tick), then an input reporting a newer
/// window event the client applied (a grant) — the server applies it
/// before the fifth toggle runs. The toggle reads no window: it is served,
/// the seat is never torn down, and the door ends where the client has it.
#[test]
fn c3d_mirror_toggle_that_waits_past_a_window_boundary_move_is_served() {
    let mut m = mirror_joined("mirror-toggle-boundary");
    for w in [&mut m.hs.server.world, &mut m.world] { stand_door(w, [41, 80, 40], Facing::North); }
    let i = m.rc.player_index().expect("joined") as usize;
    let base = m.rc.applied_window_boundary();
    for _ in 0..5 { click(&mut m, [41, 80, 40]); }
    m.hs.grant_to_joiner(i, [crate::item::ItemStack::new_block(crate::block::DIRT, 1)]);
    let event = m.hs.server.players[i].window_events.last_sent();
    assert!(event > base, "the grant is a newer window event");
    m.rc.note_window_event(event);
    m.rc.send_input(&protocol::InputPacket { health: 20.0, ..Default::default() }); // alive (0 reports a death)
    for _ in 0..6 { m.pump(); }
    assert!(!m.hs.disconnected[i], "an honest Mirror joiner is never torn down by its own door click");
    assert_eq!(m.fates, vec![ActionFate::Accepted; 5], "all five toggles served");
    assert_eq!(m.hs.server.world.get_block(41, 80, 40), OAK_DOOR);
    for y in [80, 81] {
        assert!(open(&m.hs.server.world, [41, y, 40]), "five flips leave the server's half at y={y} open");
        assert!(open(&m.world, [41, y, 40]), "and the joiner's");
    }
}

/// Run-19 review MEDIUM-1: a Mirror toggle waits behind the joiner's
/// waiting edits, as every other Mirror request does. Six edits in one
/// input (four stone, then a door's two halves: the edit budget is four, so
/// the door waits a tick), then a click on that door: the server places the
/// door first and opens it — never judges the toggle against the world
/// before the client's own earlier edits.
#[test]
fn c3d_mirror_toggle_waits_behind_the_joiners_waiting_edits() {
    let mut m = mirror_joined("mirror-toggle-after-edits");
    let i = m.rc.player_index().expect("joined") as usize;
    let bottom = crate::meta::with_facing(0, Facing::North);
    let change = |x: i32, y: i32, z: i32, new_block, meta| protocol::BlockChange { x, y, z, new_block, meta };
    let edits = vec![
        change(39, 80, 40, crate::block::STONE, 0),
        change(39, 80, 41, crate::block::STONE, 0),
        change(39, 80, 39, crate::block::STONE, 0),
        change(40, 80, 42, crate::block::STONE, 0),
        change(42, 80, 41, OAK_DOOR, bottom),
        change(42, 81, 41, OAK_DOOR, crate::use_edits::door_top_meta(bottom)),
    ];
    for e in &edits {
        m.world.set_block(e.x, e.y, e.z, e.new_block);
        m.world.set_meta((e.x, e.y, e.z), e.meta);
    }
    m.rc.send_input(&protocol::InputPacket { block_changes: edits, health: 20.0, ..Default::default() });
    click(&mut m, [42, 80, 41]);
    for _ in 0..6 { m.pump(); }
    assert!(!m.hs.disconnected[i], "never torn down");
    assert_eq!(m.fates, vec![ActionFate::Accepted], "the toggle is served after the door it opens was placed");
    for y in [80, 81] {
        assert_eq!(m.hs.server.world.get_block(42, y, 41), OAK_DOOR, "the waiting edit placed the half at y={y}");
        assert!(open(&m.hs.server.world, [42, y, 41]), "and the toggle opened it");
        assert!(open(&m.world, [42, y, 41]), "as the joiner has it");
    }
}

/// Run 20 (with HIGH-1): a live Mirror toggle whose base predates a
/// whole-window `Replace` the client hadn't applied (here a correction
/// `Replace` queued after the click) is SERVED, not superseded: it read
/// nothing from the window and the client predicted nothing in it, so
/// there is nothing for the `Replace` to make stale. The door opens on both
/// copies; the seat is never torn down.
#[test]
fn c3d_mirror_toggle_made_before_a_replace_is_served() {
    let mut m = mirror_joined("mirror-toggle-pre-replace");
    for w in [&mut m.hs.server.world, &mut m.world] { stand_door(w, [41, 80, 40], Facing::North); }
    let i = m.rc.player_index().expect("joined") as usize;
    let base = m.rc.applied_window_boundary();
    click(&mut m, [41, 80, 40]);
    m.hs.queue_correction_replace(i);
    assert!(m.hs.server.players[i].window_events.predates_replace(base), "the click's base predates the Replace");
    for _ in 0..6 { m.pump(); }
    assert!(!m.hs.disconnected[i], "never torn down");
    assert_eq!(m.fates, vec![ActionFate::Accepted], "served, not superseded");
    for y in [80, 81] {
        assert!(open(&m.hs.server.world, [41, y, 40]), "the server's half at y={y} is open");
        assert!(open(&m.world, [41, y, 40]), "and the joiner's");
    }
}
