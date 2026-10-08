//! C3b-1 (2026-10-08, protocol v77) — shared chests, dispensers and furnaces
//! for joiners.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport (a
//! dedicated server, or a lending host) and joiners whose client half is the
//! real one: a `CraftingUi` with the player's `Inventory` and armour, and a
//! `container_window::SharedContainer` mirror built from the server's
//! `ContainerOpened` and kept by its `WindowSlotSet`s
//! (`container_window::apply_slot_set`), exactly as `game_loop` keeps it.
//! Clicks go through `CraftingUi::apply_container_click` (the prediction,
//! logged as a window op with its digest and touched slots), and the log is
//! sent numbered, as `GameState::flush_window_ops` sends it.

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
use crate::protocol::{self, slot_set_reason, OpenRefusal, WindowOpPacket, WindowSlotSetPacket, WireWindowSlot};
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
    /// The highest window event applied (C3a-fix-1; here a correction of
    /// player slots), reported with every op.
    events: u32,
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
        }
    }

    /// Read everything the server sent: an opened container becomes the
    /// mirror, a slot set is applied to the window and the mirror.
    fn receive(&mut self, registry: &block::BlockRegistry) {
        while let Some(pkt) = self.transport.try_recv_from_server() {
            match protocol::deserialize_header(&pkt) {
                Some((protocol::PacketType::ContainerOpened, payload)) => {
                    let opened: protocol::ContainerOpenedPacket = protocol::safe_deserialize(payload).unwrap();
                    match opened.refused {
                        Some(why) => self.refusals.push(why),
                        None => self.mirror = SharedContainer::from_opened(&opened, registry),
                    }
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
                    container_window::apply_slot_set(&mut view, &set, registry);
                    self.events = self.events.max(set.window_event);
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
            let pkt = WindowOpPacket {
                op_seq: self.seq,
                op: logged.op,
                digest: logged.digest,
                events_applied: self.events,
                touched: logged.touched,
                claims: logged.claims,
            };
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

    fn count(&self, item: &Item) -> u32 {
        let in_inv: u32 = self.inv.slots_iter().flatten().filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum();
        in_inv + self.ui.cursor_item.iter().filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum::<u32>()
    }
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
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        for c in &mut self.cs {
            c.receive(&self.registry);
        }
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
        let kind = sp.container_sent.as_ref()?.kind;
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
        assert_eq!(sp.possession.container_corrections, 0, "{what}: nothing corrected");
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
    assert_eq!((t.window_mismatch, t.container_corrections, t.container_refused), (0, 0, 0));
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
    let server_stone: u32 = (0..2)
        .map(|n| rig.hs.server.players[rig.cs[n].slot].inventory.slots_iter().flatten().filter(|s| s.item == stone_item).map(|s| u32::from(s.count)).sum::<u32>())
        .sum();
    assert_eq!(server_stone, 16, "the server holds exactly one stack between them");
    assert!(rig.server_container(winner).is_some_and(|c| c.get(0).is_none()));
    let corrections: Vec<_> = rig.cs[loser].slot_sets.iter().filter(|s| s.reason == slot_set_reason::CORRECTION).collect();
    assert_eq!(corrections.len(), 1, "one correction");
    assert!(corrections[0].sets.iter().any(|(at, v)| matches!(at, WireWindowSlot::Inv(_)) && v.is_none()), "it names the inventory slot only the client filled");
    assert_eq!(rig.tally(loser).container_corrections, 1);
    for n in [winner, loser] {
        let what = format!("joiner {n} after the race");
        let c = &rig.cs[n];
        assert_eq!(rig.server_digest(n), c.digest(), "{what}: digests");
    }
}

/// Race, staggered — the loser clicks before the push of the winner's take
/// reaches it, and its op lands a tick later. Its own touched slots still
/// name where its phantom stack landed, so the correction finds it.
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
    // and the slots the client touched are corrected back.
    rig.give(0, 5, Item::Block(block::DIRT), 3);
    let mut fake = ChestData::new();
    fake.slots[0] = Some(stone(4));
    rig.cs[0].mirror = Some(SharedContainer {
        cell: protected,
        kind: ContainerKind::Chest { tier: ChestTier::Wood },
        contents: container_window::ContainerData::Chest(fake),
    });
    let eye = rig.eye(0);
    rig.cs[0].click(ContainerClick::Withdraw { slot: 0, all: true }, eye);
    rig.cs[0].flush();
    rig.tick();
    let t = rig.tally(0);
    assert_eq!((t.container_refused, t.container_corrections), (1, 1));
    assert_eq!(rig.cs[0].count(&Item::Block(block::STONE)), 0, "the stone from a chest the server never opened is gone");
    assert_eq!(rig.cs[0].count(&Item::Block(block::DIRT)), 3, "an untouched slot stays");
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
/// copy applies when the loser's next op reports it.
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
    assert_eq!(player_sets(corrections[0]), vec![WireWindowSlot::Inv(0)], "exactly the slot it predicted into");
    let event = corrections[0].window_event;
    assert!(event > 0, "a correction of player slots is a numbered window event");
    assert!(rig.server_container(loser).is_some_and(|c| c.get(0).is_none()), "the stack is the winner's");
    // The loser's next op reports the event: the server's copy takes it.
    assert_eq!(rig.sp(loser).inventory.slot(0), None, "the server's copy waits for the client's word");
    rig.cs[loser].close();
    rig.cs[loser].flush();
    rig.tick();
    assert_eq!(rig.sp(loser).inventory.slot(0), Some(&stone(10)), "applied in the client's order");
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
    assert!(!sets.is_empty(), "the race was corrected");
    assert!(!sets.contains(&WireWindowSlot::Inv(20)) && !sets.contains(&WireWindowSlot::Inv(21)), "never a drifted slot: {sets:?}");
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
