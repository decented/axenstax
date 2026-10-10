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
use crate::container_window::{ContainerClick, ContainerData, ContainerKind};
use crate::item::{Item, ItemStack};
use crate::strict_admission::StrictTeardown;
use crate::transport::ClientTransport as _;
use std::collections::BTreeMap;

const KEY_B: [u8; 32] = [67; 32];

/// One joined Strict client: its connection, the action identity the
/// server issued, its next sequence and the window events it applied.
struct Joiner {
    i: usize,
    client: ChannelClientTransport,
    accept: protocol::JoinAcceptPacket,
    seq: u64,
    applied: u32,
}

fn join_as(hs: &mut HostedServer, key: [u8; 32], b: u8) -> Joiner {
    let client = hs.attach_test_remote_with_binding(Some([b; 32]));
    send_signed_join(&client, key, Some([b; 32]));
    hs.tick();
    let i = hs.server.players.len() - 1;
    let accept = hear(&client).accepts.into_iter().next().expect("Strict admits a first visit");
    assert_eq!(accept.inventory_mode, InventoryMode::Strict);
    let mut j = Joiner { i, client, accept, seq: 0, applied: 0 };
    j.apply_all(hs);
    j
}

impl Joiner {
    /// The client applies every window event sent so far.
    fn apply_all(&mut self, hs: &mut HostedServer) {
        let sp = &mut hs.server.players[self.i];
        let sent = sp.window_events.last_number();
        crate::window_events::apply_through(sp, sent, hs.server.tick_counter);
        self.applied = sent;
    }
    fn next(&mut self) -> ActionHeader {
        self.seq += 1;
        ActionHeader { session: self.accept.action_session, epoch: 0, seq: self.seq, life: self.accept.action_life, events_applied: self.applied }
    }
    fn op(&mut self, hs: &mut HostedServer, op: protocol::WireWindowOp, digest: u32, client_ok: bool) -> ActionHeader {
        let action = self.next();
        let pkt = protocol::WindowOpPacket { op_seq: 0, op, digest, events_applied: self.applied, touched: Vec::new(), claims: Vec::new(), client_ok };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action, intent: ActionIntent::Window(pkt) }));
        hs.tick();
        action
    }
    /// The window's digest now, as the client (in lockstep) holds it.
    fn digest(hs: &HostedServer, i: usize) -> u32 {
        let sp = &hs.server.players[i];
        crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station)
    }
    /// The client predicts `click` on its mirror (the view it applied) and
    /// its window: the digest it sends, and its own verdict.
    fn predict(hs: &HostedServer, i: usize, click: &ContainerClick) -> (u32, bool) {
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
    fn row(&self, hs: &mut HostedServer, action: ActionHeader) -> Option<(ActionHeader, ActionFate)> {
        hs.server.players[self.i].action_results.as_mut().and_then(|r| r.repeat(action)).map(|r| (r.action, r.fate))
    }
    fn click(&mut self, hs: &mut HostedServer, click: ContainerClick) -> ActionHeader {
        let (digest, ok) = Self::predict(hs, self.i, &click);
        self.op(hs, protocol::WireWindowOp::Container(click), digest, ok)
    }
}

/// What the client heard, with the ContainerOpened answers.
fn heard_with_opens(client: &ChannelClientTransport) -> (Heard, Vec<protocol::ContainerOpenedPacket>) {
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

fn worn_pickaxe() -> ItemStack {
    let mut tool = crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Iron);
    tool.durability = 5;
    ItemStack { item: Item::Tool(tool), count: 1 }
}

/// A chest one block beside the shared spawn, in reach of every joiner.
fn place_chest(hs: &mut HostedServer, i: usize, contents: &[(usize, ItemStack)]) -> [i32; 3] {
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

fn chest_slots(hs: &HostedServer, cell: [i32; 3]) -> Vec<protocol::WireSlot> {
    let kind = ContainerKind::of_block(crate::block::CHEST).unwrap();
    let c = crate::container_window::container_at(&hs.server.world, cell, kind).expect("the chest");
    (0..c.len()).map(|s| c.get(s).map(crate::inventory::stack_to_wire)).collect()
}

fn ground(hs: &HostedServer) -> usize {
    hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().count()
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
    hs.inventory_mode = InventoryMode::Strict;
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
    hs.inventory_mode = InventoryMode::Strict;
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
    hs.inventory_mode = InventoryMode::Strict;
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
}

/// Review L2 — edits on a Strict movement packet are never dropped
/// silently: the session fails closed with the plain reason, and the world
/// keeps its block.
#[test]
fn c3d_strict_movement_packet_edit_fails_closed_with_a_reason() {
    let _g = crate::save::WorldsRootGuard::new("c3d-movement-edit");
    let mut hs = start_room_test_server("c3d-movement-edit", 0);
    hs.inventory_mode = InventoryMode::Strict;
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
