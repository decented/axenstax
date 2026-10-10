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

const TABLE: [i32; 3] = [41, 80, 40];
const SECKEY: [u8; 32] = [71; 32];

struct Joined {
    _g: crate::save::WorldsRootGuard,
    hs: HostedServer,
    rc: RemoteClient,
    i: usize,
    inv: crate::inventory::Inventory,
    armour: [Option<crate::armour::ArmourItem>; 4],
    ui: crate::craft_ui::CraftingUi,
    inbox: crate::window_events::WindowInbox,
    reg: crate::block::BlockRegistry,
}

fn stone(n: u8) -> ItemStack { ItemStack::new_block(STONE, n) }

fn units(inv: &crate::inventory::Inventory, item: &crate::item::Item) -> u32 {
    (0..36).filter_map(|at| inv.slot(at)).filter(|s| s.item == *item).map(|s| u32::from(s.count)).sum()
}

/// A server in `mode` on a stone floor with a crafting table beside the
/// joiner's spot (40.5, 80, 40.5). `saved_grid`: a signed rejoin whose saved
/// record holds those grid cells (row, col, stack); otherwise a guest.
fn joined(tag: &str, mode: InventoryMode, saved_grid: Option<&[(usize, usize, ItemStack)]>) -> Joined {
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
    let rc = match saved_grid {
        None => RemoteClient::from_transport(Box::new(hs.attach_test_remote()), crate::remote_client::build_join_request_guest("Crafter", 0), None),
        Some(cells) => {
            // A saved record: a crash, a drop or a refused Close left items
            // in the grid (the save keeps the grid).
            let keys = nostr::Keys::new(nostr::SecretKey::from_slice(&SECKEY).unwrap());
            let mut saved = crate::server::ServerPlayer::new(hs.server.world_spawn());
            saved.server_simulated = true;
            saved.verified_pubkey = Some(keys.public_key().to_bytes());
            saved.enter_world();
            for (row, col, stack) in cells { saved.craft_grid[*row][*col] = Some(stack.clone()); }
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
    let mut j = Joined {
        _g: g, hs, rc, i: 0, inv: crate::inventory::Inventory::new(), armour: [None; 4],
        ui: crate::craft_ui::CraftingUi::new(), inbox: Default::default(), reg: crate::block::BlockRegistry::new(),
    };
    for _ in 0..20 {
        j.pump();
        if j.rc.player_index().is_some() && (saved_grid.is_none() || j.rc.applied_window_boundary() >= 1) { break; }
    }
    j.pumps(2);
    j.i = j.rc.player_index().expect("joined") as usize;
    assert_eq!(j.rc.inventory_mode, mode, "the session's mode");
    let p = &mut j.hs.server.players[j.i].player;
    p.pos = glam::Vec3::new(40.5, 80.0, 40.5);
    p.velocity = glam::Vec3::ZERO;
    p.on_ground = true;
    j
}

impl Joined {
    /// One server tick, then the client reads and applies what it was sent,
    /// in event order (the game loop's `apply_window_inbox`).
    fn pump(&mut self) {
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
                }
                InboxItem::Grant(grant) => { crate::remote_entities::apply_inventory_grant(&mut self.inv, &grant, &self.reg); }
                _ => {}
            }
            self.rc.note_window_event(event);
        }
        // Mirror: the op log goes out as the game loop sends it.
        if self.rc.inventory_mode == InventoryMode::Mirror {
            for logged in self.ui.take_ops(&self.inv, &self.armour) { self.rc.send_window_op(logged); }
        }
    }

    fn pumps(&mut self, n: usize) { for _ in 0..n { self.pump(); } }

    /// Both copies hold `stack` in their bags from the same grant.
    fn give(&mut self, slot: usize, stack: ItemStack) {
        let applied = self.rc.applied_window_boundary();
        let sp = &mut self.hs.server.players[self.i];
        crate::window_events::apply_through(sp, applied, 0);
        sp.inventory.set_slot(slot, Some(stack.clone()));
        self.inv.set_slot(slot, Some(stack));
    }

    /// A Strict click through the real reserving entry point, by a body
    /// beside the table.
    fn click(&mut self, click: WindowClick) {
        let eye = self.hs.server.players[self.i].player.eye_pos();
        let ctx = crate::window::ClickCtx::new(false, self.ui.station(), eye, |_| CRAFTING_TABLE);
        self.ui.apply_joined_click(&mut self.rc, &mut self.inv, &mut self.armour, &click, &ctx, &self.reg).expect("reserved and sent");
    }

    /// E (or a table) through the real open: Strict reserves, Mirror logs.
    fn open(&mut self, table: Option<[i32; 3]>) -> bool {
        match self.rc.inventory_mode {
            InventoryMode::Strict => self.ui.open_joined(&mut self.rc, table, &mut self.inv, &mut self.armour, &self.reg).expect("reserved and sent"),
            InventoryMode::Mirror => self.ui.open_crafting(table, &mut self.inv, &mut self.armour),
        }
    }

    fn kill(&mut self) {
        let sp = &mut self.hs.server.players[self.i];
        sp.combat.health = 0.0;
        sp.combat.dead = true;
    }

    fn respawn(&mut self) {
        self.hs.server.players[self.i].dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
        self.rc.send_respawn();
        self.pumps(3);
        assert!(self.hs.server.players[self.i].is_present_and_alive(), "respawned");
    }

    /// The server's copy once it has applied what the client has.
    fn server_settled(&mut self) -> &crate::server::ServerPlayer {
        let applied = self.rc.applied_window_boundary();
        let sp = &mut self.hs.server.players[self.i];
        crate::window_events::apply_through(sp, applied, 0);
        sp
    }

    fn assert_live(&self, what: &str) {
        assert!(matches!(self.rc.state, ConnectionState::Connected { .. }), "{what}: never torn down");
        assert!(self.hs.server.players.get(self.i).is_some_and(|sp| sp.is_present_and_alive()), "{what}: the joiner is still in the world");
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
