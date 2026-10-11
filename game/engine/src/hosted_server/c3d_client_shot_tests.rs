//! C3d run 13 — the client half of a typed shot, on a REAL `RemoteClient`
//! joined to a Strict `HostedServer` (the `c3d_client_edit_tests` rig). The
//! shot is routed as `GameState::send_shot` routes it
//! (`strict_requests::route_shot`); a `Legacy` route is sent exactly as the
//! game loop's `send_use_request` sends it. The client predicts nothing:
//! the ammo and the wear arrive on the server's numbered outcome, applied
//! through the client's own `joiner_actions` path.
use super::c3d_client_edit_tests::{joined, Joined};
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack};
use crate::joiner_actions::{Asked, JoinerActions, Pending};
use crate::protocol::{self, ShotWeapon};
use crate::remote_client::{ConnectionState, RemoteClient, RequestOutcome};
use crate::strict_requests::{route_shot, ShotRecipe, ShotRoute};

fn bow() -> Tool { Tool::new(ToolType::Bow, ToolMaterial::Wood) }
fn arrows(n: u8) -> ItemStack { ItemStack { item: Item::Material(crate::shot::ammo_for(ShotWeapon::Bow)), count: n } }

fn projectiles(j: &Joined) -> usize {
    j.hs.server.ecs.query::<&crate::entity::ProjectileEntity>().iter().count()
}

/// The shot `GameState::send_shot` builds for hotbar `hot`, and its record.
fn shot_of(inv: &crate::inventory::Inventory, hot: usize) -> (Pending, protocol::ItemAction) {
    let item = inv.slot(hot).expect("a weapon in hand").item.clone();
    let Item::Tool(tool) = item.clone() else { panic!("a weapon") };
    let weapon = crate::shot::weapon_of(Some(&item)).expect("a shot weapon");
    let (held_kind, held_id) = crate::inventory::item_to_ref(&item).to_wire();
    let action = protocol::ItemAction::Shoot {
        weapon, hotbar_slot: hot as u8, held_kind, held_id, held_full: crate::inventory::item_to_wire_full(&item),
        yaw: 0.0, pitch: 0.0, charge: crate::shot::max_charge(weapon),
    };
    let pending = Pending { kind: Asked::Shoot { weapon, material: tool.material }, mob: None, hotbar_slot: hot, held: Some(Item::Material(crate::shot::ammo_for(weapon))) };
    (pending, action)
}

/// The legacy request, as `GameState::send_use_request` sends it.
fn send_legacy(rc: &mut RemoteClient, actions: &mut JoinerActions, pending: Pending, action: protocol::ItemAction) {
    let seq = actions.record(pending, rc.next_input_seq());
    rc.send_item_action(&protocol::ItemActionPacket { seq, action, events_applied: 0 });
}

/// Shoot from hotbar `hot` on the Strict rig, routed as the game loop does.
fn shoot(j: &mut Joined, hot: usize) -> Option<ActionHeader> {
    let (pending, action) = shot_of(&j.inv, hot);
    match route_shot(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut j.actions, pending.clone(), &action).expect("routed") {
        ShotRoute::Typed(header) => Some(header),
        ShotRoute::Legacy => { send_legacy(&mut j.rc, &mut j.actions, pending, action); None }
        ShotRoute::Unaffordable => panic!("the ammo is there"),
    }
}

fn fate(j: &Joined, action: ActionHeader) -> Option<ActionFate> {
    j.sp().action_results.as_ref().expect("a joined result history").peek(action).map(|r| r.fate)
}

/// An honest Strict shot is served: one typed action (its recipe the
/// weapon's slot), nothing predicted, ONE projectile, its row Accepted, the
/// Take and the Wear applied on the client from the server's outcome, the
/// two windows agreeing, never torn down.
#[test]
fn c3d_client_strict_shot_is_served_typed() {
    let mut j = joined("shot-served", &[(0, ItemStack { item: Item::Tool(bow()), count: 1 }), (1, arrows(8))], true);
    let ammo = arrows(1).item;
    let before = j.sp().window_events.last_number();
    let routed = shoot(&mut j, 0);
    assert_eq!(j.held(&ammo), 8, "nothing predicted: no arrow spent before the outcome");
    assert!(matches!(j.inv.slot(0).map(|s| &s.item), Some(Item::Tool(t)) if t.durability == bow().durability), "no wear predicted");
    j.hs.tick();
    j.rc.poll();
    let reason = match &j.rc.state { ConnectionState::Failed(m) => m.clone(), _ => String::new() };
    assert!(!j.hs.slot_is_free(j.i) && reason.is_empty(), "an honest Strict shot is never torn down: {reason}");
    let shot = routed.expect("a Strict seat sends the shot typed");
    let recipe: ShotRecipe = bincode::deserialize(&j.rc.action_journal.as_ref().unwrap().actions().last().unwrap().recipe).expect("a shot recipe");
    assert_eq!((recipe.slot, recipe.weapon), (0, ShotWeapon::Bow));
    assert_eq!(fate(&j, shot), Some(ActionFate::Accepted), "served");
    assert_eq!(projectiles(&j), 1, "one projectile");
    assert_eq!(j.sp().window_events.last_number(), before + 2, "the Take and the Wear numbered");
    j.assert_honest("the served shot");
    assert_eq!(j.held(&ammo), 7, "the server's Take applied on the client");
    assert!(matches!(j.inv.slot(0).map(|s| &s.item), Some(Item::Tool(t)) if t.durability + 1 == bow().durability), "the server's Wear applied on the client");
    assert_eq!((j.sp().strict_refusals.stale_world, j.sp().strict_refusals.other), (0, 0));
}

/// A shot the server's copy can't pay (no arrows there, though the client
/// shows some) is refused with zero effect and corrected by ONE `Replace`:
/// no projectile, no teardown, the client's window put back to the server's.
#[test]
fn c3d_client_strict_shot_without_server_ammo_is_corrected() {
    let mut j = joined("shot-refused", &[(0, ItemStack { item: Item::Tool(bow()), count: 1 }), (1, arrows(8))], true);
    // The session's refill setting goes first, on agreeing windows (its own
    // refusal still tears down: `c3d_s1_refused_auto_refill_setting_tears_down_unconfirmed`).
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.assert_honest("the refill setting");
    j.hs.server.players[j.i].inventory.set_slot(1, None);
    let before = j.sp().window_events.last_number();
    let shot = shoot(&mut j, 0).expect("typed");
    j.hs.tick();
    assert_eq!(fate(&j, shot), Some(ActionFate::Refused), "refused");
    assert_eq!(projectiles(&j), 0, "no projectile");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace");
    assert!(!j.hs.slot_is_free(j.i), "never torn down");
    j.assert_honest("the refused shot");
    assert_eq!(j.held(&arrows(1).item), 0, "the client's phantom arrows are corrected away");
    assert!(matches!(j.inv.slot(0).map(|s| &s.item), Some(Item::Tool(t)) if t.durability == bow().durability), "no wear");
    assert_eq!((j.sp().strict_refusals.stale_world, j.sp().strict_refusals.other), (0, 1));
}

/// Mirror is unchanged: `route_shot` sends nothing and records nothing
/// (the game loop then sends its legacy request exactly as before), and the
/// legacy request is served as before: one projectile, an accepted outcome
/// that takes one arrow and wears the bow.
#[test]
fn c3d_client_mirror_shot_stays_the_legacy_request() {
    use super::c3d_grid_tests::joined as grid_joined;
    let mut j = grid_joined("shot-mirror", InventoryMode::Mirror, None);
    j.give(0, ItemStack { item: Item::Tool(bow()), count: 1 });
    j.give(1, arrows(8));
    j.pumps(3);
    let mut actions = JoinerActions::default();
    let (pending, action) = shot_of(&j.inv, 0);
    let routed = route_shot(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut actions, pending.clone(), &action).expect("routed");
    assert_eq!(routed, ShotRoute::Legacy, "Mirror: the legacy request");
    assert_eq!(actions.len(), 0, "Mirror: route_shot records nothing");
    assert!(j.rc.action_journal.as_ref().is_none_or(|journal| journal.len() == 0), "Mirror: nothing reserved");
    send_legacy(&mut j.rc, &mut actions, pending, action);
    j.hs.tick();
    let shots = j.hs.server.ecs.query::<&crate::entity::ProjectileEntity>().iter().count();
    assert_eq!(shots, 1, "Mirror: one projectile, as before");
    j.rc.poll();
    let outcome = j.rc.pending_outcomes.iter().find_map(|o| match o { RequestOutcome::Item(out) => Some(out.clone()), _ => None }).expect("the legacy outcome");
    assert!(outcome.accepted && outcome.consume_held == 1 && outcome.wear_held, "Mirror: the same outcome: {outcome:?}");
    j.assert_live("Mirror: never torn down");
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }));
}

/// C3d run 14 (run-13 review LOW-2) — `RawUnrepresentable` through the shot
/// arm: refused, then torn down with the `Unrepresentable` sentence; no
/// projectile, no Replace.
#[test]
fn c3d_client_strict_shot_on_a_raw_unrepresentable_window_is_refused_then_torn_down() {
    let mut j = joined("shot-raw", &[(0, ItemStack { item: Item::Tool(bow()), count: 1 }), (1, arrows(8))], true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the refill setting");
    let mut tool = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
    tool.durability = tool.max_durability() + 5;
    j.hs.server.players[j.i].inventory.set_slot(5, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
    let before = j.sp().window_events.last_number();
    let shot = shoot(&mut j, 0).expect("typed");
    j.hs.tick();
    assert_eq!(fate(&j, shot), Some(ActionFate::Refused), "the shot's refusal stands");
    assert!(j.hs.slot_is_free(j.i), "torn down");
    j.rc.poll();
    let reason = match &j.rc.state { ConnectionState::Failed(m) => m.as_str(), _ => "not Failed" };
    assert_eq!(reason, crate::strict_admission::StrictTeardown::Unrepresentable.reason(), "its own reason");
    assert_eq!(projectiles(&j), 0, "no projectile");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction Replace");
}
