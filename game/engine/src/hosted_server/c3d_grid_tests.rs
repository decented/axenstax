//! C3d run 8 (run-6 review H1, LOW-2) — the client never discards
//! crafting-grid items locally, and a whole-window `Replace` leaves both
//! copies at the station it carries.
//!
//! The client is a REAL `RemoteClient` joined over the in-process transport
//! (a guest, or a signed rejoin from a saved record). Its window is its own:
//! built only from what the server sends (`inventory_sync::apply` in the
//! session's mode, `adopt_station`, grants, in event order) and changed only
//! through the real `CraftingUi` entry points (`open_joined`, `close_joined`,
//! `apply_joined_click`; Mirror's `open_crafting`, its logged ops flushed as
//! the game loop flushes them). Nothing forces the screen open.
use super::*;
use crate::action_identity::InventoryMode;
use crate::block::{AIR, CRAFTING_TABLE, STONE};
use crate::item::ItemStack;
use crate::remote_client::{ConnectionState, RemoteClient, RequestOutcome};
use crate::transport::ClientTransport as _;
use crate::window::{Station, WindowClick};
use crate::window_events::InboxItem;

pub(super) const TABLE: [i32; 3] = [41, 80, 40];
const SECKEY: [u8; 32] = [71; 32];

pub(super) struct Joined {
    _g: crate::save::WorldsRootGuard,
    pub(super) hs: HostedServer,
    pub(super) rc: RemoteClient,
    pub(super) i: usize,
    pub(super) inv: crate::inventory::Inventory,
    pub(super) armour: [Option<crate::armour::ArmourItem>; 4],
    pub(super) ui: crate::craft_ui::CraftingUi,
    inbox: crate::window_events::WindowInbox,
    pub(super) reg: crate::block::BlockRegistry,
    /// Whole-window syncs the client applied.
    pub(super) syncs: usize,
}

/// A saved record to rejoin from: its grid cells (row, col, stack), and the
/// stack every bag slot holds (`None`: an empty bag).
type Record<'a> = (&'a [(usize, usize, ItemStack)], Option<ItemStack>);

pub(super) fn stone(n: u8) -> ItemStack { ItemStack::new_block(STONE, n) }

fn units(inv: &crate::inventory::Inventory, item: &crate::item::Item) -> u32 {
    (0..36).filter_map(|at| inv.slot(at)).filter(|s| s.item == *item).map(|s| u32::from(s.count)).sum()
}

/// A server in `mode` on a stone floor with a crafting table beside the
/// joiner's spot (40.5, 80, 40.5). `saved_grid`: a signed rejoin whose saved
/// record holds those grid cells (row, col, stack); otherwise a guest.
pub(super) fn joined(tag: &str, mode: InventoryMode, saved_grid: Option<&[(usize, usize, ItemStack)]>) -> Joined {
    joined_from(tag, mode, saved_grid.map(|grid| (grid, None)))
}

/// [`joined`] from a full [`Record`], the join sync applied.
fn joined_from(tag: &str, mode: InventoryMode, record: Option<Record>) -> Joined {
    let saved = record.is_some();
    let mut j = start(tag, mode, record);
    for _ in 0..20 {
        j.pump();
        if j.rc.player_index().is_some() && (!saved || j.rc.applied_window_boundary() >= 1) { break; }
    }
    j.pumps(2);
    // `syncs` counts what follows the join: every join's own Sync1 is applied.
    j.syncs = 0;
    j.place();
    j
}

/// The server and the client before the join is heard (nothing polled).
fn start(tag: &str, mode: InventoryMode, record: Option<Record>) -> Joined {
    let name = format!("c3d-grid-{tag}");
    let g = crate::save::WorldsRootGuard::new(&name);
    let mut hs = super::tests::start_room_test_server(&name, 0);
    hs.select_inventory_mode(mode).expect("a dedicated server selects the mode");
    hs.server.column_streamer = None;
    hs.server.column_refill_per_tick = 0;
    hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    for x in 32..=48 {
        for z in 32..=48 {
            hs.server.world.set_block(x, 79, z, STONE);
            for y in 80..=84 { hs.server.world.set_block(x, y, z, AIR); }
        }
    }
    hs.server.world.set_block(TABLE[0], TABLE[1], TABLE[2], CRAFTING_TABLE);
    let cs = crate::chunk::CHUNK_SIZE as i32;
    hs.server.loaded_columns.insert((40i32.div_euclid(cs), 40i32.div_euclid(cs)));
    let rc = match record {
        None => RemoteClient::from_transport(Box::new(hs.attach_test_remote()), crate::remote_client::build_join_request_guest("Crafter", 0), None),
        Some((cells, bag)) => {
            // A saved record: a crash, a drop or a refused Close left items
            // in the grid (the save keeps the grid).
            let keys = nostr::Keys::new(nostr::SecretKey::from_slice(&SECKEY).unwrap());
            let mut saved = crate::server::ServerPlayer::new(hs.server.world_spawn());
            saved.server_simulated = true;
            saved.verified_pubkey = Some(keys.public_key().to_bytes());
            saved.enter_world();
            for (row, col, stack) in cells { saved.craft_grid[*row][*col] = Some(stack.clone()); }
            if let Some(bag) = bag { for at in 0..36 { saved.inventory.set_slot(at, Some(bag.clone())); } }
            hs.server.players.push(saved);
            hs.server.save_joiner(hs.server.players.len() - 1).expect("the record is saved");
            hs.server.players.pop();
            let binding = Some([19; 32]);
            let transport = hs.attach_test_remote_with_binding(binding);
            let pkt = transport.try_recv_from_server().expect("issued challenge");
            let (kind, bytes) = protocol::deserialize_header(&pkt).unwrap();
            assert_eq!(kind, protocol::PacketType::Challenge);
            let challenge: protocol::ChallengePacket = protocol::safe_deserialize(bytes).unwrap();
            let mut request = crate::remote_client::build_join_request_guest("Saved", 0);
            request.auth_event = Some(pollster::block_on(crate::signet::native_signer::build_join_auth_event(
                &keys, &challenge.nonce_hex, &crate::signet::join_origin(binding))).unwrap());
            RemoteClient::from_transport(Box::new(transport), request, None)
        }
    };
    Joined {
        _g: g, hs, rc, i: 0, inv: crate::inventory::Inventory::new(), armour: [None; 4],
        ui: crate::craft_ui::CraftingUi::new(), inbox: Default::default(), reg: crate::block::BlockRegistry::new(), syncs: 0,
    }
}

impl Joined {
    /// The joiner's slot, its body on its spot beside the table.
    fn place(&mut self) {
        self.i = self.rc.player_index().expect("joined") as usize;
        let p = &mut self.hs.server.players[self.i].player;
        p.pos = glam::Vec3::new(40.5, 80.0, 40.5);
        p.velocity = glam::Vec3::ZERO;
        p.on_ground = true;
    }

    /// One server tick, then the client reads and applies what it was sent,
    /// in event order (the game loop's `apply_window_inbox`), then flushes
    /// its window ops.
    fn pump(&mut self) {
        self.receive();
        self.flush();
    }

    /// One server tick, then the client reads and applies what it was sent
    /// (the game loop's `network_receive`), sending nothing.
    fn receive(&mut self) {
        self.hs.tick();
        self.rc.poll();
        let outcomes = std::mem::take(&mut self.rc.pending_outcomes);
        let grants = std::mem::take(&mut self.rc.pending_grants);
        self.inbox.add(outcomes, grants, Vec::new());
        let (items, _) = self.inbox.take_ordered();
        for item in items {
            let event = item.event();
            match item {
                InboxItem::Outcome(RequestOutcome::Sync(pkt)) => {
                    let mut w = crate::window::WindowMut { inv: &mut self.inv, armour: &mut self.armour, cursor: &mut self.ui.cursor_item, grid: &mut self.ui.grid, container: None };
                    let station = crate::inventory_sync::apply(&mut w, &pkt, &self.reg, &mut crate::use_edits::SentUses::default(), &mut self.inbox.debt, self.rc.inventory_mode).expect("a readable sync");
                    if let Some(station) = station { self.ui.adopt_station(station); }
                    self.rc.note_initial_personal_window_applied(event, pkt.locks);
                    self.syncs += 1;
                }
                InboxItem::Grant(grant) => { crate::remote_entities::apply_inventory_grant(&mut self.inv, &grant, &self.reg); }
                _ => {}
            }
            self.rc.note_window_event(event);
        }
        // The game loop's `apply_personal_inbox`, past the applied events:
        // the personal state may become ready here, and a Mirror window's
        // settings are logged at once.
        super::tests::client_apply_personal(&mut self.rc);
        self.ui.settle_initial_settings(&mut self.rc, &self.inv, &self.armour);
    }

    /// Mirror: the op log goes out as the game loop sends it with no edit
    /// waiting (`flush_ops_before_edits`, then `flush_window_ops`).
    fn flush(&mut self) {
        if self.rc.inventory_mode == InventoryMode::Mirror {
            for logged in self.ui.ops.take_before(None) { self.rc.send_window_op(logged); }
            for logged in self.ui.take_ops(&self.inv, &self.armour) { self.rc.send_window_op(logged); }
        }
    }

    pub(super) fn pumps(&mut self, n: usize) { for _ in 0..n { self.pump(); } }

    /// C3d run 16 — retire the action receipts read so far (this rig's pump
    /// leaves the world intake, receipts included, unread).
    pub(super) fn retire_receipts(&mut self) {
        self.rc.pending_block_changes.clear();
        for (_, item) in std::mem::take(&mut self.rc.chunk_queue) {
            if let crate::chunk_intake::StreamItem::ActionReceipt(receipt) = item { self.rc.apply_action_receipt(receipt); }
        }
    }

    /// Both copies hold `stack` in their bags from the same grant.
    pub(super) fn give(&mut self, slot: usize, stack: ItemStack) { self.set_both(slot, Some(stack)); }

    /// Bag `slot` emptied on both copies (the player made room).
    fn clear(&mut self, slot: usize) { self.set_both(slot, None); }

    fn set_both(&mut self, slot: usize, stack: Option<ItemStack>) {
        let applied = self.rc.applied_window_boundary();
        let sp = &mut self.hs.server.players[self.i];
        crate::window_events::apply_through(sp, applied, 0);
        sp.inventory.set_slot(slot, stack.clone());
        self.inv.set_slot(slot, stack);
    }

    /// A Strict click through the real reserving entry point, by a body
    /// beside the table.
    pub(super) fn click(&mut self, click: WindowClick) {
        let eye = self.hs.server.players[self.i].player.eye_pos();
        let ctx = crate::window::ClickCtx::new(false, self.ui.station(), eye, |_| CRAFTING_TABLE);
        self.ui.apply_joined_click(&mut self.rc, &mut self.inv, &mut self.armour, &click, &ctx, &self.reg).expect("reserved and sent");
    }

    /// E (or a table) through the real open: Strict reserves, Mirror logs.
    pub(super) fn open(&mut self, table: Option<[i32; 3]>) -> bool {
        match self.rc.inventory_mode {
            InventoryMode::Strict => self.ui.open_joined(&mut self.rc, table, &mut self.inv, &mut self.armour, &self.reg).expect("reserved and sent"),
            InventoryMode::Mirror => self.ui.open_crafting(table, &mut self.inv, &mut self.armour),
        }
    }

    /// E (or Esc) on an open screen: its `Close`. Strict reserves, Mirror logs.
    fn close(&mut self) -> bool {
        match self.rc.inventory_mode {
            InventoryMode::Strict => self.ui.close_joined(&mut self.rc, &mut self.inv, &mut self.armour, &self.reg).expect("reserved and sent"),
            InventoryMode::Mirror => self.ui.close(&mut self.inv, &mut self.armour),
        }
    }

    pub(super) fn kill(&mut self) {
        let sp = &mut self.hs.server.players[self.i];
        sp.combat.health = 0.0;
        sp.combat.dead = true;
    }

    pub(super) fn respawn(&mut self) {
        self.hs.server.players[self.i].dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
        self.rc.send_respawn();
        self.pumps(3);
        assert!(self.hs.server.players[self.i].is_present_and_alive(), "respawned");
    }

    /// The server's copy once it has applied what the client has.
    pub(super) fn server_settled(&mut self) -> &crate::server::ServerPlayer {
        let applied = self.rc.applied_window_boundary();
        let sp = &mut self.hs.server.players[self.i];
        crate::window_events::apply_through(sp, applied, 0);
        sp
    }

    pub(super) fn assert_live(&self, what: &str) {
        assert!(matches!(self.rc.state, ConnectionState::Connected { .. }), "{what}: never torn down");
        assert!(self.hs.server.players.get(self.i).is_some_and(|sp| sp.is_present_and_alive()), "{what}: the joiner is still in the world");
    }

    /// Never torn down, and the client's window digests as the server's
    /// copy does once that has applied what the client has.
    pub(super) fn assert_agree(&mut self, what: &str) {
        self.assert_live(what);
        let mine = self.ui.digest(&self.inv, &self.armour);
        let sp = self.server_settled();
        let theirs = crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station);
        assert_eq!(mine, theirs, "{what}: the two windows agree");
    }

    /// The header of the client's newest reserved action.
    pub(super) fn last_action(&self) -> crate::action_identity::ActionHeader {
        self.rc.action_journal.as_ref().and_then(|j| j.actions().last()).expect("a reserved action").header
    }

    /// The server's retained fate for `action` (read without a resend).
    pub(super) fn fate(&self, action: crate::action_identity::ActionHeader) -> Option<crate::action_identity::ActionFate> {
        self.hs.server.players[self.i].action_results.as_ref()?.peek(action).map(|row| row.fate)
    }

    /// The grid is empty and `stack`'s units are in the bag on both copies,
    /// and the two windows agree.
    fn assert_returned(&mut self, stack: &ItemStack, what: &str) {
        self.assert_live(what);
        let item = stack.item.clone();
        let client_grid_empty = !self.ui.grid_holds_items();
        let client_units = units(&self.inv, &item);
        let (inv, armour, cursor, grid, station) = {
            let sp = self.server_settled();
            (sp.inventory.clone(), sp.armour, sp.cursor.clone(), sp.craft_grid.clone(), sp.station)
        };
        assert!(client_grid_empty, "{what}: the client's grid went back to its bag");
        assert_eq!(client_units, u32::from(stack.count), "{what}: the client's bag holds the grid's items");
        assert!(grid.iter().flatten().all(Option::is_none), "{what}: the server copy's grid went back too");
        assert_eq!(units(&inv, &item), u32::from(stack.count), "{what}: the server copy's bag holds them");
        assert_eq!(
            self.ui.digest(&self.inv, &self.armour),
            crate::window::digest_parts(&inv, &armour, &cursor, &grid, station),
            "{what}: the two windows agree"
        );
    }
}

/// H1 trigger 4 — a rejoin from a record holding grid items: the join sync
/// restores the grid behind a closed screen, then E. In Strict the open
/// first sends a real Close over the real grid (never an empty-grid digest,
/// which tore the session down); in Mirror (the default) the open used to
/// wipe the grid locally while the server's copy and the save kept it — the
/// pre-existing `bdd4fe87` bug, fixed deliberately. Both: the items end in
/// the bag on both copies and the screen opens.
#[test]
fn c3d_grid_rejoin_with_saved_grid_items_then_e_returns_them_in_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let what = format!("{mode:?}");
        let mut j = joined(&format!("rejoin-{what}"), mode, Some(&[(0, 0, stone(5)), (1, 1, stone(3))]));
        assert_eq!(j.ui.grid[0][0].as_ref().map(|s| s.count), Some(5), "{what}: the join sync restored the grid");
        assert!(!j.ui.open, "{what}: behind a closed screen");
        assert!(j.open(None), "{what}: E opens");
        j.pumps(3);
        assert!(j.ui.open && j.ui.station() == Station::Player, "{what}: the 2×2 is open");
        j.assert_returned(&stone(8), &what);
    }
}

/// H1 triggers 1 and 2 — a crafting table's grid holds stone when the body
/// dies. `heard`: the client has applied its death sync and the player
/// presses Esc on the death screen (the run-6 Esc bypass sent a Close on a
/// current base: `Superseded`, one correction `Replace`); otherwise the
/// Close was already in flight (the death sync is its correction). Either
/// way the grid comes back behind a closed screen; after the respawn, E
/// returns it to the bag on both copies, never torn down.
#[test]
fn c3d_strict_grid_close_at_death_then_e_returns_the_grid() {
    for heard in [true, false] {
        let what = if heard { "Esc on the death screen" } else { "Close in flight at death" };
        let mut j = joined(&format!("death-{heard}"), InventoryMode::Strict, None);
        j.give(0, stone(8));
        assert!(j.open(Some(TABLE)), "{what}: the table opens");
        j.click(WindowClick::Slot { slot: 0, right: false });
        j.click(WindowClick::Grid { row: 2, col: 2, right: false });
        j.pumps(2);
        j.assert_live(what);
        j.kill();
        // The server runs the death first; the client hears of it (`heard`)
        // or has already sent its Close on the old base.
        if heard { j.pumps(2); } else { j.hs.tick(); }
        assert!(j.ui.close_joined(&mut j.rc, &mut j.inv, &mut j.armour, &j.reg).expect("reserved"), "{what}: the Close is predicted");
        j.pumps(3);
        assert_eq!(j.ui.grid[2][2].as_ref().map(|s| s.count), Some(8), "{what}: the correction put the grid back");
        assert!(!j.ui.open, "{what}: behind a closed screen");
        j.respawn();
        assert!(j.open(None), "{what}: E opens");
        j.pumps(3);
        j.assert_returned(&stone(8), what);
    }
}

/// Run-6 review LOW-2 — E→E across a waiting correction: a dead body's
/// Close at a table is corrected by a `Replace` that snapshots the table
/// station; an `OpenPlayer` sent before the client applied it is served on
/// the old base and moves the server's station to the player's grid. The
/// `Replace` carries the station, so its apply puts the server's copy back
/// at the table, where the client adopts it: one station on both copies.
#[test]
fn c3d_strict_e_to_e_across_a_waiting_correction_ends_at_the_snapshot_station() {
    let mut j = joined("e-to-e", InventoryMode::Strict, None);
    j.give(0, stone(4));
    assert!(j.open(Some(TABLE)));
    j.click(WindowClick::Slot { slot: 0, right: false });
    j.pumps(2);
    j.kill();
    j.pumps(2);
    assert_eq!(j.ui.station(), Station::Table { cell: TABLE }, "the death sync carries the table");
    assert!(j.ui.close_joined(&mut j.rc, &mut j.inv, &mut j.armour, &j.reg).expect("reserved"));
    assert!(j.open(None), "E again, before the correction arrives");
    j.pumps(3);
    let server_station = j.server_settled().station;
    assert_eq!(j.ui.station(), Station::Table { cell: TABLE }, "the client adopted the correction's station");
    assert_eq!(server_station, Station::Table { cell: TABLE }, "the server's copy applied the same station");
    j.respawn();
    assert!(j.ui.close_joined(&mut j.rc, &mut j.inv, &mut j.armour, &j.reg).expect("reserved"));
    j.pumps(3);
    j.assert_returned(&stone(4), "after the respawn");
}

/// C3d run 10 (run-8 review LOW-A, LOW-B) — the refused return-the-grid
/// `Close`: a rejoin from a record with a full bag and a table's leftovers
/// beyond the 2×2 (the record keeps the grid, not the table, so the station
/// is the player's), then E or a table. The open's `Close` is refused, so
/// the asked screen doesn't open: the screen shows the leftovers at its own
/// station, drawing every held cell, with nothing sent and both copies
/// agreeing. Once one bag slot is free, E (a `Close` on the open screen)
/// returns them to the bag on both copies.
#[test]
fn c3d_grid_refused_return_close_shows_every_held_cell_in_both_modes() {
    let dirt = ItemStack::new_block(crate::block::DIRT, 64);
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for ask in [None, Some(TABLE)] {
            let what = format!("{mode:?}, asked {ask:?}");
            let held = [(2, 0, stone(5)), (1, 2, stone(3))];
            let mut j = joined_from(&format!("held-{mode:?}-{}", ask.is_some()), mode, Some((&held, Some(dirt.clone()))));
            assert!(!j.ui.open && j.ui.grid[2][0].is_some(), "{what}: the join sync restored the grid behind a closed screen");
            assert!(!j.open(ask), "{what}: the return Close is refused (a full bag): the asked screen doesn't open");
            j.pumps(3);
            j.assert_live(&what);
            assert!(j.ui.open, "{what}: the screen shows what is left");
            assert_eq!(j.ui.station(), Station::Player, "{what}: at its own station, never the asked one");
            assert_eq!(j.ui.grid[2][0].as_ref().map(|s| s.count), Some(5), "{what}: the leftovers are intact");
            assert_eq!(j.ui.grid[1][2].as_ref().map(|s| s.count), Some(3), "{what}: all of them");
            assert_eq!(j.ui.view_grid_size(), 3, "{what}: every held cell is drawn");
            let server_grid = j.server_settled().craft_grid.clone();
            assert_eq!(server_grid, j.ui.grid, "{what}: the server's copy holds the same grid");
            j.assert_agree(&what);
            j.clear(35);
            assert!(j.close(), "{what}: E closes once the bag has room");
            j.pumps(3);
            j.assert_returned(&stone(8), &what);
        }
    }
}

/// C3d run 10 (run-8 review LOW-D) — a Mirror joiner locks a slot before its
/// client applies the join sync. Window ops made before the personal state
/// is ready are never sent (the join `Replace` overwrites the contents), so
/// the server's copy doesn't have the lock yet. A Mirror client keeps its
/// own locks across that `Replace`, so once its personal state is installed
/// it logs fresh ops that bring the server's copy to its window's settings
/// (`CraftingUi::settle_initial_settings`): both copies end with the lock,
/// and the two windows agree.
#[test]
fn c3d_mirror_lock_toggled_before_the_join_sync_reaches_the_server_once_ready() {
    let mut j = start("lock-race", InventoryMode::Mirror, Some((&[], None)));
    for _ in 0..20 {
        j.hs.tick();
        j.rc.poll();
        if j.rc.player_index().is_some() { break; }
    }
    j.place();
    assert_eq!(j.rc.applied_window_boundary(), 0, "the join sync is heard, not applied");
    assert!(!j.rc.personal_ready(), "the personal state isn't installed yet");
    j.ui.apply_click(&mut j.inv, &mut j.armour, &WindowClick::ToggleLock { slot: 3 }, false, glam::Vec3::ZERO, |_| AIR);
    for logged in j.ui.take_ops(&j.inv, &j.armour) { j.rc.send_window_op(logged); }
    j.hs.tick();
    assert!(!j.hs.server.players[j.i].inventory.is_locked(3), "an op made before the personal state is ready isn't sent");
    j.pumps(3);
    assert!(j.rc.applied_window_boundary() >= 1 && j.syncs == 1, "the client applied the join sync");
    assert!(j.rc.personal_ready(), "the client installed its personal state");
    assert!(j.inv.is_locked(3), "the Mirror client keeps its lock");
    assert!(j.server_settled().inventory.is_locked(3), "the server's copy has the lock once the client is ready");
    j.assert_agree("after the join sync");
}

impl Joined {
    /// A rejoin heard (nothing applied), its body placed: the client's
    /// personal state is not ready yet.
    fn heard(tag: &str, bag: Option<ItemStack>) -> Joined {
        let mut j = start(tag, InventoryMode::Mirror, Some((&[], bag)));
        for _ in 0..20 {
            j.hs.tick();
            j.rc.poll();
            if j.rc.player_index().is_some() { break; }
        }
        j.place();
        assert!(!j.rc.personal_ready(), "the personal state isn't installed yet");
        j
    }

    /// Receive (no flush) until the personal state is ready, then flush
    /// through both of the game loop's send paths and let it settle.
    fn ready_then_flush(&mut self) {
        for _ in 0..20 {
            if self.rc.personal_ready() { break; }
            self.receive();
        }
        assert!(self.rc.personal_ready(), "the client installed its personal state");
        assert_eq!(self.syncs, 1, "the client applied the join sync");
        self.flush();
        self.pumps(2);
    }

    /// Every op the server served matched the client's digest, the first
    /// (the baseline) included, and none was refused.
    fn assert_lockstep(&self, what: &str) {
        let sp = &self.hs.server.players[self.i];
        assert_eq!(sp.window_events.baseline, Some(true), "{what}: the first op served matched");
        assert_eq!(sp.possession.window_mismatch, 0, "{what}: no digest mismatch counted");
        assert_eq!(sp.possession.window_refused, 0, "{what}: no op refused");
    }
}

/// Review D1 — a Mirror joiner locks a slot in the frame before its personal
/// state becomes ready, with no flush between: the op is still waiting when
/// the state installs. It must never go out (it was digested against the
/// window before the join `Replace`); the settings logged at the flip bring
/// the lock to the server's copy exactly once, ahead of everything else,
/// through both send paths (`flush_ops_before_edits` takes everything first).
#[test]
fn c3d_mirror_lock_waiting_when_the_personal_state_installs_reaches_the_server_once() {
    let mut j = Joined::heard("lock-waiting", None);
    j.ui.apply_click(&mut j.inv, &mut j.armour, &WindowClick::ToggleLock { slot: 3 }, false, glam::Vec3::ZERO, |_| AIR);
    j.ready_then_flush();
    assert!(j.inv.is_locked(3), "the Mirror client keeps its lock");
    assert!(j.server_settled().inventory.is_locked(3), "the server's copy has the lock, toggled once");
    j.assert_lockstep("after the join sync");
    j.assert_agree("after the join sync");
}

/// Review D1 — two locks, a lock toggled on and off again, and a content op,
/// all waiting when the personal state installs. The content op is dropped
/// with the rest (the join `Replace` already undid it on the client); the
/// server's copy ends with exactly the two locks.
#[test]
fn c3d_mirror_two_locks_and_a_content_op_waiting_at_ready_converge() {
    let mut j = Joined::heard("two-locks", Some(stone(4)));
    // Before the join sync, the client's own window still holds what it had.
    j.inv.set_slot(0, Some(stone(2)));
    for slot in [3, 7, 12, 12] {
        j.ui.apply_click(&mut j.inv, &mut j.armour, &WindowClick::ToggleLock { slot }, false, glam::Vec3::ZERO, |_| AIR);
    }
    j.ui.apply_click(&mut j.inv, &mut j.armour, &WindowClick::Slot { slot: 0, right: false }, false, glam::Vec3::ZERO, |_| AIR);
    j.ready_then_flush();
    let locked = |inv: &crate::inventory::Inventory| (0..36).filter(|&i| inv.is_locked(i)).collect::<Vec<_>>();
    assert_eq!(locked(&j.inv), vec![3, 7], "the client's locks");
    assert!(j.ui.cursor_item.is_none() && j.inv.slot(0).map(|s| s.count) == Some(4), "the join Replace undid the client's pickup");
    let sp = j.server_settled();
    assert_eq!(locked(&sp.inventory), vec![3, 7], "the server's copy holds the same two locks");
    assert!(sp.cursor.is_none() && sp.inventory.slot(0).map(|s| s.count) == Some(4), "the waiting pickup never reached the server");
    j.assert_lockstep("after the join sync");
    j.assert_agree("after the join sync");
}
