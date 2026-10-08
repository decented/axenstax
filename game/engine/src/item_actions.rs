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
//! - **Craft** (C2b, v74): retired in v75 (C3a-2a). The craft is the
//!   window's result click, mirrored as a window op (`window_ops`) by the
//!   same rule the client runs, the table's reach included
//!   (`window::ClickCtx::table_present`); an `ItemAction::Craft` is ignored
//!   and tallied.
//! - **Drop** (C2b, fire-and-forget): the claimed item, taken from the shadow
//!   (log-only), becomes a real ground item thrown from the server body
//!   ([`serve_drop`]), paced by a token bucket ([`DropBucket`]).
//!
//! - **UseBlock** (C3b-2, v79): a right-click on a composter, drying rack,
//!   campfire, item frame or bee hive. The rules are `block_use` (shared
//!   with single-player); the server applies them to its real block entity
//!   in `HostedServer::serve_block_use`, and refuses with the block-use
//!   notes below (`OutOfReach` … `FireFull`).
//!
//! Pure rules first (unit-tested here), then the server-side steps that
//! apply them to a [`ServerPlayer`]. `hosted_server` holds only the dispatch
//! and the shadow / outcome glue.

use glam::Vec3;

use crate::block::{self, BlockId};
use crate::combat::PlayerCombat;
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
///
/// The block on PLACING and PLANTING after a bite is counted in the same ticks
/// (C2b verify M2): the right-click chain's place and plant arms wait on
/// `PlayerSlot::biting` while `eat_cooldown` runs, so it is 0.8 s at 20 ticks
/// a second whatever the frame rate, below 20 fps included (it was 16 frames:
/// 1.07 s at 15 fps, 0.27 s at 60). C3a-fix-2 D-L1: nothing else waits on it,
/// so a door, chest, table, bed or mob answers right after a bite. A held
/// right-click with food in hand between bites is swallowed
/// (`health_sync::eat_click`), never passed on to plant, open or sleep.
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
    /// C3b-2 (v79) — a block use refused: the block is beyond the server
    /// body's reach.
    OutOfReach = 9,
    /// C3b-2 — a block use refused: the play mode or a plot the joiner
    /// doesn't own forbids touching it.
    NotHere = 10,
    /// C3b-2 — a block use refused: the cell holds no composter, drying
    /// rack, campfire, item frame or hive on the server.
    NotThatBlock = 11,
    /// C3b-2 — a block use did nothing: nothing to collect, nothing to put
    /// in, or an item it doesn't take. Silent, as single-player's click is.
    NothingToTake = 12,
    /// C3b-2 — a log for a drying rack with every slot taken.
    RackFull = 13,
    /// C3b-2 — an empty hand on a drying rack with nothing seasoned yet (the
    /// client adds the progress from its view of the rack).
    NotReady = 14,
    /// C3b-2 — a bucket or shears on a hive with no honey.
    HiveEmpty = 15,
    /// C3b-2 — anything but a bucket or shears on a hive with honey.
    HiveNeedsTool = 16,
    /// C3b-2 — raw food for a campfire with every cooking slot taken.
    FireFull = 17,
    /// C3b-2 — single-player only: what the use would give doesn't fit the
    /// inventory, so it stays (a seasoned log back on the rack, cooked food
    /// on the fire). The server never sends it: a joiner is given the whole
    /// stack and what doesn't fit comes back to the world as a ground item
    /// (`ItemAction::GrantUnfit`, the C2b-fix BRIDGE until C3d).
    InventoryFull = 18,
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
            9 => OutOfReach,
            10 => NotHere,
            11 => NotThatBlock,
            12 => NothingToTake,
            13 => RackFull,
            14 => NotReady,
            15 => HiveEmpty,
            16 => HiveNeedsTool,
            17 => FireFull,
            18 => InventoryFull,
            _ => None,
        }
    }

    /// The toast a refusal shows (UK English), if any.
    pub fn toast(self) -> Option<&'static str> {
        use ItemNote::*;
        match self {
            None | NotNow | TooSoon | NothingToTake => Option::None,
            NotHungry => Some("You're not hungry."),
            NotFood => Some("You can't eat that."),
            NotNight => Some("You can only sleep at night."),
            SleptTonight => Some("You've already slept tonight."),
            BedTooFar => Some("That bed is too far away."),
            NotABed => Some("There's no bed there."),
            OutOfReach => Some("That's too far away."),
            NotHere => Some("You can't use that here."),
            NotThatBlock => Some("There's nothing to use there."),
            RackFull => Some("Rack full"),
            NotReady => Some("Not ready yet"),
            HiveEmpty => Some("The hive has no honey yet — bees fill it over time."),
            HiveNeedsTool => Some("Use a Bucket (honey) or Shears (honeycomb) on the hive."),
            FireFull => Some("There's no room on the fire."),
            InventoryFull => Some("Inventory full"),
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

/// [`cell_in_reach`] with `slack` more blocks of reach (C3a-fix-2 B-L2: the
/// server's table verdict).
pub fn cell_in_reach_with(eye: Vec3, cell: [i32; 3], slack: f32) -> bool {
    if slack == 0.0 {
        return cell_in_reach(eye, cell);
    }
    let centre = Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5);
    let dist = ((centre - eye).length() - slack).max(0.0);
    crate::hosted_server::block_change_within_reach(dist * dist, 0, 0, true)
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

/// C3b-fix-b (A-M1) — the longest silence (server ticks) a catch-up is
/// credited for: three seconds of stall, as long as an honest hitch lasts.
pub const MAX_CREDITED_SILENCE_TICKS: u64 = 60;

/// The server's pacing of one joiner's Q-drops: a token bucket of
/// [`DROP_BUCKET_CAPACITY`], refilled one per [`DROP_INTERVAL_TICKS`]. A
/// drop that finds it empty waits in the client's inbound queue (with
/// everything sent after it) until a token is back; it is never refused or
/// dropped. Refilled lazily from the server's tick counter, and (C2b verify
/// M4) in CLIENT time too: while a client's backlog is being replayed, each
/// new `ClientInput` read is one client tick and credits a quarter of a token
/// ([`DropBucket::credit_client_input`]), so a catch-up after a stall isn't
/// slowed to real time.
///
/// C3a-fix-2 (D-M1) — the client-time credit is HONEST: an input earns it only
/// when its tick advances past the last one credited, and the total is capped
/// at the server ticks the client was actually silent before the backlog
/// arrived ([`DropBucket::note_inbound`]). A standing backlog of replayed
/// or stale inputs earns nothing, and a client that never goes silent earns
/// nothing: at most one quarter-token per silent tick, which is the honest
/// rate.
///
/// C3b-fix-b (A-M1) — and it is not banked: each new silence REPLACES the
/// last allowance (a client silent every other tick for an hour holds one
/// tick of credit, not 36,000), the silence is capped at
/// [`MAX_CREDITED_SILENCE_TICKS`], the ticks server time already refilled
/// during it are taken off (they were credited once by the clock), and a tick
/// that is not a catch-up forfeits what is left
/// ([`DropBucket::note_not_catching_up`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropBucket {
    tokens: u8,
    /// The tick the bucket last refilled at (or was last full at).
    since: u64,
    /// Client-time credit toward the next token, in quarter-tokens (one per
    /// new `ClientInput` read in a catch-up; [`DROP_INTERVAL_TICKS`] make a
    /// token).
    quarters: u8,
    /// The server tick this client last had anything waiting to be read at;
    /// `None` before the first.
    last_read: Option<u64>,
    /// Client ticks of credit still allowed: the server ticks this client was
    /// silent (nothing waiting) before its latest packets arrived, less the
    /// ticks server time already refilled, at most
    /// [`MAX_CREDITED_SILENCE_TICKS`]. Replaced at each tick with packets
    /// waiting ([`DropBucket::note_inbound`]), never summed.
    allowance: u64,
    /// The highest client input tick credited so far.
    last_credited: u64,
}

impl Default for DropBucket {
    fn default() -> Self {
        DropBucket {
            tokens: DROP_BUCKET_CAPACITY,
            since: 0,
            quarters: 0,
            last_read: None,
            allowance: 0,
            last_credited: 0,
        }
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

    /// D-M1 — note that this client has packets waiting at server tick `now`
    /// (once per tick, before they are read, whatever they are and however
    /// many the tick's budget reads): the ticks since the previous such tick
    /// were silence, and are what a later backlog may be credited for
    /// ([`Self::credit_client_input`]). A tick on which the client's own Drop
    /// holds the head of its queue still counts as waiting: that is the
    /// client's backlog, not silence.
    ///
    /// C3b-fix-b (A-M1) — the allowance is the silence just ended and no
    /// more: a new silence REPLACES the last allowance (never adds to it), is
    /// capped at [`MAX_CREDITED_SILENCE_TICKS`], and has the ticks the clock
    /// already refilled during it taken off (`level(now)` against
    /// `level(last)`, [`DROP_INTERVAL_TICKS`] a token): the same silent ticks
    /// were credited once by server time. Ticks that follow with no silence
    /// leave it alone, so a catch-up spends it over as many ticks as the
    /// backlog takes to read.
    pub fn note_inbound(&mut self, now: u64) {
        let Some(last) = self.last_read else {
            self.last_read = Some(now);
            return;
        };
        let silence = now.saturating_sub(last).saturating_sub(1).min(MAX_CREDITED_SILENCE_TICKS);
        if silence > 0 {
            let refilled = u64::from(self.level(now).0.saturating_sub(self.level(last).0)) * DROP_INTERVAL_TICKS;
            self.allowance = silence.saturating_sub(refilled);
        }
        self.last_read = Some(last.max(now));
    }

    /// C3b-fix-b (A-M1) — this client has packets waiting at this tick but is
    /// not catching up on a backlog (an ordinary tick's reading): whatever
    /// allowance is left is forfeit. The catch-up it was for is over.
    pub fn note_not_catching_up(&mut self) {
        self.allowance = 0;
    }

    /// C2b verify M4 — credit one `ClientInput` read during a catch-up: one
    /// client tick, `1 / DROP_INTERVAL_TICKS` of a token, counted in whole
    /// quarters, capacity [`DROP_BUCKET_CAPACITY`] as ever (a full bucket keeps
    /// no spare quarters). The server-time refill is unchanged and still
    /// counts; a client that sends only Drops sends no inputs and gets none of
    /// this.
    ///
    /// D-M1 — only an input whose tick (`input_tick`) advances past the last
    /// credited one earns it, and only while the client's silent ticks
    /// ([`Self::note_inbound`]) leave allowance: each credit spends one.
    pub fn credit_client_input(&mut self, now: u64, input_tick: u64) {
        let (tokens, since) = self.level(now);
        self.since = since;
        self.tokens = tokens;
        if input_tick <= self.last_credited || self.allowance == 0 {
            return;
        }
        self.last_credited = input_tick;
        self.allowance -= 1;
        if tokens >= DROP_BUCKET_CAPACITY {
            self.quarters = 0;
            return;
        }
        self.quarters += 1;
        if u64::from(self.quarters) >= DROP_INTERVAL_TICKS {
            self.quarters = 0;
            self.tokens += 1;
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

/// May this server player's drop or window op (C3a-2a) be mirrored: a
/// joiner's body in the world, DEAD OR ALIVE. The client acted before it heard of its death, and
/// the server has no reason to refuse what the shadow should have followed
/// (C2b verify L4: a Craft was refused `NotNow` while a Drop from the same
/// body spawned).
pub(crate) fn can_mirror(sp: &ServerPlayer) -> bool {
    sp.server_simulated && sp.is_in_world()
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
    if !can_mirror(sp) {
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
            ItemNote::OutOfReach,
            ItemNote::NotHere,
            ItemNote::NotThatBlock,
            ItemNote::NothingToTake,
            ItemNote::RackFull,
            ItemNote::NotReady,
            ItemNote::HiveEmpty,
            ItemNote::HiveNeedsTool,
            ItemNote::FireFull,
            ItemNote::InventoryFull,
        ] {
            assert_eq!(ItemNote::from_wire(n.to_wire()), n);
        }
        // C3b-2 — the block-use codes are appended after NotABed (8), in order.
        assert_eq!(ItemNote::OutOfReach.to_wire(), 9);
        assert_eq!(ItemNote::InventoryFull.to_wire(), 18);
        assert_eq!(ItemNote::NothingToTake.toast(), None, "silent, as single-player's click is");
        assert_eq!(ItemNote::RackFull.toast(), Some("Rack full"));
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

    fn stick() -> Item {
        Item::Material(MaterialId::Stick)
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

    /// A bucket whose client was silent for `silent` ticks before a backlog.
    fn after_silence(silent: u64) -> DropBucket {
        let mut b = DropBucket::default();
        b.note_inbound(0);
        b.note_inbound(silent + 1);
        b
    }

    /// C2b verify M4 — client-time credit: four inputs in a catch-up are one
    /// token, capacity holds, and nothing but inputs earns it.
    #[test]
    fn client_inputs_credit_the_bucket_a_quarter_token_each() {
        let mut b = after_silence(100);
        assert!(b.take(0) && b.take(0));
        assert!(!b.ready(0), "empty");
        for t in 1..=3 {
            b.credit_client_input(0, t);
        }
        assert!(!b.ready(0), "three quarters is not a token");
        b.credit_client_input(0, 4);
        assert!(b.take(0), "four inputs, one token");
        assert!(!b.take(0));
        // Capacity holds, and a full bucket keeps no spare quarters.
        let mut full = after_silence(1000);
        for t in 1..=100 {
            full.credit_client_input(0, t);
        }
        assert!(full.take(0) && full.take(0) && !full.take(0), "still capacity 2");
        for t in 101..=103 {
            full.credit_client_input(0, t);
        }
        assert!(!full.ready(0), "no hoarded quarters from the time it was full");
        // Server time still refills beside it.
        assert!(full.ready(DROP_INTERVAL_TICKS));
    }

    /// D-M1 — only an input whose tick advances past the last credited one
    /// earns credit: a replayed or stale input earns nothing.
    #[test]
    fn a_replayed_input_earns_no_client_time_credit() {
        let mut b = after_silence(1000);
        b.take(0);
        b.take(0);
        for _ in 0..100 {
            b.credit_client_input(0, 7);
        }
        assert!(!b.ready(0), "one quarter at most, however often tick 7 is replayed");
        b.credit_client_input(0, 5);
        assert!(!b.ready(0), "a tick behind the last credited earns nothing either");
        assert_eq!((b.quarters, b.last_credited), (1, 7));
    }

    /// D-M1 — the credit is capped at the ticks the client was silent: eight
    /// silent ticks are two tokens, however many fresh inputs follow; and a
    /// client that never goes silent earns nothing.
    #[test]
    fn the_credit_is_capped_at_the_ticks_the_client_was_silent() {
        let mut b = after_silence(8);
        b.take(0);
        b.take(0);
        for t in 1..=500 {
            b.credit_client_input(0, t);
        }
        assert!(b.take(0) && b.take(0) && !b.take(0), "eight quarters, two tokens");
        // Never silent: an input read every server tick, a standing backlog
        // of fresh ticks credited each time. Over 100 server ticks it earns
        // exactly what server time earns: one token per interval.
        let mut b = DropBucket::default();
        let mut taken = 0u32;
        let mut client_tick = 0;
        for now in 1..=100u64 {
            b.note_inbound(now);
            for _ in 0..16 {
                client_tick += 1;
                b.credit_client_input(now, client_tick);
            }
            while b.take(now) {
                taken += 1;
            }
        }
        assert!(taken <= 2 + 100 / DROP_INTERVAL_TICKS as u32, "honest rate only: {taken}");
    }

    /// A-M1 — silence is not banked. A client silent every other tick for an
    /// hour holds one tick of credit, not 36,000: when it then keeps a
    /// standing backlog of fresh inputs, it earns no more than the honest
    /// rate (a token per interval, the capacity to start).
    #[test]
    fn an_hour_of_every_other_tick_silence_banks_nothing() {
        let mut b = DropBucket::default();
        let mut client_tick = 0;
        for now in (2..=72_000u64).step_by(2) {
            // One packet a tick it is heard: not a catch-up, so no credit is
            // asked for, but each silence is noted.
            b.note_inbound(now);
            client_tick += 1;
        }
        let start = 72_000u64;
        let mut taken = 0u32;
        for now in start + 1..=start + 100 {
            b.note_inbound(now);
            for _ in 0..16 {
                client_tick += 1;
                b.credit_client_input(now, client_tick);
            }
            while b.take(now) {
                taken += 1;
            }
        }
        assert!(taken <= 2 + 100 / DROP_INTERVAL_TICKS as u32, "honest rate only, not a banked hour: {taken}");
    }

    /// A-M1 — the ticks server time already refilled during a silence are not
    /// credited a second time: eight silent ticks refilled two tokens by the
    /// clock, so the catch-up that follows adds nothing to them.
    #[test]
    fn the_clocks_refill_during_the_silence_is_not_credited_again() {
        let mut b = DropBucket::default();
        b.note_inbound(0);
        b.take(0);
        b.take(0);
        b.note_inbound(9);
        assert!(b.take(9) && b.take(9), "the clock refilled both");
        for t in 1..=100 {
            b.credit_client_input(9, t);
        }
        assert!(!b.ready(9), "and the catch-up adds none");
    }

    /// A-M1 — a tick with packets waiting that is not a catch-up forfeits
    /// what is left of the allowance; a catch-up that goes on over several
    /// ticks keeps it between them.
    #[test]
    fn an_allowance_survives_a_catch_up_and_not_the_end_of_one() {
        let mut b = after_silence(40);
        b.take(0);
        b.take(0);
        b.credit_client_input(0, 1);
        b.note_inbound(0); // the next tick of the same backlog: no new silence
        for t in 2..=8 {
            b.credit_client_input(0, t);
        }
        assert!(b.take(0), "eight quarters across two ticks: two tokens");
        let mut b = after_silence(40);
        b.take(0);
        b.take(0);
        b.note_not_catching_up();
        for t in 1..=100 {
            b.credit_client_input(0, t);
        }
        assert!(!b.ready(0), "forfeit when the catch-up is over");
    }

    /// A-M1 — a long silence is credited for at most
    /// [`MAX_CREDITED_SILENCE_TICKS`], less what the clock refilled.
    #[test]
    fn a_long_silence_is_credited_for_three_seconds_at_most() {
        let mut b = DropBucket::default();
        b.note_inbound(0);
        b.take(0);
        b.take(0);
        b.note_inbound(1001);
        // The clock has refilled both tokens; spend them, then catch up.
        let mut taken = 0u32;
        while b.take(1001) {
            taken += 1;
        }
        assert_eq!(taken, 2);
        for t in 1..=500 {
            b.credit_client_input(1001, t);
            while b.take(1001) {
                taken += 1;
            }
        }
        let credit_tokens = (MAX_CREDITED_SILENCE_TICKS / DROP_INTERVAL_TICKS) as u32;
        assert!(taken - 2 <= credit_tokens, "{} tokens of credit for a 1000-tick silence", taken - 2);
        assert!(taken - 2 > 0, "but a real stall is still credited");
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
