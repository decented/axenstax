//! C3d runs 7 and 9 — the client half of typed edits, on a REAL
//! `RemoteClient` joined over the in-process transport to a real Strict
//! `HostedServer`.
//!
//! The client's window is its own: built by the test from the same item
//! list the server's copy is given once both have applied the join sync,
//! predicted only through `strict_edits::place_joined` / `break_joined`, and
//! changed otherwise only by what the server sends, through the client's
//! own appliers (`WindowInbox` in event order, `inventory_sync::apply`,
//! `apply_inventory_grant`, receipts through the world intake). It is never
//! copied from the server's copy. Routing is the game loop's own
//! (`strict_edits::route_place` / `route_break`: `judge` on the client's
//! own window, then `sends_typed`); a `Legacy` route goes on the movement
//! packet exactly as the game loop's old path does.
use super::*;
use crate::action_identity::InventoryMode;
use crate::block::{AIR, COBBLESTONE, STONE};
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{Item, ItemStack};
use crate::remote_client::{ConnectionState, RemoteClient, RequestOutcome};
use crate::strict_edits::{route_break, route_place, EditRecipe, Routed};
use crate::window_events::InboxItem;

/// Cells beside the joiner standing at (40.5, 80, 40.5), on its stone floor.
const CELLS: [[i32; 3]; 3] = [[41, 80, 40], [41, 80, 41], [39, 80, 40]];

struct Joined {
    _g: crate::save::WorldsRootGuard,
    hs: HostedServer,
    rc: RemoteClient,
    i: usize,
    inv: crate::inventory::Inventory,
    armour: [Option<crate::armour::ArmourItem>; 4],
    ui: crate::craft_ui::CraftingUi,
    inbox: crate::window_events::WindowInbox,
    /// The client's world: its own predictions, then the server's changes.
    world: crate::world::World,
    intake: crate::chunk_intake::ChunkIntake,
    reg: crate::block::BlockRegistry,
}

/// A real `RemoteClient` joined (as a guest) to a Strict dedicated server
/// whose joiner stands on a stone floor; both windows then hold `items`
/// (slot, stack) and the client's auto-refill is `refill`.
fn joined(tag: &str, items: &[(usize, ItemStack)], refill: bool) -> Joined {
    let g = crate::save::WorldsRootGuard::new(&format!("c3d-client-edit-{tag}"));
    let mut hs = super::tests::start_room_test_server(&format!("c3d-client-edit-{tag}"), 0);
    hs.select_inventory_mode(InventoryMode::Strict).expect("a dedicated server selects Strict");
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
    hs.server.loaded_columns.insert((40i32.div_euclid(cs), 40i32.div_euclid(cs)));
    let rc = RemoteClient::from_transport(
        Box::new(hs.attach_test_remote()),
        crate::remote_client::build_join_request_guest("Builder", 0),
        None,
    );
    let mut inv = crate::inventory::Inventory::new();
    inv.auto_refill = refill;
    let mut j = Joined {
        _g: g, hs, rc, i: 0, inv, armour: [None; 4], ui: crate::craft_ui::CraftingUi::new(),
        inbox: Default::default(), world: crate::world::World::new(), intake: Default::default(),
        reg: crate::block::BlockRegistry::new(),
    };
    for _ in 0..20 {
        j.pump();
        if j.rc.player_index().is_some() { break; }
    }
    j.pump();
    j.pump();
    j.i = j.rc.player_index().expect("the guest joined") as usize;
    assert_eq!(j.rc.inventory_mode, InventoryMode::Strict, "the session is Strict");
    // The joiner holds its column, as a real push would leave it: it hears
    // the server's changes there (B2a), the send-back of a refusal included.
    j.hs.hold_column_for_test(j.i, (40i32.div_euclid(cs), 40i32.div_euclid(cs)));
    // Both copies past what the client applied (a fresh guest has no join
    // sync), then the same holdings on each.
    let applied = j.rc.applied_window_boundary();
    let sp = &mut j.hs.server.players[j.i];
    crate::window_events::apply_through(sp, applied, 0);
    for (slot, stack) in items {
        sp.inventory.set_slot(*slot, Some(stack.clone()));
        j.inv.set_slot(*slot, Some(stack.clone()));
    }
    let p = &mut sp.player;
    p.pos = glam::Vec3::new(40.5, 80.0, 40.5);
    p.velocity = glam::Vec3::ZERO;
    p.on_ground = true;
    j
}

impl Joined {
    /// One server tick, then the client: its packets read, its window events
    /// applied in event order, its world intake (block changes, receipts).
    fn pump(&mut self) {
        self.hs.tick();
        self.rc.poll();
        let outcomes = std::mem::take(&mut self.rc.pending_outcomes);
        let grants = std::mem::take(&mut self.rc.pending_grants);
        self.inbox.add(outcomes, grants, Vec::new());
        let (items, _) = self.inbox.take_ordered();
        for item in items {
            let event = item.event();
            match item {
                InboxItem::Outcome(RequestOutcome::Sync(pkt)) => {
                    let mut w = crate::window::WindowMut { inv: &mut self.inv, armour: &mut self.armour, cursor: &mut self.ui.cursor_item, grid: &mut self.ui.grid, container: None };
                    let station = crate::inventory_sync::apply(&mut w, &pkt, &self.reg, &mut crate::use_edits::SentUses::default(), &mut self.inbox.debt, self.rc.inventory_mode).expect("a readable sync");
                    if let Some(station) = station { self.ui.adopt_station(station); }
                }
                InboxItem::Grant(grant) => {
                    crate::remote_entities::apply_inventory_grant(&mut self.inv, &grant, &self.reg);
                }
                _ => {}
            }
            self.rc.note_window_event(event);
        }
        let changes = std::mem::take(&mut self.rc.pending_block_changes);
        let chunks = std::mem::take(&mut self.rc.chunk_queue);
        for step in self.intake.plan_deltas(chunks, &changes, 64) {
            match step {
                crate::chunk_intake::Delta::Change(change) => { self.world.apply_remote_block_change(&change); }
                crate::chunk_intake::Delta::ActionReceipt(receipt) => self.rc.apply_action_receipt(receipt),
                _ => {}
            }
        }
    }

    /// A plain placement from hotbar `slot` at `cell`, routed as the game
    /// loop routes it (`route_place`): typed, or on the movement packet.
    fn place(&mut self, cell: [i32; 3], slot: usize) {
        let held = self.inv.slot(slot).map(|s| s.item.clone());
        let new = held.as_ref().and_then(crate::joiner_inventory::placeable_block).expect("a placeable block");
        let change = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: new, meta: self.world.meta_at(cell[0], cell[1], cell[2]) };
        let old = self.world.get_block(cell[0], cell[1], cell[2]);
        match route_place(Some(&mut self.rc), &mut self.ui, &mut self.inv, &mut self.armour, slot, change.clone(), old, &self.reg).expect("reserved and sent") {
            Routed::Typed(placed) => assert_eq!(placed, Some(new), "the window gave the block"),
            Routed::Legacy => {
                self.inv.take_placeable_from_hotbar(slot);
                let (kind, id) = crate::inventory::item_to_ref(held.as_ref().unwrap()).to_wire();
                self.rc.send_input(&protocol::InputPacket { block_changes: vec![change], edit_hands: vec![(slot as u8, kind, id)], ..Default::default() });
            }
        }
        self.world.place_player_block(cell[0], cell[1], cell[2], new);
    }

    /// A mined break at `cell` with hotbar `slot`, routed as the game loop
    /// routes it (`route_break`).
    fn mine(&mut self, cell: [i32; 3], slot: usize) {
        let held = self.inv.slot(slot).map(|s| s.item.clone());
        let tool = match &held { Some(Item::Tool(t)) => Some(*t), _ => None };
        let change = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: AIR, meta: 0 };
        let old = self.world.get_block(cell[0], cell[1], cell[2]);
        match route_break(Some(&mut self.rc), &mut self.ui, &mut self.inv, &mut self.armour, slot, change.clone(), old, &self.reg).expect("reserved and sent") {
            Routed::Typed(_) => {}
            Routed::Legacy => {
                self.inv.use_tool_at(slot);
                let mined = protocol::MinedBlock { x: cell[0], y: cell[1], z: cell[2], tool: tool.map_or(protocol::WireItem::None, |t| crate::inventory::item_to_wire_full(&Item::Tool(t))) };
                self.rc.send_input(&protocol::InputPacket { block_changes: vec![change], mined: vec![mined], ..Default::default() });
            }
        }
        self.world.set_block(cell[0], cell[1], cell[2], AIR);
    }

    /// Run 11 (run-9b review LOW-3) — a shaped or directional placement
    /// from hotbar `slot` at `cell`, clicked on `face` looking along `fwd`,
    /// as `GameState::take_for_placement` and the place arm make it: the
    /// meta hoisted (`placement_meta`, else the cell's), carried by the
    /// typed `Edit`, and written into the client's world. Returns the meta
    /// the client predicted.
    fn place_facing(&mut self, cell: [i32; 3], slot: usize, face: [i32; 3], fwd: glam::Vec3) -> u8 {
        let new = self.inv.slot(slot).and_then(|s| crate::joiner_inventory::placeable_block(&s.item)).expect("a placeable block");
        let stamped = crate::placement_meta::placement_meta(new, face, fwd);
        let meta = stamped.unwrap_or_else(|| self.world.meta_at(cell[0], cell[1], cell[2]));
        let change = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: new, meta };
        let old = self.world.get_block(cell[0], cell[1], cell[2]);
        let routed = route_place(Some(&mut self.rc), &mut self.ui, &mut self.inv, &mut self.armour, slot, change, old, &self.reg).expect("reserved and sent");
        assert!(matches!(routed, Routed::Typed(Some(b)) if b == new), "a shaped placement goes typed");
        self.world.place_player_block(cell[0], cell[1], cell[2], new);
        if let Some(m) = stamped { self.world.set_meta((cell[0], cell[1], cell[2]), m); }
        self.world.meta_at(cell[0], cell[1], cell[2])
    }

    /// Run 11 (run-9b review LOW-3) — a typed mined break whose cell keeps a
    /// block (a harvested crop's tilled soil, a papyrus top's stage 0), as
    /// `GameState::break_route` makes it: the replacement from the client's
    /// own `break_yield`, the cell's meta carried, the replacement predicted
    /// in the client's world. Returns the change sent.
    fn harvest(&mut self, cell: [i32; 3], slot: usize) -> protocol::BlockChange {
        let old = self.world.get_block(cell[0], cell[1], cell[2]);
        let tool = match self.inv.slot(slot).map(|s| &s.item) { Some(Item::Tool(t)) => Some(*t), _ => None };
        let keys = crate::break_drops::PopKeys { secret: &[0; 32], world_seed: 0, epoch: 0 };
        let replacement = crate::break_drops::break_yield(&self.world, &self.reg, old, (cell[0], cell[1], cell[2]), tool.as_ref(), 0, &keys).replacement;
        let meta = if replacement == AIR { 0 } else { self.world.meta_at(cell[0], cell[1], cell[2]) };
        let change = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: replacement, meta };
        let routed = route_break(Some(&mut self.rc), &mut self.ui, &mut self.inv, &mut self.armour, slot, change.clone(), old, &self.reg).expect("reserved and sent");
        assert!(matches!(routed, Routed::Typed(_)), "a harvest goes typed");
        self.world.set_block(cell[0], cell[1], cell[2], replacement);
        self.world.set_meta((cell[0], cell[1], cell[2]), meta);
        change
    }

    /// The header of the newest reserved action.
    fn newest_header(&self) -> crate::action_identity::ActionHeader {
        self.rc.action_journal.as_ref().expect("a joined journal").actions().last().expect("a reserved action").header
    }

    /// The digest the newest reserved edit predicts (its journal recipe).
    fn predicted_digest(&self) -> u32 {
        let journal = self.rc.action_journal.as_ref().expect("a joined journal");
        let last = journal.actions().last().expect("a reserved edit");
        bincode::deserialize::<EditRecipe>(&last.recipe).expect("an edit recipe").digest
    }

    /// The server's raw copy right after it served the edit: its window
    /// half staged and committed, the yield still waiting as numbered events.
    fn served_digest(&mut self) -> u32 {
        self.hs.tick();
        let sp = self.sp();
        crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station)
    }

    fn sp(&self) -> &crate::server::ServerPlayer { &self.hs.server.players[self.i] }

    fn client_digest(&self) -> u32 {
        crate::window::digest_parts(&self.inv, &self.armour, &self.ui.cursor_item, &self.ui.grid, self.ui.station())
    }

    /// The server's copy at the client's applied boundary (its waiting
    /// events applied through what the client applied).
    fn server_digest(&mut self) -> u32 {
        let applied = self.rc.applied_window_boundary();
        let sp = &mut self.hs.server.players[self.i];
        crate::window_events::apply_through(sp, applied, 0);
        crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station)
    }

    /// Never torn down; every reservation retired by its receipt; the two
    /// windows agree.
    fn assert_honest(&mut self, what: &str) {
        for _ in 0..3 { self.pump(); }
        assert!(matches!(self.rc.state, ConnectionState::Connected { .. }), "{what}: never torn down: {}", torn(&self.rc.state));
        assert!(self.sp().is_present_and_alive(), "{what}: the joiner is still in the world");
        assert_eq!(self.rc.action_journal.as_ref().map(|j| j.len()), Some(0), "{what}: every receipt retired its reservation");
        let (client, server) = (self.client_digest(), self.server_digest());
        assert_eq!(client, server, "{what}: the digests agree");
        assert_eq!(super::c3d_strict_tests::places(&self.inv, &self.armour, &self.ui.cursor_item, &self.ui.grid), super::c3d_strict_tests::server_places(self.sp()), "{what}: place for place");
    }

    /// Units of `item` in the client's window.
    fn held(&self, item: &Item) -> u32 {
        self.inv.slots_iter().flatten().filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum()
    }

    /// Units of `block` standing in the server's world at `cells`, and on
    /// its ground as items.
    fn world_and_ground(&self, block: crate::block::BlockId, cells: &[[i32; 3]]) -> (u32, u32) {
        let world = cells.iter().filter(|c| self.hs.server.world.get_block(c[0], c[1], c[2]) == block).count() as u32;
        let ground = self.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter()
            .filter(|(_, e)| e.stack.item == Item::Block(block)).map(|(_, e)| u32::from(e.stack.count)).sum();
        (world, ground)
    }
}

/// Why a session ended ("connected" while it lives).
fn torn(state: &ConnectionState) -> String {
    match state {
        ConnectionState::Connected { .. } => "connected".into(),
        ConnectionState::Failed(reason) => reason.clone(),
        _ => "not connected".into(),
    }
}

fn pick(durability: Option<u16>) -> Tool {
    let mut t = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
    if let Some(d) = durability { t.durability = d; }
    t
}

/// Honest placements and breaks over several ticks: every edit is ONE
/// typed action (nothing on the movement packet), accepted, never torn
/// down; the reservations retire at their boundaries; the windows agree;
/// and cobblestone is conserved over window + world + ground.
#[test]
fn c3d_client_honest_places_and_breaks_go_typed_and_agree() {
    let cobble = Item::Block(COBBLESTONE);
    let mut j = joined("honest", &[(0, ItemStack::new_block(COBBLESTONE, 5)), (1, ItemStack { item: Item::Tool(pick(None)), count: 1 })], true);
    for cell in CELLS {
        j.place(cell, 0);
        j.pump();
    }
    j.assert_honest("after three placements");
    let (world, ground) = j.world_and_ground(COBBLESTONE, &CELLS);
    assert_eq!((j.held(&cobble), world, ground), (2, 3, 0), "three placed, two held");
    for cell in &CELLS[..2] {
        j.mine(*cell, 1);
        j.pump();
    }
    j.assert_honest("after two breaks");
    let (world, ground) = j.world_and_ground(COBBLESTONE, &CELLS);
    assert_eq!(j.held(&cobble) + world + ground, 5, "cobblestone is conserved: held {} world {world} ground {ground}", j.held(&cobble));
    assert_eq!((j.held(&cobble), world), (4, 1), "the two breaks yielded into the window");
    for cell in CELLS {
        assert_eq!(j.world.get_block(cell[0], cell[1], cell[2]), j.hs.server.world.get_block(cell[0], cell[1], cell[2]), "the client's world agrees at {cell:?}");
    }
    let Some(Item::Tool(worn)) = j.inv.slot(1).map(|s| s.item.clone()) else { panic!("the pickaxe survives") };
    assert_eq!(worn.durability, pick(None).durability - 2, "two breaks wore it twice");
    let through = j.sp().action_frontier.as_ref().unwrap().through();
    assert!(through >= 5, "five edits (and the setting) numbered: through {through}");
}

/// The last item of a hotbar stack placed with auto-refill on: the client
/// predicts the refill by the server's own rule, so the windows agree.
#[test]
fn c3d_client_last_of_a_stack_refills_on_both_copies() {
    let mut j = joined("refill", &[(0, ItemStack::new_block(COBBLESTONE, 1)), (9, ItemStack::new_block(COBBLESTONE, 4))], true);
    j.place(CELLS[0], 0);
    j.pump();
    j.assert_honest("the last of the stack");
    assert_eq!(j.inv.slot(0).map(|s| (s.item.clone(), s.count)), Some((Item::Block(COBBLESTONE), 4)), "the hotbar slot refilled");
    assert!(j.inv.slot(9).is_none(), "from the bag");
    j.place(CELLS[1], 0);
    j.pump();
    j.assert_honest("the next placement from the refilled slot");
}

/// A break that wears the tool to breaking: removed on both copies; the
/// yield still lands; nothing torn down.
#[test]
fn c3d_client_break_wears_the_tool_to_breaking() {
    let mut j = joined("wear", &[(0, ItemStack::new_block(COBBLESTONE, 1)), (1, ItemStack { item: Item::Tool(pick(Some(1))), count: 1 })], true);
    j.place(CELLS[0], 0);
    j.pump();
    j.mine(CELLS[0], 1);
    j.pump();
    j.assert_honest("the last durability point");
    assert!(j.inv.slot(1).is_none(), "the pickaxe broke on the client");
    assert!(j.sp().inventory.slot(1).is_none(), "and on the server's copy");
    assert_eq!(j.held(&Item::Block(COBBLESTONE)), 1, "the yield landed");
}

/// Run 9 (run-6 review LOW-3) — the client's edit prediction pinned against
/// the server's staging: the digest each reserved edit's recipe predicts is
/// exactly the server copy's raw window right after it served that edit
/// (its window half committed, the yield still waiting as numbered events),
/// for the last of a stack with auto-refill, a tool's wear, a tool worn to
/// breaking and a bare hand. Each served row carries the edit's cell.
#[test]
fn c3d_client_edit_prediction_is_the_server_staging() {
    let mut j = joined("pin", &[(0, ItemStack::new_block(COBBLESTONE, 1)), (9, ItemStack::new_block(COBBLESTONE, 3)), (1, ItemStack { item: Item::Tool(pick(Some(2))), count: 1 })], true);
    let steps: [(&str, [i32; 3], Option<usize>, Option<usize>); 5] = [
        ("the last of a stack, refilled", CELLS[0], Some(0), None),
        ("a placement from the refilled slot", CELLS[1], Some(0), None),
        ("a break that wears the tool", CELLS[0], None, Some(1)),
        ("a break that wears it to breaking", CELLS[1], None, Some(1)),
        ("a bare-hand break of a placed block", CELLS[1], Some(0), Some(1)),
    ];
    for (what, cell, place, mine) in steps {
        if let (Some(slot), Some(_)) = (place, mine) {
            // Place first (settled), then break it with the now-empty slot.
            j.place(cell, slot);
            j.pump();
            j.pump();
        } else if let Some(slot) = place {
            j.place(cell, slot);
            let (predicted, served) = (j.predicted_digest(), j.served_digest());
            assert_eq!(predicted, served, "{what}: the client's prediction is the server's staging");
            assert!(j.sp().action_results.as_ref().unwrap().retained_cells().any(|c| *c == cell), "{what}: the served row carries the edit's cell");
            for _ in 0..3 { j.pump(); }
            continue;
        }
        let slot = mine.unwrap();
        j.mine(cell, slot);
        let (predicted, served) = (j.predicted_digest(), j.served_digest());
        assert_eq!(predicted, served, "{what}: the client's prediction is the server's staging");
        assert!(j.sp().action_results.as_ref().unwrap().retained_cells().any(|c| *c == cell), "{what}: the served row carries the edit's cell");
        for _ in 0..3 { j.pump(); }
    }
    assert!(j.inv.slot(1).is_none(), "the pickaxe broke at its second break");
    j.assert_honest("after the pinned edits");
}

/// Run 9 (run-6 review LOW-3) — a `Superseded` edit is corrected in the
/// world too: a placement from a body that died before it arrived is
/// terminal and zero-effect, its predicted cell is sent back (the server's
/// AIR), its row carries the cell, and the session lives on.
#[test]
fn c3d_client_superseded_edit_cell_comes_back_corrected() {
    // Away from the body, so no grave can land in it.
    const CELL: [i32; 3] = [43, 80, 43];
    let mut j = joined("superseded", &[(0, ItemStack::new_block(COBBLESTONE, 5))], true);
    let sp = &mut j.hs.server.players[j.i];
    sp.combat.health = 0.0;
    sp.combat.dead = true;
    j.place(CELL, 0);
    assert_eq!(j.world.get_block(CELL[0], CELL[1], CELL[2]), COBBLESTONE, "the client predicted the block");
    j.hs.tick();
    assert_eq!(j.hs.server.world.get_block(CELL[0], CELL[1], CELL[2]), AIR, "zero effect on the server's world");
    assert!(j.sp().action_results.as_ref().unwrap().retained_cells().any(|c| *c == CELL), "the Superseded row carries the edit's cell");
    for _ in 0..6 { j.pump(); }
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }), "never torn down: {}", torn(&j.rc.state));
    assert_eq!(j.world.get_block(CELL[0], CELL[1], CELL[2]), AIR, "the superseded edit's cell came back corrected");
    assert_eq!(j.rc.action_journal.as_ref().map(|journal| journal.len()), Some(0), "its receipt retired the reservation");
    assert_eq!(j.client_digest(), j.server_digest(), "the window was corrected too");
}

/// Run 11 (run-9b review LOW-3) — a shaped and a directional placement (a
/// stair's facing, a piston's look facing, pitch included): the meta the
/// client predicted (hoisted by `placement_meta` before the window is
/// predicted) is the meta the server applied from the typed `Edit`, read
/// before any broadcast could overwrite the client's; nothing tears down.
#[test]
fn c3d_client_shaped_placement_meta_is_the_servers() {
    use crate::block::{PISTON, STONE_STAIRS};
    let mut j = joined("shaped", &[(0, ItemStack::new_block(STONE_STAIRS, 2)), (1, ItemStack::new_block(PISTON, 2))], true);
    let east = glam::Vec3::new(1.0, 0.0, 0.1);
    let west_and_up = glam::Vec3::new(-1.0, 0.3, 0.1);
    for (what, cell, slot, fwd, block) in [("a stair", CELLS[0], 0, east, STONE_STAIRS), ("a piston", CELLS[1], 1, west_and_up, PISTON)] {
        let stamped = crate::placement_meta::placement_meta(block, [0, 1, 0], fwd);
        assert!(stamped.is_some_and(|m| m != 0), "{what}: a non-default meta is stamped ({stamped:?}), so the comparison is not vacuous");
        let predicted = j.place_facing(cell, slot, [0, 1, 0], fwd);
        assert_eq!(Some(predicted), stamped, "{what}: the client predicted the hoisted meta");
        j.hs.tick();
        assert_eq!(j.hs.server.world.get_block(cell[0], cell[1], cell[2]), block, "{what}: the server placed it");
        assert_eq!(j.hs.server.world.meta_at(cell[0], cell[1], cell[2]), predicted, "{what}: the server applied the client's predicted meta");
        j.assert_honest(what);
        assert_eq!(j.world.meta_at(cell[0], cell[1], cell[2]), predicted, "{what}: the client's world still agrees");
    }
}

/// Run 11 (run-9b review LOW-3) — typed breaks whose cell keeps a block: a
/// mature wheat on tilled soil goes to TILLED_SOIL, a papyrus top to its
/// STAGE_0. The edit carries the replacement and the cell's meta; the
/// server's `joiner_break_yield` accepts that replacement, applies the
/// carried meta, leaves the cell natural, and the yield lands in both
/// windows; nothing tears down.
#[test]
fn c3d_client_typed_break_with_a_block_left_behind() {
    use crate::block::{PAPYRUS_STAGE_0, PAPYRUS_STAGE_3, TILLED_SOIL, WHEAT_STAGE_3};
    let mut j = joined("harvest", &[], true);
    let (wheat, papyrus) = (CELLS[0], CELLS[1]);
    for w in [&mut j.hs.server.world, &mut j.world] {
        w.set_block(wheat[0], wheat[1] - 1, wheat[2], TILLED_SOIL);
        w.set_block(wheat[0], wheat[1], wheat[2], WHEAT_STAGE_3);
        w.set_meta((wheat[0], wheat[1], wheat[2]), 5);
        w.set_block(papyrus[0], papyrus[1], papyrus[2], PAPYRUS_STAGE_3);
        w.set_meta((papyrus[0], papyrus[1], papyrus[2]), 6);
    }
    for (what, cell, left, meta) in [("a mature wheat", wheat, TILLED_SOIL, 5u8), ("a papyrus top", papyrus, PAPYRUS_STAGE_0, 6)] {
        let change = j.harvest(cell, 0);
        assert_eq!((change.new_block, change.meta), (left, meta), "{what}: the edit carries the replacement and the cell's meta");
        j.hs.tick();
        let w = &j.hs.server.world;
        assert_eq!(w.get_block(cell[0], cell[1], cell[2]), left, "{what}: the server's joiner_break_yield accepted the replacement");
        assert_eq!(w.meta_at(cell[0], cell[1], cell[2]), meta, "{what}: the server applied the carried meta");
        assert!(!w.is_placed(cell[0], cell[1], cell[2]), "{what}: a yielded break leaves the cell natural");
        j.assert_honest(what);
        assert_eq!((j.world.get_block(cell[0], cell[1], cell[2]), j.world.meta_at(cell[0], cell[1], cell[2])), (left, meta), "{what}: the client's world agrees");
    }
    let wheat_held = j.held(&Item::Material(crate::item::MaterialId::Wheat));
    let reeds = j.held(&Item::Material(crate::item::MaterialId::PapyrusReed));
    assert!(wheat_held == 1 && reeds >= 1, "the yields landed: wheat {wheat_held}, reeds {reeds}");
}

/// Run 11 (run-9b review LOW-3) — the stale-base `Superseded` branch, a
/// LIVE body: an edit made on a base before a whole-window `Replace` its
/// client hadn't applied is terminal and zero-effect, its cell is sent
/// back, its row carries the cell, and the `Replace` it predates is its
/// correction: no second one is queued. Then the client converges.
#[test]
fn c3d_client_stale_base_edit_is_superseded_and_corrected() {
    const CELL: [i32; 3] = [43, 80, 43];
    let mut j = joined("stale", &[(0, ItemStack::new_block(COBBLESTONE, 5))], true);
    j.hs.queue_correction_replace(j.i);
    let (numbered, replace) = (j.sp().window_events.last_number(), j.sp().window_events.death_replace);
    assert!(replace.is_some(), "a whole-window Replace waits for the client");
    j.place(CELL, 0);
    let action = j.newest_header();
    assert!(j.sp().window_events.predates_replace(action.events_applied), "the edit's base predates the Replace");
    assert!(j.sp().is_present_and_alive(), "a live body: the stale-base branch, not the dead-body one");
    j.hs.tick();
    let sp = j.sp();
    assert_eq!(sp.action_results.as_ref().unwrap().peek(action).map(|r| r.fate), Some(crate::action_identity::ActionFate::Superseded), "Superseded");
    assert!(sp.action_results.as_ref().unwrap().retained_cells().any(|c| *c == CELL), "the row carries the edit's cell");
    assert_eq!((sp.window_events.last_number(), sp.window_events.death_replace), (numbered, replace), "no second Replace: the one it predates is its correction");
    assert_eq!(j.hs.server.world.get_block(CELL[0], CELL[1], CELL[2]), AIR, "zero effect on the server's world");
    for _ in 0..6 { j.pump(); }
    assert!(matches!(j.rc.state, ConnectionState::Connected { .. }), "never torn down: {}", torn(&j.rc.state));
    assert_eq!(j.world.get_block(CELL[0], CELL[1], CELL[2]), AIR, "the superseded edit's cell came back corrected");
    assert_eq!(j.rc.action_journal.as_ref().map(|journal| journal.len()), Some(0), "its receipt retired the reservation");
    assert_eq!(j.client_digest(), j.server_digest(), "the Replace corrected the window");
    assert_eq!(j.held(&Item::Block(COBBLESTONE)), 5, "the cobblestone came back");
}
