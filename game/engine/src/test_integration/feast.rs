//! Living world S3 (2026-11-08) — the Feast and Commons through the real
//! `HostedServer`, over the in-process transport, on a dedicated server and a
//! lending host (the `block_use` rig: a joiner's client half is the real one).
//!
//! What they pin (spec `docs/superpowers/specs/2026-11-08-living-world-tier1.md`
//! §3, §4):
//! - a joiner's feast shares its whole stack among the guests the AUTHORITY
//!   finds (a lent host's own seat and another joiner), as real ground items at
//!   their feet, and the table becomes a marker whose meta (the guest count)
//!   reaches everyone;
//! - no guests refuses and takes nothing; villagers are guests when no player
//!   is near, and their shares are used up;
//! - a modified client cannot make a stack out of nothing;
//! - a Commons Stone over a Plot, and a Plot Marker over a Commons, are refused
//!   for a joiner; a clear one stands and is found as a Commons area;
//! - a world with a marker save-loads with its guest count intact, and a late
//!   joiner's chunk push carries it.

use glam::Vec3;

use crate::block;
use crate::block_use::UseKind;
use crate::item::{Item, MaterialId};
use crate::item_actions::ItemNote;
use crate::plot::{PlotData, PlotOwner};

use super::block_use::{accepted, mat, refused, Rig};

fn bread() -> Item {
    mat(MaterialId::Bread)
}

/// Ground bread within `r` blocks (horizontally) of `p`, in the world the
/// server simulates.
fn bread_near(rig: &Rig, p: Vec3, r: f32) -> u32 {
    rig.ecs()
        .query::<(&crate::entity::Position, &crate::entity::ItemEntity)>()
        .iter()
        .filter(|(_, (pos, it))| it.stack.item == bread() && (pos.0 - p).truncate().length() <= r && (pos.0.z - p.z).abs() <= r)
        .map(|(_, (_, it))| u32::from(it.stack.count))
        .sum()
}

fn pos_of(rig: &Rig, slot: usize) -> Vec3 {
    rig.hs.server.players[slot].player.pos
}

/// A joiner holds a feast on a lending host. The guests are the host's own
/// seat (slot 0) and the other joiner; the holder is not one. The 9 loaves are
/// split 5 and 4, nearest first, each dropped at its guest's feet by the
/// authority; the table becomes a marker with meta 2 that reaches the host's
/// world, the other joiner's world and the event outbox; the holder paid the
/// whole stack as one window event.
#[test]
fn a_joiners_feast_drops_shares_at_the_hosts_seat_and_the_other_joiner() {
    let mut rig = Rig::lent("feast-lent", 2);
    let table = rig.place(2, block::FEAST_TABLE);
    let pos = (table[0], table[1], table[2]);
    // The host's own seat stands four blocks off; joiner 1 stays at the origin.
    let seat = rig.at + Vec3::new(4.0, 0.0, 0.0);
    rig.hs.server.players[0].player.pos = seat;
    rig.give(0, 0, bread(), 9);
    let (holder_slot, other_slot) = (rig.cs[0].slot, rig.cs[1].slot);
    assert_eq!((holder_slot, other_slot), (1, 2), "slot 0 is the host's seat");
    let other_feet = pos_of(&rig, other_slot);

    let out = rig.use_block(0, table, UseKind::Feast, 0).clone();
    accepted(&out, 9);
    assert_ne!(out.window_event, 0, "the whole stack is paid as a window event");
    assert_eq!(rig.cs[0].count(&bread()), 0, "the holder's stack is gone");
    rig.assert_lockstep(0, "after the feast");

    // The marker, with the guest count as its meta, in every world.
    let host_world = &rig.host.as_ref().unwrap().world;
    assert_eq!(host_world.get_block(pos.0, pos.1, pos.2), block::FEAST_MARKER);
    assert_eq!(host_world.meta_at(pos.0, pos.1, pos.2), 2, "two guests: the seat and the other joiner");
    assert_eq!(host_world.feast_events, vec![(pos.0, pos.1, pos.2, 2)], "the host's client will see it too");
    for n in 0..2 {
        let w = &rig.cs[n].world;
        assert_eq!(w.get_block(pos.0, pos.1, pos.2), block::FEAST_MARKER, "joiner {n} sees the marker");
        assert_eq!(w.meta_at(pos.0, pos.1, pos.2), 2, "joiner {n} sees the guest count");
    }
    assert_eq!(rig.cs[1].world.feast_events, vec![(pos.0, pos.1, pos.2, 2)], "a joiner SEES the feast, for its confetti and bloom");
    assert!(
        rig.cs[0].changes.iter().any(|c| (c.x, c.y, c.z) == pos && c.new_block == block::FEAST_MARKER && c.meta == 2),
        "the change went out carrying the meta"
    );

    // The drops: real items at each guest's feet, spawned by the authority.
    assert_eq!(rig.ground(&bread()), 9, "every loaf is on the ground, none lost or duplicated");
    assert_eq!(bread_near(&rig, other_feet, 1.5), 5, "the nearer guest (the other joiner) gets the extra loaf");
    assert_eq!(bread_near(&rig, seat, 1.5), 4, "the host's seat is served too");
}

/// The same feast on a dedicated server (no host seat): the one other joiner
/// gets the lot.
#[test]
fn a_dedicated_server_serves_the_same_feast() {
    let mut rig = Rig::dedicated("feast-dedicated", 2);
    let table = rig.place(2, block::FEAST_TABLE);
    rig.give(1, 3, bread(), 7);
    rig.hs.server.players[rig.cs[1].slot].hotbar_slot = 3;
    let other_feet = pos_of(&rig, rig.cs[0].slot);
    let out = rig.use_block(1, table, UseKind::Feast, 3).clone();
    accepted(&out, 7);
    assert_eq!(rig.world().get_block(table[0], table[1], table[2]), block::FEAST_MARKER);
    assert_eq!(rig.world().meta_at(table[0], table[1], table[2]), 1);
    assert_eq!(bread_near(&rig, other_feet, 1.5), 7);
    rig.assert_lockstep(1, "after the feast");
}

/// With no other player and no villager near, a feast is refused and nothing
/// is taken: the table stays a table.
#[test]
fn a_feast_with_no_guests_is_refused_and_takes_nothing() {
    let mut rig = Rig::dedicated("feast-alone", 1);
    let table = rig.place(2, block::FEAST_TABLE);
    rig.give(0, 0, bread(), 5);
    let out = rig.use_block(0, table, UseKind::Feast, 0).clone();
    refused(&out, ItemNote::NeedsGuests);
    assert_eq!(ItemNote::NeedsGuests.toast(), Some("A feast needs guests. Bring friends, or find a village."));
    assert_eq!(rig.world().get_block(table[0], table[1], table[2]), block::FEAST_TABLE);
    assert_eq!(rig.cs[0].count(&bread()), 5);
    assert_eq!(rig.ground(&bread()), 0);
    rig.assert_lockstep(0, "after the refusal");
}

/// A lone joiner near a village feasts the villagers: their shares are used up
/// (nothing on the ground), and the marker's meta counts them.
#[test]
fn villagers_are_the_guests_of_a_lone_joiner() {
    let mut rig = Rig::dedicated("feast-villagers", 1);
    let table = rig.place(2, block::FEAST_TABLE);
    let at = rig.at;
    for dx in [3.0, 5.0, 7.0] {
        crate::entity::spawn_mob(&mut rig.hs.server.ecs, crate::mob::MobType::Villager, at + Vec3::new(dx, 0.0, 2.0));
    }
    // A fourth, out of reach (more than 32 blocks off), is no guest.
    crate::entity::spawn_mob(&mut rig.hs.server.ecs, crate::mob::MobType::Villager, at + Vec3::new(60.0, 0.0, 2.0));
    rig.give(0, 0, bread(), 6);
    let out = rig.use_block(0, table, UseKind::Feast, 0).clone();
    accepted(&out, 6);
    assert_eq!(rig.world().meta_at(table[0], table[1], table[2]), 3, "three villagers fed");
    assert_eq!(rig.ground(&bread()), 0, "villagers' shares are used up, not dropped");
    rig.assert_lockstep(0, "after the feast");
}

/// A modified client claims bread it doesn't hold: the server's copy of its
/// window holds none, so ONE is believed (within the believed-pay bound), never
/// the 64 it claims. One loaf is under the four a feast needs, so it is refused
/// and nothing is dropped or taken.
#[test]
fn a_modified_client_cannot_feast_a_stack_it_does_not_hold() {
    let mut rig = Rig::dedicated("feast-liar", 2);
    let table = rig.place(2, block::FEAST_TABLE);
    let before = rig.cs[0].outcomes.len();
    rig.cs[0].use_raw(77, table, 0, &bread());
    rig.ticks(3);
    assert_eq!(rig.cs[0].outcomes.len(), before + 1, "answered");
    refused(&rig.cs[0].outcomes.last().unwrap().clone(), ItemNote::NeedsFood);
    assert_eq!(rig.ground(&bread()), 0, "no stack of 64, and the one believed loaf is too few");
    assert_eq!(rig.world().get_block(table[0], table[1], table[2]), block::FEAST_TABLE);
}

/// Tier 1 review L4, closed (S1): a feast needs at least four edible items in
/// the stack the joiner holds. Dirt, wheat and three loaves are refused with
/// the reason, nothing is taken, the table stays a table; four loaves feast.
#[test]
fn a_feast_needs_four_things_to_eat() {
    let mut rig = Rig::dedicated("feast-food", 2);
    let table = rig.place(2, block::FEAST_TABLE);
    rig.give(0, 0, Item::Block(block::DIRT), 32);
    refused(&rig.use_block(0, table, UseKind::Feast, 0).clone(), ItemNote::NeedsFood);
    assert_eq!(ItemNote::NeedsFood.toast(), Some("A feast needs at least four things to eat."));
    assert_eq!(rig.world().get_block(table[0], table[1], table[2]), block::FEAST_TABLE);
    assert_eq!(rig.cs[0].count(&Item::Block(block::DIRT)), 32, "nothing was taken");
    rig.assert_lockstep(0, "after the dirt refusal");

    rig.give(0, 1, bread(), 3);
    refused(&rig.use_block(0, table, UseKind::Feast, 1).clone(), ItemNote::NeedsFood);
    assert_eq!(rig.cs[0].count(&bread()), 3);

    rig.give(0, 3, bread(), 4);
    accepted(&rig.use_block(0, table, UseKind::Feast, 3).clone(), 4);
    assert_eq!(rig.world().get_block(table[0], table[1], table[2]), block::FEAST_MARKER);
}

/// Tier 1 review L4, closed (S1): a column's Feast surge cannot be renewed
/// within one world day, on the authority's own clock. A second table in the
/// same 16 x 16 column is refused with the reason, a table in the next column
/// over is not, and the next night's number lets the first column feast again.
#[test]
fn a_column_cannot_feast_twice_in_one_world_day() {
    let mut rig = Rig::dedicated("feast-day", 2);
    let first = rig.place(1, block::FEAST_TABLE);
    rig.give(0, 0, bread(), 8);
    accepted(&rig.use_block(0, first, UseKind::Feast, 0).clone(), 8);
    assert!(crate::feast::feasted_today(rig.world(), (first[0], first[1], first[2]), 0));

    // Another table in the same column, the same day.
    let second = rig.place(2, block::FEAST_TABLE);
    assert_eq!(
        crate::feast::surge_column(first[0], first[2]),
        crate::feast::surge_column(second[0], second[2]),
        "same column"
    );
    rig.give(0, 1, bread(), 8);
    refused(&rig.use_block(0, second, UseKind::Feast, 1).clone(), ItemNote::FeastedToday);
    assert_eq!(ItemNote::FeastedToday.toast(), Some("This place has feasted today."));
    assert_eq!(rig.world().get_block(second[0], second[1], second[2]), block::FEAST_TABLE);
    assert_eq!(rig.cs[0].count(&bread()), 8, "nothing was taken");
    rig.assert_lockstep(0, "after the day refusal");

    // The next world day (the authority's night count moves on): allowed again.
    rig.hs.server.night_calendar = crate::item_actions::NightCalendar::from_saved(1, Some(false));
    accepted(&rig.use_block(0, second, UseKind::Feast, 1).clone(), 8);
    assert_eq!(rig.world().get_block(second[0], second[1], second[2]), block::FEAST_MARKER);
}

/// Using a Feast Marker is a client-local read: its guest count is the meta the
/// joiner already holds, and the name is the one every client works out.
#[test]
fn a_markers_name_and_guests_are_the_same_on_every_client() {
    let mut rig = Rig::lent("feast-marker", 1);
    let table = rig.place(2, block::FEAST_TABLE);
    rig.hs.server.players[0].player.pos = rig.at + Vec3::new(2.0, 0.0, 0.0);
    rig.give(0, 0, bread(), 4);
    accepted(&rig.use_block(0, table, UseKind::Feast, 0).clone(), 4);
    let (x, z) = (table[0], table[2]);
    assert_eq!(rig.cs[0].world.meta_at(x, table[1], z), 1);
    let host = crate::feast::toast(&crate::feast::name_at(x, z), rig.host.as_ref().unwrap().world.meta_at(x, table[1], z));
    let joiner = crate::feast::toast(&crate::feast::name_at(x, z), rig.cs[0].world.meta_at(x, table[1], z));
    assert_eq!(host, joiner);
    assert!(joiner.starts_with("The Feast of ") && joiner.ends_with("1 guest."), "{joiner}");
}

/// A late joiner's chunk push carries the marker AND its meta (B2a).
#[test]
fn a_late_joiners_chunk_push_carries_the_markers_guest_count() {
    use super::push_joiner::{start_host, Joiner};
    let mut hs = start_host("feast-late");
    let host = hs.server.players[0].player.pos;
    let (x, z) = (host.x.floor() as i32 + 3 + 2 * 16, host.z.floor() as i32);
    hs.server.world.set_block(x, 90, z, block::FEAST_MARKER);
    hs.server.world.set_meta((x, 90, z), 17);
    hs.server.world.set_block(x + 4, 90, z, block::COMMONS_STONE);
    let mut j = Joiner::join(&mut hs, 4, false);
    j.settle(&mut hs);
    assert_eq!(j.world.get_block(x, 90, z), block::FEAST_MARKER, "the push carried the marker");
    assert_eq!(j.world.meta_at(x, 90, z), 17, "and its guest count");
    assert_eq!(j.world.feast_markers(), vec![(x, z, 17)], "so a late joiner's bloom feed sees it");
    assert_eq!(j.world.commons_areas(), vec![crate::commons::footprint(x + 4, z)], "and the Commons");
    assert!(j.world.feast_events.is_empty(), "arriving in a chunk is no new feast");
}

// ─── Commons ───────────────────────────────────────────────────────────────

/// A joiner's Commons Stone over a Plot is refused (the authority holds the
/// plots; a joiner's world has none); one clear of every Plot stands and is
/// found as a Commons area; and a Plot Marker over that Commons is refused.
#[test]
fn a_joiners_commons_over_a_plot_is_refused_and_a_clear_one_stands() {
    let mut rig = Rig::dedicated("commons", 1);
    let (cx, cy, cz) = (rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32);
    // Someone's plot whose footprint reaches the stone's but doesn't hold its cell.
    rig.world().plots.push(PlotData::from_marker(PlotOwner::Npub("npub1someoneelse".into()), cx, cy, cz + 30));
    rig.give(0, 0, Item::Block(block::COMMONS_STONE), 2);
    let over = (cx, cy, cz + 3);
    assert!(!rig.world().plots[0].contains_column(over.0, over.2), "the cell itself is outside the plot");
    rig.edit(0, over, block::COMMONS_STONE);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(over.0, over.1, over.2), block::AIR, "refused: the land is claimed");
    assert!(rig.world().commons_areas().is_empty());

    // Away from every plot (the other side of the joiner) it stands.
    let clear = (cx, cy, cz - 4);
    rig.edit(0, clear, block::COMMONS_STONE);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(clear.0, clear.1, clear.2), block::COMMONS_STONE, "a clear stone stands");
    assert_eq!(rig.world().commons_areas(), vec![crate::commons::footprint(clear.0, clear.2)]);

    // A Plot Marker whose claim would cover that Commons is refused.
    let marker = (cx + 1, cy, cz - 2);
    rig.edit(0, marker, block::PLOT_MARKER);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(marker.0, marker.1, marker.2), block::AIR, "a plot over a Commons is refused");
}

/// Review M2 — a modified joiner cannot make a Feast Marker, or rewrite a
/// marker's guest count, with a plain block edit; only the feast rule makes one.
/// Breaking a marker is still an ordinary edit.
#[test]
fn a_joiner_cannot_forge_a_marker_or_its_guest_count() {
    let mut rig = Rig::dedicated("feast-forge", 1);
    let (cx, cy, cz) = (rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32);
    // A forged marker over empty ground, and over a table.
    let empty = (cx, cy, cz + 2);
    rig.edit_with_meta(0, empty, block::FEAST_MARKER, 255);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(empty.0, empty.1, empty.2), block::AIR, "no forged marker over air");
    assert_eq!(rig.world().meta_at(empty.0, empty.1, empty.2), 0);
    let table = rig.place(3, block::FEAST_TABLE);
    rig.edit_with_meta(0, (table[0], table[1], table[2]), block::FEAST_MARKER, 255);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(table[0], table[1], table[2]), block::FEAST_TABLE, "a table is not converted by a plain edit");
    assert!(rig.world().feast_events.is_empty(), "and no feast is noted");

    // A real marker (the feast rule's) keeps its count against a meta-only edit.
    let marker = rig.place(4, block::FEAST_MARKER);
    rig.world().set_meta((marker[0], marker[1], marker[2]), 3);
    rig.edit_with_meta(0, (marker[0], marker[1], marker[2]), block::FEAST_MARKER, 255);
    rig.ticks(3);
    assert_eq!(rig.world().meta_at(marker[0], marker[1], marker[2]), 3, "the guest count is the feast's");

    // Breaking it is allowed, and leaves no count behind.
    rig.edit(0, (marker[0], marker[1], marker[2]), block::AIR);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(marker[0], marker[1], marker[2]), block::AIR);
    assert_eq!(rig.world().meta_at(marker[0], marker[1], marker[2]), 0);
}

/// A world with a Feast Marker and a Commons Stone saves and loads with the
/// marker's guest count intact (it is `block_meta`), and the Commons is found
/// again from the loaded chunks.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_world_with_a_marker_and_a_commons_save_loads_with_its_meta() {
    use crate::save;
    let name = "__test_feast_commons_save__";
    let _ = save::delete_world(name);
    let mut world = crate::world::World::new();
    world.set_block(10, 70, 12, block::FEAST_MARKER);
    world.set_meta((10, 70, 12), 213);
    world.set_block(-40, 64, 5, block::COMMONS_STONE);
    let slot = crate::player_slot::PlayerSlot::new(0, Vec3::new(0.0, 80.0, 0.0), 1.0);
    save::save_world(name, &world, &[slot], 42, &[], &[], None, None).expect("save_world");
    let mut loaded = crate::world::World::new();
    let (_save, _chunks) = save::load_world(name, &mut loaded).expect("load_world");
    assert_eq!(loaded.get_block(10, 70, 12), block::FEAST_MARKER);
    assert_eq!(loaded.meta_at(10, 70, 12), 213, "the guest count survives the save");
    assert_eq!(loaded.feast_markers(), vec![(10, 12, 213)]);
    assert_eq!(loaded.commons_areas(), vec![crate::commons::footprint(-40, 5)]);
    let _ = save::delete_world(name);
}

/// The save layout moved with the new block ids, so a build before them
/// refuses the world instead of reading a table as AIR.
#[test]
fn the_save_and_the_wire_moved_with_the_new_block_ids() {
    // Tier 1 took the layout to 5 (these ids); Tier 2 S3's Worn Path (376) took it to 6.
    // S8's Blighted Crop (377) took it to 7; S6's Commons Shelf (378) took it to 8, and S7's Site Board (379) joined it.
    assert_eq!(crate::save_format::SAVE_LAYOUT_REVISION, 8);
    assert_eq!(crate::save_format::SAVE_FORMAT_VERSION, 70, "62 fields + layout revision 8");
    assert_eq!(crate::protocol::PROTOCOL_VERSION, 100);
    let reg = block::BlockRegistry::new();
    for b in [block::FEAST_TABLE, block::FEAST_MARKER, block::COMMONS_STONE] {
        assert!(reg.is_known(b));
        assert!(reg.get(b).solid);
        assert!(!reg.get(b).name.is_empty());
    }
    assert_eq!(reg.mine_drop(block::FEAST_MARKER).item, Item::Block(block::FEAST_TABLE), "a marker breaks into a table");
    assert_eq!(reg.mine_drop(block::COMMONS_STONE).item, Item::Block(block::COMMONS_STONE));
}
