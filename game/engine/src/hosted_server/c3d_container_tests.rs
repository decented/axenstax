//! C3d — Strict shared containers and window configuration through the real
//! `HostedServer` dispatch, Strict selected internally on real signed
//! joiners. Two joiners contest one chest: exactly one wins each contested
//! unit, the other gets `NormalisedAcceptedNoop` with zero effect, and every
//! unit, Plan-free identity and durability is conserved over both bags,
//! grids, cursors, armour, the real container and the world's ground items.

use super::c3d_strict_tests::{arm, holdings, server_places};
use super::c3d_tests::{hear, Heard, KEY};
use super::tests::{send_signed_join, start_room_test_server};
use super::*;
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::action_wire::{ActionIntent, ActionPacket};
use crate::container_window::{ContainerClick, ContainerKind};
use crate::item::{Item, ItemStack};
use crate::strict_admission::StrictTeardown;
use crate::transport::ClientTransport as _;
use std::collections::BTreeMap;

const KEY_B: [u8; 32] = [67; 32];

/// One joined Strict client: its connection, the action identity the
/// server issued, its next sequence and the window events it applied.
pub(super) struct Joiner {
    pub(super) i: usize,
    pub(super) client: ChannelClientTransport,
    pub(super) accept: protocol::JoinAcceptPacket,
    pub(super) seq: u64,
    pub(super) applied: u32,
}

pub(super) fn join_as(hs: &mut HostedServer, key: [u8; 32], b: u8) -> Joiner {
    let mut j = join_unsynced(hs, key, b);
    j.apply_all(hs);
    j
}

/// Run 5 — a joined Strict client that has applied NO window event yet (its
/// join sync, when there is one, still waits).
pub(super) fn join_unsynced(hs: &mut HostedServer, key: [u8; 32], b: u8) -> Joiner {
    let client = hs.attach_test_remote_with_binding(Some([b; 32]));
    send_signed_join(&client, key, Some([b; 32]));
    hs.tick();
    let i = hs.server.players.len() - 1;
    let accept = hear(&client).accepts.into_iter().next().expect("Strict admits a first visit");
    assert_eq!(accept.inventory_mode, InventoryMode::Strict);
    Joiner { i, client, accept, seq: 0, applied: 0 }
}

impl Joiner {
    /// The client applies every window event sent so far.
    pub(super) fn apply_all(&mut self, hs: &mut HostedServer) {
        let sp = &mut hs.server.players[self.i];
        let sent = sp.window_events.last_number();
        crate::window_events::apply_through(sp, sent, hs.server.tick_counter);
        self.applied = sent;
    }
    pub(super) fn next(&mut self) -> ActionHeader {
        self.seq += 1;
        ActionHeader { session: self.accept.action_session, epoch: 0, seq: self.seq, life: self.accept.action_life, events_applied: self.applied }
    }
    pub(super) fn op(&mut self, hs: &mut HostedServer, op: protocol::WireWindowOp, digest: u32, client_ok: bool) -> ActionHeader {
        let action = self.send_op(op, digest, client_ok);
        hs.tick();
        action
    }
    /// Send one typed window op without ticking (run 5: several clients'
    /// actions land in ONE tick).
    pub(super) fn send_op(&mut self, op: protocol::WireWindowOp, digest: u32, client_ok: bool) -> ActionHeader {
        let action = self.next();
        let pkt = protocol::WindowOpPacket { op_seq: 0, op, digest, events_applied: self.applied, touched: Vec::new(), claims: Vec::new(), client_ok };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action, intent: ActionIntent::Window(pkt) }));
        action
    }
    /// The window's digest now, as the client (in lockstep) holds it.
    pub(super) fn digest(hs: &HostedServer, i: usize) -> u32 {
        let sp = &hs.server.players[i];
        crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station)
    }
    /// The client predicts `click` on its mirror (the view it applied) and
    /// its window: the digest it sends, and its own verdict.
    pub(super) fn predict(hs: &HostedServer, i: usize, click: &ContainerClick) -> (u32, bool) {
        let sp = &hs.server.players[i];
        let mut mirror = sp.container_sent.seen.clone().expect("a mirror is open").contents;
        let (mut inv, mut armour, mut cursor, mut grid) = (sp.inventory.clone(), sp.armour, sp.cursor.clone(), sp.craft_grid.clone());
        let ctx = crate::window::ClickCtx::new(false, sp.station, sp.player.eye_pos(), |_| crate::block::AIR).with_shared(true);
        let mut view = crate::window::WindowMut { inv: &mut inv, armour: &mut armour, cursor: &mut cursor, grid: &mut grid, container: Some(mirror.as_mut()) };
        let applied = crate::container_window::apply_container(&mut view, click, &ctx);
        (crate::window::digest(&view, sp.station), applied.result.ok())
    }
    /// The exact retained result row for `action` (a torn-down session's
    /// client never hears its fate; the server retains it).
    pub(super) fn row(&self, hs: &mut HostedServer, action: ActionHeader) -> Option<(ActionHeader, ActionFate)> {
        hs.server.players[self.i].action_results.as_mut().and_then(|r| r.repeat(action)).map(|r| (r.action, r.fate))
    }
    pub(super) fn click(&mut self, hs: &mut HostedServer, click: ContainerClick) -> ActionHeader {
        let (digest, ok) = Self::predict(hs, self.i, &click);
        self.op(hs, protocol::WireWindowOp::Container(click), digest, ok)
    }
}

/// What the client heard, with the ContainerOpened answers.
pub(super) fn heard_with_opens(client: &ChannelClientTransport) -> (Heard, Vec<protocol::ContainerOpenedPacket>) {
    let mut heard = Heard::default();
    let mut opens = Vec::new();
    while let Some(pkt) = client.try_recv_from_server() {
        match protocol::deserialize_header(&pkt) {
            Some((protocol::PacketType::ContainerOpened, p)) => opens.push(protocol::safe_deserialize(p).unwrap()),
            Some((protocol::PacketType::JoinReject, p)) => heard.rejects.push(protocol::safe_deserialize::<protocol::JoinRejectPacket>(p).unwrap().reason),
            Some((protocol::PacketType::ActionReceipt, p)) => heard.receipts.push(protocol::safe_deserialize(p).unwrap()),
            _ => {}
        }
    }
    (heard, opens)
}

pub(super) fn worn_pickaxe() -> ItemStack {
    let mut tool = crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Iron);
    tool.durability = 5;
    ItemStack { item: Item::Tool(tool), count: 1 }
}

/// A chest one block beside the shared spawn, in reach of every joiner.
pub(super) fn place_chest(hs: &mut HostedServer, i: usize, contents: &[(usize, ItemStack)]) -> [i32; 3] {
    let eye = hs.server.players[i].player.eye_pos();
    let cell = [eye.x.floor() as i32 + 1, eye.y.floor() as i32 - 1, eye.z.floor() as i32];
    hs.server.world.set_block(cell[0], cell[1], cell[2], crate::block::CHEST);
    let kind = ContainerKind::of_block(crate::block::CHEST).unwrap();
    crate::container_window::ensure_container(&mut hs.server.world, cell, kind);
    if let Some(crate::container_window::ContainerMut::Chest(chest)) = crate::container_window::container_at_mut(&mut hs.server.world, cell, kind) {
        for (slot, stack) in contents { chest.slots[*slot] = Some(stack.clone()); }
    }
    cell
}

pub(super) fn chest_slots(hs: &HostedServer, cell: [i32; 3]) -> Vec<protocol::WireSlot> {
    let kind = ContainerKind::of_block(crate::block::CHEST).unwrap();
    let c = crate::container_window::container_at(&hs.server.world, cell, kind).expect("the chest");
    (0..c.len()).map(|s| c.get(s).map(crate::inventory::stack_to_wire)).collect()
}

pub(super) fn ground(hs: &HostedServer) -> usize {
    hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().count()
}

/// Run 5 — every unit by exact identity over the joiners' 50 places (their
/// EFFECTIVE windows: every waiting event folded, a death or join sync
/// included), the chest at `cell` if it still stands, every grave and the
/// world's ground items.
pub(super) fn all_holdings(hs: &HostedServer, joiners: &[usize], cell: [i32; 3]) -> BTreeMap<String, u32> {
    let mut all: Vec<protocol::WireSlot> = joiners.iter().flat_map(|&i| {
        let w = crate::window_events::effective_window(&hs.server.players[i]);
        super::c3d_strict_tests::places(&w.inv, &w.armour, &w.cursor, &w.grid)
    }).collect();
    for entity in hs.server.world.block_entities.values() {
        if let Some(grave) = entity.as_grave() { all.extend(grave.slots.iter().map(|s| s.as_ref().map(crate::inventory::stack_to_wire))); }
    }
    let kind = ContainerKind::of_block(crate::block::CHEST).unwrap();
    if let Some(c) = crate::container_window::container_at(&hs.server.world, cell, kind) {
        all.extend((0..c.len()).map(|s| c.get(s).map(crate::inventory::stack_to_wire)));
    }
    all.extend(hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().map(|(_, e)| Some(crate::inventory::stack_to_wire(&e.stack))));
    holdings(all.iter())
}

/// Both joiners open `cell` and apply their views.
pub(super) fn open_both(hs: &mut HostedServer, joiners: [&mut Joiner; 2], cell: [i32; 3]) {
    for j in joiners {
        let digest = Joiner::digest(hs, j.i);
        let open = j.op(hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
        let (heard, opens) = heard_with_opens(&j.client);
        assert!(heard.rejects.is_empty() && opens.len() == 1 && opens[0].refused.is_none(), "open: {:?} {opens:?}", heard.rejects);
        assert_eq!(heard.receipts.last().map(|r| (r.action, r.fate)), Some((open, ActionFate::Accepted)));
        j.apply_all(hs);
    }
}

/// Every unit by exact identity over both joiners' 50 places and the chest.
fn world_holdings(hs: &HostedServer, joiners: &[usize], cell: [i32; 3]) -> BTreeMap<String, u32> {
    let mut all: Vec<protocol::WireSlot> = joiners.iter().flat_map(|&i| server_places(&hs.server.players[i])).collect();
    all.extend(chest_slots(hs, cell));
    holdings(all.iter())
}

/// Two Strict joiners on one chest through the real dispatch.
#[test]
fn c3d_strict_two_joiners_contest_one_chest_exactly_one_wins_each_unit() {
    let _g = crate::save::WorldsRootGuard::new("c3d-container-contest");
    let mut hs = start_room_test_server("c3d-container-contest", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut a = join_as(&mut hs, KEY, 17);
    let mut b = join_as(&mut hs, KEY_B, 18);
    hs.server.players[a.i].inventory.set_slot(9, Some(worn_pickaxe()));
    let stone = ItemStack::new_block(crate::block::STONE, 10);
    let cell = place_chest(&mut hs, a.i, &[(0, stone.clone())]);
    let before = world_holdings(&hs, &[a.i, b.i], cell);
    let ground_before = ground(&hs);

    // Both open it (typed OpenContainer): each gets ONE reserved numbered view.
    for j in [&mut a, &mut b] {
        let events = hs.server.players[j.i].window_events.last_number();
        let digest = Joiner::digest(&hs, j.i);
        let open = j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
        let (heard, opens) = heard_with_opens(&j.client);
        assert!(heard.rejects.is_empty(), "open: {:?}", heard.rejects);
        assert_eq!(opens.len(), 1, "one ContainerOpened");
        assert!(opens[0].refused.is_none() && opens[0].window_event == events + 1, "the opened view is numbered event {}", events + 1);
        assert_eq!((heard.receipts[0].action, heard.receipts[0].fate, heard.receipts[0].included_events), (open, ActionFate::Accepted, events + 1));
        assert_eq!(hs.server.players[j.i].open_container, Some(cell));
        j.apply_all(&mut hs);
    }

    // A takes the whole stack: Accepted, committed once on both sides.
    let take = a.click(&mut hs, ContainerClick::Withdraw { slot: 0, all: true });
    let (heard, _) = heard_with_opens(&a.client);
    assert!(heard.rejects.is_empty(), "A's take: {:?}", heard.rejects);
    assert_eq!(heard.receipts.last().map(|r| (r.action, r.fate)), Some((take, ActionFate::Accepted)));
    assert!(chest_slots(&hs, cell)[0].is_none(), "the real chest gave its stone up");
    assert_eq!(world_holdings(&hs, &[a.i, b.i], cell), before, "A's take conserves every unit");
    // B is pushed what A did, as a RESERVED numbered view it hasn't applied.
    assert!(matches!(hs.server.players[b.i].window_events.waiting_events().last(), Some(crate::window_events::WindowEvent::ContainerView(_))), "B's push is a numbered view");

    // B, still predicting on the view it applied (10 stone), takes too:
    // contested — NormalisedAcceptedNoop, nothing committed, torn down
    // with the plain reason (the correction until recovery is wired).
    let b_window = server_places(&hs.server.players[b.i]);
    let b_take = b.click(&mut hs, ContainerClick::Withdraw { slot: 0, all: true });
    let (heard, _) = heard_with_opens(&b.client);
    assert_eq!(b.row(&mut hs, b_take), Some((b_take, ActionFate::NormalisedAcceptedNoop)), "the loser's typed fate");
    assert!(heard.rejects.iter().any(|m| m == StrictTeardown::Unconfirmed.reason()), "torn down with the plain reason: {:?}", heard.rejects);
    assert_eq!(server_places(&hs.server.players[b.i]), b_window, "B's window: zero effect");
    assert_eq!(world_holdings(&hs, &[a.i, b.i], cell), before, "the contest conserves every unit");
    assert_eq!(ground(&hs), ground_before, "nothing spilled into the world");

    // A deposits the worn pickaxe: the chest holds it at durability 5.
    let deposit = a.click(&mut hs, ContainerClick::Deposit { slot: 9, all: false });
    let (heard, _) = heard_with_opens(&a.client);
    assert_eq!(heard.receipts.last().map(|r| (r.action, r.fate)), Some((deposit, ActionFate::Accepted)), "{:?}", heard.rejects);
    let worn = crate::inventory::stack_to_wire(&worn_pickaxe());
    assert!(chest_slots(&hs, cell).contains(&Some(worn)), "the very piece, durability and all");
    assert!(hs.server.players[a.i].inventory.slot(9).is_none());
    assert_eq!(world_holdings(&hs, &[a.i, b.i], cell), before, "every unit, body and durability conserved");
    assert!(hs.server.players[a.i].obligations_proven_zero(), "Strict creates no debt");
}

/// Configuration and the container lifecycle through typed actions; item 2:
/// a Shoot is admitted with an unapplied container view waiting; a deposit
/// of an item the server's copy doesn't hold is refused with zero effect.
#[test]
fn c3d_strict_config_open_shoot_close_and_fabricated_deposit() {
    let _g = crate::save::WorldsRootGuard::new("c3d-container-life");
    let mut hs = start_room_test_server("c3d-container-life", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut j = join_as(&mut hs, KEY, 17);
    let tool = arm(&mut hs.server.players[j.i]);
    let cell = place_chest(&mut hs, j.i, &[]);

    // SetAutoRefill and OpenPlayer: Accepted, applied to the server's copy.
    let on = !hs.server.players[j.i].inventory.auto_refill;
    hs.server.players[j.i].inventory.auto_refill = on;
    let digest = Joiner::digest(&hs, j.i);
    hs.server.players[j.i].inventory.auto_refill = !on;
    let set = j.op(&mut hs, protocol::WireWindowOp::SetAutoRefill { on }, digest, true);
    assert_eq!(hear(&j.client).receipts.last().map(|r| (r.action, r.fate)), Some((set, ActionFate::Accepted)));
    assert_eq!(hs.server.players[j.i].inventory.auto_refill, on);
    let digest = Joiner::digest(&hs, j.i);
    let open_player = j.op(&mut hs, protocol::WireWindowOp::OpenPlayer, digest, true);
    assert_eq!(hear(&j.client).receipts.last().map(|r| (r.action, r.fate)), Some((open_player, ActionFate::Accepted)));

    // Open the chest; leave its view UNAPPLIED (still waiting).
    let digest = Joiner::digest(&hs, j.i);
    j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    let waiting = hs.server.players[j.i].window_events.waiting();
    assert_eq!(waiting, 1, "the opened view waits");

    // Item 2 — a Shoot with that view waiting is admitted within budget.
    let projectiles = hs.server.ecs.query::<&crate::entity::ProjectileEntity>().iter().count();
    let shot = j.next();
    let item = Item::Tool(tool);
    let (held_kind, held_id) = crate::inventory::item_to_ref(&item).to_wire();
    let req = protocol::ItemActionPacket {
        seq: shot.seq as u32,
        action: protocol::ItemAction::Shoot { weapon: protocol::ShotWeapon::Bow, hotbar_slot: 0, held_kind, held_id, held_full: crate::inventory::item_to_wire_full(&item), yaw: 0.0, pitch: 0.0, charge: 20 },
        events_applied: shot.events_applied,
    };
    j.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action: shot, intent: ActionIntent::Item(req) }));
    hs.tick();
    let heard = hear(&j.client);
    assert!(heard.rejects.is_empty(), "the shot pauses nothing: {:?}", heard.rejects);
    assert_eq!(heard.receipts.last().map(|r| (r.action, r.fate)), Some((shot, ActionFate::Accepted)));
    assert_eq!(hs.server.ecs.query::<&crate::entity::ProjectileEntity>().iter().count(), projectiles + 1);
    assert_eq!(hs.server.players[j.i].window_events.waiting(), 3, "view, then Take and Wear");

    // The client applies them; Close (a click) closes the container view.
    j.apply_all(&mut hs);
    let close = {
        let sp = &hs.server.players[j.i];
        let mut w = (sp.inventory.clone(), sp.armour, sp.cursor.clone(), sp.craft_grid.clone());
        let ctx = crate::window::ClickCtx::new(false, sp.station, sp.player.eye_pos(), |_| crate::block::AIR);
        let mut view = crate::window::WindowMut { inv: &mut w.0, armour: &mut w.1, cursor: &mut w.2, grid: &mut w.3, container: None };
        let result = crate::window::apply(&mut view, &crate::window::WindowClick::Close, &ctx);
        (crate::window::digest(&view, crate::window::station_after(sp.station, &crate::window::WindowClick::Close, &result)), result.ok())
    };
    let closed = j.op(&mut hs, protocol::WireWindowOp::Click(crate::window::WindowClick::Close), close.0, close.1);
    assert_eq!(hear(&j.client).receipts.last().map(|r| (r.action, r.fate)), Some((closed, ActionFate::Accepted)));
    assert_eq!(hs.server.players[j.i].open_container, None, "the close closed the container view");

    // Reopen, apply, then claim a deposit of diamonds the copy doesn't hold.
    let digest = Joiner::digest(&hs, j.i);
    j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    j.apply_all(&mut hs);
    let _ = hear(&j.client);
    let window = server_places(&hs.server.players[j.i]);
    let chest = chest_slots(&hs, cell);
    let fake = {
        let sp = &mut hs.server.players[j.i];
        let real = sp.inventory.slot(20).cloned();
        sp.inventory.set_slot(20, Some(ItemStack { item: Item::Material(crate::item::MaterialId::Diamond), count: 3 }));
        let fake = Joiner::predict(&hs, j.i, &ContainerClick::Deposit { slot: 20, all: true });
        hs.server.players[j.i].inventory.set_slot(20, real);
        fake
    };
    let refused = j.op(&mut hs, protocol::WireWindowOp::Container(ContainerClick::Deposit { slot: 20, all: true }), fake.0, fake.1);
    let heard = hear(&j.client);
    assert_eq!(j.row(&mut hs, refused), Some((refused, ActionFate::Refused)), "no believed deposit");
    assert!(heard.rejects.iter().any(|m| m == StrictTeardown::Unconfirmed.reason()), "{:?}", heard.rejects);
    assert_eq!(server_places(&hs.server.players[j.i]), window, "zero effect on the window");
    assert_eq!(chest_slots(&hs, cell), chest, "zero effect on the real chest");
}

/// Staged raw source on the shared container: a deposit of a piece that a
/// pending (unapplied) Take already owns is not admitted — nothing commits
/// to the chest or the window, and the session pauses (fails closed).
#[test]
fn c3d_strict_container_deposit_of_a_source_a_pending_take_owns_commits_nothing() {
    let _g = crate::save::WorldsRootGuard::new("c3d-container-source");
    let mut hs = start_room_test_server("c3d-container-source", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut j = join_as(&mut hs, KEY, 17);
    hs.server.players[j.i].inventory.set_slot(9, Some(worn_pickaxe()));
    let cell = place_chest(&mut hs, j.i, &[]);
    let digest = Joiner::digest(&hs, j.i);
    j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    j.apply_all(&mut hs);
    let _ = hear(&j.client);
    let owed = crate::window_events::WindowEvent::Take { slot: 9, item: worn_pickaxe().item, n: 1, how: "test pending take" };
    crate::window_events::queue(&mut hs.server.players[j.i], owed, 0);
    let window = server_places(&hs.server.players[j.i]);
    let chest = chest_slots(&hs, cell);
    let deposit = j.click(&mut hs, ContainerClick::Deposit { slot: 9, all: false });
    let heard = hear(&j.client);
    assert_eq!(chest_slots(&hs, cell), chest, "the chest gained nothing");
    // The departure settles the pending Take (its authoritative fate): the
    // piece is paid ONCE, by the Take, never also deposited.
    let mut settled = window.clone();
    settled[9] = None;
    assert_eq!(server_places(&hs.server.players[j.i]), settled, "the Take paid the piece once; nothing else moved");
    assert_eq!(j.row(&mut hs, deposit), None, "uncertain source: no fate row");
    assert!(heard.rejects.iter().any(|m| m == StrictTeardown::FellBehind.reason()), "paused with the plain reason: {:?}", heard.rejects);
    assert_eq!(super::actions::teardown_detail::last(), "container source or pending fit is uncertain", "the internal reason");
}

/// Review L2 — edits on a Strict movement packet are never dropped
/// silently: the session fails closed with the plain reason, and the world
/// keeps its block.
#[test]
fn c3d_strict_movement_packet_edit_fails_closed_with_a_reason() {
    let _g = crate::save::WorldsRootGuard::new("c3d-movement-edit");
    let mut hs = start_room_test_server("c3d-movement-edit", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let j = join_as(&mut hs, KEY, 17);
    let eye = hs.server.players[j.i].player.eye_pos();
    let (x, y, z) = (eye.x.floor() as i32 + 1, eye.y.floor() as i32, eye.z.floor() as i32);
    let block = hs.server.world.get_block(x, y, z);
    let edit = protocol::BlockChange { x, y, z, new_block: crate::block::STONE, meta: 0 };
    let input = protocol::InputPacket { tick: 1, health: 20.0, events_applied: j.applied, block_changes: vec![edit], ..Default::default() };
    j.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    hs.tick();
    let heard = hear(&j.client);
    assert!(heard.rejects.iter().any(|m| m == StrictTeardown::Untyped.reason()), "fails closed with a reason: {:?}", heard.rejects);
    assert_eq!(hs.server.world.get_block(x, y, z), block, "the world keeps its block");
    assert!(hs.slot_is_free(j.i));
}

/// Run 5 (run-3 review L4) — both joiners' withdrawals land in ONE tick:
/// exactly one is Accepted, the other `NormalisedAcceptedNoop` with zero
/// effect; every unit conserved over both windows, the chest and the ground.
#[test]
fn c3d_strict_same_tick_contest_conserves_every_unit() {
    let _g = crate::save::WorldsRootGuard::new("c3d-contest-same-tick");
    let mut hs = start_room_test_server("c3d-contest-same-tick", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut a = join_as(&mut hs, KEY, 17);
    let mut b = join_as(&mut hs, KEY_B, 18);
    let cell = place_chest(&mut hs, a.i, &[(0, ItemStack::new_block(crate::block::STONE, 10))]);
    open_both(&mut hs, [&mut a, &mut b], cell);
    let before = all_holdings(&hs, &[a.i, b.i], cell);
    let windows = [server_places(&hs.server.players[a.i]), server_places(&hs.server.players[b.i])];
    let click = ContainerClick::Withdraw { slot: 0, all: true };
    let (da, oka) = Joiner::predict(&hs, a.i, &click);
    let (db, okb) = Joiner::predict(&hs, b.i, &click);
    let ta = a.send_op(protocol::WireWindowOp::Container(click.clone()), da, oka);
    let tb = b.send_op(protocol::WireWindowOp::Container(click), db, okb);
    hs.tick();
    let fates = [a.row(&mut hs, ta).map(|r| r.1), b.row(&mut hs, tb).map(|r| r.1)];
    let won = fates.iter().position(|f| *f == Some(ActionFate::Accepted)).expect("one withdrawal wins");
    assert_eq!(fates[1 - won], Some(ActionFate::NormalisedAcceptedNoop), "the other is the contest's typed fate: {fates:?}");
    let loser = [a.i, b.i][1 - won];
    assert_eq!(server_places(&hs.server.players[loser]), windows[1 - won], "the loser's window: zero effect");
    assert!(chest_slots(&hs, cell)[0].is_none(), "the chest gave its stone up once");
    assert_eq!(all_holdings(&hs, &[a.i, b.i], cell), before, "the same-tick contest conserves every unit");
}

/// Run 5 (run-3 review L4) — a deposit contest with the winner's push in
/// flight: B deposits into the slot it saw empty, which A's deposit filled
/// a tick earlier (B hasn't applied A's push). Zero effect for B; every
/// unit, durability included, conserved.
#[test]
fn c3d_strict_deposit_contest_with_the_push_in_flight_conserves_every_unit() {
    let _g = crate::save::WorldsRootGuard::new("c3d-contest-deposit");
    let mut hs = start_room_test_server("c3d-contest-deposit", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut a = join_as(&mut hs, KEY, 17);
    let mut b = join_as(&mut hs, KEY_B, 18);
    hs.server.players[a.i].inventory.set_slot(9, Some(ItemStack::new_block(crate::block::OAK_LOG, 7)));
    hs.server.players[b.i].inventory.set_slot(9, Some(worn_pickaxe()));
    // Every chest slot full but the last.
    let empty = place_chest(&mut hs, a.i, &[]);
    let slots = chest_slots(&hs, empty).len();
    let full: Vec<(usize, ItemStack)> = (0..slots - 1).map(|s| (s, ItemStack::new_block(crate::block::DIRT, 64))).collect();
    let cell = place_chest(&mut hs, a.i, &full);
    open_both(&mut hs, [&mut a, &mut b], cell);
    let before = all_holdings(&hs, &[a.i, b.i], cell);
    let deposit = a.click(&mut hs, ContainerClick::Deposit { slot: 9, all: true });
    assert_eq!(a.row(&mut hs, deposit).map(|r| r.1), Some(ActionFate::Accepted));
    assert!(matches!(hs.server.players[b.i].window_events.waiting_events().last(), Some(crate::window_events::WindowEvent::ContainerView(_))), "A's push to B is in flight");
    let b_window = server_places(&hs.server.players[b.i]);
    let chest = chest_slots(&hs, cell);
    let contested = b.click(&mut hs, ContainerClick::Deposit { slot: 9, all: false });
    let (heard, _) = heard_with_opens(&b.client);
    assert_eq!(b.row(&mut hs, contested), Some((contested, ActionFate::NormalisedAcceptedNoop)), "the deposit contest's typed fate");
    assert!(heard.rejects.iter().any(|m| m == StrictTeardown::Unconfirmed.reason()), "the correction follows: {:?}", heard.rejects);
    assert_eq!(server_places(&hs.server.players[b.i]), b_window, "B keeps its pickaxe: zero effect");
    assert_eq!(chest_slots(&hs, cell), chest, "the chest holds only A's deposit");
    assert_eq!(all_holdings(&hs, &[a.i, b.i], cell), before, "the deposit contest conserves every unit and durability");
}

/// Run 5 (run-3 review L4) — the chest is broken between A's commit and
/// B's click: B's click finds no container (`Refused`, nothing committed);
/// what the chest held spilled once; every unit conserved over both
/// windows and the ground.
#[test]
fn c3d_strict_container_broken_mid_contest_commits_nothing_and_conserves() {
    let _g = crate::save::WorldsRootGuard::new("c3d-contest-broken");
    let mut hs = start_room_test_server("c3d-contest-broken", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut a = join_as(&mut hs, KEY, 17);
    let mut b = join_as(&mut hs, KEY_B, 18);
    let cell = place_chest(&mut hs, a.i, &[(0, ItemStack::new_block(crate::block::STONE, 10)), (1, worn_pickaxe())]);
    open_both(&mut hs, [&mut a, &mut b], cell);
    let before = all_holdings(&hs, &[a.i, b.i], cell);
    let take = a.click(&mut hs, ContainerClick::Withdraw { slot: 0, all: true });
    assert_eq!(a.row(&mut hs, take).map(|r| r.1), Some(ActionFate::Accepted));
    assert_eq!(all_holdings(&hs, &[a.i, b.i], cell), before, "A's take conserves");
    // Broken the way the server breaks one: what it held spills once.
    let ground_before = ground(&hs);
    hs.spill_container_on_change((cell[0], cell[1], cell[2]), crate::block::CHEST, crate::block::AIR, true);
    hs.server.world.set_block(cell[0], cell[1], cell[2], crate::block::AIR);
    assert_eq!(ground(&hs), ground_before + 1, "the pickaxe spilled once");
    let b_window = server_places(&hs.server.players[b.i]);
    let click = ContainerClick::Withdraw { slot: 1, all: true };
    let (digest, ok) = Joiner::predict(&hs, b.i, &click);
    let refused = b.op(&mut hs, protocol::WireWindowOp::Container(click), digest, ok);
    assert_eq!(b.row(&mut hs, refused), Some((refused, ActionFate::Refused)), "no container: Refused");
    assert_eq!(server_places(&hs.server.players[b.i]), b_window, "B: zero effect");
    assert_eq!(ground(&hs), ground_before + 1, "nothing spilled twice");
    assert_eq!(all_holdings(&hs, &[a.i, b.i], cell), before, "every unit and durability conserved over both windows and the ground");
}

/// Run 11 (run-7 item 4) — a real Strict open past the event budget (1024
/// numbered events its client hasn't applied): the typed refusal
/// `OpenRefusal::TooLarge` with an Accepted row, never torn down, nothing
/// opened or numbered, and no block entity conjured for a lazy chest (the
/// fix: `ensure_container` runs after the reservation). Retained green:
/// once the client applies what waits, the same open is served and only
/// then creates the entity.
#[test]
fn c3d_strict_open_past_the_event_budget_is_too_large_and_conjures_nothing() {
    let _g = crate::save::WorldsRootGuard::new("c3d-container-too-large");
    let mut hs = start_room_test_server("c3d-container-too-large", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut j = join_as(&mut hs, KEY, 17);
    let eye = hs.server.players[j.i].player.eye_pos();
    let cell = [eye.x.floor() as i32 + 1, eye.y.floor() as i32 - 1, eye.z.floor() as i32];
    let pos = (cell[0], cell[1], cell[2]);
    hs.server.world.set_block(cell[0], cell[1], cell[2], crate::block::CHEST);
    assert!(hs.server.world.chest_at(pos).is_none(), "a lazy chest: the block stands, no entity yet");
    let sp = &mut hs.server.players[j.i];
    for _ in 0..crate::action_journal::MAX_EVENT_ENTRIES {
        crate::window_events::queue(sp, crate::window_events::WindowEvent::WearArmour { hits: 1 }, 0);
    }
    let waiting = hs.server.players[j.i].window_events.waiting();
    let digest = Joiner::digest(&hs, j.i);
    let open = j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    let (heard, opens) = heard_with_opens(&j.client);
    assert!(heard.rejects.is_empty(), "never torn down: {:?}", heard.rejects);
    assert_eq!(opens.iter().map(|o| o.refused).collect::<Vec<_>>(), vec![Some(protocol::OpenRefusal::TooLarge)], "the typed refusal");
    assert_eq!(heard.receipts.last().map(|r| (r.action, r.fate)), Some((open, ActionFate::Accepted)), "an Accepted row");
    let sp = &hs.server.players[j.i];
    assert_eq!((sp.open_container, sp.window_events.waiting()), (None, waiting), "nothing opened, no view numbered");
    assert!(hs.server.world.chest_at(pos).is_none(), "a refused open conjures no chest entity");
    j.apply_all(&mut hs);
    let digest = Joiner::digest(&hs, j.i);
    let open = j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    let (heard, opens) = heard_with_opens(&j.client);
    assert!(heard.rejects.is_empty() && opens.len() == 1 && opens[0].refused.is_none(), "served once the budget is free: {:?} {opens:?}", heard.rejects);
    assert_eq!(heard.receipts.last().map(|r| (r.action, r.fate)), Some((open, ActionFate::Accepted)));
    assert_eq!(hs.server.players[j.i].open_container, Some(cell));
    assert!(hs.server.world.chest_at(pos).is_some(), "a served open creates the entity");
}

/// Run 11 (run-7 item 4) — pins the push path's Plan strip: a host puts a
/// Plan whose body is past the 2 MiB event budget into a chest a Strict
/// joiner has open; the push's numbered view holds the body-less
/// placeholder (`strip_plan_body`), so it is reserved and sent, never torn
/// down, and the real chest keeps the real body.
#[test]
fn c3d_strict_push_of_a_large_plan_counts_its_placeholder() {
    let _g = crate::save::WorldsRootGuard::new("c3d-container-push-plan");
    let mut hs = start_room_test_server("c3d-container-push-plan", 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let mut j = join_as(&mut hs, KEY, 17);
    let cell = place_chest(&mut hs, j.i, &[(0, ItemStack::new_block(crate::block::STONE, 1))]);
    let digest = Joiner::digest(&hs, j.i);
    let open = j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    let (heard, opens) = heard_with_opens(&j.client);
    assert!(heard.rejects.is_empty() && opens.len() == 1 && opens[0].refused.is_none(), "opened: {:?} {opens:?}", heard.rejects);
    assert_eq!(heard.receipts.last().map(|r| (r.action, r.fate)), Some((open, ActionFate::Accepted)));
    j.apply_all(&mut hs);
    let mut body = crate::plan::PlanData::debug_3x3_stone();
    let cell0 = body.cells[0].clone();
    body.cells = vec![cell0; 3 * 1024 * 1024 / std::mem::size_of_val(&body.cells[0]).max(1)];
    assert!(crate::body_residency::allocated_body_bytes(&body).unwrap() > crate::action_journal::MAX_RETAINED_BYTES, "a body past the event budget");
    let marker = crate::plan::marker(&body);
    let kind = ContainerKind::of_block(crate::block::CHEST).unwrap();
    if let Some(crate::container_window::ContainerMut::Chest(chest)) = crate::container_window::container_at_mut(&mut hs.server.world, cell, kind) {
        chest.slots[1] = Some(ItemStack { item: Item::Plan(body), count: 1 });
    }
    let waiting = hs.server.players[j.i].window_events.waiting();
    hs.tick();
    let (mut sets, mut rejects) = (0, Vec::new());
    while let Some(pkt) = j.client.try_recv_from_server() {
        match protocol::deserialize_header(&pkt) {
            Some((protocol::PacketType::WindowSlotSet, _)) => sets += 1,
            Some((protocol::PacketType::JoinReject, p)) => rejects.push(protocol::safe_deserialize::<protocol::JoinRejectPacket>(p).unwrap().reason),
            _ => {}
        }
    }
    assert!(rejects.is_empty(), "the push is never torn down: {rejects:?}");
    assert_eq!(sets, 1, "the push was sent");
    assert_eq!(hs.server.players[j.i].window_events.waiting(), waiting + 1, "its view was numbered");
    let real = crate::container_window::container_at(&hs.server.world, cell, kind).unwrap();
    let Some(ItemStack { item: Item::Plan(kept), .. }) = real.get(1) else { panic!("the real chest keeps the Plan") };
    assert_eq!(crate::plan::marker(kept), marker, "the real body, untouched");
}
