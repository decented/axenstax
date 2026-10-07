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
        let mut rig = Rig { hs, host, client, slot, at, input: 0, grants: Vec::new() };
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
            if let Some((protocol::PacketType::InventoryGrant, payload)) = protocol::deserialize_header(&pkt) {
                self.grants.push(protocol::safe_deserialize(payload).unwrap());
            }
        }
    }

    /// One input from the joiner: `edits`, the cells it says it mined, and
    /// what its hand holds (hotbar slot 0).
    fn send(&mut self, held: Option<&Item>, edits: &[((i32, i32, i32), BlockId)], mined: &[MinedBlock]) {
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
            hotbar_slot: Some(0),
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
