//! C3d run 25 — a Strict seat's typed Q-drop and sleep, on a REAL
//! `RemoteClient` joined to a Strict `HostedServer` (the
//! `c3d_client_edit_tests` rig), routed as the game loop routes them
//! (`strict_requests::route_drop`, `route_item`). A drop is predicted (the
//! hand's take): the server copy pays it from the named slot, and every
//! refusal is corrected; the drop bucket defers, never refuses. A sleep
//! claims and predicts nothing: a world fact is the note and a receipt,
//! uncorrected. Never torn down.
use super::c3d_client_attack_tests::fate;
use super::c3d_client_edit_tests::{joined, Joined};
use crate::action_identity::{ActionFate, ActionHeader, InventoryMode};
use crate::item::{Item, ItemStack, MaterialId};
use crate::item_actions::ItemNote;
use crate::joiner_actions::{Asked, Pending};
use crate::protocol;
use crate::remote_client::RequestOutcome;
use crate::strict_requests::{route_drop, route_item};

fn mat(m: MaterialId, n: u8) -> ItemStack { ItemStack::new_material(m, n) }

fn rig(tag: &str, items: &[(usize, ItemStack)]) -> Joined {
    let mut j = joined(tag, items, true);
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("reserved and sent");
    j.pump();
    j.assert_honest("the refill setting");
    j
}

/// The game loop's Strict Q-drop of one from hotbar `hot`.
fn drop_one(j: &mut Joined, hot: usize) -> ActionHeader {
    route_drop(Some(&mut j.rc), &mut j.ui, &mut j.inv, &mut j.armour, hot, &j.reg).expect("routed").expect("a Strict seat sends the drop typed")
}

/// Units of `item` lying on the server's ground.
fn ground(j: &Joined, item: &Item) -> u32 {
    j.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().filter(|(_, e)| &e.stack.item == item).map(|(_, e)| u32::from(e.stack.count)).sum()
}

fn server_held(j: &Joined, item: &Item) -> u32 {
    j.sp().inventory.slots_iter().flatten().filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum()
}

fn tally(j: &Joined) -> (u32, u32, u32) {
    let t = j.sp().strict_refusals;
    (t.stale_world, t.other, t.world_fact)
}

#[test]
fn c3d_client_strict_drop_is_paid_by_the_server_copy() {
    let bread = Item::Material(MaterialId::Bread);
    let mut j = rig("drop-served", &[(0, mat(MaterialId::Bread, 4))]);
    let before = j.sp().window_events.last_number();
    let action = drop_one(&mut j, 0);
    assert_eq!(j.held(&bread), 3, "the hand's take is predicted");
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "served");
    assert_eq!(server_held(&j, &bread), 3, "the copy paid the drop from the named slot");
    assert_eq!(ground(&j, &bread), 1, "thrown from the server's body");
    assert_eq!(j.sp().window_events.last_number(), before, "no numbered event: the client already made the take");
    j.assert_honest("drop");
    assert_eq!(j.held(&bread), 3);
    assert_eq!(tally(&j), (0, 0, 0), "no refusal");
}

/// The client's slot 0 says bread; the server copy's slot 0 holds stone
/// (its bread is in slot 5): the window disagrees — refused and corrected
/// (one `Replace`), nothing thrown. A server that dropped the claim would
/// throw a bread (and take one from slot 5).
#[test]
fn c3d_client_strict_drop_claim_the_copy_lacks_is_refused_and_corrected() {
    let bread = Item::Material(MaterialId::Bread);
    let mut j = rig("drop-claim", &[(0, ItemStack::new_block(crate::block::STONE, 1)), (5, mat(MaterialId::Bread, 4))]);
    j.inv.set_slot(0, Some(mat(MaterialId::Bread, 2)));
    let before = j.sp().window_events.last_number();
    let action = drop_one(&mut j, 0);
    j.pump();
    assert_eq!(ground(&j, &bread), 0, "nothing thrown from the claim");
    assert_eq!(fate(&j, action), Some(ActionFate::Refused), "refused");
    assert_eq!(j.sp().window_events.last_number(), before + 1, "exactly one correction Replace");
    j.assert_honest("the refused drop");
    assert_eq!(j.inv.slot(0).cloned(), Some(ItemStack::new_block(crate::block::STONE, 1)), "the client's slot corrected");
    assert_eq!(server_held(&j, &bread), 4);
    assert_eq!(tally(&j), (0, 1, 0));
}

/// Nine Q-drops sent in one tick (faster than the drop bucket refills):
/// each waits at the head of the queue for a token and is served — none
/// refused, no correction, never torn down. A server that refused an
/// unready bucket would correct most of them.
#[test]
fn c3d_client_strict_nine_quick_drops_wait_for_the_bucket_and_are_all_served() {
    let bread = Item::Material(MaterialId::Bread);
    let mut j = rig("drop-nine", &[(0, mat(MaterialId::Bread, 16))]);
    let before = j.sp().window_events.last_number();
    let actions: Vec<_> = (0..9).map(|_| drop_one(&mut j, 0)).collect();
    assert_eq!(j.held(&bread), 7);
    // A receipt's row is freed once the client acknowledges it: read each
    // fate as it lands.
    let mut fates = vec![None; actions.len()];
    for _ in 0..(9 * crate::item_actions::DROP_INTERVAL_TICKS + 20) {
        j.pump();
        for (seen, action) in fates.iter_mut().zip(&actions) {
            if seen.is_none() { *seen = fate(&j, *action); }
        }
    }
    assert_eq!(fates, vec![Some(ActionFate::Accepted); 9], "every drop served");
    assert_eq!(ground(&j, &bread), 9, "nine thrown");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction");
    assert_eq!(tally(&j), (0, 0, 0), "no refusal");
    j.assert_honest("nine quick drops");
    assert_eq!(server_held(&j, &bread), 7);
}

/// A bed one block east of the joiner's feet.
fn bed(j: &mut Joined) -> [i32; 3] {
    let cell = [41, 80, 40];
    j.hs.server.world.set_block(cell[0], cell[1], cell[2], crate::block::BED);
    cell
}

fn sleep(j: &mut Joined, bed: [i32; 3]) -> ActionHeader {
    let pending = Pending { kind: Asked::Sleep { bed }, mob: None, hotbar_slot: 0, held: None };
    let packet = protocol::ItemActionPacket { seq: 0, action: protocol::ItemAction::Sleep { bed }, events_applied: 0 };
    let window = j.ui.digest(&j.inv, &j.armour);
    let header = route_item(Some(&mut j.rc), &mut j.ui, &j.inv, &j.armour, &mut j.actions, pending, packet).expect("routed").expect("a Strict seat sends the sleep typed");
    assert_eq!(j.ui.digest(&j.inv, &j.armour), window, "routing predicted nothing");
    header
}

fn heard_notes(j: &mut Joined) -> Vec<(bool, u8)> {
    j.hs.tick();
    j.rc.poll();
    j.rc.pending_outcomes.iter().filter_map(|o| match o { RequestOutcome::Item(out) => Some((out.accepted, out.note)), _ => None }).collect()
}

#[test]
fn c3d_client_strict_sleep_sets_the_spawn_and_heals() {
    let mut j = rig("sleep-served", &[]);
    let cell = bed(&mut j);
    j.hs.server.world_time = 0; // midnight
    j.hs.server.players[j.i].combat.health = 5.0;
    let before = j.sp().window_events.last_number();
    let action = sleep(&mut j, cell);
    j.pump();
    assert_eq!(fate(&j, action), Some(ActionFate::Accepted), "served");
    assert_eq!(j.sp().spawn_pos, crate::item_actions::bed_spawn(cell), "the spawn is the bed");
    assert_eq!(j.sp().combat.health, j.sp().combat.max_health, "healed");
    assert_eq!(j.sp().window_events.last_number(), before, "no window event");
    j.assert_honest("sleep");
    assert_eq!(j.actions.len(), 0, "the request record is answered");
    assert_eq!(tally(&j), (0, 0, 0));
}

/// A held right-click on a bed at noon: nine sleeps, each refused
/// `NotNight` — a world fact (the note and a receipt, no `Replace`, no tick
/// of the correction bound), never torn down. A server that corrected it
/// would number nine `Replace`s (or run out of the bound).
#[test]
fn c3d_client_strict_nine_sleeps_at_noon_are_world_facts_never_corrected() {
    let mut j = rig("sleep-noon", &[]);
    let cell = bed(&mut j);
    j.hs.server.world_time = 12000; // noon
    let before = j.sp().window_events.last_number();
    let mut notes = Vec::new();
    for _ in 0..9 {
        sleep(&mut j, cell);
        notes.extend(heard_notes(&mut j));
        j.pump();
    }
    assert_eq!(notes, vec![(false, ItemNote::NotNight.to_wire()); 9], "the note reaches the client");
    assert_eq!(j.sp().window_events.last_number(), before, "no correction Replace");
    assert_eq!(tally(&j), (0, 0, 9), "nine world facts");
    j.assert_honest("nine sleeps at noon");
    assert_ne!(j.sp().spawn_pos, crate::item_actions::bed_spawn(cell), "no spawn set");
}

/// Mirror keeps the legacy drop: `route_drop` reserves nothing and leaves
/// the hand alone (the caller takes it and sends the legacy request).
#[test]
fn c3d_client_mirror_drop_stays_legacy() {
    use super::c3d_grid_tests::joined as grid_joined;
    let mut j = grid_joined("drop-mirror", InventoryMode::Mirror, None);
    j.inv.set_slot(0, Some(mat(MaterialId::Bread, 2)));
    let reg = crate::block::BlockRegistry::new();
    assert_eq!(route_drop(Some(&mut j.rc), &mut j.ui, &mut j.inv, &mut j.armour, 0, &reg).expect("routed"), None, "Mirror: the legacy drop");
    assert_eq!(j.inv.slot(0).map(|s| s.count), Some(2), "Mirror: the hand untouched here");
    assert!(j.rc.action_journal.as_ref().is_none_or(|journal| journal.len() == 0), "Mirror: nothing reserved");
    j.assert_live("Mirror: never torn down");
}
