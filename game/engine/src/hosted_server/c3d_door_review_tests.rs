//! C3d run 20 — the run-19 review's door LOWs (LOW-2, LOW-4), pinned.
//! C3d run 21 — the run-18 review's door LOWs (L3, L5, L6), pinned.
use super::c3d_client_edit_tests::joined;
use super::c3d_door_tests::{doors, mirror_joined, stand_door};
use crate::item::Item;
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

/// Run-18 review L3: a Strict door break journals BOTH halves (the client
/// predicts its pair cleared), and the server's row carries both: the
/// journal's cells and the row's are the same union S3 recovery reads.
#[test]
fn c3d_strict_door_break_journals_both_halves() {
    let mut j = joined("door-break-journal", &[(0, doors(1))], false);
    j.place_door([41, 80, 40], 0, Facing::South);
    j.assert_honest("place");
    j.break_door([41, 81, 40], 1);
    let journaled: Vec<[i32; 3]> = j.rc.action_journal.as_ref().and_then(|journal| journal.actions().last()).expect("the break reserved").cells.to_vec();
    assert_eq!(journaled, vec![[41, 81, 40], [41, 80, 40]], "the journal holds the mined half and its pair");
    j.hs.tick();
    // Read before the client's ack retires the row.
    let row: Vec<[i32; 3]> = j.sp().action_results.as_ref().unwrap().retained_cells().copied().collect();
    for c in &journaled { assert!(row.contains(c), "the server's row carries {c:?}"); }
    j.assert_honest("break");
}

/// Run-18 review L5: a `DoorPlace` from a dead body is superseded — both
/// predicted halves are put back and no door is taken from either copy.
#[test]
fn c3d_strict_door_place_from_a_dead_body_is_superseded_whole() {
    let mut j = joined("door-dead-place", &[(0, doors(1))], false);
    j.hs.server.players[j.i].combat.dead = true;
    j.place_door([41, 80, 40], 0, Facing::North);
    let action = j.rc.action_journal.as_ref().and_then(|journal| journal.actions().last()).expect("reserved").header;
    j.hs.tick();
    // Read before the client's ack retires the row.
    assert_eq!(j.sp().action_results.as_ref().unwrap().peek(action).map(|r| r.fate), Some(ActionFate::Superseded), "superseded");
    for _ in 0..4 { j.pump(); }
    assert!(!j.hs.disconnected[j.i], "never torn down");
    for y in [80, 81] {
        assert_eq!(j.server_cell([41, y, 40]).0, AIR, "no half on the server at y={y}");
        assert_eq!(j.client_cell([41, y, 40]).0, AIR, "the client's predicted half at y={y} put back");
    }
    assert_eq!(j.server_doors(), 1, "no door taken from the server's copy");
}

/// Run-18 review L6: a `DoorPlace` on a creative world is refused as every
/// creative typed edit is (`judge`'s `Unsupported`), not served: no half,
/// no door taken, never torn down.
#[test]
fn c3d_strict_door_place_in_creative_is_refused() {
    let mut j = joined("door-creative", &[(0, doors(2))], false);
    j.hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    j.place_door([41, 80, 40], 0, Facing::North);
    j.assert_honest("creative door");
    for y in [80, 81] { assert_eq!(j.server_cell([41, y, 40]).0, AIR, "no half at y={y}"); }
    assert_eq!((j.held(&Item::Block(OAK_DOOR)), j.server_doors()), (2, 2), "no door taken; the client's window corrected");
    assert_eq!(j.refusals(), (0, 1, 0), "one rule refusal");
}
