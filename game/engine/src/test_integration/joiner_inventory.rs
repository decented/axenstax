//! C1 (2026-10-07) — inventory authority, merge 1: the server yields a
//! joiner's breaks (`break_drops`, granted by `InventoryGrant`) and keeps a
//! shadow of its inventory (`ServerPlayer.inventory`, `joiner_inventory`),
//! with a LOG-ONLY possession check on plain placements.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport — a
//! dedicated server, or a lending host where the world is the host's — and
//! reads what the joiner is actually sent and what the server's shadow holds.

use glam::Vec3;

use crate::block::{self, BlockId};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item::{Item, ItemStack, MaterialId};
use crate::protocol::{self, InventoryGrantPacket, MinedBlock, WireItem};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;
use super::joiners_act::floor_and_stand;
use super::lent_world::{join_guest_lent, start_lent};

/// One joiner standing on a stone floor at (40, 80, 40), on a dedicated
/// server or a lending host (`host`: the host client's world, lent to the
/// server each tick).
struct Rig {
    hs: HostedServer,
    host: Option<OwnedSimParts>,
    client: ChannelClientTransport,
    slot: usize,
    at: Vec3,
    input: u64,
    grants: Vec<InventoryGrantPacket>,
    /// Every block change the joiner was sent (`StateUpdate`s).
    changes: Vec<protocol::BlockChange>,
}

impl Rig {
    fn dedicated(tag: &str) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("joiner-inventory-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let (client, slot) = join_guest(&mut hs, "Miner");
        Self::stand(hs, None, client, slot)
    }

    fn lent(tag: &str) -> Self {
        let (mut hs, mut host) = start_lent(&format!("joiner-inventory-{tag}"));
        let (client, slot) = join_guest_lent(&mut hs, &mut host, "Miner");
        Self::stand(hs, Some(host), client, slot)
    }

    fn stand(
        mut hs: HostedServer,
        mut host: Option<OwnedSimParts>,
        client: ChannelClientTransport,
        slot: usize,
    ) -> Self {
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        let world = match host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut hs.server.world,
        };
        let mut world = std::mem::replace(world, crate::world::World::new());
        let at = floor_and_stand(&mut world, &mut hs, slot);
        match host.as_mut() {
            Some(h) => h.world = world,
            None => hs.server.world = world,
        }
        let mut rig = Rig { hs, host, client, slot, at, input: 0, grants: Vec::new(), changes: Vec::new() };
        rig.loaded((40, 79, 40));
        rig.tick();
        rig.grants.clear();
        rig
    }

    fn world(&mut self) -> &mut crate::world::World {
        match self.host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut self.hs.server.world,
        }
    }

    /// The world clock a break on the next tick is rolled at.
    fn clock(&self) -> u64 {
        match &self.host {
            // The host advances its clock, then lends.
            Some(h) => h.clock.tick_counter + 1,
            None => self.hs.server.tick_counter,
        }
    }

    /// Mark `cell`'s column loaded (the server takes edits only there).
    fn loaded(&mut self, cell: (i32, i32, i32)) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let col = (cell.0.div_euclid(cs), cell.2.div_euclid(cs));
        match self.host.as_mut() {
            Some(h) => h.loaded_columns.insert(col),
            None => self.hs.server.loaded_columns.insert(col),
        };
    }

    fn tick(&mut self) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        while let Some(pkt) = self.client.try_recv_from_server() {
            match protocol::deserialize_header(&pkt) {
                Some((protocol::PacketType::InventoryGrant, payload)) => {
                    self.grants.push(protocol::safe_deserialize(payload).unwrap());
                }
                Some((protocol::PacketType::StateUpdate, payload)) => {
                    let state: protocol::StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                    self.changes.extend(state.block_changes);
                }
                _ => {}
            }
        }
    }

    /// One input from the joiner: `edits`, the cells it says it mined, and
    /// what its hand holds (hotbar slot 0).
    fn send(&mut self, held: Option<&Item>, edits: &[((i32, i32, i32), BlockId)], mined: &[MinedBlock]) {
        self.send_at(0, held, edits, mined);
    }

    /// [`Self::send`] from hotbar slot `slot`.
    fn send_at(
        &mut self,
        slot: u8,
        held: Option<&Item>,
        edits: &[((i32, i32, i32), BlockId)],
        mined: &[MinedBlock],
    ) {
        self.input += 1;
        let (held_kind, held_id) = match held {
            Some(item) => crate::inventory::item_to_ref(item).to_wire(),
            None => protocol::ItemRef::Empty.to_wire(),
        };
        let input = protocol::InputPacket {
            tick: self.input,
            x: self.at.x,
            y: self.at.y,
            z: self.at.z,
            health: 20.0,
            held_kind,
            held_id,
            hotbar_slot: Some(slot),
            block_changes: edits
                .iter()
                .map(|&((x, y, z), b)| protocol::BlockChange { x, y, z, new_block: b, meta: 0 })
                .collect(),
            mined: mined.to_vec(),
            ..Default::default()
        };
        self.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// The joiner mines `cell` (leaving `left`) with `tool`; one tick.
    fn mine(&mut self, cell: (i32, i32, i32), left: BlockId, tool: Option<Tool>) {
        let tag = mined(cell, tool);
        self.send(tool.map(Item::Tool).as_ref(), &[(cell, left)], &[tag]);
        self.tick();
    }

    fn granted(&self, item: &Item) -> u32 {
        let (kind, id) = crate::inventory::item_to_ref(item).to_wire();
        self.grants.iter().filter(|g| (g.item_kind, g.item_id) == (kind, id)).map(|g| u32::from(g.count)).sum()
    }

    fn shadow_count(&self, item: &Item) -> u32 {
        self.hs.server.players[self.slot]
            .inventory
            .slots_iter()
            .flatten()
            .filter(|s| &s.item == item)
            .map(|s| u32::from(s.count))
            .sum()
    }

    fn tally(&self) -> crate::joiner_inventory::PossessionTally {
        self.hs.server.players[self.slot].possession
    }
}

fn mined((x, y, z): (i32, i32, i32), tool: Option<Tool>) -> MinedBlock {
    MinedBlock {
        x,
        y,
        z,
        tool: tool.map_or(WireItem::None, |t| crate::inventory::item_to_wire_full(&Item::Tool(t))),
    }
}

fn pick(material: ToolMaterial) -> Tool {
    Tool::new(ToolType::Pickaxe, material)
}

/// The floor cell beside the joiner (in reach).
const FLOOR: (i32, i32, i32) = (41, 79, 40);
/// The air cell above it.
const ABOVE: (i32, i32, i32) = (41, 80, 40);

#[test]
fn a_joiners_mined_stone_is_granted_once_by_the_server_and_lands_in_its_shadow() {
    let mut rig = Rig::dedicated("mine");
    rig.mine(FLOOR, block::AIR, Some(pick(ToolMaterial::Wood)));
    let cobble = Item::Block(block::COBBLESTONE);
    assert_eq!(rig.grants.len(), 1, "one break, one grant");
    assert_eq!(rig.granted(&cobble), 1, "stone yields cobblestone");
    assert_eq!(rig.shadow_count(&cobble), 1, "and the server's shadow holds it");
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::AIR);
    assert_eq!(rig.tally().breaks, 1);
    // A tick later nothing more arrives: the drop is granted exactly once.
    rig.tick();
    assert_eq!(rig.granted(&cobble), 1);
}

#[test]
fn an_emptying_edit_the_joiner_did_not_mine_yields_nothing() {
    let mut rig = Rig::dedicated("untagged");
    // A bucket scoop, an Eraser, the client's own piston or keg: the cell
    // empties, but nothing was mined.
    rig.send(None, &[(FLOOR, block::AIR)], &[]);
    rig.tick();
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::AIR, "the edit is accepted");
    assert!(rig.grants.is_empty(), "but yields nothing");
    assert_eq!(rig.tally().unchecked, 1);
    // A tag on a cell whose edit isn't in the packet yields nothing either.
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::STONE);
    rig.send(None, &[], &[mined(FLOOR, Some(pick(ToolMaterial::Wood)))]);
    rig.tick();
    assert!(rig.grants.is_empty());
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::STONE);
}

#[test]
fn below_the_tool_tier_the_block_breaks_and_nothing_drops() {
    let mut rig = Rig::dedicated("tier");
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::IRON_ORE);
    rig.mine(FLOOR, block::AIR, Some(pick(ToolMaterial::Wood)));
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::AIR, "broken");
    assert!(rig.grants.is_empty(), "a wooden pickaxe can't harvest iron ore");
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::IRON_ORE);
    rig.mine(FLOOR, block::AIR, Some(pick(ToolMaterial::Stone)));
    assert_eq!(rig.granted(&Item::Material(MaterialId::RawIron)), 1, "a stone one can");
}

#[test]
fn a_joiners_crop_harvest_yields_by_the_shared_rules() {
    let mut rig = Rig::dedicated("crop");
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::TILLED_SOIL);
    rig.world().set_block(ABOVE.0, ABOVE.1, ABOVE.2, block::WHEAT_STAGE_3);
    let clock = rig.clock();
    rig.mine(ABOVE, block::TILLED_SOIL, None);
    let expected = crate::growth::crop_break(
        block::WHEAT_STAGE_3,
        crate::break_drops::drop_seed(clock, ABOVE.0, ABOVE.1, ABOVE.2),
        true,
    )
    .unwrap();
    for stack in &expected.drops {
        assert_eq!(rig.granted(&stack.item), u32::from(stack.count), "{:?}", stack.item);
    }
    assert_eq!(rig.world().get_block(ABOVE.0, ABOVE.1, ABOVE.2), block::TILLED_SOIL);
}

/// A vein origin (its own member) for `secret` at Satori depth.
fn vein_cell(secret: &[u8; 32], seed: u64) -> (i32, i32, i32) {
    for x in 0..200 {
        for y in 0..=(crate::biome::Y_DP - 21) {
            for z in 0..200 {
                if crate::proof_of_play::is_vein_origin(secret, seed, 0, x, y, z) {
                    return (x, y, z);
                }
            }
        }
    }
    panic!("no vein origin found");
}

/// Ready `cell` for a Satori: pure deepslate, freshly exposed (`exposed_at`),
/// a pure-deepslate neighbour, its column loaded, the joiner's body beside it
/// facing it.
fn ready_vein(rig: &mut Rig, cell: (i32, i32, i32), exposed_at: u64) {
    let (x, y, z) = cell;
    let world = rig.world();
    world.set_block(x, y, z, block::PURE_DEEPSLATE);
    world.set_block(x + 1, y, z, block::PURE_DEEPSLATE);
    world.pop_exposure.clear();
    world.pop_exposure.insert(cell, exposed_at);
    rig.loaded(cell);
    let p = &mut rig.hs.server.players[rig.slot].player;
    p.pos = Vec3::new(x as f32 + 0.5, y as f32 + 1.0, z as f32 - 1.5);
    p.velocity = Vec3::ZERO;
}

/// The Satori is the WORLD's roll: on its secret, whatever the joiner holds
/// of its own (its client takes nothing from its break —
/// `break_drops::take_yield` — and sends no secret).
fn satori_on_the_worlds_secret(mut rig: Rig) {
    const SECRET: [u8; 32] = [0x5A; 32];
    let seed = rig.hs.server.biome_gen.seed as u64;
    let cell = vein_cell(&SECRET, seed);
    let other = (0u8..=255)
        .map(|b| [b; 32])
        .find(|s| !crate::proof_of_play::block_is_vein_member(s, seed, 0, cell.0, cell.1, cell.2))
        .unwrap();
    let diamond = Some(pick(ToolMaterial::Diamond));
    let satori = Item::Material(MaterialId::Satori);

    rig.hs.server.pop_secret = SECRET;
    let at = rig.clock();
    ready_vein(&mut rig, cell, at);
    rig.mine(cell, block::AIR, diamond);
    assert_eq!(rig.granted(&satori), 1, "a freshly exposed vein cell yields a Satori");
    assert_eq!(rig.shadow_count(&satori), 1);
    let world = rig.world();
    assert!(
        world.pop_exposure.contains_key(&(cell.0 + 1, cell.1, cell.2)),
        "the break exposed its pure-deepslate neighbour on the world's clock"
    );

    // The same break on a world whose secret has no vein here: nothing.
    rig.hs.server.pop_secret = other;
    let at = rig.clock();
    ready_vein(&mut rig, cell, at);
    rig.mine(cell, block::AIR, diamond);
    assert_eq!(rig.granted(&satori), 1, "no second Satori on a secret without the vein");
}

#[test]
fn a_joiners_satori_is_rolled_on_the_dedicated_servers_secret() {
    satori_on_the_worlds_secret(Rig::dedicated("satori"));
}

#[test]
fn a_joiners_satori_is_rolled_on_the_lending_hosts_secret_and_exposure_map() {
    satori_on_the_worlds_secret(Rig::lent("satori-lent"));
}

/// Review MEDIUM-1 — every block a joiner puts into a cell is player-placed,
/// whatever the server classifies the edit as. Only a `Place` used to be
/// flagged, so a modified client could refill a cell that just yielded a
/// Satori by another accepted path and mine it again: the roll is
/// deterministic per cell and its exposure entry stands, so it paid every
/// time. Each fill path below leaves the cell player-placed, and the re-mine
/// yields no second Satori.
#[test]
fn a_refilled_satori_cell_is_player_placed_whatever_the_fill_path() {
    const SECRET: [u8; 32] = [0x5A; 32];
    let mut rig = Rig::dedicated("refill");
    let seed = rig.hs.server.biome_gen.seed as u64;
    let cell = vein_cell(&SECRET, seed);
    let (x, y, z) = cell;
    let diamond = Some(pick(ToolMaterial::Diamond));
    let satori = Item::Material(MaterialId::Satori);
    let deepslate = Item::Block(block::PURE_DEEPSLATE);
    let tool_in_hand = Item::Tool(pick(ToolMaterial::Diamond));

    rig.hs.server.pop_secret = SECRET;
    let at = rig.clock();
    ready_vein(&mut rig, cell, at);
    rig.mine(cell, block::AIR, diamond);
    assert_eq!(rig.granted(&satori), 1, "the vein cell yields its Satori");

    for path in ["a tool claimed in hand", "a mined tag on the fill", "creative mode"] {
        let creative = path == "creative mode";
        if creative {
            rig.hs.server.set_play_mode(crate::play_mode::PlayMode::Creative);
        }
        match path {
            "a tool claimed in hand" => rig.send(Some(&tool_in_hand), &[(cell, block::PURE_DEEPSLATE)], &[]),
            "a mined tag on the fill" => {
                rig.send(Some(&deepslate), &[(cell, block::PURE_DEEPSLATE)], &[mined(cell, diamond)])
            }
            _ => rig.send(Some(&deepslate), &[(cell, block::PURE_DEEPSLATE)], &[]),
        }
        rig.tick();
        if creative {
            rig.hs.server.set_play_mode(crate::play_mode::PlayMode::Survival);
        }
        assert_eq!(rig.world().get_block(x, y, z), block::PURE_DEEPSLATE, "{path}: the refill is accepted");
        assert!(rig.world().is_placed(x, y, z), "{path}: the refilled cell reads player-placed");
        assert!(rig.world().pop_exposure.contains_key(&cell), "{path}: its exposure still stands");
        rig.mine(cell, block::AIR, diamond);
        assert_eq!(rig.world().get_block(x, y, z), block::AIR, "{path}: mined again");
        assert_eq!(rig.granted(&satori), 1, "{path}: no second Satori");
    }
}

#[test]
fn a_joiners_placement_consumes_from_the_shadow_and_is_flagged_placed() {
    let mut rig = Rig::dedicated("place");
    let stone = Item::Block(block::STONE);
    rig.hs.server.players[rig.slot].inventory.set_slot(0, Some(ItemStack::new_block(block::STONE, 3)));
    rig.send(Some(&stone), &[(ABOVE, block::STONE)], &[]);
    rig.tick();
    assert_eq!(rig.world().get_block(ABOVE.0, ABOVE.1, ABOVE.2), block::STONE);
    assert_eq!(rig.shadow_count(&stone), 2, "one consumed from the held slot");
    assert_eq!(rig.tally().matched, 1);
    assert!(
        rig.world().is_placed(ABOVE.0, ABOVE.1, ABOVE.2),
        "player-placed: re-mining it yields no Satori (Spec 06 §2.2)"
    );
    // Mined again: natural once more, and the stone comes back (cobblestone).
    rig.mine(ABOVE, block::AIR, Some(pick(ToolMaterial::Wood)));
    assert!(!rig.world().is_placed(ABOVE.0, ABOVE.1, ABOVE.2));
}

#[test]
fn placing_what_the_shadow_lacks_is_counted_and_still_accepted() {
    let mut rig = Rig::dedicated("mismatch");
    let stone = Item::Block(block::STONE);
    rig.send(Some(&stone), &[(ABOVE, block::STONE)], &[]);
    rig.tick();
    assert_eq!(rig.world().get_block(ABOVE.0, ABOVE.1, ABOVE.2), block::STONE, "log-only: accepted");
    assert_eq!(rig.tally().mismatched, 1);
    assert_eq!(rig.tally().matched, 0);
    assert!(rig.hs.server.players[rig.slot].inventory.slots_iter().all(|s| s.is_none()), "never corrected");
    // A non-block placement (a water bucket) isn't checked at all.
    let bucket = Item::Material(MaterialId::WaterBucket);
    rig.send(Some(&bucket), &[((42, 80, 40), block::WATER)], &[]);
    rig.tick();
    assert_eq!(rig.tally().mismatched, 1);
    assert_eq!(rig.tally().unchecked, 1);
    let summary = rig.tally().summary("Miner").unwrap();
    assert!(summary.contains("1 mismatched"), "{summary}");
}

/// FU1 item 3 — an honest place-then-break of one cell in one input (and a
/// break-then-place): the placement is consumed from the shadow and the
/// break is yielded. The cell's `mined` tag belongs to the break alone; it
/// used to make the placement an unchecked "tagged fill".
#[test]
fn a_place_and_a_break_of_one_cell_in_one_input_consume_and_yield_in_order() {
    let mut rig = Rig::dedicated("place-break");
    let stone = Item::Block(block::STONE);
    let cobble = Item::Block(block::COBBLESTONE);
    let wood = Some(pick(ToolMaterial::Wood));
    rig.hs.server.players[rig.slot].inventory.set_slot(0, Some(ItemStack::new_block(block::STONE, 3)));

    // Place, then break, the air cell above the floor.
    rig.send(Some(&stone), &[(ABOVE, block::STONE), (ABOVE, block::AIR)], &[mined(ABOVE, wood)]);
    rig.tick();
    assert_eq!(rig.world().get_block(ABOVE.0, ABOVE.1, ABOVE.2), block::AIR);
    assert_eq!(rig.shadow_count(&stone), 2, "the placement was consumed");
    assert_eq!(rig.granted(&cobble), 1, "the break was yielded");
    assert_eq!((rig.tally().matched, rig.tally().breaks), (1, 1));

    // Break, then place, the floor cell.
    rig.send(Some(&stone), &[(FLOOR, block::AIR), (FLOOR, block::STONE)], &[mined(FLOOR, wood)]);
    rig.tick();
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::STONE);
    assert_eq!(rig.granted(&cobble), 2, "the break was yielded");
    assert_eq!(rig.shadow_count(&stone), 1, "and the placement consumed");
    assert_eq!((rig.tally().matched, rig.tally().breaks), (2, 2));
    assert!(rig.world().is_placed(FLOOR.0, FLOOR.1, FLOOR.2), "the refill is player-placed");
}

/// FU1 (C1 verify N4) — one tag yields one break. A later emptying edit of
/// the same cell (the Eraser on what was placed there, a bucket, the
/// client's own piston) never takes it, in the same input or the next; each
/// tag the joiner sends yields exactly once.
#[test]
fn each_mined_tag_yields_exactly_once_and_never_for_another_edit_of_its_cell() {
    let mut rig = Rig::dedicated("tag-once");
    let cobble = Item::Block(block::COBBLESTONE);
    let wood = Some(pick(ToolMaterial::Wood));
    // Mine the floor cell, refill it, empty it again — one tag, in one input.
    rig.send(
        None,
        &[(FLOOR, block::AIR), (FLOOR, block::STONE), (FLOOR, block::AIR)],
        &[mined(FLOOR, wood)],
    );
    rig.tick();
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::AIR);
    assert_eq!(rig.granted(&cobble), 1, "the tag yielded its own break, and only it");
    assert_eq!(rig.tally().breaks, 1);
    // The next tick: refilled and emptied again, untagged — nothing.
    rig.send(None, &[(FLOOR, block::STONE)], &[]);
    rig.tick();
    rig.send(None, &[(FLOOR, block::AIR)], &[]);
    rig.tick();
    assert_eq!(rig.granted(&cobble), 1, "no tag of its own: nothing");
    // A second mine with its own tag yields once more.
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::STONE);
    rig.mine(FLOOR, block::AIR, wood);
    assert_eq!(rig.granted(&cobble), 2, "each tag once");
    assert_eq!(rig.tally().breaks, 2);
    // Two mines of one cell in one input, each with its tag: both yield.
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::STONE);
    rig.send(
        None,
        &[(FLOOR, block::AIR), (FLOOR, block::STONE), (FLOOR, block::AIR)],
        &[mined(FLOOR, wood), mined(FLOOR, wood)],
    );
    rig.tick();
    assert_eq!(rig.granted(&cobble), 4, "two tags, two breaks");
    // Lava dug up by hand (its tag yields nothing — lava is no item) and the
    // stone poured over it mined with a pickaxe, in one input: each edit
    // takes its own tag, so the mine has the pickaxe and yields.
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::LAVA);
    rig.send(
        None,
        &[(FLOOR, block::AIR), (FLOOR, block::STONE), (FLOOR, block::AIR)],
        &[mined(FLOOR, None), mined(FLOOR, wood)],
    );
    rig.tick();
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::AIR);
    assert_eq!(rig.granted(&cobble), 5, "the mine used its own (pickaxe) tag, not the lava's bare hand");
    assert_eq!(rig.granted(&Item::Block(block::LAVA)), 0, "and the lava yielded nothing");
}

/// C1 review LOW-7 / LOW-3 — a tagged edit the server refuses (out of reach,
/// inside someone else's plot) yields nothing, and its tag doesn't linger: a
/// later untagged emptying of the same cell yields nothing either. (FU3 — an
/// edit past the per-tick budget is no longer refused: it waits, and its tag
/// with it: `edits_past_the_budget_wait_and_each_tag_yields_once`.)
#[test]
fn a_refused_tagged_edit_yields_nothing_and_its_tag_does_not_linger() {
    let mut rig = Rig::dedicated("refused-tag");
    let wood = Some(pick(ToolMaterial::Wood));
    let stone_at = |rig: &mut Rig, c: (i32, i32, i32)| rig.world().get_block(c.0, c.1, c.2) == block::STONE;

    // Out of reach.
    let far = (60, 79, 40);
    rig.loaded(far);
    rig.world().set_block(far.0, far.1, far.2, block::STONE);
    rig.mine(far, block::AIR, wood);
    assert!(stone_at(&mut rig, far), "out of reach: refused");
    assert!(rig.grants.is_empty());

    // Inside a plot the joiner doesn't own.
    let fenced = (40, 79, 41);
    rig.hs.server.world.plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        fenced.0,
        fenced.1 - 5,
        fenced.2,
    ));
    rig.mine(fenced, block::AIR, wood);
    assert!(stone_at(&mut rig, fenced), "a foreign plot: refused");
    assert!(rig.grants.is_empty());
    rig.hs.server.world.plots.clear();

    // A refused mine and a later accepted emptying of its cell in ONE input
    // (the refusal and the acceptance a tick apart, the second edit waiting
    // past the budget): the refused edit's tag is spent with it, so the
    // later, untagged edit takes nothing.
    rig.hs.server.world.plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        fenced.0,
        fenced.1 - 5,
        fenced.2,
    ));
    let filler: Vec<((i32, i32, i32), BlockId)> = (0..3).map(|k| ((39 + k, 80, 39), block::STONE)).collect();
    let mut edits = filler.clone();
    edits.push((fenced, block::AIR)); // the 4th: refused (the plot)
    edits.push((fenced, block::AIR)); // the 5th: waits a tick
    rig.send(None, &edits, &[mined(fenced, wood)]);
    rig.tick();
    assert!(stone_at(&mut rig, fenced), "the mine in the plot: refused");
    rig.hs.server.world.plots.clear();
    rig.tick();
    assert_eq!(rig.world().get_block(fenced.0, fenced.1, fenced.2), block::AIR, "the waiting edit: accepted");
    assert!(rig.grants.is_empty(), "but it never took the refused mine's tag");

    // None of those tags lingers: cells emptied untagged yield nothing.
    rig.world().set_block(fenced.0, fenced.1, fenced.2, block::STONE);
    rig.send(None, &[(far, block::STONE), (fenced, block::AIR)], &[]);
    rig.tick();
    assert_eq!(rig.world().get_block(fenced.0, fenced.1, fenced.2), block::AIR, "accepted now");
    assert!(rig.grants.is_empty(), "but nothing yielded");
    assert_eq!(rig.tally().breaks, 0);
}

/// FU3 (FU1 verify N3) — a joiner's edits past the per-tick budget
/// (`MAX_BLOCK_CHANGES_PER_TICK` = 4) wait and go first next tick, in the
/// order they were made, each with its own `mined` tag; nothing is sent back
/// for the budget. Twelve mines in one input apply across three ticks and
/// yield exactly once each — on a dedicated server and on a lending host.
#[test]
fn edits_past_the_budget_wait_and_each_tag_yields_once() {
    for mut rig in [Rig::dedicated("edit-queue"), Rig::lent("edit-queue")] {
        let wood = Some(pick(ToolMaterial::Wood));
        let cobble = Item::Block(block::COBBLESTONE);
        let cells: Vec<(i32, i32, i32)> =
            (0..12).map(|k| (38 + k % 6, 79, 39 + k / 6)).collect();
        rig.hs.hold_column_for_test(rig.slot, crate::chunk_stream::column_of(rig.at));
        let edits: Vec<_> = cells.iter().map(|&c| (c, block::AIR)).collect();
        let tags: Vec<_> = cells.iter().map(|&c| mined(c, wood)).collect();
        rig.send(None, &edits, &tags);
        let air = |rig: &mut Rig| -> Vec<bool> {
            cells.clone().into_iter().map(|c| rig.world().get_block(c.0, c.1, c.2) == block::AIR).collect()
        };
        for (tick, applied) in [(1, 4), (2, 8), (3, 12)] {
            rig.tick();
            let expect: Vec<bool> = (0..12).map(|k| k < applied).collect();
            assert_eq!(air(&mut rig), expect, "tick {tick}: the first {applied}, in the order made");
            assert_eq!(rig.granted(&cobble), applied as u32, "tick {tick}: one yield per applied mine");
        }
        rig.tick();
        assert_eq!(rig.granted(&cobble), 12, "each tag exactly once");
        assert_eq!(rig.tally().breaks, 12);
        assert!(
            !rig.changes.iter().any(|c| c.new_block == block::STONE && cells.contains(&(c.x, c.y, c.z))),
            "nothing was sent back for the budget"
        );
    }
}

/// FU3 (FU1 verify N3) — a joiner's campfire action is one edit: the server
/// derives the smoke from its own campfire (`campfire::on_block_edit`). A
/// joiner breaking a lit, smoky campfire leaves no smoke in the shared world
/// (the 4-edit budget used to refuse three of the seven cells the client
/// sent, stranding them); lighting a smoky one raises the pillar for
/// everyone, and breaking it again clears it — on a dedicated server and on
/// a lending host.
#[test]
fn a_joiners_campfire_break_or_light_leaves_the_smoke_to_the_server() {
    for mut rig in [Rig::dedicated("campfire"), Rig::lent("campfire")] {
        let fire = (42, 80, 40);
        let pillar: Vec<(i32, i32, i32)> =
            (1..=crate::campfire::SMOKE_PILLAR_HEIGHT).map(|dy| (fire.0, fire.1 + dy, fire.2)).collect();
        rig.hs.hold_column_for_test(rig.slot, crate::chunk_stream::column_of(rig.at));
        let smoke = |rig: &mut Rig| {
            pillar.iter().filter(|c| rig.world().get_block(c.0, c.1, c.2) == block::CAMPFIRE_SMOKE).count()
        };
        // A lit, smoky campfire with its pillar standing.
        {
            let world = rig.world();
            for c in &pillar {
                world.set_block(c.0, c.1, c.2, block::AIR);
            }
            world.set_block(fire.0, fire.1, fire.2, block::CAMPFIRE);
            let cf = world.campfire_at_mut_or_default(fire);
            cf.fuel_ticks = 2_000;
            cf.smoke_ticks = 1_200;
            crate::campfire::place_smoke_pillar(world, fire.0, fire.1, fire.2);
        }
        assert_eq!(smoke(&mut rig), 6);

        // Broken: the joiner sends the one edit, as its client now does.
        rig.send(None, &[(fire, block::AIR)], &[]);
        rig.tick();
        assert_eq!(rig.world().get_block(fire.0, fire.1, fire.2), block::AIR);
        assert_eq!(smoke(&mut rig), 0, "no smoke left floating in the shared world");
        assert!(rig.world().campfire_at(fire).is_none(), "nor an orphan campfire");
        for c in &pillar {
            assert!(
                rig.changes.iter().any(|bc| (bc.x, bc.y, bc.z) == *c && bc.new_block == block::AIR),
                "the cleared cell {c:?} reached the joiner"
            );
        }

        // An unlit, smoky campfire, lit by the joiner: the pillar rises.
        {
            let world = rig.world();
            world.set_block(fire.0, fire.1, fire.2, block::CAMPFIRE_UNLIT);
            let cf = world.campfire_at_mut_or_default(fire);
            cf.fuel_ticks = 2_000;
            cf.smoke_ticks = 1_200;
        }
        rig.changes.clear();
        rig.send(None, &[(fire, block::CAMPFIRE)], &[]);
        rig.tick();
        assert_eq!(rig.world().get_block(fire.0, fire.1, fire.2), block::CAMPFIRE);
        assert_eq!(smoke(&mut rig), 6, "lit: the server raised the pillar");
        assert!(
            rig.changes.iter().filter(|bc| bc.new_block == block::CAMPFIRE_SMOKE).count() >= 6,
            "and everyone, the joiner included, is sent it"
        );
        // …then broken again: nothing left.
        rig.send(None, &[(fire, block::AIR)], &[]);
        rig.tick();
        assert_eq!(smoke(&mut rig), 0, "lit then broken: no smoke left");
    }
}

/// FU3 / FU4a (FU3 verify M2) — the edit queue's hard cap
/// (`edit_queue::MAX_DEFERRED_EDITS`), which no honest client reaches: an edit
/// past it is dropped, and nothing is sent back for it. (FU3 sent it back,
/// like any refusal: a flood at the cap then cost the host a world lookup and
/// a broadcast per edit.)
#[test]
fn an_edit_past_the_edit_queues_cap_is_dropped_and_nothing_is_sent_back() {
    let mut rig = Rig::dedicated("edit-cap");
    rig.hs.hold_column_for_test(rig.slot, crate::chunk_stream::column_of(rig.at));
    // Four processed now, MAX_DEFERRED_EDITS queued: all aimed out of reach
    // (refused when their turn comes). Sent in packets under the wire cap.
    let far = (60, 79, 40);
    rig.loaded(far);
    let mut left = 4 + crate::edit_queue::MAX_DEFERRED_EDITS;
    while left > 0 {
        let n = left.min(4_000);
        rig.send(None, &vec![(far, block::STONE); n], &[]);
        left -= n;
    }
    // The one past the cap: a placement in reach, beside the joiner.
    rig.send(None, &[(ABOVE, block::STONE)], &[]);
    rig.tick();
    assert_eq!(rig.hs.edit_queue_len_for_test(rig.slot), crate::edit_queue::MAX_DEFERRED_EDITS, "full");
    for _ in 0..4 {
        rig.tick();
    }
    assert_eq!(rig.world().get_block(ABOVE.0, ABOVE.1, ABOVE.2), block::AIR, "the edit past the cap is dropped");
    assert!(
        !rig.changes.iter().any(|c| (c.x, c.y, c.z) == ABOVE),
        "and nothing is sent back for it"
    );
}

/// FU4a (FU3 verify M2) — every refusal that sends the real block back is
/// capped at `MAX_SEND_BACKS_PER_CLIENT_PER_TICK` (64) per client per tick;
/// past it the refused edit is dropped silently. A joiner that dies with
/// 1,000 edits waiting has at most 64 of them sent back that tick (FU3 sent
/// back all of them at once), and the rest are gone, not sent later.
#[test]
fn a_dead_joiners_waiting_edits_send_back_at_most_64_in_a_tick() {
    let mut rig = Rig::dedicated("dead-send-back-cap");
    rig.hs.hold_column_for_test(rig.slot, crate::chunk_stream::column_of(rig.at));
    // 1,004 placements in distinct cells of the joiner's own column (the
    // outbox merges changes of one cell, so each must be its own): four are
    // processed now, 1,000 wait.
    let cells: Vec<(i32, i32, i32)> =
        (0..1_004).map(|k| (32 + k % 16, 90 + k / 256, 32 + (k / 16) % 16)).collect();
    let edits: Vec<_> = cells.iter().map(|&c| (c, block::STONE)).collect();
    rig.send(None, &edits, &[]);
    rig.tick();
    assert_eq!(rig.hs.edit_queue_len_for_test(rig.slot), 1_000, "1,000 wait");
    assert!(rig.hs.server.report_player_death(rig.slot), "the joiner dies");
    rig.changes.clear();
    rig.tick();
    let sent_back = |rig: &Rig| rig.changes.iter().filter(|c| cells[4..].contains(&(c.x, c.y, c.z))).count();
    assert_eq!(sent_back(&rig), 64, "64 of the waiting edits are sent back that tick, no more");
    assert_eq!(rig.hs.edit_queue_len_for_test(rig.slot), 0, "the rest are dropped");
    rig.changes.clear();
    rig.tick();
    assert_eq!(sent_back(&rig), 0, "not sent back later");
    for c in &cells[4..] {
        assert_ne!(rig.world().get_block(c.0, c.1, c.2), block::STONE, "a dead joiner's waiting edit never lands");
    }
}

/// FU4a (FU3 verify L2) — a tag is its own edit's from the moment the input
/// is read: a crop harvest refused (a foreign plot) takes its tag with it,
/// and a later edit of the same cell, waiting past the budget and accepted a
/// tick later, yields nothing. (FU3 spent a refused edit's tag only when the
/// edit emptied its cell, so the harvest's tag stayed for the later emptying
/// edit, which took it and yielded the whole crop.)
#[test]
fn a_refused_harvests_tag_goes_with_it_and_a_later_edit_of_its_cell_yields_nothing() {
    let mut rig = Rig::dedicated("refused-harvest");
    let fenced = (40, 79, 41);
    rig.world().set_block(fenced.0, fenced.1, fenced.2, block::WHEAT_STAGE_3);
    rig.hs.server.world.plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        fenced.0,
        fenced.1 - 5,
        fenced.2,
    ));
    let filler: Vec<((i32, i32, i32), BlockId)> = (0..3).map(|k| ((39 + k, 80, 39), block::STONE)).collect();
    let mut edits = filler;
    edits.push((fenced, block::TILLED_SOIL)); // the 4th: the harvest, refused (the plot)
    edits.push((fenced, block::AIR)); // the 5th: waits a tick
    rig.send(None, &edits, &[mined(fenced, None)]);
    rig.tick();
    assert_eq!(rig.world().get_block(fenced.0, fenced.1, fenced.2), block::WHEAT_STAGE_3, "the harvest: refused");
    rig.hs.server.world.plots.clear();
    rig.tick();
    assert_eq!(rig.world().get_block(fenced.0, fenced.1, fenced.2), block::AIR, "the waiting edit: accepted");
    assert!(rig.grants.is_empty(), "but it never took the refused harvest's tag: {:?}", rig.grants);
    assert_eq!(rig.tally().breaks, 0);
}

/// FU1 (C1 verify N1) — an accepted edit that leaves the block as it was
/// (a modified client "replacing" natural deepslate with itself, or a
/// meta-only toggle) puts nothing in the cell: it stays natural.
#[test]
fn an_edit_that_leaves_the_block_unchanged_leaves_a_natural_cell_natural() {
    let mut rig = Rig::dedicated("no-op-edit");
    rig.world().set_block(FLOOR.0, FLOOR.1, FLOOR.2, block::PURE_DEEPSLATE);
    assert!(!rig.world().is_placed(FLOOR.0, FLOOR.1, FLOOR.2));
    let deepslate = Item::Block(block::PURE_DEEPSLATE);
    rig.send(Some(&deepslate), &[(FLOOR, block::PURE_DEEPSLATE)], &[]);
    rig.tick();
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::PURE_DEEPSLATE);
    assert!(!rig.world().is_placed(FLOOR.0, FLOOR.1, FLOOR.2), "still natural: its Satori roll stands");
    // A real placement in that cell is still flagged.
    rig.send(Some(&deepslate), &[(FLOOR, block::AIR)], &[]);
    rig.tick();
    rig.send(Some(&deepslate), &[(FLOOR, block::PURE_DEEPSLATE)], &[]);
    rig.tick();
    assert!(rig.world().is_placed(FLOOR.0, FLOOR.1, FLOOR.2), "a block put there is player-placed");
}

// ── C3a-2b: each edit carries its own hotbar slot; the server wears tools ───

/// C3a-2b — a placement is charged to the hotbar slot of the input that
/// carried it, not the slot the joiner scrolled to in a later input before
/// the edit was processed. Five placements from slot 2: four fit this tick's
/// budget, the fifth waits in the FIFO while the next input (scrolled to
/// slot 5, no edits) is read in the same tick. All five are charged to slot
/// 2 — the waiting one used to be charged to slot 5.
#[test]
fn a_waiting_placement_is_charged_to_the_slot_it_was_made_at_not_the_one_scrolled_to() {
    for mut rig in [Rig::dedicated("slot-skew"), Rig::lent("slot-skew")] {
        let stone = Item::Block(block::STONE);
        let inv = &mut rig.hs.server.players[rig.slot].inventory;
        inv.set_slot(2, Some(ItemStack::new_block(block::STONE, 5)));
        // Not stone: the last of a stack would refill from it.
        inv.set_slot(5, Some(ItemStack::new_block(block::DIRT, 5)));
        rig.hs.hold_column_for_test(rig.slot, crate::chunk_stream::column_of(rig.at));
        let cells: Vec<(i32, i32, i32)> = (0..5).map(|k| (38 + k, 80, 42)).collect();
        let edits: Vec<_> = cells.iter().map(|&c| (c, block::STONE)).collect();
        rig.send_at(2, Some(&stone), &edits, &[]);
        rig.send_at(5, Some(&Item::Block(block::DIRT)), &[], &[]);
        rig.tick();
        rig.tick();
        for &c in &cells {
            assert_eq!(rig.world().get_block(c.0, c.1, c.2), block::STONE, "{c:?} placed");
        }
        let inv = &rig.hs.server.players[rig.slot].inventory;
        assert!(inv.slot(2).is_none(), "all five charged to slot 2 (the slot they were made at)");
        assert_eq!(inv.slot(5).map(|s| s.count), Some(5), "slot 5 untouched");
        assert_eq!(rig.tally().matched, 5);
        assert_eq!(rig.tally().mismatched, 0);
    }
}

/// One iron pickaxe on slot 0 of the shadow, `durability` left on it.
fn give_pick(rig: &mut Rig, durability: u16) {
    let mut t = pick(ToolMaterial::Iron);
    t.durability = durability;
    rig.hs.server.players[rig.slot].inventory.set_slot(0, Some(ItemStack { item: Item::Tool(t), count: 1 }));
}

fn shadow_pick(rig: &Rig) -> Option<Tool> {
    match rig.hs.server.players[rig.slot].inventory.slot(0).map(|s| &s.item) {
        Some(Item::Tool(t)) => Some(*t),
        _ => None,
    }
}

/// C3a-2b — 1,000 breaks of stone with an iron pickaxe wear the shadow's
/// pickaxe exactly as the client's wears over the same breaks
/// (`Inventory::use_hotbar_tool`, its break arm's call), on a dedicated
/// server and a lending host. Nothing is a wear mismatch.
#[test]
fn a_thousand_breaks_wear_the_shadows_pickaxe_as_the_clients_wears() {
    for mut rig in [Rig::dedicated("wear-1000"), Rig::lent("wear-1000")] {
        // More than the iron pickaxe's own durability, so the wear is not
        // cut short by it breaking (that is the next test).
        give_pick(&mut rig, 5_000);
        let mut client = crate::inventory::Inventory::new();
        client.set_slot(0, Some(ItemStack { item: Item::Tool(shadow_pick(&rig).unwrap()), count: 1 }));
        let cells = [(41, 79, 40), (42, 79, 40), (41, 79, 41), (42, 79, 41)];
        rig.hs.hold_column_for_test(rig.slot, crate::chunk_stream::column_of(rig.at));
        for _ in 0..250 {
            for &c in &cells {
                rig.world().set_block(c.0, c.1, c.2, block::STONE);
            }
            let tool = shadow_pick(&rig).unwrap();
            let edits: Vec<_> = cells.iter().map(|&c| (c, block::AIR)).collect();
            let tags: Vec<_> = cells.iter().map(|&c| mined(c, Some(tool))).collect();
            rig.send(Some(&Item::Tool(tool)), &edits, &tags);
            rig.tick();
            for _ in 0..4 {
                client.use_hotbar_tool(0);
            }
        }
        assert_eq!(rig.tally().breaks, 1000);
        assert_eq!(rig.tally().wear_mismatch, 0);
        let Some(Item::Tool(want)) = client.slot(0).map(|s| s.item.clone()) else { panic!("client pickaxe gone") };
        assert_eq!(want.durability, 4_000);
        assert_eq!(shadow_pick(&rig), Some(want), "the same wear as the client's");
    }
}

/// C3a-2b — a tool that wears out on the server is gone from the shadow, as
/// on the client; the break that used it up still yields, and a break after
/// it, mined with a tool the shadow no longer has, is a wear mismatch.
#[test]
fn a_tool_the_server_wears_out_is_gone_from_the_shadow() {
    let mut rig = Rig::dedicated("wear-break");
    give_pick(&mut rig, 2);
    let cobble = Item::Block(block::COBBLESTONE);
    let tool = shadow_pick(&rig).unwrap();
    let mut client = crate::inventory::Inventory::new();
    client.set_slot(0, Some(ItemStack { item: Item::Tool(tool), count: 1 }));
    let cells = [FLOOR, (42, 79, 40), (41, 79, 41)];
    for (n, &c) in cells.iter().enumerate() {
        rig.mine(c, block::AIR, Some(tool));
        let info = client.use_hotbar_tool(0);
        assert_eq!(rig.granted(&cobble), (n + 1) as u32, "break {n} still yields");
        match n {
            0 => assert_eq!(shadow_pick(&rig).map(|t| t.durability), Some(1)),
            _ => assert!(shadow_pick(&rig).is_none(), "break {n}: the pickaxe is gone"),
        }
        assert_eq!(shadow_pick(&rig).is_none(), client.slot(0).is_none(), "break {n}: as on the client");
        assert_eq!(info.is_some_and(|i| i.just_broke), n == 1);
    }
    assert_eq!(rig.tally().wear_mismatch, 1, "the third break had no pickaxe to wear");
}

/// C3a-2b — a slot that holds a different tool (or nothing) wears nothing
/// and is tallied, never refused: the break is still accepted and yields.
#[test]
fn a_break_whose_tool_the_shadows_slot_lacks_is_tallied_and_accepted() {
    let mut rig = Rig::dedicated("wear-mismatch");
    // Slot 0 holds a wooden pickaxe; the joiner says it mined with iron.
    rig.hs.server.players[rig.slot]
        .inventory
        .set_slot(0, Some(ItemStack { item: Item::Tool(pick(ToolMaterial::Wood)), count: 1 }));
    rig.mine(FLOOR, block::AIR, Some(pick(ToolMaterial::Iron)));
    assert_eq!(rig.world().get_block(FLOOR.0, FLOOR.1, FLOOR.2), block::AIR, "accepted");
    assert_eq!(rig.tally().wear_mismatch, 1);
    assert_eq!(rig.tally().breaks, 1);
    assert_eq!(shadow_pick_material(&rig), Some(ToolMaterial::Wood));
    assert_eq!(shadow_pick(&rig).map(|t| t.durability), Some(pick(ToolMaterial::Wood).durability), "unworn");
    // A bare-hand break wears nothing and is no mismatch.
    rig.mine((42, 79, 40), block::AIR, None);
    assert_eq!(rig.tally().wear_mismatch, 1);
}

fn shadow_pick_material(rig: &Rig) -> Option<ToolMaterial> {
    shadow_pick(rig).map(|t| t.material)
}
