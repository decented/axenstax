//! T1.5 animal-product pure helpers — egg laying + cow milking.
//!
//! Each function takes a `(last_action_tick, current_tick)` pair and
//! returns whether the next action is allowed. Live wiring (the mob
//! AI tick calling these, the right-click handler consuming a bucket)
//! is deferred to playtest. This module ships the cadence logic.
//!
//! Spec: `docs/foundations/2026-05-14-farming-tier-1.5-processed-economy.md`.

/// Chicken egg-lay interval (ticks @ 20 TPS).
///
/// 12000 ticks = 10 in-game minutes (game time runs 4× wall-clock by
/// default on alpha, so this is ~2.5 wall-minutes per egg).
/// Same shape as Minecraft chickens — eggs are frequent enough that
/// a small flock provides eggs steadily but not so fast that one
/// chicken floods your inventory.
pub const EGG_LAY_INTERVAL_TICKS: u64 = 12_000;

/// Cow milking cooldown (ticks). After milking a cow, you can't milk
/// it again for this many ticks. Stops a player draining a single cow
/// indefinitely.
///
/// 6000 ticks = 5 in-game minutes — half the egg interval. Cows take
/// longer to lactate again than chickens take to lay; this matches
/// the food-value gap (milk bucket = 6 HP, egg = 1 HP).
pub const COW_MILK_COOLDOWN_TICKS: u64 = 6_000;

/// Per-mob state tracking the last action tick. Lives on each chicken
/// / cow as a component-style field; pure-function helpers consume it
/// without touching the ECS directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnimalProductState {
    /// Tick of the last egg laid (chicken) or milking (cow). `None`
    /// means the animal has never produced — first action is immediate.
    pub last_action_tick: Option<u64>,
}

impl AnimalProductState {
    /// Freshly-spawned mob — has never produced.
    pub const fn new() -> Self {
        Self { last_action_tick: None }
    }
}

impl Default for AnimalProductState {
    fn default() -> Self {
        Self::new()
    }
}

/// Should the chicken lay an egg this tick?
///
/// True if `(current_tick - last_action_tick) >= EGG_LAY_INTERVAL_TICKS`
/// — or `last_action_tick == 0` (first egg is immediate). The mob AI
/// tick calls this once per chicken per tick; live wiring also spawns
/// the egg item at the chicken's feet and updates `last_action_tick`.
pub fn should_lay_egg(state: AnimalProductState, current_tick: u64) -> bool {
    match state.last_action_tick {
        None => true,
        Some(last) => current_tick.saturating_sub(last) >= EGG_LAY_INTERVAL_TICKS,
    }
}

/// Can the player milk the cow this tick?
///
/// True if the cooldown has elapsed. Cooldown starts on each
/// successful milking. The right-click handler calls this with the
/// cow's `AnimalProductState` and `current_tick`; if true, swap the
/// player's Bucket for a MilkBucket and update `last_action_tick`.
pub fn can_milk(state: AnimalProductState, current_tick: u64) -> bool {
    match state.last_action_tick {
        None => true,
        Some(last) => current_tick.saturating_sub(last) >= COW_MILK_COOLDOWN_TICKS,
    }
}

/// Record that an action happened. Pure mutator; sets
/// `last_action_tick` to `Some(current_tick)`. Caller mutates the
/// cow/chicken component after `should_lay_egg`/`can_milk` returns true.
pub fn record_action(state: &mut AnimalProductState, current_tick: u64) {
    state.last_action_tick = Some(current_tick);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_chicken_lays_immediately() {
        let state = AnimalProductState::new();
        assert!(should_lay_egg(state, 0));
        assert!(should_lay_egg(state, 5000));
    }

    #[test]
    fn fresh_cow_can_be_milked_immediately() {
        let state = AnimalProductState::new();
        assert!(can_milk(state, 0));
    }

    #[test]
    fn chicken_waits_for_full_interval() {
        let mut state = AnimalProductState::new();
        record_action(&mut state, 1000);
        // Just-after = no
        assert!(!should_lay_egg(state, 1001));
        // Half-interval = no
        assert!(!should_lay_egg(state, 1000 + EGG_LAY_INTERVAL_TICKS / 2));
        // Boundary - 1 = no
        assert!(!should_lay_egg(state, 1000 + EGG_LAY_INTERVAL_TICKS - 1));
        // Exact boundary = yes
        assert!(should_lay_egg(state, 1000 + EGG_LAY_INTERVAL_TICKS));
        // Past boundary = yes
        assert!(should_lay_egg(state, 1000 + EGG_LAY_INTERVAL_TICKS + 500));
    }

    #[test]
    fn cow_waits_for_full_cooldown() {
        let mut state = AnimalProductState::new();
        record_action(&mut state, 1000);
        assert!(!can_milk(state, 1001));
        assert!(!can_milk(state, 1000 + COW_MILK_COOLDOWN_TICKS - 1));
        assert!(can_milk(state, 1000 + COW_MILK_COOLDOWN_TICKS));
    }

    #[test]
    fn record_action_updates_tick() {
        let mut state = AnimalProductState::new();
        record_action(&mut state, 12345);
        assert_eq!(state.last_action_tick, Some(12345));
        record_action(&mut state, 99999);
        assert_eq!(state.last_action_tick, Some(99999));
    }

    #[test]
    fn chicken_lays_repeatedly_after_each_interval() {
        let mut state = AnimalProductState::new();
        // Tick 0 — fresh, lays.
        assert!(should_lay_egg(state, 0));
        record_action(&mut state, 0);
        // Wait until next interval — should lay again.
        let next = EGG_LAY_INTERVAL_TICKS;
        assert!(!should_lay_egg(state, next - 1));
        assert!(should_lay_egg(state, next));
        record_action(&mut state, next);
        // Third egg.
        let third = next * 2;
        assert!(should_lay_egg(state, third));
    }

    #[test]
    fn cow_cooldown_is_shorter_than_chicken_interval() {
        // Spec invariant: cow milks more frequently than chickens lay.
        // Cows take longer between products IRL but the in-game ratio
        // is calibrated so milk feels like a steady resource for the
        // active player while eggs accumulate quietly in the henhouse.
        assert!(COW_MILK_COOLDOWN_TICKS < EGG_LAY_INTERVAL_TICKS);
    }

    #[test]
    fn saturating_sub_means_no_underflow_panic() {
        // If current_tick < last_action_tick (clock skew, save/load),
        // the helper must not panic. saturating_sub returns 0, which
        // is < interval, so the result is "not ready" — sensible.
        let mut state = AnimalProductState::new();
        record_action(&mut state, 99999);
        assert!(!should_lay_egg(state, 100)); // current_tick way behind
        assert!(!can_milk(state, 100));
    }

    #[test]
    fn default_impl_matches_new() {
        let a = AnimalProductState::new();
        let b: AnimalProductState = Default::default();
        assert_eq!(a, b);
    }
}
