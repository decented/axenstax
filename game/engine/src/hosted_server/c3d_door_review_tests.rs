//! C3d run 20 — the run-19 review's door LOWs (LOW-2, LOW-4), pinned.
use super::c3d_client_edit_tests::joined;
use super::c3d_door_tests::{mirror_joined, stand_door};
use crate::action_identity::ActionFate;
use crate::block::{AIR, OAK_DOOR};
use crate::meta::Facing;

/// Run-19 review LOW-2: a Strict toggle from a dead body is superseded —
/// its cells go back — with NO correction `Replace` (a toggle predicted no
/// window; M1: a toggle never costs a correction).
#[test]
fn c3d_strict_toggle_from_a_dead_body_costs_no_correction() {
    let mut j = joined("door-dead-toggle", &[], false);
    for w in [&mut j.hs.server.world, &mut j.world] { stand_door(w, [41, 80, 40], Facing::North); }
    j.hs.server.players[j.i].combat.dead = true;
    j.toggle([41, 80, 40]);
    for _ in 0..4 { j.pump(); }
    assert!(!j.hs.disconnected[j.i], "never torn down");
    assert_eq!(j.sp().strict_corrections, crate::strict_admission::CorrectionRate::default(), "no correction Replace admitted");
    for c in [[41, 80, 40], [41, 81, 40]] {
        assert!(!crate::block_shape::is_open(j.server_cell(c).1), "the server's half at {c:?} stays shut");
        assert_eq!(j.client_cell(c), j.server_cell(c), "the client's half at {c:?} put back");
    }
}

/// Run-19 review LOW-4: an ACCEPTED toggle also puts back a cell only the
/// client's footprint held — here the client's stale world has a second
/// door beside the clicked one that the server's lacks; the client's
/// prediction flipped it, and the send-back removes the ghost.
#[test]
fn c3d_accepted_toggle_puts_back_a_ghost_companion() {
    let mut m = mirror_joined("mirror-toggle-ghost");
    stand_door(&mut m.hs.server.world, [41, 80, 40], Facing::North);
    stand_door(&mut m.world, [41, 80, 40], Facing::North);
    stand_door(&mut m.world, [42, 80, 40], Facing::North);
    assert_eq!(m.toggle([41, 80, 40]), ActionFate::Accepted, "served");
    for _ in 0..2 { m.pump(); }
    for y in [80, 81] {
        assert!(m.open_on_server([41, y, 40]) && m.open_on_client([41, y, 40]), "the real door's half at y={y} open on both");
        assert_eq!(m.world.get_block(42, y, 40), AIR, "the client's ghost half at y={y} put back as the server holds it");
    }
    assert_eq!(m.hs.server.world.get_block(41, 80, 40), OAK_DOOR);
}
