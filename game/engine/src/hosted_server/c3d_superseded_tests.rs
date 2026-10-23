//! C3d run 5 (coordinator decision; run-4 review M1 and LOW-1) — stale-base
//! and dead-body actions are `Superseded`, not breaches: an action from a
//! dead body, or made on a window base older than a whole-window `Replace`
//! (the join or death sync) its client hadn't applied, gets the terminal
//! `Superseded` fate with zero external effect and is never torn down.
//! Configuration stays served. Every case asserts conservation by exact
//! identity over the effective window, the chest, every grave and the
//! ground, through the real `HostedServer` dispatch.

use super::c3d_container_tests::{all_holdings, heard_with_opens, join_as, join_unsynced, place_chest, chest_slots, Joiner};
use super::c3d_strict_tests::{arm, server_places};
use super::c3d_tests::{hear, KEY};
use super::tests::start_room_test_server;
use super::*;
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::action_wire::{ActionIntent, ActionPacket};
use crate::item::{Item, ItemStack};
use crate::transport::ClientTransport as _;
use crate::window::WindowClick;

fn strict_server(tag: &str) -> HostedServer {
    let mut hs = start_room_test_server(tag, 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects any mode");
    hs
}

/// The client predicts a personal-window `click` on the window it holds
/// (the server's raw copy: the client is in lockstep at its base).
fn predict(hs: &HostedServer, i: usize, click: &WindowClick) -> (u32, bool) {
    let sp = &hs.server.players[i];
    let (mut inv, mut armour, mut cursor, mut grid) = (sp.inventory.clone(), sp.armour, sp.cursor.clone(), sp.craft_grid.clone());
    let ctx = crate::window::ClickCtx::new(false, sp.station, sp.player.eye_pos(), |_| crate::block::AIR);
    let mut view = crate::window::WindowMut { inv: &mut inv, armour: &mut armour, cursor: &mut cursor, grid: &mut grid, container: None };
    let result = crate::window::apply(&mut view, click, &ctx);
    (crate::window::digest(&view, crate::window::station_after(sp.station, click, &result)), result.ok())
}

fn send_click(j: &mut Joiner, hs: &HostedServer, click: WindowClick) -> ActionHeader {
    let (digest, ok) = predict(hs, j.i, &click);
    j.send_op(protocol::WireWindowOp::Click(click), digest, ok)
}

/// A bow shot from hotbar slot 0, as the client claims it.
fn send_shot(j: &mut Joiner, tool: crate::crafting::Tool) -> ActionHeader {
    let shot = j.next();
    let item = Item::Tool(tool);
    let (held_kind, held_id) = crate::inventory::item_to_ref(&item).to_wire();
    let req = protocol::ItemActionPacket {
        seq: shot.seq as u32,
        action: protocol::ItemAction::Shoot { weapon: protocol::ShotWeapon::Bow, hotbar_slot: 0, held_kind, held_id, held_full: crate::inventory::item_to_wire_full(&item), yaw: 0.0, pitch: 0.0, charge: 20 },
        events_applied: shot.events_applied,
    };
    j.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action: shot, intent: ActionIntent::Item(req) }));
    shot
}

fn projectiles(hs: &HostedServer) -> usize {
    hs.server.ecs.query::<&crate::entity::ProjectileEntity>().iter().count()
}

fn kill(hs: &mut HostedServer, i: usize) {
    let sp = &mut hs.server.players[i];
    sp.combat.health = 0.0;
    sp.combat.dead = true;
}

/// What a superseded action must leave: its row `Superseded`, no reject,
/// the seat still joined, the window events and server window unchanged.
fn assert_superseded(hs: &mut HostedServer, j: &Joiner, action: ActionHeader, what: &str) {
    assert_eq!(j.row(hs, action), Some((action, ActionFate::Superseded)), "{what}: the typed fate");
    assert!(!hs.slot_is_free(j.i), "{what}: still joined");
}

/// M1 — a joiner dies with a chest open; its client closes the screen and
/// sends the Close (before applying the death sync, and again after it,
/// while still dead). Both are `Superseded`, never torn down; the grave
/// holds what the window lost; after the respawn the next action is served.
#[test]
fn c3d_strict_death_with_a_chest_open_supersedes_the_close_in_flight() {
    let _g = crate::save::WorldsRootGuard::new("c3d-superseded-death-close");
    let mut hs = strict_server("c3d-superseded-death-close");
    let mut j = join_as(&mut hs, KEY, 17);
    hs.server.players[j.i].inventory.set_slot(3, Some(ItemStack::new_block(crate::block::STONE, 12)));
    let cell = place_chest(&mut hs, j.i, &[(0, ItemStack::new_block(crate::block::DIRT, 5))]);
    let digest = Joiner::digest(&hs, j.i);
    j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    j.apply_all(&mut hs);
    let _ = heard_with_opens(&j.client);
    let before = all_holdings(&hs, &[j.i], cell);

    kill(&mut hs, j.i);
    hs.tick(); // the death is handled: grave, view closed, death sync queued
    assert!(hs.server.players[j.i].window_events.death_replace.is_some());
    assert_eq!(hs.server.players[j.i].open_container, None, "death closed the server's view");
    let waiting = hs.server.players[j.i].window_events.waiting();
    let raw = server_places(&hs.server.players[j.i]);

    // The Close the client sends from the old base (death sync unapplied).
    let close = send_click(&mut j, &hs, WindowClick::Close);
    hs.tick();
    let heard = hear(&j.client);
    assert!(heard.rejects.is_empty(), "the Close at death is never torn down: {:?}", heard.rejects);
    assert_superseded(&mut hs, &j, close, "Close before the death sync");
    assert_eq!(hs.server.players[j.i].window_events.waiting(), waiting, "no event");
    assert_eq!(server_places(&hs.server.players[j.i]), raw, "no window change");
    assert_eq!(all_holdings(&hs, &[j.i], cell), before, "conserved over window, chest, grave and ground");

    // The client applies the death sync; still dead, it closes again.
    j.apply_all(&mut hs);
    let after_sync = server_places(&hs.server.players[j.i]);
    let close = send_click(&mut j, &hs, WindowClick::Close);
    hs.tick();
    assert!(hear(&j.client).rejects.is_empty(), "a dead body's Close is never torn down");
    assert_superseded(&mut hs, &j, close, "Close from a dead body");
    assert_eq!(server_places(&hs.server.players[j.i]), after_sync, "no window change");
    assert_eq!(all_holdings(&hs, &[j.i], cell), before, "still conserved");
    // Run 6 (coordinator rule): a dead body's action on a current base gets
    // one correction `Replace`; the client applies it.
    assert!(hs.server.players[j.i].window_events.replace_waiting(), "a correction Replace follows");
    j.apply_all(&mut hs);

    // Respawn; the next action is served normally.
    hs.server.players[j.i].dead_ticks = crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN;
    j.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Respawn, &()));
    hs.tick();
    assert!(hs.server.players[j.i].is_present_and_alive(), "respawned");
    let digest = Joiner::digest(&hs, j.i);
    let open_player = j.op(&mut hs, protocol::WireWindowOp::OpenPlayer, digest, true);
    let close = send_click(&mut j, &hs, WindowClick::Close);
    hs.tick();
    let heard = hear(&j.client);
    assert!(heard.rejects.is_empty(), "{:?}", heard.rejects);
    assert_eq!(j.row(&mut hs, open_player).map(|r| r.1), Some(ActionFate::Accepted));
    assert_eq!(j.row(&mut hs, close).map(|r| r.1), Some(ActionFate::Accepted), "the next click is served");
    assert_eq!(all_holdings(&hs, &[j.i], cell), before, "conserved through the respawn");
    assert!(hs.server.players[j.i].obligations_proven_zero());
}

/// M1 race — a window click and a Shoot already in flight when the killing
/// hit lands: both `Superseded`, no projectile, no payment (no Take or Wear
/// event), no window change; conserved.
#[test]
fn c3d_strict_click_and_shot_in_flight_at_death_are_superseded() {
    let _g = crate::save::WorldsRootGuard::new("c3d-superseded-death-race");
    let mut hs = strict_server("c3d-superseded-death-race");
    let mut j = join_as(&mut hs, KEY, 17);
    let tool = arm(&mut hs.server.players[j.i]);
    hs.server.players[j.i].inventory.set_slot(4, Some(ItemStack::new_block(crate::block::STONE, 9)));
    let cell = place_chest(&mut hs, j.i, &[]);
    let before = all_holdings(&hs, &[j.i], cell);
    let shots = projectiles(&hs);
    // Sent while alive (on the base the client holds), processed after the
    // server's death of this body.
    kill(&mut hs, j.i);
    let click = send_click(&mut j, &hs, WindowClick::Slot { slot: 4, right: false });
    let shot = send_shot(&mut j, tool);
    let next_shot = hs.server.players[j.i].next_shot_tick;
    hs.tick();
    let heard = hear(&j.client);
    assert!(heard.rejects.is_empty(), "in-flight actions at death are never torn down: {:?}", heard.rejects);
    assert_superseded(&mut hs, &j, click, "click at death");
    assert_superseded(&mut hs, &j, shot, "shot at death");
    assert_eq!(projectiles(&hs), shots, "no projectile");
    assert_eq!(hs.server.players[j.i].next_shot_tick, next_shot, "no shot cooldown");
    assert!(
        // The death sync, and the personal fence every death now delivers.
        hs.server.players[j.i].window_events.waiting_events().all(|e| matches!(
            e,
            crate::window_events::WindowEvent::Replace { .. } | crate::window_events::WindowEvent::PersonalSync(_)
        )),
        "no payment: only the death sync waits"
    );
    assert_eq!(all_holdings(&hs, &[j.i], cell), before, "conserved over window, grave and ground");
}

/// LOW-1 — a click, a container open and a shot made before the join sync
/// applies: each `Superseded`, never torn down, with zero effect (no view
/// opened, no projectile, no event); conserved.
#[test]
fn c3d_strict_click_open_and_shot_before_the_join_sync_are_superseded() {
    let _g = crate::save::WorldsRootGuard::new("c3d-superseded-join");
    let mut hs = strict_server("c3d-superseded-join");
    // A returning joiner: its join sync carries the saved window.
    let first = join_as(&mut hs, KEY, 17);
    hs.server.players[first.i].inventory.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 6)));
    hs.release_slot(first.i, None);
    let mut j = join_unsynced(&mut hs, KEY, 34);
    assert!(hs.server.players[j.i].window_events.replace_waiting(), "the join sync waits");
    let cell = place_chest(&mut hs, j.i, &[(0, ItemStack::new_block(crate::block::DIRT, 3))]);
    let before = all_holdings(&hs, &[j.i], cell);
    let (waiting, shots, chest) = (hs.server.players[j.i].window_events.waiting(), projectiles(&hs), chest_slots(&hs, cell));
    let raw = server_places(&hs.server.players[j.i]);
    let click = send_click(&mut j, &hs, WindowClick::Slot { slot: 0, right: false });
    let digest = Joiner::digest(&hs, j.i);
    let open = j.send_op(protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    let shot = send_shot(&mut j, super::c3d_strict_tests::bow(30));
    hs.tick();
    let (heard, opens) = heard_with_opens(&j.client);
    assert!(heard.rejects.is_empty(), "nothing before the join sync is torn down: {:?}", heard.rejects);
    for (action, what) in [(click, "click"), (open, "open"), (shot, "shot")] {
        assert_superseded(&mut hs, &j, action, what);
    }
    assert!(opens.is_empty(), "no view opened");
    assert_eq!(hs.server.players[j.i].open_container, None);
    assert_eq!(projectiles(&hs), shots, "no projectile");
    assert_eq!(hs.server.players[j.i].window_events.waiting(), waiting, "no event");
    assert_eq!(server_places(&hs.server.players[j.i]), raw, "no window change");
    assert_eq!(chest_slots(&hs, cell), chest, "no chest change");
    assert_eq!(all_holdings(&hs, &[j.i], cell), before, "conserved");
    // Once the sync applies, the same click is served.
    j.apply_all(&mut hs);
    let digest = Joiner::digest(&hs, j.i);
    let set = j.op(&mut hs, protocol::WireWindowOp::OpenPlayer, digest, true);
    assert_eq!(j.row(&mut hs, set).map(|r| r.1), Some(ActionFate::Accepted));
    assert!(hear(&j.client).rejects.is_empty());
}

/// Run-4 review LOW-2 — a host-deposited Plan whose body is far past the
/// 2 MiB event budget, in a shared chest: a Strict open is served (the
/// view counts the placeholder a mirror holds, not the body), and the real
/// chest keeps the real body.
#[test]
fn c3d_strict_open_of_a_chest_holding_a_large_plan_counts_its_placeholder() {
    let _g = crate::save::WorldsRootGuard::new("c3d-superseded-plan-view");
    let mut hs = strict_server("c3d-superseded-plan-view");
    let mut j = join_as(&mut hs, KEY, 17);
    let mut body = crate::plan::PlanData::debug_3x3_stone();
    let cell0 = body.cells[0].clone();
    body.cells = vec![cell0; 3 * 1024 * 1024 / std::mem::size_of_val(&body.cells[0]).max(1)];
    assert!(crate::body_residency::allocated_body_bytes(&body).unwrap() > 2 * 1024 * 1024, "a body past the budget");
    let marker = crate::plan::marker(&body);
    let cell = place_chest(&mut hs, j.i, &[(0, ItemStack { item: Item::Plan(body), count: 1 })]);
    let digest = Joiner::digest(&hs, j.i);
    let open = j.op(&mut hs, protocol::WireWindowOp::OpenContainer { cell }, digest, true);
    let (heard, opens) = heard_with_opens(&j.client);
    assert!(heard.rejects.is_empty(), "the open is never torn down: {:?}", heard.rejects);
    assert_eq!(j.row(&mut hs, open).map(|r| r.1), Some(ActionFate::Accepted));
    assert!(opens.len() == 1 && opens[0].refused.is_none(), "served: {opens:?}");
    let kind = crate::container_window::ContainerKind::of_block(crate::block::CHEST).unwrap();
    let real = crate::container_window::container_at(&hs.server.world, cell, kind).unwrap();
    let Some(ItemStack { item: Item::Plan(kept), .. }) = real.get(0) else { panic!("the real chest keeps the Plan") };
    assert_eq!(crate::plan::marker(kept), marker, "the real body, untouched");
}
