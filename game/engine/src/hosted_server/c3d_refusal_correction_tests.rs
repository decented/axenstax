//! C3d run 12 (S1) — the refusal-correction bridge, on a REAL `RemoteClient`
//! joined to a Strict `HostedServer` (the `c3d_client_edit_tests` and
//! `c3d_grid_tests` rigs). A refusal whose client prediction a `Replace`
//! plus the cell send-back fully undoes (a typed edit, every Drift
//! included; a personal click or a refused Autofill; a shot;
//! `OpenPlayer`/`OpenTable`) is corrected by ONE whole-window correction
//! `Replace`, not torn down. Every in-flight action after it has an older
//! base, so it is `Superseded` with zero effect and its cell sent back.
//! Container refusals, the contest and the `SetAutoRefill` refusal still tear
//! down (`c3d_container_tests` pins the container ones; the refill setting's
//! is pinned here).
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
    // Run 13 (run-12 review LOW-4) — the click after the corrected one is
    // served: a click suffix converges, not only the digests.
    j.click(WindowClick::Slot { slot: 0, right: false });
    let next = j.last_action();
    j.pumps(3);
    assert_eq!(j.fate(next), Some(ActionFate::Accepted), "the next click after the Replace");
    j.assert_agree("the next click");
    assert!(j.ui.cursor_item.as_ref().is_some_and(|s| s.count == 4), "the bag's stone is on the cursor");
}

/// S1 test 6 (run 13, run-12 review LOW-4) — Mirror never sees any of
/// this, through Mirror's OWN path: a Mirror client's clicks go out as its
/// window-op log, never as typed actions. A click whose digest the server's
/// copy doesn't reproduce is tallied (`window_mismatch`, log-only), never
/// refused: no typed row, no correction `Replace`, no Strict refusal
/// counted, the seat kept. (The first op after the join is the baseline,
/// so a matching op goes first.)
#[test]
fn c3d_s1_mirror_op_log_mismatch_is_tallied_never_refused() {
    use super::c3d_grid_tests::{joined as grid_joined, stone};
    let mut j = grid_joined("s1-mirror", InventoryMode::Mirror, None);
    j.give(0, stone(4));
    let eye = j.hs.server.players[j.i].player.eye_pos();
    j.ui.apply_click(&mut j.inv, &mut j.armour, &WindowClick::ToggleLock { slot: 3 }, false, eye, |_| AIR);
    j.pumps(3);
    assert_eq!(j.hs.server.players[j.i].possession.window_mismatch, 0, "the matching first op is the baseline");
    j.hs.server.players[j.i].inventory.set_slot(7, Some(stone(3)));
    let before = j.hs.server.players[j.i].window_events.last_number();
    j.ui.apply_click(&mut j.inv, &mut j.armour, &WindowClick::Slot { slot: 0, right: false }, false, eye, |_| AIR);
    assert!(j.rc.action_journal.as_ref().is_none_or(|journal| journal.len() == 0), "Mirror reserves no typed action");
    j.pumps(3);
    j.assert_live("Mirror: the seat is kept");
    let sp = &j.hs.server.players[j.i];
    assert_eq!(sp.possession.window_mismatch, 1, "Mirror: the mismatch is tallied");
    assert_eq!(sp.window_events.last_number(), before, "no correction Replace on Mirror");
    assert_eq!((sp.strict_refusals.stale_world, sp.strict_refusals.other), (0, 0), "no Strict refusal counted");
    assert!(sp.window_events.death_replace.is_none() && !sp.window_events.replace_waiting());
    assert!(sp.cursor.as_ref().is_some_and(|c| c.count == 4), "Mirror served the click on its copy");
}

/// Run 13 (run-12 review LOW-5) — a typed `Unsupported` edit (here every
/// edit while the server is Creative) is a rule refusal, corrected; only a
/// real Drift counts as stale-world.
#[test]
fn c3d_s1_unsupported_edit_counts_as_a_rule_refusal() {
    let mut j = joined("s1-unsupported", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    j.hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    j.place(CELLS[0], 0);
    let action = j.newest_header();
    j.hs.tick();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "Unsupported is a refusal");
    assert_eq!(counters(&j), (0, 1), "a rule refusal, not stale-world");
    j.hs.server.play_mode = crate::play_mode::PlayMode::Survival;
    assert_corrected(&mut j, "unsupported");
    assert_eq!(j.held(&Item::Block(COBBLESTONE)), 4, "the placement was undone");
}

/// Run 13 (run-12 review LOW-2) — the server's OWN window holds a stack the
/// marker projection can't carry (a tool over its maximum durability): a
/// `Replace` would ship it back and every action would refuse again, so
/// the action is refused, then torn down with its own reason. No
/// correction `Replace`, nothing placed.
#[test]
fn c3d_s1_raw_unrepresentable_window_is_refused_then_torn_down() {
    let mut j = joined("s1-raw-unrep", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    let mut tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
    tool.durability = tool.max_durability() + 5;
    j.hs.server.players[j.i].inventory.set_slot(5, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
    let before = j.sp().window_events.last_number();
    j.place(CELLS[0], 0);
    // The placement goes after the session's refill setting (`place_joined`
    // sends it first): that configuration action is the one refused.
    let first = j.rc.action_journal.as_ref().and_then(|journal| journal.actions().next()).expect("a reserved action").header;
    j.hs.tick();
    assert_eq!(fate(&j, first), Some(ActionFate::Refused), "the typed refusal stands");
    assert!(j.hs.slot_is_free(j.i), "torn down");
    j.rc.poll();
    let reason = match &j.rc.state { ConnectionState::Failed(m) => m.as_str(), _ => "not Failed" };
    assert_eq!(reason, crate::strict_admission::StrictTeardown::Unrepresentable.reason(), "its own reason");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction Replace");
    assert_eq!(j.hs.server.world.get_block(CELLS[0][0], CELLS[0][1], CELLS[0][2]), AIR, "nothing placed");
    assert_eq!(j.sp().inventory.slot(0).map(|s| s.count), Some(4), "nothing taken");
    assert_eq!(counters(&j), (0, 1));
}

/// Run 13 (run-12 review LOW-3) — a client provoking a refusal every tick
/// (a personal click whose digest the server's copy, changed under it each
/// tick, never reproduces) is corrected eight times; the ninth correction
/// inside 100 ticks is the `FellBehind` teardown. The refused clicks moved
/// nothing.
#[test]
fn c3d_s1_a_refusal_every_tick_is_torn_down_at_the_ninth_correction() {
    use super::c3d_grid_tests::{joined as grid_joined, stone};
    let mut j = grid_joined("s1-rate", InventoryMode::Strict, None);
    j.give(0, stone(4));
    assert!(j.open(None), "E opens");
    j.pumps(3);
    j.assert_agree("open");
    let start = j.hs.server.tick_counter;
    for round in 1..=9u8 {
        j.hs.server.players[j.i].inventory.set_slot(7, Some(stone(round)));
        j.click(WindowClick::Slot { slot: 0, right: false });
        let click = j.last_action();
        j.pumps(1);
        assert_eq!(j.fate(click), Some(ActionFate::Refused), "round {round}: refused");
        if round < 9 {
            j.assert_live(&format!("round {round}: corrected, not torn down"));
            j.assert_agree(&format!("round {round}: corrected"));
        }
    }
    assert!(j.hs.server.tick_counter - start < crate::strict_admission::CORRECTION_WINDOW_TICKS, "all nine inside one window");
    assert!(j.hs.slot_is_free(j.i), "the ninth correction tears down");
    let reason = match &j.rc.state { ConnectionState::Failed(m) => m.as_str(), _ => "not Failed" };
    assert_eq!(reason, crate::strict_admission::StrictTeardown::FellBehind.reason(), "the FellBehind reason");
    assert_eq!(super::actions::teardown_detail::last(), super::actions::CORRECTION_RATE_DETAIL, "the internal reason");
    let sp = &j.hs.server.players[j.i];
    assert_eq!(sp.inventory.slot(0).cloned(), Some(stone(4)), "the refused clicks took nothing");
    assert!(sp.cursor.is_none(), "nothing on the server's cursor");
    assert_eq!(sp.strict_refusals.other, 9);
}

/// Run 14 item 1 (revises run 13's teardown pin) — a Strict refill setting
/// whose digest the server's copy doesn't reproduce is refused with zero
/// effect and CORRECTED, never torn down: one correction `Replace` puts the
/// window back; the `Replace` keeps each copy's refill setting, so the
/// client, seeing its setting's `Refused` receipt, sends the setting again
/// on the corrected window, where it is served. Both copies then agree,
/// refill setting included.
#[test]
fn c3d_s1_refused_auto_refill_setting_is_corrected_then_resent() {
    let mut j = joined("s1-refill", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the session's setting");
    j.hs.server.players[j.i].inventory.set_slot(7, Some(ItemStack::new_block(COBBLESTONE, 3)));
    let server_refill = j.sp().inventory.auto_refill;
    j.inv.auto_refill = !j.inv.auto_refill;
    let before = j.sp().window_events.last_number();
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    let action = j.newest_header();
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "the typed refusal stands");
    assert!(!j.hs.slot_is_free(j.i) && matches!(j.rc.state, ConnectionState::Connected { .. }), "corrected, never torn down");
    assert_eq!(j.sp().inventory.auto_refill, server_refill, "zero effect: the server's setting unchanged");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace");
    assert_eq!(counters(&j), (0, 1));
    j.pump();
    assert!(j.rc.refill_refused, "the Refused receipt retired: the setting goes again");
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("resent");
    let resent = j.newest_header();
    assert_ne!(resent, action, "a new setting action");
    j.pump();
    assert_eq!(fate(&j, resent), Some(ActionFate::Accepted), "served on the corrected window");
    assert_eq!(j.sp().inventory.auto_refill, j.inv.auto_refill, "the copies' refill settings agree");
    j.assert_honest("the corrected setting");
    assert_eq!(j.held(&Item::Block(COBBLESTONE)), 7, "the client's window was corrected to the server's");
}

/// Run 15 item 1 — the resend can't loop. A server copy that diverges again
/// before every resend refuses every resend; each refusal is corrected
/// (one `Replace`), the client resends on its retired `Refused` receipt,
/// and the ninth correction inside 100 ticks is the rate bound's teardown,
/// with its own internal reason. Every refused setting changed nothing.
/// (The honest case, a copy that diverged once, is served on the FIRST
/// resend: `c3d_s1_refused_auto_refill_setting_is_corrected_then_resent`.)
#[test]
fn c3d_s1_a_refill_resend_refused_again_ends_at_the_rate_bound() {
    let mut j = joined("s1-refill-loop", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the session's setting");
    let server_refill = j.sp().inventory.auto_refill;
    j.inv.auto_refill = !j.inv.auto_refill;
    let start = j.hs.server.tick_counter;
    let mut sent = 0;
    for round in 1..=9u8 {
        // The server's copy diverges again under the client, past the
        // correction the client applied (else that `Replace` overwrites it).
        let applied = j.rc.applied_window_boundary();
        crate::window_events::apply_through(&mut j.hs.server.players[j.i], applied, 0);
        j.hs.server.players[j.i].inventory.set_slot(7, Some(ItemStack::new_block(COBBLESTONE, round)));
        if round > 1 { assert!(j.rc.refill_refused, "round {round}: the Refused receipt retired, so the setting goes again"); }
        let before = j.sp().window_events.last_number();
        j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
        let action = j.newest_header();
        sent += 1;
        j.pump();
        assert_eq!(fate(&j, action), Some(ActionFate::Refused), "round {round}: refused");
        assert_eq!(j.sp().inventory.auto_refill, server_refill, "round {round}: zero effect on the setting");
        if round < 9 {
            assert!(!j.hs.slot_is_free(j.i), "round {round}: corrected, not torn down");
            assert_eq!(j.sp().window_events.last_number(), before + 1, "round {round}: one correction Replace");
            j.pump();
        }
    }
    assert!(j.hs.server.tick_counter - start < crate::strict_admission::CORRECTION_WINDOW_TICKS, "all nine inside one window");
    assert_eq!(sent, 9, "one resend per correction, never more");
    assert!(j.hs.slot_is_free(j.i), "the ninth correction tears down");
    assert_eq!(super::actions::teardown_detail::last(), super::actions::CORRECTION_RATE_DETAIL, "the rate bound's internal reason");
    let sp = &j.hs.server.players[j.i];
    assert_eq!(sp.inventory.slot(0).cloned(), Some(ItemStack::new_block(COBBLESTONE, 4)), "zero item effect");
    assert_eq!(sp.inventory.auto_refill, server_refill, "the server's setting never changed");
    assert_eq!(counters(&j), (0, 9));
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

/// C3d run 14 (run-13 review LOW-2) — `RawUnrepresentable` through a
/// non-configuration arm: the refill setting goes first on agreeing
/// windows, then the server's copy gets a tool over its maximum
/// durability, then a typed placement. Refused, then torn down with the
/// `Unrepresentable` sentence; nothing placed, nothing taken, no Replace.
#[test]
fn c3d_s1_raw_unrepresentable_placement_is_refused_then_torn_down() {
    let mut j = joined("s1-raw-place", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the refill setting");
    let mut tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
    tool.durability = tool.max_durability() + 5;
    j.hs.server.players[j.i].inventory.set_slot(5, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
    let before = j.sp().window_events.last_number();
    j.place(CELLS[0], 0);
    let action = j.newest_header();
    j.hs.tick();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "the placement's refusal stands");
    assert!(j.hs.slot_is_free(j.i), "torn down");
    j.rc.poll();
    let reason = match &j.rc.state { ConnectionState::Failed(m) => m.as_str(), _ => "not Failed" };
    assert_eq!(reason, crate::strict_admission::StrictTeardown::Unrepresentable.reason(), "its own reason");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction Replace");
    assert_eq!(j.hs.server.world.get_block(CELLS[0][0], CELLS[0][1], CELLS[0][2]), AIR, "nothing placed");
    assert_eq!(j.sp().inventory.slot(0).map(|s| s.count), Some(4), "nothing taken");
}

/// C3d run 14 (run-13 review LOW-3) — a dead body's correction `Replace`
/// shares the seat's one rate bound: eight inside 100 ticks are queued
/// (each after the last was applied), the ninth tears down with the
/// rate-bound reason.
#[test]
fn c3d_s1_dead_body_corrections_share_the_rate_bound() {
    let mut j = joined("s1-dead-rate", &[(0, ItemStack::new_block(COBBLESTONE, 4))], true);
    for round in 1..=9u32 {
        let n = j.sp().window_events.last_number();
        crate::window_events::apply_through(&mut j.hs.server.players[j.i], n, 0);
        j.hs.queue_correction_replace(j.i);
        if round < 9 {
            assert!(!j.hs.slot_is_free(j.i), "round {round}: corrected, not torn down");
            assert_eq!(j.sp().window_events.last_number(), n + 1, "round {round}: one Replace");
        }
    }
    assert!(j.hs.slot_is_free(j.i), "the ninth correction tears down");
    assert_eq!(super::actions::teardown_detail::last(), super::actions::CORRECTION_RATE_DETAIL, "the rate-bound reason");
}
