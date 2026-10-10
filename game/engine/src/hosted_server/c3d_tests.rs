//! C3d — real `HostedServer` paths: Strict admission against the joiner's
//! obligation record (Decision 10 with OC, v88) and the typed action FIFO's
//! exact duplicate result. Strict is selected internally (the operator flag
//! is not exposed); Mirror stays the default bridge.

use super::tests::{send_signed_join, start_room_test_server};
use super::*;
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::action_wire::{ActionIntent, ActionPacket, ActionReceipt};
use crate::joiner_save::JoinerSave;
use crate::transport::ClientTransport as _;

pub(super) const KEY: [u8; 32] = [66; 32];

/// What a client has been sent, of the packets these tests read.
#[derive(Default)]
pub(super) struct Heard {
    pub(super) accepts: Vec<protocol::JoinAcceptPacket>,
    pub(super) rejects: Vec<String>,
    pub(super) receipts: Vec<ActionReceipt>,
}

pub(super) fn hear(client: &ChannelClientTransport) -> Heard {
    let mut out = Heard::default();
    while let Some(pkt) = client.try_recv_from_server() {
        match protocol::deserialize_header(&pkt) {
            Some((protocol::PacketType::JoinAccept, p)) => out.accepts.push(protocol::safe_deserialize(p).unwrap()),
            Some((protocol::PacketType::JoinReject, p)) => {
                out.rejects.push(protocol::safe_deserialize::<protocol::JoinRejectPacket>(p).unwrap().reason)
            }
            Some((protocol::PacketType::ActionReceipt, p)) => out.receipts.push(protocol::safe_deserialize(p).unwrap()),
            _ => {}
        }
    }
    out
}

/// Join `KEY` on a fresh connection (binding `b`), tick, and return the slot
/// and what the client heard.
pub(super) fn join(hs: &mut HostedServer, b: u8) -> (usize, ChannelClientTransport, Heard) {
    let client = hs.attach_test_remote_with_binding(Some([b; 32]));
    send_signed_join(&client, KEY, Some([b; 32]));
    hs.tick();
    let i = hs.server.players.len() - 1;
    let heard = hear(&client);
    (i, client, heard)
}

pub(super) fn record_path(hs: &HostedServer, i: usize) -> std::path::PathBuf {
    let pk = hs.server.players[i].verified_pubkey.expect("a signed join");
    crate::save::world_dir(&hs.server.world_name).join("joiners").join(format!("{}.dat", pubkey_to_npub(&pk)))
}

/// A correction take of an absent item on the server copy: `n` units owed.
/// The client has applied everything sent so far first (the join's
/// `Replace` among it), as a real correction follows the join sync; a raw
/// debt under a waiting `Replace` would be overwritten by it in the fold.
fn owe(sp: &mut crate::server::ServerPlayer, item: crate::item::Item, n: u8) {
    let sent = sp.window_events.last_number();
    crate::window_events::apply_through(sp, sent, 0);
    let take = crate::container_window::ItemDelta { take: vec![(0, crate::item::ItemStack { item, count: n })], give: vec![] };
    sp.window_events.debt.apply(&mut sp.inventory, &mut sp.craft_grid, &mut sp.cursor, &mut sp.armour, &take);
}

/// A Mirror session leaves `prepare`'s record behind; a Strict reconnect
/// by the same npub is then refused before `JoinAccept`, with `expect` in
/// the reason, the record's bytes untouched and the seat free.
fn assert_strict_refuses(tag: &str, prepare: impl FnOnce(&mut HostedServer, usize), expect: &str) {
    let _g = crate::save::WorldsRootGuard::new(&format!("c3d-admit-{tag}"));
    let mut hs = start_room_test_server(&format!("c3d-admit-{tag}"), 0);
    let (i, _first, heard) = join(&mut hs, 17);
    assert_eq!(heard.accepts.len(), 1, "{tag}: Mirror admits the first session");
    prepare(&mut hs, i);
    let path = record_path(&hs, i);
    let before = std::fs::read(&path).expect("the Mirror session left a record");
    hs.inventory_mode = InventoryMode::Strict;
    let (j, _second, heard) = join(&mut hs, 34);
    assert!(heard.accepts.is_empty(), "{tag}: no JoinAccept for an unproven record");
    assert_eq!(heard.rejects.len(), 1, "{tag}: one typed refusal");
    assert!(heard.rejects[0].contains(expect), "{tag}: reason {:?} names {expect:?}", heard.rejects[0]);
    // No settlement exists: the refusal points at the operator, never "until settled".
    assert!(
        heard.rejects[0].ends_with(" Ask whoever runs this world about your save."),
        "{tag}: reason {:?} ends with the operator sentence",
        heard.rejects[0]
    );
    assert!(hs.slot_is_free(j), "{tag}: the refused seat is released");
    assert_eq!(std::fs::read(&path).unwrap(), before, "{tag}: the record's bytes are untouched");
}

#[test]
fn c3d_strict_admission_refuses_a_record_that_owes() {
    assert_strict_refuses(
        "owed",
        |hs, i| {
            owe(&mut hs.server.players[i], crate::item::Item::Block(crate::block::STONE), 1);
            hs.release_slot(i, None);
        },
        "still owes items from an earlier correction (1 kind)",
    );
}

#[test]
fn c3d_strict_admission_refuses_a_v1_record() {
    assert_strict_refuses(
        "v1",
        |hs, i| {
            hs.release_slot(i, None);
            // v2 = the v1 fields, then `owed` (an empty fixint Vec: 8 zero
            // bytes) and `obligation_history` (`Complete`: variant 0, 4 zero
            // bytes). Dropping both and writing version 1 is the v1 record.
            let path = record_path(hs, i);
            let mut bytes = std::fs::read(&path).unwrap();
            assert_eq!(bytes[0], 2, "the writer writes v2");
            assert_eq!(&bytes[bytes.len() - 12..], &[0; 12], "empty owed + Complete");
            bytes.truncate(bytes.len() - 12);
            bytes[0] = 1;
            std::fs::write(&path, &bytes).unwrap();
            assert_eq!(
                hs.server.load_joiner(i).expect("a v1 record loads").history(),
                crate::joiner_save::ObligationHistory::Incomplete(crate::joiner_save::IncompleteCause::LegacyV1)
            );
        },
        "older version that didn't record owed items",
    );
}

#[test]
fn c3d_strict_admission_refuses_an_overflowed_record() {
    assert_strict_refuses(
        "overflow",
        |hs, i| {
            for durability in 1..=257 {
                let tool = crate::crafting::Tool {
                    tool_type: crate::crafting::ToolType::Pickaxe,
                    material: crate::crafting::ToolMaterial::Diamond,
                    durability,
                };
                owe(&mut hs.server.players[i], crate::item::Item::Tool(tool), 1);
            }
            hs.release_slot(i, None);
        },
        "lost track of some owed items",
    );
}

#[test]
fn c3d_strict_admission_refuses_an_unreadable_record() {
    assert_strict_refuses(
        "unreadable",
        |hs, i| {
            hs.release_slot(i, None);
            std::fs::write(record_path(hs, i), b"not a joiner save").unwrap();
        },
        "couldn't be read",
    );
}

#[test]
fn c3d_strict_admission_admits_proven_records_and_mirror_admission_is_unchanged() {
    let _g = crate::save::WorldsRootGuard::new("c3d-admit-proven");
    // No record: proven. Strict is advertised on the accept.
    let mut hs = start_room_test_server("c3d-admit-proven", 0);
    hs.inventory_mode = InventoryMode::Strict;
    let (i, _a, heard) = join(&mut hs, 17);
    assert_eq!(heard.accepts.len(), 1, "no record is proven zero");
    assert_eq!(heard.accepts[0].inventory_mode, InventoryMode::Strict);
    assert_eq!(hs.server.players[i].inventory_mode, InventoryMode::Strict);
    // A fresh v2 record with nothing owed: proven.
    hs.release_slot(i, None);
    assert!(hs.server.load_joiner(i).unwrap().obligations_proven_zero());
    let (j, _b, heard) = join(&mut hs, 34);
    assert_eq!(heard.accepts.len(), 1, "a fresh v2 record is proven zero");
    assert!(heard.rejects.is_empty());
    // Mirror admits a record that owes, as before (its debt is carried).
    owe(&mut hs.server.players[j], crate::item::Item::Block(crate::block::STONE), 1);
    hs.release_slot(j, None);
    hs.inventory_mode = InventoryMode::Mirror;
    let (k, _c, heard) = join(&mut hs, 51);
    assert_eq!(heard.accepts.len(), 1, "Mirror admission is unchanged");
    assert_eq!(heard.accepts[0].inventory_mode, InventoryMode::Mirror);
    assert!(!hs.server.players[k].obligations_proven_zero(), "and the carried debt arrives");
}

/// The real FIFO: one action executes once; its exact repeat after it is
/// terminal returns the ORIGINAL receipt with zero mutation. A Strict
/// session creates no correction debt and its capture proves zero.
#[test]
fn c3d_actual_hosted_fifo_repeats_the_original_receipt_without_mutation_or_debt() {
    let _g = crate::save::WorldsRootGuard::new("c3d-fifo");
    let mut hs = start_room_test_server("c3d-fifo", 0);
    hs.inventory_mode = InventoryMode::Strict;
    let (i, client, heard) = join(&mut hs, 17);
    let accept = heard.accepts.first().expect("Strict admits a first visit");
    let action = ActionHeader { session: accept.action_session, epoch: 0, seq: 1, life: accept.action_life, events_applied: 0 };
    let packet = protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action, intent: ActionIntent::CancelledNoop });
    let window = |sp: &crate::server::ServerPlayer| crate::inventory_sync::packet(&crate::window_events::effective_window(sp), &Default::default(), 1);
    let window_before = window(&hs.server.players[i]);
    client.send_to_server(&packet);
    hs.tick();
    let first = hear(&client).receipts;
    assert_eq!(first.len(), 1, "one terminal receipt");
    assert_eq!((first[0].action, first[0].fate), (action, ActionFate::CancelledNoop));
    let through = hs.server.players[i].action_frontier.as_ref().unwrap().through();
    assert_eq!(through, 1);
    // The exact repeat: terminal duplicate, original fate, nothing new.
    client.send_to_server(&packet);
    hs.tick();
    let again = hear(&client).receipts;
    assert_eq!(again, first, "the duplicate returns the original immutable result");
    let sp = &hs.server.players[i];
    assert!(!hs.disconnected[i], "a recent exact duplicate is not a failure");
    assert_eq!(sp.action_frontier.as_ref().unwrap().through(), 1, "no second execution");
    assert!(sp.action_queue.as_ref().unwrap().is_empty());
    assert_eq!(window(sp), window_before, "zero mutation");
    // Strict never creates debt, and every capture proves zero.
    assert!(sp.window_events.debt.is_empty());
    assert!(sp.obligations_proven_zero());
    assert!(JoinerSave::capture(sp, hs.server.run_id).obligations_proven_zero());
}

/// Outbox-failure teardown on the real broadcast path: a terminal receipt
/// waiting behind a world backlog is dropped with that prefix when the queue
/// overflows; the server never sends it ahead of the repair, tears the
/// connection down with a reason, and the departure settles and saves a
/// record that proves zero.
#[test]
fn c3d_actual_hosted_lost_receipt_tears_down_with_a_reason_and_saves_a_proven_record() {
    let _g = crate::save::WorldsRootGuard::new("c3d-lost-receipt");
    let mut hs = start_room_test_server("c3d-lost-receipt", 0);
    hs.inventory_mode = InventoryMode::Strict;
    let (i, client, heard) = join(&mut hs, 17);
    let accept = heard.accepts.first().expect("Strict admits a first visit");
    let cell = |n: i32| protocol::BlockChange::with_meta(n % 1000, 200, n / 1000, crate::block::STONE, 0);
    // A backlog past one tick's budget: the receipt queues behind it.
    let backlog: Vec<_> = (0..8_000).map(cell).collect();
    hs.outboxes[i].push_tick(0, &[], &[], &backlog, &[]);
    let action = ActionHeader { session: accept.action_session, epoch: 0, seq: 1, life: accept.action_life, events_applied: 0 };
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action, intent: ActionIntent::CancelledNoop }));
    hs.tick();
    assert!(hear(&client).receipts.is_empty(), "the receipt waits behind the backlog");
    assert!(!hs.disconnected[i]);
    // Past the queue cap: the overflow drops the queued blocks and the
    // receipt behind them (their chunks need a later repair).
    let flood: Vec<_> = (8_000..200_000).map(cell).collect();
    hs.outboxes[i].push_tick(0, &[], &[], &flood, &[]);
    hs.tick();
    let heard = hear(&client);
    assert!(heard.receipts.is_empty(), "no receipt overtakes the lost world prefix");
    assert!(
        heard.rejects.iter().any(|r| r.contains("could not be delivered")),
        "the connection is torn down with a reason: {:?}",
        heard.rejects
    );
    assert!(hs.slot_is_free(i));
    assert!(hs.server.load_joiner(i).expect("the departure saved").obligations_proven_zero());
}

/// Review L5 — the same real lost-receipt teardown with a numbered Grant
/// and a Take still waiting when it happens: the departure settles them
/// and the saved window conserves both exactly (stone 5 − 2, dirt + 3),
/// with nothing owed. A teardown that skipped settlement or lost a pending
/// event would save a different window.
#[test]
fn c3d_actual_hosted_lost_receipt_teardown_saves_the_pending_grant_and_take() {
    let _g = crate::save::WorldsRootGuard::new("c3d-lost-receipt-pending");
    let mut hs = start_room_test_server("c3d-lost-receipt-pending", 0);
    hs.inventory_mode = InventoryMode::Strict;
    let (i, client, heard) = join(&mut hs, 17);
    let accept = heard.accepts.first().expect("Strict admits a first visit");
    let sp = &mut hs.server.players[i];
    let synced = sp.window_events.last_number();
    crate::window_events::apply_through(sp, synced, 0);
    sp.inventory.set_slot(0, Some(crate::item::ItemStack::new_block(crate::block::STONE, 5)));
    let stone = crate::item::Item::Block(crate::block::STONE);
    crate::window_events::queue(sp, crate::window_events::WindowEvent::Grant(crate::item::ItemStack::new_block(crate::block::DIRT, 3)), 0);
    crate::window_events::queue(sp, crate::window_events::WindowEvent::Take { slot: 0, item: stone, n: 2, how: "test" }, 0);
    assert_eq!(sp.window_events.waiting(), 2, "a Grant and a Take wait at teardown");
    let cell = |n: i32| protocol::BlockChange::with_meta(n % 1000, 200, n / 1000, crate::block::STONE, 0);
    let backlog: Vec<_> = (0..8_000).map(cell).collect();
    hs.outboxes[i].push_tick(0, &[], &[], &backlog, &[]);
    // The action reports only the join's events applied: both still wait.
    let action = ActionHeader { session: accept.action_session, epoch: 0, seq: 1, life: accept.action_life, events_applied: synced };
    client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Action, &ActionPacket { action, intent: ActionIntent::CancelledNoop }));
    hs.tick();
    assert_eq!(hs.server.players[i].window_events.waiting(), 2, "still waiting before the teardown");
    let flood: Vec<_> = (8_000..200_000).map(cell).collect();
    hs.outboxes[i].push_tick(0, &[], &[], &flood, &[]);
    hs.tick();
    let heard = hear(&client);
    assert!(heard.rejects.iter().any(|r| r.contains("could not be delivered")), "torn down: {:?}", heard.rejects);
    let saved = hs.server.load_joiner(i).expect("the departure saved");
    assert!(saved.obligations_proven_zero(), "nothing owed");
    let window = saved.window().expect("a readable window");
    let count = |item: crate::item::Item| -> u32 {
        (0..36).filter_map(|s| window.inv.slot(s)).filter(|s| s.item == item).map(|s| u32::from(s.count)).sum()
    };
    assert_eq!(count(crate::item::Item::Block(crate::block::STONE)), 3, "the pending Take is paid once");
    assert_eq!(count(crate::item::Item::Block(crate::block::DIRT)), 3, "the pending Grant is kept once");
}
