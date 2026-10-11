//! C3d run 6 (coordinator rule after the run-5 review: M1, LOW-1, LOW-2,
//! the run-4 Nit) — every `Superseded` comes with an authoritative
//! correction, and every `Replace` carries the station and the lock mask.
//! The client's window is modelled SEPARATELY from the server's copy: it
//! predicts with the shared click rule and applies what the server sends
//! (`inventory_sync::apply`, `adopt_station`, grants); its digest is never
//! derived from the server's copy. C3d run 10 (run-8 review LOW-C): the
//! death cases that end in E moved to `c3d_death_client_tests.rs`, on a real
//! `RemoteClient` and the real open path; what stays here sends raw ops.
use super::c3d_edit_tests::edit_rig;
use super::c3d_strict_tests::Rig;
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::action_wire::ActionIntent;
use crate::item::ItemStack;
use crate::transport::ClientTransport as _;
use crate::window::Station;

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
}

impl Client {
    /// The client after the join sync (in lockstep with the server then).
    fn joined(r: &Rig) -> Self {
        let sp = r.sp();
        Client { inv: sp.inventory.clone(), armour: sp.armour, cursor: sp.cursor.clone(), grid: sp.craft_grid.clone(),
            ui: crate::craft_ui::CraftingUi::new(), debt: Default::default(), applied: r.applied, rejects: Vec::new() }
    }
    fn station(&self) -> Station { self.ui.station() }
    fn digest(&self) -> u32 { crate::window::digest_parts(&self.inv, &self.armour, &self.cursor, &self.grid, self.station()) }
    /// Apply everything the server sent, in order (what the game loop does).
    fn receive(&mut self, r: &mut Rig) {
        let reg = crate::block::BlockRegistry::new();
        while let Some(pkt) = r.client.try_recv_from_server() {
            match protocol::deserialize_header(&pkt) {
                Some((protocol::PacketType::InventorySync, p)) => {
                    let sync: protocol::InventorySyncPacket = protocol::safe_deserialize(p).unwrap();
                    let mut w = crate::window::WindowMut { inv: &mut self.inv, armour: &mut self.armour, cursor: &mut self.cursor, grid: &mut self.grid, container: None };
                    let station = crate::inventory_sync::apply(&mut w, &sync, &reg, &mut crate::use_edits::SentUses::default(), &mut self.debt, crate::action_identity::InventoryMode::Strict).unwrap();
                    self.ui.adopt_station(station.expect("a Strict session adopts the station"));
                    self.applied = self.applied.max(sync.window_event);
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

fn kill(r: &mut Rig) {
    let sp = &mut r.hs.server.players[r.i];
    sp.combat.health = 0.0;
    sp.combat.dead = true;
    r.hs.tick();
    assert!(r.sp().window_events.death_replace.is_some(), "the death sync is queued");
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

/// C3d run 10 (run-6 review LOW-4, LOW-2's narrowing) — the overdue valve
/// applied the death `Replace` on the server's copy before the client did;
/// an `OpenPlayer` the client then sends on its old base (whose window
/// differs from the replaced copy) is served with its digest skipped,
/// because its base predates the latest `Replace`, not because one waits
/// (the old `replace_waiting()` skip would compare it and refuse).
#[test]
fn c3d_strict_stale_config_after_a_valve_applied_replace_is_served() {
    let mut r = edit_rig("valve-config");
    let mut c = Client::joined(&r);
    c.inv.set_slot(5, Some(ItemStack::new_block(crate::block::DIRT, 1)));
    kill(&mut r);
    let now = r.hs.server.tick_counter + crate::window_events::EVENT_ACK_TIMEOUT_TICKS;
    crate::window_events::apply_overdue(&mut r.hs.server.players[r.i], now);
    assert!(!r.sp().window_events.replace_waiting(), "the valve applied the death Replace");
    let sp = r.sp();
    let copy = crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station);
    assert_ne!(c.digest(), copy, "the client's old base differs from the replaced copy");
    let e = op(&mut r, protocol::WireWindowOp::OpenPlayer, c.digest(), true);
    c.receive(&mut r);
    assert!(c.rejects.is_empty(), "never torn down: {:?}", c.rejects);
    assert_eq!(r.row(e).map(|row| row.fate), Some(ActionFate::Accepted), "served, its digest skipped");
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

/// C3d run 7 (coordinator rule: Mirror must not change) — a Mirror join sync
/// and a Mirror death sync, applied by the client's real
/// `inventory_sync::apply` in its session's mode, leave the locks the player
/// set and its open table screen exactly as before C3d. The same packets in
/// a Strict session install them (the run-6 behaviour, Strict only).
#[test]
fn c3d_mirror_join_and_death_syncs_keep_client_locks_and_station() {
    use super::c3d_tests::KEY;
    use super::tests::send_signed_join;
    let _g = crate::save::WorldsRootGuard::new("c3d-mirror-sync");
    // A first visit leaves a record, so the rejoin's JoinAccept carries a
    // join sync (a fresh first visit has none).
    {
        let mut first = super::tests::start_room_test_server("c3d-mirror-sync", 0);
        let (i, _client, heard) = super::c3d_tests::join(&mut first, 17);
        assert_eq!(heard.accepts.len(), 1, "the first visit is admitted");
        first.server.save_joiner(i).expect("the record is saved");
    }
    let mut hs = super::tests::start_room_test_server("c3d-mirror-sync", 0);
    assert_eq!(hs.inventory_mode(), crate::action_identity::InventoryMode::Mirror, "Mirror is the default");
    let client = hs.attach_test_remote_with_binding(Some([17; 32]));
    send_signed_join(&client, KEY, Some([17; 32]));
    hs.tick();
    let i = hs.server.players.len() - 1;
    let syncs = |client: &crate::transport::ChannelClientTransport| -> Vec<protocol::InventorySyncPacket> {
        let mut out = Vec::new();
        while let Some(pkt) = client.try_recv_from_server() {
            if let Some((protocol::PacketType::InventorySync, p)) = protocol::deserialize_header(&pkt) {
                out.push(protocol::safe_deserialize(p).unwrap());
            }
        }
        out
    };
    let mut seen = syncs(&client);
    assert_eq!(seen.len(), 1, "the join sync");
    let sp = &mut hs.server.players[i];
    sp.combat.health = 0.0;
    sp.combat.dead = true;
    hs.tick();
    seen.extend(syncs(&client));
    assert_eq!(seen.len(), 2, "and the death sync");
    let reg = crate::block::BlockRegistry::new();
    for (what, sync) in ["join", "death"].into_iter().zip(&seen) {
        assert!(sync.locks & (1 << 3) == 0 && sync.station.is_none(), "{what}: the packet carries other locks and no table");
        for mode in [crate::action_identity::InventoryMode::Mirror, crate::action_identity::InventoryMode::Strict] {
            let mut inv = crate::inventory::Inventory::new_client();
            inv.toggle_lock(3);
            let (mut armour, mut cursor, mut grid) = ([None; 4], None, crate::window::CraftGrid::default());
            let mut ui = crate::craft_ui::CraftingUi::new();
            ui.open = true;
            ui.table = Some(TABLE);
            let mut w = crate::window::WindowMut { inv: &mut inv, armour: &mut armour, cursor: &mut cursor, grid: &mut grid, container: None };
            let station = crate::inventory_sync::apply(&mut w, sync, &reg, &mut crate::use_edits::SentUses::default(), &mut Default::default(), mode).unwrap();
            if let Some(station) = station { ui.adopt_station(station); }
            if mode == crate::action_identity::InventoryMode::Mirror {
                assert!(inv.is_locked(3), "{what}: a Mirror client keeps its own locks");
                assert!(ui.open && ui.station() == Station::Table { cell: TABLE }, "{what}: and its open table screen");
            } else {
                assert!(!inv.is_locked(3), "{what}: a Strict client takes the server's lock mask");
                assert!(!ui.open && ui.station() == Station::Player, "{what}: and its station");
            }
        }
    }
}
