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
//!
//! C3c-3a (v83) — a joiner's Plans, tracked by marker: the hung
//! print (a use edit, `UseKind::HangPrint`) and the reported mints
//! (`ItemAction::PlanMinted`) land on and leave the server's copy as marker
//! placeholders, which then move under window ops in lockstep. The joined
//! client's own arms are the GPU harness's to drive.

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
    /// C3c-1-fix — every block change received, in order.
    changes: Vec<protocol::BlockChange>,
    /// C3c-1-fix (M-4) — every refusal notice received, in order.
    refused: Vec<protocol::RefusedUse>,
    /// C3c-1-fix (M-4) — the client's own records of its uses
    /// (`use_edits::SentUses`), as the game loop keeps them, and how many
    /// refusals found one and were undone.
    uses: crate::use_edits::SentUses,
    undone: u32,
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
            changes: Vec::new(),
            refused: Vec::new(),
            uses: Default::default(),
            undone: 0,
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
    /// and reports the events it applied on an empty input. C3c-1-fix — a
    /// StateUpdate's block changes and refusals are kept, and each refusal
    /// is undone from the client's own record before the update's
    /// acknowledgement lets records go (`GameState::network_receive`).
    fn tick(&mut self) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        while let Some(pkt) = self.c.transport.try_recv_from_server() {
            match protocol::deserialize_header(&pkt) {
                Some((protocol::PacketType::InventoryGrant, payload)) => {
                    let grant: protocol::InventoryGrantPacket = protocol::safe_deserialize(payload).unwrap();
                    let _ = crate::remote_entities::apply_inventory_grant(&mut self.c.inv, &grant, &self.hs.server.registry);
                    self.c.events = self.c.events.max(grant.window_event);
                }
                Some((protocol::PacketType::StateUpdate, payload)) => {
                    let state: protocol::StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                    self.c.changes.extend(state.block_changes);
                    // C3c-2-fix (F-M1) — the game loop's own undo: newest first.
                    let c = &mut self.c;
                    let undone = crate::use_edits::undo_refused(&mut c.uses, &mut c.inv, &mut c.ui, &state.refused_uses);
                    c.undone += undone.iter().filter(|(_, u)| u.is_some()).count() as u32;
                    self.c.refused.extend(state.refused_uses);
                    self.c.uses.acknowledged(state.last_acked_input);
                }
                _ => {}
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
        let mut tag = crate::use_edits::tag(kind, cell, hot, held.as_ref());
        let old = self.block(cell);
        let leftover = client_steps(&mut self.c.inv, kind, hot, old);
        // C3c-1-fix (M-2) — the arm says what didn't fit (`push_use_edit`),
        // and (M-4) keeps its own record of the use.
        tag.unfit = leftover.as_ref().map_or(0, |l| l.count);
        let landed = product_of(kind, old).and_then(|p| {
            let n = p.count - tag.unfit;
            (n > 0).then_some(ItemStack { item: p.item, count: n })
        });
        let cost = kind.consumes().then(|| held.clone()).flatten();
        self.c.uses.record(crate::use_edits::UseRecord {
            cell,
            kind: tag.kind,
            slot: hot,
            cost,
            landed,
            made_at: self.c.input_seq + 1,
        });
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
            // A full bag loses the sheet (single-player); joined, the tag
            // says it didn't fit (C3c-1-fix).
            inv.add_item(ItemStack::new_material(MaterialId::PapyrusSheet, 1))
        }
        UseKind::TapRubber => inv.add_item(ItemStack::new_material(MaterialId::Rubber, 1)),
        UseKind::Till => {
            assert!(inv.use_hotbar_tool(hot).is_some(), "the hoe wears");
            None
        }
        UseKind::DoorUpper => None,
        // C3c-3a — the hang arm's `take_one_from_hotbar`: the Plan is spent.
        UseKind::HangPrint => {
            assert!(inv.take_one_from_hotbar(hot).is_some(), "the hang spends the Plan");
            None
        }
    }
}

/// What a use of `kind` on a cell holding `old` gives back, by the arm.
fn product_of(kind: UseKind, old: BlockId) -> Option<ItemStack> {
    let m = match kind {
        UseKind::BucketFill => crate::bucket::fill_result(&Item::Material(MaterialId::Bucket), old, true)?,
        UseKind::BucketEmpty => MaterialId::Bucket,
        UseKind::Erase => MaterialId::PapyrusSheet,
        UseKind::TapRubber => MaterialId::Rubber,
        _ => return None,
    };
    Some(ItemStack::new_material(m, 1))
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
        // applied all the same, the stamp kept. C3c-1-fix (L-1) — honest
        // drift: the copy tracks the client (its rubber mirrored too).
        rig.set(ABOVE, block::RUBBER_LOG);
        rig.use_at(UseKind::TapRubber, ABOVE, block::RUBBER_LOG_TAPPED, 0);
        assert_eq!(rig.tally().use_mismatch, 1, "on the server's cooldown (lent: {lent})");
        assert_eq!(rig.world().tapped_rubber_logs.get(&(ABOVE[0], ABOVE[1], ABOVE[2])).copied(), stamped);
        rig.assert_lockstep("drift tracks the client");
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
/// (`GameState::spill_use_leftover`); its tag says one didn't fit (C3c-1-fix:
/// `UseTag::unfit`), and the server, whose full copy agrees, spawns exactly
/// that, one real water bucket at the joiner's feet: the world's buckets are
/// conserved.
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

// ─── C3c-1-fix ────────────────────────────────────────────────────────────

/// Units of `item` in `inv`.
fn units(inv: &Inventory, item: &Item) -> u32 {
    inv.slots_iter().flatten().filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum()
}

/// Fill every slot from `from` on with stone, on the client only.
fn fill_client_bag(rig: &mut Rig, from: usize) {
    for k in from..36 {
        rig.c.inv.set_slot(k, Some(ItemStack::new_block(block::STONE, 64)));
    }
}

/// M-2 scenario A — a joiner arrives with a full bag (five buckets among it)
/// and its server copy is EMPTY (the copy starts empty at attach). It fills a
/// bucket from water: its bag can't take the water bucket, so its tag says
/// one didn't fit, and the server spawns exactly that, one real water bucket
/// at the joiner, though its copy could neither pay nor corroborate it
/// (believed, within the joiner's bound). The world's buckets are conserved:
/// before C3c-1-fix the server spawned only its copy's overflow, none, and
/// one bucket was lost for good.
#[test]
fn a_full_bag_joiner_whose_copy_is_empty_gets_its_unfit_bucket_as_a_real_item() {
    let mut rig = Rig::dedicated("unfit-empty-copy");
    rig.water_source(ABOVE);
    rig.c.inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 5)));
    fill_client_bag(&mut rig, 1);
    assert!(rig.sp().inventory.slots_iter().all(|s| s.is_none()), "the copy is empty");
    let leftover = rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    assert_eq!(leftover, Some(ItemStack::new_material(MaterialId::WaterBucket, 1)));
    assert_eq!(rig.block(ABOVE), block::AIR);
    assert_eq!(rig.ground_items(), vec![ItemStack::new_material(MaterialId::WaterBucket, 1)], "one real water bucket");
    let held = units(&rig.c.inv, &mat(MaterialId::Bucket)) + units(&rig.c.inv, &mat(MaterialId::WaterBucket));
    assert_eq!(held + 1, 5, "the world's buckets are conserved");
    assert_eq!(rig.tally().use_unfit_believed, 1, "believed: the copy couldn't back it");
    assert!(rig.sp().inventory.slots_iter().all(|s| s.is_none()), "and nothing lands on the copy");
}

/// M-2 scenario B — the copy is FULLER than the client (a spend not
/// mirrored left stacks the client emptied): the client's bag takes the
/// product, so its tag says nothing didn't fit, and the server spawns
/// nothing, though its own copy has no room. Before C3c-1-fix it spilled its
/// copy's overflow: a real item the client also kept.
#[test]
fn a_copy_fuller_than_the_client_spawns_nothing_extra() {
    let mut rig = Rig::dedicated("unfit-fuller-copy");
    rig.water_source(ABOVE);
    rig.give(0, mat(MaterialId::Bucket), 2);
    for k in 1..36 {
        rig.sp().inventory.set_slot(k, Some(ItemStack::new_block(block::STONE, 64)));
    }
    let leftover = rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    assert_eq!(leftover, None, "the client's bag took it");
    assert!(rig.ground_items().is_empty(), "nothing spawned");
    assert_eq!(rig.c.inv.slot(1), Some(&ItemStack::new_material(MaterialId::WaterBucket, 1)));
    let t = rig.tally();
    assert_eq!((t.use_copy_overflow, t.use_unfit_believed), (1, 0), "the copy's overflow is only counted");
}

/// M-2 — a modified client claims one unfit water bucket on every fill, with
/// no bucket in its server copy: each spawn is believed, charged to the
/// joiner's bound (64 deep, 4 a second), and past it nothing is spawned.
#[test]
fn a_modified_clients_unfit_claims_stop_at_the_believed_bound() {
    let mut rig = Rig::dedicated("unfit-bound");
    let bucket = mat(MaterialId::Bucket);
    let start = rig.hs.server.tick_counter;
    let fills = 90;
    let at = rig.sp().player.pos;
    for _ in 0..fills {
        // The water spreads and would push the body out of reach: hold it.
        rig.sp().player.pos = at;
        rig.sp().player.velocity = Vec3::ZERO;
        rig.water_source(ABOVE);
        let mut tag = crate::use_edits::tag(UseKind::BucketFill, ABOVE, 0, Some(&bucket));
        tag.unfit = 1;
        let edit = protocol::BlockChange { x: ABOVE[0], y: ABOVE[1], z: ABOVE[2], new_block: block::AIR, meta: 0 };
        let (k, id) = crate::inventory::item_to_ref(&bucket).to_wire();
        rig.send_input(vec![edit], vec![(0, k, id)], vec![tag]);
        rig.tick();
    }
    let secs = (rig.hs.server.tick_counter - start).div_ceil(20) as usize;
    let t = rig.tally();
    let spawned = t.use_unfit_believed as usize;
    assert!(spawned >= 64, "the bound's depth is believed: {spawned} ({t:?})");
    assert!(spawned <= 64 + 4 * secs + 1, "no more than 64 + 4/s ({secs} s): {spawned}");
    assert_eq!(t.use_unfit_refused as usize, fills - spawned, "past the bound, nothing spawned");
    assert!(t.use_unfit_refused > 0);
    // What was spawned is on the ground or (past its hold) picked up: never
    // more than the believed units.
    let water = mat(MaterialId::WaterBucket);
    let ground = rig.ground_items().iter().filter(|s| s.item == water).map(|s| s.count as usize).sum::<usize>();
    assert_eq!(ground + units(&rig.c.inv, &water) as usize, spawned);
}

/// A plot of someone else's (the host's seat 1) over `cell`'s column, in the
/// world the server simulates.
fn foreign_plot(rig: &mut Rig, cell: [i32; 3]) {
    let plot = crate::plot::PlotData::from_marker(crate::plot::PlotOwner::LocalPlayer(1), cell[0], cell[1] - 3, cell[2]);
    rig.world().plots.push(plot);
}

/// M-4 — a rubber tap in someone else's plot: refused, the log sent back,
/// and the joiner TOLD (`refused_uses`: the cell, the tap's kind, "you can't
/// use that here"). It undoes the tap from its own record — no rubber, its
/// bucket kept — and the server's copy is untouched, nothing spawned. Held
/// down, it repeats and is undone each time: no rubber from nothing.
#[test]
fn a_tap_in_a_foreign_plot_is_refused_told_and_undone() {
    let mut rig = Rig::dedicated("refused-tap");
    rig.set(ABOVE, block::RUBBER_LOG);
    foreign_plot(&mut rig, ABOVE);
    rig.give(0, mat(MaterialId::Bucket), 1);
    let before = rig.sp().inventory.clone();
    for _ in 0..5 {
        // The client sees its log restored by the send-back before each tap.
        let log = protocol::BlockChange { x: ABOVE[0], y: ABOVE[1], z: ABOVE[2], new_block: block::RUBBER_LOG, meta: 0 };
        assert!(rig.c.changes.is_empty() || rig.c.changes.last() == Some(&log), "the log is sent back");
        rig.use_at(UseKind::TapRubber, ABOVE, block::RUBBER_LOG_TAPPED, 0);
        rig.tick();
    }
    assert_eq!(rig.block(ABOVE), block::RUBBER_LOG, "never tapped on the server");
    assert_eq!(rig.c.refused.len(), 5);
    let want = protocol::RefusedUse {
        x: ABOVE[0],
        y: ABOVE[1],
        z: ABOVE[2],
        kind: UseKind::TapRubber.to_wire(),
        note: crate::item_actions::ItemNote::NotHere.to_wire(),
    };
    assert!(rig.c.refused.iter().all(|r| *r == want), "{:?}", rig.c.refused);
    assert_eq!(rig.c.undone, 5, "each undone from the client's own record");
    assert_eq!(units(&rig.c.inv, &mat(MaterialId::Rubber)), 0, "no rubber");
    assert_eq!(rig.c.inv.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 1)), "its bucket kept");
    rig.assert_lockstep("the copy never moved, the client is back to it");
    assert_eq!(rig.sp().inventory.slot(0), before.slot(0));
    assert!(rig.ground_items().is_empty());
    let t = rig.tally();
    assert_eq!((t.use_edit_refused, t.use_mirrored, t.use_mismatch), (5, 0, 0));
}

/// M-4 — a bucket fill of a protected lava source (a foreign plot), from a
/// full bag (its tag says the lava bucket didn't fit): refused, told, the
/// source sent back; nothing spawned for the unfit part, the copy untouched;
/// the client gets its bucket back.
#[test]
fn a_fill_of_a_protected_lava_source_is_refused_and_undone() {
    let mut rig = Rig::dedicated("refused-lava");
    rig.set(ABOVE, block::LAVA);
    rig.hs.server.lava.add_source(ABOVE[0], ABOVE[1], ABOVE[2]);
    foreign_plot(&mut rig, ABOVE);
    rig.give(0, mat(MaterialId::Bucket), 1);
    for k in 1..36 {
        rig.give(k, Item::Block(block::STONE), 64);
    }
    let leftover = rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    assert_eq!(leftover, None, "the last bucket's slot took the lava bucket");
    rig.tick();
    assert_eq!(rig.block(ABOVE), block::LAVA, "the source stands");
    assert!(rig.c.changes.iter().any(|b| (b.x, b.y, b.z, b.new_block) == (ABOVE[0], ABOVE[1], ABOVE[2], block::LAVA)), "sent back");
    assert_eq!(rig.c.refused.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![UseKind::BucketFill.to_wire()]);
    assert_eq!(rig.c.inv.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 1)), "the bucket back, the lava bucket gone");
    assert_eq!(units(&rig.c.inv, &mat(MaterialId::LavaBucket)), 0);
    rig.assert_lockstep("undone");
    assert!(rig.ground_items().is_empty());
    // The same from a bag with no room at all: unfit 1, nothing spawned.
    rig.c.inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
    rig.sp().inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
    let leftover = rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    assert!(leftover.is_some());
    rig.tick();
    assert!(rig.ground_items().is_empty(), "a refused use's unfit is never spawned");
    assert_eq!(rig.c.inv.slot(0), Some(&ItemStack::new_material(MaterialId::Bucket, 2)));
    rig.assert_lockstep("undone again");
    assert_eq!(rig.tally().use_unfit_believed, 0);
}

/// M-4 ordering — five uses in one input, past the server's per-tick edit
/// budget (four): the fifth, in a foreign plot, is processed (and refused) a
/// tick after the input's acknowledgement can arrive. The client's record of
/// it is held past that acknowledgement (`USE_RECORD_HOLD_INPUTS`), so the
/// undo still has it.
#[test]
fn a_refused_use_deferred_past_the_edit_budget_is_still_undone() {
    let mut rig = Rig::dedicated("refused-deferred");
    let logs: Vec<[i32; 3]> = (0..5).map(|k| [ABOVE[0] - 2 + k, ABOVE[1], ABOVE[2] - 1]).collect();
    for &cell in &logs {
        rig.set(cell, block::RUBBER_LOG);
    }
    foreign_plot(&mut rig, logs[4]);
    // The plot covers only the fifth log's column? Then the others are in
    // it too unless it is small: check by the server's own rule.
    let refusable: Vec<bool> = logs
        .iter()
        .map(|c| crate::plot::is_in_foreign_plot_for_npub(&rig.world().plots, c[0], c[2], None))
        .collect();
    assert!(refusable[4]);
    rig.give(0, mat(MaterialId::Bucket), 1);
    let (mut edits, mut hands, mut tags) = (Vec::new(), Vec::new(), Vec::new());
    for &cell in &logs {
        let (tag, edit, hand, _) = rig.make_use(UseKind::TapRubber, cell, block::RUBBER_LOG_TAPPED, 0);
        edits.push(edit);
        hands.push(hand);
        tags.push(tag);
    }
    rig.send_input(edits, hands, tags);
    for _ in 0..4 {
        rig.tick();
    }
    let refused = refusable.iter().filter(|&&r| r).count();
    assert_eq!(rig.c.refused.len(), refused);
    assert_eq!(rig.c.undone as usize, refused, "every refusal found its record");
    assert_eq!(units(&rig.c.inv, &mat(MaterialId::Rubber)) as usize, 5 - refused);
    rig.assert_lockstep("the accepted taps mirrored, the refused undone");
}

/// M-3 — a door's top half with no door below it (its bottom half refused,
/// or never sent): refused now (it costs nothing), sent back, and the joiner
/// told. Before C3c-1-fix it was applied: a floating top half.
#[test]
fn a_door_top_half_over_air_is_refused() {
    let mut rig = Rig::dedicated("door-over-air");
    let top = [ABOVE[0], ABOVE[1] + 1, ABOVE[2]];
    let tag = crate::use_edits::tag(UseKind::DoorUpper, top, 0, None);
    let upper = protocol::BlockChange { x: top[0], y: top[1], z: top[2], new_block: block::OAK_DOOR, meta: crate::use_edits::door_top_meta(0) };
    let (_, (k, id)) = rig.hand_now();
    rig.send_input(vec![upper], vec![(0, k, id)], vec![tag]);
    rig.tick();
    rig.tick();
    assert_eq!(rig.block(top), block::AIR, "no floating half");
    assert!(rig.c.changes.iter().any(|b| (b.x, b.y, b.z, b.new_block) == (top[0], top[1], top[2], block::AIR)), "sent back");
    assert_eq!(rig.c.refused.iter().map(|r| (r.kind, r.note)).collect::<Vec<_>>(), vec![(UseKind::DoorUpper.to_wire(), 0)]);
    assert_eq!(rig.tally().use_edit_refused, 1);
}

/// C3c-2-fix (F-L2) — a door's top half over another door's TOP half (a
/// column of free door tops) is refused like one over AIR: a top half stands
/// on a bottom half only.
#[test]
fn a_door_top_half_over_a_top_half_is_refused() {
    let mut rig = Rig::dedicated("door-over-top");
    let top = [ABOVE[0], ABOVE[1] + 1, ABOVE[2]];
    let third = [ABOVE[0], ABOVE[1] + 2, ABOVE[2]];
    rig.set(ABOVE, block::OAK_DOOR);
    rig.set(top, block::OAK_DOOR);
    rig.world().set_meta((top[0], top[1], top[2]), crate::use_edits::door_top_meta(0));
    rig.tick();
    let tag = crate::use_edits::tag(UseKind::DoorUpper, third, 0, None);
    let upper = protocol::BlockChange { x: third[0], y: third[1], z: third[2], new_block: block::OAK_DOOR, meta: crate::use_edits::door_top_meta(0) };
    let (_, (k, id)) = rig.hand_now();
    rig.send_input(vec![upper], vec![(0, k, id)], vec![tag]);
    rig.tick();
    rig.tick();
    assert_eq!(rig.block(third), block::AIR, "no door top over a door top");
    assert_eq!(rig.c.refused.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![UseKind::DoorUpper.to_wire()]);
    assert_eq!(rig.tally().use_edit_refused, 1);
}

/// C3c-2-fix (F-L1) — a door breaks whole ON THE SERVER. A joiner breaks a
/// door's top half at the edge of the server's reach, where the bottom half
/// (a block lower, so further from the eye) is out of it: the bottom half's
/// untagged AIR is refused, but the server clears the pair of the half it
/// accepted and sends it. No floating half on any seat, and one door.
#[test]
fn a_door_broken_at_the_reach_edge_breaks_whole_on_the_server() {
    let mut rig = Rig::dedicated("door-reach-edge");
    let (other, other_slot) = join_guest(&mut rig.hs, "Neighbour");
    let cs = crate::chunk::CHUNK_SIZE as i32;
    // The eye is at (40.5, 81.62, 40.5): the top half's centre (46.5, 81.5,
    // 42.5) is 6.33 away (in reach, 6.37), the bottom's 6.42 (out of it).
    let bottom: [i32; 3] = [46, 80, 42];
    let top = [46, 81, 42];
    rig.hs.hold_column_for_test(other_slot, (bottom[0].div_euclid(cs), bottom[2].div_euclid(cs)));
    rig.set(bottom, block::OAK_DOOR);
    rig.set(top, block::OAK_DOOR);
    rig.world().set_meta((top[0], top[1], top[2]), crate::use_edits::door_top_meta(0));
    rig.tick();
    let _ = super::joiner_authority::block_changes_seen(&other);
    let air = |c: [i32; 3]| protocol::BlockChange { x: c[0], y: c[1], z: c[2], new_block: block::AIR, meta: 0 };
    let (_, (k, id)) = rig.hand_now();
    let mined = protocol::MinedBlock { x: top[0], y: top[1], z: top[2], tool: protocol::WireItem::None };
    let input = protocol::InputPacket {
        tick: { rig.c.input_seq += 1; rig.c.input_seq },
        x: rig.sp().player.pos.x,
        y: rig.sp().player.pos.y,
        z: rig.sp().player.pos.z,
        health: 20.0,
        held_kind: k,
        held_id: id,
        // The survival break arm's order: the other half first, untagged.
        block_changes: vec![air(bottom), air(top)],
        edit_hands: vec![(0, k, id), (0, k, id)],
        mined: vec![mined],
        events_applied: rig.c.events,
        ..Default::default()
    };
    rig.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    for _ in 0..3 {
        rig.tick();
    }
    assert_eq!((rig.block(bottom), rig.block(top)), (block::AIR, block::AIR), "the server cleared both halves");
    let last = |seen: &[protocol::BlockChange], c: [i32; 3]| {
        seen.iter().rev().find(|b| [b.x, b.y, b.z] == c).map(|b| b.new_block)
    };
    assert_eq!(last(&rig.c.changes, bottom), Some(block::AIR), "the breaker's last word on the bottom half is AIR");
    let seen = super::joiner_authority::block_changes_seen(&other);
    assert_eq!(last(&seen, bottom), Some(block::AIR), "and the other joiner's");
    assert_eq!(last(&seen, top), Some(block::AIR));
    let door = Item::Block(block::OAK_DOOR);
    assert_eq!(units(&rig.c.inv, &door), 1, "one door");
    assert!(rig.ground_items().iter().all(|s| s.item != door), "no second door anywhere");
    assert_eq!(rig.tally().breaks, 1);
}

/// C3c-2-fix (F-M1) — a refused fill whose water bucket the client emptied
/// before the notice came back: the undo is short (the water bucket is
/// spent), so it gives the bucket back NO more — the empty already did. The
/// client ends with exactly the buckets the server's copy holds (it used to
/// end with one more: a bucket minted per cycle).
#[test]
fn a_refused_fill_emptied_before_its_notice_mints_no_bucket() {
    let mut rig = Rig::dedicated("refused-fill-emptied");
    // A pond out of the server's reach (the client's own view let it fill).
    let far = [48, 80, 40];
    rig.water_source(far);
    rig.give(0, mat(MaterialId::Bucket), 1);
    let (fill_tag, fill, fill_hand, _) = rig.make_use(UseKind::BucketFill, far, block::AIR, 0);
    assert_eq!(rig.c.inv.slot(0), Some(&ItemStack::new_material(MaterialId::WaterBucket, 1)));
    // Before the notice: the water bucket emptied in reach.
    let (empty_tag, empty, empty_hand, _) = rig.make_use(UseKind::BucketEmpty, ABOVE, block::WATER, 0);
    rig.send_input(vec![fill, empty], vec![fill_hand, empty_hand], vec![fill_tag, empty_tag]);
    for _ in 0..3 {
        rig.tick();
    }
    assert_eq!(rig.c.refused.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![UseKind::BucketFill.to_wire()]);
    let bucket = mat(MaterialId::Bucket);
    assert_eq!(units(&rig.sp().inventory, &bucket), 1, "the copy: one bucket (the fill never happened)");
    assert_eq!(units(&rig.c.inv, &bucket), 1, "the client: one bucket, not two");
    assert_eq!(units(&rig.c.inv, &mat(MaterialId::WaterBucket)), 0);
}

/// M-3 — a joiner breaks one half of its door: its break arm sends the other
/// half's AIR as its own untagged edit beside the broken half's (with its
/// `mined` tag). The server and another joiner lose both halves, the breaker
/// gets one door (the tagged half's yield), and there is no second door
/// anywhere. Both halves, each way round.
#[test]
fn a_joiner_breaking_either_half_of_a_door_takes_the_whole_door_everywhere() {
    for broken_top in [false, true] {
        let mut rig = Rig::dedicated(if broken_top { "door-break-top" } else { "door-break-bottom" });
        let (other, other_slot) = join_guest(&mut rig.hs, "Neighbour");
        let cs = crate::chunk::CHUNK_SIZE as i32;
        rig.hs.hold_column_for_test(other_slot, (ABOVE[0].div_euclid(cs), ABOVE[2].div_euclid(cs)));
        let top = [ABOVE[0], ABOVE[1] + 1, ABOVE[2]];
        rig.set(ABOVE, block::OAK_DOOR);
        rig.set(top, block::OAK_DOOR);
        rig.world().set_meta((top[0], top[1], top[2]), crate::use_edits::door_top_meta(0));
        rig.tick();
        let _ = super::joiner_authority::block_changes_seen(&other);
        let (broken, rest) = if broken_top { (top, ABOVE) } else { (ABOVE, top) };
        let air = |c: [i32; 3]| protocol::BlockChange { x: c[0], y: c[1], z: c[2], new_block: block::AIR, meta: 0 };
        // The survival break arm: the other half first (untagged), then the
        // broken half with its `mined` tag.
        let (_, (k, id)) = rig.hand_now();
        let mined = protocol::MinedBlock { x: broken[0], y: broken[1], z: broken[2], tool: protocol::WireItem::None };
        let input = protocol::InputPacket {
            tick: { rig.c.input_seq += 1; rig.c.input_seq },
            x: rig.sp().player.pos.x,
            y: rig.sp().player.pos.y,
            z: rig.sp().player.pos.z,
            health: 20.0,
            held_kind: k,
            held_id: id,
            block_changes: vec![air(rest), air(broken)],
            edit_hands: vec![(0, k, id), (0, k, id)],
            mined: vec![mined],
            events_applied: rig.c.events,
            ..Default::default()
        };
        rig.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        for _ in 0..3 {
            rig.tick();
        }
        assert_eq!((rig.block(ABOVE), rig.block(top)), (block::AIR, block::AIR), "the server lost both halves (top broken: {broken_top})");
        let seen = super::joiner_authority::block_changes_seen(&other);
        for c in [ABOVE, top] {
            assert!(seen.iter().any(|b| (b.x, b.y, b.z, b.new_block) == (c[0], c[1], c[2], block::AIR)), "the other joiner lost {c:?}");
        }
        let door = Item::Block(block::OAK_DOOR);
        assert_eq!(units(&rig.c.inv, &door), 1, "one door in the breaker's bag (top broken: {broken_top})");
        assert!(rig.ground_items().iter().all(|s| s.item != door), "no second door anywhere");
        assert_eq!(rig.tally().breaks, 1);
    }
}

/// L-1 — a fill of a source that flowed on the server (honest drift): the
/// copy tracks the client — the bucket spent and the water bucket made —
/// tallied as a drift mismatch.
#[test]
fn a_fill_of_a_source_the_server_saw_flow_is_mirrored_as_drift() {
    let mut rig = Rig::dedicated("drift-fill");
    rig.set(ABOVE, block::WATER);
    rig.give(0, mat(MaterialId::Bucket), 2);
    rig.use_at(UseKind::BucketFill, ABOVE, block::AIR, 0);
    assert_eq!(rig.block(ABOVE), block::AIR);
    rig.assert_lockstep("drift tracks the client");
    assert_eq!(rig.c.inv.slot(1), Some(&ItemStack::new_material(MaterialId::WaterBucket, 1)));
    let t = rig.tally();
    assert_eq!((t.use_mirrored, t.use_mismatch), (0, 1));
}

/// L-1 — a hoe tag on an ordinary stone placement can't explain it: the
/// edit is classified as the placement it is, so the possession check sees
/// it (one stone charged), never as a use.
#[test]
fn a_tag_that_cant_explain_its_edit_falls_back_to_the_ordinary_check() {
    let mut rig = Rig::dedicated("unexplained");
    rig.give(0, Item::Block(block::STONE), 3);
    rig.c.inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 2)));
    let tag = crate::use_edits::tag(UseKind::Till, ABOVE, 0, rig.c.inv.slot(0).map(|s| &s.item));
    let place = protocol::BlockChange { x: ABOVE[0], y: ABOVE[1], z: ABOVE[2], new_block: block::STONE, meta: 0 };
    let (_, (k, id)) = rig.hand_now();
    rig.send_input(vec![place], vec![(0, k, id)], vec![tag]);
    rig.tick();
    assert_eq!(rig.block(ABOVE), block::STONE);
    rig.assert_lockstep("the placement charged");
    let t = rig.tally();
    assert_eq!((t.matched, t.mismatched), (1, 0), "checked as a placement");
    assert_eq!((t.use_mirrored, t.use_mismatch), (0, 1), "and counted as a use that didn't explain itself");
}

/// L-1 — bone meal the client sent for a crop the server had already grown
/// past: the server's crop is NOT set back, no refusal is sent (no undo), and
/// the bone meal is spent on both sides, tallied as drift.
#[test]
fn bone_meal_on_a_crop_the_server_grew_first_doesnt_set_it_back() {
    let mut rig = Rig::dedicated("crop-ahead");
    rig.set(FLOOR, block::TILLED_SOIL);
    rig.set(ABOVE, block::WHEAT_STAGE_0);
    rig.give(0, mat(MaterialId::Bonemeal), 3);
    let (tag, edit, hand, _) = rig.make_use(UseKind::GrowCrop, ABOVE, block::WHEAT_STAGE_1, 0);
    // The server's crop grew to stage 3 meanwhile.
    rig.set(ABOVE, block::WHEAT_STAGE_3);
    rig.send_input(vec![edit], vec![hand], vec![tag]);
    rig.tick();
    rig.tick();
    assert_eq!(rig.block(ABOVE), block::WHEAT_STAGE_3, "not set back");
    assert!(rig.c.refused.is_empty() && rig.c.undone == 0, "not a refusal");
    assert!(!rig.c.changes.iter().any(|b| (b.x, b.y, b.z) == (ABOVE[0], ABOVE[1], ABOVE[2])), "no send-back");
    rig.assert_lockstep("the bone meal spent on both sides");
    assert_eq!(rig.tally().use_mismatch, 1);
}

/// L-2 — one rubber clock, the server's: a joiner taps, the server restores
/// the log once its cooldown runs out on the server's clock, and queues the
/// change, so a second joiner (and on a lending host, the host's screen)
/// sees the log regrow.
#[test]
fn the_servers_rubber_clock_regrows_a_tapped_log_for_everyone() {
    for lent in [false, true] {
        let mut rig = if lent { Rig::lent("regrow") } else { Rig::dedicated("regrow") };
        let (other, other_slot) = match rig.host.as_mut() {
            Some(h) => join_guest_lent(&mut rig.hs, h, "Neighbour"),
            None => join_guest(&mut rig.hs, "Neighbour"),
        };
        let cs = crate::chunk::CHUNK_SIZE as i32;
        rig.hs.hold_column_for_test(other_slot, (ABOVE[0].div_euclid(cs), ABOVE[2].div_euclid(cs)));
        rig.set(ABOVE, block::RUBBER_LOG);
        rig.give(0, mat(MaterialId::Bucket), 1);
        rig.use_at(UseKind::TapRubber, ABOVE, block::RUBBER_LOG_TAPPED, 0);
        assert_eq!(rig.block(ABOVE), block::RUBBER_LOG_TAPPED);
        let _ = super::joiner_authority::block_changes_seen(&other);
        if let Some(h) = rig.host.as_mut() {
            let _ = rig.hs.take_lent_changes();
            h.clock.tick_counter += crate::rubber::TAP_COOLDOWN_TICKS;
        } else {
            rig.hs.server.tick_counter += crate::rubber::TAP_COOLDOWN_TICKS;
        }
        rig.tick();
        assert_eq!(rig.block(ABOVE), block::RUBBER_LOG, "the server's clock restored it (lent: {lent})");
        let regrown = |b: &protocol::BlockChange| (b.x, b.y, b.z, b.new_block) == (ABOVE[0], ABOVE[1], ABOVE[2], block::RUBBER_LOG);
        assert!(super::joiner_authority::block_changes_seen(&other).iter().any(regrown), "the other joiner sees it (lent: {lent})");
        assert!(rig.c.changes.iter().any(regrown), "and so does the tapper");
        if lent {
            assert!(rig.hs.take_lent_changes().0.iter().any(regrown), "the host's screen remeshes it");
        }
    }
}

/// L-2 — source lint, in `a_joined_client_runs_no_composter_hive_or_rack_sim`'s
/// tradition (the client tick needs a GPU): a joined client runs no rubber
/// regrowth of its own (`remote_client.is_none()` beside the driver).
#[test]
fn a_joined_client_runs_no_rubber_clock() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("rubber-clock lint: cannot read {} ({e})", path.display()));
    let lines: Vec<&str> = raw.lines().collect();
    let needle = "crate::rubber::tick_rubber_cooldowns(&mut self.world, self.tick_counter)";
    let hits: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(needle))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(hits.len(), 1, "rubber-clock lint: `{needle}` should appear once in game_loop.rs, found {hits:?}");
    let i = hits[0];
    assert!(
        lines[i.saturating_sub(2)..=i].iter().any(|l| l.contains("remote_client.is_none()")),
        "game_loop.rs:{}: a joined client runs its own rubber clock — gate it behind `self.remote_client.is_none()`",
        i + 1
    );
}

// ─── C3c-3a — Plans by marker ──────────────────────────────────────────

/// A developed Plan (its marker differs from `latent_plan`'s and from
/// another named one's).
fn developed_plan(name: &str) -> crate::plan::PlanData {
    crate::plan::PlanData { name: name.to_string(), ..crate::plan::PlanData::debug_3x3_stone() }
}

/// The server's stand-in for `plan`.
fn placeholder_of(plan: &crate::plan::PlanData) -> Item {
    let developed = plan.develop_state == crate::plan::DevelopState::Developed;
    Item::Plan(crate::plan::PlanData::marker_placeholder(crate::plan::marker(plan), developed))
}

/// Does `slot` of `inv` hold `plan` (a real Plan or its placeholder, by
/// marker)?
fn holds_plan(inv: &Inventory, slot: usize, plan: &crate::plan::PlanData) -> bool {
    matches!(inv.slot(slot).map(|s| &s.item), Some(Item::Plan(p)) if p.same_plan(plan))
}

impl Rig {
    /// The client sends item action `action` (as `GameState::send_request`
    /// sends a never-answered one), then one tick.
    fn send_action(&mut self, action: protocol::ItemAction) {
        let c = &mut self.c;
        // Any request number (it is never answered); not the op sequence.
        let pkt = protocol::ItemActionPacket { seq: 1000 + c.input_seq as u32, action, events_applied: c.events };
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
        self.tick();
    }

    /// The client's window ops so far, sent.
    fn send_ops(&mut self) {
        let c = &mut self.c;
        for logged in c.ui.take_ops(&c.inv, &c.armour) {
            c.seq += 1;
            let pkt = logged.packet(c.seq, c.events);
            c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
        }
        self.tick();
    }

    /// The client's window digest, and the server's copy's.
    fn digests(&self) -> (u32, u32) {
        let sp = &self.hs.server.players[self.c.slot];
        let c = &self.c;
        (
            window::digest_parts(&c.inv, &c.armour, &c.ui.cursor_item, &c.ui.grid, c.ui.station()),
            window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station),
        )
    }
}

/// A joiner hangs a developed Plan (its tag names it by marker): the edit is
/// applied as a use, never a placement, and the server's copy loses THAT
/// Plan's placeholder by marker — with two Plans in the copy and the other
/// one in the tag's slot (a drifted copy), the right one goes.
#[test]
fn a_joiners_hung_print_takes_its_plan_by_marker() {
    let mut rig = Rig::dedicated("hang");
    let wall = [ABOVE[0] + 1, ABOVE[1], ABOVE[2]];
    rig.set(wall, block::STONE);
    let (hung, kept) = (developed_plan("Hung"), developed_plan("Kept"));
    rig.c.inv.set_slot(0, Some(ItemStack { item: Item::Plan(hung.clone()), count: 1 }));
    rig.c.inv.set_slot(1, Some(ItemStack { item: Item::Plan(kept.clone()), count: 1 }));
    rig.sp().inventory.set_slot(0, Some(ItemStack { item: placeholder_of(&kept), count: 1 }));
    rig.sp().inventory.set_slot(1, Some(ItemStack { item: placeholder_of(&hung), count: 1 }));
    rig.use_at(UseKind::HangPrint, ABOVE, block::CYANOTYPE_PRINT, 0);
    assert_eq!(rig.block(ABOVE), block::CYANOTYPE_PRINT, "the edit is applied");
    let t = rig.tally();
    assert_eq!((t.use_mirrored, t.use_mismatch), (1, 0), "one use mirrored");
    assert_eq!((t.matched, t.mismatched), (0, 0), "never classified as a placement");
    let copy = &rig.hs.server.players[rig.c.slot].inventory;
    assert!(copy.slot(1).is_none(), "the hung Plan's placeholder went, by marker");
    assert!(holds_plan(copy, 0, &kept), "the other one stayed in the tag's slot");
    assert!(rig.c.inv.slot(0).is_none() && holds_plan(&rig.c.inv, 1, &kept));
}

/// A hang in someone else's plot (C3c-1-fix M-4): refused, told, and undone
/// from the client's own record — the client's Plan comes back whole into
/// its slot (never a marker placeholder); the server's copy never applied
/// it, so its placeholder stays where it was.
#[test]
fn a_refused_hang_gives_the_plan_back_and_the_copy_keeps_its_placeholder() {
    let mut rig = Rig::dedicated("hang-refused");
    rig.set([ABOVE[0] + 1, ABOVE[1], ABOVE[2]], block::STONE);
    foreign_plot(&mut rig, ABOVE);
    let plan = developed_plan("Refused");
    rig.c.inv.set_slot(0, Some(ItemStack { item: Item::Plan(plan.clone()), count: 1 }));
    rig.sp().inventory.set_slot(0, Some(ItemStack { item: placeholder_of(&plan), count: 1 }));
    rig.use_at(UseKind::HangPrint, ABOVE, block::CYANOTYPE_PRINT, 0);
    rig.tick();
    assert_eq!(rig.block(ABOVE), block::AIR, "never hung on the server");
    assert_eq!(rig.c.undone, 1, "undone from the client's own record");
    assert_eq!(rig.c.inv.slot(0).map(|s| s.item.clone()), Some(Item::Plan(plan.clone())), "the real Plan is back");
    assert!(holds_plan(&rig.hs.server.players[rig.c.slot].inventory, 0, &plan), "the copy kept its placeholder");
    let (client, server) = rig.digests();
    assert_eq!(server, client);
    let t = rig.tally();
    assert_eq!((t.use_edit_refused, t.use_mirrored, t.use_mismatch), (1, 0, 0));
}

/// A latent Plan doesn't hang: the outcome is tallied (log-only), applied,
/// and the Plan still spent on the copy as the client says.
#[test]
fn a_latent_plans_hang_is_tallied_and_still_applied() {
    let mut rig = Rig::dedicated("hang-latent");
    rig.set([ABOVE[0] + 1, ABOVE[1], ABOVE[2]], block::STONE);
    let latent = crate::plan::PlanData {
        develop_state: crate::plan::DevelopState::Latent { exposure_ticks: 0 },
        ..developed_plan("Latent")
    };
    rig.c.inv.set_slot(0, Some(ItemStack { item: Item::Plan(latent.clone()), count: 1 }));
    rig.sp().inventory.set_slot(0, Some(ItemStack { item: placeholder_of(&latent), count: 1 }));
    rig.use_at(UseKind::HangPrint, ABOVE, block::CYANOTYPE_PRINT, 0);
    assert_eq!(rig.block(ABOVE), block::CYANOTYPE_PRINT, "log-only: applied");
    let t = rig.tally();
    assert_eq!((t.use_mirrored, t.use_mismatch), (0, 1));
    assert!(rig.hs.server.players[rig.c.slot].inventory.slot(0).is_none(), "spent as the client says");
}

/// A joiner's art capture, reported (`PlanMinted { CaptureArt }`): the
/// server's copy gains the Plan's marker placeholder in the slot the client's
/// Plan landed in (the first empty one, BEFORE the last paper of slot 0 was
/// taken) and loses the paper; then the Plan moves between slots and into the
/// hotbar with no window mismatch.
#[test]
fn a_joiners_art_capture_lands_its_marker_where_the_client_did_and_moves_in_lockstep() {
    let mut rig = Rig::dedicated("art-mint");
    rig.give(0, Item::Block(block::BLUEPRINT_PAPER), 1);
    rig.give(1, Item::Block(block::STONE), 5);
    let art = crate::plan::PlanData {
        develop_state: crate::plan::DevelopState::Latent { exposure_ticks: 0 },
        kind: crate::plan::PlanKind::Art,
        ..developed_plan("Cyanotype: 1×1")
    };
    // The client's art-capture arm: add the Plan, then take the paper.
    let paper = rig.c.inv.hotbar_slot(0).cloned().map(|s| ItemStack { count: 1, ..s });
    assert!(rig.c.inv.add_item(ItemStack { item: Item::Plan(art.clone()), count: 1 }).is_none());
    assert!(rig.c.inv.take_one_from_hotbar(0).is_some());
    assert!(holds_plan(&rig.c.inv, 2, &art) && rig.c.inv.slot(0).is_none());
    rig.send_action(protocol::ItemAction::PlanMinted {
        source: crate::plan_mint::MintSource::CaptureArt.to_wire(),
        x: ABOVE[0],
        y: ABOVE[1],
        z: ABOVE[2] - 1,
        face: crate::mesh::Face::South.index() as u8,
        hotbar_slot: 0,
        spent: paper.as_ref().map(crate::inventory::stack_to_wire),
        plan: crate::inventory::plan_to_wire(&art),
    });
    let copy = &rig.hs.server.players[rig.c.slot].inventory;
    assert!(holds_plan(copy, 2, &art), "the placeholder, in the client's slot");
    assert!(copy.slot(0).is_none(), "the paper went");
    assert!(matches!(copy.slot(2).map(|s| &s.item), Some(Item::Plan(p)) if p.marker.is_some() && p.cells.is_empty()), "body-less");
    let t = rig.tally();
    assert_eq!((t.plan_minted, t.plan_mismatch), (1, 0));
    let (client, server) = rig.digests();
    assert_eq!(server, client, "it digests like the real one");
    // The Plan moves: slot 2 → slot 20 → hotbar slot 7.
    rig.c.ui.open_player_crafting(&rig.c.inv, &rig.c.armour);
    for slot in [2, 20, 20, 7] {
        let c = &mut rig.c;
        let r = c.ui.apply_click(&mut c.inv, &mut c.armour, &WindowClick::Slot { slot, right: false }, false, Vec3::ZERO, |_| block::AIR);
        assert!(r.ok(), "click {slot}");
    }
    let c = &mut rig.c;
    assert!(c.ui.close(&mut c.inv, &mut c.armour));
    rig.send_ops();
    let t = rig.tally();
    assert!(t.window_ops >= 6, "the open, four clicks and the close were mirrored: {}", t.window_ops);
    assert_eq!(t.window_mismatch, 0, "the placeholder moved as the Plan did");
    assert!(holds_plan(&rig.c.inv, 7, &art));
    assert!(holds_plan(&rig.hs.server.players[rig.c.slot].inventory, 7, &art));
}

/// A joiner's capture commit, reported (`PlanMinted { CaptureCommit }`):
/// the copy gains the placeholder and spends nothing (the paper was spent
/// when it was laid). A mint whose paper the copy doesn't hold is counted
/// (log-only) and mirrored as reported.
#[test]
fn a_joiners_capture_commit_lands_its_marker_and_a_short_mint_is_counted() {
    let mut rig = Rig::dedicated("commit-mint");
    rig.give(0, Item::Block(block::STONE), 5);
    let house = developed_plan("House");
    assert!(rig.c.inv.add_item(ItemStack { item: Item::Plan(house.clone()), count: 1 }).is_none());
    rig.send_action(protocol::ItemAction::PlanMinted {
        source: crate::plan_mint::MintSource::CaptureCommit.to_wire(),
        x: FLOOR[0],
        y: FLOOR[1],
        z: FLOOR[2],
        face: crate::mesh::Face::Top.index() as u8,
        hotbar_slot: 0,
        spent: None,
        plan: crate::inventory::plan_to_wire(&house),
    });
    let copy = &rig.hs.server.players[rig.c.slot].inventory;
    assert!(holds_plan(copy, 1, &house));
    assert_eq!(copy.slot(0).map(|s| s.count), Some(5), "nothing spent");
    assert_eq!((rig.tally().plan_minted, rig.tally().plan_mismatch), (1, 0));
    // An art capture whose paper the copy never had: counted, mirrored.
    let art = crate::plan::PlanData { kind: crate::plan::PlanKind::Art, ..developed_plan("Short") };
    rig.send_action(protocol::ItemAction::PlanMinted {
        source: crate::plan_mint::MintSource::CaptureArt.to_wire(),
        x: ABOVE[0],
        y: ABOVE[1],
        z: ABOVE[2],
        face: crate::mesh::Face::North.index() as u8,
        hotbar_slot: 3,
        spent: Some(crate::inventory::stack_to_wire(&ItemStack::new_block(block::BLUEPRINT_PAPER, 1))),
        plan: crate::inventory::plan_to_wire(&art),
    });
    let t = rig.tally();
    assert_eq!((t.plan_minted, t.plan_mismatch), (2, 1));
    assert!(holds_plan(&rig.hs.server.players[rig.c.slot].inventory, 2, &art), "mirrored as reported");
    let summary = t.summary("Drafter").unwrap();
    assert!(summary.contains("2 Plan mint(s) mirrored by marker, 1 with a shortfall"), "{summary}");
}
