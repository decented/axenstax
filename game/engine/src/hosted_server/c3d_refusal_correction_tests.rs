//! C3d run 12 (S1) — the refusal-correction bridge, on a REAL `RemoteClient`
//! joined to a Strict `HostedServer` (the `c3d_client_edit_tests` and
//! `c3d_grid_tests` rigs). A refusal whose client prediction a `Replace`
//! plus the cell send-back fully undoes (a typed edit, every Drift
//! included; a personal click or a refused Autofill; a shot;
//! `OpenPlayer`/`OpenTable`) is corrected by ONE whole-window correction
//! `Replace`, not torn down. Every in-flight action after it has an older
//! base, so it is `Superseded` with zero effect and its cell sent back.
//! Container refusals, the contest and the `SetAutoRefill` refusal still tear
//! down (`c3d_container_tests` pins them).
use super::c3d_client_edit_tests::{joined, Joined, CELLS};
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::block::{AIR, COBBLESTONE, DIRT, OAK_LEAVES};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack};
use crate::remote_client::ConnectionState;
use crate::window::WindowClick;

fn fate(j: &Joined, action: ActionHeader) -> Option<ActionFate> {
    j.sp().action_results.as_ref().expect("a joined result history").peek(action).map(|r| r.fate)
}

fn counters(j: &Joined) -> (u32, u32) {
    let t = j.sp().strict_refusals;
    (t.stale_world, t.other)
}

/// Corrected, never torn down: the client's windows and touched cells
/// agree with the server's, the fold is proven zero (test 7), and the live
/// correction took no death-only flag.
fn assert_corrected(j: &mut Joined, what: &str) {
    j.assert_honest(what);
    assert!(j.sp().obligations_proven_zero(), "{what}: the fold is proven zero after the correction");
    assert!(j.sp().window_events.death_replace.is_none(), "{what}: a live correction never takes the death flag");
    for cell in CELLS {
        assert_eq!(
            (j.world.get_block(cell[0], cell[1], cell[2]), j.world.meta_at(cell[0], cell[1], cell[2])),
            (j.hs.server.world.get_block(cell[0], cell[1], cell[2]), j.hs.server.world.meta_at(cell[0], cell[1], cell[2])),
            "{what}: the cell {cell:?} agrees on both copies"
        );
    }
}

/// S1 test 1 — a placement Drift (the server already holds a block there):
/// `Refused`, ONE correction `Replace`, the cell and the windows agree, the
/// item is conserved, and the next placement is Accepted.
#[test]
fn c3d_s1_placement_drift_is_refused_and_corrected() {
    let cobble = Item::Block(COBBLESTONE);
    let cell = CELLS[0];
    let mut j = joined("s1-place", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    j.hs.server.world.set_block(cell[0], cell[1], cell[2], DIRT);
    let before = j.sp().window_events.last_number();
    j.place(cell, 0);
    let action = j.newest_header();
    j.hs.tick();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "a Drift refusal with its typed row");
    assert!(!j.hs.slot_is_free(j.i), "never torn down: the server keeps the seat");
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }), "never torn down");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace queued");
    assert!(j.sp().window_events.replace_waiting(), "the neutral flag: a correction Replace waits");
    assert!(!j.hs.joiner_requests_blocked(j.i), "a live correction blocks no request");
    assert_corrected(&mut j, "placement drift");
    assert_eq!(j.world.get_block(cell[0], cell[1], cell[2]), DIRT, "the client's cell is the server's");
    assert_eq!(j.held(&cobble), 4, "the item is conserved");
    assert_eq!(counters(&j), (1, 0), "one stale-world refusal");
    j.place(CELLS[1], 0);
    let next = j.newest_header();
    j.hs.tick();
    assert_eq!(fate(&j, next), Some(ActionFate::Accepted), "the next placement after the Replace");
    j.assert_honest("the next placement");
    assert_eq!(j.held(&cobble), 3);
}

/// S1 test 2 — a mined Drift (a leaf the server decayed): no grant, and the
/// tool wear the client predicted is undone by the Replace.
#[test]
fn c3d_s1_mined_drift_grants_nothing_and_undoes_the_wear() {
    let cell = CELLS[0];
    let tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
    let mut j = joined("s1-mine", &[(1, ItemStack { item: Item::Tool(tool), count: 1 })], true);
    j.world.set_block(cell[0], cell[1], cell[2], OAK_LEAVES);
    let server_window = j.sp().inventory.clone();
    let client_tool = j.inv.slot(1).cloned();
    j.harvest(cell, 1);
    assert_ne!(j.inv.slot(1).cloned(), client_tool, "the client predicted the wear");
    let action = j.newest_header();
    j.hs.tick();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused));
    assert_corrected(&mut j, "mined drift");
    assert_eq!(j.inv.slot(1).cloned(), client_tool, "the Replace undid the wear");
    assert_eq!(j.sp().inventory.slots_iter().collect::<Vec<_>>(), server_window.slots_iter().collect::<Vec<_>>(), "no grant, no wear on the server");
    assert_eq!(j.world.get_block(cell[0], cell[1], cell[2]), AIR);
    assert_eq!(counters(&j), (1, 0));
}

/// S1 tests 3 and 5 — a Drift with a suffix (three placements and a click
/// in one frame): the first is Refused, the rest are Superseded with their
/// cells sent back, ONE Replace corrects all four (the second Drift-able
/// action never becomes a second refusal: its base predates the Replace),
/// and the next action after the Replace is Accepted.
#[test]
fn c3d_s1_drift_with_a_suffix_supersedes_the_rest_with_one_replace() {
    let cobble = Item::Block(COBBLESTONE);
    let mut j = joined("s1-suffix", &[(0, ItemStack::new_block(COBBLESTONE, 8))], true);
    j.hs.server.world.set_block(CELLS[0][0], CELLS[0][1], CELLS[0][2], DIRT);
    // A second drifted cell: were it judged, it too would be a Drift.
    j.hs.server.world.set_block(CELLS[2][0], CELLS[2][1], CELLS[2][2], DIRT);
    let before = j.sp().window_events.last_number();
    let mut actions = Vec::new();
    for cell in CELLS {
        j.place(cell, 0);
        actions.push(j.newest_header());
    }
    let eye = j.sp().player.eye_pos();
    let ctx = crate::window::ClickCtx::new(false, j.ui.station(), eye, |_| AIR);
    j.ui.apply_joined_click(&mut j.rc, &mut j.inv, &mut j.armour, &WindowClick::ToggleLock { slot: 5 }, &ctx, &j.reg).expect("reserved and sent");
    actions.push(j.newest_header());
    j.hs.tick();
    let fates: Vec<_> = actions.iter().map(|a| fate(&j, *a)).collect();
    assert_eq!(fates, vec![Some(ActionFate::Refused), Some(ActionFate::Superseded), Some(ActionFate::Superseded), Some(ActionFate::Superseded)]);
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace for the whole frame");
    assert_corrected(&mut j, "drift with a suffix");
    assert_eq!(j.world.get_block(CELLS[1][0], CELLS[1][1], CELLS[1][2]), AIR, "a superseded placement's cell is sent back");
    assert_eq!(j.held(&cobble), 8, "every predicted placement is undone");
    assert_eq!(counters(&j), (1, 0), "superseded actions are no refusals");
    j.place(CELLS[1], 0);
    let next = j.newest_header();
    j.hs.tick();
    assert_eq!(fate(&j, next), Some(ActionFate::Accepted));
    j.assert_honest("after the Replace");
}

/// S1 test 4 (and 8) — a personal-click digest refusal converges, and it
/// counts as the other kind of refusal, beside a Drift.
#[test]
fn c3d_s1_personal_click_digest_refusal_converges_and_is_counted() {
    use super::c3d_grid_tests::{joined as grid_joined, stone};
    let mut j = grid_joined("s1-click", InventoryMode::Strict, None);
    j.give(0, stone(4));
    assert!(j.open(None), "E opens");
    j.pumps(3);
    j.assert_agree("open");
    // The server's copy changes under the client (never the client's).
    j.hs.server.players[j.i].inventory.set_slot(7, Some(stone(3)));
    j.click(WindowClick::Slot { slot: 0, right: false });
    let click = j.last_action();
    j.hs.tick();
    assert_eq!(j.fate(click), Some(ActionFate::Refused), "the digest refusal");
    let sp = &j.hs.server.players[j.i];
    assert_eq!((sp.strict_refusals.stale_world, sp.strict_refusals.other), (0, 1), "a rule/mismatch refusal, not a Drift");
    assert!(sp.window_events.death_replace.is_none() && sp.window_events.replace_waiting());
    j.pumps(3);
    j.assert_live("after the correction");
    j.assert_agree("after the correction");
    assert!(j.hs.server.players[j.i].obligations_proven_zero());
    let summary = j.hs.server.players[j.i].strict_refusals.summary().expect("a counted refusal");
    assert!(summary.contains("0 stale-world") && summary.contains("1 other"), "{summary}");
}

/// S1 test 6 — Mirror never sees any of this. A Mirror client's clicks go
/// out as its op log, never as typed actions; the same typed click on a
/// Mirror seat never reaches a Strict handler (`process_typed_actions`
/// gates each on `strict_slot`), so it is the `Untyped` pause exactly as on
/// `0e2ab5d4`: no typed row, the `Untyped` reason, no correction `Replace`,
/// no Strict refusal counted, no flag. (Run 12's version asserted "never
/// torn down" with the rig's Strict-only `click`: a wrong premise. The
/// non-Strict branch of `refuse_typed_action_at` is the old line verbatim.)
#[test]
fn c3d_s1_mirror_click_mismatch_is_unchanged() {
    use super::c3d_grid_tests::{joined as grid_joined, stone};
    let mut j = grid_joined("s1-mirror", InventoryMode::Mirror, None);
    j.give(0, stone(4));
    assert!(j.open(None), "E opens");
    j.pumps(3);
    j.hs.server.players[j.i].inventory.set_slot(7, Some(stone(3)));
    let before = j.hs.server.players[j.i].window_events.last_number();
    j.click(WindowClick::Slot { slot: 0, right: false });
    let click = j.last_action();
    j.hs.tick();
    assert_eq!(j.fate(click), None, "Mirror: no typed row, as today");
    assert!(j.hs.slot_is_free(j.i), "Mirror: torn down, as today");
    j.pumps(1);
    let reason = match &j.rc.state { ConnectionState::Failed(m) => m.as_str(), _ => "not Failed" };
    assert_eq!(reason, crate::strict_admission::StrictTeardown::Untyped.reason(), "Mirror: the Untyped reason, as today");
    let sp = &j.hs.server.players[j.i];
    assert_eq!(sp.window_events.last_number(), before, "no correction Replace on Mirror");
    assert_eq!((sp.strict_refusals.stale_world, sp.strict_refusals.other), (0, 0), "no Strict refusal counted");
    assert!(sp.window_events.death_replace.is_none() && !sp.window_events.replace_waiting());
}

/// The live-correction flag test: a live correction leaves the death flag
/// alone, a dead body's correction still sets it, and a second correction
/// while one waits queues nothing.
#[test]
fn c3d_s1_live_correction_never_takes_the_death_flag() {
    let mut j = joined("s1-flag", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    let before = j.sp().window_events.last_number();
    j.hs.queue_live_correction_replace(j.i);
    j.hs.queue_live_correction_replace(j.i);
    let sp = j.sp();
    assert_eq!(sp.window_events.last_number(), before + 1, "one Replace while one waits");
    assert!(sp.window_events.death_replace.is_none(), "the death flag keeps its meaning");
    assert!(sp.window_events.replace_waiting());
    assert!(!j.hs.joiner_requests_blocked(j.i));
    j.assert_honest("the live correction converges");
    let mut d = joined("s1-flag-dead", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    d.hs.queue_correction_replace(d.i);
    assert!(d.sp().window_events.death_replace.is_some(), "a dead body's correction still sets the death flag");
}
