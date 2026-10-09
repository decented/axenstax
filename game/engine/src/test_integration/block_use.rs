//! C3b-2 (2026-10-08, protocol v79) — composters, drying racks, campfires,
//! item frames and bee hives for joiners.
//!
//! Every test drives a REAL `HostedServer` over the in-process transport (a
//! dedicated server, or a lending host) and a joiner whose client half is the
//! real one: its window (`Inventory`, `CraftingUi`, armour), its requests in
//! flight (`JoinerActions`, claims included), and a `World` of its own that
//! takes in the server's block views (`block_views::apply_view`). A use is
//! sent as `GameState::send_block_use` sends it; the client applies the
//! outcome (`joiner_actions::apply_item_outcome` / `apply_use_wear`) and the
//! grants (`remote_entities::apply_inventory_grant`) in arrival order,
//! counting the window events, and reports the count on its next input, so
//! the server's copy of its window applies them at the same point.

use glam::Vec3;

use crate::armour::ArmourItem;
use crate::block;
use crate::block_use::UseKind;
use crate::craft_ui::CraftingUi;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::hosted_server::{HostedServer, RemoteTransport};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::item_actions::ItemNote;
use crate::joiner_actions::{Asked, JoinerActions, Pending};
use crate::protocol::{self, BlockEntityView, BlockView, ItemActionOutcomePacket};
use crate::sim_lend::OwnedSimParts;
use crate::transport::{ChannelClientTransport, ClientTransport};
use crate::window;

use super::joiner_authority::join_guest;
use super::joiners_act::floor_and_stand;
use super::lent_world::{join_guest_lent, start_lent};

/// One joiner's client half.
pub(super) struct Client {
    pub(super) transport: ChannelClientTransport,
    pub(super) slot: usize,
    pub(super) inv: Inventory,
    pub(super) ui: CraftingUi,
    pub(super) armour: [Option<ArmourItem>; 4],
    /// The highest window event applied.
    pub(super) events: u32,
    pub(super) actions: JoinerActions,
    pub(super) input_seq: u64,
    /// Its own world: only what block views put in it.
    pub(super) world: crate::world::World,
    /// Every view received, in order.
    pub(super) views: Vec<BlockEntityView>,
    /// Every outcome received, in order.
    pub(super) outcomes: Vec<ItemActionOutcomePacket>,
    /// Every block change received, in order.
    pub(super) changes: Vec<protocol::BlockChange>,
    /// Shears' wear applied (accepted uses that wore the tool).
    pub(super) wore: u32,
    /// `GrantUnfit` units reported.
    pub(super) unfit: u32,
    /// C3c-3b — every attachment change received, in order.
    pub(super) attachments: Vec<protocol::AttachmentChange>,
}

impl Client {
    pub(super) fn new((transport, slot): (ChannelClientTransport, usize)) -> Self {
        Client {
            transport,
            slot,
            inv: Inventory::new(),
            ui: CraftingUi::new(),
            armour: [None; 4],
            events: 0,
            actions: JoinerActions::default(),
            input_seq: 0,
            world: crate::world::World::new(),
            views: Vec::new(),
            outcomes: Vec::new(),
            changes: Vec::new(),
            wore: 0,
            unfit: 0,
            attachments: Vec::new(),
        }
    }

    /// Apply everything the server sent, in arrival order, as the game loop
    /// does: outcomes and grants change the window (each a window event),
    /// block views change its world.
    pub(super) fn receive(&mut self, registry: &block::BlockRegistry) {
        while let Some(pkt) = self.transport.try_recv_from_server() {
            let Some((ptype, payload)) = protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                protocol::PacketType::ItemActionOutcome => {
                    let out: ItemActionOutcomePacket = protocol::safe_deserialize(payload).unwrap();
                    if let Some(request) = self.actions.take(out.seq) {
                        crate::joiner_actions::apply_item_outcome(&mut self.inv, &mut self.ui, &request, &out);
                        if crate::joiner_actions::apply_use_wear(&mut self.inv, &request, &out).is_some() {
                            self.wore += 1;
                        }
                    }
                    self.events = self.events.max(out.window_event);
                    self.outcomes.push(out);
                }
                protocol::PacketType::InventoryGrant => {
                    let grant: protocol::InventoryGrantPacket = protocol::safe_deserialize(payload).unwrap();
                    let unfit = crate::remote_entities::apply_inventory_grant(&mut self.inv, &grant, registry).unwrap_or(0);
                    self.events = self.events.max(grant.window_event);
                    if unfit > 0 {
                        self.unfit += u32::from(unfit);
                        let seq = self.actions.unanswered();
                        let pkt = protocol::ItemActionPacket {
                            seq,
                            action: protocol::ItemAction::GrantUnfit { event: grant.window_event, count: unfit },
                            events_applied: self.events,
                        };
                        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
                    }
                }
                protocol::PacketType::StateUpdate => {
                    // As the client's world stream: a packet's block changes,
                    // then its views (`remote_client`, `chunk_stream`).
                    let state: protocol::StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
                    for bc in &state.block_changes {
                        self.world.apply_remote_block_change(bc);
                    }
                    self.changes.extend(state.block_changes);
                    for v in state.block_views {
                        crate::block_views::apply_view(&mut self.world, registry, &v);
                        self.views.push(v);
                    }
                    // C3c-3b — and its attachment changes, as render stubs.
                    for a in &state.attachment_changes {
                        crate::chunk_intake::apply_attachment_change(&mut self.world, a);
                    }
                    self.attachments.extend(state.attachment_changes);
                }
                _ => {}
            }
        }
    }

    /// Right-click the `kind` block at `cell` with what is in hotbar slot
    /// `hot`, as `GameState::send_block_use` does: claimed, recorded, sent.
    /// `false` when the claim stops it (nothing sent).
    pub(super) fn use_block(&mut self, cell: [i32; 3], kind: UseKind, hot: usize) -> bool {
        let held = self.inv.hotbar_slot(hot).map(|s| s.item.clone());
        let asked = Asked::UseBlock { cell, kind, claim: crate::block_use::claim(kind, held.as_ref()) };
        if !self.actions.can_afford(&self.inv, &self.ui, asked, held.as_ref()) {
            return false;
        }
        let (held_kind, held_id) = held.as_ref().map_or(protocol::ItemRef::Empty.to_wire(), |i| {
            crate::inventory::item_to_ref(i).to_wire()
        });
        let held_full = held.as_ref().map_or(protocol::WireItem::None, crate::inventory::item_to_wire_full);
        let seq = self.actions.record(Pending { kind: asked, mob: None, hotbar_slot: hot, held }, self.input_seq + 1);
        let pkt = protocol::ItemActionPacket {
            seq,
            action: protocol::ItemAction::UseBlock { cell, hotbar_slot: hot as u8, held_kind, held_id, held_full },
            events_applied: self.events,
        };
        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
        true
    }

    /// A modified client's use: it claims `held` from hotbar slot `hot`
    /// whatever its window holds, unrecorded (no claim of its own), with
    /// request number `seq`.
    pub(super) fn use_raw(&mut self, seq: u32, cell: [i32; 3], hot: usize, held: &Item) {
        let (held_kind, held_id) = crate::inventory::item_to_ref(held).to_wire();
        let held_full = crate::inventory::item_to_wire_full(held);
        let pkt = protocol::ItemActionPacket {
            seq,
            action: protocol::ItemAction::UseBlock { cell, hotbar_slot: hot as u8, held_kind, held_id, held_full },
            events_applied: self.events,
        };
        self.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ItemAction, &pkt));
    }

    pub(super) fn count(&self, item: &Item) -> u32 {
        self.inv.slots_iter().flatten().filter(|s| crate::joiner_actions::same_item(&s.item, item)).map(|s| u32::from(s.count)).sum()
    }

    pub(super) fn last(&self) -> &ItemActionOutcomePacket {
        self.outcomes.last().expect("an outcome")
    }

    /// The last view received for `cell`.
    pub(super) fn view_at(&self, cell: [i32; 3]) -> Option<&BlockView> {
        self.views.iter().rev().find(|v| v.cell == cell).map(|v| &v.view)
    }
}

/// Joiners standing on a stone floor round (40, 80, 40), each holding the
/// column there as pushed (`hold_column_for_test`), on a dedicated server or
/// a lending host (`host`: the host client's world and ECS).
pub(super) struct Rig {
    pub(super) hs: HostedServer,
    pub(super) host: Option<OwnedSimParts>,
    pub(super) cs: Vec<Client>,
    pub(super) at: Vec3,
    pub(super) registry: block::BlockRegistry,
}

impl Rig {
    pub(super) fn dedicated(tag: &str, joiners: usize) -> Self {
        let mut hs = HostedServer::start(
            0,
            format!("block-use-{tag}-{}", std::process::id()),
            42,
            0,
            RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let joined: Vec<_> = (0..joiners).map(|n| join_guest(&mut hs, &format!("Gardener{n}"))).collect();
        Self::stand(hs, None, joined)
    }

    pub(super) fn lent(tag: &str, joiners: usize) -> Self {
        let (mut hs, mut host) = start_lent(&format!("block-use-{tag}"));
        let joined: Vec<_> = (0..joiners).map(|n| join_guest_lent(&mut hs, &mut host, &format!("Gardener{n}"))).collect();
        Self::stand(hs, Some(host), joined)
    }

    pub(super) fn stand(mut hs: HostedServer, mut host: Option<OwnedSimParts>, joined: Vec<(ChannelClientTransport, usize)>) -> Self {
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
            hs.hold_column_for_test(*slot, crate::chunk_stream::column_of(at));
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

    /// One server tick; then every client reads what it was sent and sends
    /// its input, reporting the window events it applied.
    pub(super) fn tick(&mut self) {
        match self.host.as_mut() {
            Some(h) => h.lend_tick(&mut self.hs),
            None => self.hs.tick(),
        }
        for n in 0..self.cs.len() {
            self.cs[n].receive(&self.registry);
            self.input(n);
        }
    }

    pub(super) fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            self.tick();
        }
    }

    /// Joiner `n`'s input: standing still, reporting its window events.
    pub(super) fn input(&mut self, n: usize) {
        let c = &mut self.cs[n];
        c.input_seq += 1;
        let sp = &self.hs.server.players[c.slot];
        let input = protocol::InputPacket {
            tick: c.input_seq,
            x: sp.player.pos.x,
            y: sp.player.pos.y,
            z: sp.player.pos.z,
            yaw: sp.yaw,
            pitch: sp.pitch,
            health: 20.0,
            events_applied: c.events,
            ..Default::default()
        };
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// Joiner `n`'s input carrying the edit `b` at `pos` (a break with AIR).
    pub(super) fn edit(&mut self, n: usize, pos: (i32, i32, i32), b: block::BlockId) {
        let c = &mut self.cs[n];
        c.input_seq += 1;
        let sp = &self.hs.server.players[c.slot];
        let input = protocol::InputPacket {
            tick: c.input_seq,
            x: sp.player.pos.x,
            y: sp.player.pos.y,
            z: sp.player.pos.z,
            yaw: sp.yaw,
            pitch: sp.pitch,
            health: 20.0,
            block_changes: vec![protocol::BlockChange { x: pos.0, y: pos.1, z: pos.2, new_block: b, meta: 0 }],
            events_applied: c.events,
            ..Default::default()
        };
        c.transport.send_to_server(&protocol::serialize_packet(protocol::PacketType::ClientInput, &input));
    }

    /// Joiner `n` uses the `kind` block at `cell` from hotbar slot `hot`;
    /// the server answers and the window events land on both sides.
    pub(super) fn use_block(&mut self, n: usize, cell: [i32; 3], kind: UseKind, hot: usize) -> &ItemActionOutcomePacket {
        let before = self.cs[n].outcomes.len();
        assert!(self.cs[n].use_block(cell, kind, hot), "sent");
        self.ticks(3);
        assert_eq!(self.cs[n].outcomes.len(), before + 1, "answered once");
        self.cs[n].last()
    }

    pub(super) fn world(&mut self) -> &mut crate::world::World {
        match self.host.as_mut() {
            Some(h) => &mut h.world,
            None => &mut self.hs.server.world,
        }
    }

    pub(super) fn ecs(&self) -> &hecs::World {
        match self.host.as_ref() {
            Some(h) => &h.ecs,
            None => &self.hs.server.ecs,
        }
    }

    /// The cell `dz` blocks ahead of the joiners' feet, holding `b` (in the
    /// world the server simulates and, as its column's push would, in every
    /// joiner's world).
    pub(super) fn place(&mut self, dz: i32, b: block::BlockId) -> [i32; 3] {
        let cell = [self.at.x.floor() as i32, self.at.y as i32, self.at.z.floor() as i32 + dz];
        self.set(cell, b);
        cell
    }

    /// `cell` holds `b`, everywhere (as [`Self::place`]).
    pub(super) fn set(&mut self, cell: [i32; 3], b: block::BlockId) {
        self.world().set_block(cell[0], cell[1], cell[2], b);
        for c in &mut self.cs {
            c.world.set_block(cell[0], cell[1], cell[2], b);
        }
    }

    /// Put `count` of `item` in slot `slot` on both sides of joiner `n`'s window.
    pub(super) fn give(&mut self, n: usize, slot: usize, item: Item, count: u8) {
        let stack = ItemStack { item, count };
        self.cs[n].inv.set_slot(slot, Some(stack.clone()));
        let s = self.cs[n].slot;
        self.hs.server.players[s].inventory.set_slot(slot, Some(stack));
    }

    /// The server's copy of joiner `n`'s window is the client's, slot for
    /// slot, with equal digests, no event left waiting and no mismatch.
    pub(super) fn assert_lockstep(&self, n: usize, what: &str) {
        let c = &self.cs[n];
        let sp = &self.hs.server.players[c.slot];
        let slots = |inv: &Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
        assert_eq!(slots(&sp.inventory), slots(&c.inv), "{what}: the 36 slots");
        assert_eq!(
            window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station),
            window::digest_parts(&c.inv, &c.armour, &c.ui.cursor_item, &c.ui.grid, c.ui.station()),
            "{what}: the digests"
        );
        assert_eq!(sp.window_events.waiting(), 0, "{what}: every event applied on the client's word");
        assert_eq!(sp.window_events.tally.forced, 0, "{what}: none forced");
        assert_eq!(sp.possession.mismatched, 0, "{what}: the server's copy paid every take");
        assert_eq!(sp.possession.wear_mismatch, 0, "{what}: and wore every tool it wore");
    }

    /// Ground items in the world the server simulates.
    pub(super) fn ground(&self, item: &Item) -> u32 {
        self.ecs()
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter(|(_, it)| &it.stack.item == item)
            .map(|(_, it)| u32::from(it.stack.count))
            .sum()
    }
}

pub(super) fn mat(m: MaterialId) -> Item {
    Item::Material(m)
}

fn shears() -> Item {
    Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron))
}

pub(super) fn accepted(out: &ItemActionOutcomePacket, consume: u8) {
    assert!(out.accepted, "accepted: {out:?}");
    assert_eq!(out.consume_held, consume, "{out:?}");
    assert_eq!(out.note, 0);
}

pub(super) fn refused(out: &ItemActionOutcomePacket, note: ItemNote) {
    assert!(!out.accepted, "refused: {out:?}");
    assert_eq!((out.consume_held, out.window_event, out.wear_held), (0, 0, false), "a refusal changes nothing");
    assert_eq!(ItemNote::from_wire(out.note), note);
}

// ─── Each block, through a joiner ──────────────────────────────────────────

/// A composter: a compostable loads one into the server's real composter
/// (the joiner pays one, as a window event), the server ages it, and the
/// aged output is collected by an empty hand (a grant). Every step reaches
/// the joiner's world as the composter's view.
#[test]
fn a_joiner_loads_and_empties_the_servers_composter() {
    let mut rig = Rig::dedicated("composter", 1);
    let cell = rig.place(2, block::COMPOSTER);
    rig.give(0, 0, mat(MaterialId::WheatSeeds), 5);
    let out = rig.use_block(0, cell, UseKind::Composter, 0).clone();
    accepted(&out, 1);
    assert_ne!(out.window_event, 0, "the take is a window event");
    let pos = (cell[0], cell[1], cell[2]);
    assert_eq!(rig.world().composter_at(pos).and_then(|c| c.input.as_ref()).map(|s| s.count), Some(1), "on the REAL composter");
    assert_eq!(rig.cs[0].count(&mat(MaterialId::WheatSeeds)), 4);
    rig.assert_lockstep(0, "after loading");
    let Some(BlockView::Composter { input: Some(input), .. }) = rig.cs[0].view_at(cell) else {
        panic!("the composter's view: {:?}", rig.cs[0].views)
    };
    assert_eq!(input.count, 1);
    // The dedicated server ages it (its block machines, every 4th tick) —
    // the joiner sees the output arrive without doing anything.
    rig.world().composter_at_mut(pos).unwrap().progress_ticks = crate::composter::COMPOST_PERIOD_TICKS - 1;
    rig.ticks(12);
    assert!(matches!(rig.cs[0].view_at(cell), Some(BlockView::Composter { output: Some(_), .. })), "the server's own tick, seen");
    let out = rig.use_block(0, cell, UseKind::Composter, 1).clone();
    accepted(&out, 0);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Compost)), 1, "the output, granted");
    assert!(rig.world().composter_at(pos).unwrap().output.is_none());
    rig.assert_lockstep(0, "after collecting");
    assert!(matches!(rig.cs[0].view_at(cell), Some(BlockView::Composter { output: None, .. })));
}

/// A drying rack on a lending host: a green log hangs on the host's own rack
/// (the one world), a seasoned one comes down to an empty hand.
#[test]
fn a_joiner_hangs_and_takes_down_logs_on_a_lending_hosts_rack() {
    let mut rig = Rig::lent("rack", 1);
    let cell = rig.place(2, block::DRYING_RACK);
    let pos = (cell[0], cell[1], cell[2]);
    rig.give(0, 0, mat(MaterialId::GreenLog), 2);
    accepted(&rig.use_block(0, cell, UseKind::DryingRack, 0).clone(), 1);
    assert_eq!(rig.host.as_ref().unwrap().world.drying_racks[&pos].occupied_slots(), 1, "the host's own rack");
    assert_eq!(rig.cs[0].count(&mat(MaterialId::GreenLog)), 1);
    rig.assert_lockstep(0, "after hanging");
    assert_eq!(rig.cs[0].world.drying_racks[&pos].occupied_slots(), 1, "the joiner's world holds the server's rack");
    // Seasoned (the host client runs the rack sweep on its lent world).
    rig.world().drying_racks.get_mut(&pos).unwrap().slots[0].seasoning_ticks = crate::drying_rack::SEASON_TICKS;
    accepted(&rig.use_block(0, cell, UseKind::DryingRack, 1).clone(), 0);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::SeasonedLog)), 1, "granted");
    assert_eq!(rig.host.as_ref().unwrap().world.drying_racks[&pos].occupied_slots(), 0);
    rig.assert_lockstep(0, "after taking down");
}

/// A campfire: raw food goes on the server's fire, fuel burns (relighting a
/// smouldering fire, whose block flip and smoke everyone gets), and cooked
/// food comes off to an empty hand.
#[test]
fn a_joiner_cooks_on_the_servers_campfire_and_its_fuel_relights_a_smouldering_one() {
    let mut rig = Rig::dedicated("campfire", 1);
    let cell = rig.place(2, block::CAMPFIRE_UNLIT);
    let pos = (cell[0], cell[1], cell[2]);
    rig.world().insert_campfire(pos, crate::campfire::CampfireData { smoulder_ticks: 300, ..Default::default() });
    rig.give(0, 0, mat(MaterialId::RawBeef), 3);
    rig.give(0, 1, mat(MaterialId::Coal), 2);
    accepted(&rig.use_block(0, cell, UseKind::Campfire, 0).clone(), 1);
    assert_eq!(rig.world().campfire_at(pos).unwrap().slots[0].item, Some(MaterialId::RawBeef), "on the REAL fire");
    accepted(&rig.use_block(0, cell, UseKind::Campfire, 1).clone(), 1);
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::CAMPFIRE, "relit");
    assert!(
        rig.cs[0].changes.iter().any(|bc| [bc.x, bc.y, bc.z] == cell && bc.new_block == block::CAMPFIRE),
        "the relight reached the joiner as a block change"
    );
    let cf = rig.world().campfire_at(pos).unwrap().clone();
    assert_eq!((cf.fuel_ticks > 0, cf.smoulder_ticks), (true, 0));
    let Some(BlockView::Campfire { fuel_ticks, slots, .. }) = rig.cs[0].view_at(cell) else { panic!("the fire's view") };
    assert!(*fuel_ticks > 0);
    assert_eq!(slots[0].item, Some(MaterialId::RawBeef), "the joiner sees what is on the fire");
    rig.assert_lockstep(0, "after cooking and fuelling");
    // Cooked (the dedicated server runs no campfire sweep until D4).
    rig.world().campfire_at_mut(pos).unwrap().slots[0].progress_ticks = crate::campfire::COOK_TICKS_PER_ITEM;
    accepted(&rig.use_block(0, cell, UseKind::Campfire, 5).clone(), 0);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::CookedBeef)), 1);
    assert!(rig.world().campfire_at(pos).unwrap().slots[0].item.is_none());
    rig.assert_lockstep(0, "after the pickup");
}

/// An item frame: an item is mounted (one taken), a second click turns it;
/// breaking the frame drops the framed item from the server's frame (its
/// "take"), never from the joiner's view of it.
#[test]
fn a_joiner_frames_turns_and_breaks_out_an_item() {
    let mut rig = Rig::dedicated("frame", 1);
    let cell = rig.place(2, block::ITEM_FRAME);
    let pos = (cell[0], cell[1], cell[2]);
    rig.give(0, 0, Item::Block(block::DIAMOND_BLOCK), 3);
    accepted(&rig.use_block(0, cell, UseKind::ItemFrame, 0).clone(), 1);
    assert_eq!(rig.cs[0].count(&Item::Block(block::DIAMOND_BLOCK)), 2);
    accepted(&rig.use_block(0, cell, UseKind::ItemFrame, 0).clone(), 0);
    assert_eq!(rig.world().item_frame_at(pos).unwrap().rotation, 1, "turned, nothing taken");
    let Some(BlockView::ItemFrame { rotation, item_id, .. }) = rig.cs[0].view_at(cell) else { panic!("the frame's view") };
    assert_eq!((*rotation, *item_id), (1, block::DIAMOND_BLOCK));
    assert!(rig.cs[0].world.item_frame_at(pos).is_some_and(|f| !f.is_empty()), "the joiner's world draws it");
    rig.assert_lockstep(0, "after framing");
    // The joiner breaks the frame: the server spills the real diamond.
    rig.edit(0, pos, block::AIR);
    rig.ticks(2);
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::AIR);
    assert_eq!(rig.ground(&Item::Block(block::DIAMOND_BLOCK)), 1, "the frame's item, spilled by the server");
    assert!(rig.world().item_frame_at(pos).is_none(), "nothing left behind the broken frame");
}

/// A hive: a bucket scoops a jar (the bucket is taken), shears cut
/// honeycomb and WEAR on both sides at the same point.
#[test]
fn a_joiner_scoops_and_shears_the_servers_hive() {
    let mut rig = Rig::dedicated("hive", 1);
    let cell = rig.place(2, block::BEE_HIVE);
    let pos = (cell[0], cell[1], cell[2]);
    rig.world().insert_hive(pos, crate::bee_hive::HiveData { bees_inside: 0, honey_level: 3 });
    rig.give(0, 0, mat(MaterialId::Bucket), 1);
    rig.give(0, 1, shears(), 1);
    accepted(&rig.use_block(0, cell, UseKind::Hive, 0).clone(), 1);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Bucket)), 0, "the bucket is taken");
    assert_eq!(rig.cs[0].count(&mat(MaterialId::HoneyBottle)), 1);
    rig.assert_lockstep(0, "after scooping");
    let out = rig.use_block(0, cell, UseKind::Hive, 1).clone();
    accepted(&out, 0);
    assert!(out.wear_held && out.window_event != 0, "the wear is the outcome's window event");
    assert_eq!(rig.cs[0].wore, 1);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Honeycomb)), 3);
    assert_eq!(rig.world().hive_at(pos).unwrap().honey_level, 1);
    assert_eq!(rig.cs[0].view_at(cell), Some(&BlockView::Hive { honey_level: 1 }));
    let durability = |inv: &Inventory| match inv.slot(1).map(|s| &s.item) {
        Some(Item::Tool(t)) => t.durability,
        other => panic!("the shears: {other:?}"),
    };
    let fresh = match shears() {
        Item::Tool(t) => t.durability,
        _ => unreachable!(),
    };
    assert_eq!(durability(&rig.cs[0].inv), fresh - 1, "the client's shears wore");
    rig.assert_lockstep(0, "after shearing (the server's shears wore the same)");
}

/// C3b-fix-e (decision 6) — a joiner breaks a filled hive and places it
/// again: the server's re-placed hive starts with an empty state
/// (`World::set_block`), so a bee nearby fills it, and the joiner is shown
/// the honey. Before, a re-placed hive had no state (a refused click creates
/// none since C3b-2-fix L7) and never filled.
#[test]
fn a_hive_a_joiner_breaks_and_places_again_fills_on_the_server() {
    let mut rig = Rig::dedicated("hive-replaced", 1);
    let cell = rig.place(2, block::BEE_HIVE);
    let pos = (cell[0], cell[1], cell[2]);
    rig.world().insert_hive(pos, crate::bee_hive::HiveData { bees_inside: 0, honey_level: 5 });
    rig.edit(0, pos, block::AIR);
    rig.ticks(2);
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::AIR, "broken");
    assert!(rig.world().hive_at(pos).is_none(), "its state went with it");
    rig.edit(0, pos, block::BEE_HIVE);
    rig.ticks(2);
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::BEE_HIVE, "placed again");
    assert_eq!(rig.world().hive_at(pos), Some(&crate::bee_hive::HiveData::default()), "the server's hive starts empty");
    let mut bees = hecs::World::new();
    bees.spawn((
        crate::entity::Position(Vec3::new(cell[0] as f32 + 2.5, cell[1] as f32, cell[2] as f32 + 0.5)),
        crate::entity::MobKind(crate::mob::MobType::Bee),
    ));
    crate::bee_hive::accumulate_honey(rig.world(), &bees, crate::bee_hive::HONEY_ACCUM_INTERVAL_TICKS);
    assert_eq!(rig.world().hive_at(pos).map(|h| h.honey_level), Some(1), "a bee nearby fills it");
    rig.ticks(2);
    assert_eq!(rig.cs[0].view_at(cell), Some(&BlockView::Hive { honey_level: 1 }), "the joiner is shown the honey");
}

// ─── Refusals ──────────────────────────────────────────────────────────────

/// Out of reach, in a foreign plot, a cell holding none of these blocks,
/// nothing to take and a full block: each refused with its note, the
/// outcome carrying no window event, and nothing changed on either side.
#[test]
fn refused_uses_change_nothing_and_say_why() {
    let mut rig = Rig::dedicated("refusals", 1);
    rig.give(0, 0, mat(MaterialId::GreenLog), 4);
    let far = rig.place(9, block::DRYING_RACK);
    refused(&rig.use_block(0, far, UseKind::DryingRack, 0).clone(), ItemNote::OutOfReach);
    assert!(!rig.world().drying_racks.contains_key(&(far[0], far[1], far[2])), "no state created for a refusal");
    let fenced = rig.place(1, block::DRYING_RACK);
    rig.world().plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        fenced[0],
        fenced[1] - 5,
        fenced[2],
    ));
    refused(&rig.use_block(0, fenced, UseKind::DryingRack, 0).clone(), ItemNote::NotHere);
    rig.world().plots.clear();
    let stone = rig.place(-1, block::STONE);
    refused(&rig.use_block(0, stone, UseKind::DryingRack, 0).clone(), ItemNote::NotThatBlock);
    let bin = rig.place(-2, block::COMPOSTER);
    refused(&rig.use_block(0, bin, UseKind::Composter, 5).clone(), ItemNote::NothingToTake);
    // A full rack.
    let rack = rig.place(2, block::DRYING_RACK);
    let mut full = crate::drying_rack::DryingRackData::default();
    for s in &mut full.slots {
        *s = crate::drying_rack::RackSlot { species: Some(crate::drying_rack::LogSpecies::Oak), seasoning_ticks: 0 };
    }
    rig.world().drying_racks.insert((rack[0], rack[1], rack[2]), full);
    refused(&rig.use_block(0, rack, UseKind::DryingRack, 0).clone(), ItemNote::RackFull);
    // An empty hand on a rack with nothing seasoned.
    refused(&rig.use_block(0, rack, UseKind::DryingRack, 5).clone(), ItemNote::NotReady);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::GreenLog)), 4, "nothing was taken");
    rig.assert_lockstep(0, "after the refusals");
}

/// A claim gate: while a use claims the only bucket, a second use that
/// would take it isn't sent (one bucket can't scoop two hives on a slow
/// link); once the first is answered, the bucket is gone.
#[test]
fn a_pending_claim_on_the_only_bucket_stops_a_second_hive_use() {
    let mut rig = Rig::dedicated("claim", 1);
    let a = rig.place(2, block::BEE_HIVE);
    let b = rig.place(-2, block::BEE_HIVE);
    for cell in [a, b] {
        rig.world().insert_hive((cell[0], cell[1], cell[2]), crate::bee_hive::HiveData { bees_inside: 0, honey_level: 2 });
    }
    rig.give(0, 0, mat(MaterialId::Bucket), 1);
    assert!(rig.cs[0].use_block(a, UseKind::Hive, 0), "the first goes");
    assert!(!rig.cs[0].use_block(b, UseKind::Hive, 0), "the second would spend the claimed bucket: not sent");
    rig.ticks(3);
    assert_eq!(rig.cs[0].outcomes.len(), 1, "one answered use");
    assert_eq!(rig.world().hive_at((b[0], b[1], b[2])).unwrap().honey_level, 2, "the second hive untouched");
    assert_eq!(rig.cs[0].count(&mat(MaterialId::HoneyBottle)), 1);
    rig.assert_lockstep(0, "after the claim");
}

// ─── Everyone sees the change ──────────────────────────────────────────────

/// Another joiner sees a joiner's use, the host sees it in its own (lent)
/// world, and a host's own click on its lent world reaches both joiners:
/// every view goes to every joiner holding the chunk, whatever changed it.
#[test]
fn every_joiner_and_the_host_see_a_block_change_whoever_made_it() {
    let mut rig = Rig::lent("everyone", 2);
    let cell = rig.place(2, block::ITEM_FRAME);
    let pos = (cell[0], cell[1], cell[2]);
    rig.give(0, 0, Item::Block(block::IRON_BLOCK), 1);
    accepted(&rig.use_block(0, cell, UseKind::ItemFrame, 0).clone(), 1);
    let frame_item = |w: &crate::world::World| w.item_frame_at(pos).and_then(|f| f.item.as_ref().map(|s| s.item.clone()));
    assert_eq!(frame_item(&rig.host.as_ref().unwrap().world), Some(Item::Block(block::IRON_BLOCK)), "the host's world");
    assert_eq!(frame_item(&rig.cs[1].world), Some(Item::Block(block::IRON_BLOCK)), "the other joiner's world");
    // The host turns it in its own world (its client's click, `block_use`).
    let host_world = &mut rig.host.as_mut().unwrap().world;
    crate::block_use::use_block(host_world, pos, UseKind::ItemFrame, None, &|_| true).unwrap();
    rig.ticks(2);
    for n in 0..2 {
        assert!(matches!(rig.cs[n].view_at(cell), Some(BlockView::ItemFrame { rotation: 1, .. })), "joiner {n} sees the host's turn");
        assert_eq!(rig.cs[n].world.item_frame_at(pos).unwrap().rotation, 1);
    }
    // Nothing changed: nothing more is sent.
    let seen = rig.cs[1].views.len();
    rig.ticks(5);
    assert_eq!(rig.cs[1].views.len(), seen, "an unchanged view is not sent again");
}

/// A joiner is shown only the blocks in chunks it holds (its sent-set).
#[test]
fn a_view_goes_only_to_joiners_holding_its_chunk() {
    let mut rig = Rig::dedicated("sent-set", 1);
    let near = rig.place(2, block::BEE_HIVE);
    rig.world().insert_hive((near[0], near[1], near[2]), crate::bee_hive::HiveData { bees_inside: 0, honey_level: 4 });
    let far = [near[0] + 160, near[1], near[2]];
    rig.world().set_block(far[0], far[1], far[2], block::BEE_HIVE);
    rig.world().insert_hive((far[0], far[1], far[2]), crate::bee_hive::HiveData { bees_inside: 0, honey_level: 4 });
    rig.ticks(2);
    assert_eq!(rig.cs[0].view_at(near), Some(&BlockView::Hive { honey_level: 4 }), "first sight of a held chunk");
    assert_eq!(rig.cs[0].view_at(far), None, "a chunk it doesn't hold: nothing");
}

// ─── Joined-client sims off ────────────────────────────────────────────────

#[test]
fn a_joined_client_runs_no_composter_hive_or_rack_sim() {
    // Source lint, in `block_machines`' tradition (the client tick needs a
    // GPU; `TestHost` has no client). A joiner's composters, hives and racks
    // are the server's, shown to it as views: its own sweeps would age,
    // fill and season its copy between views, and its toasts would announce
    // logs the server never seasoned. Each stays behind
    // `remote_client.is_none()`.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("joiner-sim lint: cannot read {} ({e}). If the client tick moved, update this lint's path.", path.display())
    });
    let lines: Vec<&str> = raw.lines().collect();
    let sites: [(&str, usize); 3] = [
        ("crate::composter::tick_all(&mut self.world)", 3),
        ("crate::bee_hive::accumulate_honey(&mut self.world", 3),
        ("self.world.drying_racks.keys().copied().collect()", 3),
    ];
    for (needle, back) in sites {
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(needle))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(hits.len(), 1, "joiner-sim lint: `{needle}` should appear once in game_loop.rs, found {hits:?}");
        let i = hits[0];
        assert!(
            lines[i.saturating_sub(back)..=i].iter().any(|l| l.contains("remote_client.is_none()")),
            "game_loop.rs:{}: `{needle}` runs on a joined client — gate it behind `self.remote_client.is_none()`",
            i + 1
        );
    }
}

// ─── C3b-2-fix ─────────────────────────────────────────────────────────────

/// M2 — a modified client claims an item it doesn't hold: each use's take is
/// believed only within the joiner's bound (the believed-deposit bucket, 64
/// deep, refilled at 4 a second), and past it the use is refused
/// (`NothingToTake`) and nothing is framed. The items it can make out of
/// nothing are bounded by 64 + 4 a second, as believed container deposits.
#[test]
fn believed_block_use_pays_are_bounded_like_believed_deposits() {
    let mut rig = Rig::dedicated("believed", 1);
    let c = [rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32];
    let mut frames = Vec::new();
    for y in 0..3 {
        for dx in -3..=3 {
            for dz in -3..=3 {
                if (dx, dz) != (0, 0) && frames.len() < 80 {
                    frames.push([c[0] + dx, c[1] + y, c[2] + dz]);
                }
            }
        }
    }
    for &cell in &frames {
        rig.set(cell, block::ITEM_FRAME);
    }
    let diamond = Item::Block(block::DIAMOND_BLOCK);
    let start = rig.hs.server.tick_counter;
    for (n, &cell) in frames.iter().enumerate() {
        rig.cs[0].use_raw(1_000 + n as u32, cell, 0, &diamond);
    }
    let mut answered_at = None;
    for _ in 0..60 {
        rig.tick();
        if answered_at.is_none() && rig.cs[0].outcomes.len() == frames.len() {
            answered_at = Some(rig.hs.server.tick_counter);
        }
    }
    let secs = (answered_at.expect("every use answered") - start).div_ceil(20) as usize;
    let framed = frames
        .iter()
        .filter(|cell| rig.world().item_frame_at((cell[0], cell[1], cell[2])).is_some_and(|f| !f.is_empty()))
        .count();
    let refused: Vec<_> = rig.cs[0].outcomes.iter().filter(|o| !o.accepted).collect();
    assert_eq!(framed + refused.len(), frames.len(), "each use framed one or was refused");
    assert!(framed >= 64, "the bound's depth is believed: {framed}");
    assert!(framed <= 64 + 4 * secs + 1, "no more than 64 + 4/s ({secs} s): {framed}");
    assert!(!refused.is_empty(), "past the bound, refused");
    assert!(refused.iter().all(|o| ItemNote::from_wire(o.note) == ItemNote::NothingToTake && o.window_event == 0));
    let sp = &rig.hs.server.players[rig.cs[0].slot];
    assert_eq!(sp.possession.use_believed as usize, framed, "each framed unit was believed");
    assert_eq!(sp.possession.use_refused as usize, refused.len());
    assert_eq!(rig.ground(&diamond), 0, "nothing else made");
}

/// C3c-1-fix (L-3) — a block use that only WEARS its tool (shears on a hive,
/// `pay` 0) with shears the server's copy doesn't hold is believed within the
/// same bound (`window_ops::believe_wear`), and refused past it: a modified
/// client claiming shears it doesn't hold harvests until the bound refuses.
#[test]
fn believed_wear_only_uses_are_bounded_like_believed_pays() {
    let mut rig = Rig::dedicated("believed-shears", 1);
    let c = [rig.at.x.floor() as i32, rig.at.y as i32, rig.at.z.floor() as i32];
    let mut hives = Vec::new();
    for y in 0..3 {
        for dx in -3..=3 {
            for dz in -3..=3 {
                if (dx, dz) != (0, 0) && hives.len() < 80 {
                    hives.push([c[0] + dx, c[1] + y, c[2] + dz]);
                }
            }
        }
    }
    for &cell in &hives {
        rig.set(cell, block::BEE_HIVE);
        rig.world().insert_hive((cell[0], cell[1], cell[2]), crate::bee_hive::HiveData { bees_inside: 0, honey_level: 3 });
    }
    let start = rig.hs.server.tick_counter;
    for (n, &cell) in hives.iter().enumerate() {
        rig.cs[0].use_raw(2_000 + n as u32, cell, 0, &shears());
    }
    let mut answered_at = None;
    for _ in 0..60 {
        rig.tick();
        if answered_at.is_none() && rig.cs[0].outcomes.len() == hives.len() {
            answered_at = Some(rig.hs.server.tick_counter);
        }
    }
    let secs = (answered_at.expect("every use answered") - start).div_ceil(20) as usize;
    let sheared = rig.cs[0].outcomes.iter().filter(|o| o.accepted).count();
    let refused: Vec<_> = rig.cs[0].outcomes.iter().filter(|o| !o.accepted).cloned().collect();
    assert!(sheared >= 64, "the bound's depth is believed: {sheared}");
    assert!(sheared <= 64 + 4 * secs + 1, "no more than 64 + 4/s ({secs} s): {sheared}");
    assert!(!refused.is_empty(), "past the bound, refused");
    assert!(refused.iter().all(|o| ItemNote::from_wire(o.note) == ItemNote::NothingToTake && o.window_event == 0));
    let full = hives.iter().filter(|cell| rig.world().hive_at((cell[0], cell[1], cell[2])).is_some_and(|h| h.honey_level == 3)).count();
    assert_eq!(full, refused.len(), "a refused use took no honey");
    let sp = &rig.hs.server.players[rig.cs[0].slot];
    assert_eq!(sp.possession.use_believed as usize, sheared, "each shearing was believed");
    assert_eq!(sp.possession.use_refused as usize, refused.len());
}

/// M2 — a take the server's copy covers (the joiner really holds it) never
/// touches the believed bound, even with the bound spent; with the bound
/// spent, a claim the copy can't cover is refused and changes nothing.
#[test]
fn a_take_the_servers_copy_covers_never_touches_the_believed_bound() {
    let mut rig = Rig::dedicated("believed-honest", 1);
    let frame = rig.place(2, block::ITEM_FRAME);
    let other = rig.place(-2, block::ITEM_FRAME);
    rig.give(0, 0, Item::Block(block::DIAMOND_BLOCK), 2);
    let slot = rig.cs[0].slot;
    let spend = |rig: &mut Rig| {
        let now = rig.hs.server.tick_counter + 1;
        let b = &mut rig.hs.server.players[slot].container_sent.believed;
        while b.try_take(1, now) {}
    };
    spend(&mut rig);
    accepted(&rig.use_block(0, frame, UseKind::ItemFrame, 0).clone(), 1);
    assert!(rig.world().item_frame_at((frame[0], frame[1], frame[2])).is_some_and(|f| !f.is_empty()));
    assert_eq!(rig.hs.server.players[slot].possession.use_believed, 0);
    rig.assert_lockstep(0, "an honest take");
    // Iron only the client says it holds, with the bound spent.
    rig.cs[0].inv.set_slot(1, Some(ItemStack::new_block(block::IRON_BLOCK, 1)));
    spend(&mut rig);
    refused(&rig.use_block(0, other, UseKind::ItemFrame, 1).clone(), ItemNote::NothingToTake);
    assert!(rig.world().item_frame_at((other[0], other[1], other[2])).is_none(), "nothing framed, nothing created");
    assert_eq!(rig.cs[0].count(&Item::Block(block::IRON_BLOCK)), 1, "the client keeps it");
    assert_eq!(rig.hs.server.players[slot].possession.use_refused, 1);
}

/// M3 — on a lending host, a joiner's accepted use is remeshed by the host's
/// client (`lent_edit_cells`): the framed iron block is drawn on the host's
/// screen, not only held in its world. A refused use has nothing to redraw.
#[test]
fn a_lending_hosts_client_redraws_a_block_a_joiner_used() {
    let mut rig = Rig::lent("redraw", 1);
    let frame = rig.place(2, block::ITEM_FRAME);
    let bin = rig.place(-2, block::COMPOSTER);
    let _ = rig.hs.take_lent_changes();
    rig.give(0, 0, Item::Block(block::IRON_BLOCK), 1);
    accepted(&rig.use_block(0, frame, UseKind::ItemFrame, 0).clone(), 1);
    let (_, cells) = rig.hs.take_lent_changes();
    assert!(cells.contains(&(frame[0], frame[1], frame[2])), "the host's client remeshes the filled frame: {cells:?}");
    refused(&rig.use_block(0, bin, UseKind::Composter, 5).clone(), ItemNote::NothingToTake);
    let (_, cells) = rig.hs.take_lent_changes();
    assert!(!cells.contains(&(bin[0], bin[1], bin[2])), "a refusal changed nothing to redraw");
}

/// M4 (a) — a joiner's break of a filled frame is refused (a plot it doesn't
/// own): its client had already cleared its copy (the joined break arm), and
/// the server's send-back restores the block AND, its view forgotten with
/// the change, the frame's item.
#[test]
fn a_refused_break_of_a_filled_frame_gives_the_breaker_its_frame_and_item_back() {
    let mut rig = Rig::dedicated("refused-break", 1);
    let cell = rig.place(2, block::ITEM_FRAME);
    let pos = (cell[0], cell[1], cell[2]);
    rig.give(0, 0, Item::Block(block::DIAMOND_BLOCK), 1);
    accepted(&rig.use_block(0, cell, UseKind::ItemFrame, 0).clone(), 1);
    assert!(rig.cs[0].world.item_frame_at(pos).is_some_and(|f| !f.is_empty()));
    rig.world().plots.push(crate::plot::PlotData::from_marker(
        crate::plot::PlotOwner::LocalPlayer(0),
        cell[0],
        cell[1] - 5,
        cell[2],
    ));
    // The joined break arm: the block and this copy's frame go at once.
    let w = &mut rig.cs[0].world;
    w.set_block(cell[0], cell[1], cell[2], block::AIR);
    let _ = crate::block_use::take_on_break(w, pos, block::ITEM_FRAME, false);
    rig.edit(0, pos, block::AIR);
    rig.ticks(3);
    assert_eq!(rig.world().get_block(cell[0], cell[1], cell[2]), block::ITEM_FRAME, "refused");
    assert_eq!(rig.cs[0].world.get_block(cell[0], cell[1], cell[2]), block::ITEM_FRAME, "the send-back restored it");
    let item = rig.cs[0].world.item_frame_at(pos).and_then(|f| f.item.as_ref().map(|s| s.item.clone()));
    assert_eq!(item, Some(Item::Block(block::DIAMOND_BLOCK)), "and the view came back after it");
}

/// M4 (b) — another player breaks a framed frame and re-places an empty
/// one: the first joiner's copy dropped the frame's entity with the break,
/// so it draws the new frame empty (the server sends no view for a frame
/// never used).
#[test]
fn a_frame_another_player_breaks_and_replaces_is_shown_empty() {
    let mut rig = Rig::dedicated("remote-break", 2);
    let cell = rig.place(2, block::ITEM_FRAME);
    let pos = (cell[0], cell[1], cell[2]);
    rig.give(0, 0, Item::Block(block::DIAMOND_BLOCK), 1);
    accepted(&rig.use_block(0, cell, UseKind::ItemFrame, 0).clone(), 1);
    assert!(rig.cs[0].world.item_frame_at(pos).is_some_and(|f| !f.is_empty()));
    rig.edit(1, pos, block::AIR);
    rig.ticks(3);
    assert_eq!(rig.cs[0].world.get_block(cell[0], cell[1], cell[2]), block::AIR);
    rig.edit(1, pos, block::ITEM_FRAME);
    rig.ticks(3);
    assert_eq!(rig.cs[0].world.get_block(cell[0], cell[1], cell[2]), block::ITEM_FRAME, "re-placed");
    assert!(
        rig.cs[0].world.item_frame_at(pos).is_none_or(|f| f.is_empty()),
        "drawn empty, never the broken frame's diamond"
    );
}

/// L5 — shears on a hive are claimed while the use is in flight (they wear):
/// with one pair of nearly worn-out shears, a second hive use isn't sent
/// until the first is answered, so two uses can't both cut honeycomb.
#[test]
fn shears_in_flight_are_claimed_so_worn_out_shears_cut_once() {
    let mut rig = Rig::dedicated("shears-claim", 1);
    let a = rig.place(2, block::BEE_HIVE);
    let b = rig.place(-2, block::BEE_HIVE);
    for cell in [a, b] {
        rig.world().insert_hive((cell[0], cell[1], cell[2]), crate::bee_hive::HiveData { bees_inside: 0, honey_level: 3 });
    }
    let mut worn = Tool::new(ToolType::Shears, ToolMaterial::Iron);
    worn.durability = 1;
    rig.give(0, 0, Item::Tool(worn), 1);
    assert!(rig.cs[0].use_block(a, UseKind::Hive, 0), "the first goes");
    assert!(!rig.cs[0].use_block(b, UseKind::Hive, 0), "the second would wear the claimed shears: not sent");
    rig.ticks(3);
    assert_eq!(rig.cs[0].outcomes.len(), 1);
    assert_eq!(rig.cs[0].count(&mat(MaterialId::Honeycomb)), 3, "one cut, not two");
    assert_eq!(rig.world().hive_at((b[0], b[1], b[2])).unwrap().honey_level, 3, "the second hive untouched");
    assert_eq!(rig.cs[0].count(&Item::Tool(worn)), 0, "the shears wore out");
    rig.assert_lockstep(0, "after the one cut");
}

/// L8 — source lint, in the sims-off lint's tradition (the click arms need a
/// GPU; `TestHost` has no client; the GPU harness drives each one,
/// `game_harness_a_joiners_click_on_each_of_the_five_blocks_asks_the_server`):
/// each of the five joined click arms sends its request and then
/// `continue`s, never falling through to the single-player rule on the
/// joiner's own copy of the block.
#[test]
fn each_joined_block_use_arm_asks_the_server_and_goes_no_further() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("block-use arm lint: cannot read {} ({e})", path.display()));
    let lines: Vec<&str> = raw.lines().collect();
    for kind in ["Hive", "Composter", "ItemFrame", "Campfire", "DryingRack"] {
        let call = format!("crate::block_use::UseKind::{kind});");
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains("self.send_block_use(pidx,") && l.contains(&call))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(hits.len(), 1, "block-use arm lint: one joined {kind} arm expected in game_loop.rs, found {hits:?}");
        let i = hits[0];
        assert!(
            lines[i.saturating_sub(6)..i].iter().any(|l| l.contains("if self.joined()")),
            "game_loop.rs:{}: the {kind} request must be sent only when joined",
            i + 1
        );
        let after = &lines[i + 1..(i + 5).min(lines.len())];
        let Some(stop) = after.iter().position(|l| l.trim() == "continue;") else {
            panic!("game_loop.rs:{}: the joined {kind} arm must `continue` right after asking the server", i + 1)
        };
        assert!(
            after[..stop].iter().all(|l| !l.contains("use_block(")),
            "game_loop.rs:{}: the joined {kind} arm reaches the single-player rule",
            i + 1
        );
    }
}

/// M1 — source lint: when joined, every hand spend a block-use claim could
/// race (a placement, a sown seed or reed, a crop accelerator; C3c-1-fix M-1:
/// a bucket filled or emptied, bone meal on grass, salt; C3c-3a: an art
/// capture's Blueprint Paper) first asks `hand_may_spend`
/// (`JoinerActions::can_spend`, as the Q-drop does), so one block can't be
/// framed AND placed on a slow link. The real arms are driven by
/// `game_harness_a_joiners_placement_waits_for_the_claim_of_a_frame_use_in_flight`
/// and `game_harness_a_joiners_art_capture_waits_for_the_claim_on_its_paper`.
#[test]
fn a_joined_hand_spends_nothing_a_block_use_in_flight_claims() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("game_loop.rs");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("hand-spend lint: cannot read {} ({e})", path.display()));
    let lines: Vec<&str> = raw.lines().collect();
    for needle in [
        ".take_placeable_from_hotbar(hotbar)",
        ".consume_one_material(hotbar, seed_id)",
        ".consume_one_material(hotbar, crate::item::MaterialId::PapyrusReed)",
        ".consume_one_material(hotbar, material)",
        // C3c-1-fix (M-1) — a bucket filled or emptied, bone meal on grass,
        // salt: every spend of a claimable item.
        ".consume_one_material(hotbar, crate::item::MaterialId::Bucket)",
        ".consume_one_material(hotbar, filled)",
        ".consume_one_material(hotbar, crate::item::MaterialId::Bonemeal)",
        ".consume_one_material(hotbar, crate::item::MaterialId::Salt)",
        // C3c-3a — the art capture's paper.
        ".take_one_from_hotbar(hot_art)",
    ] {
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(needle))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(hits.len(), 1, "hand-spend lint: `{needle}` should appear once in game_loop.rs, found {hits:?}");
        let i = hits[0];
        assert!(
            lines[i.saturating_sub(4)..i].iter().any(|l| l.contains("self.hand_may_spend(pidx)")),
            "game_loop.rs:{}: `{needle}` spends without asking `hand_may_spend` (a block use in flight may claim it)",
            i + 1
        );
    }
}

/// C3c-3-fix (M2) — a modified client's `UseBlock` on an item frame that
/// claims a Plan frames nothing: a held claim never decodes to a Plan (the
/// marker placeholder never leaves the joiner's own window), so the server
/// refuses the use and no body-less Plan enters the shared world.
#[test]
fn a_plan_claimed_on_an_item_frame_frames_nothing() {
    let mut rig = Rig::dedicated("frame-plan-claim", 1);
    let cell = rig.place(2, block::ITEM_FRAME);
    let pos = (cell[0], cell[1], cell[2]);
    let plan = Item::Plan(crate::plan::PlanData::debug_3x3_stone());
    let before = rig.cs[0].outcomes.len();
    rig.cs[0].use_raw(7_000, cell, 0, &plan);
    rig.ticks(3);
    assert_eq!(rig.cs[0].outcomes.len(), before + 1, "answered once");
    refused(&rig.cs[0].last().clone(), ItemNote::NothingToTake);
    assert!(rig.world().item_frame_at(pos).is_none_or(|f| f.is_empty()), "the frame holds nothing");
    // A claimed Plan decodes to an empty hand, not a placeholder.
    let w = crate::inventory::item_to_wire_full(&plan);
    assert!(matches!(w, protocol::WireItem::Plan { .. }), "the claim is a Plan on the wire");
}
