//! C3d run 23 — a Strict seat's typed entity interactions (feed, tame,
//! shear, milk, a Lead on, off and on a post, a pet command, a mount), on a
//! REAL `RemoteClient` joined to a Strict `HostedServer` (the
//! `c3d_client_edit_tests` rig), routed as `GameState::send_interaction`
//! routes them (`strict_requests::route_interact`). The item is the server
//! copy's, never the claim; nothing is predicted; a window disagreement is
//! corrected, a world fact is a receipt and its note alone; never torn down.
use super::c3d_client_attack_tests::{ahead, fate, place};
use super::c3d_client_edit_tests::{joined, Joined};
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack, MaterialId};
use crate::joiner_actions::{Asked, JoinerActions, Pending};
use crate::mob::MobType;
use crate::mob_interact::InteractNote;
use crate::protocol::{self, InteractKind};
use crate::remote_client::{ConnectionState, RequestOutcome};
use crate::strict_requests::route_interact;
use crate::tether::{TetherTarget, Tethered};
use glam::Vec3;

fn mat(m: MaterialId, n: u8) -> ItemStack { ItemStack::new_material(m, n) }

/// A mob of `kind` in the server's world at `at`, broadcast once (so it
/// has an id), at rest with the joiner's body facing it.
fn spawn(j: &mut Joined, kind: MobType, at: Vec3) -> (hecs::Entity, u32) {
    let e = crate::entity::spawn_mob(&mut j.hs.server.ecs, kind, at);
    j.hs.tick();
    let id = j.hs.server.ecs.get::<&crate::entity::ProtocolId>(e).expect("broadcast").0;
    place(&mut j.hs, j.i, e, at);
    (e, id)
}

/// A Strict rig whose world runs animal life (`life`: a lent world's; a
/// dedicated server's is `false`), signed in as `[7; 32]` (a pet owner), the
/// refill setting agreed.
fn rig(tag: &str, items: &[(usize, ItemStack)], life: bool) -> Joined {
    let mut j = joined(tag, items, true);
    j.hs.server.animal_life_simulated = life;
    j.hs.server.players[j.i].verified_pubkey = Some([7; 32]);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the refill setting");
    j
}

fn owner_key(j: &Joined) -> String { j.sp().pet_owner_key().expect("signed in") }

/// The interaction `GameState::send_interaction` builds from hotbar `hot`,
/// routed as the game loop routes it.
fn interact(j: &mut Joined, hot: usize, mob: Option<MobType>, entity: u32, kind: InteractKind) -> ActionHeader {
    let item = j.inv.slot(hot).map(|s| s.item.clone());
    let (held_kind, held_id) = item.as_ref().map_or(protocol::ItemRef::Empty.to_wire(), |i| crate::inventory::item_to_ref(i).to_wire());
    let held_full = item.as_ref().map_or(protocol::WireItem::None, crate::inventory::item_to_wire_full);
    let pending = Pending { kind: Asked::Interact(kind), mob, hotbar_slot: hot, held: item };
    let packet = protocol::EntityInteractPacket { seq: 0, entity, kind, held_kind, held_id, held_full, hotbar_slot: hot as u8, sneak: false, events_applied: 0 };
    let window = j.ui.digest(&j.inv, &j.armour);
    let header = route_interact(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut j.actions, pending, packet).expect("routed").expect("a Strict seat sends the interaction typed");
    assert_eq!(j.ui.digest(&j.inv, &j.armour), window, "routing predicted nothing");
    header
}

/// One server tick and the client's read, then the interaction outcomes the
/// client heard (before the rig applies them).
fn heard_outcomes(j: &mut Joined) -> Vec<protocol::InteractOutcomePacket> {
    j.hs.tick();
    j.rc.poll();
    j.rc.pending_outcomes.iter().filter_map(|o| match o { RequestOutcome::Interact(out) => Some(out.clone()), _ => None }).collect()
}

/// Served: an Accepted row, the events the cost table names, the client's
/// window moved only by the server's answer, never torn down.
fn assert_served(j: &mut Joined, action: ActionHeader, before: u32, events: u32, what: &str) {
    assert_eq!(fate(j, action), Some(ActionFate::Accepted), "{what}: served");
    assert_eq!(j.sp().window_events.last_number(), before + events, "{what}: exactly the table's numbered events");
    j.assert_honest(what);
    assert_eq!(j.actions.len(), 0, "{what}: the request record is answered");
    let t = j.sp().strict_refusals;
    assert_eq!((t.stale_world, t.other, t.world_fact), (0, 0, 0), "{what}: no refusal");
}

#[test]
fn c3d_client_strict_feed_is_served_from_the_server_item() {
    let food = crate::breeding::breeding_food(MobType::Cow).unwrap();
    let mut j = rig("interact-feed", &[(0, mat(food, 4))], true);
    let at = ahead(j.sp().player.pos, 2.0);
    let (cow, id) = spawn(&mut j, MobType::Cow, at);
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Cow), id, InteractKind::Feed);
    j.pump();
    assert!(j.hs.server.ecs.get::<&crate::breeding::InLove>(cow).is_ok(), "the cow is in love");
    assert_served(&mut j, action, before, 1, "feed");
    assert_eq!(j.held(&Item::Material(food)), 3, "one food taken by the server's Take");
}

#[test]
fn c3d_client_strict_tame_is_served_from_the_server_item() {
    let mut j = rig("interact-tame", &[(0, mat(MaterialId::CatTreat, 2))], true);
    let at = ahead(j.sp().player.pos, 2.0);
    let (cat, id) = spawn(&mut j, MobType::Cat, at);
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Cat), id, InteractKind::Tame);
    j.pump();
    let key = owner_key(&j);
    assert_eq!(crate::tameable::pet_owner_of(&j.hs.server.ecs, cat).as_deref(), Some(key.as_str()), "the cat is the joiner's");
    assert_served(&mut j, action, before, 1, "tame");
    assert_eq!(j.held(&Item::Material(MaterialId::CatTreat)), 1);
}

#[test]
fn c3d_client_strict_shear_is_served_and_the_shears_stay() {
    let shears = ItemStack { item: Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron)), count: 1 };
    let mut j = rig("interact-shear", &[(0, shears.clone())], true);
    let at = ahead(j.sp().player.pos, 2.0);
    let (sheep, id) = spawn(&mut j, MobType::Sheep, at);
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Sheep), id, InteractKind::Shear);
    j.pump();
    assert!(!crate::mob_interact::product_ready(&j.hs.server.ecs, sheep, j.hs.server.tick_counter), "the sheep is shorn");
    assert_served(&mut j, action, before, 0, "shear");
    assert_eq!(j.inv.slot(0).cloned(), Some(shears), "the shears stay");
}

#[test]
fn c3d_client_strict_milk_is_served_take_then_grant() {
    let mut j = rig("interact-milk", &[(0, mat(MaterialId::Bucket, 1))], true);
    let at = ahead(j.sp().player.pos, 2.0);
    let (_, id) = spawn(&mut j, MobType::Cow, at);
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Cow), id, InteractKind::Milk);
    j.pump();
    assert_served(&mut j, action, before, 2, "milk");
    assert_eq!(j.held(&Item::Material(MaterialId::Bucket)), 0, "the bucket taken");
    assert_eq!(j.held(&Item::Material(MaterialId::MilkBucket)), 1, "a milk bucket granted");
}

#[test]
fn c3d_client_strict_lead_on_and_off_are_served() {
    let mut j = rig("interact-lead", &[(0, mat(MaterialId::Lead, 1))], true);
    let at = ahead(j.sp().player.pos, 2.0);
    let (cow, id) = spawn(&mut j, MobType::Cow, at);
    let before = j.sp().window_events.last_number();
    let on = interact(&mut j, 0, Some(MobType::Cow), id, InteractKind::LeadAttach);
    j.pump();
    assert_eq!(j.hs.server.ecs.get::<&Tethered>(cow).map(|t| t.target).ok(), Some(TetherTarget::Player(j.i)), "leashed to the joiner");
    assert_served(&mut j, on, before, 1, "lead on");
    assert_eq!(j.held(&Item::Material(MaterialId::Lead)), 0);
    for _ in 0..super::INTERACT_COOLDOWN_TICKS { j.pump(); }
    place(&mut j.hs, j.i, cow, at);
    let before = j.sp().window_events.last_number();
    let off = interact(&mut j, 0, Some(MobType::Cow), id, InteractKind::LeadDetach);
    j.pump();
    assert!(j.hs.server.ecs.get::<&Tethered>(cow).is_err(), "the Lead is off");
    assert_served(&mut j, off, before, 1, "lead off");
    assert_eq!(j.held(&Item::Material(MaterialId::Lead)), 1, "the Lead granted back");
}

#[test]
fn c3d_client_strict_pet_command_is_served() {
    let mut j = rig("interact-sit", &[], true);
    let at = ahead(j.sp().player.pos, 2.0);
    let (wolf, id) = spawn(&mut j, MobType::Wolf, at);
    let key = owner_key(&j);
    j.hs.server.ecs.get::<&mut crate::wolf::WolfData>(wolf).unwrap().ownership.owner_pubkey = key;
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Wolf), id, InteractKind::SitToggle);
    let notes: Vec<_> = heard_outcomes(&mut j).iter().map(|o| (o.accepted, o.note)).collect();
    assert_eq!(notes, vec![(true, InteractNote::Sat.to_wire())], "the wolf sits");
    assert_served(&mut j, action, before, 0, "sit");
}

#[test]
fn c3d_client_strict_mount_is_served_with_no_teardown() {
    let mut j = rig("interact-mount", &[], false);
    let at = ahead(j.sp().player.pos, 2.0);
    let (horse, id) = spawn(&mut j, MobType::Horse, at);
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Horse), id, InteractKind::Mount);
    j.pump();
    assert_eq!(j.sp().riding, Some(horse), "seated (riding needs no animal life)");
    assert_served(&mut j, action, before, 0, "mount");
}

#[test]
fn c3d_client_strict_lead_to_post_is_served_with_no_teardown() {
    let mut j = rig("interact-post", &[(0, mat(MaterialId::Lead, 2))], true);
    let post = [40, 80, 38];
    j.hs.server.world.set_block(post[0], post[1], post[2], crate::block::OAK_FENCE_POST);
    let cow = crate::entity::spawn_mob(&mut j.hs.server.ecs, MobType::Cow, Vec3::new(41.5, 80.0, 38.5));
    j.hs.server.ecs.insert_one(cow, Tethered { target: TetherTarget::Player(j.i) }).unwrap();
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, None, 0, InteractKind::LeadToPost { post });
    j.pump();
    assert_eq!(j.hs.server.ecs.get::<&Tethered>(cow).map(|t| t.target).ok(), Some(TetherTarget::Post(post)), "tied to the post");
    assert_served(&mut j, action, before, 1, "lead to a post");
    assert_eq!(j.held(&Item::Material(MaterialId::Lead)), 1);
}

/// The client's slot says food, the server copy's holds a carrot there (the
/// food is in slot 5): the window disagrees, so the interaction is refused
/// and corrected (one `Replace`), the cow untouched, never torn down. A
/// server that took the claim's item would feed the cow from slot 5.
#[test]
fn c3d_client_strict_claimed_item_the_copy_lacks_is_refused_and_corrected() {
    let food = crate::breeding::breeding_food(MobType::Cow).unwrap();
    let mut j = rig("interact-claim", &[(0, mat(MaterialId::Carrot, 1)), (5, mat(food, 4))], true);
    j.inv.set_slot(0, Some(mat(food, 1)));
    let at = ahead(j.sp().player.pos, 2.0);
    let (cow, id) = spawn(&mut j, MobType::Cow, at);
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Cow), id, InteractKind::Feed);
    j.pump();
    assert!(j.hs.server.ecs.get::<&crate::breeding::InLove>(cow).is_err(), "the cow was not fed from the claim");
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace, no Take");
    j.assert_honest("the refused claim");
    assert_eq!(j.actions.len(), 0, "the refused interaction's request record is released");
    assert_eq!(j.inv.slot(0).cloned(), Some(mat(MaterialId::Carrot, 1)), "the client's slot corrected");
    assert_eq!(j.held(&Item::Material(food)), 4);
    assert_eq!((j.sp().strict_refusals.stale_world, j.sp().strict_refusals.other), (0, 1));
}

/// A dedicated server runs no breeding: a Strict feed is refused for that
/// world fact — its `InteractOutcome { accepted: false, NotOnThisServer }`
/// reaches the client beside the `Refused` row, no correction, the food
/// kept on both copies, never torn down.
#[test]
fn c3d_client_strict_feed_on_a_dedicated_server_is_not_on_this_server() {
    let food = crate::breeding::breeding_food(MobType::Cow).unwrap();
    let mut j = rig("interact-dedicated", &[(0, mat(food, 4))], false);
    let at = ahead(j.sp().player.pos, 2.0);
    let (cow, id) = spawn(&mut j, MobType::Cow, at);
    let before = j.sp().window_events.last_number();
    let action = interact(&mut j, 0, Some(MobType::Cow), id, InteractKind::Feed);
    let notes: Vec<_> = heard_outcomes(&mut j).iter().map(|o| (o.accepted, o.note)).collect();
    assert_eq!(notes, vec![(false, InteractNote::NotOnThisServer.to_wire())], "the note reaches the client");
    assert!(j.hs.server.ecs.get::<&crate::breeding::InLove>(cow).is_err(), "nothing fed");
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction, no Take");
    j.assert_honest("not on this server");
    assert_eq!(j.held(&Item::Material(food)), 4, "the food kept");
    let t = j.sp().strict_refusals;
    assert_eq!((t.stale_world, t.other, t.world_fact), (0, 0, 1), "a world fact");
}

/// Right-click held on a cow that wandered ten blocks off: one interaction
/// every client cooldown (8 ticks), nine in under 100 ticks, each a world
/// fact (a receipt alone, no `Replace`, no tick of the correction bound),
/// the food kept, never torn down.
#[test]
fn c3d_client_strict_nine_out_of_reach_interactions_never_tear_down() {
    let food = crate::breeding::breeding_food(MobType::Cow).unwrap();
    let mut j = rig("interact-reach", &[(0, mat(food, 16))], true);
    let at = ahead(j.sp().player.pos, 10.0);
    let (cow, id) = spawn(&mut j, MobType::Cow, at);
    let before = j.sp().window_events.last_number();
    let start = j.hs.server.tick_counter;
    for n in 1..=9 {
        let action = interact(&mut j, 0, Some(MobType::Cow), id, InteractKind::Feed);
        place(&mut j.hs, j.i, cow, at);
        j.pump();
        assert_eq!(fate(&j, action), Some(ActionFate::Refused), "click {n}: refused");
        for _ in 1..8 { place(&mut j.hs, j.i, cow, at); j.pump(); }
        assert!(matches!(j.rc.state, ConnectionState::Connected { .. }) && !j.hs.slot_is_free(j.i), "click {n}: never torn down");
    }
    assert!(j.hs.server.tick_counter - start < 100, "inside 100 ticks");
    assert_eq!(j.sp().window_events.last_number(), before, "no Replace, no Take");
    j.assert_honest("nine out-of-reach interactions");
    assert_eq!(j.held(&Item::Material(food)), 16, "the food kept");
    assert_eq!(j.sp().strict_refusals.world_fact, 9);
}

/// Mirror keeps the legacy request: `route_interact` records and reserves
/// nothing, and the claim is served as before.
#[test]
fn c3d_client_mirror_interaction_stays_the_legacy_request() {
    use super::c3d_grid_tests::joined as grid_joined;
    let mut j = grid_joined("interact-mirror", InventoryMode::Mirror, None);
    j.give(0, mat(MaterialId::Bucket, 1));
    j.pumps(3);
    let at = ahead(j.hs.server.players[j.i].player.pos, 2.0);
    let e = crate::entity::spawn_mob(&mut j.hs.server.ecs, MobType::Cow, at);
    j.hs.tick();
    let id = j.hs.server.ecs.get::<&crate::entity::ProtocolId>(e).expect("broadcast").0;
    place(&mut j.hs, j.i, e, at);
    let item = Item::Material(MaterialId::Bucket);
    let (held_kind, held_id) = crate::inventory::item_to_ref(&item).to_wire();
    let held_full = crate::inventory::item_to_wire_full(&item);
    let pending = Pending { kind: Asked::Interact(InteractKind::Milk), mob: Some(MobType::Cow), hotbar_slot: 0, held: Some(item) };
    let packet = protocol::EntityInteractPacket { seq: 0, entity: id, kind: InteractKind::Milk, held_kind, held_id, held_full, hotbar_slot: 0, sneak: false, events_applied: 0 };
    let mut actions = JoinerActions::default();
    let routed = route_interact(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut actions, pending.clone(), packet.clone()).expect("routed");
    assert_eq!(routed, None, "Mirror: the legacy request");
    assert_eq!(actions.len(), 0, "Mirror: route_interact records nothing");
    assert!(j.rc.action_journal.as_ref().is_none_or(|journal| journal.len() == 0), "Mirror: nothing reserved");
    let seq = actions.record(pending, j.rc.next_input_seq());
    j.rc.send_entity_interact(&protocol::EntityInteractPacket { seq, ..packet });
    j.hs.tick();
    j.rc.poll();
    let outcome = j.rc.pending_outcomes.iter().find_map(|o| match o { RequestOutcome::Interact(out) => Some(out.clone()), _ => None }).expect("the legacy outcome");
    assert!(outcome.accepted && outcome.consume_held == 1 && outcome.window_event > 0, "Mirror: the same outcome, its Take numbered: {outcome:?}");
    j.assert_live("Mirror: never torn down");
}
