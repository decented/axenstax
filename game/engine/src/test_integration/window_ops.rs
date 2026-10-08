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

use glam::Vec3;

use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
use crate::block;
use crate::craft_ui::CraftingUi;
use crate::crafting::{CraftSlot, Tool, ToolMaterial, ToolType};
use crate::hosted_server::{HostedServer, RemoteTransport, MAX_WINDOW_OPS_PER_TICK};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::protocol::{self, PlayerEventType, WindowOpPacket};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};
use crate::window::{self, ClickResult, Station, WindowClick, WindowSlot};
use crate::window_ops::OpKind;

use super::joiner_authority::{join_guest, send_edits};
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
        };
        let mut rig = Rig { hs, host, c, at };
        rig.tick();
        rig
    }

    fn tick(&mut self) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        while let Some(pkt) = self.c.transport.try_recv_from_server() {
            if let Some((protocol::PacketType::PlayerEvent, payload)) = protocol::deserialize_header(&pkt) {
                let ev: protocol::PlayerEventPacket = protocol::safe_deserialize(payload).unwrap();
                if let PlayerEventType::ArmourWorn { hits } = ev.event {
                    // The client wears its own armour by the shared rule, as
                    // `game_loop` does on `OwnLifeEvent::ArmourWorn`.
                    for _ in 0..hits {
                        window::wear_armour(&mut self.c.armour);
                    }
                    self.c.worn += u32::from(hits);
                }
            }
        }
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

    /// Send every logged op, numbered, as the game loop's flush does.
    fn flush(&mut self) -> usize {
        let c = &mut self.c;
        let ops = c.ui.take_ops(&c.inv, &c.armour);
        let n = ops.len();
        for (op, digest) in ops {
            c.seq += 1;
            let pkt = WindowOpPacket { op_seq: c.seq, op, digest };
            c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::WindowOp, &pkt));
        }
        n
    }

    fn client_digest(&self) -> u32 {
        window::digest_parts(&self.c.inv, &self.c.armour, &self.c.ui.cursor_item, &self.c.ui.grid)
    }

    fn server_digest(&self) -> u32 {
        let sp = &self.hs.server.players[self.c.slot];
        window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid)
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
    assert_eq!(t.window_refused, 0, "no click of this session is refused");
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
    assert_eq!(rig.tally().window_refused, 1, "the refusal is tallied");
    // Autofill there moves nothing either.
    let fill = WindowClick::Autofill { example: example("Iron Pickaxe") };
    assert_eq!(rig.step("autofill out of reach", fill), ClickResult::NeedsTable);
    assert_eq!(rig.server_digest(), laid);

    // Back in reach, but the table is broken.
    let at = rig.at;
    rig.sp().player.pos = at;
    rig.world().set_block(table[0], table[1], table[2], block::AIR);
    assert_eq!(rig.step("a result click at a broken table", WindowClick::Result), ClickResult::Refused);
    assert_eq!(rig.server_digest(), laid);
    // Rebuilt: the same click crafts on both sides.
    rig.world().set_block(table[0], table[1], table[2], block::CRAFTING_TABLE);
    assert!(matches!(rig.step("in reach again", WindowClick::Result), ClickResult::Crafted(_)));
    assert_eq!(rig.tally().window_refused, 3);
    assert_eq!(rig.tally().window_mismatch, 0);
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
    assert_eq!(rig.tally().window_refused, 1);
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
    send_edits(&rig.hs, &rig.c.transport, rig.c.slot, 1, &edits);
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
    assert_eq!(rig.sp().armour[0], None);
    assert_eq!(rig.c.armour[0], None);
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
    let pkt = protocol::ItemActionPacket {
        seq: 1,
        action: protocol::ItemAction::Eat {
            hotbar_slot: 0,
            held_kind: protocol::item_kind::MATERIAL,
            held_id: MaterialId::Bread as u16,
            held_full: protocol::WireItem::None,
        },
    };
    rig.c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    rig.tick();
    assert_eq!(rig.sp().craft_grid[1][1].as_ref().map(|s| s.count), Some(1), "paid from the server's grid");
    assert_eq!(rig.tally().mismatched, 0, "nothing short");
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
