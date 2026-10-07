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
//! Pure rules first (unit-tested here), then the two server-side steps that
//! apply them to a [`ServerPlayer`]. `hosted_server` holds only the dispatch
//! and the shadow / outcome glue.

use glam::Vec3;

use crate::block::{self, BlockId};
use crate::combat::PlayerCombat;
use crate::item::Item;
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
/// at `eye`)? The reach a joiner's block edit gets, measured to the cell's
/// centre (the held item is the client's word, so no reach bonus).
pub fn bed_in_reach(eye: Vec3, bed: [i32; 3]) -> bool {
    let centre = Vec3::new(bed[0] as f32 + 0.5, bed[1] as f32 + 0.5, bed[2] as f32 + 0.5);
    crate::hosted_server::block_change_within_reach((centre - eye).length_squared(), 0, 0, true)
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
}
