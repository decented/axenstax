//! C3d run 24 — a Strict seat's typed eat, creative give and power-device
//! right-click, on a REAL `RemoteClient` joined to a Strict `HostedServer`
//! (the `c3d_client_edit_tests` rig), routed as the game loop routes them
//! (`strict_requests::route_item`, `route_device`). The item is the server
//! copy's, never the claim; nothing is predicted; a window disagreement is
//! corrected, a world fact is a receipt (and its note) alone; never torn
//! down. And a Strict seat that sends a legacy inventory-touching packet
//! is still refused.
use super::c3d_client_attack_tests::fate;
use super::c3d_client_edit_tests::{joined, Joined};
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::item::{Item, ItemStack, MaterialId};
use crate::item_actions::ItemNote;
use crate::joiner_actions::{Asked, JoinerActions, Pending};
use crate::power::{PowerDeviceData, PowerDeviceKind};
use crate::protocol;
use crate::remote_client::{ConnectionState, RequestOutcome};
use crate::strict_requests::{route_device, route_item};

fn mat(m: MaterialId, n: u8) -> ItemStack { ItemStack::new_material(m, n) }

fn rig(tag: &str, items: &[(usize, ItemStack)]) -> Joined {
    let mut j = joined(tag, items, true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the refill setting");
    j
}

/// The eat `GameState::send_eat_request` builds from hotbar `hot`, routed
/// as the game loop routes it.
fn eat(j: &mut Joined, hot: usize) -> ActionHeader {
    let item = j.inv.slot(hot).map(|s| s.item.clone());
    let (held_kind, held_id) = item.as_ref().map_or(protocol::ItemRef::Empty.to_wire(), |i| crate::inventory::item_to_ref(i).to_wire());
    let held_full = item.as_ref().map_or(protocol::WireItem::None, crate::inventory::item_to_wire_full);
    let pending = Pending { kind: Asked::Eat, mob: None, hotbar_slot: hot, held: item };
    let packet = protocol::ItemActionPacket { seq: 0, action: protocol::ItemAction::Eat { hotbar_slot: hot as u8, held_kind, held_id, held_full }, events_applied: 0 };
    let window = j.ui.digest(&j.inv, &j.armour);
    let header = route_item(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut j.actions, pending, packet).expect("routed").expect("a Strict seat sends the eat typed");
    assert_eq!(j.ui.digest(&j.inv, &j.armour), window, "routing predicted nothing");
    header
}

/// The give `GameState::request_creative_give` builds.
fn give(j: &mut Joined, stack: &ItemStack) -> ActionHeader {
    let pending = Pending { kind: Asked::CreativeGive, mob: None, hotbar_slot: 0, held: None };
    let packet = protocol::ItemActionPacket { seq: 0, action: protocol::ItemAction::CreativeGive { item: crate::inventory::stack_to_wire(stack) }, events_applied: 0 };
    let window = j.ui.digest(&j.inv, &j.armour);
    let header = route_item(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut j.actions, pending, packet).expect("routed").expect("a Strict seat sends the give typed");
    assert_eq!(j.ui.digest(&j.inv, &j.armour), window, "routing predicted nothing");
    header
}

/// A right-click on the device at `cell` with hotbar `hot` in hand.
fn device(j: &mut Joined, cell: [i32; 3], hot: u8) -> ActionHeader {
    let window = j.ui.digest(&j.inv, &j.armour);
    let header = route_device(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, cell, hot).expect("routed").expect("a Strict seat sends the right-click typed");
    assert_eq!(j.ui.digest(&j.inv, &j.armour), window, "routing predicted nothing");
    header
}

/// One server tick and the client's read, then the item outcomes heard.
fn heard_notes(j: &mut Joined) -> Vec<(bool, u8)> {
    j.hs.tick();
    j.rc.poll();
    j.rc.pending_outcomes.iter().filter_map(|o| match o { RequestOutcome::Item(out) => Some((out.accepted, out.note)), _ => None }).collect()
}

fn tally(j: &Joined) -> (u32, u32, u32) {
    let t = j.sp().strict_refusals;
    (t.stale_world, t.other, t.world_fact)
}

/// A lever at `dx` blocks east of the joiner's eye, at eye height.
fn lever(j: &mut Joined, dx: i32) -> ([i32; 3], f32) {
    let eye = j.sp().player.eye_pos();
    let cell = [eye.x.floor() as i32 + dx, eye.y.floor() as i32, eye.z.floor() as i32];
    j.hs.server.world.set_block(cell[0], cell[1], cell[2], crate::block::LEVER);
    j.hs.server.world.insert_power_device((cell[0], cell[1], cell[2]), PowerDeviceData::new(PowerDeviceKind::Lever, crate::meta::Facing::East));
    let d = (glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5) - eye).length();
    (cell, d)
}

fn lever_on(j: &Joined, cell: [i32; 3]) -> bool {
    j.hs.server.world.power_device_at((cell[0], cell[1], cell[2])).expect("the lever").on
}

#[test]
fn c3d_client_strict_eat_is_served_from_the_server_item() {
    let bread = Item::Material(MaterialId::Bread);
    let mut j = rig("item-eat", &[(0, mat(MaterialId::Bread, 4))]);
    j.hs.server.players[j.i].combat.hunger = 2;
    let before = j.sp().window_events.last_number();
    let action = eat(&mut j, 0);
    j.pump();
    assert!(j.sp().combat.hunger > 2, "the body is fed");
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "served");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "one Take");
    j.assert_honest("eat");
    assert_eq!(j.actions.len(), 0, "the request record is answered");
    assert_eq!(j.held(&bread), 3, "one bread taken by the server's Take");
    assert_eq!(tally(&j), (0, 0, 0), "no refusal");
}

/// The client's slot 0 says bread; the server copy's slot 0 holds stone and
/// its bread is in slot 5: the window disagrees — refused and corrected
/// (one `Replace`), nothing eaten. A server that took the claim's item would
/// feed the body (and take a bread from slot 5).
#[test]
fn c3d_client_strict_eat_claim_the_copy_lacks_is_refused_and_corrected() {
    let bread = Item::Material(MaterialId::Bread);
    let mut j = rig("item-eat-claim", &[(0, ItemStack::new_block(crate::block::STONE, 1)), (5, mat(MaterialId::Bread, 4))]);
    j.inv.set_slot(0, Some(mat(MaterialId::Bread, 1)));
    j.hs.server.players[j.i].combat.hunger = 2;
    let before = j.sp().window_events.last_number();
    let action = eat(&mut j, 0);
    j.pump();
    assert_eq!(j.sp().combat.hunger, 2, "nothing eaten from the claim");
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace, no Take");
    j.assert_honest("the refused claim");
    assert_eq!(j.actions.len(), 0, "the refused eat's request record is released");
    assert_eq!(j.inv.slot(0).cloned(), Some(ItemStack::new_block(crate::block::STONE, 1)), "the client's slot corrected");
    assert_eq!(j.held(&bread), 4);
    assert_eq!(tally(&j), (0, 1, 0));
}

/// A held right-click with bread on a full stomach: nine eats in under 100
/// ticks, each refused `NotHungry` — a world fact (the note and a receipt,
/// no `Replace`, no tick of the correction bound), the bread kept, never
/// torn down.
#[test]
fn c3d_client_strict_nine_eats_on_a_full_stomach_never_tear_down() {
    let bread = Item::Material(MaterialId::Bread);
    let mut j = rig("item-eat-full", &[(0, mat(MaterialId::Bread, 16))]);
    let before = j.sp().window_events.last_number();
    let start = j.hs.server.tick_counter;
    for n in 1..=9 {
        let c = &mut j.hs.server.players[j.i].combat;
        (c.hunger, c.health) = (c.max_hunger, c.max_health);
        let action = eat(&mut j, 0);
        let notes = heard_notes(&mut j);
        assert_eq!(notes, vec![(false, ItemNote::NotHungry.to_wire())], "eat {n}: the note reaches the client");
        j.pump();
        assert_eq!(fate(&j, action), Some(ActionFate::Refused), "eat {n}: refused");
        for _ in 1..8 { j.pump(); }
        assert!(matches!(j.rc.state, ConnectionState::Connected { .. }) && !j.hs.slot_is_free(j.i), "eat {n}: never torn down");
    }
    assert!(j.hs.server.tick_counter - start < 100, "inside 100 ticks");
    assert_eq!(j.sp().window_events.last_number(), before, "no Replace, no Take");
    j.assert_honest("nine eats on a full stomach");
    assert_eq!(j.held(&bread), 16, "the bread kept");
    assert_eq!(tally(&j), (0, 0, 9));
}

#[test]
fn c3d_client_strict_creative_give_is_served_as_a_numbered_grant() {
    let mut j = rig("item-give", &[]);
    j.hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    let stone = ItemStack::new_block(crate::block::STONE, 64);
    let before = j.sp().window_events.last_number();
    let action = give(&mut j, &stone);
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "served");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "one Grant");
    j.assert_honest("creative give");
    assert_eq!(j.actions.len(), 0, "the request record is answered");
    assert_eq!(j.held(&stone.item), 64, "the give landed by its grant");
    assert_eq!(tally(&j), (0, 0, 0));
}

/// Not a Creative world: the give is refused `NotHere`, a world fact (the
/// play mode can change under the client), nothing granted, no correction.
#[test]
fn c3d_client_strict_creative_give_in_survival_is_refused_uncorrected() {
    let mut j = rig("item-give-survival", &[]);
    let stone = ItemStack::new_block(crate::block::STONE, 64);
    let before = j.sp().window_events.last_number();
    let action = give(&mut j, &stone);
    assert_eq!(heard_notes(&mut j), vec![(false, ItemNote::NotHere.to_wire())], "the note reaches the client");
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(j.sp().window_events.last_number(), before, "nothing granted, no correction");
    j.assert_honest("a give in survival");
    assert_eq!(j.held(&stone.item), 0, "nothing granted");
    assert_eq!(tally(&j), (0, 0, 1));
}

/// A full bag: the give is refused `InventoryFull` (both copies agree it is
/// full; nothing predicted), a world fact, never a correction.
#[test]
fn c3d_client_strict_creative_give_into_a_full_bag_is_refused_uncorrected() {
    let full: Vec<(usize, ItemStack)> = (0..36).map(|s| (s, ItemStack::new_block(crate::block::DIRT, 64))).collect();
    let mut j = rig("item-give-full", &full);
    j.hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    let before = j.sp().window_events.last_number();
    let action = give(&mut j, &ItemStack::new_block(crate::block::STONE, 1));
    assert_eq!(heard_notes(&mut j), vec![(false, ItemNote::InventoryFull.to_wire())], "the note reaches the client");
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction");
    j.assert_honest("a give into a full bag");
    assert_eq!(tally(&j), (0, 0, 1));
}

#[test]
fn c3d_client_strict_device_right_click_is_served() {
    let mut j = rig("item-device", &[]);
    let (cell, d) = lever(&mut j, 2);
    assert!(d < 4.0, "in plain reach: {d}");
    let action = device(&mut j, cell, 0);
    j.pump();
    assert!(lever_on(&j, cell), "the lever flipped on the server");
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "served");
    j.assert_honest("device");
    assert_eq!(tally(&j), (0, 0, 0));
}

/// A lever about 7 blocks off: out of plain reach. The server copy's named
/// slot holds a Reach Claw → served; the claw elsewhere in the bag (the
/// named slot empty) while the movement packet's claimed hand says Reach
/// Claw → refused, a world fact, never torn down. A server that reached
/// with the claimed hand would flip it.
#[test]
fn c3d_client_strict_device_reach_is_the_server_copys_named_slot() {
    let claw = mat(MaterialId::ReachClaw, 1);
    let mut j = rig("item-device-claw", &[(3, claw)]);
    let (cell, d) = lever(&mut j, 7);
    assert!(d > 6.4 && d < 8.3, "beyond plain reach, inside a claw's: {d}");
    let (kind, id) = crate::inventory::item_to_ref(&Item::Material(MaterialId::ReachClaw)).to_wire();
    let sp = &mut j.hs.server.players[j.i];
    (sp.held_kind, sp.held_id) = (kind, id);
    let refused = device(&mut j, cell, 0);
    j.pump();
    assert!(!lever_on(&j, cell), "not flipped by the claimed hand");
    assert_eq!(fate(&j, refused), Some(ActionFate::Refused), "refused");
    j.assert_honest("a device out of reach");
    assert_eq!(tally(&j), (0, 0, 1), "a world fact");
    let served = device(&mut j, cell, 3);
    j.pump();
    assert!(lever_on(&j, cell), "flipped with the copy's claw in the named slot");
    assert_eq!(fate(&j, served), Some(ActionFate::Accepted), "served");
    j.assert_honest("a device in a claw's reach");
}

/// Mirror keeps the legacy requests: `route_item` and `route_device`
/// record and reserve nothing.
#[test]
fn c3d_client_mirror_eat_give_and_device_stay_legacy() {
    use super::c3d_grid_tests::joined as grid_joined;
    let mut j = grid_joined("item-mirror", InventoryMode::Mirror, None);
    let mut actions = JoinerActions::default();
    let pending = Pending { kind: Asked::CreativeGive, mob: None, hotbar_slot: 0, held: None };
    let packet = protocol::ItemActionPacket { seq: 0, action: protocol::ItemAction::CreativeGive { item: crate::inventory::stack_to_wire(&ItemStack::new_block(crate::block::STONE, 1)) }, events_applied: 0 };
    assert_eq!(route_item(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut actions, pending, packet).expect("routed"), None, "Mirror: the legacy give");
    assert_eq!(route_device(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, [0, 80, 0], 0).expect("routed"), None, "Mirror: the legacy right-click");
    assert_eq!(actions.len(), 0, "Mirror: nothing recorded");
    assert!(j.rc.action_journal.as_ref().is_none_or(|journal| journal.len() == 0), "Mirror: nothing reserved");
    j.assert_live("Mirror: never torn down");
}

/// The legacy packets that touch the inventory or the world still refuse a
/// Strict seat (fail closed, the `Untyped` reason): a `DeviceInteract`, a
/// legacy `ItemAction` (an eat, a drop) and a legacy `EntityInteract`.
#[test]
fn c3d_client_strict_seat_sending_a_legacy_packet_is_refused() {
    let bread_full = crate::inventory::item_to_wire_full(&Item::Material(MaterialId::Bread));
    let (bk, bi) = crate::inventory::item_to_ref(&Item::Material(MaterialId::Bread)).to_wire();
    for (n, what) in ["device", "eat", "drop", "interact"].into_iter().enumerate() {
        let mut j = rig(&format!("item-legacy-{n}"), &[(0, mat(MaterialId::Bread, 4))]);
        match what {
            "device" => j.rc.send_device_interact((40, 81, 40)),
            "eat" => j.rc.send_item_action(&protocol::ItemActionPacket { seq: 1, action: protocol::ItemAction::Eat { hotbar_slot: 0, held_kind: bk, held_id: bi, held_full: bread_full }, events_applied: 0 }),
            "drop" => j.rc.send_item_action(&protocol::ItemActionPacket { seq: 1, action: protocol::ItemAction::Drop { hotbar_slot: 0, held_kind: bk, held_id: bi, held_full: bread_full }, events_applied: 0 }),
            _ => j.rc.send_entity_interact(&protocol::EntityInteractPacket { seq: 1, entity: 1, kind: protocol::InteractKind::Feed, held_kind: bk, held_id: bi, held_full: bread_full, hotbar_slot: 0, sneak: false, events_applied: 0 }),
        }
        for _ in 0..3 { j.hs.tick(); j.rc.poll(); }
        assert!(j.hs.slot_is_free(j.i), "{what}: a legacy packet refuses the Strict seat");
        assert!(matches!(&j.rc.state, ConnectionState::Failed(reason) if reason.contains(crate::strict_admission::StrictTeardown::Untyped.reason())), "{what}: with the Untyped reason: {}", match &j.rc.state { ConnectionState::Failed(r) => r.clone(), _ => "not failed".into() });
    }
}
