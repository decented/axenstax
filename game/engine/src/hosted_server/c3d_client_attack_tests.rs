//! C3d run 15 — a Strict seat's typed entity attack, on a REAL
//! `RemoteClient` joined to a Strict `HostedServer` (the
//! `c3d_client_edit_tests` rig), routed as `GameState::send_entity_attack`
//! routes it (`strict_requests::route_attack`), and its caps through the raw
//! dispatch rig (`c3d_strict_tests`). The damage is the server copy's item,
//! never the claim; a tool's wear is one numbered Wear; every refusal has
//! zero external effect and is corrected, never torn down; Mirror keeps its
//! legacy request.
use super::c3d_client_edit_tests::{joined, Joined};
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::action_wire::ActionIntent;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack};
use crate::joiner_actions::{Asked, JoinerActions, Pending};
use crate::mob::MobType;
use crate::protocol;
use crate::remote_client::{ConnectionState, RequestOutcome};
use crate::strict_requests::{route_attack, AttackRecipe};
use crate::transport::ClientTransport as _;
use glam::Vec3;

fn sword(material: ToolMaterial) -> Tool { Tool::new(ToolType::Sword, material) }
fn held(tool: Tool) -> ItemStack { ItemStack { item: Item::Tool(tool), count: 1 } }

/// Two blocks ahead of a body at `pos` facing yaw 0 (−z), on its floor.
fn ahead(pos: Vec3, blocks: f32) -> Vec3 { pos + Vec3::new(0.0, 0.0, -blocks) }

/// A cow in the server's world at `at`, broadcast once (so it has an id).
fn spawn_cow(hs: &mut super::HostedServer, at: Vec3) -> (hecs::Entity, u32) {
    let e = crate::entity::spawn_mob(&mut hs.server.ecs, MobType::Cow, at);
    hs.tick();
    let id = hs.server.ecs.get::<&crate::entity::ProtocolId>(e).expect("broadcast").0;
    (e, id)
}

/// Put the cow back at `at`, at rest; the joiner's body faces it (yaw 0).
fn place(hs: &mut super::HostedServer, i: usize, e: hecs::Entity, at: Vec3) {
    hs.server.ecs.get::<&mut crate::entity::Position>(e).unwrap().0 = at;
    hs.server.ecs.get::<&mut crate::entity::Velocity>(e).unwrap().0 = Vec3::ZERO;
    let sp = &mut hs.server.players[i];
    sp.yaw = 0.0;
    sp.pitch = 0.0;
}

/// What a hit leaves on the cow: health, whether it is stamped as hit, its
/// height (a knockback pops it off the floor).
fn cow_state(hs: &super::HostedServer, e: hecs::Entity) -> (f32, bool, f32) {
    let ecs = &hs.server.ecs;
    let health = ecs.get::<&crate::combat::Health>(e).unwrap().current;
    let stamped = ecs.get::<&crate::combat::LastAttacker>(e).is_ok();
    (health, stamped, ecs.get::<&crate::entity::Position>(e).unwrap().0.y)
}

/// The swing `GameState::send_entity_attack` builds from hotbar `hot`.
fn swing_of(inv: &crate::inventory::Inventory, hot: usize, entity: u32) -> (Pending, protocol::EntityAttackPacket) {
    let item = inv.slot(hot).map(|s| s.item.clone());
    let (held_kind, held_id) = item.as_ref().map_or(protocol::ItemRef::Empty.to_wire(), |i| crate::inventory::item_to_ref(i).to_wire());
    let held_full = item.as_ref().map_or(protocol::WireItem::None, crate::inventory::item_to_wire_full);
    let pending = Pending { kind: Asked::Swing, mob: Some(MobType::Cow), hotbar_slot: hot, held: item };
    let packet = protocol::EntityAttackPacket { seq: 0, entity, held_kind, held_id, held_full, sprint: false, sneak: false, hotbar_slot: hot as u8, events_applied: 0 };
    (pending, packet)
}

/// Swing from hotbar `hot` at `entity` on the Strict rig, routed as the
/// game loop does.
fn attack(j: &mut Joined, hot: usize, entity: u32) -> ActionHeader {
    let (pending, packet) = swing_of(&j.inv, hot, entity);
    route_attack(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut j.actions, pending, packet).expect("routed").expect("a Strict seat sends the attack typed")
}

fn fate(j: &Joined, action: ActionHeader) -> Option<ActionFate> {
    j.sp().action_results.as_ref().expect("a joined result history").peek(action).map(|r| r.fate)
}

fn durability(inv: &crate::inventory::Inventory, slot: usize) -> Option<u16> {
    match inv.slot(slot).map(|s| &s.item) { Some(Item::Tool(t)) => Some(t.durability), _ => None }
}

/// A Strict rig with the refill setting already agreed and a cow two
/// blocks ahead of the joiner.
fn rig(tag: &str, items: &[(usize, ItemStack)], cow_blocks: f32) -> (Joined, hecs::Entity, u32, Vec3) {
    let mut j = joined(tag, items, true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the refill setting");
    let at = ahead(j.sp().player.pos, cow_blocks);
    let (cow, id) = spawn_cow(&mut j.hs, at);
    place(&mut j.hs, j.i, cow, at);
    (j, cow, id, at)
}

/// An honest Strict hit: damage from the server copy's sword, ONE numbered
/// Wear, an Accepted row, the client's sword worn by the server's outcome
/// (nothing predicted before it), the windows agreeing, never torn down.
#[test]
fn c3d_client_strict_hit_is_served_from_the_server_item() {
    let iron = sword(ToolMaterial::Iron);
    let (mut j, cow, id, at) = rig("attack-served", &[(0, held(iron))], 2.0);
    let (max, _, y0) = cow_state(&j.hs, cow);
    let before = j.sp().window_events.last_number();
    let action = attack(&mut j, 0, id);
    assert_eq!(durability(&j.inv, 0), Some(iron.durability), "no wear predicted");
    let recipe: AttackRecipe = bincode::deserialize(&j.rc.action_journal.as_ref().unwrap().actions().last().unwrap().recipe).expect("an attack recipe");
    assert_eq!(recipe.slot, 0);
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    let (health, stamped, y) = cow_state(&j.hs, cow);
    assert_eq!(health, max - Item::Tool(iron).attack_damage(), "the server's sword's damage");
    assert!(stamped, "the cow names its attacker");
    assert!(y > y0, "the hit's knockback ({y} > {y0})");
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "served");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one numbered Wear");
    j.assert_honest("the served hit");
    assert_eq!(durability(&j.inv, 0), Some(iron.durability - 1), "the server's Wear applied on the client");
    assert_eq!(durability(&j.sp().inventory, 0), Some(iron.durability - 1), "and on the server copy");
    assert_eq!((j.sp().strict_refusals.stale_world, j.sp().strict_refusals.other), (0, 0));
}

/// A bare-hand hit: damage 1, no event, Accepted, never torn down.
#[test]
fn c3d_client_strict_bare_hand_hit_wears_nothing() {
    let (mut j, cow, id, at) = rig("attack-bare", &[], 2.0);
    let (max, _, _) = cow_state(&j.hs, cow);
    let before = j.sp().window_events.last_number();
    let action = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    assert_eq!(cow_state(&j.hs, cow).0, max - 1.0, "a bare hand's damage");
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted));
    assert_eq!(j.sp().window_events.last_number(), before, "no event");
    j.assert_honest("the bare-hand hit");
}

/// The client claims a diamond sword; the server's copy holds a wooden
/// one: refused (a Rule refusal) with zero effect (no damage, knockback,
/// wear or schedule move), corrected by ONE `Replace` (the client's slot is
/// the wooden sword again), never torn down.
#[test]
fn c3d_client_strict_claimed_better_sword_is_refused_and_corrected() {
    let wood = sword(ToolMaterial::Wood);
    let (mut j, cow, id, at) = rig("attack-claim", &[(0, held(wood))], 2.0);
    j.inv.set_slot(0, Some(held(sword(ToolMaterial::Diamond))));
    let (max, _, y0) = cow_state(&j.hs, cow);
    let schedule = j.sp().next_swing_tick;
    let before = j.sp().window_events.last_number();
    let action = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    // The effect first: a server that took the claim's damage is red here.
    let (health, stamped, y) = cow_state(&j.hs, cow);
    assert_eq!(health, max, "no damage (the claimed diamond sword's would be {})", max - Item::Tool(sword(ToolMaterial::Diamond)).attack_damage());
    assert!(!stamped, "nothing names the joiner");
    assert!(y <= y0 + 1e-3, "no knockback ({y} vs {y0})");
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(j.sp().next_swing_tick, schedule, "the swing schedule didn't move");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace, no Wear");
    assert!(!j.hs.slot_is_free(j.i), "never torn down");
    j.assert_honest("the refused claim");
    assert_eq!(j.inv.slot(0).cloned(), Some(held(wood)), "the client's slot corrected to the server's sword");
    assert_eq!((j.sp().strict_refusals.stale_world, j.sp().strict_refusals.other), (0, 1));
}

/// Out of reach (ten blocks ahead): refused by the shared Mirror swing
/// rule, zero effect, corrected, never torn down.
#[test]
fn c3d_client_strict_out_of_reach_hit_is_refused_and_corrected() {
    let iron = sword(ToolMaterial::Iron);
    let (mut j, cow, id, at) = rig("attack-reach", &[(0, held(iron))], 10.0);
    let (max, _, y0) = cow_state(&j.hs, cow);
    let schedule = j.sp().next_swing_tick;
    let before = j.sp().window_events.last_number();
    let action = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    let (health, stamped, y) = cow_state(&j.hs, cow);
    assert_eq!(health, max, "no damage");
    assert!(!stamped, "nothing names the joiner");
    assert!(y <= y0 + 1e-3, "no knockback ({y} vs {y0})");
    assert_eq!(j.sp().next_swing_tick, schedule, "the swing schedule didn't move");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace, no Wear");
    j.assert_honest("the out-of-reach hit");
    assert_eq!(durability(&j.inv, 0), Some(iron.durability), "no wear");
    assert_eq!(durability(&j.sp().inventory, 0), Some(iron.durability), "no wear on the server copy");
}

/// A sword on its last point breaks as in single-player: the hit lands,
/// and the server's Wear empties the slot on both copies.
#[test]
fn c3d_client_strict_last_point_sword_breaks_on_both_copies() {
    let mut worn = sword(ToolMaterial::Wood);
    worn.durability = 1;
    let (mut j, cow, id, at) = rig("attack-break", &[(0, held(worn))], 2.0);
    let (max, _, _) = cow_state(&j.hs, cow);
    let action = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    assert_eq!(cow_state(&j.hs, cow).0, max - Item::Tool(worn).attack_damage(), "the hit lands");
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted));
    j.assert_honest("the breaking hit");
    assert!(j.inv.slot(0).is_none() && j.sp().inventory.slot(0).is_none(), "broken on both copies");
}

/// Mirror is unchanged: `route_attack` records and sends nothing (the game
/// loop sends its legacy request), and the legacy request is served as
/// before: the claimed weapon's damage, an accepted outcome naming the Wear.
#[test]
fn c3d_client_mirror_attack_stays_the_legacy_request() {
    use super::c3d_grid_tests::joined as grid_joined;
    let mut j = grid_joined("attack-mirror", InventoryMode::Mirror, None);
    let iron = sword(ToolMaterial::Iron);
    j.give(0, held(iron));
    j.pumps(3);
    let at = ahead(j.hs.server.players[j.i].player.pos, 2.0);
    let (cow, id) = spawn_cow(&mut j.hs, at);
    let (max, _, _) = cow_state(&j.hs, cow);
    let mut actions = JoinerActions::default();
    let (pending, packet) = swing_of(&j.inv, 0, id);
    let routed = route_attack(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut actions, pending.clone(), packet.clone()).expect("routed");
    assert_eq!(routed, None, "Mirror: the legacy request");
    assert_eq!(actions.len(), 0, "Mirror: route_attack records nothing");
    assert!(j.rc.action_journal.as_ref().is_none_or(|journal| journal.len() == 0), "Mirror: nothing reserved");
    let seq = actions.record(pending, j.rc.next_input_seq());
    place(&mut j.hs, j.i, cow, at);
    j.rc.send_entity_attack(&protocol::EntityAttackPacket { seq, ..packet });
    j.hs.tick();
    assert_eq!(cow_state(&j.hs, cow).0, max - Item::Tool(iron).attack_damage(), "Mirror: the claimed sword's damage, as before");
    j.rc.poll();
    let outcome = j.rc.pending_outcomes.iter().find_map(|o| match o { RequestOutcome::Interact(out) => Some(out.clone()), _ => None }).expect("the legacy outcome");
    assert!(outcome.accepted && outcome.window_event > 0, "Mirror: the same outcome, its Wear numbered: {outcome:?}");
    j.assert_live("Mirror: never torn down");
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }));
}

// ---- caps, through the raw dispatch rig ----

use super::c3d_strict_tests::{strict_rig, Rig};

/// The joiner of a raw rig with an iron sword in slot 0 and a cow two
/// blocks ahead of it.
fn armed(tag: &str) -> (Rig, hecs::Entity, u32, Vec3) {
    let mut r = strict_rig(tag);
    r.hs.server.players[r.i].inventory.set_slot(0, Some(held(sword(ToolMaterial::Iron))));
    let at = ahead(r.sp().player.pos, 2.0);
    let (cow, id) = spawn_cow(&mut r.hs, at);
    (r, cow, id, at)
}

fn raw_attack(r: &mut Rig, cow: hecs::Entity, id: u32, at: Vec3) -> ActionHeader {
    let action = r.next();
    let (_, mut packet) = swing_of(&r.sp().inventory.clone(), 0, id);
    packet.seq = action.seq as u32;
    packet.events_applied = action.events_applied;
    let i = r.i;
    // The raw rig has no floor: the body and the cow are put back where
    // they stood before the swing is read.
    let _ = at;
    let pos = r.sp().player.pos;
    r.hs.server.players[i].player.velocity = Vec3::ZERO;
    place(&mut r.hs, i, cow, ahead(pos, 2.0));
    r.send(action, ActionIntent::Attack(packet));
    action
}

/// The interact outcomes and the rest the raw client heard.
fn heard(r: &Rig) -> (Vec<protocol::InteractOutcomePacket>, super::c3d_tests::Heard) {
    let mut out = Vec::new();
    let mut heard = super::c3d_tests::Heard::default();
    while let Some(pkt) = r.client.try_recv_from_server() {
        match protocol::deserialize_header(&pkt) {
            Some((protocol::PacketType::InteractOutcome, p)) => out.push(protocol::safe_deserialize(p).unwrap()),
            Some((protocol::PacketType::JoinReject, p)) => heard.rejects.push(protocol::safe_deserialize::<protocol::JoinRejectPacket>(p).unwrap().reason),
            Some((protocol::PacketType::ActionReceipt, p)) => heard.receipts.push(protocol::safe_deserialize(p).unwrap()),
            _ => {}
        }
    }
    (out, heard)
}

#[derive(Debug, PartialEq)]
struct AttackSnapshot {
    sword: Vec<protocol::WireSlot>,
    last: u32,
    capacity: usize,
    through: u64,
    result_bytes: usize,
    health: f32,
    stamped: bool,
    next_swing_tick: u64,
}

fn snapshot(r: &Rig, cow: hecs::Entity) -> AttackSnapshot {
    let sp = r.sp();
    let (health, stamped, _) = cow_state(&r.hs, cow);
    AttackSnapshot {
        sword: super::c3d_strict_tests::server_places(sp)[..1].to_vec(),
        last: sp.window_events.last_number(),
        capacity: sp.window_events.waiting_capacity(),
        through: r.through(),
        result_bytes: sp.action_results.as_ref().unwrap().retained_bytes(),
        health,
        stamped,
        next_swing_tick: sp.next_swing_tick,
    }
}

/// An attack refused at a cap: no damage, knockback stamp, wear, event,
/// row or schedule move; nothing answered; torn down with a reason (fail
/// closed, exactly as a shot at its caps).
fn assert_zero_effect_attack(mut r: Rig, cow: hecs::Entity, id: u32, at: Vec3, why: &str, detail: &str) {
    let before = snapshot(&r, cow);
    let action = raw_attack(&mut r, cow, id, at);
    let (outcome, heard) = heard(&r);
    assert_eq!(snapshot(&r, cow), before, "{why}: zero external effect");
    assert!(outcome.is_empty() && heard.receipts.is_empty(), "{why}: nothing answered");
    assert_eq!(r.row(action), None, "{why}: no result row");
    assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::FellBehind.reason()), "{why}: teardown reason {:?}", heard.rejects);
    assert_eq!(super::actions::teardown_detail::last(), detail, "the internal reason");
}

/// The 1024 event cap with a one-event attack: at 1023 pending it fits
/// (1024) and is Accepted; at 1024 it would need 1025: zero effect.
#[test]
fn c3d_strict_attack_at_the_event_cap_has_zero_external_effect() {
    for (pending, fits) in [(1023u32, true), (1024, false)] {
        let (mut r, cow, id, at) = armed(&format!("attack-cap-{pending}"));
        let sp = &mut r.hs.server.players[r.i];
        for _ in 0..pending {
            crate::window_events::queue(sp, crate::window_events::WindowEvent::WearArmour { hits: 1 }, 0);
        }
        if fits {
            let action = raw_attack(&mut r, cow, id, at);
            let (outcome, heard) = heard(&r);
            assert!(heard.rejects.is_empty(), "1023 pending: admitted, {:?}", heard.rejects);
            assert_eq!(r.sp().window_events.waiting(), 1024, "exactly at the cap");
            assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Accepted));
            assert_eq!(outcome.len(), 1, "one outcome");
        } else {
            assert_zero_effect_attack(r, cow, id, at, "1024 pending", "Whole attack event reservation is unavailable: the pending-event count cap");
        }
    }
}

/// The result cap (a full unacknowledged result history: the Wear is
/// reserved first, then given back exactly) leaves zero effect.
#[test]
fn c3d_strict_attack_at_the_result_cap_has_zero_external_effect() {
    let (mut r, cow, id, at) = armed("attack-results");
    r.fill_results();
    r.hs.server.players[r.i].window_events.shrink_waiting_to_fit();
    assert_zero_effect_attack(r, cow, id, at, "result cap", "Attack result history is full");
}
