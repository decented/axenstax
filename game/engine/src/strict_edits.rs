//! C3d runs 7 and 9 — typed block edits, the rule both halves share and
//! the client's half.
//!
//! - [`judge`] is the Decision 18 classification of one edit, read from a
//!   window's slot (the server's raw R; the client's own window to route
//!   it): a plain placement, a mined break, a definite refusal, or a class
//!   still paused (Creative free edit, same-block Noop, door, non-yielding
//!   manual break). The server's `hosted_server/strict_edit.rs` serves it;
//!   the client sends an edit typed exactly when it judges Place or Break.
//! - [`place_window`] / [`break_window`] are the window halves, the ONE rule
//!   the server stages on raw R and the client predicts with (a placement's
//!   `take_placeable_from_hotbar`, auto-refill included; a break's
//!   `use_tool_at`, a broken tool removed), so an honest client's window
//!   digest is the server's.
//! - [`place_joined`] / [`break_joined`]: a Strict client's plain placement
//!   or mined break is ONE reserved, numbered `ActionIntent::Edit`, never on
//!   the movement packet. As a click is, it is reserved before anything is
//!   predicted (its recipe: the slot, the window sources it changes and the
//!   digest of the window it predicts), then the window is predicted, then
//!   the reservation is sent; its receipt retires it at its boundary (a
//!   break's after its numbered yield is applied). The world half is the
//!   caller's.
//!
//! - [`route_place`] / [`route_break`]: the routing the game loop's place
//!   and break arms take (`GameState::take_for_placement`,
//!   `GameState::break_route`): typed exactly when the session is Strict and
//!   the client's own window judges Place or Break; otherwise the old path.
//!
//! Every other edit a Strict client makes (the paused classes, a use's
//! tagged edit) keeps the movement packet, which a Strict server refuses by
//! closing the session until recovery is wired. Mirror goes through exactly
//! the old path.
//!
//! C3d run 9 — rebased from the parked run-7 work onto `7b0a3e11` and
//! wired into the game loop.
use crate::{
    armour::ArmourItem,
    block::BlockId,
    craft_ui::CraftingUi,
    inventory::{Inventory, ToolUseInfo},
    item::Item,
    protocol,
    remote_client::RemoteClient,
};

/// What one edit is, read from a window's slot at the action's boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrictEdit {
    /// A block-item placement; the slot places exactly the edit's block.
    Place { slot: usize },
    /// A mined break with the slot's tool (`None`: no tool, a bare hand).
    Break { slot: usize, tool: Option<crate::crafting::Tool> },
    /// Definitively invalid: a typed `Refused`, zero effect.
    Refuse(&'static str),
    /// A class not typed yet (Creative free edit, same-block Noop, door,
    /// non-yielding manual break): paused, as every untyped mutation is.
    Unsupported(&'static str),
}

/// Decision 18 — the classification, exhaustive for this class. `held` is
/// the window's item in the action's slot; a claimed hand or
/// `MinedBlock.tool` is never read. `received`: the server judging an edit
/// its client sent typed (the client classifying its own edit passes
/// `false`).
///
/// C3d run 10 (run-9b review MEDIUM-1) — Decision 19 Drift runs before the
/// same-block and non-yielding checks ([`drifted`]): an honest edit on a
/// cell that changed since its client acted (a leaf the server decayed, a
/// block another player mined, a crop someone harvested) is a Drift
/// `Refuse`, its cell sent back, never a paused class.
pub fn judge(old: BlockId, change: &protocol::BlockChange, mined: Option<&protocol::MinedBlock>, slot: usize, held: Option<&Item>, creative: bool, received: Option<Received>) -> StrictEdit {
    use crate::block::{AIR, OAK_DOOR};
    let new = change.new_block;
    if creative { return StrictEdit::Unsupported("Creative free edits await their typed class"); }
    if let Some(r) = received && drifted((old, r.meta), r.seen, new, mined.is_some()) { return StrictEdit::Refuse(DRIFT); }
    if old == new { return StrictEdit::Unsupported("A same-block edit awaits the typed Noop class"); }
    // Decision 20: a door placement is ONE compound `DoorPlace` (never a
    // plain edit). C3d run 18: a mined door half is a Break below, whose
    // server derives the pair once (`clear_door_pair`); any other edit of
    // a door is untagged (refused).
    if new == OAK_DOOR { return StrictEdit::Unsupported("A door placement is one compound DoorPlace"); }
    if let Some(m) = mined {
        if (m.x, m.y, m.z) != (change.x, change.y, change.z) { return StrictEdit::Refuse("mined tag names another cell"); }
        if crate::break_drops::yields_drops(old) && (new == AIR || crate::growth::is_crop(old)) {
            let tool = match held { Some(Item::Tool(t)) => Some(*t), _ => None };
            return StrictEdit::Break { slot, tool };
        }
        return StrictEdit::Unsupported("A non-yielding manual break awaits its typed class");
    }
    if placement_replaceable(old) && new != AIR {
        // Heldness: the slot must place exactly this block.
        return if held.and_then(crate::joiner_inventory::placeable_block) == Some(new) {
            StrictEdit::Place { slot }
        } else {
            StrictEdit::Refuse("the server's copy of the slot doesn't hold the placed block")
        };
    }
    // Untagged emptying edits, untagged in-place changes: no mutation authority.
    StrictEdit::Refuse("an untagged edit is no placement or break")
}

/// Decision 19 (C3d run 12, coordinator, after the run-11 review) — the
/// blocks whose cell meta decides a typed Place or Break's outcome, so whose
/// meta a Drift compares. EMPTY today: growth stages are block ids
/// (`block.rs`, `growth.rs`), and no Place or Break outcome reads the cell's
/// meta (`break_drops.rs`). Meta the server changes without a broadcast must
/// never make an honest edit Drift. Doors (typed in C3d run 18) are NOT on
/// it: a door break's pair is derived from the SERVER's own meta
/// (`clear_door_pair`), so the client's half bit decides nothing, and an
/// open/closed bit must never be on it; any later class whose outcome reads
/// meta adds its own entry when it is typed.
pub const DRIFT_META_BLOCKS: &[BlockId] = &[];

/// The cells a plain placement may fill: AIR and water (`judge`'s Place
/// arm), one replaceable class for Drift too.
pub fn placement_replaceable(block: BlockId) -> bool {
    block == crate::block::AIR || block == crate::block::WATER
}

/// Why a drifted edit is refused (Decision 19).
pub const DRIFT: &str = "Drift: the cell no longer holds what the client acted on";

/// Decision 19 — has the cell of a received typed edit changed under it? A
/// client sends an edit typed only when its own world judged it Place
/// (into a cell other than the placed block) or Break (of a block that
/// yields), so on the server a placement onto its own block, or a mined
/// edit onto its own result or onto a block that yields nothing (air, a
/// fluid, fire), means the cell is no longer what the client acted on.
pub fn drifted(server: (BlockId, u8), seen: (BlockId, u8), new: BlockId, mined: bool) -> bool {
    // C3d run 11 (run-10 review LOW-1, LOW-2) — exact: the server's cell
    // against the cell the client saw (`ActionIntent::Edit::old`).
    // C3d run 12 (coordinator, run-11 review) — ids only: meta counts only
    // for a block on [`DRIFT_META_BLOCKS`] (empty today). For a placement,
    // AIR and water are one replaceable class (the Place arm's own set):
    // the outcome is the same, so a mismatch between them is no Drift.
    let same = server.0 == seen.0 || (!mined && placement_replaceable(server.0) && placement_replaceable(seen.0));
    let meta_counts = DRIFT_META_BLOCKS.contains(&server.0);
    !same
        || (meta_counts && server.1 != seen.1)
        || server.0 == new
        || (mined && !crate::break_drops::yields_drops(server.0))
}

/// What the server's `judge` of a received typed edit knows beyond the
/// cell's block: the cell's meta, and the cell as its client saw it.
#[derive(Clone, Copy, Debug)]
pub struct Received {
    pub meta: u8,
    pub seen: (BlockId, u8),
}

/// A placement's window half: the placed block taken from hotbar `slot`
/// (auto-refill included).
pub fn place_window(inv: &mut Inventory, slot: usize) -> Option<BlockId> {
    inv.take_placeable_from_hotbar(slot)
}

/// A mined break's window half: the tool in `slot` worn (removed when it
/// breaks); nothing for a slot holding no tool.
pub fn break_window(inv: &mut Inventory, slot: usize) -> Option<ToolUseInfo> {
    inv.use_tool_at(slot)
}

/// Which window half an edit's recipe names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EditKind {
    Place,
    Break,
}

/// The journal's recipe for a typed edit (Decision 4: semantic facts and
/// marker sources, never owned Plan bodies).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct EditRecipe {
    pub kind: EditKind,
    pub slot: u8,
    /// The digest of the window the edit predicts (after its window half).
    pub digest: u32,
    /// The initial sources of the places the window half changes.
    pub sources: crate::action_wire::BoundedList<protocol::WireSlot, 50>,
}

impl EditRecipe {
    /// The recipe of a window half staged from `raw` to `after`: the
    /// predicted digest and the changed places' initial sources.
    pub fn from_windows(kind: EditKind, slot: u8, raw: &crate::staged_window::MarkerWindow, after: &crate::staged_window::MarkerWindow, station: crate::window::Station) -> Result<Self, String> {
        let (before, now) = (raw.slots(), after.slots());
        let sources: Vec<protocol::WireSlot> = (0..crate::staged_window::PLAYER_PLACES)
            .map(|i| if before[i] != now[i] { before[i].clone() } else { None })
            .collect();
        Ok(EditRecipe { kind, slot, digest: after.digest_with(station, None), sources: crate::action_wire::BoundedList::new(sources)? })
    }

    /// C3d run 18 — a door placement's recipe (one door taken).
    pub fn new_place(slot: u8, raw: &crate::staged_window::MarkerWindow, after: &crate::staged_window::MarkerWindow, station: crate::window::Station) -> Result<Self, String> {
        Self::from_windows(EditKind::Place, slot, raw, after, station)
    }
}

/// Does this client send a plain placement or a mined break typed? Only a
/// Strict session, and only for an edit its own window judges Place or Break.
pub fn sends_typed(remote: Option<&RemoteClient>, judged: &StrictEdit) -> bool {
    remote.is_some_and(|r| r.inventory_mode == crate::action_identity::InventoryMode::Strict)
        && matches!(judged, StrictEdit::Place { .. } | StrictEdit::Break { .. })
}

/// Reserve one typed edit BEFORE anything is predicted: its window half is
/// staged on a marker copy for the recipe's sources and digest. A failed
/// reservation changes nothing.
#[allow(clippy::too_many_arguments)]
fn reserve(
    remote: &mut RemoteClient,
    ui: &CraftingUi,
    inv: &Inventory,
    armour: &[Option<ArmourItem>; 4],
    kind: EditKind,
    slot: usize,
    change: protocol::BlockChange,
    mined: Option<protocol::MinedBlock>,
    old: (BlockId, u8),
    registry: &crate::block::BlockRegistry,
) -> Result<crate::action_identity::ActionHeader, String> {
    let slot_u8 = u8::try_from(slot).ok().filter(|s| *s < 9).ok_or("A typed edit needs a hotbar slot")?;
    crate::window_prediction::retain_private_window(inv, &ui.cursor_item, &ui.grid)?;
    let raw = crate::staged_window::MarkerWindow::capture_parts(inv, armour, &ui.cursor_item, &ui.grid, registry);
    let mut after = raw.clone();
    {
        let view = after.view();
        match kind {
            EditKind::Place => { place_window(view.inv, slot); }
            EditKind::Break => { break_window(view.inv, slot); }
        }
    }
    let recipe = EditRecipe::from_windows(kind, slot_u8, &raw, &after, ui.station())?;
    let (kind_id, item_id) = inv.slot(slot).map_or((protocol::item_kind::EMPTY, 0), |s| crate::inventory::item_to_ref(&s.item).to_wire());
    let cell = [change.x, change.y, change.z];
    let intent = crate::action_wire::ActionIntent::Edit {
        // Reserved and unread by the server (`ActionIntent::Edit`).
        source_tick: remote.next_input_seq(),
        change,
        hand: (slot_u8, kind_id, item_id),
        mined,
        use_tag: None,
        old,
    };
    let bytes = bincode::serialize(&recipe).map_err(|e| e.to_string())?.into_boxed_slice();
    // C3d run 21 (run-18 review L3) — the journal holds every cell the
    // client predicts: a door break clears its pair too (the game loop's
    // break arm), so the pair derived from the half's own meta is journaled
    // beside it. The server's row carries its live pair (`edit_cells`), so
    // the two sides' cells are the union S3 recovery reads.
    let mut cells = vec![cell];
    if matches!(kind, EditKind::Break) && old.0 == crate::block::OAK_DOOR && let Some(pair) = crate::strict_doors::door_pair_cell(cell, old.1) {
        cells.push(pair);
    }
    remote.reserve_action(intent, bytes, cells.into_boxed_slice())
}

/// A Strict client's plain placement of `change` from hotbar `slot`: the
/// auto-refill setting first (the server stages on it), then the
/// reservation, then the window half; then it is sent. Returns the block
/// the window gave (the caller places it in its world). On `Err` nothing
/// changed unless the reservation was already made and only its send failed
/// (as a click's).
#[allow(clippy::too_many_arguments)]
pub fn place_joined(
    remote: &mut RemoteClient,
    ui: &mut CraftingUi,
    inv: &mut Inventory,
    armour: &mut [Option<ArmourItem>; 4],
    slot: usize,
    change: protocol::BlockChange,
    old: (BlockId, u8),
    registry: &crate::block::BlockRegistry,
) -> Result<Option<BlockId>, String> {
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    let action = reserve(remote, ui, inv, armour, EditKind::Place, slot, change, None, old, registry)?;
    let placed = place_window(inv, slot);
    send_reserved(remote, action);
    Ok(placed)
}

/// A Strict client's mined break of `change` with hotbar `slot`
/// (`claimed`: the tool it shows, carried in the `mined` tag for
/// diagnostics only — the server reads its own copy's slot): the setting,
/// the reservation, the window half (the tool worn), then the send. Returns
/// the wear, for the caller's tool-use feedback. The yield arrives as the
/// server's numbered grants.
#[allow(clippy::too_many_arguments)]
pub fn break_joined(
    remote: &mut RemoteClient,
    ui: &mut CraftingUi,
    inv: &mut Inventory,
    armour: &mut [Option<ArmourItem>; 4],
    slot: usize,
    change: protocol::BlockChange,
    old: (BlockId, u8),
    claimed: Option<crate::crafting::Tool>,
    registry: &crate::block::BlockRegistry,
) -> Result<Option<ToolUseInfo>, String> {
    ui.sync_auto_refill_joined(remote, inv, armour)?;
    let mined = protocol::MinedBlock {
        x: change.x,
        y: change.y,
        z: change.z,
        tool: claimed.map_or(protocol::WireItem::None, |t| crate::inventory::item_to_wire_full(&Item::Tool(t))),
    };
    let action = reserve(remote, ui, inv, armour, EditKind::Break, slot, change, Some(mined), old, registry)?;
    let wear = break_window(inv, slot);
    send_reserved(remote, action);
    Ok(wear)
}

/// Send a reserved edit. Its prediction already stands, so a send failure
/// (only a recovery prepared after the reservation) is logged and the
/// retained reservation is left to recovery, as a click's is; never undone.
fn send_reserved(remote: &mut RemoteClient, action: crate::action_identity::ActionHeader) {
    if let Err(error) = remote.send_reserved_action(action) {
        log::warn!("Reserved block edit {} could not be sent: {error}", action.seq);
    }
}

/// How an edit went: on the movement packet as before, or as a typed action.
#[derive(Debug, PartialEq)]
pub enum Routed<T> {
    /// Not typed: the caller takes the old path (Mirror, a solo or host
    /// world, a paused class).
    Legacy,
    /// Reserved, predicted and sent; the window half's result.
    Typed(T),
}

/// The game loop's routing of a plain placement of `change` from hotbar
/// `slot` into a cell holding `old`: typed (reserved, the window predicted,
/// sent) when [`sends_typed`] says so, else [`Routed::Legacy`] with nothing
/// changed. `Err`: the reservation failed and nothing changed.
#[allow(clippy::too_many_arguments)]
pub fn route_place(
    remote: Option<&mut RemoteClient>,
    ui: &mut CraftingUi,
    inv: &mut Inventory,
    armour: &mut [Option<ArmourItem>; 4],
    slot: usize,
    change: protocol::BlockChange,
    old: (BlockId, u8),
    registry: &crate::block::BlockRegistry,
) -> Result<Routed<Option<BlockId>>, String> {
    if remote.as_ref().is_some_and(|client| client.pack_request_pending()) { return Err("Pack action is waiting".into()); }
    let held = inv.slot(slot).map(|s| s.item.clone());
    let judged = judge(old.0, &change, None, slot, held.as_ref(), false, None);
    match remote {
        Some(remote) if sends_typed(Some(remote), &judged) => place_joined(remote, ui, inv, armour, slot, change, old, registry).map(Routed::Typed),
        _ => Ok(Routed::Legacy),
    }
}

/// The game loop's routing of a mined break of the `old` block at
/// `change`'s cell (to `change.new_block`) with hotbar `slot`: typed (the
/// tool worn) when [`sends_typed`] says so, else [`Routed::Legacy`] with
/// nothing changed. `Err`: the reservation failed and nothing changed.
#[allow(clippy::too_many_arguments)]
pub fn route_break(
    remote: Option<&mut RemoteClient>,
    ui: &mut CraftingUi,
    inv: &mut Inventory,
    armour: &mut [Option<ArmourItem>; 4],
    slot: usize,
    change: protocol::BlockChange,
    old: (BlockId, u8),
    registry: &crate::block::BlockRegistry,
) -> Result<Routed<Option<ToolUseInfo>>, String> {
    if remote.as_ref().is_some_and(|client| client.pack_request_pending()) { return Err("Pack action is waiting".into()); }
    let held = inv.slot(slot).map(|s| s.item.clone());
    let tool = match &held { Some(Item::Tool(t)) => Some(*t), _ => None };
    let mined = protocol::MinedBlock { x: change.x, y: change.y, z: change.z, tool: protocol::WireItem::None };
    let judged = judge(old.0, &change, Some(&mined), slot, held.as_ref(), false, None);
    match remote {
        Some(remote) if sends_typed(Some(remote), &judged) => break_joined(remote, ui, inv, armour, slot, change, old, tool, registry).map(Routed::Typed),
        _ => Ok(Routed::Legacy),
    }
}

/// What the place arm does with a plain placement ([`GameState::take_for_placement`]).
#[derive(Debug, PartialEq)]
pub enum PlaceRoute {
    /// The old path: the block taken here, the edit on the movement packet.
    Legacy(Option<BlockId>),
    /// A typed `Edit`, reserved and sent: the block the window gave; the
    /// caller places it in its world and puts nothing on the movement packet.
    Typed(Option<BlockId>),
    /// The reservation failed: nothing taken, nothing placed.
    Wait,
}

/// What the survival break arm does once the break time is reached
/// ([`GameState::break_route`]).
#[derive(Debug)]
pub enum BreakRoute {
    /// The break time isn't reached.
    NotYet,
    /// The reservation failed: nothing changed, the block stands.
    Wait,
    /// The old path: the edit and its `mined` tag on the movement packet,
    /// the tool worn after the break.
    Legacy,
    /// A typed `Edit`, reserved, its tool already worn (the wear, for the
    /// tool-use feedback), sent: nothing on the movement packet.
    Typed(Option<ToolUseInfo>),
}

impl BreakRoute {
    /// Does the block break this tick?
    pub fn breaks(&self) -> bool { matches!(self, Self::Legacy | Self::Typed(_)) }
    pub fn is_typed(&self) -> bool { matches!(self, Self::Typed(_)) }
}

impl crate::GameState {
    /// The Strict session player `pidx` plays on, if any (only the joined
    /// local player, slot 0, is a remote client's).
    pub(crate) fn strict_remote(&self, pidx: usize) -> bool {
        pidx == 0 && self.remote_client.as_ref().is_some_and(|r| r.inventory_mode == crate::action_identity::InventoryMode::Strict)
    }

    pub(crate) fn typed_edit_failed(&mut self, error: String) {
        log::warn!("Joined block edit could not be retained: {error}");
        self.toast = Some(("Block action is waiting; try again.".into(), web_time::Instant::now() + web_time::Duration::from_secs(2)));
    }

    /// C3d run 9 — the place arm's take for a survival placement of the
    /// hotbar block at `cell` (holding `existing`), clicked on `face`. Mirror
    /// and every unjoined world take it as before; a Strict client reserves
    /// a typed `Edit` (its meta hoisted, `placement_meta`) before the window
    /// is predicted, and the edit never rides the movement packet.
    pub(crate) fn take_for_placement(&mut self, pidx: usize, hotbar: usize, cell: [i32; 3], existing: BlockId, face: [i32; 3]) -> PlaceRoute {
        if self.pack_window_frozen(pidx) { return PlaceRoute::Wait; }
        let new = self.players[pidx].inventory.hotbar_placeable_id(hotbar);
        let Some(new) = new.filter(|_| self.strict_remote(pidx)) else {
            return PlaceRoute::Legacy(self.players[pidx].inventory.take_placeable_from_hotbar(hotbar));
        };
        let fwd = self.players[pidx].camera.forward();
        let meta = crate::placement_meta::placement_meta(new, face, fwd).unwrap_or_else(|| self.world.meta_at(cell[0], cell[1], cell[2]));
        // C3d run 18 (Decision 20) — a door is ONE compound `DoorPlace`
        // naming its bottom; the server derives both halves. Never a plain
        // `Edit` (whose `judge` leaves doors to this class).
        if new == crate::block::OAK_DOOR {
            let facing = crate::meta::facing(meta).to_bits();
            let p = &mut self.players[pidx];
            let remote = self.remote_client.as_mut().expect("a Strict seat is joined");
            return match crate::strict_doors::place_door_joined(remote, &mut p.crafting_ui, &mut p.inventory, &mut p.armour_slots, hotbar, cell, facing, &self.registry) {
                Ok(placed) => PlaceRoute::Typed(placed),
                Err(error) => { self.typed_edit_failed(error); PlaceRoute::Wait }
            };
        }
        let change = protocol::BlockChange { x: cell[0], y: cell[1], z: cell[2], new_block: new, meta };
        let seen = (existing, self.world.meta_at(cell[0], cell[1], cell[2]));
        let p = &mut self.players[pidx];
        match route_place(self.remote_client.as_mut(), &mut p.crafting_ui, &mut p.inventory, &mut p.armour_slots, hotbar, change, seen, &self.registry) {
            Ok(Routed::Legacy) => PlaceRoute::Legacy(p.inventory.take_placeable_from_hotbar(hotbar)),
            Ok(Routed::Typed(placed)) => PlaceRoute::Typed(placed),
            Err(error) => { self.typed_edit_failed(error); PlaceRoute::Wait }
        }
    }

    /// C3d run 9 — the survival break arm, once the break time is reached:
    /// Mirror and every unjoined world break as before; a Strict client's
    /// mined break is reserved as a typed `Edit` BEFORE the world changes,
    /// its tool worn by the shared rule. A failed reservation leaves the
    /// block standing and restarts the break.
    pub(crate) fn break_route(&mut self, pidx: usize, pos: [i32; 3], blk: BlockId, tool: Option<crate::crafting::Tool>) -> BreakRoute {
        if self.pack_window_frozen(pidx) { return BreakRoute::Wait; }
        if !self.strict_remote(pidx) { return BreakRoute::Legacy; }
        let cell = (pos[0], pos[1], pos[2]);
        // The cell's replacement (AIR, or a harvested crop's) and the meta
        // the old path's `broadcast_change` would carry after it.
        let replacement = crate::break_drops::break_yield(&self.world, &self.registry, blk, cell, tool.as_ref(), self.tick_counter, &self.pop_keys()).replacement;
        let meta = if replacement == crate::block::AIR { 0 } else { self.world.meta_at(pos[0], pos[1], pos[2]) };
        let change = protocol::BlockChange { x: pos[0], y: pos[1], z: pos[2], new_block: replacement, meta };
        let seen = (blk, self.world.meta_at(pos[0], pos[1], pos[2]));
        let p = &mut self.players[pidx];
        let slot = p.hotbar_slot;
        match route_break(self.remote_client.as_mut(), &mut p.crafting_ui, &mut p.inventory, &mut p.armour_slots, slot, change, seen, &self.registry) {
            Ok(Routed::Legacy) => BreakRoute::Legacy,
            Ok(Routed::Typed(wear)) => BreakRoute::Typed(wear),
            Err(error) => {
                p.break_progress = 0;
                self.typed_edit_failed(error);
                BreakRoute::Wait
            }
        }
    }
}

#[cfg(test)]
mod drift_rule_tests {
    use super::*;
    use crate::block::{AIR, COBBLESTONE, DIRT, OAK_LEAVES, WATER, WHEAT_STAGE_2, WHEAT_STAGE_3};

    /// C3d run 12 — Drift compares ids; meta never (the allow-list is empty);
    /// AIR and water are one replaceable class for a placement.
    #[test]
    fn c3d_drift_compares_ids_with_one_replaceable_class() {
        assert!(DRIFT_META_BLOCKS.is_empty(), "no Place or Break outcome reads meta today");
        assert!(!drifted((OAK_LEAVES, 5), (OAK_LEAVES, 0), AIR, true), "a leaf's unbroadcast meta never makes an honest break Drift");
        assert!(!drifted((WATER, 3), (AIR, 0), COBBLESTONE, false), "client saw AIR, the server has water: same outcome");
        assert!(!drifted((AIR, 0), (WATER, 7), COBBLESTONE, false), "and the reverse");
        assert!(drifted((DIRT, 0), (AIR, 0), COBBLESTONE, false), "another block stands there");
        assert!(drifted((WHEAT_STAGE_3, 0), (WHEAT_STAGE_2, 0), AIR, true), "a crop that grew a stage is an id mismatch");
        // Run 13 (run-12 review LOW-4): the `!mined` guard on the
        // replaceable class is unobservable — a mined edit onto AIR or water
        // is already Drift because neither yields. Pin THAT, which can fail.
        assert!(!crate::break_drops::yields_drops(WATER) && !crate::break_drops::yields_drops(AIR), "AIR and water yield nothing");
        assert!(drifted((WATER, 0), (AIR, 0), AIR, true), "so a mined edit onto either is Drift");
    }
}
