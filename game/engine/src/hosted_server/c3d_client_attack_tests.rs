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

pub(super) fn sword(material: ToolMaterial) -> Tool { Tool::new(ToolType::Sword, material) }
pub(super) fn held(tool: Tool) -> ItemStack { ItemStack { item: Item::Tool(tool), count: 1 } }

/// Two blocks ahead of a body at `pos` facing yaw 0 (−z), on its floor.
pub(super) fn ahead(pos: Vec3, blocks: f32) -> Vec3 { pos + Vec3::new(0.0, 0.0, -blocks) }

/// A cow in the server's world at `at`, broadcast once (so it has an id).
pub(super) fn spawn_cow(hs: &mut super::HostedServer, at: Vec3) -> (hecs::Entity, u32) {
    let e = crate::entity::spawn_mob(&mut hs.server.ecs, MobType::Cow, at);
    hs.tick();
    let id = hs.server.ecs.get::<&crate::entity::ProtocolId>(e).expect("broadcast").0;
    (e, id)
}

/// Put the cow back at `at`, at rest; the joiner's body faces it (yaw 0).
pub(super) fn place(hs: &mut super::HostedServer, i: usize, e: hecs::Entity, at: Vec3) {
    hs.server.ecs.get::<&mut crate::entity::Position>(e).unwrap().0 = at;
    hs.server.ecs.get::<&mut crate::entity::Velocity>(e).unwrap().0 = Vec3::ZERO;
    let sp = &mut hs.server.players[i];
    sp.yaw = 0.0;
    sp.pitch = 0.0;
}

/// What a hit leaves on the cow: health, whether it is stamped as hit, its
/// height (a knockback pops it off the floor).
pub(super) fn cow_state(hs: &super::HostedServer, e: hecs::Entity) -> (f32, bool, f32) {
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
pub(super) fn attack(j: &mut Joined, hot: usize, entity: u32) -> ActionHeader {
    let (pending, packet) = swing_of(&j.inv, hot, entity);
    route_attack(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut j.actions, pending, packet).expect("routed").expect("a Strict seat sends the attack typed")
}

pub(super) fn fate(j: &Joined, action: ActionHeader) -> Option<ActionFate> {
    j.sp().action_results.as_ref().expect("a joined result history").peek(action).map(|r| r.fate)
}

pub(super) fn durability(inv: &crate::inventory::Inventory, slot: usize) -> Option<u16> {
    match inv.slot(slot).map(|s| &s.item) { Some(Item::Tool(t)) => Some(t.durability), _ => None }
}

/// A Strict rig with the refill setting already agreed and a cow two
/// blocks ahead of the joiner.
pub(super) fn rig(tag: &str, items: &[(usize, ItemStack)], cow_blocks: f32) -> (Joined, hecs::Entity, u32, Vec3) {
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

/// C3d run 17 (run-15 review LOW-3) — the one place a Strict action kills
/// its own actor mid-commit: a joiner at 1 HP hits an adult wild Nostrich
/// with an iron sword and its kick-back kills it. The swing is served
/// (ONE numbered Wear, reserved before the damage, committed with the
/// swing), the debt-zero fold still holds after the commit (never torn
/// down for an obligation), and the death that follows is the existing
/// "Wear pending at death" case: the death `Replace` comes after the Wear,
/// and the sword (worn exactly once) is conserved over the window, every
/// grave and the ground.
#[test]
fn c3d_client_strict_attacker_killed_by_the_nostrich_kick_back_conserves_the_worn_sword() {
    let iron = sword(ToolMaterial::Iron);
    let mut j = joined("attack-kick", &[(0, held(iron))], true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the refill setting");
    let at = ahead(j.sp().player.pos, 1.5);
    let e = crate::entity::spawn_mob(&mut j.hs.server.ecs, MobType::Nostrich, at);
    j.hs.tick();
    let id = j.hs.server.ecs.get::<&crate::entity::ProtocolId>(e).expect("broadcast").0;
    let nowhere = [0, -200, 0];
    let before_hold = super::c3d_container_tests::all_holdings(&j.hs, &[j.i], nowhere);
    let mut worn = iron;
    worn.durability -= 1;
    let expect_worn = super::c3d_strict_tests::holdings([Some(crate::inventory::stack_to_wire(&held(worn)))].iter());
    assert_ne!(before_hold, expect_worn, "the sword starts unworn");
    j.hs.server.players[j.i].combat.health = 1.0;
    j.hs.server.players[j.i].combat.invincible_timer = 0;
    let before = j.sp().window_events.last_number();
    let action = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, e, at);
    j.hs.tick();
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "the swing is served");
    assert!(!j.hs.slot_is_free(j.i), "the fold held: never torn down for an obligation");
    let sp = j.sp();
    assert!(sp.combat.dead, "the kick-back killed the attacker");
    assert!(sp.window_events.last_number() >= before + 1, "the Wear is numbered");
    let replace = sp.window_events.death_replace.expect("the death Replace is queued");
    assert!(replace > before + 1, "the death Replace ({replace}) follows the Wear ({})", before + 1);
    j.pump();
    j.pump();
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }), "never torn down");
    assert_eq!(super::c3d_container_tests::all_holdings(&j.hs, &[j.i], nowhere), expect_worn, "the sword, worn exactly once, over window + graves + ground");
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
    assert_eq!(j.actions.len(), 0, "the refused attack's request record is released (run-15 review LOW-2)");
    assert_eq!(j.inv.slot(0).cloned(), Some(held(wood)), "the client's slot corrected to the server's sword");
    assert_eq!((j.sp().strict_refusals.stale_world, j.sp().strict_refusals.other), (0, 1));
}

/// Out of reach (ten blocks ahead): refused by the shared Mirror swing
/// rule for a WORLD FACT (run 16): zero effect, its `Refused` receipt
/// alone, no correction `Replace` (the client predicted nothing), never
/// torn down. (Run 15's `..._is_refused_and_corrected` asserted a `Replace`
/// here.)
#[test]
fn c3d_client_strict_out_of_reach_hit_is_refused_uncorrected() {
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
    assert_eq!(j.sp().window_events.last_number(), before, "no correction Replace, no Wear");
    assert!(!j.sp().window_events.replace_waiting(), "no correction waits");
    j.assert_honest("the out-of-reach hit");
    assert_eq!(j.actions.len(), 0, "the refused attack's request record is released (run-15 review LOW-2)");
    assert_eq!(durability(&j.inv, 0), Some(iron.durability), "no wear");
    assert_eq!(durability(&j.sp().inventory, 0), Some(iron.durability), "no wear on the server copy");
    let t = j.sp().strict_refusals;
    assert_eq!((t.stale_world, t.other, t.world_fact), (0, 0, 1), "a world fact, never a window mismatch");
}

/// Run 16 — nine out-of-reach swings inside 100 ticks, one every swing
/// cooldown (one past the correction bound's eight): each is a receipt alone, so no `Replace`, no
/// tick of the bound and no teardown; zero effect on the cow and the
/// sword; the client's window is untouched by every routed swing.
#[test]
fn c3d_client_strict_nine_misses_in_100_ticks_never_tear_down() {
    let iron = sword(ToolMaterial::Iron);
    let (mut j, cow, id, at) = rig("attack-misses", &[(0, held(iron))], 10.0);
    let (max, _, _) = cow_state(&j.hs, cow);
    let before = j.sp().window_events.last_number();
    let start = j.hs.server.tick_counter;
    for n in 1..=9 {
        let window = j.ui.digest(&j.inv, &j.armour);
        let action = attack(&mut j, 0, id);
        assert_eq!(j.ui.digest(&j.inv, &j.armour), window, "swing {n}: routing predicted nothing");
        // Attack held: a swing every `ATTACK_COOLDOWN` ticks, each answer
        // applied before the next (the run-15 review's own-pet / reach-edge
        // case: nine refusals in about 81 ticks).
        place(&mut j.hs, j.i, cow, at);
        j.pump();
        assert_eq!(fate(&j, action), Some(ActionFate::Refused), "swing {n}: refused");
        for _ in 1..crate::combat::ATTACK_COOLDOWN { place(&mut j.hs, j.i, cow, at); j.pump(); }
        assert!(matches!(j.rc.state, ConnectionState::Connected { .. }) && !j.hs.slot_is_free(j.i), "swing {n}: never torn down");
    }
    assert!(j.hs.server.tick_counter - start < crate::strict_admission::CORRECTION_WINDOW_TICKS, "inside one correction window");
    assert_eq!(j.sp().window_events.last_number(), before, "no Replace, no Wear");
    assert_eq!(cow_state(&j.hs, cow).0, max, "zero effect on the cow");
    j.assert_honest("nine misses");
    assert_eq!(durability(&j.sp().inventory, 0), Some(iron.durability), "zero effect on the sword");
    assert_eq!(j.sp().strict_refusals.world_fact, 9);
}

/// Run 16 — two swings in one tick: the second is early on the server's
/// swing schedule, a world fact (receipt alone, no `Replace`); the first
/// lands and wears once.
#[test]
fn c3d_client_strict_swing_early_on_the_schedule_is_refused_uncorrected() {
    let iron = sword(ToolMaterial::Iron);
    let (mut j, cow, id, at) = rig("attack-early", &[(0, held(iron))], 2.0);
    let (max, _, _) = cow_state(&j.hs, cow);
    let before = j.sp().window_events.last_number();
    let first = attack(&mut j, 0, id);
    let second = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    assert_eq!((fate(&j, first), fate(&j, second)), (Some(ActionFate::Accepted), Some(ActionFate::Refused)));
    assert_eq!(cow_state(&j.hs, cow).0, max - Item::Tool(iron).attack_damage(), "one hit landed");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "the first swing's Wear, no Replace");
    j.assert_honest("the early swing");
    assert_eq!(durability(&j.inv, 0), Some(iron.durability - 1), "worn once");
    let t = j.sp().strict_refusals;
    assert_eq!((t.other, t.world_fact), (0, 1));
}

/// Run 16 — the joiner's body dies with a swing in flight: the swing is
/// `Superseded` (before the attack arm runs), zero effect, never torn
/// down; not a world-fact refusal.
#[test]
fn c3d_client_strict_swing_from_a_dead_body_is_superseded() {
    let iron = sword(ToolMaterial::Iron);
    let (mut j, cow, id, at) = rig("attack-dead", &[(0, held(iron))], 2.0);
    let (max, _, y0) = cow_state(&j.hs, cow);
    let action = attack(&mut j, 0, id);
    let sp = &mut j.hs.server.players[j.i];
    sp.combat.health = 0.0;
    sp.combat.dead = true;
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Superseded), "superseded");
    let (health, stamped, y) = cow_state(&j.hs, cow);
    assert_eq!((health, stamped), (max, false), "zero effect on the cow");
    assert!(y <= y0 + 1e-3, "no knockback ({y} vs {y0}; run-16 review LOW-5: was compared with itself)");
    assert_eq!(durability(&j.sp().inventory, 0), Some(iron.durability), "no wear");
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }) && !j.hs.slot_is_free(j.i), "never torn down");
    let t = j.sp().strict_refusals;
    assert_eq!((t.stale_world, t.other, t.world_fact), (0, 0, 0), "superseded, not refused");
}

/// Run 16 — a sword on its last point swung twice inside one round trip:
/// the first hit lands and its Wear (which breaks the sword) waits for the
/// client; the second swing, made before the client applied it and past
/// the swing schedule, names a sword the server's effective window no
/// longer holds. A corrected refusal (one `Replace`), never a teardown;
/// the sword is gone on both copies.
#[test]
fn c3d_client_strict_last_point_sword_swung_twice_in_one_round_trip_is_corrected() {
    let mut worn = sword(ToolMaterial::Wood);
    worn.durability = 1;
    let (mut j, cow, id, at) = rig("attack-break-twice", &[(0, held(worn))], 2.0);
    let (max, _, _) = cow_state(&j.hs, cow);
    let before = j.sp().window_events.last_number();
    let first = attack(&mut j, 0, id);
    // Past the swing schedule, with the client reading nothing.
    for _ in 0..8 { place(&mut j.hs, j.i, cow, at); j.hs.tick(); }
    assert_eq!(fate(&j, first), Some(ActionFate::Accepted));
    assert!(j.inv.slot(0).is_some(), "the client still shows the sword (its Wear unread)");
    let second = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.pump();
    assert_eq!(fate(&j, second), Some(ActionFate::Refused), "refused");
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }) && !j.hs.slot_is_free(j.i), "never torn down");
    assert_eq!(cow_state(&j.hs, cow).0, max - Item::Tool(worn).attack_damage(), "only the first hit landed");
    assert_eq!(j.sp().window_events.last_number(), before + 2, "the Wear, then ONE correction Replace");
    j.assert_honest("the last point swung twice");
    assert!(j.inv.slot(0).is_none() && j.sp().inventory.slot(0).is_none(), "broken on both copies");
    assert_eq!((j.sp().strict_refusals.other, j.sp().strict_refusals.world_fact), (1, 0), "a window disagreement");
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

/// C3d run 17 (run-16 review LOW-1, decided) — the observed guard behind
/// `ActionIntent::predicts_nothing`: an honest Strict swing leaves the
/// joined window's digest as it was until its answer, and a predicting path
/// (the weapon worn locally, as a single-player hit does) trips it.
// Run-18 review M2: `NoPrediction` is debug-only, so this runs only
// under `debug_assertions` (`./check.sh --release` skips it).
#[cfg(debug_assertions)]
#[test]
fn c3d_strict_no_prediction_guard_trips_on_a_predicted_swing_wear() {
    use crate::strict_requests::NoPrediction;
    let iron = sword(ToolMaterial::Iron);
    let (mut j, _cow, id, _at) = rig("attack-guard", &[(0, held(iron))], 2.0);
    let guard = NoPrediction::begin(Some(&j.rc), &j.ui, &j.inv, &j.armour, "swing").expect("a Strict seat is guarded");
    attack(&mut j, 0, id);
    assert_eq!(guard.end(&j.ui, &j.inv, &j.armour), Ok(()), "an honest swing predicts nothing");
    let guard = NoPrediction::begin(Some(&j.rc), &j.ui, &j.inv, &j.armour, "swing").expect("guarded");
    attack(&mut j, 0, id);
    j.inv.use_hotbar_tool(0);
    let error = guard.end(&j.ui, &j.inv, &j.armour).expect_err("a predicted wear trips the guard");
    assert!(error.contains("Strict swing changed the joined window"), "{error}");
}

/// C3d run 17 (run-16 review LOW-5) — the action right behind a world-fact
/// refusal, sent in the same round trip, is SERVED (not `Superseded`): no
/// correction `Replace` came between them, so its base is still current.
#[test]
fn c3d_client_strict_action_behind_a_world_fact_refusal_is_served() {
    let iron = sword(ToolMaterial::Iron);
    let stone = ItemStack::new_block(crate::block::STONE, 4);
    let (mut j, _cow, id, _at) = rig("attack-behind", &[(0, held(iron)), (1, stone)], 10.0);
    let miss = attack(&mut j, 0, id);
    j.place(super::c3d_client_edit_tests::CELLS[0], 1);
    let placed = j.rc.action_journal.as_ref().unwrap().actions().last().unwrap().header;
    j.pump();
    assert_eq!(fate(&j, miss), Some(ActionFate::Refused), "the miss is refused");
    assert_eq!(fate(&j, placed), Some(ActionFate::Accepted), "the placement behind it is served, not superseded");
    j.assert_honest("a placement behind a miss");
    assert_eq!(j.sp().strict_refusals.world_fact, 1);
}

/// C3d run 17 (run-16 review LOW-5) — a signed-in seat's swing (not
/// sneaking) at its OWN tamed wolf: refused for a world fact (1C
/// no-friendly-fire), its receipt alone, no `Replace`, zero effect.
#[test]
fn c3d_client_strict_swing_at_own_pet_is_refused_uncorrected() {
    let iron = sword(ToolMaterial::Iron);
    let (mut j, _cow, _id, _at) = rig("attack-own-pet", &[(0, held(iron))], 10.0);
    j.hs.server.players[j.i].verified_pubkey = Some([7; 32]);
    let key = j.sp().pet_owner_key().expect("a signed-in seat");
    let at = ahead(j.sp().player.pos, 2.0);
    let wolf = crate::entity::spawn_mob(&mut j.hs.server.ecs, MobType::Wolf, at);
    j.hs.server.ecs.get::<&mut crate::wolf::WolfData>(wolf).expect("a wolf").ownership.owner_pubkey = key;
    j.hs.tick();
    let id = j.hs.server.ecs.get::<&crate::entity::ProtocolId>(wolf).expect("broadcast").0;
    let (max, _, _) = cow_state(&j.hs, wolf);
    let before = j.sp().window_events.last_number();
    let action = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, wolf, at);
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(cow_state(&j.hs, wolf).0, max, "the pet is untouched");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction Replace, no Wear");
    j.assert_honest("a swing at the own pet");
    let t = j.sp().strict_refusals;
    assert_eq!((t.other, t.world_fact), (0, 1));
}

/// C3d run 17 (run-16 review LOW-2, DECIDED: fail-closed for now) — PINS a
/// known honest-client teardown until S3 replay: a last-point sword breaks
/// on swing 1, and before the client applies that Wear an identical-kind
/// sword is granted into the freed slot; swing 2 (claiming the old sword,
/// which the server's raw copy still holds) passes the survivor check on
/// the kind, but staging its Wear on raw and folding the pending Wear finds
/// no unique piece (`AmbiguousWear`): torn down (`FellBehind`) with zero
/// effect. A change that serves or corrects this case must revise this test.
#[test]
fn c3d_strict_last_point_sword_replaced_in_flight_by_an_identical_kind_is_a_known_teardown() {
    let mut last = sword(ToolMaterial::Iron);
    last.durability = 1;
    let (mut j, cow, id, at) = rig("attack-ambiguous", &[(0, held(last))], 2.0);
    let first = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.hs.tick();
    assert_eq!(fate(&j, first), Some(ActionFate::Accepted), "swing 1 is served");
    j.hs.grant_to_joiner(j.i, [held(sword(ToolMaterial::Iron))]);
    j.hs.server.players[j.i].next_swing_tick = 0;
    let (health, _, _) = cow_state(&j.hs, cow);
    let second = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.hs.tick();
    assert!(j.hs.slot_is_free(j.i), "torn down today (fail-closed)");
    // C3d run 18 (run-17 review LOW-4) — the cause named, not the catch-all.
    assert_eq!(super::actions::teardown_detail::last(), "Attack wear is ambiguous (AmbiguousWear)", "the internal reason");
    assert_eq!(cow_state(&j.hs, cow).0, health, "zero effect: no second hit");
    // C3d run 21 (run-18 review L6) — pinned exactly: paused at its
    // frontier, so no row of any fate (never served, refused or superseded).
    assert_eq!(fate(&j, second), None, "the second swing has no result row");
}

/// C3d run 18 (run-17 review LOW-4) — the no-grant twin: without the
/// identical sword granted into the freed slot, the second swing names a
/// weapon the effective window no longer holds: a corrected refusal, never
/// a teardown. Pins the grant as the teardown's cause.
#[test]
fn c3d_strict_last_point_sword_without_a_replacement_is_refused_and_corrected() {
    let mut last = sword(ToolMaterial::Iron);
    last.durability = 1;
    let (mut j, cow, id, at) = rig("attack-ambiguous-twin", &[(0, held(last))], 2.0);
    let first = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.hs.tick();
    assert_eq!(fate(&j, first), Some(ActionFate::Accepted), "swing 1 is served");
    j.hs.server.players[j.i].next_swing_tick = 0;
    let (health, _, _) = cow_state(&j.hs, cow);
    let second = attack(&mut j, 0, id);
    place(&mut j.hs, j.i, cow, at);
    j.hs.tick();
    assert!(!j.hs.slot_is_free(j.i), "never torn down");
    assert_eq!(fate(&j, second), Some(ActionFate::Refused), "refused");
    assert_eq!(cow_state(&j.hs, cow).0, health, "zero effect: no second hit");
    j.assert_honest("the corrected refusal");
}
