//! C3d run 6 (coordinator rule after the run-5 review: M1, LOW-1, LOW-2,
//! the run-4 Nit) — every `Superseded` comes with an authoritative
//! correction, and every `Replace` carries the station and the lock mask.
//! The client's window is modelled SEPARATELY from the server's copy: it
//! predicts with the shared click rule and applies what the server sends
//! (`inventory_sync::apply`, `adopt_station`, grants); its digest is never
//! derived from the server's copy.
use super::c3d_edit_tests::edit_rig;
use super::c3d_strict_tests::Rig;
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::action_wire::ActionIntent;
use crate::item::ItemStack;
use crate::transport::ClientTransport as _;
use crate::window::{Station, WindowClick};

const TABLE: [i32; 3] = [41, 80, 40];

/// The joined client's own window, station and applied event count.
struct Client {
    inv: crate::inventory::Inventory,
    armour: [Option<crate::armour::ArmourItem>; 4],
    cursor: Option<ItemStack>,
    grid: crate::window::CraftGrid,
    ui: crate::craft_ui::CraftingUi,
    debt: crate::container_window::CorrectionDebt,
    applied: u32,
    rejects: Vec<String>,
    syncs: usize,
}

impl Client {
    /// The client after the join sync (in lockstep with the server then).
    fn joined(r: &Rig) -> Self {
        let sp = r.sp();
        Client { inv: sp.inventory.clone(), armour: sp.armour, cursor: sp.cursor.clone(), grid: sp.craft_grid.clone(),
            ui: crate::craft_ui::CraftingUi::new(), debt: Default::default(), applied: r.applied, rejects: Vec::new(), syncs: 0 }
    }
    fn station(&self) -> Station { self.ui.station() }
    fn digest(&self) -> u32 { crate::window::digest_parts(&self.inv, &self.armour, &self.cursor, &self.grid, self.station()) }
    /// Predict `click` with the shared rule; the station follows it.
    fn predict(&mut self, r: &Rig, click: &WindowClick) -> (u32, bool) {
        let eye = r.sp().player.eye_pos();
        let ctx = crate::window::ClickCtx::new(false, self.station(), eye, |_| crate::block::CRAFTING_TABLE);
        let mut view = crate::window::WindowMut { inv: &mut self.inv, armour: &mut self.armour, cursor: &mut self.cursor, grid: &mut self.grid, container: None };
        let result = crate::window::apply(&mut view, click, &ctx);
        let station = crate::window::station_after(self.station(), click, &result);
        self.ui.table = match station { Station::Table { cell } => Some(cell), Station::Player => None };
        (self.digest(), result.ok())
    }
    /// Apply everything the server sent, in order (what the game loop does).
    fn receive(&mut self, r: &mut Rig) {
        let reg = crate::block::BlockRegistry::new();
        while let Some(pkt) = r.client.try_recv_from_server() {
            match protocol::deserialize_header(&pkt) {
                Some((protocol::PacketType::InventorySync, p)) => {
                    let sync: protocol::InventorySyncPacket = protocol::safe_deserialize(p).unwrap();
                    let mut w = crate::window::WindowMut { inv: &mut self.inv, armour: &mut self.armour, cursor: &mut self.cursor, grid: &mut self.grid, container: None };
                    crate::inventory_sync::apply(&mut w, &sync, &reg, &mut crate::use_edits::SentUses::default(), &mut self.debt).unwrap();
                    self.ui.open = true; // a screen stays logically open
                    self.ui.adopt_station(crate::inventory_sync::station(&sync));
                    self.applied = self.applied.max(sync.window_event);
                    self.syncs += 1;
                }
                Some((protocol::PacketType::InventoryGrant, p)) => {
                    let grant: protocol::InventoryGrantPacket = protocol::safe_deserialize(p).unwrap();
                    crate::remote_entities::apply_inventory_grant(&mut self.inv, &grant, &reg);
                    self.applied = self.applied.max(grant.window_event);
                }
                Some((protocol::PacketType::JoinReject, p)) => self.rejects.push(protocol::safe_deserialize::<protocol::JoinRejectPacket>(p).unwrap().reason),
                _ => {}
            }
        }
        r.applied = self.applied;
    }
}

fn op(r: &mut Rig, op: protocol::WireWindowOp, digest: u32, client_ok: bool) -> ActionHeader {
    let action = r.next();
    let pkt = protocol::WindowOpPacket { op_seq: 0, op, digest, events_applied: action.events_applied, touched: Vec::new(), claims: Vec::new(), client_ok };
    r.send(action, ActionIntent::Window(pkt));
    action
}

fn click(r: &mut Rig, c: &mut Client, click: WindowClick) -> ActionHeader {
    let (digest, ok) = c.predict(r, &click);
    op(r, protocol::WireWindowOp::Click(click), digest, ok)
}

/// A Strict joiner at a crafting table holding 8 stone in slot 0.
fn at_table(tag: &str) -> (Rig, Client) {
    let mut r = edit_rig(tag);
    r.hs.server.world.set_block(TABLE[0], TABLE[1], TABLE[2], crate::block::CRAFTING_TABLE);
    r.hs.server.players[r.i].inventory.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 8)));
    let mut c = Client::joined(&r);
    c.ui.open = true;
    c.ui.table = Some(TABLE);
    let open = op(&mut r, protocol::WireWindowOp::OpenTable { cell: TABLE }, c.digest(), true);
    assert_eq!(r.row(open).map(|row| row.fate), Some(ActionFate::Accepted), "the table opens");
    (r, c)
}

fn kill(r: &mut Rig) {
    let sp = &mut r.hs.server.players[r.i];
    sp.combat.health = 0.0;
    sp.combat.dead = true;
    r.hs.tick();
    assert!(r.sp().window_events.death_replace.is_some(), "the death sync is queued");
}

fn respawn(r: &mut Rig) {
    r.hs.server.players[r.i].dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
    r.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Respawn, &()));
    r.hs.tick();
    assert!(r.sp().is_present_and_alive(), "respawned");
}

fn server_digest(r: &Rig) -> u32 {
    let sp = r.sp();
    crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station)
}

/// After the respawn: E, then a click, both Accepted on the client's own
/// digests, never torn down, and the two windows agree.
fn assert_converges(r: &mut Rig, c: &mut Client, what: &str) {
    respawn(r);
    c.receive(r);
    c.ui.table = None; // E opens the player's own screen
    let e = op(r, protocol::WireWindowOp::OpenPlayer, c.digest(), true);
    let put = click(r, c, WindowClick::Slot { slot: 20, right: false });
    c.receive(r);
    assert!(c.rejects.is_empty(), "{what}: never torn down: {:?}", c.rejects);
    assert_eq!(r.row(e).map(|row| row.fate), Some(ActionFate::Accepted), "{what}: E after the respawn");
    assert_eq!(r.row(put).map(|row| row.fate), Some(ActionFate::Accepted), "{what}: the next click");
    let sp = &mut r.hs.server.players[r.i];
    crate::window_events::apply_through(sp, c.applied, 0);
    assert_eq!(c.digest(), server_digest(r), "{what}: the windows agree");
}

/// Run-5 review M1 sequence A/B — a crafting table with a cursor stack: the
/// body dies, the client applies the death sync, then (Esc on the death
/// screen, or the force-close after an early local respawn) predicts a
/// Close on a CURRENT base. It is `Superseded` and ONE correction `Replace`
/// (cursor, grid, station, locks) follows; a second action on the old base
/// is superseded without another. The client converges after the respawn.
#[test]
fn c3d_strict_dead_body_close_at_a_table_gets_one_correction_replace() {
    let (mut r, mut c) = at_table("dead-close");
    click(&mut r, &mut c, WindowClick::Slot { slot: 0, right: false });
    c.receive(&mut r);
    kill(&mut r);
    c.receive(&mut r);
    assert_eq!(c.syncs, 1, "the death sync applied on the client");
    assert_eq!(c.station(), Station::Table { cell: TABLE }, "the death sync carries the table station");
    let close = click(&mut r, &mut c, WindowClick::Close);
    assert_eq!(r.row(close).map(|row| row.fate), Some(ActionFate::Superseded));
    assert!(r.sp().window_events.replace_waiting(), "a correction Replace follows the dead body's Close");
    let events = r.sp().window_events.last_number();
    let lock = click(&mut r, &mut c, WindowClick::ToggleLock { slot: 5 });
    assert_eq!(r.row(lock).map(|row| row.fate), Some(ActionFate::Superseded));
    assert_eq!(r.sp().window_events.last_number(), events, "at most one correction outstanding");
    c.receive(&mut r);
    assert_eq!(c.syncs, 2, "exactly one correction reached the client");
    assert_converges(&mut r, &mut c, "dead-body Close");
}

/// Run-5 review LOW-1 — a Close (grid holding stone) and a `ToggleLock`
/// already in flight when the killing hit lands: both predate the death
/// sync, which corrects them (it carries the grid, the station and the lock
/// mask); no further `Replace`.
#[test]
fn c3d_strict_close_and_toggle_lock_in_flight_at_death_converge() {
    let (mut r, mut c) = at_table("in-flight");
    click(&mut r, &mut c, WindowClick::Slot { slot: 0, right: false });
    click(&mut r, &mut c, WindowClick::Grid { row: 0, col: 0, right: false });
    c.receive(&mut r);
    kill(&mut r);
    // Sent before the client heard of its death: on the old base.
    let close = click(&mut r, &mut c, WindowClick::Close);
    let lock = click(&mut r, &mut c, WindowClick::ToggleLock { slot: 7 });
    assert_eq!(r.row(close).map(|row| row.fate), Some(ActionFate::Superseded));
    assert_eq!(r.row(lock).map(|row| row.fate), Some(ActionFate::Superseded));
    c.receive(&mut r);
    assert_eq!(c.syncs, 1, "the death sync is the only correction");
    assert!(!c.inv.is_locked(7), "the lock mask is the server's");
    assert_eq!(c.station(), Station::Table { cell: TABLE });
    assert_converges(&mut r, &mut c, "in flight at death");
}

/// Run-5 review LOW-2 — a real death with an unapplied grant, then E
/// (`OpenPlayer`) at the old base: served, never torn down.
#[test]
fn c3d_strict_open_player_at_a_base_before_the_death_sync_is_served() {
    let mut r = edit_rig("config-stale");
    let mut c = Client::joined(&r);
    let i = r.i;
    r.hs.grant_to_joiner(i, [ItemStack::new_block(crate::block::DIRT, 3)]);
    // The death settles the unapplied grant (its base moves past the
    // client's) and queues the death sync; E was pressed before either.
    kill(&mut r);
    let e = op(&mut r, protocol::WireWindowOp::OpenPlayer, c.digest(), true);
    c.receive(&mut r);
    assert!(c.rejects.is_empty(), "never torn down: {:?}", c.rejects);
    assert_eq!(r.row(e).map(|row| row.fate), Some(ActionFate::Accepted), "configuration at a stale base is served");
}

/// Configuration while dead moves no items: `SetAutoRefill`, `OpenTable`
/// and `OpenPlayer` against a body with `combat.dead` set are Accepted with
/// their configuration result and zero movement over window, world and
/// ground.
#[test]
fn c3d_strict_configuration_while_dead_moves_no_items() {
    let mut r = edit_rig("config-dead");
    r.hs.server.world.set_block(TABLE[0], TABLE[1], TABLE[2], crate::block::CRAFTING_TABLE);
    r.hs.server.players[r.i].inventory.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 8)));
    let mut c = Client::joined(&r);
    kill(&mut r);
    c.receive(&mut r);
    let applied = c.applied;
    crate::window_events::apply_through(&mut r.hs.server.players[r.i], applied, 0);
    let places = super::c3d_strict_tests::server_places(r.sp());
    let ground = r.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().count();
    let refill = op(&mut r, protocol::WireWindowOp::SetAutoRefill { on: !c.inv.auto_refill }, { c.inv.auto_refill = !c.inv.auto_refill; c.digest() }, true);
    assert_eq!(r.sp().inventory.auto_refill, c.inv.auto_refill, "auto-refill set");
    c.ui.table = Some(TABLE);
    let table = op(&mut r, protocol::WireWindowOp::OpenTable { cell: TABLE }, c.digest(), true);
    assert_eq!(r.sp().station, Station::Table { cell: TABLE }, "the table station");
    c.ui.table = None;
    let player = op(&mut r, protocol::WireWindowOp::OpenPlayer, c.digest(), true);
    assert_eq!(r.sp().station, Station::Player);
    c.receive(&mut r);
    assert!(c.rejects.is_empty(), "{:?}", c.rejects);
    for (what, action) in [("SetAutoRefill", refill), ("OpenTable", table), ("OpenPlayer", player)] {
        assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Accepted), "{what} is served while dead");
    }
    assert_eq!(super::c3d_strict_tests::server_places(r.sp()), places, "no window place moved");
    assert_eq!(r.hs.server.world.get_block(TABLE[0], TABLE[1], TABLE[2]), crate::block::CRAFTING_TABLE, "world untouched");
    assert_eq!(r.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().count(), ground, "nothing on the ground");
}

/// Run-4 review Nit — a `SetAutoRefill` served across a waiting `Replace`:
/// the obligation fold and the raw copy after the `Replace` applies agree
/// on auto-refill and on the lock mask (one source each).
#[test]
fn c3d_strict_auto_refill_across_a_waiting_replace_agrees_in_fold_and_copy() {
    let mut r = edit_rig("nit");
    let c = Client::joined(&r);
    r.hs.server.players[r.i].inventory.toggle_lock(4);
    let sp = &mut r.hs.server.players[r.i];
    sp.combat.health = 0.0;
    sp.combat.dead = true;
    r.hs.tick();
    let n = r.sp().window_events.death_replace.expect("death sync waits");
    let on = !r.sp().inventory.auto_refill;
    let action = op(&mut r, protocol::WireWindowOp::SetAutoRefill { on }, c.digest(), true);
    assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Accepted));
    let reg = crate::block::BlockRegistry::new();
    let mut fold = crate::window_events::strict_effective_marker(r.sp(), &reg).expect("fold");
    let (fold_refill, fold_locks) = { let v = fold.view(); (v.inv.auto_refill, v.inv.locked_indices()) };
    let sp = &mut r.hs.server.players[r.i];
    crate::window_events::apply_through(sp, n, 0);
    assert_eq!(fold_refill, on, "the fold has the served setting");
    assert_eq!(sp.inventory.auto_refill, fold_refill, "the applied Replace keeps the same setting");
    assert_eq!(sp.inventory.locked_indices(), fold_locks, "and the same lock mask");
}
