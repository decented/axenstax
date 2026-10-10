//! C3d — Strict Click and Shoot through the real `HostedServer` dispatch
//! (`process_typed_actions` → `serve_strict_window` / `serve_strict_shot`),
//! Strict selected internally on a real signed joiner. Conservation is
//! asserted per place and per identity (Plan marker, durability), not as
//! totals; every refusal or cap is asserted to have zero external effect;
//! after every Strict commit the effective fold proves zero (Decision 10).

use super::c3d_tests::{hear, join};
use super::tests::start_room_test_server;
use super::*;
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::action_wire::{ActionIntent, ActionPacket, ActionReceipt};
use crate::item::{Item, ItemStack};
use crate::transport::ClientTransport as _;
use crate::window::WindowClick;
use std::collections::BTreeMap;

/// A Strict world with one admitted real joiner whose client has applied
/// every window event so far (the join's sync among them).
struct Rig {
    _g: crate::save::WorldsRootGuard,
    hs: HostedServer,
    i: usize,
    client: ChannelClientTransport,
    accept: protocol::JoinAcceptPacket,
    seq: u64,
    /// The highest window event the client has applied (what it reports).
    applied: u32,
}

fn strict_rig(tag: &str) -> Rig {
    let g = crate::save::WorldsRootGuard::new(&format!("c3d-strict-{tag}"));
    let mut hs = start_room_test_server(&format!("c3d-strict-{tag}"), 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    let (i, client, heard) = join(&mut hs, 17);
    let accept = heard.accepts.into_iter().next().expect("Strict admits a first visit");
    assert_eq!(accept.inventory_mode, InventoryMode::Strict);
    let sp = &mut hs.server.players[i];
    let sent = sp.window_events.last_number();
    crate::window_events::apply_through(sp, sent, 0);
    assert_eq!(sp.window_events.waiting(), 0, "the join's sync is applied");
    Rig { _g: g, hs, i, client, accept, seq: 0, applied: sent }
}

impl Rig {
    fn sp(&self) -> &crate::server::ServerPlayer { &self.hs.server.players[self.i] }
    fn next(&mut self) -> ActionHeader {
        self.seq += 1;
        ActionHeader { session: self.accept.action_session, epoch: 0, seq: self.seq, life: self.accept.action_life, events_applied: self.applied }
    }
    fn send(&mut self, action: ActionHeader, intent: ActionIntent) {
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action, intent }));
        self.hs.tick();
    }
    fn through(&self) -> u64 { self.sp().action_frontier.as_ref().unwrap().through() }
    /// The exact retained result row for `action`, if any.
    fn row(&mut self, action: ActionHeader) -> Option<ActionReceipt> {
        self.hs.server.players[self.i].action_results.as_mut().unwrap().repeat(action)
    }
    fn projectiles(&self) -> usize {
        self.hs.server.ecs.query::<&crate::entity::ProjectileEntity>().iter().count()
    }
    /// Fill the unacknowledged result history to its cap with terminal
    /// CancelledNoops (as a client that never acknowledges would).
    fn fill_results(&mut self) {
        let sp = &mut self.hs.server.players[self.i];
        let frontier = sp.action_frontier.as_mut().unwrap();
        let results = sp.action_results.as_mut().unwrap();
        for seq in frontier.through() + 1..=crate::action_journal::MAX_ACTIONS as u64 {
            let action = frontier.header(seq, 0);
            frontier.terminal(action, ActionFate::CancelledNoop).unwrap();
            let prepared = results.prepare(ActionReceipt { action, fate: ActionFate::CancelledNoop, included_events: 0 }, Box::new([])).unwrap();
            results.commit(prepared);
        }
        results.take_unsent();
        self.seq = crate::action_journal::MAX_ACTIONS as u64;
    }
}

/// The client's own copy of its window, predicted by the shared rule.
#[derive(Clone)]
struct ClientWindow {
    inv: crate::inventory::Inventory,
    armour: [Option<crate::armour::ArmourItem>; 4],
    cursor: Option<ItemStack>,
    grid: crate::window::CraftGrid,
    station: crate::window::Station,
}

impl ClientWindow {
    fn of(sp: &crate::server::ServerPlayer) -> Self {
        Self { inv: sp.inventory.clone(), armour: sp.armour, cursor: sp.cursor.clone(), grid: sp.craft_grid.clone(), station: sp.station }
    }
    /// Apply `click` as the client does; its digest and verdict.
    fn click(&mut self, hs: &HostedServer, eye: glam::Vec3, click: &WindowClick) -> (u32, bool) {
        let ctx = crate::window::ClickCtx::new(hs.server.play_mode.is_creative(), self.station, eye, |c| hs.server.world.get_block(c[0], c[1], c[2]));
        let mut view = crate::window::WindowMut { inv: &mut self.inv, armour: &mut self.armour, cursor: &mut self.cursor, grid: &mut self.grid, container: None };
        let result = crate::window::apply(&mut view, click, &ctx);
        self.station = crate::window::station_after(self.station, click, &result);
        (crate::window::digest(&view, self.station), result.ok())
    }
    fn places(&self) -> Vec<protocol::WireSlot> {
        places(&self.inv, &self.armour, &self.cursor, &self.grid)
    }
}

/// All 50 places at full fidelity (a Plan by marker, a tool by durability).
pub(super) fn places(
    inv: &crate::inventory::Inventory,
    armour: &[Option<crate::armour::ArmourItem>; 4],
    cursor: &Option<ItemStack>,
    grid: &crate::window::CraftGrid,
) -> Vec<protocol::WireSlot> {
    let mut out: Vec<protocol::WireSlot> = (0..36).map(|s| inv.slot(s).map(crate::inventory::stack_to_wire)).collect();
    out.extend(armour.iter().map(|a| a.map(|a| crate::inventory::stack_to_wire(&ItemStack { item: Item::Armour(a), count: 1 }))));
    out.push(cursor.as_ref().map(crate::inventory::stack_to_wire));
    out.extend(grid.iter().flatten().map(|c| c.as_ref().map(crate::inventory::stack_to_wire)));
    out
}

pub(super) fn server_places(sp: &crate::server::ServerPlayer) -> Vec<protocol::WireSlot> {
    places(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid)
}

/// Units held, by exact identity (kind, id, full item: durability, Plan
/// marker and developed state), across every place given.
pub(super) fn holdings<'a>(slots: impl IntoIterator<Item = &'a protocol::WireSlot>) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for stack in slots.into_iter().flatten() {
        *out.entry(format!("{}/{}/{:?}", stack.item_kind, stack.item_id, stack.full_item)).or_insert(0) += u32::from(stack.count);
    }
    out
}

fn worn_pickaxe() -> Item {
    let mut tool = crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Iron);
    tool.durability = 5;
    Item::Tool(tool)
}

pub(super) fn bow(durability: u16) -> crate::crafting::Tool {
    let mut tool = crate::crafting::Tool::new(crate::crafting::ToolType::Bow, crate::crafting::ToolMaterial::Wood);
    tool.durability = durability;
    tool
}

/// The window every click test starts from: a worn pickaxe, a Plan (the
/// server copy's body-less marker), stone, and a chestplate to equip.
fn stock(sp: &mut crate::server::ServerPlayer) {
    let plan = crate::plan::PlanData::marker_placeholder([9; 32], true);
    sp.inventory.set_slot(0, Some(ItemStack { item: worn_pickaxe(), count: 1 }));
    sp.inventory.set_slot(1, Some(ItemStack::new_block(crate::block::STONE, 10)));
    sp.inventory.set_slot(9, Some(ItemStack { item: Item::Plan(plan), count: 1 }));
    let chestplate = crate::armour::ArmourItem { slot: crate::armour::ArmourSlot::Chestplate, material: crate::armour::ArmourMaterial::Iron, durability: 77 };
    sp.inventory.set_slot(10, Some(ItemStack { item: Item::Armour(chestplate), count: 1 }));
}

fn click_packet(r: &mut Rig, cw: &mut ClientWindow, click: WindowClick, wrong_digest: bool) -> ActionHeader {
    let action = r.next();
    let eye = r.sp().player.eye_pos();
    let (digest, ok) = cw.click(&r.hs, eye, &click);
    let op = protocol::WindowOpPacket {
        op_seq: action.seq as u32,
        op: protocol::WireWindowOp::Click(click),
        digest: if wrong_digest { digest ^ 1 } else { digest },
        events_applied: action.events_applied,
        touched: Vec::new(),
        claims: Vec::new(),
        client_ok: ok,
    };
    r.send(action, ActionIntent::Window(op));
    action
}

/// Moves, swaps, a split, an armour equip and a close through the real
/// Strict dispatch: each is Accepted once, the server's raw window becomes
/// exactly the client's prediction, every unit keeps its identity (the
/// Plan its marker, the pickaxe and chestplate their durability), and the
/// fold proves zero after every commit.
#[test]
fn c3d_strict_click_through_real_dispatch_conserves_places_bodies_and_durability() {
    let mut r = strict_rig("click");
    stock(&mut r.hs.server.players[r.i]);
    let mut cw = ClientWindow::of(r.sp());
    let start = holdings(&server_places(r.sp()));
    let clicks = [
        WindowClick::Slot { slot: 0, right: false },  // pick up the worn pickaxe
        WindowClick::Slot { slot: 5, right: false },  // put it in slot 5
        WindowClick::Slot { slot: 9, right: false },  // pick up the Plan
        WindowClick::Grid { row: 0, col: 0, right: false }, // into the grid
        WindowClick::Slot { slot: 1, right: true },   // half the stone
        WindowClick::Grid { row: 1, col: 1, right: true }, // one stone in the grid
        WindowClick::Slot { slot: 2, right: false },  // the rest into slot 2
        WindowClick::Slot { slot: 10, right: false }, // pick up the chestplate
        WindowClick::Armour { slot: crate::armour::ArmourSlot::Chestplate as usize }, // equip it
        WindowClick::Close,                            // grid and cursor return to the bag
    ];
    for (n, click) in clicks.into_iter().enumerate() {
        let action = click_packet(&mut r, &mut cw, click.clone(), false);
        let heard = hear(&r.client);
        assert!(heard.rejects.is_empty(), "click {n} {click:?}: no teardown, heard {:?}", heard.rejects);
        assert_eq!(heard.receipts.len(), 1, "click {n} {click:?}: exactly one receipt");
        assert_eq!((heard.receipts[0].action, heard.receipts[0].fate), (action, ActionFate::Accepted), "click {n}");
        assert_eq!(r.through(), action.seq, "click {n}: executed once, terminal");
        let sp = r.sp();
        assert_eq!(server_places(sp), cw.places(), "click {n} {click:?}: the raw window is exactly the prediction");
        assert_eq!(sp.station, cw.station);
        assert_eq!(holdings(&server_places(sp)), start, "click {n} {click:?}: every identity conserved");
        assert_eq!(sp.window_events.waiting(), 0, "click {n}: a click numbers no event");
        assert!(sp.window_events.debt.is_empty() && sp.obligations_proven_zero(), "click {n}: Strict creates no debt");
    }
    // The pickaxe kept its wear, the Plan its marker, the chestplate is worn.
    let sp = r.sp();
    assert_eq!(sp.inventory.slot(5).map(|s| s.item.clone()), Some(worn_pickaxe()));
    assert!(sp.armour[crate::armour::ArmourSlot::Chestplate as usize].is_some_and(|a| a.durability == 77));
    assert!(sp.cursor.is_none() && sp.craft_grid.iter().flatten().all(Option::is_none), "Close emptied grid and cursor");
    assert!((0..36).any(|s| matches!(sp.inventory.slot(s), Some(ItemStack { item: Item::Plan(p), .. }) if crate::plan::marker(p) == [9; 32])));
    assert!(crate::joiner_save::JoinerSave::capture(sp, r.hs.server.run_id).obligations_proven_zero());
}

/// A click whose digest the server's raw transition doesn't reproduce is
/// Refused at its seq: zero effect on the server copy, no event, no
/// accepted row, the typed fate retained; until recovery is wired the
/// connection is torn down with a reason (fail closed).
#[test]
fn c3d_strict_refused_click_has_zero_effect_and_its_typed_fate() {
    let mut r = strict_rig("refused");
    stock(&mut r.hs.server.players[r.i]);
    let mut cw = ClientWindow::of(r.sp());
    let before = server_places(r.sp());
    let events = r.sp().window_events.last_number();
    let action = click_packet(&mut r, &mut cw, WindowClick::Slot { slot: 0, right: false }, true);
    let heard = hear(&r.client);
    assert_eq!(server_places(r.sp()), before, "zero effect on the server copy");
    assert_eq!(r.sp().window_events.last_number(), events, "no event numbered");
    assert_eq!(r.sp().window_events.waiting(), 0);
    assert_eq!(r.through(), action.seq, "the refusal is terminal at its seq");
    assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Refused), "its typed fate");
    assert!(heard.receipts.is_empty(), "no Accepted receipt reaches the client");
    assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::Unconfirmed.reason()), "teardown reason: {:?}", heard.rejects);
    assert!(r.hs.slot_is_free(r.i));
    assert!(r.hs.server.load_joiner(r.i).expect("departure saved").obligations_proven_zero());
}

/// The result row is reserved BEFORE the raw commit: with the
/// unacknowledged result history full, a valid click commits nothing.
#[test]
fn c3d_strict_click_at_full_result_history_commits_nothing() {
    let mut r = strict_rig("click-results");
    stock(&mut r.hs.server.players[r.i]);
    r.fill_results();
    let mut cw = ClientWindow::of(r.sp());
    let before = server_places(r.sp());
    let action = click_packet(&mut r, &mut cw, WindowClick::Slot { slot: 0, right: false }, false);
    let heard = hear(&r.client);
    assert_eq!(server_places(r.sp()), before, "no raw commit without a reserved result row");
    assert_eq!(r.through(), action.seq - 1, "the action is not terminal");
    assert_eq!(r.row(action), None, "no row was invented for it");
    assert!(heard.receipts.is_empty());
    assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::FellBehind.reason()), "teardown reason: {:?}", heard.rejects);
}

fn shoot(r: &mut Rig, tool: crate::crafting::Tool) -> ActionHeader {
    let action = r.next();
    let item = Item::Tool(tool);
    let (held_kind, held_id) = crate::inventory::item_to_ref(&item).to_wire();
    let held_full = crate::inventory::item_to_wire_full(&item);
    let req = protocol::ItemActionPacket {
        seq: action.seq as u32,
        action: protocol::ItemAction::Shoot { weapon: protocol::ShotWeapon::Bow, hotbar_slot: 0, held_kind, held_id, held_full, yaw: 0.0, pitch: 0.0, charge: 20 },
        events_applied: action.events_applied,
    };
    r.send(action, ActionIntent::Item(req));
    action
}

pub(super) fn arm(sp: &mut crate::server::ServerPlayer) -> crate::crafting::Tool {
    let tool = bow(30);
    sp.inventory.set_slot(0, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
    sp.inventory.set_slot(1, Some(ItemStack { item: Item::Material(crate::item::MaterialId::Arrow), count: 5 }));
    tool
}

fn outcomes(client: &ChannelClientTransport) -> (Vec<protocol::ItemActionOutcomePacket>, super::c3d_tests::Heard) {
    let mut out = Vec::new();
    let mut heard = super::c3d_tests::Heard::default();
    while let Some(pkt) = client.try_recv_from_server() {
        match protocol::deserialize_header(&pkt) {
            Some((protocol::PacketType::ItemActionOutcome, p)) => out.push(protocol::safe_deserialize(p).unwrap()),
            Some((protocol::PacketType::JoinReject, p)) => heard.rejects.push(protocol::safe_deserialize::<protocol::JoinRejectPacket>(p).unwrap().reason),
            Some((protocol::PacketType::ActionReceipt, p)) => heard.receipts.push(protocol::safe_deserialize(p).unwrap()),
            _ => {}
        }
    }
    (out, heard)
}

/// One Strict shot: one projectile, exactly two numbered events (the
/// arrow's Take, then the bow's Wear), one Accepted result row naming the
/// second event; raw R is untouched until the client applies them, and
/// then the arrow and one point of wear are paid exactly once.
#[test]
fn c3d_strict_shot_through_real_dispatch_spawns_one_projectile_and_two_events() {
    let mut r = strict_rig("shot");
    let tool = arm(&mut r.hs.server.players[r.i]);
    let raw = server_places(r.sp());
    let events = r.sp().window_events.last_number();
    let projectiles = r.projectiles();
    let action = shoot(&mut r, tool);
    let (outcome, heard) = outcomes(&r.client);
    assert!(heard.rejects.is_empty(), "no teardown: {:?}", heard.rejects);
    assert_eq!(r.projectiles(), projectiles + 1, "exactly one projectile");
    let sp = r.sp();
    assert_eq!(sp.window_events.last_number(), events + 2, "exactly two numbered events");
    let waiting: Vec<_> = sp.window_events.waiting_events().cloned().collect();
    assert!(matches!(
        &waiting[..],
        [crate::window_events::WindowEvent::Take { slot: 0, item: Item::Material(crate::item::MaterialId::Arrow), n: 1, .. },
         crate::window_events::WindowEvent::WearWeapon { slot: 0, tool: t }] if t == &tool
    ), "Take then Wear: {waiting:?}");
    assert_eq!(server_places(sp), raw, "raw R waits for the client's applied events");
    assert_eq!(heard.receipts.len(), 1);
    assert_eq!((heard.receipts[0].action, heard.receipts[0].fate, heard.receipts[0].included_events), (action, ActionFate::Accepted, events + 2));
    assert_eq!(outcome.len(), 1, "one outcome");
    assert!(outcome[0].accepted && outcome[0].window_event == events + 2);
    assert!(sp.obligations_proven_zero(), "Strict creates no debt");
    // The client applies both: one arrow and one point of wear, once.
    let sp = &mut r.hs.server.players[r.i];
    crate::window_events::apply_through(sp, events + 2, 0);
    assert_eq!(sp.inventory.slot(1).map(|s| s.count), Some(4));
    assert!(matches!(sp.inventory.slot(0), Some(ItemStack { item: Item::Tool(t), count: 1 }) if t.durability == 29));
    assert!(sp.window_events.debt.is_empty() && sp.obligations_proven_zero());
}

/// What a refused-at-cap shot must leave exactly as it was.
#[derive(Debug, PartialEq)]
struct ShotSnapshot {
    /// The bow and the arrows (slots 0 and 1): unpaid.
    weapon_and_ammo: Vec<protocol::WireSlot>,
    last: u32,
    capacity: usize,
    through: u64,
    result_bytes: usize,
    projectiles: usize,
    next_shot_tick: u64,
}

fn snapshot(r: &Rig) -> ShotSnapshot {
    let sp = r.sp();
    ShotSnapshot {
        weapon_and_ammo: server_places(sp)[..2].to_vec(),
        last: sp.window_events.last_number(),
        capacity: sp.window_events.waiting_capacity(),
        through: r.through(),
        result_bytes: sp.action_results.as_ref().unwrap().retained_bytes(),
        projectiles: r.projectiles(),
        next_shot_tick: sp.next_shot_tick,
    }
}

/// A shot refused at a cap: no projectile, no payment, no event, no row,
/// no grown reservation; the action is not terminal; the connection is
/// torn down with a reason (fail closed until recovery is wired).
fn assert_zero_effect_shot(mut r: Rig, why: &str) {
    let tool = bow(30);
    let before = snapshot(&r);
    let action = shoot(&mut r, tool);
    let (outcome, heard) = outcomes(&r.client);
    // The teardown's departure settles the events that were already
    // waiting (they are the client's, not the shot's); it numbers none and
    // pops without shrinking, so these are comparable afterwards.
    assert_eq!(snapshot(&r), before, "{why}: zero external effect");
    assert!(outcome.is_empty() && heard.receipts.is_empty(), "{why}: nothing answered");
    assert_eq!(r.row(action), None, "{why}: no result row");
    assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::FellBehind.reason()), "{why}: teardown reason {:?}", heard.rejects);
}

/// The 1023/1024 event cap with a two-event shot: at 1022 pending the
/// shot fits (1024) and is Accepted; at 1023 it would need 1025, so it has
/// zero external effect.
#[test]
fn c3d_strict_shot_at_the_event_cap_has_zero_external_effect() {
    for (pending, fits) in [(1022u32, true), (1023, false)] {
        let mut r = strict_rig(&format!("shot-cap-{pending}"));
        let tool = arm(&mut r.hs.server.players[r.i]);
        let sp = &mut r.hs.server.players[r.i];
        for _ in 0..pending {
            crate::window_events::queue(sp, crate::window_events::WindowEvent::WearArmour { hits: 1 }, 0);
        }
        if fits {
            let projectiles = r.projectiles();
            let action = shoot(&mut r, tool);
            let (_, heard) = outcomes(&r.client);
            assert!(heard.rejects.is_empty(), "1022 pending: admitted, {:?}", heard.rejects);
            assert_eq!(r.projectiles(), projectiles + 1);
            assert_eq!(r.sp().window_events.waiting(), 1024, "exactly at the cap");
            assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Accepted));
        } else {
            assert_zero_effect_shot(r, "1023 pending");
        }
    }
}

/// The byte cap (a pending Grant carrying a large Plan body) and the
/// result cap (a full unacknowledged result history: the shot's events are
/// reserved first, then given back exactly) likewise leave zero effect.
#[test]
fn c3d_strict_shot_at_the_byte_and_receipt_caps_has_zero_external_effect() {
    let mut r = strict_rig("shot-bytes");
    arm(&mut r.hs.server.players[r.i]);
    let mut body = crate::plan::PlanData::marker_placeholder([5; 32], true);
    body.marker = None;
    body.name = "p".repeat(crate::action_journal::MAX_RETAINED_BYTES);
    let sp = &mut r.hs.server.players[r.i];
    crate::window_events::queue(sp, crate::window_events::WindowEvent::Grant(ItemStack { item: Item::Plan(body), count: 1 }), 0);
    assert_zero_effect_shot(r, "byte cap");

    let mut r = strict_rig("shot-receipts");
    arm(&mut r.hs.server.players[r.i]);
    r.fill_results();
    // Run-2 review LOW-4 — an empty deque at its own size, so the shot's
    // two-event reservation must grow it and `abandon` must give it back.
    r.hs.server.players[r.i].window_events.shrink_waiting_to_fit();
    assert_zero_effect_shot(r, "result cap");
}

/// Decision 10, detected at staging: a fold that already owes refuses the
/// click before any effect (nothing committed, no row), logged and torn
/// down with a reason; the saved record keeps the obligation (never
/// written off).
#[test]
fn c3d_strict_debt_at_staging_commits_nothing_and_is_never_written_off() {
    let mut r = strict_rig("debt-staging");
    stock(&mut r.hs.server.players[r.i]);
    let sp = &mut r.hs.server.players[r.i];
    let take = crate::container_window::ItemDelta { take: vec![(0, ItemStack::new_block(crate::block::DIRT, 1))], give: vec![] };
    sp.window_events.debt.apply(&mut sp.inventory, &mut sp.craft_grid, &mut sp.cursor, &mut sp.armour, &take);
    let mut cw = ClientWindow::of(r.sp());
    let before = server_places(r.sp());
    let action = click_packet(&mut r, &mut cw, WindowClick::Slot { slot: 0, right: false }, false);
    let heard = hear(&r.client);
    assert_eq!(server_places(r.sp()), before, "nothing committed");
    assert_eq!(r.row(action), None);
    assert_eq!(r.through(), action.seq - 1);
    assert!(heard.rejects.iter().any(|m| m.contains("obligation it can't account for")), "reason: {:?}", heard.rejects);
    assert!(!r.hs.server.load_joiner(r.i).expect("departure saved").obligations_proven_zero(), "the obligation is saved, not written off");
}

/// Decision 10, seen after a commit (the test fault seam makes the commit
/// leave an obligation): the guard tears the session down with a reason,
/// never panics, and the saved record keeps the obligation.
#[test]
fn c3d_strict_commit_that_leaves_debt_is_torn_down_and_saved_as_it_stands() {
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) { super::actions::commit_fault::OWE_AFTER_COMMIT.with(|f| f.set(false)); }
    }
    for (tag, shot) in [("click", false), ("shot", true)] {
        let mut r = strict_rig(&format!("debt-commit-{tag}"));
        stock(&mut r.hs.server.players[r.i]);
        let tool = arm(&mut r.hs.server.players[r.i]);
        let mut cw = ClientWindow::of(r.sp());
        super::actions::commit_fault::OWE_AFTER_COMMIT.with(|f| f.set(true));
        let _off = Off;
        let action = if shot { shoot(&mut r, tool) } else { click_packet(&mut r, &mut cw, WindowClick::Slot { slot: 1, right: false }, false) };
        let (_, heard) = outcomes(&r.client);
        assert_eq!(r.through(), action.seq, "{tag}: the commit happened once");
        assert!(heard.rejects.iter().any(|m| m.contains("obligation it can't account for")), "{tag}: torn down with a reason: {:?}", heard.rejects);
        assert!(r.hs.slot_is_free(r.i), "{tag}");
        assert!(!r.hs.server.load_joiner(r.i).expect("departure saved").obligations_proven_zero(), "{tag}: saved as it stands, never written off");
    }
}

/// C3d review M2 — Strict staging is the server's EXISTING transition: an
/// honest craft at the edge of table reach (the client's eye 6.3 blocks
/// from the table, the server's lagging body 6.6, inside the half-block
/// server slack) is Accepted, as `window_ops::serve_op` accepts it.
#[test]
fn c3d_strict_honest_craft_at_the_table_reach_edge_is_accepted() {
    let mut r = strict_rig("table-edge");
    let cell = [3, 90, 3];
    let centre = glam::Vec3::new(3.5, 90.5, 3.5);
    r.hs.server.world.set_block(cell[0], cell[1], cell[2], crate::block::CRAFTING_TABLE);
    let server_eye = centre + glam::Vec3::new(0.0, 0.0, 6.6);
    let client_eye = centre + glam::Vec3::new(0.0, 0.0, 6.3);
    let sp = &mut r.hs.server.players[r.i];
    sp.player.pos = server_eye - (sp.player.eye_pos() - sp.player.pos);
    sp.station = crate::window::Station::Table { cell };
    sp.table_gone_ticks = None;
    sp.craft_grid[0][0] = Some(ItemStack::new_block(crate::block::OAK_LOG, 1));
    assert!((r.sp().player.eye_pos() - server_eye).length() < 1e-4);
    let mut cw = ClientWindow::of(r.sp());
    let action = r.next();
    let (digest, ok) = cw.click(&r.hs, client_eye, &WindowClick::Result);
    assert!(ok, "the client crafted at its own (exact) reach");
    let unslacked = crate::window::ClickCtx::new(false, crate::window::Station::Table { cell }, server_eye, |_| crate::block::CRAFTING_TABLE);
    assert!(!unslacked.table_present(), "without the server's slack this body is out of reach");
    let op = protocol::WindowOpPacket {
        op_seq: action.seq as u32, op: protocol::WireWindowOp::Click(WindowClick::Result), digest,
        events_applied: action.events_applied, touched: Vec::new(), claims: Vec::new(), client_ok: ok,
    };
    r.send(action, ActionIntent::Window(op));
    let heard = hear(&r.client);
    assert!(heard.rejects.is_empty(), "no teardown: {:?}", heard.rejects);
    assert_eq!(heard.receipts.first().map(|x| (x.action, x.fate)), Some((action, ActionFate::Accepted)));
    assert_eq!(server_places(r.sp()), cw.places(), "the server crafted exactly what the client did");
    assert!(r.sp().obligations_proven_zero());
}

/// Run-2 review LOW-1 — "Strict leaves no unpaid obligation" is ONE
/// predicate for staging and the post-commit guard, and it covers a pending
/// Take the window can't pay (not only correction debt). At staging: nothing
/// committed, torn down with the reason. After a commit (fault seam): the
/// guard notices and tears down.
#[test]
fn c3d_strict_unpayable_pending_take_is_an_obligation_at_staging_and_after_commit() {
    let mut r = strict_rig("short-take-staging");
    stock(&mut r.hs.server.players[r.i]);
    crate::window_events::queue(&mut r.hs.server.players[r.i], crate::window_events::WindowEvent::Take { slot: 35, item: Item::Block(crate::block::BEDROCK), n: 1, how: "test" }, 0);
    assert!(r.sp().obligations_proven_zero(), "the correction-debt fold alone misses it");
    let mut cw = ClientWindow::of(r.sp());
    let before = server_places(r.sp());
    let action = click_packet(&mut r, &mut cw, WindowClick::Slot { slot: 0, right: false }, false);
    let heard = hear(&r.client);
    assert_eq!(server_places(r.sp()), before, "staging: nothing committed");
    assert_eq!(r.row(action), None, "staging: no row");
    assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::Obligation.reason()), "staging: {:?}", heard.rejects);

    struct Off;
    impl Drop for Off {
        fn drop(&mut self) { super::actions::commit_fault::SHORT_TAKE_AFTER_COMMIT.with(|f| f.set(false)); }
    }
    let mut r = strict_rig("short-take-commit");
    stock(&mut r.hs.server.players[r.i]);
    let mut cw = ClientWindow::of(r.sp());
    super::actions::commit_fault::SHORT_TAKE_AFTER_COMMIT.with(|f| f.set(true));
    let _off = Off;
    let action = click_packet(&mut r, &mut cw, WindowClick::Slot { slot: 1, right: false }, false);
    let heard = hear(&r.client);
    assert_eq!(r.through(), action.seq, "the commit happened once");
    assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::Obligation.reason()), "post-commit guard noticed the unpaid Take: {:?}", heard.rejects);
    assert!(r.hs.slot_is_free(r.i));
}

/// Run-2 review LOW-2 — an honest Autofill its client refused for reach
/// (`NeedsTable`: its eye 6.6 blocks from the table) while the server's
/// half-block slack still sees the table is Accepted with no change, not
/// torn down; its grid stays where it was.
#[test]
fn c3d_strict_refused_autofill_inside_the_slack_is_accepted_unchanged() {
    let mut r = strict_rig("autofill-slack");
    let cell = [3, 90, 3];
    let centre = glam::Vec3::new(3.5, 90.5, 3.5);
    r.hs.server.world.set_block(cell[0], cell[1], cell[2], crate::block::CRAFTING_TABLE);
    let eye = centre + glam::Vec3::new(0.0, 0.0, 6.6);
    let sp = &mut r.hs.server.players[r.i];
    sp.player.pos = eye - (sp.player.eye_pos() - sp.player.pos);
    sp.station = crate::window::Station::Table { cell };
    sp.table_gone_ticks = None;
    sp.inventory.set_slot(0, Some(ItemStack::new_block(crate::block::OAK_LOG, 5)));
    sp.craft_grid[1][1] = Some(ItemStack::new_block(crate::block::OAK_LOG, 1));
    let mut example = [[crate::crafting::CraftSlot::Empty; 3]; 3];
    example[0][0] = crate::crafting::CraftSlot::Block(crate::block::OAK_LOG);
    let click = WindowClick::Autofill { example };
    let mut cw = ClientWindow::of(r.sp());
    let action = r.next();
    let (digest, ok) = cw.click(&r.hs, eye, &click);
    assert!(!ok, "the client's exact rule refused it (NeedsTable)");
    let slack = crate::window::ClickCtx::new(false, crate::window::Station::Table { cell }, eye, |_| crate::block::CRAFTING_TABLE).with_server_slack(false);
    assert!(slack.table_present(), "the server's half-block slack still sees the table");
    let op = protocol::WindowOpPacket {
        op_seq: action.seq as u32, op: protocol::WireWindowOp::Click(click), digest,
        events_applied: action.events_applied, touched: Vec::new(), claims: Vec::new(), client_ok: ok,
    };
    r.send(action, ActionIntent::Window(op));
    let heard = hear(&r.client);
    assert!(heard.rejects.is_empty(), "no teardown: {:?}", heard.rejects);
    assert_eq!(heard.receipts.first().map(|x| (x.action, x.fate)), Some((action, ActionFate::Accepted)));
    assert_eq!(server_places(r.sp()), cw.places(), "exactly the client's (unchanged) window");
}

/// Run 4 — a Strict short Take applied by `window_events::apply_front` is an
/// invariant breach, never log-only. Here the action's own boundary applies
/// it (its `events_applied` covers the Take) before staging can see it:
/// torn down with the obligation reason through the one BRIDGE, no row, the
/// action not terminal, nothing committed.
#[test]
fn c3d_strict_short_take_applied_at_the_boundary_is_torn_down_with_a_reason() {
    let mut r = strict_rig("short-take");
    stock(&mut r.hs.server.players[r.i]);
    let mut cw = ClientWindow::of(r.sp());
    let sp = &mut r.hs.server.players[r.i];
    let take = crate::window_events::WindowEvent::Take { slot: 35, item: Item::Block(crate::block::BEDROCK), n: 1, how: "test short take" };
    r.applied = crate::window_events::queue(sp, take, 0);
    let before = server_places(r.sp());
    let action = click_packet(&mut r, &mut cw, WindowClick::Slot { slot: 1, right: false }, false);
    let heard = hear(&r.client);
    assert!(
        heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::Obligation.reason()),
        "a short Take is torn down with a reason: {:?}", heard.rejects
    );
    assert!(heard.receipts.is_empty(), "nothing answered");
    assert_eq!(r.row(action), None, "no result row");
    assert_eq!(r.through(), action.seq - 1, "the action is not terminal");
    assert_eq!(server_places(r.sp()), before, "nothing committed");
    assert!(r.hs.slot_is_free(r.i));
}

/// Run 4 (run-3 review M1) — the session-start auto-refill setting can reach
/// the server before its client applied the join's window sync (the Sync
/// landed a poll late), and a toggle can arrive while the death sync waits.
/// Configuration touches no contents: both are Accepted, the setting
/// applied, nothing else changed, never torn down.
#[test]
fn c3d_strict_auto_refill_before_the_join_sync_or_during_death_is_accepted() {
    for death in [false, true] {
        let tag = if death { "refill-death" } else { "refill-late-sync" };
        let mut r = strict_rig(tag);
        stock(&mut r.hs.server.players[r.i]);
        let sp = &mut r.hs.server.players[r.i];
        let window = crate::window_events::EffectiveWindow { inv: sp.inventory.clone(), armour: sp.armour, cursor: sp.cursor.clone(), grid: sp.craft_grid.clone() };
        let n = crate::window_events::queue(sp, crate::window_events::WindowEvent::Replace { window: Box::new(window), debt: Default::default() }, 0);
        if death { sp.window_events.death_replace = Some(n); }
        let on = !r.sp().inventory.auto_refill;
        let before = server_places(r.sp());
        // The client's boundary is before the sync: its digest is of the
        // window the sync is about to replace (here: nothing like it).
        let action = r.next();
        let op = protocol::WindowOpPacket {
            op_seq: action.seq as u32, op: protocol::WireWindowOp::SetAutoRefill { on }, digest: 0,
            events_applied: action.events_applied, touched: Vec::new(), claims: Vec::new(), client_ok: true,
        };
        r.send(action, ActionIntent::Window(op));
        let heard = hear(&r.client);
        assert!(heard.rejects.is_empty(), "{tag}: an honest setting is never torn down: {:?}", heard.rejects);
        assert_eq!(heard.receipts.first().map(|x| (x.action, x.fate)), Some((action, ActionFate::Accepted)), "{tag}");
        assert_eq!(r.sp().inventory.auto_refill, on, "{tag}: the setting is applied");
        assert_eq!(server_places(r.sp()), before, "{tag}: no contents change");
        assert_eq!(r.sp().window_events.waiting(), 1, "{tag}: the sync still waits for its client");
        assert!(r.sp().obligations_proven_zero(), "{tag}");
    }
}
