//! C2b (2026-10-07, protocol v74) — a joiner's Q-drops are mirrored on the
//! server, and a grant the server's shadow of its inventory has no room for
//! still reaches the client whole (C2b-fix). Its crafting was mirrored here
//! too until C3a-2a (v75) made the craft a window op
//! (`test_integration::window_ops`); an `ItemAction::Craft` is now ignored.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport — a
//! dedicated server, or a lending host where the world and ECS are the
//! host's — and reads what the server's shadow holds, what lies on its
//! ground and what the joiner is sent.
//!
//! The client half (the claims gate on a Q-drop or a craft click, the owed
//! payment from the crafting grid and the cursor) needs a GPU-backed
//! `GameState`; its rules are the pure `joiner_actions` functions the
//! client sites call (`can_spend`, `may_craft`, `take_owed_held`),
//! unit-tested there.

use glam::Vec3;

use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::entity::{ItemEntity, Position};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::item::{Item, ItemStack, MaterialId};
use crate::item_actions::DROP_INTERVAL_TICKS;
use crate::protocol::{self, InventoryGrantPacket, ItemAction};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};

use super::joiner_authority::join_guest;
use super::joiners_act::floor_and_stand;
use super::lent_world::{join_guest_lent, start_lent};

/// One joiner.
struct Joiner {
    client: ChannelClientTransport,
    slot: usize,
    seq: u32,
    grants: Vec<InventoryGrantPacket>,
}

/// Joiners standing on a stone floor round (40, 80, 40), on a dedicated
/// server or a lending host (`host`: the host client's world and ECS, lent
/// to the server each tick).
struct Rig {
    hs: HostedServer,
    host: Option<OwnedSimParts>,
    joiners: Vec<Joiner>,
    at: Vec3,
}

impl Rig {
    fn dedicated(tag: &str, joiners: usize) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("joiner-craft-drop-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let joined: Vec<_> = (0..joiners).map(|n| join_guest(&mut hs, &format!("Crafter{n}"))).collect();
        Self::stand(hs, None, joined)
    }

    fn lent(tag: &str) -> Self {
        let (mut hs, mut host) = start_lent(&format!("joiner-craft-drop-{tag}"));
        let joined = join_guest_lent(&mut hs, &mut host, "Crafter0");
        Self::stand(hs, Some(host), vec![joined])
    }

    fn stand(mut hs: HostedServer, mut host: Option<OwnedSimParts>, joined: Vec<(ChannelClientTransport, usize)>) -> Self {
        hs.server.column_streamer = None;
        hs.server.column_refill_per_tick = 0;
        hs.server.difficulty = crate::survival::Difficulty::Peaceful;
        let world = match host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut hs.server.world,
        };
        let mut world = std::mem::replace(world, crate::world::World::new());
        let mut at = Vec3::ZERO;
        for (_, slot) in &joined {
            at = floor_and_stand(&mut world, &mut hs, *slot);
        }
        match host.as_mut() {
            Some(h) => h.world = world,
            None => hs.server.world = world,
        }
        let ecs = match host.as_mut() {
            Some(h) => &mut h.ecs,
            None => &mut hs.server.ecs,
        };
        crate::remote_mobs::purge_private_mobs(ecs);
        clear_items(ecs);
        let joiners =
            joined.into_iter().map(|(client, slot)| Joiner { client, slot, seq: 0, grants: Vec::new() }).collect();
        let mut rig = Rig { hs, host, joiners, at };
        rig.tick();
        for j in &mut rig.joiners {
            j.grants.clear();
        }
        rig
    }

    fn tick(&mut self) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        for j in &mut self.joiners {
            while let Some(pkt) = j.client.try_recv_from_server() {
                if let Some((protocol::PacketType::InventoryGrant, payload)) = protocol::deserialize_header(&pkt) {
                    j.grants.push(protocol::safe_deserialize(payload).unwrap());
                }
            }
        }
    }

    fn ticks(&mut self, n: u32) {
        for _ in 0..n {
            self.tick();
        }
    }

    fn world(&mut self) -> &mut crate::world::World {
        match self.host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut self.hs.server.world,
        }
    }

    fn ecs(&self) -> &hecs::World {
        match &self.host {
            Some(h) => &h.ecs,
            None => &self.hs.server.ecs,
        }
    }

    fn sp(&mut self, j: usize) -> &mut crate::server::ServerPlayer {
        let slot = self.joiners[j].slot;
        &mut self.hs.server.players[slot]
    }

    fn send(&mut self, j: usize, action: ItemAction) {
        let joiner = &mut self.joiners[j];
        joiner.seq += 1;
        let pkt = protocol::ItemActionPacket { seq: joiner.seq, action };
        joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    }

    /// A plain `ClientInput` for joiner `j`, standing where it stands, with
    /// sequence `tick`.
    fn send_input(&mut self, j: usize, tick: u64) {
        let input = protocol::InputPacket {
            tick,
            x: self.at.x,
            y: self.at.y,
            z: self.at.z,
            health: 20.0,
            ..Default::default()
        };
        self.joiners[j].client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    fn craft(&mut self, j: usize, grid: [(u8, u16); 9], table: Option<[i32; 3]>) {
        self.send(j, ItemAction::Craft { grid, table });
    }

    fn drop(&mut self, j: usize, item: &Item) {
        let (held_kind, held_id) = crate::inventory::item_to_ref(item).to_wire();
        self.send(
            j,
            ItemAction::Drop { hotbar_slot: 0, held_kind, held_id, held_full: crate::inventory::item_to_wire_full(item) },
        );
    }

    /// Put `stack` in joiner `j`'s shadow, slot `slot`.
    fn give(&mut self, j: usize, slot: usize, stack: ItemStack) {
        self.sp(j).inventory.set_slot(slot, Some(stack));
    }

    fn shadow_count(&self, j: usize, item: &Item) -> u32 {
        self.hs.server.players[self.joiners[j].slot]
            .inventory
            .slots_iter()
            .flatten()
            .filter(|s| same(&s.item, item))
            .map(|s| u32::from(s.count))
            .sum()
    }

    fn tally(&self, j: usize) -> crate::joiner_inventory::PossessionTally {
        self.hs.server.players[self.joiners[j].slot].possession
    }

    /// Every ground item: where it is, what it holds, its dropper.
    fn items(&self) -> Vec<(Vec3, ItemStack, Option<u8>)> {
        self.ecs()
            .query::<(&Position, &ItemEntity)>()
            .iter()
            .map(|(_, (p, it))| (p.0, it.stack.clone(), it.dropper))
            .collect()
    }

    fn granted(&self, j: usize, item: &Item) -> u32 {
        self.joiners[j]
            .grants
            .iter()
            .filter(|g| {
                let got = crate::inventory::item_from_wire_full(&g.full_item)
                    .or_else(|| crate::inventory::item_from_ref(g.item_kind, g.item_id, &crate::block::BlockRegistry::new()));
                got.is_some_and(|got| same(&got, item))
            })
            .map(|g| u32::from(g.count))
            .sum()
    }
}

fn clear_items(ecs: &mut hecs::World) {
    let ids: Vec<hecs::Entity> = ecs.query::<&ItemEntity>().iter().map(|(e, _)| e).collect();
    for e in ids {
        let _ = ecs.despawn(e);
    }
}

/// The same item for counting: a tool by type and material.
fn same(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::Tool(a), Item::Tool(b)) => (a.tool_type, a.material) == (b.tool_type, b.material),
        (a, b) => a == b,
    }
}

fn pair(item: &Item) -> (u8, u16) {
    crate::inventory::item_to_ref(item).to_wire()
}

fn grid_of(cells: &[(usize, Item)]) -> [(u8, u16); 9] {
    let mut g = [protocol::ItemRef::Empty.to_wire(); 9];
    for (i, item) in cells {
        g[*i] = pair(item);
    }
    g
}

fn planks() -> Item {
    Item::Block(block::OAK_PLANKS)
}

fn cobble() -> Item {
    Item::Block(block::COBBLESTONE)
}

fn stick() -> Item {
    Item::Material(MaterialId::Stick)
}

/// 4 planks, 2×2 → a crafting table.
fn table_recipe() -> [(u8, u16); 9] {
    grid_of(&[(0, planks()), (1, planks()), (3, planks()), (4, planks())])
}

// ─── Crafting (C3a-2a: retired) ────────────────────────────────────────────

/// C3a-2a (v75) — a craft is the window's result click, mirrored as a window
/// op (`test_integration::window_ops`). An `ItemAction::Craft` (unused since
/// v75; the variant stays, append-only) is ignored and tallied: nothing in
/// the shadow changes and nothing is answered. Was C2b's
/// `a_joiners_2x2_craft_is_mirrored_on_the_shadow`.
#[test]
fn a_v75_server_ignores_a_craft_and_tallies_it() {
    let mut rig = Rig::dedicated("craft-ignored", 1);
    rig.give(0, 3, ItemStack::new_block(block::OAK_PLANKS, 5));
    rig.craft(0, table_recipe(), None);
    rig.tick();
    assert_eq!(rig.shadow_count(0, &planks()), 5, "nothing taken");
    assert_eq!(rig.shadow_count(0, &Item::Block(block::CRAFTING_TABLE)), 0, "nothing added");
    assert_eq!(rig.tally(0).crafts_ignored, 1);
    assert_eq!(rig.tally(0).mismatched, 0);
    assert!(rig.joiners[0].grants.is_empty());
}

/// An ignored craft still counts against the item-action budget: one past it
/// waits for the next tick, as before. Was C2b's
/// `crafts_count_against_the_item_action_budget`.
#[test]
fn ignored_crafts_still_count_against_the_item_action_budget() {
    let mut rig = Rig::dedicated("craft-budget", 1);
    for _ in 0..6 {
        rig.craft(0, table_recipe(), None);
    }
    rig.tick();
    assert_eq!(rig.tally(0).crafts_ignored, 4, "MAX_ITEM_ACTIONS_PER_TICK this tick");
    rig.tick();
    assert_eq!(rig.tally(0).crafts_ignored, 6, "the rest the next");
}

// ─── Dropping (decision 2) ─────────────────────────────────────────────────

/// The item lands, and its position settles.
fn settle(rig: &mut Rig) -> Vec3 {
    rig.ticks(12);
    let items = rig.items();
    assert_eq!(items.len(), 1, "one ground item");
    items[0].0
}

#[test]
fn a_joiners_drop_is_a_real_item_another_joiner_can_pick_up() {
    let mut rig = Rig::dedicated("drop-other", 2);
    let stick_item = stick();
    rig.give(0, 0, ItemStack::new_material(MaterialId::Stick, 3));
    rig.sp(1).player.pos = Vec3::new(34.5, 80.0, 34.5); // on the floor, out of reach
    rig.drop(0, &stick_item);
    rig.tick();
    let items = rig.items();
    assert_eq!(items.len(), 1, "the drop is on the server's ground");
    assert_eq!(items[0].1, ItemStack::new_material(MaterialId::Stick, 1));
    assert_eq!(items[0].2, Some(rig.joiners[0].slot as u8), "the dropper is the joiner's server slot");
    let eye = rig.hs.server.players[rig.joiners[0].slot].player.eye_pos();
    assert!((items[0].0 - eye).length() < 1.0, "thrown from the server body's eye");
    assert_eq!(rig.shadow_count(0, &stick_item), 2, "the shadow is debited");
    assert_eq!(rig.tally(0).drops, 1);

    // The other joiner walks onto it inside the dropper's delay and picks
    // it up at once.
    let at = settle(&mut rig);
    rig.sp(1).player.pos = at;
    rig.tick();
    assert!(rig.items().is_empty(), "picked up");
    assert_eq!(rig.granted(1, &stick_item), 1, "granted to the other joiner");
    assert_eq!(rig.shadow_count(1, &stick_item), 1);
    assert_eq!(rig.granted(0, &stick_item), 0);
}

#[test]
fn the_dropper_waits_out_the_pickup_delay_then_gets_it_back() {
    let mut rig = Rig::dedicated("drop-self", 1);
    let bone = Item::Material(MaterialId::Bone);
    rig.give(0, 0, ItemStack::new_material(MaterialId::Bone, 1));
    rig.drop(0, &bone);
    rig.tick();
    assert_eq!(rig.shadow_count(0, &bone), 0);
    let at = settle(&mut rig); // 13 ticks since the drop
    rig.sp(0).player.pos = at;
    rig.ticks(crate::entity::ITEM_DROP_PICKUP_DELAY_TICKS - 16);
    assert_eq!(rig.items().len(), 1, "still on the ground inside the delay");
    assert_eq!(rig.granted(0, &bone), 0);
    rig.ticks(6);
    assert!(rig.items().is_empty(), "the delay is over: picked up");
    assert_eq!(rig.granted(0, &bone), 1, "by InventoryGrant");
    assert_eq!(rig.shadow_count(0, &bone), 1);
}

#[test]
fn an_item_the_shadow_lacks_still_drops_and_is_counted() {
    let mut rig = Rig::dedicated("drop-short", 1);
    let wheat = Item::Material(MaterialId::Wheat);
    rig.drop(0, &wheat);
    rig.tick();
    let items = rig.items();
    assert_eq!(items.len(), 1, "it still spawns (log-only, as placements until C3)");
    assert_eq!(items[0].1, ItemStack::new_material(MaterialId::Wheat, 1));
    assert_eq!(rig.tally(0).mismatched, 1);
    assert_eq!(rig.tally(0).drops, 1);
}

/// The claimed item drops at full fidelity: a worn tool keeps its wear.
#[test]
fn a_dropped_tool_keeps_the_wear_the_client_claims() {
    let mut rig = Rig::dedicated("drop-tool", 1);
    let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
    pick.durability = 17;
    rig.give(0, 0, ItemStack { item: Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron)), count: 1 });
    rig.drop(0, &Item::Tool(pick));
    rig.tick();
    let items = rig.items();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].1.item, Item::Tool(pick), "the client's wear, not the shadow's");
    assert_eq!(rig.shadow_count(0, &Item::Tool(pick)), 0, "the shadow's pickaxe is taken");
}

/// Pacing: five drops arriving in one tick all spawn, in order, two at once
/// (the bucket's capacity) and then one per `DROP_INTERVAL_TICKS`. None is
/// lost or refused; what follows them waits with them.
#[test]
fn five_drops_in_one_tick_all_spawn_in_order() {
    let mut rig = Rig::dedicated("drop-pace", 1);
    let sent: Vec<Item> = [MaterialId::Stick, MaterialId::Bone, MaterialId::Wheat, MaterialId::Bread, MaterialId::Coal]
        .into_iter()
        .map(Item::Material)
        .collect();
    for item in &sent {
        rig.drop(0, item);
    }
    // C2b verify L7 — an input behind the drops waits with them (one ordered
    // stream): it is read only once the last drop has been.
    rig.send_input(0, 1);
    let mut seen: Vec<(Item, u32)> = Vec::new();
    let step = DROP_INTERVAL_TICKS as u32;
    for t in 0..20u32 {
        rig.tick();
        let read = rig.sp(0).last_input_tick;
        if t < 3 * step {
            assert_eq!(read, 0, "tick {t}: the input waits behind the drops still queued");
        } else {
            assert_eq!(read, 1, "tick {t}: read right after the last drop");
        }
        for (_, stack, _) in rig.items() {
            if !seen.iter().any(|(i, _)| *i == stack.item) {
                seen.push((stack.item, t));
            }
        }
    }
    let order: Vec<Item> = seen.iter().map(|(i, _)| i.clone()).collect();
    assert_eq!(order, sent, "every drop spawned, in the order sent");
    let at: Vec<u32> = seen.iter().map(|(_, t)| *t).collect();
    assert_eq!(at, vec![0, 0, step, 2 * step, 3 * step], "two at once, then one per interval");
    assert_eq!(rig.tally(0).drops, 5);
}

/// The host already drops into the shared world; a joiner's drop on a
/// lending host lands there too — in the host client's own ECS.
#[test]
fn a_joiners_drop_on_a_lending_host_lands_in_the_hosts_world() {
    let mut rig = Rig::lent("drop-lent");
    rig.give(0, 0, ItemStack::new_material(MaterialId::Stick, 2));
    rig.drop(0, &stick());
    rig.tick();
    let host_items = rig.host.as_ref().unwrap().ecs.query::<&ItemEntity>().iter().count();
    assert_eq!(host_items, 1, "the drop is in the host's ECS");
    assert_eq!(rig.shadow_count(0, &stick()), 1);
}

/// C2b verify L7 — the host itself picks up a joiner's drop on a lent world:
/// its own client's pickup pass (`entity::tick_item_pickups`, over the lent
/// ECS) takes the stack, with no dropper delay since the dropper is the
/// joiner's server slot.
#[test]
fn the_host_picks_up_a_joiners_drop_on_a_lent_world() {
    let mut rig = Rig::lent("drop-host-pickup");
    rig.give(0, 0, ItemStack::new_material(MaterialId::Stick, 2));
    rig.drop(0, &stick());
    rig.tick();
    let at = settle(&mut rig);
    let mut host_inv = crate::inventory::Inventory::new();
    let host = rig.host.as_mut().unwrap();
    let picked = {
        let mut players = [(0usize, at, &mut host_inv)];
        crate::entity::tick_item_pickups(&mut host.ecs, &mut players, |_| true)
    };
    assert_eq!(picked.len(), 1, "the host's own pickup pass took it");
    assert_eq!(picked[0].1, ItemStack::new_material(MaterialId::Stick, 1));
    assert_eq!(host_inv.count_material(MaterialId::Stick), 1);
    assert!(rig.items().is_empty(), "gone from the shared ground");
}

// ─── Grant overflow (decision 3) ───────────────────────────────────────────

fn fill_shadow(rig: &mut Rig, j: usize) {
    for i in 0..36 {
        rig.give(j, i, ItemStack::new_material(MaterialId::Bone, 64));
    }
}

/// C2b-fix (verify M1) — a shadow with no room for a grant still sends the
/// client the WHOLE stack: the shadow fills by drift, so refusing the rest
/// would cost the joiner the item. What the shadow couldn't hold is tallied;
/// nothing spills.
#[test]
fn a_full_shadow_still_grants_the_whole_stack_and_spills_nothing() {
    let mut rig = Rig::dedicated("grant-full", 1);
    fill_shadow(&mut rig, 0);
    rig.give(0, 7, ItemStack::new_material(MaterialId::Wheat, 60));
    let slot = rig.joiners[0].slot;
    rig.hs.grant_to_joiner(slot, [ItemStack::new_material(MaterialId::Wheat, 10)]);
    rig.tick();
    let wheat = Item::Material(MaterialId::Wheat);
    assert_eq!(rig.granted(0, &wheat), 10, "the client is granted the whole stack");
    assert_eq!(rig.shadow_count(0, &wheat), 64, "the shadow took what fitted");
    assert!(rig.items().is_empty(), "nothing is spilled on the ground");
    assert_eq!(rig.tally(0).grant_overflow, 6, "the rest is tallied");
}

/// End to end: a break the joiner mines into a full shadow still grants the
/// whole yield to the client, with nothing spilled.
#[test]
fn a_break_into_a_full_shadow_still_grants_its_whole_yield() {
    let mut rig = Rig::dedicated("grant-break", 1);
    fill_shadow(&mut rig, 0);
    let floor = (rig.at.x.floor() as i32 + 1, rig.at.y as i32 - 1, rig.at.z.floor() as i32);
    let cs = crate::chunk::CHUNK_SIZE as i32;
    rig.hs.server.loaded_columns.insert((floor.0.div_euclid(cs), floor.2.div_euclid(cs)));
    let wood = Tool::new(ToolType::Pickaxe, ToolMaterial::Wood);
    let input = protocol::InputPacket {
        tick: 1,
        x: rig.at.x,
        y: rig.at.y,
        z: rig.at.z,
        health: 20.0,
        held_kind: pair(&Item::Tool(wood)).0,
        held_id: pair(&Item::Tool(wood)).1,
        hotbar_slot: Some(0),
        block_changes: vec![protocol::BlockChange { x: floor.0, y: floor.1, z: floor.2, new_block: block::AIR, meta: 0 }],
        mined: vec![protocol::MinedBlock {
            x: floor.0,
            y: floor.1,
            z: floor.2,
            tool: crate::inventory::item_to_wire_full(&Item::Tool(wood)),
        }],
        ..Default::default()
    };
    rig.joiners[0].client.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    rig.tick();
    assert_eq!(rig.world().get_block(floor.0, floor.1, floor.2), block::AIR);
    assert_eq!(rig.tally(0).breaks, 1);
    assert_eq!(rig.granted(0, &cobble()), 1, "the whole yield reaches the client");
    assert!(rig.items().is_empty(), "nothing spilled: the joiner could never pick it up");
    assert_eq!(rig.tally(0).grant_overflow, 1);
    assert_eq!(rig.shadow_count(0, &cobble()), 0, "the shadow had no room");
}

/// C2b-fix (verify M1) — a drifted-full shadow must not stop a joiner
/// picking things up: the server grants the whole ground item, adds what
/// fits and tallies the rest.
#[test]
fn a_joiner_with_a_full_shadow_still_picks_up_a_ground_item() {
    let mut rig = Rig::dedicated("pickup-full", 1);
    fill_shadow(&mut rig, 0);
    let at = rig.hs.server.players[rig.joiners[0].slot].player.pos;
    crate::entity::spawn_item(
        match rig.host.as_mut() {
            Some(h) => &mut h.ecs,
            None => &mut rig.hs.server.ecs,
        },
        at,
        ItemStack::new_material(MaterialId::Wheat, 5),
        0,
    );
    // Natural drops wait out their pickup delay.
    rig.ticks(crate::entity::ITEM_PICKUP_DELAY_TICKS + 3);
    let wheat = Item::Material(MaterialId::Wheat);
    assert!(rig.items().is_empty(), "picked up, not left on the ground");
    assert_eq!(rig.granted(0, &wheat), 5, "the client is granted it");
    assert_eq!(rig.tally(0).grant_overflow, 5, "the shadow had no room: tallied");
}

// ─── The drop bucket in client time (C2b verify M4) ────────────────────────

/// After a stall, an honest backlog of 25 drops interleaved with ~100 inputs
/// drains in a few ticks of catch-up, not at one drop per
/// `DROP_INTERVAL_TICKS` of server time: each input read credits the bucket
/// a quarter token. None is lost.
#[test]
fn a_stalled_backlog_of_drops_and_inputs_drains_in_catch_up_time() {
    let mut rig = Rig::dedicated("drop-catch-up", 1);
    rig.give(0, 0, ItemStack::new_material(MaterialId::Stick, 64));
    let mut tick = 0;
    for _ in 0..25 {
        for _ in 0..4 {
            tick += 1;
            rig.send_input(0, tick);
        }
        rig.drop(0, &stick());
    }
    let mut spawned_by = None;
    for t in 1..=60u32 {
        rig.tick();
        if rig.tally(0).drops == 25 && spawned_by.is_none() {
            spawned_by = Some(t);
        }
    }
    let t = spawned_by.expect("every one of the 25 drops spawned");
    assert!(t <= 12, "drained in {t} ticks; server time alone would take about 92");
    assert_eq!(rig.sp(0).last_input_tick, 100, "and every input was read");
}

/// A client that sends only Drops sends no inputs, so it earns no client-time
/// credit: it still gets two at once and then one per interval.
#[test]
fn a_client_sending_only_drops_earns_no_client_time_credit() {
    let mut rig = Rig::dedicated("drop-no-credit", 1);
    rig.give(0, 0, ItemStack::new_material(MaterialId::Stick, 64));
    for _ in 0..50 {
        rig.drop(0, &stick());
    }
    rig.ticks(DROP_INTERVAL_TICKS as u32 * 5);
    assert!(rig.tally(0).drops <= 2 + 5 + 1, "paced by server time only: {}", rig.tally(0).drops);
}

/// A closed connection's queued Drops are discarded, not spawned, so the slot
/// is reaped within a few ticks instead of dribbling items out for hours.
#[test]
fn a_closed_connection_holding_a_thousand_drops_is_reaped_and_none_spawn() {
    let mut rig = Rig::dedicated("drop-closed", 1);
    let slot = rig.joiners[0].slot;
    for _ in 0..1000 {
        rig.drop(0, &stick());
    }
    drop(rig.joiners.remove(0).client); // the link just goes
    let mut freed_at = None;
    for t in 1..=6u32 {
        rig.tick();
        if rig.hs.slot_is_free(slot) && freed_at.is_none() {
            freed_at = Some(t);
        }
    }
    assert!(freed_at.is_some(), "the slot is reaped within a few ticks");
    assert!(rig.items().is_empty(), "none of the 1000 drops spawned");
}
