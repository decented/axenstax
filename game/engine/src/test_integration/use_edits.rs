//! C3c-1 (2026-10-08, protocol v80) — a joiner's block-edit uses are
//! mirrored on the server's copy of its inventory.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport (a
//! dedicated server, or a lending host). The joiner's client half is its
//! window (`Inventory`, `CraftingUi`, armour) and, for each use, the steps
//! its right-click arm takes (`game_loop.rs`): the tag read from the hand
//! BEFORE the use (`use_edits::tag`), then the arm's own inventory calls
//! (`consume_one_material`, `add_item`, `use_hotbar_tool`), then the edit
//! sent with its tag and the hand AFTER the use (the edit's `EditHand`).
//! The tests compare the server's copy with the client's, slot for slot.
//! Two tests drive the real `RemoteClient` send path (its pairing, its
//! hold-back and its order cut).

use glam::Vec3;

use crate::armour::ArmourItem;
use crate::block::{self, BlockId};
use crate::craft_ui::CraftingUi;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::protocol::{self, UseTag};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};
use crate::use_edits::UseKind;
use crate::window::{self, WindowClick};

use super::joiner_authority::join_guest;
use super::joiners_act::floor_and_stand;
use super::lent_world::{join_guest_lent, start_lent};

/// The floor cell beside the joiner's feet (in reach), and the air above it.
const FLOOR: [i32; 3] = [41, 79, 40];
const ABOVE: [i32; 3] = [41, 80, 40];

/// The joiner's client half.
struct Client {
    transport: ChannelClientTransport,
    slot: usize,
    ui: CraftingUi,
    inv: Inventory,
    armour: [Option<ArmourItem>; 4],
    /// The last window op's `op_seq`.
    seq: u32,
    /// The highest window event applied.
    events: u32,
    input_seq: u64,
    /// The selected hotbar slot.
    hot: usize,
}

struct Rig {
    hs: HostedServer,
    host: Option<OwnedSimParts>,
    c: Client,
    /// The server tick the last use's edit was processed on.
    used_on: u64,
}

impl Rig {
    fn dedicated(tag: &str) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("use-edits-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let joined = join_guest(&mut hs, "Gardener");
        Self::stand(hs, None, joined)
    }

    fn lent(tag: &str) -> Self {
        let (mut hs, mut host) = start_lent(&format!("use-edits-{tag}"));
        let joined = join_guest_lent(&mut hs, &mut host, "Gardener");
        Self::stand(hs, Some(host), joined)
    }

    fn stand(mut hs: HostedServer, mut host: Option<OwnedSimParts>, (transport, slot): (ChannelClientTransport, usize)) -> Self {
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        let world = match host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut hs.server.world,
        };
        let mut world = std::mem::replace(world, crate::world::World::new());
        floor_and_stand(&mut world, &mut hs, slot);
        match host.as_mut() {
            Some(h) => h.world = world,
            None => hs.server.world = world,
        }
        let c = Client {
            transport,
            slot,
            ui: CraftingUi::new(),
            inv: Inventory::new(),
            armour: [None; 4],
            seq: 0,
            events: 0,
            input_seq: 0,
            hot: 0,
        };
        let mut rig = Rig { hs, host, c, used_on: 0 };
        rig.c.inv.auto_refill = rig.hs.server.players[slot].inventory.auto_refill;
        rig.tick();
        rig
    }

    fn world(&mut self) -> &mut crate::world::World {
        match self.host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut self.hs.server.world,
        }
    }

    fn block(&mut self, cell: [i32; 3]) -> BlockId {
        self.world().get_block(cell[0], cell[1], cell[2])
    }

    fn set(&mut self, cell: [i32; 3], b: BlockId) {
        self.world().set_block(cell[0], cell[1], cell[2], b);
    }

    /// A water source at `cell` in the world the server simulates.
    fn water_source(&mut self, cell: [i32; 3]) {
        self.set(cell, block::WATER);
        match self.host.as_mut() {
            Some(h) => h.water.add_source(cell[0], cell[1], cell[2]),
            None => self.hs.server.water.add_source(cell[0], cell[1], cell[2]),
        }
    }

    /// The tick a use sent now is processed on (a lending host advances its
    /// clock, then lends).
    fn next_tick(&self) -> u64 {
        match &self.host {
            Some(h) => h.clock.tick_counter + 1,
            None => self.hs.server.tick_counter,
        }
    }

    fn sp(&mut self) -> &mut crate::server::ServerPlayer {
        &mut self.hs.server.players[self.c.slot]
    }

    fn tally(&self) -> crate::joiner_inventory::PossessionTally {
        self.hs.server.players[self.c.slot].possession
    }

    /// Put `count` of `item` in `slot` on both sides (where the joiner starts).
    fn give(&mut self, slot: usize, item: Item, count: u8) {
        let stack = ItemStack { item, count };
        self.c.inv.set_slot(slot, Some(stack.clone()));
        self.sp().inventory.set_slot(slot, Some(stack));
    }

    /// One server tick; the client applies what changes its window (grants)
    /// and reports the events it applied on an empty input.
    fn tick(&mut self) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        while let Some(pkt) = self.c.transport.try_recv_from_server() {
            if let Some((protocol::PacketType::InventoryGrant, payload)) = protocol::deserialize_header(&pkt) {
                let grant: protocol::InventoryGrantPacket = protocol::safe_deserialize(payload).unwrap();
                let _ = crate::remote_entities::apply_inventory_grant(&mut self.c.inv, &grant, &self.hs.server.registry);
                self.c.events = self.c.events.max(grant.window_event);
            }
        }
        self.send_input(Vec::new(), Vec::new(), Vec::new());
    }

    /// One input: `edits`, each with its hand, and the use tags beside them.
    fn send_input(&mut self, edits: Vec<protocol::BlockChange>, hands: Vec<protocol::EditHand>, use_tags: Vec<UseTag>) {
        self.c.input_seq += 1;
        let sp = &self.hs.server.players[self.c.slot];
        let (held_kind, held_id) = self.hand_now().1;
        let input = protocol::InputPacket {
            tick: self.c.input_seq,
            x: sp.player.pos.x,
            y: sp.player.pos.y,
            z: sp.player.pos.z,
            yaw: sp.yaw,
            pitch: sp.pitch,
            health: 20.0,
            held_kind,
            held_id,
            hotbar_slot: Some(self.c.hot as u8),
            block_changes: edits,
            edit_hands: hands,
            use_tags,
            events_applied: self.c.events,
            ..Default::default()
        };
        self.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// The selected slot and the wire pair of what it holds now.
    fn hand_now(&self) -> (usize, (u8, u16)) {
        let pair = match self.c.inv.hotbar_slot(self.c.hot) {
            Some(s) => crate::inventory::item_to_ref(&s.item).to_wire(),
            None => protocol::ItemRef::Empty.to_wire(),
        };
        (self.c.hot, pair)
    }

    /// The client makes a use of `kind` at `cell`, leaving `new` (with
    /// `meta`) there: tag first, then its arm's inventory steps
    /// ([`client_steps`]), then the edit is sent with its tag and the hand
    /// after; one tick. Returns what didn't fit the client's bag (a joined
    /// client spills nothing of its own).
    fn use_at(&mut self, kind: UseKind, cell: [i32; 3], new: BlockId, meta: u8) -> Option<ItemStack> {
        let (tag, edit, hand, leftover) = self.make_use(kind, cell, new, meta);
        self.used_on = self.next_tick();
        self.send_input(vec![edit], vec![hand], vec![tag]);
        self.tick();
        leftover
    }

    /// [`Self::use_at`]'s client half alone: the tag, the edit, the edit's
    /// hand (after the use) and the leftover.
    fn make_use(
        &mut self,
        kind: UseKind,
        cell: [i32; 3],
        new: BlockId,
        meta: u8,
    ) -> (UseTag, protocol::BlockChange, protocol::EditHand, Option<ItemStack>) {
        let hot = self.c.hot;
        let held = self.c.inv.hotbar_slot(hot).map(|s| s.item.clone());
        let tag = crate::use_edits::tag(kind, cell, hot, held.as_ref());
        let old = self.block(cell);
        let leftover = client_steps(&mut self.c.inv, kind, hot, old);
        let (slot, (k, id)) = self.hand_now();
        let edit = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: new, meta };
        (tag, edit, (slot as u8, k, id), leftover)
    }

    /// The server's copy is the client's window, slot for slot.
    fn assert_lockstep(&self, what: &str) {
        let sp = &self.hs.server.players[self.c.slot];
        let slots = |inv: &Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
        assert_eq!(slots(&sp.inventory), slots(&self.c.inv), "{what}: the 36 slots");
        let client = window::digest_parts(&self.c.inv, &self.c.armour, &self.c.ui.cursor_item, &self.c.ui.grid, self.c.ui.station());
        let server = window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station);
        assert_eq!(server, client, "{what}: the digests");
    }

    /// A use mirrored cleanly: the edit is in the world, and it was counted
    /// as a use, never a placement.
    fn assert_mirrored(&mut self, what: &str, cell: [i32; 3], new: BlockId) {
        assert_eq!(self.block(cell), new, "{what}: the edit is applied");
        self.assert_lockstep(what);
        let t = self.tally();
        assert_eq!((t.use_mirrored, t.use_mismatch), (1, 0), "{what}: one use mirrored, no mismatch");
        assert_eq!((t.matched, t.mismatched), (0, 0), "{what}: never classified as a placement");
    }

    /// The ground items in the world the server simulates.
    fn ground_items(&self) -> Vec<ItemStack> {
        let ecs = match self.host.as_ref() {
            Some(h) => &h.ecs,
            None => &self.hs.server.ecs,
        };
        ecs.query::<&crate::entity::ItemEntity>().iter().map(|(_, it)| it.stack.clone()).collect()
    }
}

/// The client arm's own inventory steps for a use of `kind` from hotbar
/// slot `hot` on a cell holding `old` (as `game_loop.rs` runs them): what it
/// spends, wears and gets back, in that order. Returns what didn't fit.
fn client_steps(inv: &mut Inventory, kind: UseKind, hot: usize, old: BlockId) -> Option<ItemStack> {
    let held = inv.hotbar_slot(hot).map(|s| s.item.clone());
    let spend = |inv: &mut Inventory| match &held {
        Some(Item::Material(m)) => assert!(inv.consume_one_material(hot, *m), "the arm spends one"),
        other => panic!("{kind:?} with {other:?} in hand"),
    };
    match kind {
        UseKind::BucketFill => {
            spend(inv);
            let filled = crate::bucket::fill_result(&Item::Material(MaterialId::Bucket), old, true).expect("a fluid");
            inv.add_item(ItemStack::new_material(filled, 1))
        }
        UseKind::BucketEmpty => {
            spend(inv);
            inv.add_item(ItemStack::new_material(MaterialId::Bucket, 1))
        }
        UseKind::Sow | UseKind::PlantPapyrus | UseKind::GrowGrass | UseKind::GrowCrop | UseKind::Salt => {
            spend(inv);
            None
        }
        UseKind::Erase => {
            assert!(inv.use_hotbar_tool(hot).is_some(), "the eraser wears");
            // The arm's own `let _ =`: a full bag loses the sheet.
            let _ = inv.add_item(ItemStack::new_material(MaterialId::PapyrusSheet, 1));
            None
        }
        UseKind::TapRubber => inv.add_item(ItemStack::new_material(MaterialId::Rubber, 1)),
        UseKind::Till => {
            assert!(inv.use_hotbar_tool(hot).is_some(), "the hoe wears");
            None
        }
        UseKind::DoorUpper => None,
    }
}

fn mat(m: MaterialId) -> Item {
    Item::Material(m)
}

// ─── One test per use kind ─────────────────────────────────────────────

#[test]
fn a_joiners_bucket_fill_is_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("fill");
    rig.water_source(ABOVE);
    rig.give(0, mat(MaterialId::Bucket), 2);
    rig.give(1, Item::Block(block::STONE), 5);
    rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    rig.assert_mirrored("fill", ABOVE, block::AIR);
    assert_eq!(rig.c.inv.slot(2), Some(&ItemStack::new_material(MaterialId::WaterBucket, 1)), "the first empty slot");
}

#[test]
fn a_joiners_bucket_empty_is_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("empty");
    rig.give(0, mat(MaterialId::WaterBucket), 1);
    rig.give(4, mat(MaterialId::Bucket), 2);
    rig.use_at(UseKind::BucketEmpty, ABOVE, block::WATER, 0);
    rig.assert_mirrored("empty", ABOVE, block::WATER);
    assert_eq!(rig.c.inv.slot(4).map(|s| s.count), Some(3), "the empty bucket stacked onto the others");
    assert!(rig.hs.server.water.is_source(ABOVE[0], ABOVE[1], ABOVE[2]), "the server's water has a source there");
}

/// The last seed: the hand is empty once the use spent it, and the edit is
/// still a use, not an empty-handed placement.
#[test]
fn a_joiners_last_seed_is_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("sow");
    rig.set(FLOOR, block::TILLED_SOIL);
    rig.give(0, mat(MaterialId::WheatSeeds), 1);
    rig.use_at(UseKind::Sow, ABOVE, block::WHEAT_STAGE_0, 0);
    rig.assert_mirrored("sow", ABOVE, block::WHEAT_STAGE_0);
    assert!(rig.c.inv.slot(0).is_none());
}

#[test]
fn a_joiners_papyrus_planting_is_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("papyrus");
    rig.set(FLOOR, block::DIRT);
    rig.set([FLOOR[0] + 1, FLOOR[1], FLOOR[2]], block::WATER);
    rig.give(0, mat(MaterialId::PapyrusReed), 3);
    rig.use_at(UseKind::PlantPapyrus, ABOVE, block::PAPYRUS_STAGE_0, 0);
    rig.assert_mirrored("papyrus", ABOVE, block::PAPYRUS_STAGE_0);
}

/// The last bone meal on grass: before C3c-1 the empty hand made the tall
/// grass an empty-handed placement, a possession mismatch.
#[test]
fn a_joiners_last_bone_meal_on_grass_is_a_use_not_a_placement_mismatch() {
    let mut rig = Rig::dedicated("grass");
    rig.set(FLOOR, block::GRASS);
    rig.give(0, mat(MaterialId::Bonemeal), 1);
    rig.use_at(UseKind::GrowGrass, ABOVE, block::TALL_GRASS, 0);
    rig.assert_mirrored("bone meal on grass", ABOVE, block::TALL_GRASS);
    assert_eq!(rig.tally().unchecked, 0);
}

#[test]
fn a_joiners_crop_accelerators_are_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("crop");
    rig.set(FLOOR, block::TILLED_SOIL);
    rig.set(ABOVE, block::WHEAT_STAGE_0);
    rig.give(0, mat(MaterialId::Bonemeal), 4);
    rig.give(1, mat(MaterialId::Fertiliser), 2);
    // Bone meal two stages (one of its outcomes; not re-rolled).
    rig.use_at(UseKind::GrowCrop, ABOVE, block::WHEAT_STAGE_2, 0);
    rig.assert_mirrored("bone meal", ABOVE, block::WHEAT_STAGE_2);
    rig.c.hot = 1;
    rig.use_at(UseKind::GrowCrop, ABOVE, block::WHEAT_STAGE_3, 0);
    assert_eq!(rig.block(ABOVE), block::WHEAT_STAGE_3);
    rig.assert_lockstep("fertiliser");
    assert_eq!((rig.tally().use_mirrored, rig.tally().use_mismatch), (2, 0));
}

#[test]
fn a_joiners_salt_is_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("salt");
    rig.set(FLOOR, block::GRASS);
    rig.give(0, mat(MaterialId::Salt), 2);
    rig.use_at(UseKind::Salt, FLOOR, block::SALT_PATH, 0);
    rig.assert_mirrored("salt", FLOOR, block::SALT_PATH);
}

#[test]
fn a_joiners_eraser_is_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("erase");
    rig.set(ABOVE, block::BLUEPRINT_PAPER);
    rig.give(0, Item::Tool(Tool::new(ToolType::Eraser, ToolMaterial::Wood)), 1);
    rig.use_at(UseKind::Erase, ABOVE, block::AIR, 0);
    rig.assert_mirrored("erase", ABOVE, block::AIR);
    assert_eq!(rig.c.inv.slot(1), Some(&ItemStack::new_material(MaterialId::PapyrusSheet, 1)));
    assert_eq!(rig.tally().wear_mismatch, 0);
}

/// The tap stamps the SERVER's cooldown with the server's own tick (a
/// dedicated server and a lending host's world alike).
#[test]
fn a_joiners_rubber_tap_is_mirrored_and_the_server_stamps_its_own_cooldown() {
    for lent in [false, true] {
        let mut rig = if lent { Rig::lent("tap") } else { Rig::dedicated("tap") };
        rig.set(ABOVE, block::RUBBER_LOG);
        rig.give(0, mat(MaterialId::Bucket), 1);
        rig.use_at(UseKind::TapRubber, ABOVE, block::RUBBER_LOG_TAPPED, 0);
        rig.assert_mirrored("tap", ABOVE, block::RUBBER_LOG_TAPPED);
        assert_eq!(rig.c.inv.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 1)), "the bucket is kept");
        let stamped = rig.world().tapped_rubber_logs.get(&(ABOVE[0], ABOVE[1], ABOVE[2])).copied();
        assert_eq!(stamped, Some(rig.used_on), "the server's own tick (lent: {lent})");
        // A second tap while the server holds it on cooldown: a mismatch,
        // applied all the same, the stamp kept.
        rig.set(ABOVE, block::RUBBER_LOG);
        rig.use_at(UseKind::TapRubber, ABOVE, block::RUBBER_LOG_TAPPED, 0);
        assert_eq!(rig.tally().use_mismatch, 1, "on the server's cooldown (lent: {lent})");
        assert_eq!(rig.world().tapped_rubber_logs.get(&(ABOVE[0], ABOVE[1], ABOVE[2])).copied(), stamped);
    }
}

#[test]
fn a_joiners_hoe_is_mirrored_on_the_servers_copy() {
    let mut rig = Rig::dedicated("till");
    rig.set(FLOOR, block::DIRT);
    rig.give(0, Item::Tool(Tool::new(ToolType::Hoe, ToolMaterial::Stone)), 1);
    rig.use_at(UseKind::Till, FLOOR, block::TILLED_SOIL, 0);
    rig.assert_mirrored("till", FLOOR, block::TILLED_SOIL);
    assert_eq!(rig.tally().wear_mismatch, 0);
}

/// A door's two halves in one input: the bottom a plain placement (it pays
/// for the door), the top a use; the server and another joiner see a whole
/// door.
#[test]
fn a_joiners_door_reaches_the_server_and_another_joiner_whole() {
    let mut rig = Rig::dedicated("door");
    let (other, other_slot) = join_guest(&mut rig.hs, "Neighbour");
    // The neighbour holds the door's column (a joiner hears only of chunks
    // it holds, B2a).
    let cs = crate::chunk::CHUNK_SIZE as i32;
    rig.hs.hold_column_for_test(other_slot, (ABOVE[0].div_euclid(cs), ABOVE[2].div_euclid(cs)));
    let _ = super::joiner_authority::block_changes_seen(&other);
    rig.give(0, Item::Block(block::OAK_DOOR), 2);
    let top = [ABOVE[0], ABOVE[1] + 1, ABOVE[2]];
    // The place arm: one door taken, the bottom half, then the top half.
    assert_eq!(rig.c.inv.take_placeable_from_hotbar(0), Some(block::OAK_DOOR));
    let (slot, (k, id)) = rig.hand_now();
    let bottom = protocol::BlockChange { x: ABOVE[0], y: ABOVE[1], z: ABOVE[2], new_block: block::OAK_DOOR, meta: 0 };
    let top_meta = crate::use_edits::door_top_meta(0);
    let tag = crate::use_edits::tag(UseKind::DoorUpper, top, 0, None);
    let upper = protocol::BlockChange { x: top[0], y: top[1], z: top[2], new_block: block::OAK_DOOR, meta: top_meta };
    let hand = (slot as u8, k, id);
    rig.send_input(vec![bottom, upper.clone()], vec![hand, hand], vec![tag]);
    rig.tick();
    assert_eq!(rig.block(ABOVE), block::OAK_DOOR);
    assert_eq!(rig.block(top), block::OAK_DOOR);
    assert!(crate::block_shape::door_is_top(rig.world().meta_at(top[0], top[1], top[2])), "the top half's meta");
    rig.assert_lockstep("door");
    let t = rig.tally();
    assert_eq!((t.matched, t.mismatched), (1, 0), "the bottom half paid for the door");
    assert_eq!((t.use_mirrored, t.use_mismatch), (1, 0), "the top half is the door's use");
    let seen = super::joiner_authority::block_changes_seen(&other);
    assert!(seen.iter().any(|b| (b.x, b.y, b.z, b.new_block) == (ABOVE[0], ABOVE[1], ABOVE[2], block::OAK_DOOR)));
    assert!(seen.contains(&upper), "the other joiner sees the top half, meta and all");
}

// ─── Mismatches (log-only) ─────────────────────────────────────────────

/// Bone meal claimed to jump three stages: no outcome of its rule. Tallied,
/// applied all the same; the bone meal the client spent is taken, nothing
/// is made.
#[test]
fn an_illegal_bone_meal_outcome_is_tallied_and_still_applied() {
    let mut rig = Rig::dedicated("illegal-meal");
    rig.set(FLOOR, block::TILLED_SOIL);
    rig.set(ABOVE, block::WHEAT_STAGE_0);
    rig.give(0, mat(MaterialId::Bonemeal), 2);
    rig.use_at(UseKind::GrowCrop, ABOVE, block::WHEAT_STAGE_3, 0);
    assert_eq!(rig.block(ABOVE), block::WHEAT_STAGE_3, "log-only: applied");
    let t = rig.tally();
    assert_eq!((t.use_mirrored, t.use_mismatch), (0, 1));
    rig.assert_lockstep("the spend is mirrored");
}

/// Water poured from a claimed EMPTY bucket: no outcome of emptying.
#[test]
fn water_poured_from_an_empty_bucket_is_tallied_and_still_applied() {
    let mut rig = Rig::dedicated("illegal-pour");
    rig.give(0, mat(MaterialId::Bucket), 3);
    let tag = crate::use_edits::tag(UseKind::BucketEmpty, ABOVE, 0, Some(&mat(MaterialId::Bucket)));
    let pour = protocol::BlockChange { x: ABOVE[0], y: ABOVE[1], z: ABOVE[2], new_block: block::WATER, meta: 0 };
    let hand = rig.hand_now();
    rig.send_input(vec![pour], vec![(0, hand.1 .0, hand.1 .1)], vec![tag]);
    rig.tick();
    assert_eq!(rig.block(ABOVE), block::WATER, "log-only: applied");
    let t = rig.tally();
    assert_eq!((t.use_mirrored, t.use_mismatch), (0, 1));
    assert_eq!(rig.sp().inventory.slot(0).map(|s| s.count), Some(2), "the claimed bucket is spent, no bucket made");
}

/// A use whose cost the copy can't pay makes nothing (no item from nothing).
#[test]
fn a_use_the_copy_cant_pay_for_makes_nothing() {
    let mut rig = Rig::dedicated("unpaid");
    rig.water_source(ABOVE);
    rig.c.inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
    rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    assert_eq!(rig.block(ABOVE), block::AIR);
    assert_eq!(rig.tally().use_mismatch, 1);
    assert!(rig.sp().inventory.slots_iter().all(|s| s.is_none()), "no water bucket from nothing");
    assert!(rig.ground_items().is_empty(), "and nothing spilled");
}

// ─── Overflow ─────────────────────────────────────────────────────────

/// A full joiner fills a bucket: the client keeps no ground item of its own
/// (`GameState::spill_use_leftover`); the server spills its copy's overflow,
/// one real water bucket at the joiner's feet. In lockstep, the same unit:
/// the world's buckets are conserved.
#[test]
fn a_full_joiners_fill_spills_one_real_bucket_from_the_servers_copy() {
    let mut rig = Rig::dedicated("overflow");
    rig.water_source(ABOVE);
    rig.give(0, mat(MaterialId::Bucket), 2);
    for k in 1..36 {
        rig.give(k, Item::Block(block::STONE), 64);
    }
    let leftover = rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    assert_eq!(leftover, Some(ItemStack::new_material(MaterialId::WaterBucket, 1)), "the client's bag was full");
    rig.assert_mirrored("overflow", ABOVE, block::AIR);
    let ground = rig.ground_items();
    assert_eq!(ground, vec![ItemStack::new_material(MaterialId::WaterBucket, 1)], "one real water bucket spilled");
    let buckets = |inv: &Inventory| {
        inv.slots_iter()
            .flatten()
            .filter(|s| matches!(s.item, Item::Material(MaterialId::Bucket | MaterialId::WaterBucket)))
            .map(|s| u32::from(s.count))
            .sum::<u32>()
    };
    assert_eq!(buckets(&rig.c.inv) + ground.len() as u32, 2, "the world's buckets are conserved");
}

// ─── Order ────────────────────────────────────────────────────────────

/// A window op the client made BETWEEN two uses lands between them on the
/// server too (the client's send cuts its edits at the op:
/// `RemoteClient::note_order_cut`): a use's product lands on the copy when
/// its edit is processed, so the order decides where.
#[test]
fn a_window_op_between_two_uses_lands_between_them_on_the_server() {
    let mut rig = Rig::dedicated("op-between");
    rig.set(ABOVE, block::RUBBER_LOG);
    let pond = [ABOVE[0], ABOVE[1], ABOVE[2] + 1];
    rig.water_source(pond);
    rig.give(0, mat(MaterialId::Bucket), 2);
    rig.give(1, Item::Block(block::STONE), 1);
    let before = rig.c.inv.clone();
    // Use 1: a tap (Rubber to the first empty slot, 2).
    rig.use_at(UseKind::TapRubber, ABOVE, block::RUBBER_LOG_TAPPED, 0);
    // The op: the stone from slot 1 to slot 20 (slot 1 is now the first empty).
    rig.c.ui.open_player_crafting(&rig.c.inv, &rig.c.armour);
    for click in [WindowClick::Slot { slot: 1, right: false }, WindowClick::Slot { slot: 20, right: false }] {
        let c = &mut rig.c;
        let r = c.ui.apply_click(&mut c.inv, &mut c.armour, &click, false, Vec3::ZERO, |_| block::AIR);
        assert!(r.ok());
    }
    assert!(rig.c.ui.cursor_item.is_none(), "the stone is down");
    let c = &mut rig.c;
    for logged in c.ui.take_ops(&c.inv, &c.armour) {
        c.seq += 1;
        let pkt = logged.packet(c.seq, c.events);
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
    }
    // Use 2: a fill (the water bucket to the first empty slot: 1, after the op).
    rig.use_at(UseKind::BucketFill, pond, block::AIR, 0);
    assert_eq!(rig.c.inv.slot(1), Some(&ItemStack::new_material(MaterialId::WaterBucket, 1)));
    rig.assert_lockstep("use, op, use");
    let t = rig.tally();
    assert_eq!((t.use_mirrored, t.use_mismatch, t.window_mismatch), (2, 0, 0));
    // The order matters: both uses first, then the op, would have put the
    // water bucket in slot 3.
    let mut wrong = before;
    client_steps(&mut wrong, UseKind::TapRubber, 0, block::RUBBER_LOG);
    client_steps(&mut wrong, UseKind::BucketFill, 0, block::WATER);
    assert_eq!(wrong.slot(3), Some(&ItemStack::new_material(MaterialId::WaterBucket, 1)));
    assert_ne!(wrong.slot(1), rig.c.inv.slot(1));
}

/// Through the REAL send path (`RemoteClient`): two uses of one cell in one
/// send (a bucket emptied, then filled again) — the second waits for the
/// next input with its tag, and both mirror; then a use, a window op and a
/// use in one send — the second use waits behind the op, which reaches the
/// server between them, as the client made them.
#[test]
fn through_the_real_send_path_uses_keep_their_tags_and_their_order() {
    let mut hs = HostedServer::start(
        0,
        format!("use-edits-send-path-{}", std::process::id()),
        42,
        0,
        RemoteTransport::WebSocket { port: 0 },
    )
    .expect("dedicated server starts");
    hs.server.difficulty = crate::survival::Difficulty::Peaceful;
    let transport = hs.attach_test_remote();
    let mut rc = crate::remote_client::RemoteClient::from_transport(
        Box::new(transport),
        crate::remote_client::build_join_request_guest("Plumber", 0),
        None,
    );
    for _ in 0..5 {
        hs.tick();
        rc.poll();
    }
    assert!(rc.is_connected());
    let slot = rc.player_index().expect("joined") as usize;
    hs.server.column_streamer = None;
    hs.server.column_refill_per_tick = 0;
    let mut world = std::mem::replace(&mut hs.server.world, crate::world::World::new());
    floor_and_stand(&mut world, &mut hs, slot);
    hs.server.world = world;
    let cs = crate::chunk::CHUNK_SIZE as i32;
    hs.server.loaded_columns.insert((ABOVE[0].div_euclid(cs), ABOVE[2].div_euclid(cs)));
    // The client's window and the server's copy: one water bucket, two empty.
    let mut inv = Inventory::new();
    inv.set_slot(0, Some(ItemStack::new_material(MaterialId::WaterBucket, 1)));
    inv.set_slot(5, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
    hs.server.players[slot].inventory = inv.clone();
    hs.server.players[slot].possession = Default::default();
    // One send: the empty, then (the same cell) the fill.
    let mut pending = crate::window_ops::PendingEdits::default();
    let pour_tag = crate::use_edits::tag(UseKind::BucketEmpty, ABOVE, 0, inv.hotbar_slot(0).map(|s| &s.item));
    client_steps(&mut inv, UseKind::BucketEmpty, 0, block::AIR);
    pending.push_use(protocol::BlockChange { x: ABOVE[0], y: ABOVE[1], z: ABOVE[2], new_block: block::WATER, meta: 0 }, pour_tag);
    // The empty bucket came back to the stack in slot 5; select it.
    let fill_tag = crate::use_edits::tag(UseKind::BucketFill, ABOVE, 5, inv.slot(5).map(|s| &s.item));
    // (Spent from slot 5 by the arm's own rule: the hotbar is 0..9.)
    assert!(inv.consume_one_material(5, MaterialId::Bucket));
    assert!(inv.add_item(ItemStack::new_material(MaterialId::WaterBucket, 1)).is_none());
    pending.push_use(protocol::BlockChange { x: ABOVE[0], y: ABOVE[1], z: ABOVE[2], new_block: block::AIR, meta: 0 }, fill_tag);
    let send = |rc: &mut crate::remote_client::RemoteClient, pending: &mut crate::window_ops::PendingEdits, cut: Option<u64>| {
        let (edits, hands, stamps, uses) = pending.take((0, 0, 0));
        rc.note_edit_stamps(stamps);
        rc.note_edit_uses(uses);
        rc.note_order_cut(cut);
        rc.send_input(&protocol::InputPacket { block_changes: edits, edit_hands: hands, health: 20.0, ..Default::default() });
    };
    send(&mut rc, &mut pending, None);
    assert!(rc.has_carry_over(), "the second use of the cell waits");
    hs.tick();
    let t = hs.server.players[slot].possession;
    assert_eq!((t.use_mirrored, t.use_mismatch), (1, 0), "the empty, alone in its input");
    assert_eq!(hs.server.world.get_block(ABOVE[0], ABOVE[1], ABOVE[2]), block::WATER);
    send(&mut rc, &mut pending, None);
    hs.tick();
    let sp = &hs.server.players[slot];
    assert_eq!((sp.possession.use_mirrored, sp.possession.use_mismatch), (2, 0), "then the fill, with its own tag");
    assert_eq!(hs.server.world.get_block(ABOVE[0], ABOVE[1], ABOVE[2]), block::AIR);
    let slots = |inv: &Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
    assert_eq!(slots(&sp.inventory), slots(&inv), "the copy is the client's window");

    // A tap, then an op (the stone from slot 1 to slot 20), then an Eraser,
    // all in one send, as `GameState::network_send_input` sends them.
    let log = [ABOVE[0], ABOVE[1], ABOVE[2] - 1];
    let paper = [ABOVE[0], ABOVE[1] + 1, ABOVE[2] - 1];
    hs.server.world.set_block(log[0], log[1], log[2], block::RUBBER_LOG);
    hs.server.world.set_block(paper[0], paper[1], paper[2], block::BLUEPRINT_PAPER);
    let eraser = Item::Tool(Tool::new(ToolType::Eraser, ToolMaterial::Wood));
    for inv in [&mut inv, &mut hs.server.players[slot].inventory] {
        inv.set_slot(1, Some(ItemStack::new_block(block::STONE, 1)));
        inv.set_slot(6, Some(ItemStack { item: eraser.clone(), count: 1 }));
    }
    let before = inv.clone();
    let mut ui = CraftingUi::new();
    let mut armour: [Option<ArmourItem>; 4] = [None; 4];
    let tap_tag = crate::use_edits::tag(UseKind::TapRubber, log, 5, inv.slot(5).map(|s| &s.item));
    assert_eq!(client_steps(&mut inv, UseKind::TapRubber, 5, block::RUBBER_LOG), None);
    pending.push_use(protocol::BlockChange { x: log[0], y: log[1], z: log[2], new_block: block::RUBBER_LOG_TAPPED, meta: 0 }, tap_tag);
    ui.open_player_crafting(&inv, &armour);
    for click in [WindowClick::Slot { slot: 1, right: false }, WindowClick::Slot { slot: 20, right: false }] {
        assert!(ui.apply_click(&mut inv, &mut armour, &click, false, Vec3::ZERO, |_| block::AIR).ok());
    }
    let erase_tag = crate::use_edits::tag(UseKind::Erase, paper, 6, inv.slot(6).map(|s| &s.item));
    assert_eq!(client_steps(&mut inv, UseKind::Erase, 6, block::BLUEPRINT_PAPER), None);
    pending.push_use(protocol::BlockChange { x: paper[0], y: paper[1], z: paper[2], new_block: block::AIR, meta: 0 }, erase_tag);
    assert_eq!(inv.slot(1), Some(&ItemStack::new_material(MaterialId::PapyrusSheet, 1)), "after the op, slot 1 was the first empty");
    // `flush_ops_before_edits`: nothing was logged before the tap.
    assert!(ui.ops.take_before(pending.first_stamp()).is_empty());
    // The input, cut at the first op still waiting…
    let cut = ui.ops.first_stamp();
    send(&mut rc, &mut pending, cut);
    assert!(rc.has_carry_over(), "the Eraser, made after the op, waits");
    // …then `flush_window_ops`: the ops made before the edit that waits.
    for logged in ui.ops.take_before(rc.first_carried_stamp()) {
        rc.send_window_op(logged);
    }
    hs.tick();
    send(&mut rc, &mut pending, ui.ops.first_stamp());
    hs.tick();
    let sp = &hs.server.players[slot];
    assert_eq!(slots(&sp.inventory), slots(&inv), "use, op, use: the copy is the client's window");
    assert_eq!((sp.possession.use_mirrored, sp.possession.use_mismatch), (4, 0));
    assert_eq!(sp.possession.window_mismatch, 0);
    // In the other order (both uses, then the op) the sheet lands in slot 3.
    let mut wrong = before;
    client_steps(&mut wrong, UseKind::TapRubber, 5, block::RUBBER_LOG);
    client_steps(&mut wrong, UseKind::Erase, 6, block::BLUEPRINT_PAPER);
    assert_eq!(wrong.slot(3), Some(&ItemStack::new_material(MaterialId::PapyrusSheet, 1)));
}
