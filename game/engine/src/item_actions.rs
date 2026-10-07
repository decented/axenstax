//! A joiner's item actions — eating and sleeping — decided by the server
//! (C2a, protocol v73, Spec 04 §4.2f).
//!
//! The server runs every joiner's hunger (its metabolism, in
//! `GameServer::tick`), so what changes hunger or heals the body is the
//! server's too. A joined client sends an `ItemAction` instead of eating or
//! sleeping itself; the server judges it here and answers with an
//! `ItemActionOutcome`:
//! - **Eat**: the claimed food must be food, the body alive and hungry or
//!   hurt, and the server's eating cooldown (nearly) spent. Accepted, the food is taken
//!   from the server's shadow of the joiner's inventory (a shortfall is
//!   log-only, as for D2b's interactions) and the body is fed and healed by
//!   the single-player rule ([`eat`]); the client takes the food it claimed.
//! - **Sleep**: a bed in reach of the server body, at night by the server's
//!   clock ([`is_night`], the rule the client's bed uses), once a night
//!   ([`NightCalendar`]). Accepted, the server sets the spawn point it
//!   respawns the joiner at ([`bed_spawn`]) and heals the body to full;
//!   hunger is left alone. A joiner's sleep never skips the night: the clock
//!   is the host's (until D4).
//!
//! - **Craft** (C2b, fire-and-forget): the server mirrors the craft on its
//!   shadow of the joiner's inventory ([`judge_craft`], [`serve_craft`]): a
//!   known recipe from blocks and materials, a recipe bigger than 2×2 only at
//!   a crafting table in reach. One of each input is taken (owed; a
//!   shortfall is log-only) and the output added. Nothing is answered: the
//!   client's own craft stands.
//! - **Drop** (C2b, fire-and-forget): the claimed item, taken from the shadow
//!   (log-only), becomes a real ground item thrown from the server body
//!   ([`serve_drop`]), paced by a token bucket ([`DropBucket`]).
//!
//! Pure rules first (unit-tested here), then the server-side steps that
//! apply them to a [`ServerPlayer`]. `hosted_server` holds only the dispatch
//! and the shadow / outcome glue.

use glam::Vec3;

use crate::block::{self, BlockId};
use crate::combat::PlayerCombat;
use crate::crafting::CraftSlot;
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};
use crate::server::ServerPlayer;
use crate::world::World;

/// The cooldown after eating, in fixed 20 Hz ticks (0.8 s). Every path counts
/// it in ticks, never frames: single-player and a host's own slots arm
/// `PlayerSlot::eat_cooldown` and decrement it in the fixed-tick `tick()`, a
/// joined client does the same before it asks, and the server counts its own
/// (`ServerPlayer::eat_cooldown`, once per `GameServer::tick`) between two
/// eats of one joiner. C2a verify M1: the first cut counted the client's in
/// frames (`place_cooldown`) and the server's in ticks, so an honest joiner
/// at 60 fps was refused two bites in three.
pub const EAT_COOLDOWN_TICKS: u32 = 16;

/// How many ticks early the server accepts an eat: it takes one once its
/// cooldown is down to this, so two accepted eats are at least
/// `EAT_COOLDOWN_TICKS - EAT_JITTER_SLACK_TICKS` = 12 ticks apart. Why: a
/// client spaces its requests 16 *client* ticks apart, but the server sees
/// them with arrival skew (a hitch bunches packets; a stall delays one and
/// not the next), so two requests sent 16 ticks apart can arrive 13 ticks
/// apart. The cooldown is a rate limit only: an accepted eat takes its food
/// from the server's shadow of the inventory either way, so the slack buys no
/// free food, just a slightly faster bite than the client's own pace.
pub const EAT_JITTER_SLACK_TICKS: u32 = 4;

/// What the client is told about an item action (`ItemActionOutcome.note`).
/// Wire-stable codes, append only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemNote {
    None = 0,
    /// Eat refused: hunger and health are both full.
    NotHungry = 1,
    /// Eat refused: the claimed item isn't food.
    NotFood = 2,
    /// Eat refused: inside the server's eating cooldown. Silent (no toast):
    /// the client paces itself, so this only follows arrival skew.
    TooSoon = 3,
    /// Refused: the body is dead or not in the world.
    NotNow = 4,
    /// Sleep refused: it is day by the server's clock.
    NotNight = 5,
    /// Sleep refused: this joiner already slept this night.
    SleptTonight = 6,
    /// Sleep refused: the bed is out of the server body's reach.
    BedTooFar = 7,
    /// Sleep refused: the cell holds no bed on the server.
    NotABed = 8,
}

impl ItemNote {
    pub fn to_wire(self) -> u8 {
        self as u8
    }

    /// An unknown code (a newer server) reads as nothing.
    pub fn from_wire(code: u8) -> Self {
        use ItemNote::*;
        match code {
            1 => NotHungry,
            2 => NotFood,
            3 => TooSoon,
            4 => NotNow,
            5 => NotNight,
            6 => SleptTonight,
            7 => BedTooFar,
            8 => NotABed,
            _ => None,
        }
    }

    /// The toast a refusal shows (UK English), if any.
    pub fn toast(self) -> Option<&'static str> {
        use ItemNote::*;
        match self {
            None | NotNow | TooSoon => Option::None,
            NotHungry => Some("You're not hungry."),
            NotFood => Some("You can't eat that."),
            NotNight => Some("You can only sleep at night."),
            SleptTonight => Some("You've already slept tonight."),
            BedTooFar => Some("That bed is too far away."),
            NotABed => Some("There's no bed there."),
        }
    }
}

/// The toast an accepted sleep shows a joiner: the spawn point is set and
/// the body healed, but the night goes on (the clock is the host's).
pub const RESTED_TOAST: &str = "You feel rested. Spawn point set.";

// ─── Eating ─────────────────────────────────────────────────────────────────

/// May a body eat `held` now? `alive` is whether it is in the world and
/// alive; `cooldown_left` the ticks left of its eating cooldown, of which
/// [`EAT_JITTER_SLACK_TICKS`] are forgiven. Returns the food's value
/// (`Item::food_value`), or why not.
pub fn judge_eat(
    alive: bool,
    held: Option<&Item>,
    combat: &PlayerCombat,
    cooldown_left: u32,
) -> Result<f32, ItemNote> {
    if !alive {
        return Err(ItemNote::NotNow);
    }
    if cooldown_left > EAT_JITTER_SLACK_TICKS {
        return Err(ItemNote::TooSoon);
    }
    let Some(value) = held.and_then(Item::food_value) else {
        return Err(ItemNote::NotFood);
    };
    if combat.hunger >= combat.max_hunger && combat.health >= combat.max_health {
        return Err(ItemNote::NotHungry);
    }
    Ok(value)
}

/// What eating a food worth `value` does to a body: heals `value` HP and
/// feeds `value` hunger points (one number drives both), and poisons it for
/// `poison` ticks (`Item::eat_poison_ticks`, 0 for every food today). The
/// one rule: single-player's right-click and the server's accepted `Eat`.
pub fn eat(combat: &mut PlayerCombat, value: f32, poison: u32) {
    combat.heal(value);
    combat.feed(value as u8);
    if poison > 0 {
        combat.apply_poison(poison);
    }
}

// ─── Sleeping ───────────────────────────────────────────────────────────────

/// The sky brightness below which it is night (`camera::compute_sun`).
const NIGHT_BRIGHTNESS: f32 = 0.3;

/// Is it night — when a bed can be slept in — at `world_time` on `world`,
/// its time lock applied (day-locked: never; night-locked: always)? The one
/// rule both sides call: the client's bed and the server's `Sleep`.
pub fn is_night(world: &World, world_time: u32) -> bool {
    is_night_at(world.effective_world_time(world_time))
}

fn is_night_at(world_time: u32) -> bool {
    crate::camera::compute_sun(world_time).1 < NIGHT_BRIGHTNESS
}

/// Where a body put to bed at `bed` stands up: on top of it, centred — the
/// spawn point a sleep sets on both sides.
pub fn bed_spawn(bed: [i32; 3]) -> Vec3 {
    Vec3::new(bed[0] as f32 + 0.5, bed[1] as f32 + 1.0, bed[2] as f32 + 0.5)
}

/// The server's count of nights, for "once a night" (C2a). `world_time` is
/// cyclic and can jump (the host's own sleep sets it to morning, `/time`),
/// so a night is counted at its dusk: each tick the server shows this its
/// raw clock, and the clock passing from day into night starts the next
/// night. Raw, not time-locked: a night-locked world's beds are always
/// usable ([`is_night`]), and its raw cycle still counts nights, so a joiner
/// sleeps there once a cycle. Not saved: a restart starts at night 0, and so
/// do the players' marks (they are per connection).
#[derive(Clone, Copy, Debug, Default)]
pub struct NightCalendar {
    nights: u32,
    was_night: Option<bool>,
}

impl NightCalendar {
    /// The raw clock reads `world_time` this tick.
    pub fn observe(&mut self, world_time: u32) {
        let night = is_night_at(world_time);
        if self.was_night == Some(false) && night {
            self.nights = self.nights.wrapping_add(1);
        }
        self.was_night = Some(night);
    }

    /// The night it is (or, by day, the last one).
    pub fn tonight(&self) -> u32 {
        self.nights
    }
}

/// May a body sleep in the bed at a cell holding `block_there`? `alive`: in
/// the world and alive; `in_reach`: the bed is within the server body's
/// block reach; `night`: [`is_night`] by the server's clock; `tonight`: the
/// [`NightCalendar`]'s night; `slept`: the night this body last slept.
pub fn judge_sleep(
    alive: bool,
    block_there: BlockId,
    in_reach: bool,
    night: bool,
    tonight: u32,
    slept: Option<u32>,
) -> Result<(), ItemNote> {
    if !alive {
        return Err(ItemNote::NotNow);
    }
    if block_there != block::BED {
        return Err(ItemNote::NotABed);
    }
    if !in_reach {
        return Err(ItemNote::BedTooFar);
    }
    if !night {
        return Err(ItemNote::NotNight);
    }
    if slept == Some(tonight) {
        return Err(ItemNote::SleptTonight);
    }
    Ok(())
}

/// Is the bed at `bed` within the block reach of a joiner's server body (eye
/// at `eye`)? [`cell_in_reach`].
pub fn bed_in_reach(eye: Vec3, bed: [i32; 3]) -> bool {
    cell_in_reach(eye, bed)
}

/// Is the block at `cell` within the block reach of a joiner's server body
/// (eye at `eye`)? The reach a joiner's block edit gets, measured to the
/// cell's centre (the held item is the client's word, so no reach bonus).
/// One rule for a bed (C2a) and a crafting table (C2b).
pub fn cell_in_reach(eye: Vec3, cell: [i32; 3]) -> bool {
    let centre = Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5);
    crate::hosted_server::block_change_within_reach((centre - eye).length_squared(), 0, 0, true)
}

// ─── Crafting (C2b) ─────────────────────────────────────────────────────────

/// Why the server didn't mirror a joiner's craft. Not on the wire (a craft
/// is never answered): counted per reason in the joiner's
/// `PossessionTally`. The shadow is left as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CraftRefusal {
    /// The body is dead or not in the world.
    NotNow = 0,
    /// A cell holds something that is not empty, a block or a material.
    BadIngredient = 1,
    /// The grid matches no recipe.
    NoRecipe = 2,
    /// A recipe bigger than 2×2 with no crafting table named.
    NeedsTable = 3,
    /// The named cell holds no crafting table on the server.
    NotATable = 4,
    /// The crafting table is out of the server body's block reach.
    TableTooFar = 5,
}

impl CraftRefusal {
    /// Every reason, in tally order ([`Self::index`]).
    pub const ALL: [CraftRefusal; 6] = [
        CraftRefusal::NotNow,
        CraftRefusal::BadIngredient,
        CraftRefusal::NoRecipe,
        CraftRefusal::NeedsTable,
        CraftRefusal::NotATable,
        CraftRefusal::TableTooFar,
    ];

    /// The reason's slot in a per-reason tally.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The reason in the server log's summary line.
    pub fn label(self) -> &'static str {
        match self {
            CraftRefusal::NotNow => "dead or away",
            CraftRefusal::BadIngredient => "bad ingredient",
            CraftRefusal::NoRecipe => "no recipe",
            CraftRefusal::NeedsTable => "needs a table",
            CraftRefusal::NotATable => "not a table",
            CraftRefusal::TableTooFar => "table too far",
        }
    }
}

/// A craft the server accepts: one of each input to take, the output to give.
#[derive(Clone, Debug, PartialEq)]
pub struct CraftPlan {
    /// One per non-empty cell, row-major (two cells of planks = two planks).
    pub inputs: Vec<Item>,
    pub output: ItemStack,
}

/// The wire form of a crafting grid for `ItemAction::Craft`: row-major
/// `(item_kind, item_id)` pairs (`inventory::item_to_ref`), an empty cell
/// `(EMPTY, 0)`. What the client sends; [`decode_craft_grid`] reads it.
pub fn craft_grid_wire(grid: &[[Option<ItemStack>; 3]; 3]) -> [(u8, u16); 9] {
    let mut wire = [crate::protocol::ItemRef::Empty.to_wire(); 9];
    for (i, cell) in grid.iter().flatten().enumerate() {
        if let Some(stack) = cell {
            wire[i] = crate::inventory::item_to_ref(&stack.item).to_wire();
        }
    }
    wire
}

/// Decode a wire craft grid (`ItemAction::Craft`, row-major `(item_kind,
/// item_id)` pairs) to the matcher's cells, with the item each non-empty
/// cell holds. An empty pair is an empty cell; a block or material decodes
/// as `inventory::item_from_ref` does (a block id the registry doesn't know
/// is no item); anything else — a tool, an unknown kind — is refused.
fn decode_craft_grid(
    grid: &[(u8, u16); 9],
    registry: &crate::block::BlockRegistry,
) -> Result<([[CraftSlot; 3]; 3], Vec<Item>), CraftRefusal> {
    let mut cells = [[CraftSlot::Empty; 3]; 3];
    let mut inputs = Vec::new();
    for (i, &(kind, id)) in grid.iter().enumerate() {
        if kind == crate::protocol::item_kind::EMPTY {
            continue;
        }
        let item = crate::inventory::item_from_ref(kind, id, registry).ok_or(CraftRefusal::BadIngredient)?;
        cells[i / 3][i % 3] = CraftSlot::from_item(&item);
        inputs.push(item);
    }
    Ok((cells, inputs))
}

/// May a joiner's craft from `grid` be mirrored? `alive`: in the world and
/// alive; `table`: the crafting table the client's 3×3 grid was opened from
/// (`None` for the 2×2 player grid); `block_at`: the server's world;
/// `eye`: the server body's eye. In order: alive, every cell empty, a block
/// or a material, a recipe (`crafting::match_recipe`, the client's own
/// matcher), and — when the recipe's trimmed bounding box
/// (`crafting::grid_bounds`) is bigger than 2×2 — a crafting table at
/// `table` within block reach ([`cell_in_reach`], the bed's rule).
pub fn judge_craft(
    alive: bool,
    grid: &[(u8, u16); 9],
    table: Option<[i32; 3]>,
    registry: &crate::block::BlockRegistry,
    block_at: impl Fn([i32; 3]) -> BlockId,
    eye: Vec3,
) -> Result<CraftPlan, CraftRefusal> {
    if !alive {
        return Err(CraftRefusal::NotNow);
    }
    let (cells, inputs) = decode_craft_grid(grid, registry)?;
    let output = crate::crafting::match_recipe(&cells).ok_or(CraftRefusal::NoRecipe)?;
    let (min_r, max_r, min_c, max_c) = crate::crafting::grid_bounds(&cells);
    if max_r - min_r >= 2 || max_c - min_c >= 2 {
        let cell = table.ok_or(CraftRefusal::NeedsTable)?;
        if block_at(cell) != block::CRAFTING_TABLE {
            return Err(CraftRefusal::NotATable);
        }
        if !cell_in_reach(eye, cell) {
            return Err(CraftRefusal::TableTooFar);
        }
    }
    Ok(CraftPlan { inputs, output })
}

/// What mirroring a craft did to the shadow inventory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CraftApplied {
    /// The output given.
    pub output: Option<ItemStack>,
    /// Inputs the shadow couldn't pay (one each; a log-only mismatch).
    pub short: Vec<Item>,
    /// How many of the output didn't fit the shadow. Tallied, never
    /// spilled: the client already holds them.
    pub overflow: u8,
}

/// Mirror an accepted craft on `inv`: one of each input taken by the owed
/// rule (`joiner_actions::take_owed`, from wherever it is), then the output
/// added.
pub fn apply_craft(inv: &mut Inventory, plan: &CraftPlan) -> CraftApplied {
    let short = plan
        .inputs
        .iter()
        .filter(|item| crate::joiner_actions::take_owed(inv, 0, item, 1) == 0)
        .cloned()
        .collect();
    let overflow = inv.add_item(plan.output.clone()).map_or(0, |rest| rest.count);
    CraftApplied { output: Some(plan.output.clone()), short, overflow }
}

// ─── Dropping (C2b) ─────────────────────────────────────────────────────────

/// The fewest ticks between two Q-drops on a joined client, and the
/// interval the server's [`DropBucket`] refills a token at. Q is
/// edge-triggered (one drop per press, never a key repeat), so before C2b a
/// client had no drop interval at all.
pub const DROP_INTERVAL_TICKS: u64 = 4;

/// The drops a [`DropBucket`] holds when full: two drops a client sent
/// [`DROP_INTERVAL_TICKS`] apart can arrive in one tick after jitter.
pub const DROP_BUCKET_CAPACITY: u8 = 2;

/// The server's pacing of one joiner's Q-drops: a token bucket of
/// [`DROP_BUCKET_CAPACITY`], refilled one per [`DROP_INTERVAL_TICKS`]. A
/// drop that finds it empty waits in the client's inbound queue (with
/// everything sent after it) until a token is back; it is never refused or
/// dropped. Refilled lazily from the server's tick counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropBucket {
    tokens: u8,
    /// The tick the bucket last refilled at (or was last full at).
    since: u64,
}

impl Default for DropBucket {
    fn default() -> Self {
        DropBucket { tokens: DROP_BUCKET_CAPACITY, since: 0 }
    }
}

impl DropBucket {
    /// The tokens it holds at tick `now`, and the tick they were counted to.
    fn level(&self, now: u64) -> (u8, u64) {
        let refills = now.saturating_sub(self.since) / DROP_INTERVAL_TICKS;
        let tokens = u64::from(self.tokens).saturating_add(refills);
        if tokens >= u64::from(DROP_BUCKET_CAPACITY) {
            (DROP_BUCKET_CAPACITY, now)
        } else {
            (tokens as u8, self.since + refills * DROP_INTERVAL_TICKS)
        }
    }

    /// May a drop go at tick `now`?
    pub fn ready(&self, now: u64) -> bool {
        self.level(now).0 > 0
    }

    /// Spend a token at tick `now`; `false` (and nothing spent) when empty.
    pub fn take(&mut self, now: u64) -> bool {
        let (tokens, since) = self.level(now);
        self.since = since;
        self.tokens = tokens;
        if tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

// ─── The server's steps ─────────────────────────────────────────────────────

/// May this server player act at all: a joiner's body, in the world, alive.
fn can_act(sp: &ServerPlayer) -> bool {
    sp.server_simulated && sp.is_present_and_alive()
}

/// The server's `Eat` for joiner `sp`, claiming `held`: judged, and if
/// accepted the body is fed and healed ([`eat`]) and the eating cooldown
/// starts. The caller takes the food from the shadow inventory (`Ok`) and
/// answers the client either way.
pub fn serve_eat(sp: &mut ServerPlayer, held: Option<&Item>) -> Result<(), ItemNote> {
    let value = judge_eat(can_act(sp), held, &sp.combat, sp.eat_cooldown)?;
    let poison = held.map_or(0, Item::eat_poison_ticks);
    eat(&mut sp.combat, value, poison);
    sp.eat_cooldown = EAT_COOLDOWN_TICKS;
    Ok(())
}

/// The server's `Sleep` for joiner `sp` in the bed at `bed`, on `world` at
/// `world_time` on night `tonight`: judged, and if accepted the spawn point
/// is the bed's (a later Respawn stands the body on it), the body is healed
/// to full, hunger left alone, and the night is marked slept.
pub fn serve_sleep(
    sp: &mut ServerPlayer,
    world: &World,
    world_time: u32,
    tonight: u32,
    bed: [i32; 3],
) -> Result<(), ItemNote> {
    judge_sleep(
        can_act(sp),
        world.get_block(bed[0], bed[1], bed[2]),
        bed_in_reach(sp.player.eye_pos(), bed),
        is_night(world, world_time),
        tonight,
        sp.slept_night,
    )?;
    sp.spawn_pos = bed_spawn(bed);
    let max = sp.combat.max_health;
    sp.combat.heal(max);
    sp.slept_night = Some(tonight);
    Ok(())
}

/// The server's `Craft` for joiner `sp` from `grid` (`table`: where its 3×3
/// grid was opened), on `world`: judged ([`judge_craft`]) and, if accepted,
/// mirrored on the shadow inventory ([`apply_craft`]). A refusal leaves the
/// shadow as it was.
pub fn serve_craft(
    sp: &mut ServerPlayer,
    world: &World,
    registry: &crate::block::BlockRegistry,
    grid: &[(u8, u16); 9],
    table: Option<[i32; 3]>,
) -> Result<CraftApplied, CraftRefusal> {
    let plan = judge_craft(
        can_act(sp),
        grid,
        table,
        registry,
        |c| world.get_block(c[0], c[1], c[2]),
        sp.player.eye_pos(),
    )?;
    Ok(apply_craft(&mut sp.inventory, &plan))
}

/// A joiner's drop the server spawns ([`serve_drop`]).
#[derive(Clone, Debug, PartialEq)]
pub struct DropServed {
    /// One of the claimed item (a tool whole), full fidelity.
    pub stack: ItemStack,
    /// The server body's eye and look, where the throw starts
    /// (`entity::q_drop_launch`).
    pub eye: Vec3,
    pub forward: Vec3,
    /// The shadow held one to take (else a log-only mismatch).
    pub paid: bool,
}

/// The server's `Drop` for joiner `sp` of `held` from hotbar slot
/// `hotbar_slot`: one taken from the shadow by the owed rule (a shortfall is
/// log-only: the item still spawns, as a placement still lands, until C3),
/// thrown from the server body. `None` — nothing spawns — for a body not in
/// the world or a claim that names no item (or a Plan, which has no wire
/// form). A dead body still drops: the client took the item from its hand
/// before it heard of the death, and the item is not lost.
pub fn serve_drop(sp: &mut ServerPlayer, hotbar_slot: usize, held: Option<Item>) -> Option<DropServed> {
    if !(sp.server_simulated && sp.is_in_world()) {
        return None;
    }
    let held = held.filter(|item| !matches!(item, Item::Plan(_)))?;
    let paid = crate::joiner_actions::take_owed(&mut sp.inventory, hotbar_slot, &held, 1) == 1;
    Some(DropServed {
        stack: ItemStack { item: held, count: 1 },
        eye: sp.player.eye_pos(),
        forward: crate::camera::forward_from(sp.yaw, sp.pitch),
        paid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::MaterialId;

    fn bread() -> Item {
        Item::Material(MaterialId::Bread)
    }

    fn hungry() -> PlayerCombat {
        let mut c = PlayerCombat::new();
        c.hunger = 10;
        c
    }

    #[test]
    fn a_hungry_living_body_may_eat_food() {
        let value = judge_eat(true, Some(&bread()), &hungry(), 0).unwrap();
        assert_eq!(Some(value), bread().food_value());
    }

    #[test]
    fn a_hurt_but_full_body_may_eat() {
        let mut c = PlayerCombat::new();
        c.health = 15.0;
        assert!(judge_eat(true, Some(&bread()), &c, 0).is_ok());
    }

    #[test]
    fn eating_is_refused_full_dead_too_soon_or_not_food() {
        assert_eq!(judge_eat(true, Some(&bread()), &PlayerCombat::new(), 0), Err(ItemNote::NotHungry));
        assert_eq!(judge_eat(false, Some(&bread()), &hungry(), 0), Err(ItemNote::NotNow));
        assert_eq!(
            judge_eat(true, Some(&bread()), &hungry(), EAT_JITTER_SLACK_TICKS + 1),
            Err(ItemNote::TooSoon)
        );
        assert_eq!(
            judge_eat(true, Some(&Item::Material(MaterialId::Stick)), &hungry(), 0),
            Err(ItemNote::NotFood)
        );
        assert_eq!(judge_eat(true, None, &hungry(), 0), Err(ItemNote::NotFood));
    }

    #[test]
    fn the_server_forgives_the_jitter_slack_of_the_cooldown() {
        let ok = |left| judge_eat(true, Some(&bread()), &hungry(), left).is_ok();
        assert!(ok(0));
        assert!(ok(EAT_JITTER_SLACK_TICKS), "at the slack: accepted");
        assert!(!ok(EAT_JITTER_SLACK_TICKS + 1), "one tick more: refused");
        assert!(!ok(EAT_COOLDOWN_TICKS));
    }

    #[test]
    fn a_too_soon_eat_shows_no_toast() {
        assert_eq!(ItemNote::TooSoon.toast(), None);
    }

    #[test]
    fn eating_heals_and_feeds_by_the_food_value() {
        let mut c = hungry();
        c.health = 10.0;
        eat(&mut c, 5.0, 0);
        assert_eq!(c.health, 15.0);
        assert_eq!(c.hunger, 15);
        assert!(!c.is_poisoned());
        eat(&mut c, 1.0, 40);
        assert!(c.is_poisoned());
    }

    #[test]
    fn notes_round_trip_and_unknown_codes_read_as_nothing() {
        for n in [
            ItemNote::None,
            ItemNote::NotHungry,
            ItemNote::NotFood,
            ItemNote::TooSoon,
            ItemNote::NotNow,
            ItemNote::NotNight,
            ItemNote::SleptTonight,
            ItemNote::BedTooFar,
            ItemNote::NotABed,
        ] {
            assert_eq!(ItemNote::from_wire(n.to_wire()), n);
        }
        assert_eq!(ItemNote::from_wire(200), ItemNote::None);
        assert_eq!(ItemNote::NotHungry.toast(), Some("You're not hungry."));
        assert_eq!(ItemNote::NotNight.toast(), Some("You can only sleep at night."));
        assert_eq!(ItemNote::SleptTonight.toast(), Some("You've already slept tonight."));
        assert_eq!(ItemNote::BedTooFar.toast(), Some("That bed is too far away."));
        assert_eq!(ItemNote::NotNow.toast(), None);
    }

    #[test]
    fn night_is_the_clients_brightness_rule_with_the_time_lock() {
        let mut w = World::new();
        assert!(is_night(&w, 0), "midnight");
        assert!(!is_night(&w, 12000), "noon");
        assert!(is_night(&w, 6000), "first light is still night");
        // The exact rule the client's bed has always used.
        for t in (0..24000).step_by(250) {
            assert_eq!(is_night(&w, t), crate::camera::compute_sun(t).1 < 0.3, "t={t}");
        }
        w.time_lock = "day".to_string();
        assert!(!is_night(&w, 0), "a day-locked world never sleeps");
        w.time_lock = "night".to_string();
        assert!(is_night(&w, 12000), "a night-locked world always may");
    }

    #[test]
    fn the_calendar_counts_a_night_at_each_dusk() {
        // `compute_sun`: 0 is midnight, 12000 noon; night is roughly
        // 17300..24000 and 0..6700.
        let mut cal = NightCalendar::default();
        cal.observe(12000);
        assert_eq!(cal.tonight(), 0);
        // Into the evening: a new night, the same one past midnight.
        cal.observe(20000);
        assert_eq!(cal.tonight(), 1);
        cal.observe(500);
        assert_eq!(cal.tonight(), 1, "past midnight is still the same night");
        // The host sleeps: the clock jumps to morning. The next dusk is the
        // next night.
        cal.observe(7000);
        assert_eq!(cal.tonight(), 1);
        cal.observe(19000);
        assert_eq!(cal.tonight(), 2);
        // A server started at night: that night is night 0.
        let mut at_night = NightCalendar::default();
        at_night.observe(0);
        assert_eq!(at_night.tonight(), 0);
    }

    #[test]
    fn sleeping_needs_a_bed_in_reach_at_night_once() {
        assert_eq!(judge_sleep(true, block::BED, true, true, 3, None), Ok(()));
        assert_eq!(judge_sleep(true, block::BED, true, true, 3, Some(2)), Ok(()));
        assert_eq!(judge_sleep(true, block::BED, true, true, 3, Some(3)), Err(ItemNote::SleptTonight));
        assert_eq!(judge_sleep(true, block::BED, true, false, 3, None), Err(ItemNote::NotNight));
        assert_eq!(judge_sleep(true, block::BED, false, true, 3, None), Err(ItemNote::BedTooFar));
        assert_eq!(judge_sleep(true, block::STONE, true, true, 3, None), Err(ItemNote::NotABed));
        assert_eq!(judge_sleep(false, block::BED, true, true, 3, None), Err(ItemNote::NotNow));
    }

    #[test]
    fn a_bed_is_in_reach_at_block_reach_and_not_beyond() {
        let eye = Vec3::new(0.5, 65.62, 0.5);
        assert!(bed_in_reach(eye, [2, 64, 0]));
        assert!(!bed_in_reach(eye, [12, 64, 0]));
    }

    #[test]
    fn the_bed_spawn_stands_on_top_of_the_bed() {
        assert_eq!(bed_spawn([3, 64, -2]), Vec3::new(3.5, 65.0, -1.5));
    }

    // ─── C2b: crafting ───────────────────────────────────────────────────

    use crate::crafting::{ToolMaterial, ToolType};
    use crate::protocol::item_kind;

    const EYE: Vec3 = Vec3::new(0.5, 65.62, 0.5);

    fn pair(item: &Item) -> (u8, u16) {
        crate::inventory::item_to_ref(item).to_wire()
    }

    fn grid_of(cells: &[(usize, Item)]) -> [(u8, u16); 9] {
        let mut g = [(item_kind::EMPTY, 0u16); 9];
        for (i, item) in cells {
            g[*i] = pair(item);
        }
        g
    }

    fn planks() -> Item {
        Item::Block(block::OAK_PLANKS)
    }

    fn stick() -> Item {
        Item::Material(MaterialId::Stick)
    }

    /// 4 planks in the top-left 2×2 → a crafting table.
    fn table_grid() -> [(u8, u16); 9] {
        grid_of(&[(0, planks()), (1, planks()), (3, planks()), (4, planks())])
    }

    /// 3 cobblestone over 2 sticks → a stone pickaxe (3×3: a table recipe).
    fn pickaxe_grid() -> [(u8, u16); 9] {
        let cobble = Item::Block(block::COBBLESTONE);
        grid_of(&[(0, cobble.clone()), (1, cobble.clone()), (2, cobble), (4, stick()), (7, stick())])
    }

    fn judge(grid: &[(u8, u16); 9], table: Option<[i32; 3]>, at: BlockId) -> Result<CraftPlan, CraftRefusal> {
        judge_craft(true, grid, table, &crate::block::BlockRegistry::new(), |_| at, EYE)
    }

    /// The client's grid encodes to what the server decodes: the same
    /// recipe either side.
    #[test]
    fn the_clients_grid_wire_form_is_what_the_server_judges() {
        let mut grid: [[Option<ItemStack>; 3]; 3] = Default::default();
        grid[0][0] = Some(ItemStack::new_block(block::COBBLESTONE, 5));
        grid[0][1] = Some(ItemStack::new_block(block::COBBLESTONE, 1));
        grid[0][2] = Some(ItemStack::new_block(block::COBBLESTONE, 2));
        grid[1][1] = Some(ItemStack::new_material(MaterialId::Stick, 9));
        grid[2][1] = Some(ItemStack::new_material(MaterialId::Stick, 1));
        let wire = craft_grid_wire(&grid);
        assert_eq!(wire, pickaxe_grid());
        assert_eq!(wire[3], (item_kind::EMPTY, 0));
    }

    #[test]
    fn a_2x2_recipe_needs_no_table_and_names_its_inputs_and_output() {
        let plan = judge(&table_grid(), None, block::AIR).unwrap();
        assert_eq!(plan.output, ItemStack::new_block(block::CRAFTING_TABLE, 1));
        assert_eq!(plan.inputs, vec![planks(); 4], "one per non-empty cell");
    }

    #[test]
    fn a_recipe_bigger_than_2x2_needs_a_crafting_table_in_reach() {
        let g = pickaxe_grid();
        assert_eq!(judge(&g, None, block::CRAFTING_TABLE), Err(CraftRefusal::NeedsTable));
        assert_eq!(judge(&g, Some([2, 64, 0]), block::STONE), Err(CraftRefusal::NotATable));
        assert_eq!(judge(&g, Some([12, 64, 0]), block::CRAFTING_TABLE), Err(CraftRefusal::TableTooFar));
        let plan = judge(&g, Some([2, 64, 0]), block::CRAFTING_TABLE).unwrap();
        match plan.output.item {
            Item::Tool(t) => assert_eq!((t.tool_type, t.material), (ToolType::Pickaxe, ToolMaterial::Stone)),
            other => panic!("expected a stone pickaxe, got {other:?}"),
        }
        assert_eq!(plan.inputs.len(), 5);
        // A 2×2 recipe laid out in a 3×3 table grid's far corner is still
        // 2×2: it needs no table.
        let corner = grid_of(&[(4, planks()), (5, planks()), (7, planks()), (8, planks())]);
        assert!(judge(&corner, None, block::AIR).is_ok());
    }

    #[test]
    fn a_craft_is_refused_dead_with_a_bad_ingredient_or_no_recipe() {
        let reg = crate::block::BlockRegistry::new();
        assert_eq!(
            judge_craft(false, &table_grid(), None, &reg, |_| block::AIR, EYE),
            Err(CraftRefusal::NotNow)
        );
        // A tool's pair (its tier) is no ingredient; nor is an unknown kind
        // or a block the registry doesn't know.
        let pick = Item::Tool(crate::crafting::Tool::new(ToolType::Pickaxe, ToolMaterial::Wood));
        let mut g = table_grid();
        g[8] = pair(&pick);
        assert_eq!(judge(&g, None, block::AIR), Err(CraftRefusal::BadIngredient));
        g[8] = (9, 1);
        assert_eq!(judge(&g, None, block::AIR), Err(CraftRefusal::BadIngredient));
        g[8] = (item_kind::BLOCK, u16::MAX);
        assert_eq!(judge(&g, None, block::AIR), Err(CraftRefusal::BadIngredient));
        // No recipe: two planks side by side and a lone stick.
        let nothing = grid_of(&[(0, planks()), (4, stick())]);
        assert_eq!(judge(&nothing, None, block::AIR), Err(CraftRefusal::NoRecipe));
        assert_eq!(judge(&grid_of(&[]), None, block::AIR), Err(CraftRefusal::NoRecipe));
    }

    fn tables(inv: &Inventory) -> u32 {
        inv.slots_iter()
            .flatten()
            .filter(|s| s.item == Item::Block(block::CRAFTING_TABLE))
            .map(|s| u32::from(s.count))
            .sum()
    }

    #[test]
    fn a_mirrored_craft_takes_one_per_cell_and_adds_the_output() {
        let plan = judge(&table_grid(), None, block::AIR).unwrap();
        let mut inv = Inventory::new();
        inv.set_slot(5, Some(ItemStack::new_block(block::OAK_PLANKS, 6)));
        let applied = apply_craft(&mut inv, &plan);
        assert!(applied.short.is_empty());
        assert_eq!(applied.overflow, 0);
        assert_eq!(inv.slot(5).unwrap().count, 2, "four planks taken");
        assert_eq!(tables(&inv), 1);
        // The shadow lacks two of them: two short (log-only), the output
        // still added.
        let applied = apply_craft(&mut inv, &plan);
        assert_eq!(applied.short, vec![planks(); 2]);
        assert_eq!(tables(&inv), 2);
    }

    #[test]
    fn a_crafted_output_that_does_not_fit_is_counted_not_spilled() {
        let plan = judge(&table_grid(), None, block::AIR).unwrap();
        let mut inv = Inventory::new();
        for i in 0..36 {
            inv.set_slot(i, Some(ItemStack::new_material(MaterialId::Bone, 64)));
        }
        let applied = apply_craft(&mut inv, &plan);
        assert_eq!(applied.overflow, 1);
        assert_eq!(applied.short.len(), 4);
    }

    #[test]
    fn a_crafted_tool_lands_at_full_durability() {
        let iron = Item::Material(MaterialId::IronIngot);
        let shears = grid_of(&[(0, iron.clone()), (3, iron)]);
        let plan = judge(&shears, None, block::AIR).unwrap();
        let mut inv = Inventory::new();
        apply_craft(&mut inv, &plan);
        let got = inv.slots_iter().flatten().find_map(|s| match &s.item {
            Item::Tool(t) => Some(*t),
            _ => None,
        });
        let fresh = crate::crafting::Tool::new(ToolType::Shears, ToolMaterial::Iron);
        assert_eq!(got.map(|t| (t.tool_type, t.durability)), Some((ToolType::Shears, fresh.durability)));
    }

    #[test]
    fn refusal_indices_and_labels_are_distinct() {
        for (i, r) in CraftRefusal::ALL.iter().enumerate() {
            assert_eq!(r.index(), i);
        }
        let labels: std::collections::HashSet<_> = CraftRefusal::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(labels.len(), CraftRefusal::ALL.len());
    }

    // ─── C2b: dropping ───────────────────────────────────────────────────

    #[test]
    fn the_drop_bucket_holds_two_and_refills_one_per_interval() {
        assert_eq!((DROP_INTERVAL_TICKS, DROP_BUCKET_CAPACITY), (4, 2));
        let mut b = DropBucket::default();
        assert!(b.take(1000) && b.take(1000), "two at once");
        assert!(!b.ready(1000) && !b.take(1000), "then empty");
        assert!(!b.ready(1003));
        assert!(b.ready(1004) && b.take(1004));
        assert!(!b.take(1007));
        assert!(b.take(1008));
        // Idle long enough, it is full again — never fuller.
        assert!(b.take(2000) && b.take(2000));
        assert!(!b.take(2000));
        // A partial refill keeps its remainder: one token at +4, the next
        // at +8 from the same start, not from the later take.
        let mut b = DropBucket::default();
        b.take(0);
        b.take(0);
        assert!(b.take(6), "one back at +4");
        assert!(b.take(8), "and the next at +8");
    }

    #[test]
    fn a_drop_is_thrown_from_the_body_and_paid_from_the_shadow_if_it_can_be() {
        let mut sp = ServerPlayer::new(Vec3::new(3.5, 70.0, 3.5));
        sp.server_simulated = true;
        sp.connected = true;
        sp.awaiting_join = false;
        sp.inventory.set_slot(2, Some(ItemStack::new_material(MaterialId::Stick, 3)));
        let served = serve_drop(&mut sp, 2, Some(stick())).unwrap();
        assert!(served.paid);
        assert_eq!(served.stack, ItemStack::new_material(MaterialId::Stick, 1));
        assert_eq!(served.eye, sp.player.eye_pos());
        assert_eq!(sp.inventory.slot(2).unwrap().count, 2);
        // What the shadow lacks still drops, unpaid.
        let bone = Item::Material(MaterialId::Bone);
        assert!(!serve_drop(&mut sp, 0, Some(bone)).unwrap().paid);
        // A dead body still drops; one not in the world, an empty claim or
        // a Plan doesn't.
        sp.combat.dead = true;
        assert!(serve_drop(&mut sp, 2, Some(stick())).is_some());
        assert!(serve_drop(&mut sp, 2, None).is_none());
        sp.awaiting_join = true;
        assert!(serve_drop(&mut sp, 2, Some(stick())).is_none());
    }
}
