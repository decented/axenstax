//! C3b-1 (2026-10-08, protocol v77; C3b-fix-a, v78) — shared chests, dispensers and furnaces
//! for joiners.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport (a
//! dedicated server, or a lending host) and joiners whose client half is the
//! real one: a `CraftingUi` with the player's `Inventory` and armour, and a
//! `container_window::SharedContainer` mirror built from the server's
//! `ContainerOpened` and kept by its `WindowSlotSet`s
//! (`container_window::apply_slot_set`, with the session's correction debt),
//! exactly as `game_loop` keeps it, each a window event applied in arrival
//! order (C3b-fix-a, v78: every container view is numbered). Clicks go
//! through `CraftingUi::apply_container_click` (the prediction, logged as a
//! window op with its digest, touched slots, claims and verdict), and the
//! log is sent numbered, as `GameState::flush_window_ops` sends it. A
//! client can be held (`Rig::tick_holding`) so it acts inside a correction's
//! round trip.

use glam::Vec3;

use crate::armour::ArmourItem;
use crate::block;
use crate::chest::{ChestData, ChestTier};
use crate::container_window::{self, ContainerClick, ContainerKind, ContainerRef, SharedContainer};
use crate::craft_ui::CraftingUi;
use crate::furnace::{ClickMode, SlotKind};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::protocol::{self, slot_set_reason, OpenRefusal, WindowSlotSetPacket, WireWindowSlot};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};
use crate::window::{self, ClickCtx, ClickResult, Station};

use super::joiner_authority::join_guest;
use super::joiners_act::floor_and_stand;
use super::lent_world::{join_guest_lent, start_lent};

/// One joiner's client: its window, its mirror of the open container, and
/// what the server told it.
struct Client {
    transport: ChannelClientTransport,
    slot: usize,
    ui: CraftingUi,
    inv: Inventory,
    armour: [Option<ArmourItem>; 4],
    mirror: Option<SharedContainer>,
    /// The last `op_seq` sent.
    seq: u32,
    /// Every `ContainerOpened` refusal received.
    refusals: Vec<OpenRefusal>,
    /// Every `WindowSlotSet` received, in order.
    slot_sets: Vec<WindowSlotSetPacket>,
    /// The highest window event applied (C3a-fix-1; here an opened
    /// container, a push or a correction), reported with every op.
    events: u32,
    /// C3b-fix-a — what its window owes from correction takes it couldn't
    /// pay (`WindowInbox::debt` in the game).
    debt: container_window::CorrectionDebt,
    /// `GrantUnfit` units reported back.
    unfit: u32,
    /// Its `ItemAction` sequence (`JoinerActions::unanswered`).
    actions: u32,
    /// Its last input's sequence number ([`Rig::report`]).
    inputs: u64,
}

impl Client {
    fn new((transport, slot): (ChannelClientTransport, usize)) -> Self {
        Client {
            transport,
            slot,
            ui: CraftingUi::new(),
            inv: Inventory::new(),
            armour: [None; 4],
            mirror: None,
            seq: 0,
            refusals: Vec::new(),
            slot_sets: Vec::new(),
            events: 0,
            debt: Default::default(),
            unfit: 0,
            actions: 0,
            inputs: 0,
        }
    }

    /// Read everything the server sent: an opened container becomes the
    /// mirror, a slot set is applied to the window and the mirror. As
    /// `GameState::apply_window_inbox` does, the ops logged before go out
    /// first, so each reports the events applied when it was made.
    fn receive(&mut self, registry: &block::BlockRegistry) {
        self.flush();
        while let Some(pkt) = self.transport.try_recv_from_server() {
            match protocol::deserialize_header(&pkt) {
                Some((protocol::PacketType::ContainerOpened, payload)) => {
                    let opened: protocol::ContainerOpenedPacket = protocol::safe_deserialize(payload).unwrap();
                    match opened.refused {
                        Some(why) => self.refusals.push(why),
                        None => self.mirror = SharedContainer::from_opened(&opened, registry),
                    }
                    self.events = self.events.max(opened.window_event);
                }
                Some((protocol::PacketType::WindowSlotSet, payload)) => {
                    let set: WindowSlotSetPacket = protocol::safe_deserialize(payload).unwrap();
                    let mut view = window::WindowMut {
                        inv: &mut self.inv,
                        armour: &mut self.armour,
                        cursor: &mut self.ui.cursor_item,
                        grid: &mut self.ui.grid,
                        container: self.mirror.as_mut().map(|m| m.as_mut()),
                    };
                    let applied = container_window::apply_slot_set(&mut view, &set, registry, &mut self.debt);
                    self.events = self.events.max(set.window_event);
                    // What didn't fit goes back to the server, one report per
                    // give in order, as `GameState::apply_window_inbox` sends it.
                    let reported = applied.unfit.iter().rposition(|&n| n > 0).map_or(0, |last| last + 1);
                    for &count in &applied.unfit[..reported] {
                        self.unfit += u32::from(count);
                        self.actions += 1;
                        let pkt = protocol::ItemActionPacket {
                            seq: self.actions,
                            action: protocol::ItemAction::GrantUnfit { event: set.window_event, count },
                            events_applied: self.events,
                        };
                        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
                    }
                    self.slot_sets.push(set);
                }
                _ => {}
            }
        }
    }

    /// Ask to open the container at `cell` (`OpenContainer`).
    fn ask_open(&mut self, cell: [i32; 3]) {
        self.ui.log_open_container(cell, &self.inv, &self.armour);
    }

    /// Predict `click` on the mirror and the window, from `eye`.
    fn click(&mut self, click: ContainerClick, eye: Vec3) -> ClickResult {
        let ctx = ClickCtx::new(false, Station::Player, eye, |_| block::AIR).with_shared(true);
        let mirror = self.mirror.as_mut().expect("a container is open");
        self.ui.apply_container_click(&mut self.inv, &mut self.armour, mirror.as_mut(), &click, &ctx).result
    }

    /// Close the container screen (`Close`, as `tick_shared_container` does).
    fn close(&mut self) {
        self.mirror = None;
        self.ui.close_container(&mut self.inv, &mut self.armour);
    }

    /// Send every logged op, numbered, as the game loop's flush does.
    fn flush(&mut self) {
        for logged in self.ui.take_ops(&self.inv, &self.armour) {
            self.seq += 1;
            let pkt = logged.packet(self.seq, self.events);
            self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
        }
    }

    fn digest(&self) -> u32 {
        window::digest_with(
            &self.inv,
            &self.armour,
            &self.ui.cursor_item,
            &self.ui.grid,
            Station::Player,
            self.mirror.as_ref().map(|m| m.as_ref()),
        )
    }

    /// Units of `item` in its window: the 36 slots, the cursor, the grid
    /// and the armour slots (C3b-fix-c: a correction take reaches all four).
    fn count(&self, item: &Item) -> u32 {
        window_units(&self.inv, &self.ui.cursor_item, &self.ui.grid, &self.armour, item)
    }
}

/// Units of `item` (by exact identity) in a window: its 36 slots, cursor,
/// crafting grid and armour slots.
fn window_units(
    inv: &Inventory,
    cursor: &Option<ItemStack>,
    grid: &window::CraftGrid,
    armour: &[Option<ArmourItem>; 4],
    item: &Item,
) -> u32 {
    let stacks = inv.slots_iter().flatten().chain(cursor.iter()).chain(grid.iter().flatten().flatten());
    let held: u32 = stacks.filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum();
    held + armour.iter().flatten().filter(|p| &Item::Armour(**p) == item).count() as u32
}

/// Joiners standing on a stone floor round (40, 80, 40), on a dedicated
/// server or a lending host (`host`: the host client's world and ECS).
struct Rig {
    hs: HostedServer,
    host: Option<OwnedSimParts>,
    cs: Vec<Client>,
    at: Vec3,
    registry: block::BlockRegistry,
}

impl Rig {
    fn dedicated(tag: &str, joiners: usize) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("shared-containers-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let joined: Vec<_> = (0..joiners).map(|n| join_guest(&mut hs, &format!("Keeper{n}"))).collect();
        Self::stand(hs, None, joined)
    }

    fn lent(tag: &str) -> Self {
        let (mut hs, mut host) = start_lent(&format!("shared-containers-{tag}"));
        let joined = join_guest_lent(&mut hs, &mut host, "Keeper0");
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
        let cs = joined.into_iter().map(Client::new).collect();
        let mut rig = Rig { hs, host, cs, at, registry: block::BlockRegistry::new() };
        rig.tick();
        rig
    }

    /// One server tick, then every client reads what it was sent.
    fn tick(&mut self) {
        self.tick_holding(None);
    }

    /// One server tick, then every client but `held` reads what it was
    /// sent: what the server sent `held` stays on its way (in the channel),
    /// so `held` can act inside that round trip.
    fn tick_holding(&mut self, held: Option<usize>) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        for (n, c) in self.cs.iter_mut().enumerate() {
            if Some(n) != held {
                c.receive(&self.registry);
            }
        }
    }

    /// Joiner `n` reports the window events it applied, as every input
    /// does (standing still), and the server ticks: the server's copy of
    /// its window catches up with what its client applied.
    fn report(&mut self, n: usize) {
        let slot = self.cs[n].slot;
        self.cs[n].inputs += 1;
        let sp = &self.hs.server.players[slot];
        let input = protocol::InputPacket {
            tick: self.cs[n].inputs,
            x: sp.player.pos.x,
            y: sp.player.pos.y,
            z: sp.player.pos.z,
            yaw: sp.yaw,
            pitch: sp.pitch,
            health: 20.0,
            events_applied: self.cs[n].events,
            ..Default::default()
        };
        self.cs[n].transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
        self.tick();
    }

    /// Units of `item` in the world: every joiner's client window (C3b-fix-c:
    /// its 36 slots, cursor, grid and armour), the chest at `chest`, and the
    /// ground items; and the same with the server's copies of the windows
    /// instead of the clients'.
    fn world_total(&self, item: &Item, chest: [i32; 3]) -> (u32, u32) {
        let in_chest: u32 = self
            .world_ref()
            .chest_at((chest[0], chest[1], chest[2]))
            .map(|c| c.slots.iter().flatten().filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum())
            .unwrap_or(0);
        let ground: u32 = self
            .hs
            .server
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter(|(_, it)| &it.stack.item == item)
            .map(|(_, it)| u32::from(it.stack.count))
            .sum();
        let clients: u32 = self.cs.iter().map(|c| c.count(item)).sum();
        let servers: u32 = self
            .cs
            .iter()
            .map(|c| {
                let sp = &self.hs.server.players[c.slot];
                window_units(&sp.inventory, &sp.cursor, &sp.craft_grid, &sp.armour, item)
            })
            .sum();
        (clients + in_chest + ground, servers + in_chest + ground)
    }

    fn world(&mut self) -> &mut crate::world::World {
        match self.host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut self.hs.server.world,
        }
    }

    fn world_ref(&self) -> &crate::world::World {
        match self.host.as_ref() {
            Some(h) => &h.world,
            None => &self.hs.server.world,
        }
    }

    fn sp(&mut self, n: usize) -> &mut crate::server::ServerPlayer {
        let slot = self.cs[n].slot;
        &mut self.hs.server.players[slot]
    }

    fn tally(&self, n: usize) -> crate::joiner_inventory::PossessionTally {
        self.hs.server.players[self.cs[n].slot].possession
    }

    fn eye(&self, n: usize) -> Vec3 {
        self.hs.server.players[self.cs[n].slot].player.eye_pos()
    }

    /// Put `count` of `item` in slot `slot` on both sides of joiner `n`.
    fn give(&mut self, n: usize, slot: usize, item: Item, count: u8) {
        let stack = ItemStack { item, count };
        self.cs[n].inv.set_slot(slot, Some(stack.clone()));
        self.sp(n).inventory.set_slot(slot, Some(stack));
    }

    /// A block `dz` ahead of the joiners' feet.
    fn place(&mut self, dx: i32, dz: i32, b: block::BlockId) -> [i32; 3] {
        let cell = [self.at.x.floor() as i32 + dx, self.at.y as i32, self.at.z.floor() as i32 + dz];
        self.world().set_block(cell[0], cell[1], cell[2], b);
        cell
    }

    /// Joiner `n` opens the container at `cell`: ask, send, tick.
    fn open(&mut self, n: usize, cell: [i32; 3]) {
        self.cs[n].ask_open(cell);
        self.cs[n].flush();
        self.tick();
    }

    /// Joiner `n` clicks, sends, the server ticks; then the two windows and
    /// containers must agree.
    fn step(&mut self, n: usize, what: &str, click: ContainerClick) -> ClickResult {
        let eye = self.eye(n);
        let result = self.cs[n].click(click, eye);
        self.cs[n].flush();
        self.tick();
        self.assert_lockstep(n, what);
        result
    }

    /// The server's real container `n` has open, as a read-only view.
    fn server_container(&self, n: usize) -> Option<ContainerRef<'_>> {
        let sp = &self.hs.server.players[self.cs[n].slot];
        let cell = sp.open_container?;
        let kind = sp.container_sent.open_kind?;
        container_window::container_at(self.world_ref(), cell, kind)
    }

    fn server_digest(&self, n: usize) -> u32 {
        let sp = &self.hs.server.players[self.cs[n].slot];
        window::digest_with(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station, self.server_container(n))
    }

    /// Joiner `n`'s window and container mirror equal the server's, slot for
    /// slot, and no op has mismatched.
    fn assert_lockstep(&self, n: usize, what: &str) {
        let c = &self.cs[n];
        let sp = &self.hs.server.players[c.slot];
        let slots = |inv: &Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
        assert_eq!(slots(&sp.inventory), slots(&c.inv), "{what}: the 36 slots");
        assert_eq!(sp.cursor, c.ui.cursor_item, "{what}: the cursor");
        let server = self.server_container(n).map(|r| r.prints());
        let mirror = c.mirror.as_ref().map(|m| m.as_ref().prints());
        assert_eq!(server, mirror, "{what}: the container");
        assert_eq!(self.server_digest(n), c.digest(), "{what}: the digests");
        assert_eq!(sp.possession.window_mismatch, 0, "{what}: no op mismatched");
        assert_eq!(sp.possession.container_corrected, 0, "{what}: nothing corrected");
    }

    /// C3b-fix-c — a modified client's container op: `click`, claiming
    /// `claims` (each slot's value before it), reporting the events joiner
    /// `n` applied.
    fn send_op(&mut self, n: usize, click: ContainerClick, touched: Vec<WireWindowSlot>, claims: Vec<(WireWindowSlot, protocol::WireSlot)>) {
        let c = &mut self.cs[n];
        c.seq += 1;
        let pkt = protocol::WindowOpPacket {
            op_seq: c.seq,
            op: protocol::WireWindowOp::Container(click),
            digest: 0,
            events_applied: c.events,
            touched,
            claims,
            client_ok: true,
        };
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
    }

    /// C3b-fix-c — joiner `n` places `placed` at `cell` from hotbar slot 0,
    /// whose hand holds `held`: its client takes one from slot 0, and one
    /// input carries the edit with its own hand (as the game loop sends it),
    /// reporting the events the client applied.
    fn place_from_hotbar(&mut self, n: usize, cell: [i32; 3], placed: block::BlockId, held: &Item) {
        let (kind, id) = crate::inventory::item_to_ref(held).to_wire();
        let c = &mut self.cs[n];
        take_one(&mut c.inv, 0);
        c.inputs += 1;
        let sp = &self.hs.server.players[c.slot];
        let input = protocol::InputPacket {
            tick: c.inputs,
            x: sp.player.pos.x,
            y: sp.player.pos.y,
            z: sp.player.pos.z,
            yaw: sp.yaw,
            pitch: sp.pitch,
            health: 20.0,
            held_kind: kind,
            held_id: id,
            hotbar_slot: Some(0),
            block_changes: vec![protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: placed, meta: 0 }],
            edit_hands: vec![(0, kind, id)],
            events_applied: c.events,
            ..Default::default()
        };
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// C3b-fix-c — joiner `n` Q-drops one `held` from hotbar slot 0 (its
    /// client takes it), reporting the events it applied.
    fn drop_from_hotbar(&mut self, n: usize, held: &Item) {
        let (held_kind, held_id) = crate::inventory::item_to_ref(held).to_wire();
        let held_full = crate::inventory::item_to_wire_full(held);
        take_one(&mut self.cs[n].inv, 0);
        self.send_action(n, protocol::ItemAction::Drop { hotbar_slot: 0, held_kind, held_id, held_full });
    }

    /// C3b-fix-c — joiner `n` sends `action`, reporting the events it
    /// applied (a modified client's `GrantUnfit`, say).
    fn send_action(&mut self, n: usize, action: protocol::ItemAction) {
        let c = &mut self.cs[n];
        c.actions += 1;
        let pkt = protocol::ItemActionPacket { seq: c.actions, action, events_applied: c.events };
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    }

    /// The ground items of `item` the server simulates.
    fn ground(&self, item: &Item) -> u32 {
        self.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter().filter(|(_, it)| &it.stack.item == item).map(|(_, it)| u32::from(it.stack.count)).sum()
    }

    /// The last correction joiner `n` was sent that gives it something:
    /// its window event.
    fn last_give(&self, n: usize) -> u32 {
        self.cs[n]
            .slot_sets
            .iter()
            .rev()
            .find(|s| s.reason == slot_set_reason::CORRECTION && !s.give.is_empty())
            .map(|s| s.window_event)
            .expect("a correction that gives")
    }
}

/// Take one from inventory slot `slot`.
fn take_one(inv: &mut Inventory, slot: usize) {
    if let Some(mut stack) = inv.take_slot(slot) {
        stack.count -= 1;
        if stack.count > 0 {
            inv.set_slot(slot, Some(stack));
        }
    }
}

fn stone(n: u8) -> ItemStack {
    ItemStack::new_block(block::STONE, n)
}

fn a_plan() -> ItemStack {
    ItemStack { item: Item::Plan(crate::plan::PlanData::debug_3x3_stone()), count: 1 }
}

/// The brief's scripted session on a dedicated server: a chest (withdraw by
/// click and shift, deposit one and all, Sort, Dump matching, Restock, Take
/// all), a dispenser, and a furnace (input, fuel, the server cooks, the
/// output). After every op the server's window and real container equal
/// the joiner's window and mirror, slot for slot.
#[test]
fn a_scripted_container_session_keeps_the_server_in_lockstep() {
    let mut rig = Rig::dedicated("session", 1);
    rig.give(0, 0, Item::Block(block::STONE), 10);
    rig.give(0, 1, Item::Block(block::DIRT), 6);
    rig.give(0, 2, Item::Material(MaterialId::RawIron), 3);
    rig.give(0, 3, Item::Material(MaterialId::Coal), 4);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(stone(5));
    contents.slots[4] = Some(ItemStack::new_material(MaterialId::Stick, 7));
    contents.slots[9] = Some(stone(20));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);

    rig.open(0, chest);
    let mirror = rig.cs[0].mirror.as_ref().expect("the chest opened");
    assert_eq!(mirror.kind, ContainerKind::Chest { tier: ChestTier::Wood });
    rig.assert_lockstep(0, "open");
    assert_eq!(rig.hs.server.players[rig.cs[0].slot].open_container, Some(chest));

    assert_eq!(rig.step(0, "withdraw one", ContainerClick::Withdraw { slot: 0, all: false }), ClickResult::Done);
    assert_eq!(rig.step(0, "withdraw the stack (shift)", ContainerClick::Withdraw { slot: 4, all: true }), ClickResult::Done);
    assert_eq!(rig.step(0, "deposit one", ContainerClick::Deposit { slot: 1, all: false }), ClickResult::Done);
    assert_eq!(rig.step(0, "deposit the stack", ContainerClick::Deposit { slot: 1, all: true }), ClickResult::Done);
    rig.step(0, "sort", ContainerClick::Sort);
    rig.step(0, "dump matching", ContainerClick::DumpMatching);
    rig.step(0, "restock", ContainerClick::Restock);
    rig.step(0, "take all", ContainerClick::TakeAll);
    assert!(rig.server_container(0).is_some_and(|c| (0..c.len()).all(|i| c.get(i).is_none())), "the server's chest is empty");
    rig.cs[0].close();
    rig.cs[0].flush();
    rig.tick();
    assert_eq!(rig.hs.server.players[rig.cs[0].slot].open_container, None, "a close closes the server's too");

    // A dispenser: the chest rules on its 9 slots.
    let dispenser = rig.place(1, 2, block::DISPENSER);
    rig.open(0, dispenser);
    assert_eq!(rig.cs[0].mirror.as_ref().map(|m| m.kind), Some(ContainerKind::Dispenser), "created on the server");
    assert!(rig.world().dispenser_at((dispenser[0], dispenser[1], dispenser[2])).is_some());
    let dirt = rig.cs[0].inv.slots_iter().position(|s| matches!(s, Some(st) if st.item == Item::Block(block::DIRT)));
    let at = dirt.expect("the dirt came back");
    rig.step(0, "into the dispenser", ContainerClick::Deposit { slot: at, all: true });
    rig.step(0, "out again, one", ContainerClick::Withdraw { slot: 0, all: false });
    rig.cs[0].close();
    rig.cs[0].flush();
    rig.tick();

    // A furnace: input and fuel from the held hotbar slot, then the server
    // cooks and the joiner sees the progress and the output.
    let furnace = rig.place(-1, 2, block::FURNACE);
    rig.open(0, furnace);
    assert_eq!(rig.cs[0].mirror.as_ref().map(|m| m.kind), Some(ContainerKind::Furnace));
    let feed = |kind, hotbar| ContainerClick::Furnace { kind, mode: ClickMode::Stack, hotbar };
    rig.step(0, "input", feed(SlotKind::Input, 2));
    rig.step(0, "fuel", feed(SlotKind::Fuel, 3));
    let pushes_before = rig.cs[0].slot_sets.len();
    // The dedicated server ticks its machines on its 4-tick block.
    let cooked = |rig: &Rig| rig.server_container(0).and_then(|c| c.get(container_window::FURNACE_OUTPUT)).is_some();
    for _ in 0..crate::furnace::SMELT_TICKS_PER_ITEM * 5 {
        if cooked(&rig) {
            break;
        }
        rig.tick();
    }
    assert!(cooked(&rig), "the server cooked one");
    let pushed: Vec<_> = rig.cs[0].slot_sets[pushes_before..].iter().collect();
    assert!(pushed.iter().all(|s| s.reason == slot_set_reason::CHANGED), "pushes, not corrections");
    assert!(pushed.iter().any(|s| s.furnace.is_some_and(|f| f.smelt_progress > 0)), "the progress was pushed");
    let crate::container_window::ContainerData::Furnace(f) = &rig.cs[0].mirror.as_ref().unwrap().contents else {
        panic!("a furnace mirror")
    };
    assert_eq!(f.output, Some(ItemStack::new_material(MaterialId::IronIngot, 1)), "the joiner sees the output");
    rig.assert_lockstep(0, "after cooking");
    rig.step(0, "take the output", feed(SlotKind::Output, 0));
    assert_eq!(rig.cs[0].count(&Item::Material(MaterialId::IronIngot)), 1);

    let t = rig.tally(0);
    assert_eq!((t.window_mismatch, t.container_corrected, t.container_refused), (0, 0, 0));
}

/// Race — two joiners take the last stack of one chest slot in the same
/// tick. The first the server reads gets it; the other's op is refused on
/// the server and corrected, so it ends with the server's values: no
/// duplicate, no loss.
#[test]
fn two_joiners_racing_for_one_stack_end_with_one_stack_between_them() {
    let mut rig = Rig::dedicated("race", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(stone(16));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    rig.open(0, chest);
    rig.open(1, chest);
    assert!(rig.cs[0].mirror.is_some() && rig.cs[1].mirror.is_some());

    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (e0, e1) = (rig.eye(0), rig.eye(1));
    assert_eq!(rig.cs[0].click(take.clone(), e0), ClickResult::Done);
    assert_eq!(rig.cs[1].click(take, e1), ClickResult::Done, "both predict the take");
    rig.cs[0].flush();
    rig.cs[1].flush();
    rig.tick();

    let first = rig.cs[0].slot < rig.cs[1].slot;
    let (winner, loser) = if first { (0, 1) } else { (1, 0) };
    let stone_item = Item::Block(block::STONE);
    assert_eq!(rig.cs[winner].count(&stone_item), 16, "the winner holds it");
    assert_eq!(rig.cs[loser].count(&stone_item), 0, "the loser's phantom stack was corrected away");
    // C3b-fix-a — the server's copy follows the loser's own order: it took
    // the prediction with the op, and takes the correction when the loser's
    // next packet reports it applied it.
    rig.report(loser);
    rig.report(winner);
    let server_stone: u32 = (0..2)
        .map(|n| rig.hs.server.players[rig.cs[n].slot].inventory.slots_iter().flatten().filter(|s| s.item == stone_item).map(|s| u32::from(s.count)).sum::<u32>())
        .sum();
    assert_eq!(server_stone, 16, "the server holds exactly one stack between them");
    assert!(rig.server_container(winner).is_some_and(|c| c.get(0).is_none()));
    let corrections: Vec<_> = rig.cs[loser].slot_sets.iter().filter(|s| s.reason == slot_set_reason::CORRECTION).collect();
    assert_eq!(corrections.len(), 1, "one correction");
    // C3b-fix-a (v78) — by item, never a slot value: take back the 16 it
    // predicted, from the slot it filled first.
    assert_eq!(corrections[0].take, vec![(0, crate::inventory::stack_to_wire(&stone(16)))], "take 16 stone, from slot 0 first");
    assert!(corrections[0].give.is_empty());
    assert!(player_sets(corrections[0]).is_empty(), "no set names a player slot");
    assert_eq!(rig.tally(loser).container_corrected, 1);
    assert_eq!(rig.tally(loser).window_mismatch, 0, "a lost race is container convergence, not a lockstep mismatch");
    for n in [winner, loser] {
        let what = format!("joiner {n} after the race");
        let c = &rig.cs[n];
        assert_eq!(rig.server_digest(n), c.digest(), "{what}: digests");
    }
}

/// Race, staggered — the loser clicks before the push of the winner's take
/// reaches it, and its op lands a tick later. C3b-fix-a (C-M1) — the op
/// reports the views it applied (none since it opened), so the server
/// re-runs its prediction on exactly the view it was made on and corrects
/// the phantom stack.
#[test]
fn a_take_predicted_before_the_push_arrived_is_still_corrected() {
    let mut rig = Rig::dedicated("race-staggered", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[3] = Some(stone(9));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    rig.open(0, chest);
    rig.open(1, chest);
    let take = ContainerClick::Withdraw { slot: 3, all: true };
    let (e0, e1) = (rig.eye(0), rig.eye(1));
    rig.cs[1].click(take.clone(), e1); // predicted on the stale mirror
    rig.cs[0].click(take, e0);
    rig.cs[0].flush();
    rig.tick(); // the server applies 0's take and pushes it to 1
    assert!(rig.cs[1].slot_sets.iter().any(|s| s.reason == slot_set_reason::CHANGED), "1 heard of the take");
    assert_eq!(rig.cs[1].count(&Item::Block(block::STONE)), 9, "but already predicted its own");
    rig.cs[1].flush();
    rig.tick();
    assert_eq!(rig.cs[1].count(&Item::Block(block::STONE)), 0, "corrected: no duplicate");
    assert_eq!(rig.cs[0].count(&Item::Block(block::STONE)), 9);
    rig.report(1);
    assert_eq!(rig.server_digest(1), rig.cs[1].digest());
}

/// Pushes — on a lending host, the host's own click on a chest a joiner has
/// open (the host's world is the server's) reaches the joiner as `Changed`
/// for exactly that slot.
#[test]
fn a_hosts_own_click_reaches_the_joiner_as_a_change_of_that_slot() {
    let mut rig = Rig::lent("host-push");
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest((chest[0], chest[1], chest[2]), ChestData::new());
    rig.open(0, chest);
    assert!(rig.cs[0].mirror.is_some(), "opened on the host's world");
    let before = rig.cs[0].slot_sets.len();
    // The host deposits stone through its own screen: the one rule, on its
    // own world (not shared: its rules don't change).
    let mut host_inv = Inventory::new();
    host_inv.set_slot(0, Some(stone(12)));
    let (mut armour, mut cursor, mut grid) = ([None; 4], None, Default::default());
    {
        let c = rig.world().chest_at_mut((chest[0], chest[1], chest[2])).unwrap();
        let mut view = window::WindowMut {
            inv: &mut host_inv,
            armour: &mut armour,
            cursor: &mut cursor,
            grid: &mut grid,
            container: Some(container_window::ContainerMut::Chest(c)),
        };
        let ctx = ClickCtx::new(false, Station::Player, Vec3::ZERO, |_| block::AIR);
        let done = container_window::apply_container(&mut view, &ContainerClick::Deposit { slot: 0, all: true }, &ctx);
        assert_eq!(done.result, ClickResult::Done);
    }
    rig.tick();
    let pushed = &rig.cs[0].slot_sets[before..];
    assert_eq!(pushed.len(), 1, "one push");
    assert_eq!(pushed[0].reason, slot_set_reason::CHANGED);
    assert_eq!(pushed[0].sets, vec![(WireWindowSlot::Container(0), Some(crate::inventory::stack_to_wire(&stone(12))))]);
    rig.assert_lockstep(0, "after the host's click");
    rig.tick();
    assert_eq!(rig.cs[0].slot_sets.len(), before + 1, "nothing more to push");
}

/// Pushes — a hopper on a dedicated server feeding the chest a joiner has
/// open: each item it moves is a `Changed` push of the slot it landed in,
/// and the joiner's own clicks are never pushed back.
#[test]
fn a_hopper_feeding_an_open_chest_is_pushed_and_own_clicks_are_not() {
    let mut rig = Rig::dedicated("hopper", 1);
    rig.give(0, 0, Item::Block(block::DIRT), 4);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest((chest[0], chest[1], chest[2]), ChestData::new());
    let hopper = [chest[0], chest[1] + 1, chest[2]];
    let source = [chest[0], chest[1] + 2, chest[2]];
    rig.world().set_block(hopper[0], hopper[1], hopper[2], block::HOPPER);
    rig.world().set_block(source[0], source[1], source[2], block::CHEST);
    rig.open(0, chest);
    // The joiner deposits: its own click, never pushed back.
    let before = rig.cs[0].slot_sets.len();
    rig.step(0, "deposit", ContainerClick::Deposit { slot: 0, all: true });
    for _ in 0..crate::hopper::HOPPER_INTERVAL_TICKS {
        rig.tick();
    }
    assert_eq!(rig.cs[0].slot_sets.len(), before, "an own click is not pushed");
    // Now the hopper moves stone in from the chest above.
    let mut above = ChestData::new();
    above.slots[0] = Some(stone(2));
    rig.world().insert_chest((source[0], source[1], source[2]), above);
    for _ in 0..crate::hopper::HOPPER_INTERVAL_TICKS * 2 {
        rig.tick();
    }
    let pushed = &rig.cs[0].slot_sets[before..];
    assert!(!pushed.is_empty(), "the hopper's moves were pushed");
    assert!(pushed.iter().all(|s| s.reason == slot_set_reason::CHANGED));
    assert!(pushed.iter().flat_map(|s| s.sets.iter()).all(|(at, _)| *at == WireWindowSlot::Container(1)), "exactly the slot the stone landed in");
    rig.assert_lockstep(0, "after the hopper");
}

/// Refusals — out of reach, in a foreign plot, and a cell holding no
/// container each answer `ContainerOpened` with the note, and nothing opens.
/// A container op with nothing open is refused, tallied and corrected.
#[test]
fn opens_out_of_reach_protected_or_not_a_container_are_refused() {
    let mut rig = Rig::dedicated("refusals", 1);
    let far = rig.place(0, 9, block::CHEST);
    rig.open(0, far);
    let protected = rig.place(2, 1, block::CHEST);
    rig.world().plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        protected[0],
        protected[1] - 5,
        protected[2],
    ));
    rig.open(0, protected);
    let stone_cell = rig.place(-2, 1, block::STONE);
    rig.open(0, stone_cell);
    assert_eq!(rig.cs[0].refusals, vec![OpenRefusal::OutOfReach, OpenRefusal::Protected, OpenRefusal::NotAContainer]);
    assert!(rig.cs[0].mirror.is_none(), "nothing opened");
    assert_eq!(rig.hs.server.players[rig.cs[0].slot].open_container, None);
    assert!(rig.world().chest_at((far[0], far[1], far[2])).is_none(), "no entity created for a refusal");

    // A container op with nothing open on the server: refused, tallied,
    // and the client's prediction undone. C3b-fix-a — undone by item, on
    // the view the server sent: the chest it opened was broken under it
    // (the server closed it) before the click reached the server.
    rig.world().plots.clear();
    rig.give(0, 5, Item::Block(block::DIRT), 3);
    let gone = rig.place(-1, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(stone(4));
    rig.world().insert_chest((gone[0], gone[1], gone[2]), contents);
    rig.open(0, gone);
    assert!(rig.cs[0].mirror.is_some(), "opened");
    rig.world().set_block(gone[0], gone[1], gone[2], block::AIR);
    rig.tick();
    assert_eq!(rig.hs.server.players[rig.cs[0].slot].open_container, None, "the server closed it");
    let eye = rig.eye(0);
    rig.cs[0].click(ContainerClick::Withdraw { slot: 0, all: true }, eye);
    rig.cs[0].flush();
    rig.tick();
    let t = rig.tally(0);
    assert_eq!((t.container_refused, t.container_corrected), (1, 1));
    assert_eq!(rig.cs[0].count(&Item::Block(block::STONE)), 0, "the stone from a chest the server no longer had open is gone");
    assert_eq!(rig.cs[0].count(&Item::Block(block::DIRT)), 3, "an untouched slot stays");
    assert_eq!(t.window_mismatch, 0, "a refused container op is container convergence, not a lockstep mismatch");
    rig.report(0);
    let sp = &rig.hs.server.players[rig.cs[0].slot];
    assert!(sp.inventory.slots_iter().flatten().all(|s| s.item != Item::Block(block::STONE)), "the server's copy never moved");
}

/// Plans — a joiner can't put a Plan in a shared container (refused on both
/// sides; a correction never destroys the Plan it holds), and a host's Plan
/// already in the chest reaches it as a placeholder it can't take, which
/// Take all leaves in place.
#[test]
fn plans_stay_with_their_holder() {
    let mut rig = Rig::dedicated("plans", 1);
    // The joiner holds a Plan (the server can't: a Plan has no wire form).
    rig.cs[0].inv.set_slot(3, Some(a_plan()));
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[5] = Some(a_plan());
    contents.slots[6] = Some(stone(3));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    rig.open(0, chest);
    let mirror = rig.cs[0].mirror.as_ref().expect("opened");
    let crate::container_window::ContainerData::Chest(m) = &mirror.contents else { panic!("a chest") };
    assert!(matches!(m.slots[5].as_ref().map(|s| &s.item), Some(Item::Plan(p)) if p.cells.is_empty()), "a body-less placeholder");

    let eye = rig.eye(0);
    assert_eq!(rig.cs[0].click(ContainerClick::Deposit { slot: 3, all: true }, eye), ClickResult::PlanStays);
    assert_eq!(rig.cs[0].click(ContainerClick::Withdraw { slot: 5, all: true }, eye), ClickResult::PlanStays);
    assert_eq!(rig.cs[0].click(ContainerClick::TakeAll, eye), ClickResult::Done);
    rig.cs[0].flush();
    rig.tick();
    assert!(matches!(rig.cs[0].inv.slot(3).map(|s| &s.item), Some(Item::Plan(_))), "the joiner keeps its Plan");
    let server = rig.server_container(0).unwrap();
    assert!(matches!(server.get(5).map(|s| &s.item), Some(Item::Plan(p)) if !p.cells.is_empty()), "the host's Plan stays, body and all");
    assert!(server.get(6).is_none(), "Take all took the stone");
    assert!(
        rig.hs.server.players[rig.cs[0].slot].inventory.slots_iter().flatten().all(|s| !matches!(s.item, Item::Plan(_))),
        "no Plan reached the joiner's server window"
    );
    assert_eq!(rig.cs[0].count(&Item::Block(block::STONE)), 3);
}

/// Loot chest (c3-evidence-a G6) — a joiner opening a worldgen loot chest
/// gets the server's contents and empties the server's chest; its own copy
/// is never opened, so there is no private loot to take twice.
#[test]
fn a_worldgen_loot_chest_opens_with_the_servers_contents() {
    let mut rig = Rig::dedicated("loot", 1);
    let chest = rig.place(0, 2, block::CHEST);
    let loot = crate::mineshaft_gen::loot_for_chest(7);
    assert!(loot.iter().any(Option::is_some), "the loot table filled something");
    rig.world().insert_chest((chest[0], chest[1], chest[2]), ChestData { slots: loot.clone(), tier: ChestTier::Wood });
    rig.open(0, chest);
    let mirror = rig.cs[0].mirror.as_ref().expect("opened");
    let crate::container_window::ContainerData::Chest(m) = &mirror.contents else { panic!("a chest") };
    assert_eq!(m.slots, loot, "the server's loot, slot for slot");
    rig.step(0, "take all", ContainerClick::TakeAll);
    rig.cs[0].close();
    rig.cs[0].flush();
    rig.tick();
    rig.open(0, chest);
    let crate::container_window::ContainerData::Chest(m) = &rig.cs[0].mirror.as_ref().unwrap().contents else { panic!() };
    assert!(m.slots.iter().all(Option::is_none), "the server's chest is empty: the loot was taken once");
}

/// A container the joiner has open that goes (broken) stops being pushed
/// and its ops are refused; the joiner's screen closes by the same rule.
#[test]
fn a_broken_container_closes_on_both_sides() {
    let mut rig = Rig::dedicated("broken", 1);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest((chest[0], chest[1], chest[2]), ChestData::new());
    rig.open(0, chest);
    rig.world().set_block(chest[0], chest[1], chest[2], block::AIR);
    rig.tick();
    assert_eq!(rig.hs.server.players[rig.cs[0].slot].open_container, None, "the server stopped pushing it");
    let mirror = rig.cs[0].mirror.as_ref().unwrap();
    assert!(!mirror.still_open(block::AIR, rig.eye(0)), "the client's rule closes the screen");
    assert!(mirror.still_open(block::CHEST, rig.eye(0)));
    assert!(!mirror.still_open(block::CHEST, rig.eye(0) + Vec3::new(0.0, 0.0, -9.0)), "and out of reach");
}

/// Sims off — a joined client runs no hopper tick, furnace sweep or chest
/// autocollect: the server (or the lending host) runs the real ones. Pinned
/// on the game loop's source, as the game loop needs a GPU (the harness's
/// `game_harness_a_joined_clients_machines_push_no_edits` drives it live).
#[test]
fn a_joined_client_runs_no_hopper_furnace_sweep_or_autocollect() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("sims-off lint: cannot read {} ({e})", path.display()));
    for call in ["crate::hopper::tick_hoppers(", "crate::furnace::tick_all(", "crate::chest::tick_chest_autocollect("] {
        let at = raw.find(call).unwrap_or_else(|| panic!("{call} is gone from game_loop.rs: update this lint, don't delete it"));
        assert_eq!(raw.matches(call).count(), 1, "{call}: one call site");
        let before = &raw[at.saturating_sub(400)..at];
        assert!(before.contains("self.remote_client.is_none()"), "{call} must be gated on not joined");
    }
}

// ── v77 (integration) — corrections are relative to the client's claims ──

/// The player-slot sets (not container slots) a slot set carries.
fn player_sets(set: &WindowSlotSetPacket) -> Vec<WireWindowSlot> {
    set.sets.iter().map(|(at, _)| *at).filter(|at| !matches!(at, WireWindowSlot::Container(_))).collect()
}

fn fish(n: u8) -> ItemStack {
    ItemStack::new_material(MaterialId::RawFish, n)
}

/// Decision 3 — an honest joiner deposits a fish it caught locally (fishing
/// is unmirrored until C3c, so the server's copy of its window doesn't hold
/// it). The deposit is believed: the real chest gets the fish, and the
/// joiner's slot is never reverted — not by the first deposit, nor by two
/// more clicked within one round trip (a correction relative to the
/// server's drifted copy would hand the fish back while the chest kept it).
#[test]
fn a_locally_caught_fish_deposited_reaches_the_chest_and_the_slot_is_not_reverted() {
    let mut rig = Rig::dedicated("believed", 1);
    rig.cs[0].inv.set_slot(4, Some(fish(3))); // the client's only: the server never saw the catch
    rig.give(0, 5, Item::Material(MaterialId::Bread), 2);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest((chest[0], chest[1], chest[2]), ChestData::new());
    rig.open(0, chest);

    let eye = rig.eye(0);
    assert_eq!(rig.cs[0].click(ContainerClick::Deposit { slot: 4, all: false }, eye), ClickResult::Done);
    rig.cs[0].flush();
    rig.tick();
    assert_eq!(rig.server_container(0).and_then(|c| c.get(0).cloned()), Some(fish(1)), "the real chest got the fish");
    assert_eq!(rig.cs[0].inv.slot(4), Some(&fish(2)), "the joiner's slot is not reverted");

    // Two more, clicked before any answer arrives.
    rig.cs[0].click(ContainerClick::Deposit { slot: 4, all: false }, eye);
    rig.cs[0].flush();
    rig.cs[0].click(ContainerClick::Deposit { slot: 4, all: false }, eye);
    rig.cs[0].flush();
    rig.tick();
    rig.tick();
    assert_eq!(rig.server_container(0).and_then(|c| c.get(0).cloned()), Some(fish(3)), "three fish in the chest");
    assert_eq!(rig.cs[0].inv.slot(4), None, "and none left: no fish made, none lost");
    assert_eq!(rig.cs[0].inv.slot(5), Some(&ItemStack::new_material(MaterialId::Bread, 2)));
    assert!(
        rig.cs[0].slot_sets.iter().all(|s| player_sets(s).is_empty() && s.window_event == 0),
        "no correction named a player slot: nobody raced, so the real chest changed no outcome"
    );
    let mirror = rig.cs[0].mirror.as_ref().unwrap().as_ref().prints();
    assert_eq!(Some(mirror), rig.server_container(0).map(|c| c.prints()), "the mirror shows the real chest");
    let t = rig.tally(0);
    assert_eq!(t.container_believed, 3, "each believed unit tallied");
}

/// Decision 3 — two joiners race for the last stack. The loser already
/// held 10 stone the server never saw (a local gain) in the slot its
/// prediction merged into. Its correction takes back exactly the 16 it
/// predicted it gained: the 10 survive (the server's drifted copy had none
/// there). The correction is a numbered window event, which the server's
/// copy applies when the loser's next op reports it. C3b-fix-a (v78) — by
/// item ("take 16 stone"), never a slot value, and the server's copy
/// applies its own change (it made the same prediction, so it takes the
/// same 16), never the client's claimed 10 (C-M2).
#[test]
fn the_loser_of_a_race_gives_back_only_what_it_predicted_it_gained() {
    let mut rig = Rig::dedicated("race-claims", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(stone(16));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    // The server reads the lower slot's op first: the other one loses.
    let loser = if rig.cs[0].slot < rig.cs[1].slot { 1 } else { 0 };
    let winner = 1 - loser;
    rig.cs[loser].inv.set_slot(0, Some(stone(10))); // the client's only
    rig.open(0, chest);
    rig.open(1, chest);
    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    assert_eq!(rig.cs[loser].inv.slot(0), Some(&stone(26)), "predicted: merged into its own 10");
    rig.cs[0].flush();
    rig.cs[1].flush();
    rig.tick();

    assert_eq!(rig.cs[winner].count(&Item::Block(block::STONE)), 16);
    assert_eq!(rig.cs[loser].inv.slot(0), Some(&stone(10)), "the 16 went back; its own 10 survive");
    let corrections: Vec<_> = rig.cs[loser].slot_sets.iter().filter(|s| s.reason == slot_set_reason::CORRECTION).collect();
    assert_eq!(corrections.len(), 1);
    assert!(player_sets(corrections[0]).is_empty(), "no slot value");
    assert_eq!(
        corrections[0].take,
        vec![(0, crate::inventory::stack_to_wire(&stone(16)))],
        "take the 16 it predicted, from the slot it predicted into first"
    );
    let event = corrections[0].window_event;
    assert!(event > 0, "a correction is a numbered window event");
    assert!(rig.server_container(loser).is_some_and(|c| c.get(0).is_none()), "the stack is the winner's");
    // The loser's next op reports the event: the server's copy takes it.
    assert_eq!(rig.sp(loser).inventory.slot(0), Some(&stone(16)), "the server's copy made the same prediction, and waits for the client's word");
    rig.cs[loser].close();
    rig.cs[loser].flush();
    rig.tick();
    assert_eq!(rig.sp(loser).inventory.slot(0), None, "applied in the client's order: never the client's claimed 10");
    assert_eq!(rig.sp(loser).window_events.tally.forced, 0);
}

/// Decision 3 — a joiner whose window drifted from the server's copy in
/// slots its op never touched (the server holds dirt in slot 20 the client
/// doesn't; the client holds bread in slot 21 the server doesn't) loses a
/// race: no correction names either slot, and both keep what they hold.
#[test]
fn drift_in_an_untouched_slot_is_never_corrected() {
    let mut rig = Rig::dedicated("drift", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[2] = Some(stone(8));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    let loser = if rig.cs[0].slot < rig.cs[1].slot { 1 } else { 0 };
    let winner = 1 - loser;
    rig.sp(loser).inventory.set_slot(20, Some(ItemStack::new_block(block::DIRT, 5))); // the server's only
    rig.cs[loser].inv.set_slot(21, Some(ItemStack::new_material(MaterialId::Bread, 3))); // the client's only
    rig.open(0, chest);
    rig.open(1, chest);
    let take = ContainerClick::Withdraw { slot: 2, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    rig.cs[0].flush();
    rig.cs[1].flush();
    rig.tick();
    let sets: Vec<WireWindowSlot> = rig.cs[loser].slot_sets.iter().flat_map(player_sets).collect();
    let corrected = rig.cs[loser].slot_sets.iter().any(|s| s.reason == slot_set_reason::CORRECTION && !s.take.is_empty());
    assert!(corrected, "the race was corrected");
    assert!(sets.is_empty(), "C3b-fix-a — a correction names no player slot, so never a drifted one: {sets:?}");
    assert_eq!(rig.cs[loser].inv.slot(20), None);
    assert_eq!(rig.cs[loser].inv.slot(21), Some(&ItemStack::new_material(MaterialId::Bread, 3)), "the client's bread stays");
    assert_eq!(rig.cs[loser].count(&Item::Block(block::STONE)), 0, "the phantom stack went back");
    assert_eq!(rig.cs[winner].count(&Item::Block(block::STONE)), 8);
}

/// Decision 4 — a joiner breaks a full loot chest. Its own world held the
/// generated copy; its client clears that copy without spilling it
/// (`container_client::clear_broken_containers`, `spill = false` when
/// joined), and the server spills the real one: exactly one set of real
/// items exists. Single-player still spills (`spill = true`).
#[test]
fn a_joiner_breaking_a_full_loot_chest_leaves_exactly_one_set_of_real_items() {
    let mut rig = Rig::dedicated("loot-break", 1);
    let chest = rig.place(0, 2, block::CHEST);
    let cell = (chest[0], chest[1], chest[2]);
    let loot = crate::mineshaft_gen::loot_for_chest(11);
    let total = |slots: &[Option<ItemStack>]| slots.iter().flatten().map(|s| u32::from(s.count)).sum::<u32>();
    assert!(total(&loot) > 0, "the loot table filled something");
    rig.world().insert_chest(cell, ChestData { slots: loot.clone(), tier: ChestTier::Wood });
    // The joiner's own world: the same generated loot chest.
    let mut client_world = crate::world::World::new();
    client_world.set_block(chest[0], chest[1], chest[2], block::CHEST);
    client_world.insert_chest(cell, ChestData { slots: loot.clone(), tier: ChestTier::Wood });
    let mut client_ecs = hecs::World::new();

    // The client's break arm (joined: no spill), then the edit to the server.
    client_world.set_block(chest[0], chest[1], chest[2], block::AIR);
    crate::container_client::clear_broken_containers(&mut client_world, &mut client_ecs, chest, false);
    super::joiner_authority::send_edits(&rig.hs, &rig.cs[0].transport, rig.cs[0].slot, 1, &[(cell, block::AIR)]);
    rig.tick();

    let items = |ecs: &hecs::World| {
        ecs.query::<&crate::entity::ItemEntity>().iter().map(|(_, it)| u32::from(it.stack.count)).sum::<u32>()
    };
    assert!(client_world.chest_at(cell).is_none(), "the joiner's copy is cleared");
    assert_eq!(items(&client_ecs), 0, "the joiner spilled nothing of its own");
    assert_eq!(rig.world_ref().get_block(chest[0], chest[1], chest[2]), block::AIR);
    assert_eq!(items(&rig.hs.server.ecs), total(&loot), "the server spilled the real loot, once");

    // Single-player (not joined) still spills its own chest.
    let mut sp_world = crate::world::World::new();
    sp_world.insert_chest(cell, ChestData { slots: loot.clone(), tier: ChestTier::Wood });
    let mut sp_ecs = hecs::World::new();
    crate::container_client::clear_broken_containers(&mut sp_world, &mut sp_ecs, chest, true);
    assert_eq!(items(&sp_ecs), total(&loot));
}

// ── C3b-fix-a (v78) — corrections are relative, phantoms never believed ──

/// The loser of a race (joiner index) and the winner: the server reads the
/// lower slot's op first.
fn loser_and_winner(rig: &Rig) -> (usize, usize) {
    let loser = if rig.cs[0].slot < rig.cs[1].slot { 1 } else { 0 };
    (loser, 1 - loser)
}

/// C-H1 scenario 1 — two joiners shift-withdraw the same 16 stone; the
/// loser's correction is still on its way when it shift-deposits its
/// phantom stack straight back. Nothing is duplicated: the world holds 16
/// stone, on the clients' side and on the server's.
#[test]
fn a_loser_depositing_its_phantom_back_inside_the_round_trip_duplicates_nothing() {
    let mut rig = Rig::dedicated("race-deposit-back", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(stone(16));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    rig.open(0, chest);
    rig.open(1, chest);
    let (loser, winner) = loser_and_winner(&rig);
    let stone_item = Item::Block(block::STONE);
    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    rig.cs[winner].flush();
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    // Inside the correction's round trip: the loser deposits its phantom.
    let at = rig.cs[loser].inv.slots_iter().position(|s| s.is_some_and(|s| s.item == stone_item)).expect("the phantom");
    assert_eq!(rig.cs[loser].click(ContainerClick::Deposit { slot: at, all: true }, el), ClickResult::Done);
    rig.cs[loser].flush();
    rig.tick();
    rig.tick();
    rig.report(loser);
    rig.report(winner);
    assert_eq!(rig.cs[winner].count(&stone_item), 16, "the winner keeps its stack");
    assert_eq!(rig.cs[loser].count(&stone_item), 0, "the loser holds none");
    assert_eq!(rig.world_total(&stone_item, chest), (16, 16), "16 stone in the world (clients' view, servers' view)");
    assert_eq!(rig.tally(loser).container_believed, 0, "the phantom was never believed");
}

/// C-H1 scenario 2 — the loser's phantom stack sits in its slot when, inside
/// the correction's round trip, it shift-withdraws a second chest slot that
/// merges into it. The second stack is real and survives the correction:
/// nothing is lost.
#[test]
fn a_loser_withdrawing_a_second_slot_inside_the_round_trip_loses_nothing() {
    let mut rig = Rig::dedicated("race-two-slots", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(stone(16));
    contents.slots[1] = Some(stone(20));
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    rig.open(0, chest);
    rig.open(1, chest);
    let (loser, winner) = loser_and_winner(&rig);
    let stone_item = Item::Block(block::STONE);
    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    rig.cs[winner].flush();
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    assert_eq!(rig.cs[loser].click(ContainerClick::Withdraw { slot: 1, all: true }, el), ClickResult::Done);
    assert_eq!(rig.cs[loser].count(&stone_item), 36, "predicted: its phantom 16 and the real 20");
    rig.cs[loser].flush();
    rig.tick();
    rig.tick();
    rig.report(loser);
    rig.report(winner);
    assert_eq!(rig.cs[winner].count(&stone_item), 16);
    assert_eq!(rig.cs[loser].count(&stone_item), 20, "the real 20 survive the correction");
    assert_eq!(rig.world_total(&stone_item, chest), (36, 36), "36 stone in the world");
}

fn cobble(n: u8) -> ItemStack {
    ItemStack::new_block(block::COBBLESTONE, n)
}

/// A chest at `chest` fed by a hopper above it from a source chest holding
/// `feed` cobblestone; `contents` in the chest.
fn hopper_fed(rig: &mut Rig, chest: [i32; 3], contents: ChestData, feed: u8) {
    rig.world().insert_chest((chest[0], chest[1], chest[2]), contents);
    let hopper = [chest[0], chest[1] + 1, chest[2]];
    let source = [chest[0], chest[1] + 2, chest[2]];
    rig.world().set_block(hopper[0], hopper[1], hopper[2], block::HOPPER);
    rig.world().set_block(source[0], source[1], source[2], block::CHEST);
    let mut above = ChestData::new();
    above.slots[0] = Some(cobble(feed));
    rig.world().insert_chest((source[0], source[1], source[2]), above);
}

/// The real chest's slot `i` at `chest`.
fn real_slot(rig: &Rig, chest: [i32; 3], i: usize) -> Option<ItemStack> {
    rig.world_ref().chest_at((chest[0], chest[1], chest[2])).and_then(|c| c.slots[i].clone())
}

/// C-M1 — one joiner whose window drifted from the server's copy (the
/// server's holds dirt the client doesn't, so no digest matches) and a
/// hopper topping up the chest it has open: slot 7 goes from 19 to 20
/// cobblestone and the push lands. The joiner shift-withdraws slot 7, then —
/// inside that op's round trip — slot 8, which merges into the same stack.
/// The server judges each op on exactly the view it was made on, so neither
/// earns a correction and nothing is lost (the two-view guess corrected the
/// first to 20 after the merge and lost 15).
#[test]
fn a_drifted_joiner_withdrawing_from_a_hopper_fed_chest_loses_nothing() {
    let mut rig = Rig::dedicated("hopper-view", 1);
    rig.sp(0).inventory.set_slot(30, Some(ItemStack::new_block(block::DIRT, 5))); // the server's only
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[7] = Some(cobble(19));
    contents.slots[8] = Some(cobble(15));
    hopper_fed(&mut rig, chest, contents, 1);
    rig.open(0, chest);
    for _ in 0..crate::hopper::HOPPER_INTERVAL_TICKS * 3 {
        if real_slot(&rig, chest, 7) == Some(cobble(20)) {
            break;
        }
        rig.tick();
    }
    assert_eq!(real_slot(&rig, chest, 7), Some(cobble(20)), "the hopper topped slot 7 up");
    rig.tick();
    rig.report(0);
    let mirror_7 = |rig: &Rig| match &rig.cs[0].mirror.as_ref().unwrap().contents {
        container_window::ContainerData::Chest(c) => c.slots[7].clone(),
        _ => None,
    };
    assert_eq!(mirror_7(&rig), Some(cobble(20)), "the push landed");
    let eye = rig.eye(0);
    assert_eq!(rig.cs[0].click(ContainerClick::Withdraw { slot: 7, all: true }, eye), ClickResult::Done);
    rig.cs[0].flush();
    rig.tick_holding(Some(0));
    assert_eq!(rig.cs[0].click(ContainerClick::Withdraw { slot: 8, all: true }, eye), ClickResult::Done);
    assert_eq!(rig.cs[0].count(&Item::Block(block::COBBLESTONE)), 35, "merged: 20 + 15");
    rig.cs[0].flush();
    rig.tick();
    rig.tick();
    rig.report(0);
    assert_eq!(rig.cs[0].count(&Item::Block(block::COBBLESTONE)), 35, "nothing lost");
    assert!(rig.cs[0].slot_sets.iter().all(|s| s.take.is_empty() && s.give.is_empty()), "no correction moved an item");
    assert_eq!(rig.tally(0).container_corrected, 0, "each op was judged on its own view");
    assert_eq!((real_slot(&rig, chest, 7), real_slot(&rig, chest, 8)), (None, None));
    assert_eq!(rig.world_total(&Item::Block(block::COBBLESTONE), chest), (35, 35), "19 + 15 + the hopper's 1");
    assert_eq!(rig.sp(0).inventory.slot(30), Some(&ItemStack::new_block(block::DIRT, 5)), "the drift is untouched");
}

/// C-M1 — the same hopper, but the joiner clicks before the push of its
/// top-up reaches it: its op was made on the view with 19, the server's run
/// on the real chest gave it 20, so it is given the one more — and a second
/// withdraw made inside that round trip keeps its stack.
#[test]
fn a_withdraw_made_before_a_hopper_push_arrived_is_given_the_difference() {
    let mut rig = Rig::dedicated("hopper-in-flight", 1);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[7] = Some(cobble(19));
    contents.slots[8] = Some(cobble(15));
    hopper_fed(&mut rig, chest, contents, 1);
    rig.open(0, chest);
    // The hopper moves while the joiner hears nothing.
    for _ in 0..crate::hopper::HOPPER_INTERVAL_TICKS * 3 {
        if real_slot(&rig, chest, 7) == Some(cobble(20)) {
            break;
        }
        rig.tick_holding(Some(0));
    }
    assert_eq!(real_slot(&rig, chest, 7), Some(cobble(20)));
    let eye = rig.eye(0);
    rig.cs[0].click(ContainerClick::Withdraw { slot: 7, all: true }, eye);
    assert_eq!(rig.cs[0].count(&Item::Block(block::COBBLESTONE)), 19, "predicted on the view with 19");
    rig.cs[0].flush();
    rig.tick_holding(Some(0));
    rig.cs[0].click(ContainerClick::Withdraw { slot: 8, all: true }, eye);
    rig.cs[0].flush();
    rig.tick();
    rig.tick();
    rig.report(0);
    assert_eq!(rig.cs[0].count(&Item::Block(block::COBBLESTONE)), 35, "given the one the hopper added");
    let gives: Vec<_> = rig.cs[0].slot_sets.iter().flat_map(|s| s.give.iter()).collect();
    assert_eq!(gives, vec![&crate::inventory::stack_to_wire(&cobble(1))], "one correction: give 1");
    assert_eq!(rig.world_total(&Item::Block(block::COBBLESTONE), chest), (35, 35));
    let mirror = rig.cs[0].mirror.as_ref().unwrap().as_ref().prints();
    assert_eq!(Some(mirror), rig.server_container(0).map(|c| c.prints()), "the mirror shows the real chest");
}

/// C-M1 — a furnace output click while a cook's push is on its way: the
/// joiner takes the one ingot its view shows; the furnace has cooked a
/// second by then. It is given the second, the output it shows is set to
/// the real (empty) one after the push, and no ingot is made or lost.
#[test]
fn a_furnace_output_click_during_a_cook_push_takes_what_the_furnace_really_held() {
    let mut rig = Rig::dedicated("furnace-push", 1);
    let furnace = rig.place(-1, 2, block::FURNACE);
    let pos = (furnace[0], furnace[1], furnace[2]);
    let data = crate::furnace::FurnaceData {
        input: Some(ItemStack::new_material(MaterialId::RawIron, 2)),
        fuel: Some(ItemStack::new_material(MaterialId::Coal, 2)),
        ..Default::default()
    };
    rig.world().insert_furnace(pos, data);
    rig.open(0, furnace);
    let ingot = Item::Material(MaterialId::IronIngot);
    let output = |rig: &Rig| rig.world_ref().furnace_at(pos).and_then(|f| f.output.clone()).map_or(0, |s| s.count);
    // Cook quickly: the test hurries each smelt to its last tick.
    let hurry = |rig: &mut Rig| {
        if let Some(f) = rig.world().furnace_at_mut(pos)
            && f.smelt_total > 1
            && f.smelt_progress + 1 < f.smelt_total
        {
            f.smelt_progress = f.smelt_total - 1;
        }
    };
    for _ in 0..40 {
        if output(&rig) == 1 {
            break;
        }
        hurry(&mut rig);
        rig.tick();
    }
    assert_eq!(output(&rig), 1, "one cooked");
    rig.tick();
    rig.report(0);
    // The second cooks while the joiner hears nothing.
    for _ in 0..40 {
        if output(&rig) == 2 {
            break;
        }
        hurry(&mut rig);
        rig.tick_holding(Some(0));
    }
    assert_eq!(output(&rig), 2, "a second cooked: its push is on its way");
    let take = ContainerClick::Furnace { kind: SlotKind::Output, mode: ClickMode::Stack, hotbar: 0 };
    let eye = rig.eye(0);
    assert_eq!(rig.cs[0].click(take, eye), ClickResult::Done);
    assert_eq!(rig.cs[0].count(&ingot), 1, "predicted: the one its view shows");
    rig.cs[0].flush();
    rig.tick();
    rig.report(0);
    assert_eq!(rig.cs[0].count(&ingot), 2, "given the second");
    assert_eq!(output(&rig), 0, "the real output is empty");
    let crate::container_window::ContainerData::Furnace(f) = &rig.cs[0].mirror.as_ref().unwrap().contents else {
        panic!("a furnace mirror")
    };
    assert_eq!(f.output, None, "the mirror shows it empty, after the push");
    let server: u32 = rig.sp(0).inventory.slots_iter().flatten().filter(|s| s.item == ingot).map(|s| u32::from(s.count)).sum();
    assert_eq!(server, 2, "the server's copy holds both, in the client's order");
    assert_eq!(rig.tally(0).window_mismatch, 0);
}

/// C-M2 (decision 3) — a modified client sends `Container(Sort)` with no
/// container open, claiming 64 diamonds it touched. Refused: the server's
/// copy gains nothing (a refused op never moves it), and with no view sent
/// there is nothing to undo on the client.
#[test]
fn a_container_op_with_nothing_open_claiming_diamonds_gives_the_server_copy_nothing() {
    let mut rig = Rig::dedicated("no-container-claims", 1);
    let diamonds = ItemStack::new_material(MaterialId::Diamond, 64);
    for n in 0..8u32 {
        let pkt = protocol::WindowOpPacket {
            op_seq: n + 1,
            op: protocol::WireWindowOp::Container(ContainerClick::Sort),
            digest: 0,
            events_applied: 0,
            touched: vec![WireWindowSlot::Inv(0)],
            claims: vec![(WireWindowSlot::Inv(0), Some(crate::inventory::stack_to_wire(&diamonds)))],
            client_ok: true,
        };
        rig.cs[0].transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
    }
    rig.tick();
    rig.tick();
    rig.report(0);
    let sp = rig.sp(0);
    assert!(sp.inventory.slots_iter().all(|s| s.is_none()), "the server's copy gained nothing");
    assert_eq!(sp.cursor, None);
    assert_eq!(rig.tally(0).container_refused, 8);
    assert!(rig.cs[0].slot_sets.is_empty(), "nothing to undo: no view was ever sent");
    assert_eq!(rig.tally(0).window_mismatch, 0, "container convergence, not lockstep");
}

/// C-L3 (decision 4) — believed deposits are bounded per joiner: 64 units,
/// refilled at 4 a second. A deposit past the bound is refused and
/// corrected: the client gets its stack back and the real chest gains
/// nothing. A second later 4 more units are believed.
#[test]
fn believed_deposits_past_the_bound_are_refused_and_corrected() {
    let mut rig = Rig::dedicated("believed-bound", 1);
    for slot in [0, 1, 2] {
        rig.cs[0].inv.set_slot(slot, Some(fish(64))); // the client's only: caught locally
    }
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest((chest[0], chest[1], chest[2]), ChestData::new());
    rig.open(0, chest);
    let eye = rig.eye(0);
    let fish_item = Item::Material(MaterialId::RawFish);
    let in_chest = |rig: &Rig| -> u32 {
        rig.world_ref().chest_at((chest[0], chest[1], chest[2])).map_or(0, |c| {
            c.slots.iter().flatten().filter(|s| s.item == fish_item).map(|s| u32::from(s.count)).sum()
        })
    };
    rig.cs[0].click(ContainerClick::Deposit { slot: 0, all: true }, eye);
    rig.cs[0].flush();
    rig.tick();
    assert_eq!(in_chest(&rig), 64, "the first stack is believed");
    assert_eq!(rig.tally(0).container_believed, 64);
    // The bound is spent: the next stack is refused, and the client's
    // prediction undone.
    rig.cs[0].click(ContainerClick::Deposit { slot: 1, all: true }, eye);
    assert_eq!(rig.cs[0].count(&fish_item), 64, "predicted: the second stack went in");
    rig.cs[0].flush();
    rig.tick();
    assert_eq!(in_chest(&rig), 64, "refused: the real chest gains nothing");
    assert_eq!(rig.cs[0].count(&fish_item), 128, "corrected: the stack came back");
    let t = rig.tally(0);
    assert_eq!((t.container_believed, t.container_refused, t.container_corrected), (64, 1, 1));
    let mirror = rig.cs[0].mirror.as_ref().unwrap().as_ref().prints();
    assert_eq!(Some(mirror), rig.server_container(0).map(|c| c.prints()), "its mirror is the real chest again");
    // A second refills four units.
    for _ in 0..20 {
        rig.tick();
    }
    rig.cs[0].click(ContainerClick::Deposit { slot: 2, all: false }, eye);
    rig.cs[0].flush();
    rig.tick();
    assert_eq!(in_chest(&rig), 65, "one unit within the refill is believed");
    assert_eq!(rig.tally(0).container_believed, 65);
    // A real deposit (the server's copy holds it) is never bounded.
    rig.give(0, 5, Item::Block(block::DIRT), 64);
    rig.cs[0].click(ContainerClick::Deposit { slot: 5, all: true }, eye);
    rig.cs[0].flush();
    rig.tick();
    assert_eq!(rig.world_total(&Item::Block(block::DIRT), chest).0, 64);
    assert_eq!(rig.tally(0).container_refused, 1, "not refused");
    assert_eq!(real_slot(&rig, chest, 2), Some(ItemStack::new_block(block::DIRT, 64)), "the dirt went in");
}

/// C-L3 (decision 4) — a claim no honest client can make is refused: a
/// stack above its item's `max_stack()`, or a tool counted above one. The
/// real chest gains nothing.
#[test]
fn claims_above_a_stack_are_refused() {
    let mut rig = Rig::dedicated("over-stack", 1);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest((chest[0], chest[1], chest[2]), ChestData::new());
    rig.open(0, chest);
    let pick = ItemStack {
        item: Item::Tool(crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Diamond)),
        count: 2,
    };
    for (n, claim) in [ItemStack::new_block(block::STONE, 65), pick].into_iter().enumerate() {
        let pkt = protocol::WindowOpPacket {
            op_seq: rig.cs[0].seq + 1 + n as u32,
            op: protocol::WireWindowOp::Container(ContainerClick::Deposit { slot: 0, all: true }),
            digest: 0,
            events_applied: rig.cs[0].events,
            touched: vec![WireWindowSlot::Inv(0), WireWindowSlot::Container(0)],
            claims: vec![(WireWindowSlot::Inv(0), Some(crate::inventory::stack_to_wire(&claim)))],
            client_ok: true,
        };
        rig.cs[0].transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
    }
    rig.tick();
    rig.tick();
    assert!(real_slot(&rig, chest, 0).is_none(), "the real chest gained nothing");
    let t = rig.tally(0);
    assert_eq!((t.container_refused, t.container_believed), (2, 0));
}

/// C-L5 (decision 6) — a joined client's Bulk vendor pulls no stock from an
/// adjacent chest: its chests are never the real ones (a worldgen loot
/// chest's generated copy, or nothing). Pinned on the game loop's source,
/// as the vendor dialog needs a GPU.
#[test]
fn a_joined_clients_bulk_vendor_pulls_nothing_from_an_adjacent_chest() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("C-L5 lint: cannot read {} ({e})", path.display()));
    let call = "crate::rail::depot_chest_for(vpos,";
    let at = raw.find(call).unwrap_or_else(|| panic!("{call} is gone from game_loop.rs: update this lint, don't delete it"));
    assert_eq!(raw.matches(call).count(), 1, "{call}: one call site");
    let before = &raw[at.saturating_sub(200)..at];
    assert!(before.contains("(!self.joined())"), "the depot pull must be gated on not joined");
}

// ── C3b-fix-c — phantoms spent outside containers, unfits backed, armour ──

fn chestplate(durability: Option<u16>) -> ArmourItem {
    let piece = ArmourItem::new(crate::armour::ArmourSlot::Chestplate, crate::armour::ArmourMaterial::Iron);
    ArmourItem { durability: durability.unwrap_or(piece.durability), ..piece }
}

fn armour_stack(piece: ArmourItem) -> ItemStack {
    ItemStack { item: Item::Armour(piece), count: 1 }
}

fn chest_cell(chest: [i32; 3]) -> (i32, i32, i32) {
    (chest[0], chest[1], chest[2])
}

/// B-M1 scenario 1 (decision 6) — the loser of a race for 16 cobblestone
/// closes the chest, places 3 and Q-drops 1 before its correction lands. The
/// server's copy held the phantom, so it paid all four; the correction then
/// takes the 12 left and falls 4 short. Those 4 show as `correction_short`
/// (they are a dupe until C3d judges every spend against the effective
/// window).
#[test]
fn a_phantom_placed_and_dropped_inside_the_round_trip_shows_as_a_correction_short() {
    let mut rig = Rig::dedicated("phantom-spent", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(cobble(16));
    rig.world().insert_chest(chest_cell(chest), contents);
    rig.open(0, chest);
    rig.open(1, chest);
    let (loser, winner) = loser_and_winner(&rig);
    let item = Item::Block(block::COBBLESTONE);
    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    assert_eq!(rig.cs[loser].inv.slot(0), Some(&cobble(16)), "predicted into hotbar slot 0");
    rig.cs[winner].flush();
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    // Inside the correction's round trip: close, place 3, Q-drop 1.
    rig.cs[loser].close();
    rig.cs[loser].flush();
    let base = [rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32];
    for (dx, dz) in [(2, 0), (2, -1), (-2, 0)] {
        rig.place_from_hotbar(loser, [base[0] + dx, base[1], base[2] + dz], block::COBBLESTONE, &item);
    }
    rig.drop_from_hotbar(loser, &item);
    rig.tick_holding(Some(loser));
    rig.tick_holding(Some(loser));
    assert_eq!(rig.tally(loser).matched, 3, "the three placements were paid from the phantom");
    assert_eq!(rig.tally(loser).drops, 1, "the drop was spawned");
    assert_eq!(rig.sp(loser).inventory.slot(0), Some(&cobble(12)), "the server's copy paid all four");
    // The correction lands on both sides: 12 taken, 4 short.
    rig.tick();
    rig.report(loser);
    assert_eq!(rig.sp(loser).inventory.slot(0), None);
    assert_eq!(rig.cs[loser].count(&item), 0);
    assert_eq!(rig.tally(loser).correction_short, 4, "the take that fell short shows");
    assert!(rig.tally(loser).summary("Keeper").unwrap().contains("4 unit(s) a container correction took short"));
}

/// B-L6 (decision 7) — the loser of a race hears nothing for longer than the
/// valve: its correction is applied to the server's copy without its word.
/// It then deposits its phantom back, an op made before it applied that
/// correction: the phantom is still debited, never believed, and the world
/// holds 16 stone.
#[test]
fn a_correction_the_valve_forced_still_debits_an_op_made_before_the_client_applied_it() {
    let mut rig = Rig::dedicated("forced-phantom", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(stone(16));
    rig.world().insert_chest(chest_cell(chest), contents);
    rig.open(0, chest);
    rig.open(1, chest);
    let (loser, winner) = loser_and_winner(&rig);
    let stone_item = Item::Block(block::STONE);
    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    rig.cs[winner].flush();
    rig.cs[loser].flush();
    for _ in 0..=crate::window_events::EVENT_ACK_TIMEOUT_TICKS {
        rig.tick_holding(Some(loser));
    }
    assert!(rig.sp(loser).window_events.tally.forced >= 1, "the valve applied the correction");
    assert_eq!(window_units(&rig.sp(loser).inventory, &None, &Default::default(), &[None; 4], &stone_item), 0);
    // Still before it applied the correction: it deposits its phantom.
    let at = rig.cs[loser].inv.slots_iter().position(|s| s.is_some_and(|s| s.item == stone_item)).expect("the phantom");
    assert_eq!(rig.cs[loser].click(ContainerClick::Deposit { slot: at, all: true }, el), ClickResult::Done);
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    assert_eq!(rig.tally(loser).container_believed, 0, "the phantom was debited, not believed");
    rig.tick();
    rig.report(loser);
    rig.report(winner);
    assert_eq!(rig.world_total(&stone_item, chest), (16, 16), "16 stone in the world");
    assert!(crate::window_events::phantom(rig.sp(loser)).is_empty(), "the client applied it: the ledger lets it go");
}

/// B-M2 (decision 4 a) — a modified client's deposit into room its view
/// shows but the real chest no longer has, claiming 64 cobblestone the
/// server's copy doesn't hold: R puts nothing in, and the correction gives
/// the client its stack back. A `GrantUnfit` claiming that give didn't fit
/// spawns nothing: the server's copy never gave anything up.
#[test]
fn a_claimed_unfit_for_a_give_back_from_stale_room_spawns_nothing() {
    let mut rig = Rig::dedicated("stale-room-unfit", 1);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest(chest_cell(chest), ChestData::new());
    let item = Item::Block(block::COBBLESTONE);
    rig.cs[0].inv.set_slot(0, Some(cobble(64))); // the client's only
    rig.open(0, chest);
    // The real chest fills; the push is still on its way.
    if let Some(c) = rig.world().chest_at_mut(chest_cell(chest)) {
        for s in c.slots.iter_mut() {
            *s = Some(ItemStack::new_block(block::DIRT, 64));
        }
    }
    rig.tick_holding(Some(0));
    let eye = rig.eye(0);
    assert_eq!(rig.cs[0].click(ContainerClick::Deposit { slot: 0, all: true }, eye), ClickResult::Done, "room in its view");
    rig.cs[0].flush();
    rig.tick();
    rig.report(0);
    assert_eq!(rig.cs[0].count(&item), 64, "the correction gave the stack back");
    let event = rig.last_give(0);
    let before = rig.world_total(&item, chest).1;
    rig.send_action(0, protocol::ItemAction::GrantUnfit { event, count: 64 });
    rig.tick();
    rig.tick();
    assert_eq!(rig.ground(&item), 0, "nothing spawned from nothing");
    assert_eq!(rig.world_total(&item, chest).1, before, "the server's view of the world is unchanged");
    assert_eq!(rig.sp(0).window_events.tally.unfit_unbacked, 64);
}

/// B-M2 (decision 4 b) — the Restock shape: the loser of a race (its 16
/// cobblestone a phantom) Restocks inside the round trip with a full window.
/// The phantom debit gives R room P lacked, so the client is given 16 while
/// the server's roomier copy (it lacks the client's local dirt) restocked
/// more, and owes 4 back. A claimed unfit of the 16 spawns only what the
/// server's copy gives up: the world's cobblestone is conserved.
#[test]
fn a_claimed_unfit_after_a_restock_on_debited_claims_spawns_only_what_the_copy_gave_up() {
    let mut rig = Rig::dedicated("restock-unfit", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(cobble(16));
    contents.slots[1] = Some(cobble(20));
    rig.world().insert_chest(chest_cell(chest), contents);
    let (loser, winner) = loser_and_winner(&rig);
    let item = Item::Block(block::COBBLESTONE);
    rig.give(loser, 0, item.clone(), 48);
    for s in 2..36 {
        rig.give(loser, s, Item::Block(block::DIRT), 64);
    }
    rig.cs[loser].inv.set_slot(1, Some(ItemStack::new_block(block::DIRT, 64))); // the client's only
    rig.open(0, chest);
    rig.open(1, chest);
    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    rig.cs[winner].flush();
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    rig.cs[loser].click(ContainerClick::Restock, el);
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    rig.tick();
    rig.report(loser);
    rig.report(winner);
    assert_eq!(rig.world_total(&item, chest).1, 84, "48 + 16 + 20, on the server's side");
    assert_eq!(rig.cs[loser].count(&item), 64, "given the 16 R restocked");
    let event = rig.last_give(loser);
    // The winner steps away, so the stack lies where it lands.
    rig.sp(winner).player.pos.x += 7.0;
    rig.send_action(loser, protocol::ItemAction::GrantUnfit { event, count: 16 });
    rig.tick();
    rig.tick();
    assert_eq!(rig.ground(&item), 16, "what the server's copy gave up");
    assert_eq!(rig.world_total(&item, chest).1, 84, "conserved");
}

/// B-M2 (decision 4 c) — an honest give that really fits nowhere: a hopper
/// added one cobblestone the joiner's withdraw didn't see, and a second
/// withdraw inside the round trip filled its last room. The correction's
/// give of 1 fits neither the client nor the server's copy: exactly that one
/// comes back to the world.
#[test]
fn an_honest_correction_give_that_fits_nowhere_spawns_exactly_what_came_back() {
    let mut rig = Rig::dedicated("true-unfit", 1);
    let item = Item::Block(block::COBBLESTONE);
    for s in 1..36 {
        rig.give(0, s, Item::Block(block::DIRT), 64);
    }
    let chest = rig.place(0, 2, block::CHEST);
    let mut contents = ChestData::new();
    contents.slots[7] = Some(cobble(19));
    contents.slots[8] = Some(cobble(45));
    hopper_fed(&mut rig, chest, contents, 1);
    rig.open(0, chest);
    for _ in 0..crate::hopper::HOPPER_INTERVAL_TICKS * 3 {
        if real_slot(&rig, chest, 7) == Some(cobble(20)) {
            break;
        }
        rig.tick_holding(Some(0));
    }
    assert_eq!(real_slot(&rig, chest, 7), Some(cobble(20)));
    let eye = rig.eye(0);
    rig.cs[0].click(ContainerClick::Withdraw { slot: 7, all: true }, eye);
    rig.cs[0].flush();
    rig.tick_holding(Some(0));
    rig.cs[0].click(ContainerClick::Withdraw { slot: 8, all: true }, eye);
    assert_eq!(rig.cs[0].inv.slot(0), Some(&cobble(64)), "its last room filled");
    rig.cs[0].flush();
    rig.tick();
    rig.tick();
    rig.report(0);
    assert_eq!(rig.cs[0].unfit, 1, "the client reported the give that didn't fit");
    assert_eq!(rig.ground(&item), 1, "exactly what came back");
    assert_eq!(rig.world_total(&item, chest), (65, 65), "19 + 45 + the hopper's 1");
    let t = rig.sp(0).window_events.tally;
    assert_eq!((t.unfit_returned, t.unfit_unbacked), (1, 0));
}

/// B-M3 (decision 5 a) — a modified client deposits an iron chestplate whose
/// durability differs from the one the server's copy wears (and the copy
/// holds no other): no take can pay it, so it is believed — it costs the
/// bound and is tallied — and the copy keeps wearing its own.
#[test]
fn a_claimed_chestplate_unlike_the_one_worn_is_believed() {
    let mut rig = Rig::dedicated("believed-armour", 1);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest(chest_cell(chest), ChestData::new());
    rig.open(0, chest);
    let worn = chestplate(Some(50));
    rig.sp(0).armour[1] = Some(worn);
    let claimed = chestplate(None);
    let claims = vec![(WireWindowSlot::Inv(0), Some(crate::inventory::stack_to_wire(&armour_stack(claimed))))];
    rig.send_op(0, ContainerClick::Deposit { slot: 0, all: true }, vec![WireWindowSlot::Inv(0), WireWindowSlot::Container(0)], claims);
    rig.tick();
    rig.tick();
    assert_eq!(real_slot(&rig, chest, 0), Some(armour_stack(claimed)));
    assert_eq!(rig.tally(0).container_believed, 1, "believed, tallied");
    assert_eq!(rig.sp(0).container_sent.believed.units(), crate::window_ops::BELIEVED_BUCKET_UNITS - 1, "it cost the bound");
    assert_eq!(rig.sp(0).armour[1], Some(worn), "the copy still wears its own");
}

/// B-M3 (decision 5 b) — the same claim with the durability the copy's worn
/// chestplate has: the world's chestplates are conserved — either the
/// deposit is believed, or the server's copy gives up the one it wears.
#[test]
fn a_claimed_chestplate_like_the_one_worn_is_paid_or_believed_never_neither() {
    let mut rig = Rig::dedicated("worn-armour", 1);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest(chest_cell(chest), ChestData::new());
    rig.open(0, chest);
    let worn = chestplate(Some(50));
    rig.sp(0).armour[1] = Some(worn);
    let claims = vec![(WireWindowSlot::Inv(0), Some(crate::inventory::stack_to_wire(&armour_stack(worn))))];
    rig.send_op(0, ContainerClick::Deposit { slot: 0, all: true }, vec![WireWindowSlot::Inv(0), WireWindowSlot::Container(0)], claims);
    rig.tick();
    rig.tick();
    assert_eq!(real_slot(&rig, chest, 0), Some(armour_stack(worn)), "it went in");
    let total = rig.world_total(&Item::Armour(worn), chest).1;
    assert_eq!(total, 1 + rig.tally(0).container_believed, "one chestplate, unless the deposit was believed");
    assert_eq!(rig.tally(0).correction_short, 0, "the take found it");
}

/// B-M3 (decision 5 c) — a claimed fresh pickaxe while the server's copy
/// holds a worn one: believed, never paid by taking the worn one (which
/// would repair it).
#[test]
fn a_claimed_fresh_pickaxe_while_the_copy_holds_a_worn_one_is_believed() {
    let mut rig = Rig::dedicated("believed-pick", 1);
    let chest = rig.place(0, 2, block::CHEST);
    rig.world().insert_chest(chest_cell(chest), ChestData::new());
    rig.open(0, chest);
    let fresh = crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Diamond);
    let worn = crate::crafting::Tool { durability: fresh.durability - 100, ..fresh };
    rig.sp(0).inventory.set_slot(5, Some(ItemStack::new_tool(worn)));
    let claims = vec![(WireWindowSlot::Inv(0), Some(crate::inventory::stack_to_wire(&ItemStack::new_tool(fresh))))];
    rig.send_op(0, ContainerClick::Deposit { slot: 0, all: true }, vec![WireWindowSlot::Inv(0), WireWindowSlot::Container(0)], claims);
    rig.tick();
    rig.tick();
    assert_eq!(real_slot(&rig, chest, 0), Some(ItemStack::new_tool(fresh)));
    assert_eq!(rig.tally(0).container_believed, 1, "believed, tallied");
    assert_eq!(rig.sp(0).inventory.slot(5), Some(&ItemStack::new_tool(worn)), "the worn one stays: no repair");
}

/// B-M1 scenario 2 (decision 2) — the loser of a race for an iron
/// chestplate equips it inside the round trip (an ordinary window op, which
/// equips it in the server's copy too). Its correction's take reaches the
/// armour slots on both sides: no chestplate is duplicated.
#[test]
fn a_phantom_chestplate_equipped_inside_the_round_trip_is_taken_by_its_correction() {
    let mut rig = Rig::dedicated("equipped-phantom", 2);
    let chest = rig.place(0, 2, block::CHEST);
    let plate = chestplate(None);
    let mut contents = ChestData::new();
    contents.slots[0] = Some(armour_stack(plate));
    rig.world().insert_chest(chest_cell(chest), contents);
    rig.open(0, chest);
    rig.open(1, chest);
    let (loser, winner) = loser_and_winner(&rig);
    let take = ContainerClick::Withdraw { slot: 0, all: true };
    let (ew, el) = (rig.eye(winner), rig.eye(loser));
    rig.cs[winner].click(take.clone(), ew);
    rig.cs[loser].click(take, el);
    rig.cs[winner].flush();
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    // Inside the round trip: pick it up and put it on.
    let at = rig.cs[loser].inv.slots_iter().position(|s| s.is_some_and(|s| s.item == Item::Armour(plate))).expect("the phantom");
    for click in [window::WindowClick::Slot { slot: at, right: false }, window::WindowClick::Armour { slot: 1 }] {
        let c = &mut rig.cs[loser];
        assert!(c.ui.apply_click(&mut c.inv, &mut c.armour, &click, false, el, |_| block::AIR).ok());
    }
    assert_eq!(rig.cs[loser].armour[1], Some(plate), "worn");
    rig.cs[loser].flush();
    rig.tick_holding(Some(loser));
    assert_eq!(rig.sp(loser).armour[1], Some(plate), "the server's copy wears the phantom too");
    rig.tick();
    rig.report(loser);
    rig.report(winner);
    assert_eq!((rig.cs[loser].armour[1], rig.sp(loser).armour[1]), (None, None), "taken off on both sides");
    assert_eq!(rig.world_total(&Item::Armour(plate), chest), (1, 1), "the winner's, alone");
    assert_eq!(rig.tally(loser).correction_short, 0);
}
