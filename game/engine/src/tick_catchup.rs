//! Fixed-rate tick catch-up policy (Spec 01 tick loop, as-built).
//!
//! The frame loop adds elapsed wall time to an accumulator and runs one 20 Hz
//! tick per `TICK_DURATION` it holds. Left unclamped, a tick that costs more
//! than 50 ms (a weak CPU, WASM) lets the accumulator grow without bound: each
//! frame then runs the per-frame maximum, takes ten times as long, and the
//! backlog never drains (the "spiral of death": slow motion, frozen UI).
//!
//! Two pure rules stop that, both **only while no remote joiner is connected**
//! (`hosting_joiners` below):
//!
//! * [`clamp_backlog`] — the accumulator never holds more than
//!   [`MAX_BACKLOG_TICKS`] ticks of debt; the excess is dropped (the world
//!   simply runs slower than real time until the machine keeps up).
//! * [`may_run_another_tick`] — a frame stops catching up once it has spent
//!   [`MAX_TICK_TIME_PER_FRAME`] on ticks (it always runs at least one), so a
//!   slow tick costs frame rate, not a multi-hundred-millisecond stall.
//!
//! Nothing else reads wall time for the simulation: `tick_counter` and every
//! "ticks since" counter advance per executed tick, so dropped time is
//! invisible to them.
//!
//! **Joiners are the exception.** A hosted server ticks once per client tick,
//! but a joiner's inputs keep arriving at its own steady 20 Hz and are consumed
//! one per server tick. Dropping host ticks after a stall would leave the
//! unconsumed inputs queued for good (standing latency, then dropped movement
//! and rubber-banding), so while a hosted server has a remote client the old
//! behaviour applies exactly: no backlog cap, no per-frame time guard, up to
//! [`MAX_TICKS_PER_FRAME`] ticks a frame to repay the debt.

use std::cell::RefCell;
use web_time::{Duration, Instant};

/// Most ticks of debt the accumulator may carry between frames (250 ms).
pub const MAX_BACKLOG_TICKS: u32 = 5;
/// Most ticks one frame runs, whatever the backlog (pre-existing limit).
pub const MAX_TICKS_PER_FRAME: u32 = 10;
/// A frame stops starting new catch-up ticks after this much tick time.
pub const MAX_TICK_TIME_PER_FRAME: Duration = Duration::from_millis(100);
/// At most one "dropped ticks" log line per this interval.
const LOG_INTERVAL: Duration = Duration::from_secs(10);

/// Cap `accumulator` at [`MAX_BACKLOG_TICKS`] of `tick`; returns the dropped
/// excess (zero when nothing was dropped). With `hosting_joiners` nothing is
/// ever dropped: the joiners' queued inputs need every tick repaid.
pub fn clamp_backlog(accumulator: &mut Duration, tick: Duration, hosting_joiners: bool) -> Duration {
    let cap = tick * MAX_BACKLOG_TICKS;
    if !hosting_joiners && *accumulator > cap {
        let dropped = *accumulator - cap;
        *accumulator = cap;
        dropped
    } else {
        Duration::ZERO
    }
}

/// May the frame start another catch-up tick? `spent` is the wall time this
/// frame's ticks have taken so far. Without joiners a frame stops once it has
/// spent [`MAX_TICK_TIME_PER_FRAME`] (after at least one tick); with
/// `hosting_joiners` only [`MAX_TICKS_PER_FRAME`] bounds it (the pre-clamp
/// rule).
pub fn may_run_another_tick(
    accumulator: Duration,
    tick: Duration,
    ticks_run: u32,
    spent: Duration,
    hosting_joiners: bool,
) -> bool {
    accumulator >= tick
        && ticks_run < MAX_TICKS_PER_FRAME
        && (hosting_joiners || ticks_run == 0 || spent < MAX_TICK_TIME_PER_FRAME)
}

/// Rolling record of what was dropped, for the rate-limited log line.
#[derive(Default)]
struct DropLog {
    dropped: Duration,
    events: u32,
    last_logged: Option<Instant>,
}

thread_local! {
    static DROP_LOG: RefCell<DropLog> = RefCell::new(DropLog::default());
}

/// Record a drop and emit at most one summary log line per [`LOG_INTERVAL`].
pub fn note_dropped(dropped: Duration, now: Instant) {
    if dropped.is_zero() {
        return;
    }
    DROP_LOG.with(|log| {
        let mut log = log.borrow_mut();
        log.dropped += dropped;
        log.events += 1;
        let due = log.last_logged.is_none_or(|t| now.duration_since(t) >= LOG_INTERVAL);
        if due {
            log::warn!(
                "tick backlog clamped: dropped {} ms of simulation time over {} frame(s) \
                 (a stall, or a machine that cannot sustain 20 ticks/s)",
                log.dropped.as_millis(),
                log.events
            );
            log.dropped = Duration::ZERO;
            log.events = 0;
            log.last_logged = Some(now);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(50);

    #[test]
    fn a_small_backlog_is_kept_whole() {
        let mut acc = TICK * 3 + Duration::from_millis(20);
        assert_eq!(clamp_backlog(&mut acc, TICK, false), Duration::ZERO);
        assert_eq!(acc, TICK * 3 + Duration::from_millis(20));
    }

    #[test]
    fn a_big_backlog_is_cut_to_the_cap_and_the_excess_reported() {
        let mut acc = Duration::from_secs(3);
        let dropped = clamp_backlog(&mut acc, TICK, false);
        assert_eq!(acc, TICK * MAX_BACKLOG_TICKS);
        assert_eq!(dropped, Duration::from_secs(3) - TICK * MAX_BACKLOG_TICKS);
    }

    #[test]
    fn a_frame_always_runs_one_tick_but_stops_catching_up_when_over_time() {
        assert!(may_run_another_tick(TICK, TICK, 0, Duration::from_secs(5), false));
        assert!(!may_run_another_tick(TICK * 4, TICK, 1, MAX_TICK_TIME_PER_FRAME, false));
        assert!(may_run_another_tick(TICK * 4, TICK, 1, Duration::from_millis(10), false));
        assert!(!may_run_another_tick(Duration::from_millis(49), TICK, 0, Duration::ZERO, false));
        assert!(!may_run_another_tick(TICK * 20, TICK, MAX_TICKS_PER_FRAME, Duration::ZERO, false));
    }

    /// Simulate the frame loop for `frames` frames. Every tick costs
    /// `tick_cost`; the non-tick part of a frame costs `render_cost`. Returns
    /// (largest backlog seen at a frame start, longest frame, total ticks).
    fn simulate(
        frames: u32,
        tick_cost: Duration,
        render_cost: Duration,
        clamped: bool,
    ) -> (Duration, Duration, u32) {
        let mut acc = Duration::ZERO;
        let (mut max_backlog, mut max_frame, mut total) = (Duration::ZERO, Duration::ZERO, 0u32);
        let mut last_frame = Duration::from_millis(16);
        for _ in 0..frames {
            acc += last_frame;
            if clamped {
                clamp_backlog(&mut acc, TICK, false);
            }
            max_backlog = max_backlog.max(acc);
            let (mut ran, mut spent) = (0u32, Duration::ZERO);
            loop {
                let go = if clamped {
                    may_run_another_tick(acc, TICK, ran, spent, false)
                } else {
                    acc >= TICK && ran < MAX_TICKS_PER_FRAME
                };
                if !go {
                    break;
                }
                spent += tick_cost;
                acc = acc.saturating_sub(TICK);
                ran += 1;
            }
            total += ran;
            last_frame = spent + render_cost;
            max_frame = max_frame.max(last_frame);
        }
        (max_backlog, max_frame, total)
    }

    #[test]
    fn a_slow_tick_spirals_without_the_clamp_and_stays_bounded_with_it() {
        // 80 ms per tick (a weak CPU), 10 ms of everything else.
        let tick_cost = Duration::from_millis(80);
        let render = Duration::from_millis(10);
        let (open_backlog, open_frame, _) = simulate(200, tick_cost, render, false);
        assert!(open_backlog > Duration::from_secs(10), "unclamped debt grows: {open_backlog:?}");
        assert!(open_frame >= Duration::from_millis(800), "and frames balloon: {open_frame:?}");

        let (backlog, frame, total) = simulate(200, tick_cost, render, true);
        assert!(backlog <= TICK * MAX_BACKLOG_TICKS, "backlog bounded: {backlog:?}");
        // One tick over the 100 ms budget, plus the render part.
        assert!(frame <= MAX_TICK_TIME_PER_FRAME + tick_cost + render, "frame bounded: {frame:?}");
        assert!(total >= 200, "the simulation still advances every frame: {total}");
    }

    #[test]
    fn a_fast_machine_is_untouched() {
        // 2 ms ticks: the clamp never fires and every tick of real time is run.
        let (backlog, _, total) = simulate(300, Duration::from_millis(2), Duration::from_millis(14), true);
        assert!(backlog < TICK * 2);
        // 300 frames of ~16 ms is 4.8 s of wall time = ~96 ticks (the last
        // frame's time is still owed when the run ends).
        assert!((84..=100).contains(&total), "{total}");
    }

    #[test]
    fn a_long_stall_drops_instead_of_replaying() {
        // A 30 s freeze (backgrounded tab / long load): next frame runs a few
        // ticks, not 600.
        let mut acc = Duration::from_secs(30);
        let dropped = clamp_backlog(&mut acc, TICK, false);
        assert!(dropped > Duration::from_secs(29));
        let mut ran = 0;
        while may_run_another_tick(acc, TICK, ran, Duration::ZERO, false) {
            acc = acc.saturating_sub(TICK);
            ran += 1;
        }
        assert_eq!(ran, MAX_BACKLOG_TICKS);
    }

    #[test]
    fn hosting_joiners_keeps_the_old_catch_up_exactly() {
        // Table: (accumulator, ticks_run, spent) -> may run another tick, for
        // the two branches. With joiners only the accumulator and the
        // 10-per-frame limit matter, as before the clamp.
        let ms = Duration::from_millis;
        let table = [
            // acc,        run, spent,        alone, with joiners
            (TICK,         0, ms(5_000),      true,  true),
            (TICK * 4,     1, ms(10),         true,  true),
            (TICK * 4,     1, ms(100),        false, true),   // time guard off with joiners
            (TICK * 4,     3, ms(400),        false, true),
            (TICK * 20,    9, ms(900),        false, true),
            (TICK * 20,   10, ms(0),          false, false),  // per-frame limit stays
            (ms(49),       0, ms(0),          false, false),
        ];
        for (acc, run, spent, alone, joiners) in table {
            assert_eq!(may_run_another_tick(acc, TICK, run, spent, false), alone, "alone {acc:?} {run} {spent:?}");
            assert_eq!(may_run_another_tick(acc, TICK, run, spent, true), joiners, "joiners {acc:?} {run} {spent:?}");
        }
    }

    #[test]
    fn hosting_joiners_never_drops_a_stall() {
        // A 400 ms host stall: alone, the backlog is cut to 250 ms (5 ticks);
        // with a joiner connected it is kept whole and fully repaid.
        let mut alone = Duration::from_millis(400);
        assert_eq!(clamp_backlog(&mut alone, TICK, false), Duration::from_millis(150));
        assert_eq!(alone, TICK * 5);

        let mut with = Duration::from_millis(400);
        assert_eq!(clamp_backlog(&mut with, TICK, true), Duration::ZERO);
        assert_eq!(with, Duration::from_millis(400));
        let mut ran = 0;
        while may_run_another_tick(with, TICK, ran, Duration::from_millis(200), true) {
            with = with.saturating_sub(TICK);
            ran += 1;
        }
        assert_eq!(ran, 8, "all eight ticks of the stall are repaid, one per queued input");
        assert!(with < TICK);
        // Even a long freeze is bounded per frame by the old 10-tick limit.
        let mut long = Duration::from_secs(30);
        assert_eq!(clamp_backlog(&mut long, TICK, true), Duration::ZERO);
        let mut ran = 0;
        while may_run_another_tick(long, TICK, ran, Duration::ZERO, true) {
            long = long.saturating_sub(TICK);
            ran += 1;
        }
        assert_eq!(ran, MAX_TICKS_PER_FRAME);
    }
}
