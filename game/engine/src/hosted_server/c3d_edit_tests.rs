//! C3d run 6 — typed block edits in Strict, through the real HostedServer
//! dispatch: an honest placement and break are accepted with conservation
//! over window + world + ground; an unheld placement is refused with zero
//! effect; a claimed better tool never sets the drop tier.
use super::c3d_strict_tests::{strict_rig, Rig};
use super::*;
use crate::action_identity::{ActionFate, ActionHeader};
use crate::action_wire::ActionIntent;
use crate::block::{AIR, COBBLESTONE, STONE};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack};

/// The floor cell beside the joiner (in reach) and the air cell above it.
const FLOOR: (i32, i32, i32) = (41, 79, 40);
const ABOVE: (i32, i32, i32) = (41, 80, 40);

/// A Strict rig whose joiner stands on a stone floor at (40.5, 80, 40.5),
/// its column loaded, with no terrain streaming and no mobs.
pub(super) fn edit_rig(tag: &str) -> Rig {
    let mut r = strict_rig(&format!("edit-{tag}"));
    let hs = &mut r.hs;
    hs.server.column_streamer = None;
    hs.server.column_refill_per_tick = 0;
    hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    for x in 32..=48 {
        for z in 32..=48 {
            hs.server.world.set_block(x, 79, z, STONE);
            for y in 80..=84 { hs.server.world.set_block(x, y, z, AIR); }
        }
    }
    let cs = crate::chunk::CHUNK_SIZE as i32;
    hs.server.loaded_columns.insert((40i32.div_euclid(cs), 40i32.div_euclid(cs)));
    let p = &mut hs.server.players[r.i].player;
    p.pos = glam::Vec3::new(40.5, 80.0, 40.5);
    p.velocity = glam::Vec3::ZERO;
    p.on_ground = true;
    r
}

fn pick(material: ToolMaterial) -> Tool { Tool::new(ToolType::Pickaxe, material) }

/// Send one typed edit of `cell` to `new`, made with hotbar `slot`, whose
/// input claims `claimed` in hand and (a break) `mined_with` as its tool.
fn edit(r: &mut Rig, cell: (i32, i32, i32), new: u16, slot: u8, claimed: Option<&Item>, mined_with: Option<Option<Tool>>) -> ActionHeader {
    let action = r.next();
    let change = protocol::BlockChange { x: cell.0, y: cell.1, z: cell.2, new_block: new, meta: 0 };
    let (kind, id) = claimed.map_or((protocol::item_kind::EMPTY, 0), |item| crate::inventory::item_to_ref(item).to_wire());
    let mined = mined_with.map(|tool| protocol::MinedBlock {
        x: cell.0, y: cell.1, z: cell.2,
        tool: tool.map_or(protocol::WireItem::None, |t| crate::inventory::item_to_wire_full(&Item::Tool(t))),
    });
    r.send(action, ActionIntent::Edit { source_tick: 0, change, hand: (slot, kind, id), mined, use_tag: None });
    action
}

#[derive(Default)]
struct Heard { grants: Vec<protocol::InventoryGrantPacket>, rejects: Vec<String> }

fn listen(r: &Rig) -> Heard {
    use crate::transport::ClientTransport as _;
    let mut out = Heard::default();
    while let Some(pkt) = r.client.try_recv_from_server() {
        match protocol::deserialize_header(&pkt) {
            Some((protocol::PacketType::InventoryGrant, p)) => out.grants.push(protocol::safe_deserialize(p).unwrap()),
            Some((protocol::PacketType::JoinReject, p)) => out.rejects.push(protocol::safe_deserialize::<protocol::JoinRejectPacket>(p).unwrap().reason),
            _ => {}
        }
    }
    out
}

fn count(sp: &crate::server::ServerPlayer, block: u16) -> u32 {
    sp.inventory.slots_iter().flatten().filter(|s| matches!(s.item, Item::Block(b) if b == block)).map(|s| u32::from(s.count)).sum()
}

fn ground_items(r: &Rig) -> usize { r.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().count() }

fn block_at(r: &Rig, (x, y, z): (i32, i32, i32)) -> u16 { r.hs.server.world.get_block(x, y, z) }

/// An honest placement: accepted, the server's copy takes exactly what the
/// client's shared placement rule takes (auto-refill from the bag on the
/// last unit included), no event is numbered, and stone is conserved over
/// window + world.
#[test]
fn c3d_strict_honest_placement_is_accepted_by_the_shared_rule_with_conservation() {
    let mut r = edit_rig("place");
    let sp = &mut r.hs.server.players[r.i];
    sp.inventory.set_slot(0, Some(ItemStack::new_block(STONE, 1)));
    sp.inventory.set_slot(9, Some(ItemStack::new_block(STONE, 5)));
    sp.inventory.auto_refill = true;
    let mut predicted = sp.inventory.clone();
    assert_eq!(predicted.take_placeable_from_hotbar(0), Some(STONE), "the client's own placement rule");
    let events = sp.window_events.last_number();
    let before = count(sp, STONE) + u32::from(block_at(&r, ABOVE) == STONE);
    let stone = Item::Block(STONE);
    let action = edit(&mut r, ABOVE, STONE, 0, Some(&stone), None);
    let heard = listen(&r);
    assert!(heard.rejects.is_empty(), "never torn down: {:?}", heard.rejects);
    assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Accepted));
    assert_eq!(block_at(&r, ABOVE), STONE, "the block is in the world");
    assert!(r.hs.server.world.is_placed(ABOVE.0, ABOVE.1, ABOVE.2), "player-placed");
    let sp = r.sp();
    assert_eq!(sp.inventory.slot(0), predicted.slot(0), "slot 0 refilled exactly as the client's rule did");
    assert_eq!(sp.inventory.slot(9), predicted.slot(9));
    assert_eq!(sp.window_events.last_number(), events, "a predicted placement numbers no event");
    assert_eq!(count(sp, STONE) + u32::from(block_at(&r, ABOVE) == STONE), before, "stone conserved over window + world");
    assert_eq!(ground_items(&r), 0);
}

/// An honest break with the held wooden pickaxe: accepted, the cell is
/// empty, the pickaxe in the server's copy wears exactly as the client's
/// `use_tool_at` did, and the yield is ONE numbered grant carried to the
/// client; once it applies, the cobblestone is in the window and nothing is
/// on the ground.
#[test]
fn c3d_strict_honest_break_wears_the_held_tool_and_grants_its_yield_once() {
    let mut r = edit_rig("break");
    let wood = pick(ToolMaterial::Wood);
    let sp = &mut r.hs.server.players[r.i];
    sp.inventory.set_slot(0, Some(ItemStack { item: Item::Tool(wood), count: 1 }));
    let mut predicted = sp.inventory.clone();
    assert!(predicted.use_tool_at(0).is_some(), "the client's own wear rule");
    let events = sp.window_events.last_number();
    let action = edit(&mut r, FLOOR, AIR, 0, Some(&Item::Tool(wood)), Some(Some(wood)));
    let heard = listen(&r);
    assert!(heard.rejects.is_empty(), "never torn down: {:?}", heard.rejects);
    assert_eq!(r.row(action).map(|row| (row.fate, row.included_events)), Some((ActionFate::Accepted, events + 1)));
    assert_eq!(block_at(&r, FLOOR), AIR);
    assert_eq!(r.sp().inventory.slot(0), predicted.slot(0), "the held pickaxe wore once, as predicted");
    assert_eq!(r.sp().window_events.last_number(), events + 1, "one numbered grant");
    assert_eq!(heard.grants.len(), 1, "carried to the client");
    assert_eq!((heard.grants[0].window_event, heard.grants[0].count), (events + 1, 1));
    let sp = &mut r.hs.server.players[r.i];
    crate::window_events::apply_through(sp, events + 1, 0);
    assert_eq!(count(sp, COBBLESTONE), 1, "the yield, once, in the window");
    assert_eq!(ground_items(&r), 0, "nothing spilled");
}

/// A placement of an item the server's copy doesn't hold in that slot (a
/// modified client claiming stone in an empty hand): refused with its typed
/// fate and zero effect — no block, no event, the window untouched — and
/// the session corrected (the bridge teardown until recovery is wired).
#[test]
fn c3d_strict_unheld_placement_is_refused_with_zero_effect() {
    let mut r = edit_rig("unheld");
    let sp = &mut r.hs.server.players[r.i];
    sp.inventory.set_slot(0, Some(ItemStack::new_block(crate::block::DIRT, 4)));
    let window = sp.inventory.clone();
    let events = sp.window_events.last_number();
    let stone = Item::Block(STONE);
    let action = edit(&mut r, ABOVE, STONE, 0, Some(&stone), None);
    let heard = listen(&r);
    assert_eq!(block_at(&r, ABOVE), AIR, "no block");
    assert_eq!(r.sp().inventory.slot(0), window.slot(0), "window untouched");
    assert_eq!(r.sp().window_events.last_number(), events, "no event");
    assert_eq!(r.through(), action.seq, "terminal at its seq");
    assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Refused), "its typed fate");
    assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::Unconfirmed.reason()), "{:?}", heard.rejects);
    assert!(r.hs.server.load_joiner(r.i).expect("departure saved").obligations_proven_zero());
}

/// A break whose input claims a wooden pickaxe while the server's copy of
/// the slot is empty: the bare hand mines stone for nothing. The claimed
/// tool never sets the drop tier (Decision 8).
#[test]
fn c3d_strict_claimed_better_tool_is_ignored_for_the_drop_tier() {
    let mut r = edit_rig("claimed");
    let events = r.sp().window_events.last_number();
    let wood = pick(ToolMaterial::Wood);
    let action = edit(&mut r, FLOOR, AIR, 0, Some(&Item::Tool(wood)), Some(Some(wood)));
    let heard = listen(&r);
    assert!(heard.rejects.is_empty(), "{:?}", heard.rejects);
    assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Accepted), "a bare-hand break is a break");
    assert_eq!(block_at(&r, FLOOR), AIR);
    assert!(heard.grants.is_empty(), "the bare hand's tier yields no cobblestone: {:?}", heard.grants.iter().map(|g| (g.item_kind, g.item_id, g.count)).collect::<Vec<_>>());
    assert_eq!(r.sp().window_events.last_number(), events, "no grant numbered");
    assert_eq!(count(r.sp(), COBBLESTONE), 0);
    assert_eq!(ground_items(&r), 0);
}

/// A cell 6.7 blocks from the joiner's eye: past the base reach limit
/// (5 + 1.37), inside the Reach Claw's (+2.0).
const FAR: (i32, i32, i32) = (46, 81, 43);

/// C3d run 10 (run-9b review MEDIUM-1) — Decision 19 Drift first: an honest
/// mined `Edit` whose cell the server has already changed (a leaf it
/// decayed, a block another player mined, a crop someone harvested) is a
/// Drift `Refuse` with its typed `Refused` row (the bridge teardown until
/// recovery rebases it), never an `Untyped` pause. Zero effect.
#[test]
fn c3d_strict_mined_edit_on_an_already_changed_cell_is_a_drift_refusal() {
    // (case, what the server's cell holds now, what the client's change leaves)
    for (k, (case, now, left)) in [
        ("a leaf the server already decayed", AIR, AIR),
        ("a block another player just mined", AIR, AIR),
        ("a crop someone already harvested", crate::block::TILLED_SOIL, crate::block::TILLED_SOIL),
    ].into_iter().enumerate() {
        let mut r = edit_rig(&format!("drift-{k}"));
        r.hs.server.world.set_block(ABOVE.0, ABOVE.1, ABOVE.2, now);
        let wood = pick(ToolMaterial::Wood);
        r.hs.server.players[r.i].inventory.set_slot(0, Some(ItemStack { item: Item::Tool(wood), count: 1 }));
        let window = r.sp().inventory.clone();
        let events = r.sp().window_events.last_number();
        let action = edit(&mut r, ABOVE, left, 0, Some(&Item::Tool(wood)), Some(Some(wood)));
        let heard = listen(&r);
        assert_eq!(r.row(action).map(|row| row.fate), Some(ActionFate::Refused), "{case}: a Drift refusal, with its typed row");
        assert!(heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::Unconfirmed.reason()), "{case}: the refusal's teardown: {:?}", heard.rejects);
        assert!(!heard.rejects.iter().any(|m| m == crate::strict_admission::StrictTeardown::Untyped.reason()), "{case}: never an Untyped pause");
        assert_eq!(block_at(&r, ABOVE), now, "{case}: the world is untouched");
        assert_eq!(r.sp().inventory.slot(0), window.slot(0), "{case}: the tool didn't wear");
        assert_eq!(r.sp().window_events.last_number(), events, "{case}: nothing granted");
    }
}

/// C3d run 10 (run-9b review MEDIUM-2) — a Strict joiner's Reach Claw
/// counts from the server's own copy of the edit's slot: a bare-claw break
/// at 6.7 blocks is accepted; the same break claiming a claw the copy
/// doesn't hold is refused.
#[test]
fn c3d_strict_reach_claw_break_counts_from_the_servers_copy() {
    let claw = ItemStack::new_material(crate::item::MaterialId::ReachClaw, 1);
    for held in [true, false] {
        let what = if held { "the copy holds the claw" } else { "only claimed" };
        let mut r = edit_rig(&format!("claw-{held}"));
        r.hs.server.world.set_block(FAR.0, FAR.1, FAR.2, crate::block::DIRT);
        if held { r.hs.server.players[r.i].inventory.set_slot(0, Some(claw.clone())); }
        let action = edit(&mut r, FAR, AIR, 0, Some(&claw.item), Some(None));
        let heard = listen(&r);
        let fate = r.row(action).map(|row| row.fate);
        if held {
            assert!(heard.rejects.is_empty(), "{what}: never torn down: {:?}", heard.rejects);
            assert_eq!(fate, Some(ActionFate::Accepted), "{what}: in the claw's reach");
            assert_eq!(block_at(&r, FAR), AIR, "{what}: broken");
        } else {
            assert_eq!(fate, Some(ActionFate::Refused), "{what}: never on the client's word");
            assert_eq!(block_at(&r, FAR), crate::block::DIRT, "{what}: untouched");
        }
    }
}
