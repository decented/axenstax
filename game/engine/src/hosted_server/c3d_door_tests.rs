//! C3d run 18 — doors (Decision 20): the Strict compound door place, break
//! and toggle. C3d run 19 — a Mirror joiner's toggle reaches the server and
//! every other player (run 18's finding, fixed): a typed `ToggleBlock`
//! served under `FlipEach`, refusals sent back.
use super::*;
use crate::action_identity::InventoryMode;
use crate::block::{AIR, OAK_DOOR, STONE};
use crate::remote_client::RemoteClient;

/// A Mirror (default) dedicated server whose joiner stands at (40.5, 80,
/// 40.5) on a stone floor, its column held, and the client's own world.
pub(super) struct MirrorJoined {
    _g: crate::save::WorldsRootGuard,
    pub(super) hs: HostedServer,
    pub(super) rc: RemoteClient,
    pub(super) world: crate::world::World,
    /// The fates of the action receipts this client retired, in order.
    pub(super) fates: Vec<crate::action_identity::ActionFate>,
}

pub(super) fn mirror_joined(tag: &str) -> MirrorJoined {
    let g = crate::save::WorldsRootGuard::new(&format!("c3d-door-{tag}"));
    let mut hs = super::tests::start_room_test_server(&format!("c3d-door-{tag}"), 0);
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
    let col = (40i32.div_euclid(cs), 40i32.div_euclid(cs));
    hs.server.loaded_columns.insert(col);
    let rc = RemoteClient::from_transport(Box::new(hs.attach_test_remote()), crate::remote_client::build_join_request_guest("Doorman", 0), None);
    let mut m = MirrorJoined { _g: g, hs, rc, world: crate::world::World::new(), fates: Vec::new() };
    for _ in 0..20 {
        m.pump();
        if m.rc.player_index().is_some() { break; }
    }
    let i = m.rc.player_index().expect("the guest joined") as usize;
    super::tests::install_client_initial_personal(&mut m.rc, &mut m.hs, i, HostedServer::tick);
    assert_eq!(m.rc.inventory_mode, InventoryMode::Mirror, "the session is Mirror");
    m.hs.hold_column_for_test(i, col);
    let p = &mut m.hs.server.players[i].player;
    p.pos = glam::Vec3::new(40.5, 80.0, 40.5);
    p.velocity = glam::Vec3::ZERO;
    p.on_ground = true;
    m
}

impl MirrorJoined {
    /// One server tick; the client's packets read, its block changes
    /// applied to its world and its action receipts retired (as the game
    /// loop's intake does).
    pub(super) fn pump(&mut self) {
        self.hs.tick();
        self.rc.poll();
        for change in std::mem::take(&mut self.rc.pending_block_changes) { self.world.apply_remote_block_change(&change); }
        for (_, item) in std::mem::take(&mut self.rc.chunk_queue) {
            if let crate::chunk_intake::StreamItem::ActionReceipt(receipt) = item {
                self.rc.apply_action_receipt(receipt);
                self.fates.push(receipt.fate);
            }
        }
    }

    /// The game loop's Mirror toggle arm (run 19): ONE reserved
    /// `ToggleBlock` (hotbar slot 0), then `FlipEach` predicted in its own
    /// world exactly as before. Pumped until the receipt retires.
    pub(super) fn toggle(&mut self, cell: [i32; 3]) -> crate::action_identity::ActionFate {
        let expected = self.world.get_block(cell[0], cell[1], cell[2]);
        crate::strict_doors::reserve_toggle(&mut self.rc, &self.world, cell, expected, 0).expect("reserved and sent");
        for (c, meta) in crate::strict_doors::toggle_metas(&self.world, cell, crate::strict_doors::OpenRule::FlipEach) {
            self.world.set_meta((c[0], c[1], c[2]), meta);
        }
        let before = self.fates.len();
        for _ in 0..6 {
            self.pump();
            if self.fates.len() > before { break; }
        }
        assert_eq!(self.fates.len(), before + 1, "the toggle's receipt retired");
        assert_eq!(self.rc.action_journal.as_ref().map(|j| j.actions().count()), Some(0), "nothing left in the journal");
        assert!(!self.hs.disconnected[self.rc.player_index().expect("still joined") as usize], "never torn down");
        self.fates[before]
    }

    pub(super) fn open_on_server(&self, c: [i32; 3]) -> bool { crate::block_shape::is_open(self.hs.server.world.meta_at(c[0], c[1], c[2])) }
    pub(super) fn open_on_client(&self, c: [i32; 3]) -> bool { crate::block_shape::is_open(self.world.meta_at(c[0], c[1], c[2])) }
}

/// A second Mirror guest joined to `m`'s server, its column held: its
/// client and its own world (fed only by what the server sends it).
fn second_joiner(m: &mut MirrorJoined) -> (RemoteClient, crate::world::World) { second_guest(&mut m.hs) }

/// A second guest joined to `hs` (either mode), its column held: its
/// client and its own world (fed only by what the server sends it).
fn second_guest(hs: &mut HostedServer) -> (RemoteClient, crate::world::World) {
    let mut rc = RemoteClient::from_transport(Box::new(hs.attach_test_remote()), crate::remote_client::build_join_request_guest("Neighbour", 0), None);
    for _ in 0..20 {
        hs.tick();
        rc.poll();
        if rc.player_index().is_some() { break; }
    }
    let i = rc.player_index().expect("the second guest joined") as usize;
    let cs = crate::chunk::CHUNK_SIZE as i32;
    hs.hold_column_for_test(i, (40i32.div_euclid(cs), 40i32.div_euclid(cs)));
    let p = &mut hs.server.players[i].player;
    p.pos = glam::Vec3::new(38.5, 80.0, 38.5);
    p.velocity = glam::Vec3::ZERO;
    p.on_ground = true;
    hs.tick();
    rc.poll();
    let _ = std::mem::take(&mut rc.pending_block_changes);
    (rc, crate::world::World::new())
}

/// `other` reads what the server sent it into `world`.
fn hear(other: &mut RemoteClient, world: &mut crate::world::World) {
    other.poll();
    for change in std::mem::take(&mut other.pending_block_changes) { world.apply_remote_block_change(&change); }
}

/// A closed door (bottom at `bottom`, facing `f`) standing in `world`.
pub(super) fn stand_door(world: &mut crate::world::World, bottom: [i32; 3], f: crate::meta::Facing) {
    let m = crate::meta::with_facing(0, f);
    world.set_block(bottom[0], bottom[1], bottom[2], OAK_DOOR);
    world.set_meta((bottom[0], bottom[1], bottom[2]), m);
    world.set_block(bottom[0], bottom[1] + 1, bottom[2], OAK_DOOR);
    world.set_meta((bottom[0], bottom[1] + 1, bottom[2]), crate::use_edits::door_top_meta(m));
}

/// Run 18's finding, fixed in run 19: a Mirror joiner opens a door (the
/// game loop's Mirror arm: a typed `ToggleBlock`, `FlipEach` predicted) and
/// the server — the world every other player is served from — opens it
/// too; closing it again closes it. Served `Accepted`, never torn down.
#[test]
fn c3d_mirror_joiner_door_toggle_reaches_the_server() {
    let mut m = mirror_joined("mirror-toggle");
    let bottom = [41, 80, 40];
    stand_door(&mut m.hs.server.world, bottom, crate::meta::Facing::North);
    stand_door(&mut m.world, bottom, crate::meta::Facing::North);
    for (click, open) in [([41, 80, 40], true), ([41, 81, 40], false)] {
        assert_eq!(m.toggle(click), crate::action_identity::ActionFate::Accepted, "served");
        for y in [80, 81] {
            assert_eq!(m.open_on_client([41, y, 40]), open, "the joiner's own half at y={y}");
            assert_eq!(m.open_on_server([41, y, 40]), open, "the server (and so every other player) sees the door at y={y} {}", if open { "open" } else { "shut" });
        }
    }
}

/// Run 19: another joiner hears the door open (the server broadcasts the
/// footprint it applied).
#[test]
fn c3d_mirror_joiner_door_toggle_reaches_a_second_joiner() {
    let mut m = mirror_joined("mirror-toggle-second");
    let (mut other, mut other_world) = second_joiner(&mut m);
    for w in [&mut m.hs.server.world, &mut m.world, &mut other_world] { stand_door(w, [41, 80, 40], crate::meta::Facing::North); }
    assert_eq!(m.toggle([41, 80, 40]), crate::action_identity::ActionFate::Accepted);
    for _ in 0..3 {
        m.pump();
        other.poll();
        for change in std::mem::take(&mut other.pending_block_changes) { other_world.apply_remote_block_change(&change); }
    }
    for y in [80, 81] {
        assert!(crate::block_shape::is_open(other_world.meta_at(41, y, 40)), "the second joiner sees the door at y={y} open");
    }
}

/// C3d run 20 — a toggle is a USE, not an edit: on an Adventure world
/// (where a joiner can't break or place) a Mirror joiner still opens and
/// shuts a door, as single-player always let it; served `Accepted`, the
/// server and a second joiner see it, never torn down.
#[test]
fn c3d_mirror_adventure_joiner_opens_and_shuts_a_door() {
    let mut m = mirror_joined("mirror-toggle-adventure");
    m.hs.server.play_mode = crate::play_mode::PlayMode::Adventure;
    let (mut other, mut other_world) = second_joiner(&mut m);
    for w in [&mut m.hs.server.world, &mut m.world, &mut other_world] { stand_door(w, [41, 80, 40], crate::meta::Facing::North); }
    for (click, open) in [([41, 80, 40], true), ([41, 81, 40], false)] {
        assert_eq!(m.toggle(click), crate::action_identity::ActionFate::Accepted, "an Adventure joiner's toggle is served");
        for _ in 0..3 { m.pump(); hear(&mut other, &mut other_world); }
        for y in [80, 81] {
            assert_eq!(m.open_on_server([41, y, 40]), open, "the server's half at y={y}");
            assert_eq!(m.open_on_client([41, y, 40]), open, "the joiner's own half at y={y}");
            assert_eq!(crate::block_shape::is_open(other_world.meta_at(41, y, 40)), open, "the second joiner's half at y={y}");
        }
    }
}

/// Run 19: a toggle the server refuses (the door stands in someone else's
/// plot) changes nothing on the server, is sent back — the joiner's door
/// shuts again — and retires `Refused`, never torn down.
#[test]
fn c3d_mirror_joiner_toggle_in_a_foreign_plot_is_sent_back() {
    let mut m = mirror_joined("mirror-toggle-plot");
    m.hs.server.world.plots.push(crate::plot::PlotData { owner: crate::plot::PlotOwner::Npub("npub1someoneelse".into()), marker: (50, 80, 40), min_x: 36, max_x: 58, min_z: 30, max_z: 50 });
    stand_door(&mut m.hs.server.world, [41, 80, 40], crate::meta::Facing::North);
    stand_door(&mut m.world, [41, 80, 40], crate::meta::Facing::North);
    assert_eq!(m.toggle([41, 80, 40]), crate::action_identity::ActionFate::Refused, "refused");
    for y in [80, 81] {
        assert!(!m.open_on_server([41, y, 40]), "the server's half at y={y} stays shut");
        assert!(!m.open_on_client([41, y, 40]), "the joiner's half at y={y} was sent back shut");
    }
}

/// Run 19: a double door opens whole (both halves of both doors), a
/// trapdoor and a fence gate open, on the server and the joiner alike.
#[test]
fn c3d_mirror_joiner_toggles_a_double_door_a_trapdoor_and_a_gate() {
    let mut m = mirror_joined("mirror-toggle-kinds");
    for w in [&mut m.hs.server.world, &mut m.world] {
        stand_door(w, [41, 80, 41], crate::meta::Facing::North);
        stand_door(w, [42, 80, 41], crate::meta::Facing::North);
        w.set_block(39, 80, 40, crate::block::OAK_TRAPDOOR);
        w.set_block(40, 80, 42, crate::block::OAK_FENCE_GATE);
        w.set_meta((40, 80, 42), crate::meta::with_facing(0, crate::meta::Facing::North));
    }
    let cases: [([i32; 3], Vec<[i32; 3]>); 3] = [
        ([42, 81, 41], vec![[41, 80, 41], [41, 81, 41], [42, 80, 41], [42, 81, 41]]),
        ([39, 80, 40], vec![[39, 80, 40]]),
        ([40, 80, 42], vec![[40, 80, 42]]),
    ];
    for (click, cells) in cases {
        assert_eq!(m.toggle(click), crate::action_identity::ActionFate::Accepted, "{click:?} served");
        for c in cells {
            assert!(m.open_on_server(c), "the server's cell {c:?} open after a click on {click:?}");
            assert_eq!(m.world.meta_at(c[0], c[1], c[2]), m.hs.server.world.meta_at(c[0], c[1], c[2]), "client and server agree at {c:?}");
        }
    }
}

// ─── Strict (Decision 20) ────────────────────────────────────────────────

use super::c3d_client_edit_tests::{joined, Joined};
use crate::item::{Item, ItemStack};
use crate::meta::Facing;
use crate::strict_doors::{door_halves, place_door_joined, toggle_joined, toggle_metas, OpenRule};

pub(super) fn doors(n: u8) -> ItemStack { ItemStack { item: Item::Block(OAK_DOOR), count: n } }

impl Joined {
    /// The game loop's Strict door placement (`take_for_placement`'s door
    /// arm, then both halves in its world).
    pub(super) fn place_door(&mut self, bottom: [i32; 3], slot: usize, f: Facing) {
        let facing = f.to_bits();
        let placed = place_door_joined(&mut self.rc, &mut self.ui, &mut self.inv, &mut self.armour, slot, bottom, facing, &self.reg).expect("reserved and sent");
        assert_eq!(placed, Some(OAK_DOOR), "the window gave a door");
        for (c, m) in door_halves(bottom, facing).unwrap() {
            self.world.place_player_block(c[0], c[1], c[2], OAK_DOOR);
            self.world.set_meta((c[0], c[1], c[2]), m);
        }
    }

    /// The game loop's Strict toggle arm: the typed `ToggleBlock`, then the
    /// server's canonical rule predicted in its world.
    pub(super) fn toggle(&mut self, cell: [i32; 3]) { self.toggle_from(cell, 0) }

    /// [`Self::toggle`] with hotbar `slot` selected (run 19: its item sets
    /// the reach).
    pub(super) fn toggle_from(&mut self, cell: [i32; 3], slot: usize) {
        let expected = self.world.get_block(cell[0], cell[1], cell[2]);
        toggle_joined(&mut self.rc, &mut self.ui, &self.inv, &self.armour, &self.world, cell, expected, slot).expect("reserved and sent");
        for (c, m) in toggle_metas(&self.world, cell, OpenRule::Canonical) { self.world.set_meta((c[0], c[1], c[2]), m); }
    }

    /// The game loop's survival break of a door half (`break_route`, then
    /// the pair cleared in its world).
    pub(super) fn break_door(&mut self, cell: [i32; 3], slot: usize) {
        let old = (self.world.get_block(cell[0], cell[1], cell[2]), self.world.meta_at(cell[0], cell[1], cell[2]));
        let change = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: AIR, meta: 0 };
        let routed = crate::strict_edits::route_break(Some(&mut self.rc), &mut self.ui, &mut self.inv, &mut self.armour, slot, change, old, &self.reg).expect("reserved and sent");
        assert!(routed.is_typed(), "a door break goes typed");
        let pair_y = if crate::block_shape::door_is_top(old.1) { cell[1] - 1 } else { cell[1] + 1 };
        for y in [cell[1], pair_y] { self.world.set_block(cell[0], y, cell[2], AIR); self.world.set_meta((cell[0], y, cell[2]), 0); }
    }

    pub(super) fn server_cell(&self, c: [i32; 3]) -> (u16, u8) { (self.hs.server.world.get_block(c[0], c[1], c[2]), self.hs.server.world.meta_at(c[0], c[1], c[2])) }
    pub(super) fn client_cell(&self, c: [i32; 3]) -> (u16, u8) { (self.world.get_block(c[0], c[1], c[2]), self.world.meta_at(c[0], c[1], c[2])) }
    pub(super) fn server_doors(&self) -> u32 { self.sp().inventory.slots_iter().flatten().filter(|s| s.item == Item::Block(OAK_DOOR)).map(|s| u32::from(s.count)).sum() }
    pub(super) fn refusals(&self) -> (u32, u32, u32) { let t = self.sp().strict_refusals; (t.stale_world, t.other, t.world_fact) }
}

trait Routed2 { fn is_typed(&self) -> bool; }
impl<T> Routed2 for crate::strict_edits::Routed<T> { fn is_typed(&self) -> bool { matches!(self, crate::strict_edits::Routed::Typed(_)) } }

/// Decision 20, place: ONE compound action (one reservation, no
/// `DoorUpper`), both halves derived by the server with their orientation,
/// ONE door consumed on both copies, never torn down.
#[test]
fn c3d_strict_door_place_is_one_compound_action() {
    let mut j = joined("door-place", &[(0, doors(3))], false);
    let bottom = [41, 80, 40];
    // The refill setting goes first (as before every typed action); then
    // the door is exactly one more reservation.
    j.ui.sync_auto_refill_joined(&mut j.rc, &j.inv, &j.armour).expect("the setting");
    let before = j.rc.action_journal.as_ref().map_or(0, |a| a.len());
    j.place_door(bottom, 0, Facing::East);
    assert_eq!(j.rc.action_journal.as_ref().map(|a| a.len()), Some(before + 1), "one reservation for both halves");
    j.assert_honest("door place");
    let halves = door_halves(bottom, Facing::East.to_bits()).unwrap();
    for (c, m) in halves { assert_eq!(j.server_cell(c), (OAK_DOOR, m), "the server derived the half at {c:?}"); }
    assert_eq!((j.held(&Item::Block(OAK_DOOR)), j.server_doors()), (2, 2), "exactly one door consumed on both copies");
    assert_eq!(j.refusals(), (0, 0, 0), "no Strict refusal");
}

/// Decision 20, two credits: a door waits for two edit credits rather
/// than running on the last one (`MAX_BLOCK_CHANGES_PER_TICK` = 4).
#[test]
fn c3d_strict_door_place_waits_for_two_edit_credits() {
    let cobble = ItemStack { item: Item::Block(crate::block::COBBLESTONE), count: 8 };
    let mut j = joined("door-credits", &[(0, doors(1)), (1, cobble)], false);
    for c in super::c3d_client_edit_tests::CELLS { j.place(c, 1); }
    j.place_door([43, 80, 43], 0, Facing::North);
    j.hs.tick();
    assert!(super::c3d_client_edit_tests::CELLS.iter().all(|c| j.server_cell(*c).0 == crate::block::COBBLESTONE), "three placements served");
    assert_eq!(j.server_cell([43, 80, 43]).0, AIR, "the door waits: one credit is left, it needs two");
    j.hs.tick();
    assert_eq!((j.server_cell([43, 80, 43]).0, j.server_cell([43, 81, 43]).0), (OAK_DOOR, OAK_DOOR), "both halves next tick");
    j.assert_honest("door after placements");
}

/// Decision 20, toggle: a typed click opens and closes a single door and
/// a double door (canonical bit, every half), served, never torn down.
#[test]
fn c3d_strict_door_open_close_single_and_double() {
    let mut j = joined("door-toggle", &[], false);
    for w in [&mut j.hs.server.world, &mut j.world] {
        stand_door(w, [41, 80, 40], Facing::North);
        stand_door(w, [42, 80, 41], Facing::North);
        stand_door(w, [43, 80, 41], Facing::North);
    }
    for (click, cells, open) in [
        ([41, 80, 40], vec![[41, 80, 40], [41, 81, 40]], true),
        ([41, 81, 40], vec![[41, 80, 40], [41, 81, 40]], false),
        ([42, 81, 41], vec![[42, 80, 41], [42, 81, 41], [43, 80, 41], [43, 81, 41]], true),
        ([43, 80, 41], vec![[42, 80, 41], [42, 81, 41], [43, 80, 41], [43, 81, 41]], false),
    ] {
        j.toggle(click);
        j.assert_honest("toggle");
        for c in cells {
            assert_eq!(crate::block_shape::is_open(j.server_cell(c).1), open, "the server's half at {c:?} after a click on {click:?}");
            assert_eq!(j.server_cell(c), j.client_cell(c), "client and server agree at {c:?}");
        }
    }
    assert_eq!(j.refusals(), (0, 0, 0), "no Strict refusal");
}

/// C3d run 20 — the Strict twin: on an Adventure world a Strict joiner's
/// typed toggle opens and shuts a door (a use, not an edit), served with
/// no refusal and no teardown, and a second joiner sees it.
#[test]
fn c3d_strict_adventure_joiner_opens_and_shuts_a_door() {
    let mut j = joined("door-toggle-adventure", &[], false);
    j.hs.server.play_mode = crate::play_mode::PlayMode::Adventure;
    let (mut other, mut other_world) = second_guest(&mut j.hs);
    for w in [&mut j.hs.server.world, &mut j.world, &mut other_world] { stand_door(w, [41, 80, 40], Facing::North); }
    for (click, open) in [([41, 80, 40], true), ([41, 81, 40], false)] {
        j.toggle(click);
        j.assert_honest("an Adventure toggle");
        hear(&mut other, &mut other_world);
        for c in [[41, 80, 40], [41, 81, 40]] {
            assert_eq!(crate::block_shape::is_open(j.server_cell(c).1), open, "the server's half at {c:?}");
            assert_eq!(j.server_cell(c), j.client_cell(c), "client and server agree at {c:?}");
            assert_eq!(crate::block_shape::is_open(other_world.meta_at(c[0], c[1], c[2])), open, "the second joiner's half at {c:?}");
        }
    }
    assert_eq!(j.refusals(), (0, 0, 0), "no Strict refusal");
}

/// Decision 20, break: a mined door primary derives its pair once, yields
/// one door, served, never torn down.
#[test]
fn c3d_strict_door_break_derives_the_pair_once() {
    let mut j = joined("door-break", &[(0, doors(1))], false);
    j.place_door([41, 80, 40], 0, Facing::South);
    j.assert_honest("place");
    j.break_door([41, 81, 40], 1);
    j.assert_honest("break");
    for y in [80, 81] { assert_eq!(j.server_cell([41, y, 40]).0, AIR, "both halves gone at y={y}"); }
    assert_eq!((j.held(&Item::Block(OAK_DOOR)), j.server_doors()), (1, 1), "one door back, once");
    let ground: u32 = j.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().filter(|(_, e)| e.stack.item == Item::Block(OAK_DOOR)).map(|(_, e)| u32::from(e.stack.count)).sum();
    assert_eq!(ground, 0, "no second door on the ground");
    assert_eq!(j.refusals(), (0, 0, 0), "no Strict refusal");
}

/// Decision 20, a forbidden upper cell: the top is above the world, so the
/// whole placement refuses — zero effect (no half, no door taken) — and is
/// corrected (the client's predicted door and window put back).
#[test]
fn c3d_strict_door_place_with_a_forbidden_upper_cell_is_refused_whole() {
    let mut j = joined("door-forbidden-top", &[(0, doors(3))], false);
    let top_y = crate::world::WORLD_HEIGHT - 1;
    for x in 39..=42 { for z in 39..=42 { j.hs.server.world.set_block(x, top_y - 3, z, STONE); } }
    let p = &mut j.hs.server.players[j.i].player;
    p.pos = glam::Vec3::new(40.5, (top_y - 2) as f32, 40.5);
    let bottom = [41, top_y, 40];
    j.place_door(bottom, 0, Facing::North);
    j.assert_honest("forbidden top");
    assert_eq!(j.server_cell(bottom).0, AIR, "no half committed");
    assert_eq!(j.client_cell(bottom).0, AIR, "the client's predicted half sent back");
    assert_eq!((j.held(&Item::Block(OAK_DOOR)), j.server_doors()), (3, 3), "no door taken; the client's window corrected");
    assert_eq!(j.refusals(), (0, 1, 0), "one rule refusal");
}

/// Decision 20, a forbidden companion of a toggle: the adjacent door of a
/// double door stands in someone else's plot, so the whole click refuses —
/// no half of either door changes — and is corrected.
#[test]
fn c3d_strict_double_door_toggle_with_a_forbidden_companion_is_refused_whole() {
    let mut j = joined("door-forbidden-pair", &[], false);
    j.hs.server.world.plots.push(crate::plot::PlotData { owner: crate::plot::PlotOwner::Npub("npub1someoneelse".into()), marker: (50, 80, 40), min_x: 42, max_x: 58, min_z: 30, max_z: 50 });
    for w in [&mut j.hs.server.world, &mut j.world] {
        stand_door(w, [41, 80, 40], Facing::North);
        stand_door(w, [42, 80, 40], Facing::North);
    }
    j.toggle([41, 80, 40]);
    j.assert_honest("forbidden companion");
    for c in [[41, 80, 40], [41, 81, 40], [42, 80, 40], [42, 81, 40]] {
        assert!(!crate::block_shape::is_open(j.server_cell(c).1), "the server's half at {c:?} stays shut");
        assert_eq!(j.client_cell(c), j.server_cell(c), "the client's predicted half at {c:?} sent back");
    }
    assert_eq!(j.refusals(), (0, 1, 0), "one rule refusal");
}

/// Run 15's rule on a toggle: stale-world only when the block id differs
/// from the one the client clicked; any other refusal is a rule refusal.
#[test]
fn c3d_strict_toggle_stale_block_is_stale_world_otherwise_rule() {
    let mut j = joined("door-stale", &[], false);
    stand_door(&mut j.world, [41, 80, 40], Facing::North);
    j.hs.server.world.set_block(41, 80, 40, STONE);
    j.toggle([41, 80, 40]);
    j.assert_honest("stale toggle");
    assert_eq!(j.refusals(), (1, 0, 0), "a different block id is stale-world");
    assert_eq!(j.client_cell([41, 80, 40]).0, STONE, "the server's cell sent back");
    for w in [&mut j.hs.server.world, &mut j.world] { stand_door(w, [47, 80, 47], Facing::North); }
    j.toggle([47, 80, 47]);
    j.assert_honest("out of reach");
    assert_eq!(j.refusals(), (1, 1, 0), "the same block out of reach is a rule refusal");
    assert!(!crate::block_shape::is_open(j.client_cell([47, 80, 47]).1), "corrected shut");
}

/// Run 19 — the toggle names its hotbar slot, so its reach is an edit's: the
/// server copy's Reach Claw in that slot lengthens it (run 18's note), an
/// empty slot does not (a corrected rule refusal), in Strict as in Mirror.
#[test]
fn c3d_strict_toggle_reach_uses_the_server_copys_slot_item() {
    let claw = ItemStack { item: Item::Material(crate::item::MaterialId::ReachClaw), count: 1 };
    let mut j = joined("door-claw", &[(0, claw)], false);
    for w in [&mut j.hs.server.world, &mut j.world] { stand_door(w, [47, 80, 40], Facing::North); }
    j.toggle_from([47, 80, 40], 1);
    j.assert_honest("empty hand, 7 blocks");
    assert!(!crate::block_shape::is_open(j.server_cell([47, 80, 40]).1), "out of bare-hand reach: shut");
    assert_eq!(j.refusals(), (0, 1, 0), "a rule refusal");
    j.toggle_from([47, 80, 40], 0);
    j.assert_honest("Reach Claw, 7 blocks");
    for y in [80, 81] { assert!(crate::block_shape::is_open(j.server_cell([47, y, 40]).1), "the Claw reaches: open at y={y}"); }
    assert_eq!(j.refusals(), (0, 1, 0), "no further refusal");
}

/// Run-18 review M1 — a refused toggle never costs a window correction: an
/// honest Strict joiner holding right-click on a door in someone else's
/// plot is never torn down, draws no `Replace`, and its door ends shut as
/// the server holds it. Run 20 (run-19 review LOW-3): the hold is paced as
/// the game loop paces it — a click every 8 ticks for 200 ticks, each on a
/// fresh base after the last one's send-back — so 25 refusals land well
/// past the correction bound's 8 per 100 ticks, and no correction is ever
/// admitted.
#[test]
fn c3d_strict_held_toggle_in_a_foreign_plot_never_tears_down() {
    let mut j = joined("door-held-plot", &[], false);
    j.hs.server.world.plots.push(crate::plot::PlotData { owner: crate::plot::PlotOwner::Npub("npub1someoneelse".into()), marker: (50, 80, 40), min_x: 36, max_x: 58, min_z: 30, max_z: 50 });
    for w in [&mut j.hs.server.world, &mut j.world] { stand_door(w, [41, 80, 40], Facing::North); }
    for click in 0..25 {
        assert!(!j.hs.disconnected[j.i], "never torn down (before click {click})");
        j.toggle([41, 80, 40]);
        for _ in 0..8 { j.pump(); }
    }
    assert!(!j.hs.disconnected[j.i], "never torn down");
    assert_eq!(j.sp().strict_corrections, crate::strict_admission::CorrectionRate::default(), "no correction Replace ever admitted");
    assert!(j.hs.server.players[j.i].window_events.death_replace.is_none(), "no correction Replace outstanding");
    assert_eq!(j.refusals(), (0, 25, 0), "25 rule refusals, tallied");
    for c in [[41, 80, 40], [41, 81, 40]] {
        assert!(!crate::block_shape::is_open(j.client_cell(c).1), "the client's half at {c:?} ends shut");
        assert_eq!(j.client_cell(c), j.server_cell(c), "as the server holds it");
    }
}
