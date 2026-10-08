//! C3b-2 (protocol v79) — right-clicking a composter, drying rack, campfire,
//! item frame or bee hive: ONE rule per block, shared by every path
//! (`docs/foundations/2026-10-07-c3-server-owned-inventory.md`, C3b row;
//! Spec 04 §4.2f, Spec 05).
//!
//! Each `use_*` fn takes the block's state, the held stack (`None` = an
//! empty hand) and what the player has room for, changes the state, and
//! says what the player pays and gains ([`Used`]) or why nothing happened
//! (an [`ItemNote`]). The caller applies the rest:
//! - **Single-player and a host's own seats** (`GameState`'s right-click
//!   arms): pay from the hotbar slot, add the gain, play the feedback. Their
//!   behaviour is what it was before the rules moved here.
//! - **The server, for a joiner** (`HostedServer`, `ItemAction::UseBlock`):
//!   the rule runs on the server's REAL block entity (created if missing, as
//!   the client's open always did); the pay is an owed take (or, for shears,
//!   a wear) and the gain a grant, each a numbered window event
//!   (`window_events`), so both copies of the joiner's window apply them at
//!   the same point. A joiner's room is never checked: it is given the whole
//!   gain, and what its client can't hold comes back to the world as a
//!   ground item (`ItemAction::GrantUnfit`; the C2b-fix BRIDGE until C3d).
//!
//! Lighting a campfire (flint and steel, friction) is not a use here: it is a
//! block edit (FU3), and C3c's local uses. The item frame's "take" is
//! breaking it ([`take_on_break`]).

use crate::block::{self, BlockId};
use crate::item::{Item, ItemStack, MaterialId};
use crate::item_actions::ItemNote;
use crate::world::World;

/// The blocks this module rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseKind {
    Composter,
    DryingRack,
    /// Lit or not (`CAMPFIRE`, `CAMPFIRE_UNLIT`).
    Campfire,
    ItemFrame,
    Hive,
}

impl UseKind {
    /// The kind of block `b` is, if it is one of these.
    pub fn of_block(b: BlockId) -> Option<Self> {
        match b {
            block::COMPOSTER => Some(UseKind::Composter),
            block::DRYING_RACK => Some(UseKind::DryingRack),
            block::CAMPFIRE | block::CAMPFIRE_UNLIT => Some(UseKind::Campfire),
            block::ITEM_FRAME => Some(UseKind::ItemFrame),
            block::BEE_HIVE => Some(UseKind::Hive),
            _ => None,
        }
    }
}

impl UseKind {
    /// The kind a block view shows.
    pub fn of_view(kind: crate::protocol::BlockViewKind) -> Self {
        use crate::protocol::BlockViewKind as V;
        match kind {
            V::ItemFrame => UseKind::ItemFrame,
            V::Campfire => UseKind::Campfire,
            V::DryingRack => UseKind::DryingRack,
            V::Composter => UseKind::Composter,
            V::Hive => UseKind::Hive,
        }
    }

    /// The kind a block entity is the state of, if one of these (a drying
    /// rack's state lives in `World::drying_racks`, never here).
    fn of_entity(e: &crate::world::BlockEntityData) -> Option<Self> {
        use crate::world::BlockEntityData as E;
        match e {
            E::ItemFrame(_) => Some(UseKind::ItemFrame),
            E::Campfire(_) => Some(UseKind::Campfire),
            E::Composter(_) => Some(UseKind::Composter),
            E::Hive(_) => Some(UseKind::Hive),
            _ => None,
        }
    }

    /// Does the block at `cell` in `world` stand as this kind?
    pub fn stands_at(self, world: &World, cell: (i32, i32, i32)) -> bool {
        UseKind::of_block(world.get_block(cell.0, cell.1, cell.2)) == Some(self)
    }
}

/// C3b-2-fix (M4) — the block at `cell` went `old → new`: when that took it
/// out of its kind (a composter, drying rack, campfire, item frame or hive
/// broken or replaced), drop the state the kind left there — no spill (the
/// world holding the real state spills it: [`take_on_break`], or a
/// campfire's `campfire::on_block_edit`). A lit/unlit campfire flip is the
/// same kind and keeps the fire. Run by every received block change
/// (`World::apply_remote_block_change`), so a joiner never keeps a framed
/// item for the next frame placed in that cell to draw.
pub fn drop_left_state(world: &mut World, cell: (i32, i32, i32), old: BlockId, new: BlockId) {
    let Some(kind) = UseKind::of_block(old) else { return };
    if UseKind::of_block(new) == Some(kind) {
        return;
    }
    if kind == UseKind::DryingRack {
        if world.drying_racks.remove(&cell).is_some() {
            world.mark_edited(cell);
        }
    } else if world.block_entities.get(&cell).and_then(UseKind::of_entity) == Some(kind) {
        world.remove_block_entity(cell);
    }
}

/// What one use did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    /// Nothing visible.
    #[default]
    None,
    /// Composter: one unit loaded.
    Loaded,
    /// Composter: the aged output collected.
    Collected,
    /// Drying rack: a green log hung up.
    Hung,
    /// Drying rack: a seasoned log taken down.
    Seasoned,
    /// Campfire: fuel added (it relit if it was smouldering: [`Used::relit`]).
    Fuelled,
    /// Campfire: raw food put on to cook.
    OnTheFire,
    /// Campfire: cooked food taken off.
    TookCooked,
    /// Item frame: an item mounted.
    Framed,
    /// Item frame: the framed item turned a step.
    Rotated,
    /// Hive: a bucket of honey scooped.
    Scooped,
    /// Hive: honeycomb sheared.
    Sheared,
}

/// What a use costs and gives the player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Used {
    /// Units of the held item it takes (0 or 1).
    pub pay: u8,
    /// The held tool wears once (shears on a hive) instead.
    pub wear: bool,
    /// What the player gains, in order.
    pub gain: Vec<ItemStack>,
    /// A smouldering campfire was fuelled: it is lit again, and its block
    /// must go `CAMPFIRE_UNLIT → CAMPFIRE` (the caller's, with the light and
    /// the smoke pillar that go with it).
    pub relit: bool,
    pub effect: Effect,
}

/// The outcome of one use: what it did, or why nothing happened (the use
/// changed nothing).
pub type UseResult = Result<Used, ItemNote>;

fn material(held: Option<&ItemStack>) -> Option<MaterialId> {
    match held.map(|s| &s.item) {
        Some(Item::Material(m)) => Some(*m),
        _ => None,
    }
}

/// The composter: a compostable item in hand loads one unit
/// (`composter::try_load_input`); otherwise — an empty or other hand, or an
/// input slot that won't take it — the aged output is collected.
pub fn use_composter(state: &mut crate::workstation::WorkstationState, held: Option<&ItemStack>) -> UseResult {
    if let Some(stack) = held
        && crate::composter::is_compostable(stack)
        && crate::composter::try_load_input(state, stack)
    {
        return Ok(Used { pay: 1, effect: Effect::Loaded, ..Used::default() });
    }
    match state.output.take() {
        Some(out) => Ok(Used { gain: vec![out], effect: Effect::Collected, ..Used::default() }),
        None => Err(ItemNote::NothingToTake),
    }
}

/// The drying rack: a green log in hand hangs in the first empty slot (or
/// the rack is full); an empty hand takes down the first seasoned log — or,
/// when `room` says it wouldn't fit, leaves it seasoned on the rack. Anything
/// else in hand does nothing.
pub fn use_drying_rack(
    rack: &mut crate::drying_rack::DryingRackData,
    held: Option<&ItemStack>,
    room: &dyn Fn(&ItemStack) -> bool,
) -> UseResult {
    if held.is_some() {
        let species = material(held).and_then(crate::drying_rack::species_of_green).ok_or(ItemNote::NothingToTake)?;
        if !rack.try_place_green(species) {
            return Err(ItemNote::RackFull);
        }
        return Ok(Used { pay: 1, effect: Effect::Hung, ..Used::default() });
    }
    let species = rack.first_mature_slot().and_then(|idx| rack.slots[idx].species).ok_or(ItemNote::NotReady)?;
    let out = ItemStack::new_material(crate::drying_rack::mature_output(species), 1);
    if !room(&out) {
        return Err(ItemNote::InventoryFull);
    }
    rack.take_mature();
    Ok(Used { gain: vec![out], effect: Effect::Seasoned, ..Used::default() })
}

/// The campfire, past the ignition gestures: fuel in hand burns (a
/// smouldering fire relights; leaves and green logs smoke); raw food goes in
/// the first empty cooking slot; an empty hand takes the first cooked item
/// off (when `room` says it fits). Anything else does nothing.
pub fn use_campfire(
    cf: &mut crate::campfire::CampfireData,
    held: Option<&ItemStack>,
    room: &dyn Fn(&ItemStack) -> bool,
) -> UseResult {
    let Some(stack) = held else {
        let Some((idx, cooked)) = cf.first_cooked_slot() else { return Err(ItemNote::NothingToTake) };
        let out = ItemStack::new_material(cooked, 1);
        if !room(&out) {
            return Err(ItemNote::InventoryFull);
        }
        cf.slots[idx] = crate::campfire::CookSlot::default();
        return Ok(Used { gain: vec![out], effect: Effect::TookCooked, ..Used::default() });
    };
    let (fuel_material, fuel_block) = match &stack.item {
        Item::Material(m) => (Some(*m), None),
        Item::Block(b) => (None, Some(*b)),
        _ => (None, None),
    };
    if let Some(ticks) = crate::campfire::fuel_value(fuel_material, fuel_block) {
        let relit = cf.is_smouldering();
        cf.add_fuel(ticks);
        if relit {
            // A smouldering fire takes fuel and lights at once: no
            // friction or flint and steel needed.
            cf.smoulder_ticks = 0;
        }
        if crate::campfire::is_smoky_fuel(fuel_material, fuel_block) {
            let bump = match (fuel_material, fuel_block) {
                (_, Some(block::OAK_LEAVES)) => crate::campfire::SMOKE_TICKS_PER_LEAF,
                (Some(MaterialId::GreenLog), _) => crate::campfire::SMOKE_TICKS_PER_GREEN_LOG,
                _ => 0,
            };
            cf.smoke_ticks = cf.smoke_ticks.saturating_add(bump);
        }
        return Ok(Used { pay: 1, relit, effect: Effect::Fuelled, ..Used::default() });
    }
    match fuel_material {
        Some(m) if crate::campfire::is_raw_cookable(m) => {
            if cf.try_place_raw(m) {
                Ok(Used { pay: 1, effect: Effect::OnTheFire, ..Used::default() })
            } else {
                Err(ItemNote::FireFull)
            }
        }
        _ => Err(ItemNote::NothingToTake),
    }
}

/// The item frame: an empty frame mounts one of what is in hand; a filled
/// one turns its item a step. An empty frame and an empty hand do nothing.
pub fn use_item_frame(frame: &mut crate::item_frame::ItemFrameData, held: Option<&ItemStack>) -> UseResult {
    if !frame.is_empty() {
        frame.rotate();
        return Ok(Used { effect: Effect::Rotated, ..Used::default() });
    }
    let stack = held.ok_or(ItemNote::NothingToTake)?;
    if !frame.try_insert(stack.clone()) {
        return Err(ItemNote::NothingToTake);
    }
    Ok(Used { pay: 1, effect: Effect::Framed, ..Used::default() })
}

/// The bee hive (`bee_hive::resolve_right_click`): a bucket scoops a jar of
/// honey (the bucket is taken), shears cut three honeycomb (the shears
/// wear); either needs honey. Anything else does nothing.
pub fn use_hive(hive: &mut crate::bee_hive::HiveData, held: Option<&ItemStack>) -> UseResult {
    use crate::bee_hive::HiveRightClickOutcome as O;
    let empty = hive.is_empty();
    match crate::bee_hive::resolve_right_click(hive, held.map(|s| &s.item)) {
        O::Give { item } => Ok(Used { pay: 1, gain: vec![item], effect: Effect::Scooped, ..Used::default() }),
        O::GiveAndUseTool { item } => {
            Ok(Used { wear: true, gain: vec![item], effect: Effect::Sheared, ..Used::default() })
        }
        // Single-player's two hints: an empty hive says so, whatever is in
        // hand; a hive with honey asks for a bucket or shears.
        O::Nothing if empty => Err(ItemNote::HiveEmpty),
        O::Nothing => Err(ItemNote::HiveNeedsTool),
    }
}

/// Use the `kind` block at `cell` in `world` with `held`: its rule runs on
/// the cell's state (a fresh one if it has none yet), and the state is kept
/// only when the rule accepts — a refusal (`NothingToTake`, `HiveEmpty`,
/// `NotReady`, `RackFull`, `FireFull`, …) creates, marks and streams nothing
/// (C3b-2-fix, L7). The caller has checked the block is `kind` and applies
/// [`Used::relit`]'s block change.
pub fn use_block(
    world: &mut World,
    cell: (i32, i32, i32),
    kind: UseKind,
    held: Option<&ItemStack>,
    room: &dyn Fn(&ItemStack) -> bool,
) -> UseResult {
    use_block_admitted(world, cell, kind, held, room, &mut |_| Ok(()))
}

/// [`use_block`], with `admit` shown what an accepted rule would do before
/// anything is kept: an `Err` from it refuses the use with that note, and the
/// cell's state stays exactly as it was. The server's believed-pay bound
/// (C3b-2-fix, M2: `HostedServer::serve_block_use`) is such a check.
pub fn use_block_admitted(
    world: &mut World,
    cell: (i32, i32, i32),
    kind: UseKind,
    held: Option<&ItemStack>,
    room: &dyn Fn(&ItemStack) -> bool,
    admit: &mut dyn FnMut(&Used) -> Result<(), ItemNote>,
) -> UseResult {
    let mut judged = |r: UseResult| r.and_then(|used| admit(&used).map(|()| used));
    match kind {
        UseKind::Composter => {
            let mut state = world.composter_at(cell).cloned().unwrap_or_else(crate::workstation::WorkstationState::new);
            let used = judged(use_composter(&mut state, held))?;
            world.insert_composter(cell, state);
            Ok(used)
        }
        UseKind::DryingRack => {
            let mut rack = world.drying_racks.get(&cell).cloned().unwrap_or_default();
            let used = judged(use_drying_rack(&mut rack, held, room))?;
            // A raw side table: the edit is noted by hand (Phase B2b).
            world.mark_edited(cell);
            world.drying_racks.insert(cell, rack);
            Ok(used)
        }
        UseKind::Campfire => {
            let mut fire = world.campfire_at(cell).cloned().unwrap_or_default();
            let used = judged(use_campfire(&mut fire, held, room))?;
            world.insert_campfire(cell, fire);
            Ok(used)
        }
        UseKind::ItemFrame => {
            let mut frame = world.item_frame_at(cell).cloned().unwrap_or_else(crate::item_frame::ItemFrameData::new);
            let used = judged(use_item_frame(&mut frame, held))?;
            world.insert_item_frame(cell, frame);
            Ok(used)
        }
        UseKind::Hive => {
            let mut hive = world.hive_at(cell).copied().unwrap_or_default();
            let used = judged(use_hive(&mut hive, held))?;
            world.insert_hive(cell, hive);
            Ok(used)
        }
    }
}

/// Does `stack` fit `inv` whole? Single-player's room for a use's gain (a
/// seasoned log, cooked food): what doesn't fit stays where it was.
pub fn has_room(inv: &crate::inventory::Inventory, stack: &ItemStack) -> bool {
    inv.clone().add_item(stack.clone()).is_none()
}

/// The toast a use that did `effect` shows (UK English), if any: the same
/// words single-player has always shown, and a joiner sees once its outcome
/// comes back. Raw food on the fire has its own ([`on_the_fire_toast`]).
pub fn effect_toast(effect: &Effect) -> Option<&'static str> {
    match effect {
        Effect::Hung => Some("Log added — drying"),
        Effect::Seasoned => Some("Seasoned log!"),
        Effect::Scooped => Some("Scooped a Honey Jar from the hive."),
        Effect::Sheared => Some("Sheared 3 Honeycomb from the hive."),
        _ => None,
    }
}

/// What raw food just put on the campfire at `cell` says: it only cooks on a
/// lit, fuelled fire — otherwise a nudge to light it.
pub fn on_the_fire_toast(world: &World, cell: [i32; 3]) -> &'static str {
    let lit = world.get_block(cell[0], cell[1], cell[2]) == block::CAMPFIRE
        && crate::campfire::can_ignite(world.campfire_at((cell[0], cell[1], cell[2])));
    if lit { "On the fire — cooking…" } else { "On the fire — light it to start cooking" }
}

/// The toast a drying rack's refusal shows: its note's, with the seasoning
/// progress of `rack` (this client's copy: a joiner's is the server's view)
/// for "not ready yet".
pub fn rack_note_toast(note: ItemNote, rack: Option<&crate::drying_rack::DryingRackData>) -> Option<String> {
    if note == ItemNote::NotReady {
        let pct = rack.map(|d| (d.max_progress() * 100.0).round() as u32).unwrap_or(0);
        return Some(format!("Not ready yet — {pct}%"));
    }
    note.toast().map(str::to_string)
}

/// How many of `held` a use of a `kind` block might take — what a joined
/// client claims while its request is in flight (`JoinerActions::can_afford`),
/// so one bucket can't scoop two hives on a slow link. 1 for anything the
/// rule could take (a compostable, a green log, a fuel or raw food, anything
/// for a frame, a bucket for a hive) or wear (shears on a hive, C3b-2-fix L5:
/// two shears uses in flight on worn-out shears can't both pay); 0 for an
/// empty hand.
pub fn claim(kind: UseKind, held: Option<&Item>) -> u8 {
    let Some(item) = held else { return 0 };
    let stack = ItemStack { item: item.clone(), count: 1 };
    let takes = match kind {
        UseKind::Composter => crate::composter::is_compostable(&stack),
        UseKind::DryingRack => material(Some(&stack)).and_then(crate::drying_rack::species_of_green).is_some(),
        UseKind::Campfire => {
            let (m, b) = match item {
                Item::Material(m) => (Some(*m), None),
                Item::Block(b) => (None, Some(*b)),
                _ => (None, None),
            };
            crate::campfire::fuel_value(m, b).is_some() || m.is_some_and(crate::campfire::is_raw_cookable)
        }
        UseKind::ItemFrame => true,
        UseKind::Hive => {
            matches!(item, Item::Material(MaterialId::Bucket))
                || matches!(item, Item::Tool(t) if t.tool_type == crate::crafting::ToolType::Shears)
        }
    };
    u8::from(takes)
}

/// Clear what a broken composter, drying rack or item frame at `cell` held
/// from `world` and return it to spill: an item frame's framed item (its
/// "take"), a rack's logs (as green logs, `drying_rack::cleanup_drying_rack`;
/// none in `creative`, single-player's creative rule) and a composter's input
/// and output. A hive holds no items; a campfire's cooking is
/// `campfire::on_block_edit`'s. The state always goes, so the next such block
/// placed in that cell starts empty.
///
/// C3b-2-fix (L6) — one break rule for every seat: the server runs it for a
/// joiner's accepted break (`HostedServer::spill_used_block`, ground items at
/// the block), single-player and a host's own seats from their break arms
/// (the frame's item and a composter's contents at the block, a rack's logs
/// into the breaker's inventory), and a joined client on its own copy, which
/// spills nothing of its view (the server spills the real one).
pub fn take_on_break(world: &mut World, cell: (i32, i32, i32), old: BlockId, creative: bool) -> Vec<ItemStack> {
    let mut spill = Vec::new();
    match UseKind::of_block(old) {
        Some(UseKind::ItemFrame) => {
            if let Some(frame) = world.item_frame_at_mut(cell) {
                spill.extend(frame.take());
            }
            world.remove_block_entity(cell);
        }
        Some(UseKind::DryingRack) => {
            let logs = crate::drying_rack::cleanup_drying_rack(world, cell.0, cell.1, cell.2);
            if !creative {
                spill.extend(logs);
            }
        }
        Some(UseKind::Composter) => {
            if let Some(c) = world.composter_at_mut(cell) {
                spill.extend(c.input.take());
                spill.extend(c.output.take());
            }
            world.remove_block_entity(cell);
        }
        Some(UseKind::Hive) => world.remove_block_entity(cell),
        Some(UseKind::Campfire) | None => {}
    }
    spill.retain(|s| s.count > 0);
    spill
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bee_hive::HiveData;
    use crate::campfire::{CampfireData, CookSlot, COOK_TICKS_PER_ITEM};
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::drying_rack::{DryingRackData, LogSpecies, RackSlot, RACK_SLOTS, SEASON_TICKS};
    use crate::item_frame::ItemFrameData;
    use crate::workstation::WorkstationState;

    fn mat(m: MaterialId) -> ItemStack {
        ItemStack::new_material(m, 5)
    }

    fn room(_: &ItemStack) -> bool {
        true
    }

    fn full(_: &ItemStack) -> bool {
        false
    }

    fn shears() -> ItemStack {
        ItemStack { item: Item::Tool(Tool::new(ToolType::Shears, ToolMaterial::Iron)), count: 1 }
    }

    // ── Composter ──

    #[test]
    fn a_compostable_loads_one_and_the_output_is_collected_otherwise() {
        let mut c = WorkstationState::new();
        let used = use_composter(&mut c, Some(&mat(MaterialId::WheatSeeds))).unwrap();
        assert_eq!((used.pay, used.effect), (1, Effect::Loaded));
        assert_eq!(c.input.as_ref().unwrap().count, 1, "one unit, never the stack");
        c.output = Some(ItemStack::new_material(MaterialId::Compost, 3));
        // A different compostable won't stack on the input: the output comes out.
        let used = use_composter(&mut c, Some(&mat(MaterialId::Compost))).unwrap();
        assert_eq!((used.pay, used.effect), (0, Effect::Collected));
        assert_eq!(used.gain, vec![ItemStack::new_material(MaterialId::Compost, 3)]);
        assert!(c.output.is_none());
        assert_eq!(use_composter(&mut c, None), Err(ItemNote::NothingToTake), "nothing to take");
    }

    // ── Drying rack ──

    #[test]
    fn a_green_log_hangs_and_a_full_rack_refuses() {
        let mut r = DryingRackData::default();
        for _ in 0..RACK_SLOTS {
            assert_eq!(use_drying_rack(&mut r, Some(&mat(MaterialId::GreenLog)), &room).unwrap().pay, 1);
        }
        assert_eq!(use_drying_rack(&mut r, Some(&mat(MaterialId::GreenLog)), &room), Err(ItemNote::RackFull));
        assert_eq!(
            use_drying_rack(&mut r, Some(&mat(MaterialId::Stick)), &room),
            Err(ItemNote::NothingToTake),
            "anything else in hand does nothing"
        );
    }

    #[test]
    fn an_empty_hand_takes_a_seasoned_log_or_leaves_it_when_full() {
        let mut r = DryingRackData::default();
        assert_eq!(use_drying_rack(&mut r, None, &room), Err(ItemNote::NotReady));
        r.slots[3] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: SEASON_TICKS };
        assert_eq!(use_drying_rack(&mut r, None, &full), Err(ItemNote::InventoryFull));
        assert!(r.slots[3].is_mature(), "left seasoned on the rack");
        let used = use_drying_rack(&mut r, None, &room).unwrap();
        assert_eq!(used.gain, vec![ItemStack::new_material(MaterialId::SeasonedLog, 1)]);
        assert!(r.slots[3].is_empty());
    }

    // ── Campfire ──

    #[test]
    fn fuel_burns_and_relights_a_smouldering_fire_and_smoky_fuel_smokes() {
        let mut cf = CampfireData { smoulder_ticks: 100, ..Default::default() };
        let used = use_campfire(&mut cf, Some(&mat(MaterialId::GreenLog)), &room).unwrap();
        assert_eq!((used.pay, used.relit, used.effect), (1, true, Effect::Fuelled));
        assert_eq!(cf.fuel_ticks, 30 * 20);
        assert_eq!(cf.smoulder_ticks, 0);
        assert_eq!(cf.smoke_ticks, crate::campfire::SMOKE_TICKS_PER_GREEN_LOG);
        let used = use_campfire(&mut cf, Some(&ItemStack::new_block(block::OAK_PLANKS, 2)), &room).unwrap();
        assert!(!used.relit, "a burning fire just takes the fuel");
        assert_eq!(cf.fuel_ticks, 30 * 20 + 12 * 20);
    }

    #[test]
    fn raw_food_cooks_until_the_fire_is_full_and_cooked_food_comes_off() {
        let mut cf = CampfireData::default();
        for _ in 0..crate::campfire::CAMPFIRE_SLOTS {
            assert_eq!(use_campfire(&mut cf, Some(&mat(MaterialId::RawBeef)), &room).unwrap().effect, Effect::OnTheFire);
        }
        assert_eq!(use_campfire(&mut cf, Some(&mat(MaterialId::RawBeef)), &room), Err(ItemNote::FireFull));
        assert_eq!(use_campfire(&mut cf, None, &room), Err(ItemNote::NothingToTake), "nothing cooked yet");
        cf.slots[1] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: COOK_TICKS_PER_ITEM };
        assert_eq!(use_campfire(&mut cf, None, &full), Err(ItemNote::InventoryFull));
        let used = use_campfire(&mut cf, None, &room).unwrap();
        assert_eq!(used.effect, Effect::TookCooked);
        assert_eq!(used.gain.len(), 1);
        assert!(cf.slots[1].item.is_none());
        assert_eq!(
            use_campfire(&mut cf, Some(&shears()), &room),
            Err(ItemNote::NothingToTake),
            "a tool is neither fuel nor food"
        );
    }

    // ── Item frame ──

    #[test]
    fn a_frame_mounts_one_then_rotates() {
        let mut f = ItemFrameData::new();
        assert_eq!(use_item_frame(&mut f, None), Err(ItemNote::NothingToTake));
        let used = use_item_frame(&mut f, Some(&ItemStack::new_block(block::STONE, 9))).unwrap();
        assert_eq!((used.pay, used.effect), (1, Effect::Framed));
        assert_eq!(f.item.as_ref().unwrap().count, 1);
        let used = use_item_frame(&mut f, Some(&mat(MaterialId::Stick))).unwrap();
        assert_eq!((used.pay, used.effect), (0, Effect::Rotated));
        assert_eq!(f.rotation, 1);
    }

    // ── Hive ──

    #[test]
    fn a_bucket_scoops_and_shears_wear_and_an_empty_hive_says_so() {
        let mut h = HiveData { bees_inside: 0, honey_level: 2 };
        let used = use_hive(&mut h, Some(&ItemStack::new_material(MaterialId::Bucket, 1))).unwrap();
        assert_eq!((used.pay, used.wear, used.effect), (1, false, Effect::Scooped));
        assert_eq!(used.gain, vec![ItemStack::new_material(MaterialId::HoneyBottle, 1)]);
        let used = use_hive(&mut h, Some(&shears())).unwrap();
        assert_eq!((used.pay, used.wear, used.effect), (0, true, Effect::Sheared));
        assert_eq!(h.honey_level, 0);
        assert_eq!(use_hive(&mut h, Some(&shears())), Err(ItemNote::HiveEmpty));
        h.honey_level = 1;
        assert_eq!(use_hive(&mut h, Some(&mat(MaterialId::Stick))), Err(ItemNote::HiveNeedsTool));
        assert_eq!(h.honey_level, 1, "a refusal changes nothing");
    }

    // ── Dispatch, claims, breaks ──

    /// C3b-2-fix (L7) — an accepted use creates the state it needs (as the
    /// client's open always did); a refused one leaves nothing behind, so
    /// nothing is marked edited or streamed to joiners as a view.
    #[test]
    fn a_use_keeps_the_state_only_when_its_rule_accepts() {
        let mut w = World::new();
        w.track_edited_columns();
        let cell = (4, 70, 4);
        assert_eq!(use_block(&mut w, cell, UseKind::Composter, None, &room), Err(ItemNote::NothingToTake));
        assert!(w.composter_at(cell).is_none(), "NothingToTake: no composter left behind");
        assert_eq!(use_block(&mut w, (7, 70, 4), UseKind::Hive, None, &room), Err(ItemNote::HiveEmpty));
        assert!(w.hive_at((7, 70, 4)).is_none(), "HiveEmpty: no hive left behind");
        assert_eq!(use_block(&mut w, (5, 70, 4), UseKind::DryingRack, None, &room), Err(ItemNote::NotReady));
        assert!(!w.drying_racks.contains_key(&(5, 70, 4)), "NotReady: no rack left behind");
        assert_eq!(use_block(&mut w, (8, 70, 4), UseKind::Campfire, None, &room), Err(ItemNote::NothingToTake));
        assert!(w.campfire_at((8, 70, 4)).is_none(), "nothing cooked: no fire left behind");
        assert_eq!(use_block(&mut w, (6, 70, 4), UseKind::ItemFrame, None, &room), Err(ItemNote::NothingToTake));
        assert!(w.item_frame_at((6, 70, 4)).is_none(), "an empty hand on no frame: nothing left behind");
        assert!(w.take_edited_columns().is_empty(), "no refusal marked an edit");
        // Accepted: created and kept.
        assert!(use_block(&mut w, (5, 70, 4), UseKind::DryingRack, Some(&mat(MaterialId::GreenLog)), &room).is_ok());
        assert_eq!(w.drying_racks[&(5, 70, 4)].occupied_slots(), 1);
        assert!(use_block(&mut w, (6, 70, 4), UseKind::ItemFrame, Some(&mat(MaterialId::Stick)), &room).is_ok());
        assert!(w.item_frame_at((6, 70, 4)).is_some_and(|f| !f.is_empty()));
        assert!(use_block(&mut w, (8, 70, 4), UseKind::Campfire, Some(&mat(MaterialId::Coal)), &room).is_ok());
        assert_eq!(w.campfire_at((8, 70, 4)).unwrap().fuel_ticks, 240 * 20);
        assert!(use_block(&mut w, cell, UseKind::Composter, Some(&mat(MaterialId::Wheat)), &room).is_ok());
        assert_eq!(w.composter_at(cell).and_then(|c| c.input.as_ref()).map(|s| s.count), Some(1));
        // A full rack's refusal leaves the rack as it was.
        for _ in 1..RACK_SLOTS {
            use_block(&mut w, (5, 70, 4), UseKind::DryingRack, Some(&mat(MaterialId::GreenLog)), &room).unwrap();
        }
        let before = w.drying_racks[&(5, 70, 4)].clone();
        assert_eq!(
            use_block(&mut w, (5, 70, 4), UseKind::DryingRack, Some(&mat(MaterialId::GreenLog)), &room),
            Err(ItemNote::RackFull)
        );
        assert_eq!(w.drying_racks[&(5, 70, 4)].slots, before.slots);
    }

    /// C3b-2-fix (M2) — what `admit` refuses changes nothing: the frame
    /// stays empty, and the note is the admit's.
    #[test]
    fn an_admit_check_sees_the_pay_and_its_refusal_changes_nothing() {
        let mut w = World::new();
        let frame = (1, 70, 1);
        w.insert_item_frame(frame, ItemFrameData::new());
        let mut seen = Vec::new();
        let r = use_block_admitted(
            &mut w,
            frame,
            UseKind::ItemFrame,
            Some(&ItemStack::new_block(block::DIAMOND_BLOCK, 1)),
            &room,
            &mut |u| {
                seen.push(u.pay);
                Err(ItemNote::NothingToTake)
            },
        );
        assert_eq!(r, Err(ItemNote::NothingToTake));
        assert_eq!(seen, vec![1], "shown the pay before anything was kept");
        assert!(w.item_frame_at(frame).unwrap().is_empty(), "the frame took nothing");
        // A rule refusal never reaches `admit`.
        let r = use_block_admitted(&mut w, (2, 70, 1), UseKind::Hive, None, &room, &mut |_| panic!("not asked"));
        assert_eq!(r, Err(ItemNote::HiveEmpty));
        // Admitted: kept.
        let r = use_block_admitted(&mut w, frame, UseKind::ItemFrame, Some(&mat(MaterialId::Stick)), &room, &mut |_| Ok(()));
        assert!(r.is_ok());
        assert!(!w.item_frame_at(frame).unwrap().is_empty());
    }

    #[test]
    fn of_block_names_the_five_blocks_and_nothing_else() {
        assert_eq!(UseKind::of_block(block::CAMPFIRE_UNLIT), Some(UseKind::Campfire));
        assert_eq!(UseKind::of_block(block::CAMPFIRE), Some(UseKind::Campfire));
        assert_eq!(UseKind::of_block(block::BEE_HIVE), Some(UseKind::Hive));
        assert_eq!(UseKind::of_block(block::STONE), None);
        assert_eq!(UseKind::of_block(block::CHEST), None, "a chest is a shared container, not a use");
    }

    #[test]
    fn a_claim_covers_what_the_rule_could_take() {
        let bucket = Item::Material(MaterialId::Bucket);
        assert_eq!(claim(UseKind::Hive, Some(&bucket)), 1);
        assert_eq!(
            claim(UseKind::Hive, Some(&shears().item)),
            1,
            "shears wear: claimed, so two uses in flight on worn-out shears can't both pay (C3b-2-fix L5)"
        );
        assert_eq!(claim(UseKind::Hive, Some(&Item::Material(MaterialId::Stick))), 0, "nothing the hive takes");
        assert_eq!(claim(UseKind::Campfire, Some(&Item::Material(MaterialId::RawBeef))), 1);
        assert_eq!(claim(UseKind::Campfire, Some(&Item::Block(block::OAK_LOG))), 1);
        assert_eq!(claim(UseKind::Campfire, None), 0);
        assert_eq!(claim(UseKind::DryingRack, Some(&Item::Material(MaterialId::GreenLog))), 1);
        assert_eq!(claim(UseKind::DryingRack, Some(&Item::Material(MaterialId::Stick))), 0);
        assert_eq!(claim(UseKind::Composter, Some(&Item::Material(MaterialId::Wheat))), 1);
        assert_eq!(claim(UseKind::ItemFrame, Some(&Item::Block(block::STONE))), 1);
    }

    #[test]
    fn a_break_takes_out_what_the_block_held() {
        let mut w = World::new();
        let frame = (1, 70, 1);
        w.set_block(frame.0, frame.1, frame.2, block::ITEM_FRAME);
        use_block(&mut w, frame, UseKind::ItemFrame, Some(&ItemStack::new_block(block::STONE, 1)), &room).unwrap();
        assert_eq!(take_on_break(&mut w, frame, block::ITEM_FRAME, false), vec![ItemStack::new_block(block::STONE, 1)]);
        assert!(w.item_frame_at(frame).is_none(), "the frame's state is gone");
        let rack = (2, 70, 1);
        use_block(&mut w, rack, UseKind::DryingRack, Some(&mat(MaterialId::GreenLog)), &room).unwrap();
        assert_eq!(take_on_break(&mut w, rack, block::DRYING_RACK, false).len(), 1);
        assert!(!w.drying_racks.contains_key(&rack));
        let bin = (3, 70, 1);
        use_block(&mut w, bin, UseKind::Composter, Some(&mat(MaterialId::Wheat)), &room).unwrap();
        assert_eq!(take_on_break(&mut w, bin, block::COMPOSTER, false), vec![ItemStack::new_material(MaterialId::Wheat, 1)]);
        assert!(w.composter_at(bin).is_none(), "the composter's state is gone");
        assert!(take_on_break(&mut w, (9, 9, 9), block::STONE, false).is_empty());
        // C3b-2-fix (L6) — creative: a rack's logs are discarded (single-
        // player's rule), a frame's item and a composter's contents still
        // spill, and every state goes.
        use_block(&mut w, rack, UseKind::DryingRack, Some(&mat(MaterialId::GreenLog)), &room).unwrap();
        assert!(take_on_break(&mut w, rack, block::DRYING_RACK, true).is_empty(), "creative discards rack logs");
        assert!(!w.drying_racks.contains_key(&rack));
        use_block(&mut w, bin, UseKind::Composter, Some(&mat(MaterialId::Wheat)), &room).unwrap();
        w.composter_at_mut(bin).unwrap().output = Some(ItemStack::new_material(MaterialId::Compost, 2));
        assert_eq!(take_on_break(&mut w, bin, block::COMPOSTER, true).len(), 2, "input and output, in every mode");
        let hive = (4, 70, 1);
        w.insert_hive(hive, HiveData { bees_inside: 1, honey_level: 2 });
        assert!(take_on_break(&mut w, hive, block::BEE_HIVE, true).is_empty());
        assert!(w.hive_at(hive).is_none(), "a hive's state goes too: the next hive here starts empty");
    }
}
