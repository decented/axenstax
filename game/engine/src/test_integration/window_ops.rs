//! C3a-2a (2026-10-08, protocol v75) — the server mirrors a joiner's
//! inventory window, click for click.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport (a
//! dedicated server, or a lending host) and a joiner whose client half is the
//! real one: a `CraftingUi` with the player's `Inventory` and armour, clicked
//! through `CraftingUi::apply_click` / `close` / `open_*` (each logs its op
//! with the window's digest after it), its log drained with `take_ops` and
//! sent numbered, as `GameState::flush_window_ops` and
//! `RemoteClient::send_window_op` send it. The tests compare the server's
//! copy of the window with the client's, slot for slot.
//!
//! C3a-fix-1 (v76) — the client also applies what the server sends that
//! changes its window (grants, request outcomes, armour wear), in arrival
//! order, by the client's own functions, after a simulated latency
//! (`Client::latency`), counts the window events it applied, and reports the
//! count on every op and on the input it sends each tick, as
//! `RemoteClient` does: so an event and an op can cross, and the server must
//! still end in lockstep.

use std::collections::VecDeque;

use glam::Vec3;

use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
use crate::block;
use crate::craft_ui::CraftingUi;
use crate::crafting::{CraftSlot, Tool, ToolMaterial, ToolType};
use crate::hosted_server::{HostedServer, RemoteTransport, MAX_WINDOW_OPS_PER_TICK};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::joiner_actions::{Asked, JoinerActions, Pending};
use crate::protocol::{self, PlayerEventType, WindowOpPacket};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};
use crate::window::{self, ClickResult, Station, WindowClick, WindowSlot};
use crate::window_ops::OpKind;

use super::joiner_authority::join_guest;
use super::joiners_act::floor_and_stand;
use super::lent_world::{join_guest_lent, start_lent};

/// The joiner's client: its window, and the ops it has sent.
struct Client {
    transport: ChannelClientTransport,
    slot: usize,
    ui: CraftingUi,
    inv: Inventory,
    armour: [Option<ArmourItem>; 4],
    /// The last `op_seq` sent.
    seq: u32,
    /// `ArmourWorn` hits it was told of.
    worn: u32,
    /// C3a-fix-1 — the highest server window event it applied.
    events: u32,
    /// Packets from the server not applied yet, each with the rig tick it
    /// arrives on.
    inbox: VecDeque<(u64, Vec<u8>)>,
    /// Ticks a server packet takes to arrive: 0 = applied in the tick it was
    /// sent.
    latency: u64,
    /// The rig's tick count.
    now: u64,
    /// Its requests in flight (`joiner_actions`).
    actions: JoinerActions,
    /// The last input's sequence number.
    input_seq: u64,
    /// Grants received.
    grants: u32,
    /// `GrantUnfit` units reported back.
    unfit: u32,
}

/// One joiner standing on a stone floor round (40, 80, 40), on a dedicated
/// server or a lending host (`host`: the host client's world and ECS).
struct Rig {
    hs: HostedServer,
    host: Option<OwnedSimParts>,
    c: Client,
    at: Vec3,
}

impl Rig {
    fn dedicated(tag: &str) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("window-ops-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let joined = join_guest(&mut hs, "Tinkerer");
        Self::stand(hs, None, joined)
    }

    fn lent(tag: &str) -> Self {
        let (mut hs, mut host) = start_lent(&format!("window-ops-{tag}"));
        let joined = join_guest_lent(&mut hs, &mut host, "Tinkerer");
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
        let at = floor_and_stand(&mut world, &mut hs, slot);
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
            worn: 0,
            events: 0,
            inbox: VecDeque::new(),
            latency: 0,
            now: 0,
            actions: JoinerActions::default(),
            input_seq: 0,
            grants: 0,
            unfit: 0,
        };
        let mut rig = Rig { hs, host, c, at };
        rig.tick();
        rig
    }

    /// One server tick, then the client: what arrived by now is applied in
    /// arrival order, and its input goes out reporting the events applied.
    fn tick(&mut self) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        self.c.now += 1;
        while let Some(pkt) = self.c.transport.try_recv_from_server() {
            self.c.inbox.push_back((self.c.now + self.c.latency, pkt));
        }
        while self.c.inbox.front().is_some_and(|(due, _)| *due <= self.c.now) {
            let (_, pkt) = self.c.inbox.pop_front().unwrap();
            self.receive(&pkt);
        }
        self.send_input(&[]);
    }

    /// The client applies one server packet that changes its window, by the
    /// client's own functions, and counts its window event.
    fn receive(&mut self, pkt: &[u8]) {
        let Some((ptype, payload)) = protocol::deserialize_header(pkt) else { return };
        let event = match ptype {
            protocol::PacketType::PlayerEvent => {
                let ev: protocol::PlayerEventPacket = protocol::safe_deserialize(payload).unwrap();
                if let PlayerEventType::ArmourWorn { hits } = ev.event {
                    // The client wears its own armour by the shared rule, as
                    // `game_loop` does on `OwnLifeEvent::ArmourWorn`.
                    for _ in 0..hits {
                        window::wear_armour(&mut self.c.armour);
                    }
                    self.c.worn += u32::from(hits);
                }
                ev.window_event
            }
            protocol::PacketType::InventoryGrant => {
                let grant: protocol::InventoryGrantPacket = protocol::safe_deserialize(payload).unwrap();
                self.c.grants += 1;
                let registry = &self.hs.server.registry;
                let unfit = crate::remote_entities::apply_inventory_grant(&mut self.c.inv, &grant, registry).unwrap_or(0);
                self.c.events = self.c.events.max(grant.window_event);
                if unfit > 0 {
                    self.c.unfit += u32::from(unfit);
                    self.send_action(protocol::ItemAction::GrantUnfit { event: grant.window_event, count: unfit });
                }
                return;
            }
            protocol::PacketType::ItemActionOutcome => {
                let out: protocol::ItemActionOutcomePacket = protocol::safe_deserialize(payload).unwrap();
                if let Some(request) = self.c.actions.take(out.seq) {
                    let c = &mut self.c;
                    crate::joiner_actions::apply_item_outcome(&mut c.inv, &mut c.ui, &request, &out);
                }
                out.window_event
            }
            protocol::PacketType::InteractOutcome => {
                let out: protocol::InteractOutcomePacket = protocol::safe_deserialize(payload).unwrap();
                if let Some(request) = self.c.actions.take(out.seq) {
                    let c = &mut self.c;
                    crate::joiner_actions::apply_outcome(&mut c.inv, &mut c.ui, &request, &out);
                }
                out.window_event
            }
            _ => return,
        };
        self.c.events = self.c.events.max(event);
    }

    /// The client's input for this tick: standing still, carrying `edits`
    /// and the events it has applied.
    fn send_input(&mut self, edits: &[((i32, i32, i32), block::BlockId)]) {
        self.c.input_seq += 1;
        let sp = &self.hs.server.players[self.c.slot];
        let input = protocol::InputPacket {
            tick: self.c.input_seq,
            x: sp.player.pos.x,
            y: sp.player.pos.y,
            z: sp.player.pos.z,
            yaw: sp.yaw,
            pitch: sp.pitch,
            health: 20.0,
            block_changes: edits
                .iter()
                .map(|&((x, y, z), b)| protocol::BlockChange { x, y, z, new_block: b, meta: 0 })
                .collect(),
            events_applied: self.c.events,
            ..Default::default()
        };
        self.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// One item action from the client, reporting the events it applied.
    fn send_action(&mut self, action: protocol::ItemAction) {
        let seq = self.c.actions.unanswered();
        let pkt = protocol::ItemActionPacket { seq, action, events_applied: self.c.events };
        self.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    }

    /// C3b-fix-b — one input carrying a placement of `placed` at `cell` from
    /// hotbar slot 0, whose hand holds `held` (as the game loop's input does:
    /// the edit and its own hand).
    fn send_placement(&mut self, cell: (i32, i32, i32), placed: block::BlockId, held: &Item) {
        self.c.input_seq += 1;
        let sp = &self.hs.server.players[self.c.slot];
        let (kind, id) = crate::inventory::item_to_ref(held).to_wire();
        let input = protocol::InputPacket {
            tick: self.c.input_seq,
            x: sp.player.pos.x,
            y: sp.player.pos.y,
            z: sp.player.pos.z,
            yaw: sp.yaw,
            pitch: sp.pitch,
            health: 20.0,
            held_kind: kind,
            held_id: id,
            hotbar_slot: Some(0),
            block_changes: vec![protocol::BlockChange { x: cell.0, y: cell.1, z: cell.2, new_block: placed, meta: 0 }],
            edit_hands: vec![(0, kind, id)],
            events_applied: self.c.events,
            ..Default::default()
        };
        self.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// A Q-drop of one `held` from hotbar slot `slot`, as `send_drop_request`
    /// sends it.
    fn send_drop(&mut self, slot: u8, held: &Item) {
        let (held_kind, held_id) = crate::inventory::item_to_ref(held).to_wire();
        let held_full = crate::inventory::item_to_wire_full(held);
        self.send_action(protocol::ItemAction::Drop { hotbar_slot: slot, held_kind, held_id, held_full });
    }

    /// The client asks to eat `held` from hotbar slot `slot` (claimed until
    /// the outcome, as `GameState::send_eat_request` does).
    fn eat(&mut self, slot: usize, held: Item) {
        let (held_kind, held_id) = crate::inventory::item_to_ref(&held).to_wire();
        let held = Some(held);
        let seq = self.c.actions.record(Pending { kind: Asked::Eat, mob: None, hotbar_slot: slot, held }, u64::MAX);
        let action = protocol::ItemAction::Eat {
            hotbar_slot: slot as u8,
            held_kind,
            held_id,
            held_full: protocol::WireItem::None,
        };
        let pkt = protocol::ItemActionPacket { seq, action, events_applied: self.c.events };
        self.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    }

    /// A stack another player dropped, lying at the joiner's feet (no pickup
    /// delay for the joiner).
    fn drop_at_feet(&mut self, stack: ItemStack) {
        let at = self.hs.server.players[self.c.slot].player.pos;
        let ecs = match self.host.as_mut() {
            Some(h) => &mut h.ecs,
            None => &mut self.hs.server.ecs,
        };
        crate::entity::spawn_thrown_item(ecs, at, Vec3::ZERO, stack, 200);
    }

    /// The ground items in the world the server simulates, with where they lie.
    fn ground_items(&self) -> Vec<(Vec3, ItemStack)> {
        let ecs = match self.host.as_ref() {
            Some(h) => &h.ecs,
            None => &self.hs.server.ecs,
        };
        ecs.query::<(&crate::entity::Position, &crate::entity::ItemEntity)>()
            .iter()
            .map(|(_, (p, it))| (p.0, it.stack.clone()))
            .collect()
    }

    fn world(&mut self) -> &mut crate::world::World {
        match self.host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut self.hs.server.world,
        }
    }

    fn sp(&mut self) -> &mut crate::server::ServerPlayer {
        &mut self.hs.server.players[self.c.slot]
    }

    fn tally(&self) -> crate::joiner_inventory::PossessionTally {
        self.hs.server.players[self.c.slot].possession
    }

    /// Put `stack` in slot `slot` on both sides (where the joiner starts).
    fn give(&mut self, slot: usize, item: Item, count: u8) {
        let stack = ItemStack { item, count };
        self.c.inv.set_slot(slot, Some(stack.clone()));
        self.sp().inventory.set_slot(slot, Some(stack));
    }

    /// The client applies `click`, from the eye its server body has, in the
    /// world the joiner sees (the same cells).
    fn click(&mut self, click: WindowClick) -> ClickResult {
        let eye = self.hs.server.players[self.c.slot].player.eye_pos();
        let world = match self.host.as_ref() {
            Some(h) => &h.world,
            None => &self.hs.server.world,
        };
        let c = &mut self.c;
        c.ui.apply_click(&mut c.inv, &mut c.armour, &click, false, eye, |p| world.get_block(p[0], p[1], p[2]))
    }

    fn open_player(&mut self) {
        let c = &mut self.c;
        c.ui.open_player_crafting(&c.inv, &c.armour);
    }

    fn open_table(&mut self, cell: [i32; 3]) {
        let c = &mut self.c;
        c.ui.open_table_crafting(cell, &c.inv, &c.armour);
    }

    fn close(&mut self) -> bool {
        let c = &mut self.c;
        c.ui.close(&mut c.inv, &mut c.armour)
    }

    /// Send every logged op, numbered, as the game loop's flush does, each
    /// reporting the window events applied (none is applied between the
    /// ops being logged and sent here).
    fn flush(&mut self) -> usize {
        let c = &mut self.c;
        let ops = c.ui.take_ops(&c.inv, &c.armour);
        let n = ops.len();
        for logged in ops {
            c.seq += 1;
            let pkt = logged.packet(c.seq, c.events);
            c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
        }
        n
    }

    fn client_digest(&self) -> u32 {
        window::digest_parts(&self.c.inv, &self.c.armour, &self.c.ui.cursor_item, &self.c.ui.grid, self.c.ui.station())
    }

    fn server_digest(&self) -> u32 {
        let sp = &self.hs.server.players[self.c.slot];
        window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station)
    }

    /// The server's window is the client's, slot for slot, and no op has
    /// mismatched.
    fn assert_lockstep(&self, what: &str) {
        let sp = &self.hs.server.players[self.c.slot];
        let slots = |inv: &Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
        assert_eq!(slots(&sp.inventory), slots(&self.c.inv), "{what}: the 36 slots");
        assert_eq!(sp.armour, self.c.armour, "{what}: the armour");
        assert_eq!(sp.cursor, self.c.ui.cursor_item, "{what}: the cursor");
        assert_eq!(sp.craft_grid, self.c.ui.grid, "{what}: the grid");
        assert_eq!(sp.inventory.auto_refill, self.c.inv.auto_refill, "{what}: auto-refill");
        assert_eq!(self.server_digest(), self.client_digest(), "{what}: the digests");
        assert_eq!(sp.possession.window_mismatch, 0, "{what}: no op mismatched");
        assert_eq!(sp.window_events.tally.ops_lost, 0, "{what}: no op lost");
        if self.c.ui.open {
            assert_eq!(sp.station, self.c.ui.station(), "{what}: the station");
        }
    }

    /// Click, send, tick, and check the two windows agree.
    fn step(&mut self, what: &str, click: WindowClick) -> ClickResult {
        let result = self.click(click);
        self.flush();
        self.tick();
        self.assert_lockstep(what);
        result
    }

    /// A crafting table `dz` blocks ahead of the joiner's feet.
    fn place_table(&mut self, dz: i32) -> [i32; 3] {
        let cell = [self.at.x.floor() as i32, self.at.y as i32, self.at.z.floor() as i32 + dz];
        self.world().set_block(cell[0], cell[1], cell[2], block::CRAFTING_TABLE);
        cell
    }
}

fn slot(slot: usize, right: bool) -> WindowClick {
    WindowClick::Slot { slot, right }
}

fn paint(at: WindowSlot) -> WindowClick {
    // The screen paints one slot per frame (`craft_ui::ClickTarget::DragInventory`).
    WindowClick::DragDistribute { slots: vec![at] }
}

fn example(name: &str) -> [[CraftSlot; 3]; 3] {
    crate::crafting_catalogue::all_cards()
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no card named {name}"))
        .example_grid
}

fn helmet() -> Item {
    Item::Armour(ArmourItem::new(ArmourSlot::Helmet, ArmourMaterial::Iron))
}

/// The brief's scripted session: every kind of click the screen makes, on
/// a dedicated server. After every op the server's window equals the
/// client's, slot for slot.
#[test]
fn a_scripted_inventory_session_keeps_the_servers_window_in_lockstep() {
    let mut rig = Rig::dedicated("session");
    rig.give(0, Item::Block(block::STONE), 10);
    rig.give(1, Item::Block(block::OAK_PLANKS), 4);
    rig.give(2, Item::Material(MaterialId::IronIngot), 3);
    rig.give(3, Item::Material(MaterialId::Stick), 2);
    rig.give(5, helmet(), 1);
    rig.give(7, Item::Block(block::DIRT), 5);
    rig.assert_lockstep("the start");

    rig.open_player();
    rig.flush();
    rig.tick();
    rig.assert_lockstep("E opens the inventory");
    assert_eq!(rig.sp().station, Station::Player);

    // Pick up, place, split, merge.
    rig.step("pick up", slot(0, false));
    rig.step("place", slot(9, false));
    rig.step("split (right: the ceil-half)", slot(9, true));
    assert_eq!(rig.c.ui.cursor_item.as_ref().map(|s| s.count), Some(5));
    rig.step("merge", slot(9, false));
    // Drag distribute (one slot a frame, then a multi-slot paint), gather.
    rig.step("pick up again", slot(9, false));
    rig.step("paint a bag slot", paint(WindowSlot::Inv(10)));
    rig.step("paint another", paint(WindowSlot::Inv(11)));
    rig.step("paint a grid cell", paint(WindowSlot::Grid(0, 0)));
    rig.step(
        "a multi-slot paint",
        WindowClick::DragDistribute { slots: vec![WindowSlot::Inv(13), WindowSlot::Inv(14)] },
    );
    rig.step(
        "drag gather",
        WindowClick::DragGather {
            slots: vec![WindowSlot::Inv(10), WindowSlot::Inv(11), WindowSlot::Grid(0, 0), WindowSlot::Inv(13), WindowSlot::Inv(14)],
        },
    );
    assert_eq!(rig.c.ui.cursor_item.as_ref().map(|s| s.count), Some(10), "all ten gathered");
    rig.step("put it down", slot(12, false));
    // Armour equip and unequip.
    rig.step("pick up the helmet", slot(5, false));
    rig.step("equip it", WindowClick::Armour { slot: 0 });
    assert!(rig.sp().armour[0].is_some(), "the server's copy wears it");
    rig.step("unequip it", WindowClick::Armour { slot: 0 });
    rig.step("put it back", slot(5, false));
    // A 2×2 result click: four planks, one per cell, make a crafting table.
    rig.step("pick up the planks", slot(1, false));
    for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        rig.step("lay a plank", paint(WindowSlot::Grid(r, c)));
    }
    assert!(matches!(rig.step("the 2×2 result click", WindowClick::Result), ClickResult::Crafted(_)));
    rig.step("put the table down", slot(20, false));
    assert_eq!(rig.sp().inventory.slot(20).map(|s| s.item.clone()), Some(Item::Block(block::CRAFTING_TABLE)));
    // Lock, sort, trash.
    rig.step("lock", WindowClick::ToggleLock { slot: 12 });
    assert!(rig.sp().inventory.is_locked(12), "the server's copy holds the lock");
    rig.step("sort", WindowClick::Sort);
    rig.step("pick up the dirt", slot(7, false));
    assert!(matches!(rig.step("bin it", WindowClick::Trash), ClickResult::Binned(_)));
    // Close the inventory; open a crafting table in reach.
    assert!(rig.close());
    rig.flush();
    rig.tick();
    rig.assert_lockstep("close");
    let table = rig.place_table(2);
    rig.open_table(table);
    rig.flush();
    rig.tick();
    rig.assert_lockstep("open the table");
    assert_eq!(rig.sp().station, Station::Table { cell: table });
    // Autofill, then the table's result click.
    assert!(rig.step("autofill", WindowClick::Autofill { example: example("Iron Pickaxe") }).ok());
    let crafted = rig.step("the table's result click", WindowClick::Result);
    assert!(matches!(crafted, ClickResult::Crafted(ref s) if matches!(s.item, Item::Tool(t) if t.tool_type == ToolType::Pickaxe)));
    let fresh = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron).durability;
    assert!(
        matches!(rig.sp().cursor.as_ref().map(|s| &s.item), Some(Item::Tool(t)) if t.durability == fresh),
        "the server's cursor holds the pickaxe at full durability"
    );
    // Close with the grid returning: one stone left in a cell, the pickaxe
    // and some stone on the cursor go back to the inventory.
    rig.step("put the pickaxe down", slot(30, false));
    let stone_at = rig
        .c
        .inv
        .slots_iter()
        .position(|s| matches!(s, Some(st) if st.item == Item::Block(block::STONE)))
        .expect("the stone is in the bag");
    rig.step("pick up the stone", slot(stone_at, false));
    rig.step("one into the grid", WindowClick::Grid { row: 2, col: 2, right: true });
    assert!(rig.sp().craft_grid[2][2].is_some());
    assert!(rig.close());
    rig.flush();
    rig.tick();
    rig.assert_lockstep("close with the grid returning");
    assert!(rig.sp().craft_grid.iter().flatten().all(Option::is_none) && rig.sp().cursor.is_none());
    assert_eq!(rig.sp().station, Station::Player, "a close resets the station");

    let t = rig.tally();
    assert_eq!(t.window_ops, rig.c.seq, "every op sent was mirrored");
    assert_eq!(t.window_mismatch, 0);
    assert_eq!((t.window_refused, t.window_noop), (0, 0), "no click of this session is refused");
}

/// Refusal parity — a table result click with the table out of reach (the
/// joiner walked away), or gone (broken), is refused on both sides by the
/// same rule, and neither window changes.
#[test]
fn a_table_result_click_out_of_reach_is_refused_on_both_sides() {
    let mut rig = Rig::dedicated("refusal");
    rig.give(0, Item::Material(MaterialId::IronIngot), 3);
    rig.give(1, Item::Material(MaterialId::Stick), 2);
    let table = rig.place_table(-2);
    rig.open_table(table);
    assert!(rig.step("autofill", WindowClick::Autofill { example: example("Iron Pickaxe") }).ok());
    let laid = rig.client_digest();

    // Walk away: the server body (and so the client's eye) nine blocks off.
    let away = rig.at + Vec3::new(0.0, 0.0, 7.0);
    rig.sp().player.pos = away;
    assert_eq!(rig.step("a result click out of reach", WindowClick::Result), ClickResult::Refused);
    assert_eq!(rig.client_digest(), laid, "the client's window is unchanged");
    assert_eq!(rig.server_digest(), laid, "and so is the server's");
    assert_eq!(rig.tally().window_noop, 1, "both rules refused: a benign no-op");
    assert_eq!(rig.tally().window_refused, 0, "the client did not disagree");
    // Autofill there moves nothing either.
    let fill = WindowClick::Autofill { example: example("Iron Pickaxe") };
    assert_eq!(rig.step("autofill out of reach", fill), ClickResult::NeedsTable);
    assert_eq!(rig.server_digest(), laid);

    // Back in reach, but the table is broken (and the server's grace for a
    // table that just changed, B-L2, has run out).
    let at = rig.at;
    rig.sp().player.pos = at;
    rig.world().set_block(table[0], table[1], table[2], block::AIR);
    for _ in 0..=window::SERVER_TABLE_GRACE_TICKS {
        rig.tick();
    }
    assert_eq!(rig.step("a result click at a broken table", WindowClick::Result), ClickResult::Refused);
    assert_eq!(rig.server_digest(), laid);
    // Rebuilt: the same click crafts on both sides.
    rig.world().set_block(table[0], table[1], table[2], block::CRAFTING_TABLE);
    assert!(matches!(rig.step("in reach again", WindowClick::Result), ClickResult::Crafted(_)));
    assert_eq!(rig.tally().window_noop, 3);
    assert_eq!(rig.tally().window_refused, 0);
    assert_eq!(rig.tally().window_mismatch, 0);
}

/// B-L2 — the server's verdict on a table click is a superset of an honest
/// client's. The client's eye is 6.3 blocks from the table (in reach, 6.37);
/// the server's body, a little further off (knockback while the screen was
/// open), is 6.6 away. The result click crafts on both sides, with no refusal
/// and no mismatch.
#[test]
fn a_table_click_the_client_judged_in_reach_is_accepted_by_the_server_too() {
    let mut rig = Rig::dedicated("slack");
    rig.give(0, Item::Material(MaterialId::IronIngot), 3);
    rig.give(1, Item::Material(MaterialId::Stick), 2);
    let table = rig.place_table(-2);
    rig.open_table(table);
    assert!(rig.step("autofill", WindowClick::Autofill { example: example("Iron Pickaxe") }).ok());

    // The server body is 6.6 blocks (eye to table centre) from the table...
    let centre = Vec3::new(table[0] as f32 + 0.5, table[1] as f32 + 0.5, table[2] as f32 + 0.5);
    let offset = |rig: &mut Rig, d: f32| {
        let eye = rig.sp().player.eye_pos();
        let dy = eye.y - centre.y;
        let dx = eye.x - centre.x;
        let dz = (d * d - dy * dy - dx * dx).sqrt();
        (eye, Vec3::new(eye.x, eye.y, centre.z + dz))
    };
    let (eye, server_eye) = offset(&mut rig, 6.6);
    rig.sp().player.pos += server_eye - eye;
    assert!(((centre - rig.sp().player.eye_pos()).length() - 6.6).abs() < 0.01);
    // ... the client's eye 6.3.
    let (_, client_eye) = offset(&mut rig, 6.3);
    assert!(crate::window::table_in_reach(block::CRAFTING_TABLE, table, client_eye), "in the client's reach");
    assert!(!crate::window::table_in_reach(block::CRAFTING_TABLE, table, rig.sp().player.eye_pos()));

    let result = {
        let world = &rig.hs.server.world;
        let c = &mut rig.c;
        c.ui.apply_click(&mut c.inv, &mut c.armour, &WindowClick::Result, false, client_eye, |p| {
            world.get_block(p[0], p[1], p[2])
        })
    };
    assert!(matches!(result, ClickResult::Crafted(_)), "the client crafts");
    rig.flush();
    rig.tick();
    rig.assert_lockstep("a click at 6.6 from the server body");
    assert_eq!(rig.tally().window_refused, 0, "the server did not refuse it");
    assert_eq!(rig.tally().window_mismatch, 0);
}

/// A-L2 (C3b-fix-a) — the converse of the slack: a client whose forced
/// close is stuck (a full bag, the grid laid) at a table just out of its
/// reach clicks Result. Its rule refuses; the server's slack would accept
/// (its body is as far, inside the half-block slack). The server crafts only
/// what the client's own rule accepted (`client_ok`), so it crafts nothing:
/// the windows stay in lockstep, and both refusals are no-ops.
#[test]
fn a_result_click_the_client_refused_after_a_stuck_forced_close_is_not_crafted() {
    let mut rig = Rig::dedicated("stuck-close");
    rig.give(0, Item::Material(MaterialId::IronIngot), 3);
    rig.give(1, Item::Material(MaterialId::Stick), 2);
    let table = rig.place_table(-2);
    rig.open_table(table);
    assert!(rig.step("autofill", WindowClick::Autofill { example: example("Iron Pickaxe") }).ok());
    // A full bag: a close can't return the grid.
    for i in 0..36 {
        if rig.c.inv.slot(i).is_none() {
            rig.give(i, Item::Block(block::DIRT), 64);
        }
    }
    // The server body, and the client's eye, 6.6 blocks from the table: out
    // of the client's reach (6.37), inside the server's slack.
    let centre = Vec3::new(table[0] as f32 + 0.5, table[1] as f32 + 0.5, table[2] as f32 + 0.5);
    let eye = rig.sp().player.eye_pos();
    let (dx, dy) = (eye.x - centre.x, eye.y - centre.y);
    let dz = (6.6f32 * 6.6 - dy * dy - dx * dx).sqrt();
    rig.sp().player.pos += Vec3::new(eye.x, eye.y, centre.z + dz) - eye;
    let eye = rig.sp().player.eye_pos();
    assert!(!crate::window::table_in_reach(block::CRAFTING_TABLE, table, eye), "out of the client's reach");
    let slack = window::ClickCtx::new(false, Station::Table { cell: table }, eye, |_| block::CRAFTING_TABLE).with_server_slack(false);
    assert!(slack.table_present(), "inside the server's slack");
    // The screen's forced close is stuck: the bag is full.
    let closed = {
        let c = &mut rig.c;
        c.ui.force_close(&mut c.inv, &mut c.armour, true)
    };
    assert!(!closed, "stuck");
    // The client clicks Result there; its rule refuses.
    let result = {
        let world = &rig.hs.server.world;
        let c = &mut rig.c;
        c.ui.apply_click(&mut c.inv, &mut c.armour, &WindowClick::Result, false, eye, |p| world.get_block(p[0], p[1], p[2]))
    };
    assert!(!result.ok(), "the client's rule refuses");
    rig.flush();
    rig.tick();
    rig.assert_lockstep("a result click the client refused, after a stuck forced close");
    let sp = &rig.hs.server.players[rig.c.slot];
    assert!(sp.craft_grid.iter().flatten().any(|c| c.as_ref().is_some_and(|s| s.item == Item::Material(MaterialId::IronIngot))), "the grid is still laid");
    assert!(sp.inventory.slots_iter().flatten().all(|s| !matches!(s.item, Item::Tool(_))), "no pickaxe crafted on the server");
    let t = rig.tally();
    assert_eq!(t.window_refused, 0, "the client's own verdict says it refused too");
    assert_eq!(t.window_noop, 2, "the stuck close and the result click: refused on both sides");
}

/// B-M4 (C3b-fix-c) — a recipe card the player can't afford, clicked with
/// planks in the table's grid: the client's rule returns the grid to the bag
/// first, then refuses (`client_ok = false`). The server runs the same rule
/// with its slack off (`with_server_slack(false)`), so its grid goes back
/// too: lockstep, a no-op on both sides, no mismatch.
#[test]
fn an_autofill_the_client_refused_returns_the_grid_on_the_server_too() {
    let mut rig = Rig::dedicated("autofill-refused");
    rig.give(0, Item::Block(block::OAK_PLANKS), 4);
    let table = rig.place_table(-2);
    rig.open_table(table);
    rig.step("pick the planks up", slot(0, false));
    rig.step("lay them in the grid", WindowClick::Grid { row: 1, col: 1, right: false });
    assert!(rig.c.ui.grid[1][1].is_some(), "planks in the grid");
    let result = rig.step("autofill a recipe it can't afford", WindowClick::Autofill { example: example("Iron Pickaxe") });
    assert_eq!(result, ClickResult::Refused, "the client's rule refused, after returning the grid");
    let sp = &rig.hs.server.players[rig.c.slot];
    assert!(sp.craft_grid.iter().flatten().all(|c| c.is_none()), "the server's grid went back too");
    let t = rig.tally();
    assert_eq!((t.window_refused, t.window_mismatch), (0, 0));
    assert_eq!(t.window_noop, 1, "refused on both sides");
}

/// B-L2 — a table another player just broke: the honest client has not heard,
/// crafts at it, and the server (grace of ten ticks) crafts too. The grace
/// ends, and only in reach.
#[test]
fn a_table_that_just_went_is_still_craftable_on_the_server_for_a_few_ticks() {
    let mut rig = Rig::dedicated("grace");
    rig.give(0, Item::Material(MaterialId::IronIngot), 3);
    rig.give(1, Item::Material(MaterialId::Stick), 2);
    let table = rig.place_table(-2);
    rig.open_table(table);
    assert!(rig.step("autofill", WindowClick::Autofill { example: example("Iron Pickaxe") }).ok());
    // Another player breaks the table; the server's world sees it, the client's
    // (still showing a table) does not. The server has seen it for 3 ticks.
    rig.world().set_block(table[0], table[1], table[2], block::AIR);
    for _ in 0..3 {
        rig.tick();
    }
    assert_eq!(rig.sp().table_gone_ticks, Some(3), "watched, tick by tick");
    let eye = rig.sp().player.eye_pos();
    let client = {
        let c = &mut rig.c;
        c.ui.apply_click(&mut c.inv, &mut c.armour, &WindowClick::Result, false, eye, |_| block::CRAFTING_TABLE)
    };
    assert!(matches!(client, ClickResult::Crafted(_)));
    rig.flush();
    rig.tick();
    rig.assert_lockstep("a click at a table the server saw go 3 ticks ago");
    assert_eq!(rig.tally().window_refused, 0);
    // It stands again: nothing is counted.
    rig.world().set_block(table[0], table[1], table[2], block::CRAFTING_TABLE);
    rig.tick();
    assert_eq!(rig.sp().table_gone_ticks, None, "it stands again");
}

/// B-L2 — the grace ends: a table the server has seen gone for longer than
/// `SERVER_TABLE_GRACE_TICKS` is refused, as the client (who by now has closed
/// the screen) would not have clicked.
#[test]
fn the_grace_for_a_table_that_went_ends() {
    let mut rig = Rig::dedicated("grace-ends");
    rig.give(0, Item::Material(MaterialId::IronIngot), 3);
    rig.give(1, Item::Material(MaterialId::Stick), 2);
    let table = rig.place_table(-2);
    rig.open_table(table);
    assert!(rig.step("autofill", WindowClick::Autofill { example: example("Iron Pickaxe") }).ok());
    rig.world().set_block(table[0], table[1], table[2], block::AIR);
    for _ in 0..=window::SERVER_TABLE_GRACE_TICKS + 1 {
        rig.tick();
    }
    let eye = rig.sp().player.eye_pos();
    let stale = {
        let c = &mut rig.c;
        c.ui.apply_click(&mut c.inv, &mut c.armour, &WindowClick::Result, false, eye, |_| block::CRAFTING_TABLE)
    };
    assert!(matches!(stale, ClickResult::Crafted(_)), "a client that never heard crafts");
    rig.flush();
    rig.tick();
    let t = rig.tally();
    assert_eq!(t.window_refused, 1, "the server refuses it past the grace, and the client's digest says it crafted");
    assert_eq!(t.window_noop, 0, "not a benign no-op");
    assert_eq!(t.window_mismatch, 1);
}

/// B-L5 — a closed connection's queued requests are discarded, whatever their
/// kind, so a leaver's backlog of window ops (eight a tick) doesn't hold its
/// slot: a thousand are reaped within a few ticks, and none is applied.
#[test]
fn a_closed_connection_holding_a_thousand_window_ops_is_reaped() {
    let rig = Rig::dedicated("closed");
    let Rig { mut hs, c, .. } = rig;
    let slot = c.slot;
    for n in 1..=1000u32 {
        let pkt = WindowOpPacket {
            op_seq: n,
            op: protocol::WireWindowOp::Click(WindowClick::Sort),
            digest: 0,
            events_applied: 0,
            touched: Vec::new(),
            claims: Vec::new(),
            client_ok: true,
        };
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
    }
    drop(c.transport); // the link just goes
    let mut freed_at = None;
    for t in 1..=6u32 {
        hs.tick();
        if hs.slot_is_free(slot) && freed_at.is_none() {
            freed_at = Some(t);
        }
    }
    assert!(freed_at.is_some(), "the slot is reaped within a few ticks");
}

/// A grid with no recipe is refused on both sides too.
#[test]
fn a_result_click_on_a_grid_with_no_recipe_is_refused_on_both_sides() {
    let mut rig = Rig::dedicated("no-recipe");
    rig.give(0, Item::Block(block::DIRT), 2);
    rig.open_player();
    rig.step("pick up", slot(0, false));
    rig.step("one dirt", WindowClick::Grid { row: 0, col: 0, right: true });
    assert_eq!(rig.step("craft nothing", WindowClick::Result), ClickResult::Refused);
    assert_eq!(rig.tally().window_noop, 1, "a recipe-less result click refuses on both sides");
    assert_eq!(rig.tally().window_refused, 0);
}

/// Waiting — a burst of 20 ops in one tick is applied over three ticks,
/// `MAX_WINDOW_OPS_PER_TICK` a tick, in the order sent (each one's digest
/// matches, so each found the window its predecessors left).
#[test]
fn a_burst_of_twenty_ops_applies_over_three_ticks_in_order() {
    assert_eq!(MAX_WINDOW_OPS_PER_TICK, 8);
    let mut rig = Rig::dedicated("burst");
    rig.give(0, Item::Block(block::STONE), 7);
    rig.open_player();
    rig.flush();
    rig.tick();
    let before = rig.tally().window_ops;
    // Walk one stack along slots 0 → 10: twenty clicks.
    for i in 0..10 {
        rig.click(slot(i, false));
        rig.click(slot(i + 1, false));
    }
    assert_eq!(rig.flush(), 20);
    for (tick, applied) in [(1, 8), (2, 16), (3, 20)] {
        rig.tick();
        assert_eq!(rig.tally().window_ops - before, applied, "tick {tick}");
    }
    rig.assert_lockstep("after the burst");
    assert_eq!(rig.sp().inventory.slot(10).map(|s| s.count), Some(7));
}

/// Waiting — an op waits behind the same client's edits still waiting past
/// the edit budget (FU4a): the placement it follows changed the window first.
#[test]
fn an_op_waits_behind_its_clients_waiting_edits() {
    let mut rig = Rig::dedicated("behind-edits");
    rig.open_player();
    rig.flush();
    rig.tick();
    let before = rig.tally().window_ops;
    let (x, y, z) = (rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32);
    let edits: Vec<_> = (0..6).map(|i| ((x - 3 + i, y, z + 2), block::GLASS)).collect();
    // The rig's own input (its sequence numbers; `joiner_authority::send_edits`
    // would be stale behind the inputs it sends each tick).
    rig.send_input(&edits);
    rig.click(WindowClick::ToggleLock { slot: 4 });
    rig.flush();
    rig.tick();
    assert_eq!(rig.tally().window_ops, before, "two edits wait past the budget, and the op behind them");
    rig.tick();
    assert_eq!(rig.tally().window_ops, before + 1, "the edits went first, then the op");
    assert!(edits.iter().all(|&((x, y, z), _)| rig.hs.server.world.get_block(x, y, z) == block::GLASS));
    rig.assert_lockstep("after the edits");
}

/// Drift — a change only the client made (a local bucket fill, unmirrored
/// until C3c) makes the next op's digest differ: tallied with the op's kind,
/// and the op is still applied (nothing is refused).
#[test]
fn a_client_only_change_is_tallied_at_the_next_op_and_nothing_is_refused() {
    let mut rig = Rig::dedicated("drift");
    rig.give(4, Item::Material(MaterialId::Bucket), 1);
    rig.give(20, Item::Block(block::DIRT), 3);
    rig.give(30, Item::Block(block::STONE), 3);
    rig.open_player();
    rig.flush();
    rig.tick();
    rig.assert_lockstep("the start");
    // The client fills its bucket itself.
    rig.c.inv.set_slot(4, Some(ItemStack::new_material(MaterialId::WaterBucket, 1)));
    rig.click(WindowClick::Sort);
    rig.flush();
    rig.tick();
    let t = rig.tally();
    assert_eq!(t.window_mismatch, 1, "the next op's digest differs");
    assert_eq!(t.first_window_mismatch, Some(OpKind::Sort));
    assert_eq!(t.window_refused, 0, "nothing is refused");
    assert_eq!(
        rig.sp().inventory.slot(window::BAG_START).map(|s| s.item.clone()),
        rig.c.inv.slot(window::BAG_START).map(|s| s.item.clone()),
        "the sort was applied on the server all the same"
    );
    assert_eq!(rig.sp().inventory.slot(4).map(|s| s.item.clone()), Some(Item::Material(MaterialId::Bucket)));
}

/// A creative joiner is mirrored but not tallied: its item browser's gives
/// stay local until C3c.
#[test]
fn a_creative_joiner_is_mirrored_but_not_tallied() {
    let mut rig = Rig::dedicated("creative");
    rig.hs.server.set_play_mode(crate::play_mode::PlayMode::Creative);
    rig.give(9, Item::Block(block::STONE), 3);
    rig.open_player();
    // A browser give the server never hears of.
    rig.c.inv.set_slot(0, Some(ItemStack::new_block(block::GLASS, 64)));
    rig.click(slot(9, false));
    rig.flush();
    rig.tick();
    let t = rig.tally();
    assert!(t.window_ops >= 2, "mirrored");
    assert_eq!((t.window_mismatch, t.window_refused), (0, 0), "not tallied");
    assert_eq!(rig.sp().cursor, Some(ItemStack::new_block(block::STONE, 3)), "applied all the same");
}

/// A dead joiner's ops are mirrored: its client acted before it heard of
/// the death (`item_actions::can_mirror`, in the world, dead or alive).
#[test]
fn a_dead_joiners_ops_are_still_mirrored() {
    let mut rig = Rig::dedicated("dead");
    rig.give(9, Item::Block(block::STONE), 3);
    rig.open_player();
    rig.flush();
    rig.tick();
    rig.sp().combat.dead = true;
    rig.click(slot(9, false));
    rig.flush();
    rig.tick();
    assert_eq!(rig.sp().cursor, Some(ItemStack::new_block(block::STONE, 3)));
    rig.assert_lockstep("dead");
}

/// Armour wear — a hit the server lands wears the server's copy of the
/// armour piece exactly as the client's `ArmourWorn` wears its own, and a
/// piece that breaks unequips on both sides.
#[test]
fn a_server_hit_wears_the_servers_armour_as_the_client_wears_its_own() {
    let mut rig = Rig::dedicated("armour");
    rig.give(5, helmet(), 1);
    rig.open_player();
    rig.step("pick up the helmet", slot(5, false));
    rig.step("equip it", WindowClick::Armour { slot: 0 });
    let full = rig.sp().armour[0].unwrap().durability;
    let slot_index = rig.c.slot;
    assert!(rig.hs.server.land_hit_on_joiner(
        slot_index,
        2.0,
        crate::survival::DamageCause::Mob(crate::mob::MobType::Bee),
        Vec3::ZERO,
    ));
    rig.tick();
    assert_eq!(rig.c.worn, 1, "the client was told of one hit");
    // C3a-fix-1 — the server wears its copy once the client says it wore its
    // own (its next input), not before.
    assert_eq!(rig.sp().armour[0].unwrap().durability, full, "not before the client's word");
    rig.tick();
    assert_eq!(rig.sp().armour[0].unwrap().durability, full - 1, "the server's piece wore once");
    let server_armour = rig.sp().armour;
    assert_eq!(server_armour, rig.c.armour, "the same as the client's");
    rig.step("an op after the wear", WindowClick::Sort);

    // A piece on its last point breaks and unequips on both sides.
    rig.sp().armour[0].as_mut().unwrap().durability = 1;
    rig.c.armour[0].as_mut().unwrap().durability = 1;
    rig.sp().combat.invincible_timer = 0;
    assert!(rig.hs.server.land_hit_on_joiner(
        slot_index,
        1.0,
        crate::survival::DamageCause::Mob(crate::mob::MobType::Bee),
        Vec3::ZERO,
    ));
    rig.tick();
    rig.tick();
    assert_eq!(rig.sp().armour[0], None);
    assert_eq!(rig.c.armour[0], None);
    assert_eq!(rig.sp().window_events.tally.forced, 0, "every wear on the client's word");
}

/// An accepted request's owed payment is taken from the server's copy of the
/// window by the client's own search: the 36 slots, then the crafting grid,
/// then the cursor (`joiner_actions::take_owed_window`).
#[test]
fn an_accepted_eat_is_paid_from_the_servers_crafting_grid() {
    let mut rig = Rig::dedicated("owed");
    rig.give(0, Item::Material(MaterialId::Bread), 2);
    rig.open_player();
    rig.step("pick up the bread", slot(0, false));
    rig.step("both into the grid", WindowClick::Grid { row: 1, col: 1, right: false });
    rig.sp().combat.hunger = 10;
    rig.eat(0, Item::Material(MaterialId::Bread));
    rig.tick();
    // C3a-fix-1 — the take waits for the client's word: the client paid from
    // its grid when the outcome arrived, then said so.
    assert_eq!(rig.c.ui.grid[1][1].as_ref().map(|s| s.count), Some(1), "the client paid from its grid");
    rig.tick();
    assert_eq!(rig.sp().craft_grid[1][1].as_ref().map(|s| s.count), Some(1), "paid from the server's grid");
    assert_eq!(rig.tally().mismatched, 0, "nothing short");
    rig.assert_lockstep("after the eat");
}

/// On a lending host the server judges the table in the host's world (lent
/// to it each tick).
#[test]
fn a_table_craft_on_a_lending_host_is_judged_in_the_hosts_world() {
    let mut rig = Rig::lent("lent");
    rig.give(0, Item::Material(MaterialId::IronIngot), 3);
    rig.give(1, Item::Material(MaterialId::Stick), 2);
    let table = rig.place_table(2);
    rig.open_table(table);
    assert!(rig.step("autofill", WindowClick::Autofill { example: example("Iron Pickaxe") }).ok());
    assert!(matches!(rig.step("craft", WindowClick::Result), ClickResult::Crafted(_)));
    assert!(matches!(rig.sp().cursor.as_ref().map(|s| &s.item), Some(Item::Tool(_))));
}

// ── C3a-fix-1: the server's window events cross the client's ops ───────

/// B-H1, the reviewer's first scenario — a grant crossing a click. The
/// server picks a log up for the joiner and sends the grant; before it
/// lands, the client puts the dirt on its cursor down in slot 1, and then the
/// log lands in slot 2. The server applies the click first and the grant
/// second, as the client did: the same slots, no mismatch.
#[test]
fn a_grant_crossing_a_click_lands_in_the_same_slot_on_both_sides() {
    let mut rig = Rig::dedicated("grant-cross");
    rig.give(0, Item::Block(block::STONE), 1);
    rig.give(4, Item::Block(block::DIRT), 5);
    rig.open_player();
    rig.step("pick up the dirt", slot(4, false));
    rig.c.latency = 3;
    rig.drop_at_feet(ItemStack::new_block(block::OAK_LOG, 1));
    rig.tick();
    assert!(rig.ground_items().is_empty(), "the server picked the log up for the joiner");
    assert_eq!(rig.c.grants, 0, "the grant is still on its way");
    // The click, before the grant lands.
    rig.click(slot(1, false));
    rig.flush();
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(rig.c.grants, 1);
    assert_eq!(rig.c.inv.slot(1).map(|s| s.item.clone()), Some(Item::Block(block::DIRT)));
    assert_eq!(rig.c.inv.slot(2).map(|s| s.item.clone()), Some(Item::Block(block::OAK_LOG)), "the log, after the click");
    rig.assert_lockstep("the grant crossed the click");
    assert_eq!(rig.sp().window_events.tally.forced, 0);
}

/// B-H1, the second — an owed take crossing a split. An Eat of the bread in
/// slot 2 (3 of them) is accepted; before the outcome lands the client
/// opens its inventory and right-click-splits slot 2 (cursor 2, slot 1),
/// then the outcome takes the slot's last one. The server splits first,
/// then takes: cursor 2, slot empty, on both sides.
#[test]
fn an_owed_take_crossing_a_split_takes_from_the_same_place_on_both_sides() {
    let mut rig = Rig::dedicated("take-cross");
    rig.give(2, Item::Material(MaterialId::Bread), 3);
    rig.sp().combat.hunger = 10;
    rig.c.latency = 3;
    rig.eat(2, Item::Material(MaterialId::Bread));
    rig.tick();
    rig.open_player();
    rig.click(slot(2, true));
    assert_eq!(rig.c.ui.cursor_item.as_ref().map(|s| s.count), Some(2));
    rig.flush();
    for _ in 0..5 {
        rig.tick();
    }
    assert!(rig.c.inv.slot(2).is_none(), "the outcome took the slot's last bread");
    assert_eq!(rig.c.ui.cursor_item.as_ref().map(|s| s.count), Some(2));
    rig.assert_lockstep("the take crossed the split");
    assert_eq!(rig.tally().mismatched, 0, "nothing short");
}

/// B-H1, the third — `ArmourWorn` crossing an unequip. A hit lands on the
/// joiner's helmet; before the wear reaches the client it takes the helmet
/// off (onto the cursor), and then the wear finds nothing equipped. The
/// server unequips first too: the helmet on both cursors, unworn.
#[test]
fn armour_wear_crossing_an_unequip_wears_the_same_on_both_sides() {
    let mut rig = Rig::dedicated("wear-cross");
    rig.give(5, helmet(), 1);
    rig.open_player();
    rig.step("pick up the helmet", slot(5, false));
    rig.step("equip it", WindowClick::Armour { slot: 0 });
    let full = rig.sp().armour[0].unwrap().durability;
    rig.c.latency = 3;
    let slot_index = rig.c.slot;
    assert!(rig.hs.server.land_hit_on_joiner(
        slot_index,
        2.0,
        crate::survival::DamageCause::Mob(crate::mob::MobType::Bee),
        Vec3::ZERO,
    ));
    rig.tick();
    rig.click(WindowClick::Armour { slot: 0 });
    rig.flush();
    for _ in 0..5 {
        rig.tick();
    }
    assert_eq!(rig.c.worn, 1);
    let unworn = |s: &Option<ItemStack>| match s.as_ref().map(|s| &s.item) {
        Some(Item::Armour(a)) => a.durability == full,
        _ => false,
    };
    assert!(unworn(&rig.c.ui.cursor_item), "the client's helmet came off before the wear");
    rig.assert_lockstep("the wear crossed the unequip");
}

/// D-M2 — a really full joiner walks over a stack another player dropped.
/// The server picks it up and grants it whole (the BRIDGE); the client
/// can't hold it and reports it (`GrantUnfit`), spilling nothing of its
/// own; the server spawns it as a real ground item at the joiner's feet, in
/// the host's world, where the host and everyone else see it — and the
/// joiner doesn't vacuum it straight back up.
#[test]
fn a_full_joiner_leaves_a_drop_it_cannot_hold_as_a_real_item_at_its_feet() {
    let mut rig = Rig::lent("full-joiner");
    for i in 0..36 {
        rig.give(i, Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron)), 1);
    }
    let before: Vec<_> = rig.c.inv.slots_iter().map(|s| s.cloned()).collect();
    rig.drop_at_feet(ItemStack::new_material(MaterialId::Stick, 3));
    for _ in 0..4 {
        rig.tick();
    }
    assert_eq!(rig.c.grants, 1, "the server picked it up for the joiner and granted it");
    assert_eq!(rig.c.unfit, 3, "the client couldn't hold it and said so");
    assert_eq!(rig.c.inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>(), before, "nothing changed on the client");
    let feet = rig.hs.server.players[rig.c.slot].player.pos;
    let real = rig.ground_items();
    assert_eq!(real.len(), 1, "one real item, in the host's world");
    assert_eq!(real[0].1, ItemStack::new_material(MaterialId::Stick, 3));
    assert!((real[0].0 - feet).length() < 2.0, "at the joiner's feet");
    assert_eq!(rig.sp().window_events.tally.unfit_returned, 3);
    // It stays where it is: the joiner's pickups follow its (full) window
    // for the hold, so the item is not picked up, refused and respawned.
    for _ in 0..40 {
        rig.tick();
    }
    assert_eq!(rig.c.grants, 1, "no second grant");
    assert_eq!(rig.ground_items().len(), 1, "still there for anyone to take");
    rig.assert_lockstep("the full joiner");
}

/// D-M2 — a `GrantUnfit` claiming more than its grant gave is clamped to the
/// grant; one naming no grant gives nothing.
#[test]
fn a_grant_unfit_claiming_more_than_its_grant_is_clamped() {
    let mut rig = Rig::dedicated("unfit-clamp");
    rig.drop_at_feet(ItemStack::new_material(MaterialId::Stick, 2));
    for _ in 0..3 {
        rig.tick();
    }
    assert_eq!(rig.c.grants, 1);
    assert_eq!(rig.c.unfit, 0, "it fit");
    rig.assert_lockstep("the grant");
    let event = rig.c.events;
    // A modified client claims 200 of the 2 didn't fit, then a grant it
    // never had.
    rig.send_action(protocol::ItemAction::GrantUnfit { event, count: 200 });
    rig.send_action(protocol::ItemAction::GrantUnfit { event: event + 50, count: 9 });
    rig.tick();
    let real: Vec<ItemStack> = rig.ground_items().into_iter().map(|(_, s)| s).collect();
    assert_eq!(real, vec![ItemStack::new_material(MaterialId::Stick, 2)], "never more than the grant");
    let t = rig.sp().window_events.tally;
    assert_eq!((t.unfit_returned, t.unfit_clamped), (2, 198 + 9));
}

/// B-L6 — a gap in the op sequence (an op that never arrived) is tallied as
/// lost; the digest now covers the locks and the station, so a lock the
/// server never heard of shows at the next op.
#[test]
fn a_lost_op_is_tallied_and_a_lock_divergence_shows_at_once() {
    let mut rig = Rig::dedicated("lost-op");
    rig.give(3, Item::Block(block::STONE), 1);
    rig.open_player();
    rig.flush();
    rig.tick();
    // The lock op is lost on the way.
    rig.click(WindowClick::ToggleLock { slot: 3 });
    let _ = rig.c.ui.take_ops(&rig.c.inv, &rig.c.armour);
    rig.c.seq += 1;
    rig.click(slot(9, false));
    rig.flush();
    rig.tick();
    let sp = &rig.hs.server.players[rig.c.slot];
    assert_eq!(sp.window_events.tally.ops_lost, 1);
    assert_eq!(sp.possession.window_mismatch, 1, "the lock it never heard of shows at the very next op");
}

/// Decision 2 — the first digest comparison after join is the baseline,
/// not a mismatch: the window the joiner arrived with (a fresh world's kit)
/// is on no wire yet.
#[test]
fn the_first_comparison_after_join_is_the_baseline_not_a_mismatch() {
    let mut rig = Rig::dedicated("baseline");
    // The client arrived with something the server never had; its session
    // starts by sending its auto-refill setting, digested over that window.
    rig.c.inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 4)));
    assert_eq!(rig.flush(), 1, "the auto-refill setting");
    rig.tick();
    let sp = &rig.hs.server.players[rig.c.slot];
    assert_eq!(sp.window_events.baseline, Some(false), "recorded as the baseline");
    assert_eq!(sp.possession.window_mismatch, 0, "not tallied");
    // From matched windows on, the tally counts.
    rig.sp().inventory.set_slot(0, Some(ItemStack::new_block(block::STONE, 4)));
    rig.open_player();
    rig.flush();
    rig.tick();
    rig.assert_lockstep("after the baseline");
}

/// Craft — the client never sends `ItemAction::Craft` (the craft is the
/// window's result click); a v75 server that receives one ignores it and
/// tallies it (`joiner_craft_drop::a_v75_server_ignores_a_craft_and_tallies_it`).
#[test]
fn no_client_site_sends_an_item_action_craft() {
    for file in ["game_loop.rs", "craft_ui.rs", "remote_client.rs", "joiner_actions.rs", "lib.rs"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("craft-send lint: cannot read {} ({e}); update this lint's list, don't delete it", path.display())
        });
        assert!(!raw.contains("ItemAction::Craft"), "{file} names ItemAction::Craft: the craft is a window op since v75");
    }
}

// ─── C3b-fix-b ───────────────────────────────────────────────────────────────

/// The cell over the floor in front of the rig's joiner (air, in reach).
const PLACE_AT: (i32, i32, i32) = (41, 80, 40);

/// A joiner whose hotbar slot 0 holds one dirt, with 64 more in the bag and
/// auto-refill on, places it and Q-drops: on the client the placement empties
/// the slot, auto-refill moves the bag's 64 in, and the drop takes one of
/// them. `placement_first` is the order the server reads them in. Returns
/// the client's 36 slots and the server's.
fn place_then_q(tag: &str, placement_first: bool) -> (Vec<Option<ItemStack>>, Vec<Option<ItemStack>>) {
    let mut rig = Rig::dedicated(tag);
    let dirt = Item::Block(block::DIRT);
    rig.give(0, dirt.clone(), 1);
    rig.give(20, dirt.clone(), 64);
    rig.c.inv.auto_refill = true;
    rig.sp().inventory.auto_refill = true;
    // The client's own effects, in the order it made them.
    assert!(rig.c.inv.take_placeable_from_hotbar(0).is_some(), "the placement");
    assert!(rig.c.inv.take_one_from_hotbar(0).is_some(), "the Q-drop, from the refilled slot");
    assert_eq!(rig.c.inv.slot(0).map(|s| s.count), Some(63));
    if placement_first {
        rig.send_placement(PLACE_AT, block::DIRT, &dirt);
        rig.send_drop(0, &dirt);
    } else {
        rig.send_drop(0, &dirt);
        rig.send_placement(PLACE_AT, block::DIRT, &dirt);
    }
    for _ in 0..3 {
        rig.tick();
    }
    assert_eq!(rig.world().get_block(PLACE_AT.0, PLACE_AT.1, PLACE_AT.2), block::DIRT, "placed");
    let slots = |inv: &Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
    (slots(&rig.c.inv), slots(&rig.sp().inventory))
}

/// B-M1 — a placement, then Q, in one window: the server reads the placement
/// first (the send path queues the request behind the unsent edit and sends
/// it right after the input carrying it), so its auto-refill and its drop
/// leave the layout the client's did. Read the other way round (the old
/// order: the request overtook the edit) the layouts split and stay split.
#[test]
fn a_q_drop_after_a_placement_leaves_the_servers_layout_the_clients() {
    let (client, server) = place_then_q("place-then-q", true);
    assert_eq!(server, client, "placement, then drop: the same 36 slots");
    let (client, server) = place_then_q("q-overtakes", false);
    assert_ne!(server, client, "the old order (drop, then placement) split them: nothing was refilled");
}

/// B-M2 — the reviewer's modified-client scenario: a grant landed and was
/// acked, its units then moved on in the server's copy (a real deposit), and
/// a `GrantUnfit` claims them all. No item is made from the report.
#[test]
fn a_grant_unfit_for_units_the_server_no_longer_holds_spawns_nothing() {
    let mut rig = Rig::dedicated("unfit-moved-on");
    rig.drop_at_feet(ItemStack::new_block(block::COBBLESTONE, 64));
    for _ in 0..4 {
        rig.tick();
    }
    assert_eq!(rig.c.grants, 1);
    assert!(rig.ground_items().is_empty(), "the pickup took the item out of the world");
    let event = rig.c.events;
    for i in 0..36 {
        rig.sp().inventory.set_slot(i, None);
    }
    rig.send_action(protocol::ItemAction::GrantUnfit { event, count: 64 });
    rig.tick();
    rig.tick();
    assert!(rig.ground_items().is_empty(), "64 cobblestone were not made from a report");
    let t = rig.sp().window_events.tally;
    assert_eq!((t.unfit_returned, t.unfit_unbacked), (0, 64));
}

/// B-M2 — the honest ping-pong: the server's copy has room the client's
/// lacks. The stack goes back down, thrown by the joiner, which then waits
/// out the Q-drop delay instead of taking it straight back (it used to be
/// picked up again after ten ticks, granted again, refused again). The loop
/// is slowed to the delay's period, not closed (see the residual below).
#[test]
fn a_full_client_against_a_roomier_server_copy_does_not_pick_its_refusal_straight_back_up() {
    let mut rig = Rig::dedicated("unfit-pingpong");
    for i in 0..36 {
        rig.c.inv.set_slot(i, Some(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
    }
    rig.drop_at_feet(ItemStack::new_material(MaterialId::Stick, 3));
    for _ in 0..4 {
        rig.tick();
    }
    assert_eq!((rig.c.grants, rig.c.unfit), (1, 3));
    let thrown = rig.ground_items();
    assert_eq!(thrown.len(), 1, "back on the ground");
    assert_eq!(thrown[0].1, ItemStack::new_material(MaterialId::Stick, 3));
    assert!(rig.sp().inventory.slot(0).is_none(), "and out of the server's copy, which had room");
    // Not granted again while the dropper delay lasts (it used to be ten
    // ticks: the all-player pickup delay of a natural drop).
    for tick in 1..=25 {
        rig.tick();
        assert_eq!(rig.c.grants, 1, "not granted again {tick} ticks later");
    }
    // Residual (open until C3d): under this drift the server's copy has room
    // for the whole hold, so once the delay (30 ticks) ends the joiner takes
    // it again and the cycle repeats at that period, not at ten. Only the
    // server owning the window ends it.
    assert_eq!(rig.ground_items().len(), 1, "still there for anyone");
}

/// A-L1 — the table grace is for a table that was there. A modified client
/// opens a "table" at an air cell in reach and clicks a result: the server
/// has no table to be kind about, and refuses.
#[test]
fn an_open_at_a_cell_that_is_not_a_table_earns_no_grace() {
    let mut rig = Rig::dedicated("grace-never-there");
    rig.give(0, Item::Material(MaterialId::IronIngot), 3);
    rig.give(1, Item::Material(MaterialId::Stick), 2);
    let cell = [rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32 - 2];
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::AIR);
    rig.open_table(cell);
    rig.flush();
    rig.tick();
    let gone = rig.sp().table_gone_ticks;
    assert!(gone.is_some_and(|n| n > window::SERVER_TABLE_GRACE_TICKS), "past the grace from the start: {gone:?}");
    // The client believes in its table; the server refuses what it crafts.
    let eye = rig.sp().player.eye_pos();
    let fill = WindowClick::Autofill { example: example("Iron Pickaxe") };
    let crafted = {
        let c = &mut rig.c;
        c.ui.apply_click(&mut c.inv, &mut c.armour, &fill, false, eye, |_| block::CRAFTING_TABLE);
        c.ui.apply_click(&mut c.inv, &mut c.armour, &WindowClick::Result, false, eye, |_| block::CRAFTING_TABLE)
    };
    assert!(matches!(crafted, ClickResult::Crafted(_)), "the client crafts at its imagined table");
    rig.flush();
    rig.tick();
    assert!(rig.sp().cursor.is_none(), "the server crafted nothing");
    assert!(rig.tally().window_refused >= 1, "and counts the refusal");
    // A real table gets it, as ever.
    let table = rig.place_table(-3);
    rig.open_table(table);
    rig.flush();
    rig.tick();
    assert_eq!(rig.sp().table_gone_ticks, None);
}
