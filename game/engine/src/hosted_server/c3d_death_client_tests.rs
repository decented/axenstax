//! C3d run 10 (run-8 review LOW-C; run-7 item 3, run-5 review M1's test ask)
//! — deaths with a window in play, on a REAL `RemoteClient` (the
//! `c3d_grid_tests` rig): the client's window is built only from what the
//! server sends and changed only through the real `CraftingUi` entry points
//! (`open_joined`, `close_joined`, `apply_joined_click`). Nothing forces the
//! screen open, and E after the respawn is the real open (a grid holding
//! items is returned by a `Close` first). The rig has no `GameState`, so the
//! one frozen-window gate (`strict_window_frozen`, proven through the real
//! game loop by `game_harness_strict_frozen_crafting_esc_pauses_and_e_returns_the_grid`)
//! is not in front of these actions: each case shows the server converges
//! a dead body's action without it.
use super::c3d_grid_tests::{joined, stone, Joined, TABLE};
use crate::action_identity::{ActionFate, InventoryMode};
use crate::window::{Station, WindowClick};

impl Joined {
    /// After the respawn: E through the real open, then a bag click; both
    /// Accepted, never torn down, and the two windows agree.
    fn converge(&mut self, what: &str) {
        self.respawn();
        assert!(self.open(None), "{what}: E opens");
        let e = self.last_action();
        self.click(WindowClick::Slot { slot: 20, right: false });
        let put = self.last_action();
        self.hs.tick();
        assert_eq!(self.fate(e), Some(ActionFate::Accepted), "{what}: E after the respawn");
        assert_eq!(self.fate(put), Some(ActionFate::Accepted), "{what}: the next click");
        self.pumps(3);
        self.assert_agree(what);
    }

    /// A Strict `Close` through the real entry point; its header.
    fn close_now(&mut self, what: &str) -> crate::action_identity::ActionHeader {
        assert!(self.ui.close_joined(&mut self.rc, &mut self.inv, &mut self.armour, &self.reg).expect("reserved"), "{what}: the Close is predicted");
        self.last_action()
    }
}

/// Run-5 review M1 sequence A, ported from the `Client` model (run-8 review
/// LOW-C) — a crafting table with a cursor stack: the body dies, the client
/// applies the death sync, then Esc on the death screen predicts a `Close`
/// on a CURRENT base. It is `Superseded` and ONE correction `Replace`
/// (cursor, grid, station, locks) follows; a second action on the old base
/// is superseded without another. The client converges after the respawn.
#[test]
fn c3d_strict_dead_body_close_at_a_table_gets_one_correction_replace() {
    let mut j = joined("dead-close", InventoryMode::Strict, None);
    j.give(0, stone(8));
    assert!(j.open(Some(TABLE)), "the table opens");
    j.click(WindowClick::Slot { slot: 0, right: false });
    j.pumps(2);
    j.kill();
    j.pumps(2);
    assert_eq!(j.syncs, 1, "the death sync applied on the client");
    assert_eq!(j.ui.station(), Station::Table { cell: TABLE }, "the death sync carries the table station");
    let close = j.close_now("Esc on the death screen");
    j.hs.tick();
    assert_eq!(j.fate(close), Some(ActionFate::Superseded));
    assert!(j.hs.server.players[j.i].window_events.replace_waiting(), "a correction Replace follows the dead body's Close");
    let events = j.hs.server.players[j.i].window_events.last_number();
    j.click(WindowClick::ToggleLock { slot: 5 });
    let lock = j.last_action();
    j.hs.tick();
    assert_eq!(j.fate(lock), Some(ActionFate::Superseded));
    assert_eq!(j.hs.server.players[j.i].window_events.last_number(), events, "at most one correction outstanding");
    j.pumps(2);
    assert_eq!(j.syncs, 2, "exactly one correction reached the client");
    assert!(!j.inv.is_locked(5), "and the server's lock mask");
    j.converge("dead-body Close");
}

/// Run-5 review LOW-1, ported (LOW-C) — a `Close` (grid holding stone) and a
/// `ToggleLock` already in flight when the killing hit lands: both predate
/// the death sync, which corrects them (it carries the grid, the station
/// and the lock mask); no further `Replace`.
#[test]
fn c3d_strict_close_and_toggle_lock_in_flight_at_death_converge() {
    let mut j = joined("in-flight", InventoryMode::Strict, None);
    j.give(0, stone(8));
    assert!(j.open(Some(TABLE)), "the table opens");
    j.click(WindowClick::Slot { slot: 0, right: false });
    j.click(WindowClick::Grid { row: 0, col: 0, right: false });
    j.pumps(2);
    j.kill();
    j.hs.tick();
    // Sent before the client heard of its death: on the old base.
    let close = j.close_now("in flight");
    j.click(WindowClick::ToggleLock { slot: 7 });
    let lock = j.last_action();
    j.hs.tick();
    assert_eq!(j.fate(close), Some(ActionFate::Superseded));
    assert_eq!(j.fate(lock), Some(ActionFate::Superseded));
    j.pumps(2);
    assert_eq!(j.syncs, 1, "the death sync is the only correction");
    assert!(!j.inv.is_locked(7), "the lock mask is the server's");
    assert_eq!(j.ui.station(), Station::Table { cell: TABLE });
    assert_eq!(j.ui.grid[0][0].as_ref().map(|s| s.count), Some(8), "the grid is back behind a closed screen");
    j.converge("in flight at death");
}

impl Joined {
    /// A Strict joiner at the crafting table carrying 8 stone on its cursor.
    fn at_table_with_a_cursor_stack(tag: &str) -> Joined {
        let mut j = joined(tag, InventoryMode::Strict, None);
        j.give(0, stone(8));
        assert!(j.open(Some(TABLE)), "the table opens");
        j.click(WindowClick::Slot { slot: 0, right: false });
        j.pumps(2);
        assert_eq!(j.ui.cursor_item.as_ref().map(|s| s.count), Some(8), "the cursor carries the stack");
        j
    }
}

/// C3d run 10 (run-7 item 3, case 1, the frozen gate's side) — a crafting
/// table with a cursor stack: the body dies and the client hears it; Esc on
/// the death screen sends nothing (the gate; its `Close` going out is the
/// ported `c3d_strict_dead_body_close_at_a_table_gets_one_correction_replace`).
/// No correction is queued; after the respawn, E and a click are Accepted,
/// never torn down, and the windows agree.
#[test]
fn c3d_strict_client_death_with_a_cursor_stack_at_a_table_then_e_and_a_click() {
    let mut j = Joined::at_table_with_a_cursor_stack("death-cursor");
    j.kill();
    j.pumps(2);
    assert_eq!(j.syncs, 1, "the death sync applied on the client");
    assert_eq!(j.ui.station(), Station::Table { cell: TABLE }, "at the table it died at");
    let events = j.hs.server.players[j.i].window_events.last_number();
    j.pumps(2);
    assert_eq!(j.hs.server.players[j.i].window_events.last_number(), events, "Esc sent nothing: no correction");
    j.converge("a cursor stack at a table");
}

/// C3d run 10 (run-7 item 3, case 2; run-5 review M1 sequence B) — a table
/// screen and a Respawn inside 20 ticks: the client asks before the server
/// will (`MIN_DEAD_TICKS_BEFORE_RESPAWN`), and its table force-close goes out
/// on a current base while the body is still dead (the rig has no gate). It
/// is `Superseded`, one correction puts the table and the cursor stack
/// back, and once the resent Respawn lands, E and a click converge.
#[test]
fn c3d_strict_client_early_respawn_with_a_table_screen_converges() {
    let mut j = Joined::at_table_with_a_cursor_stack("early-respawn");
    j.kill();
    j.pumps(2);
    j.rc.send_respawn();
    j.pumps(1);
    assert!(j.rc.respawn_unanswered() && !j.hs.server.players[j.i].is_present_and_alive(), "the early Respawn waits");
    assert!(j.ui.force_close_joined(&mut j.rc, &mut j.inv, &mut j.armour, &j.reg, true).expect("reserved"), "the force-close is predicted");
    let close = j.last_action();
    j.hs.tick();
    assert_eq!(j.fate(close), Some(ActionFate::Superseded), "a dead body's Close");
    j.pumps(2);
    assert_eq!(j.syncs, 2, "one correction after the death sync");
    assert_eq!(j.ui.station(), Station::Table { cell: TABLE }, "the correction put the table back");
    assert_eq!(j.ui.cursor_item.as_ref().map(|s| s.count), Some(8), "and the cursor stack");
    assert!(matches!(j.rc.state, crate::remote_client::ConnectionState::Connected { .. }), "never torn down while the Respawn waits");
    j.converge("Respawn inside 20 ticks");
}

/// C3d run 10 (run-7 item 3, case 4) — a `ToggleLock` alone in flight when
/// the killing hit lands, the cursor carrying a stack at a table: it
/// predates the death sync, is `Superseded`, and the death sync's lock mask
/// undoes the client's prediction. After the respawn, E and a click
/// converge.
#[test]
fn c3d_strict_client_toggle_lock_in_flight_at_death_converges() {
    let mut j = Joined::at_table_with_a_cursor_stack("lock-in-flight");
    j.kill();
    j.hs.tick();
    j.click(WindowClick::ToggleLock { slot: 4 });
    assert!(j.inv.is_locked(4), "predicted");
    let lock = j.last_action();
    j.hs.tick();
    assert_eq!(j.fate(lock), Some(ActionFate::Superseded));
    j.pumps(2);
    assert_eq!(j.syncs, 1, "the death sync is its correction");
    assert!(!j.inv.is_locked(4), "the server's lock mask");
    j.converge("ToggleLock in flight at death");
}
