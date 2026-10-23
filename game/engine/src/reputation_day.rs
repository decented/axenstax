//! Runtime-only reputation days: actual natural clock advance, never phase relocation.
pub(crate) const SOLAR_UNITS_PER_DAY: u32 = 24_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnerId([u8; 16]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DayPulse {
    pub(crate) owner: OwnerId,
    pub(crate) tick: u64,
    pub(crate) days: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DayOffer {
    pub(crate) run_id: [u8; 16],
    pub(crate) pulse: DayPulse,
}

#[derive(Debug)]
pub(crate) struct DayClock {
    owner: OwnerId,
    remainder: u32,
    last_tick: Option<u64>,
}
impl DayClock {
    pub(crate) fn new() -> Self {
        let mut owner = [0; 16];
        getrandom::fill(&mut owner).expect("reputation clock owner randomness unavailable");
        Self {
            owner: OwnerId(owner),
            remainder: 0,
            last_tick: None,
        }
    }
    pub(crate) fn owner(&self) -> OwnerId {
        self.owner
    }
    /// Only the authoritative natural clock step calls this. Tick rollback needs a new owner.
    pub(crate) fn advance(&mut self, tick: u64, units: u32) -> Option<DayPulse> {
        if self.last_tick.is_some_and(|last| tick <= last) {
            return None;
        }
        self.last_tick = Some(tick);
        let total = u64::from(self.remainder) + u64::from(units);
        self.remainder = (total % u64::from(SOLAR_UNITS_PER_DAY)) as u32;
        Some(DayPulse {
            owner: self.owner,
            tick,
            days: (total / u64::from(SOLAR_UNITS_PER_DAY)) as u32,
        })
    }
    #[cfg(test)]
    pub(crate) fn remainder(&self) -> u32 {
        self.remainder
    }
}

#[derive(Debug, Default)]
pub(crate) struct DayConsumer {
    owner: Option<OwnerId>,
    last_tick: Option<u64>,
}
impl DayConsumer {
    /// Current owner is supplied by the actual lend window, never taken from an offered event.
    pub(crate) fn consume(
        &mut self,
        owner: OwnerId,
        tick: u64,
        run_id: [u8; 16],
        offer: Option<DayOffer>,
    ) -> u32 {
        if self.owner != Some(owner) {
            self.owner = Some(owner);
            self.last_tick = None;
        }
        let Some(offer) = offer else {
            return 0;
        };
        if offer.run_id != run_id
            || offer.pulse.owner != owner
            || offer.pulse.tick != tick
            || self.last_tick.is_some_and(|last| tick <= last)
        {
            return 0;
        }
        self.last_tick = Some(tick);
        offer.pulse.days
    }
}

/// Bounded even for a large natural increment; no per-day mutation loop.
pub(crate) fn decay_score(score: i16, days: u32) -> i16 {
    let magnitude = i32::from(score).unsigned_abs();
    let remaining = magnitude.saturating_sub(days) as i32;
    (remaining * i32::from(score.signum())) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural_units_complete_a_day_independently_of_solar_phase() {
        let mut clock = DayClock::new();
        for tick in 1..24_000 {
            assert_eq!(clock.advance(tick, 1).unwrap().days, 0);
        }
        assert_eq!(clock.advance(24_000, 1).unwrap().days, 1);
        assert_eq!(clock.remainder(), 0);
        assert_eq!(clock.advance(24_001, 1).unwrap().days, 0);
    }
    #[test]
    fn duplicate_or_stale_owner_advances_do_not_repeat_a_day() {
        let mut clock = DayClock::new();
        let owner = clock.owner();
        let pulse = clock.advance(1, 24_000).unwrap();
        assert!(clock.advance(1, 24_000).is_none());
        assert!(clock.advance(0, 24_000).is_none());
        let mut consumer = DayConsumer::default();
        let offer = DayOffer {
            run_id: [1; 16],
            pulse,
        };
        assert_eq!(consumer.consume(owner, 1, [1; 16], Some(offer)), 1);
        assert_eq!(consumer.consume(owner, 1, [1; 16], Some(offer)), 0);
        let next = DayOffer {
            pulse: clock.advance(2, 24_000).unwrap(),
            ..offer
        };
        assert_eq!(consumer.consume(owner, 2, [1; 16], Some(next)), 1);
        assert_eq!(consumer.consume(owner, 1, [1; 16], Some(offer)), 0);
        assert_eq!(consumer.consume(owner, 3, [2; 16], Some(next)), 0);
    }
    #[test]
    fn step_changes_zero_step_and_widened_addition_keep_exact_remainder() {
        let mut clock = DayClock::new();
        assert_eq!(clock.advance(1, 23_998).unwrap().days, 0);
        assert_eq!(clock.advance(2, 0).unwrap().days, 0);
        assert_eq!(clock.remainder(), 23_998);
        assert_eq!(clock.advance(3, 4).unwrap().days, 1);
        assert_eq!(clock.remainder(), 2);
        let total = 2u64 + u64::from(u32::MAX);
        assert_eq!(
            u64::from(clock.advance(4, u32::MAX).unwrap().days),
            total / 24_000
        );
        assert_eq!(u64::from(clock.remainder()), total % 24_000);
        assert_eq!(decay_score(i16::MIN, u32::MAX), 0);
        assert_eq!(decay_score(8, 2), 6);
        assert_eq!(decay_score(-25, 2), -23);
        assert_eq!(decay_score(0, 1), 0);
    }
    #[test]
    fn reset_changes_owner_epoch_and_rejects_old_pulses() {
        let mut old = DayClock::new();
        let pulse = old.advance(24_000, 24_000).unwrap();
        let mut new = DayClock::new();
        assert_ne!(old.owner(), new.owner());
        assert_eq!(new.remainder(), 0);
        let mut consumer = DayConsumer::default();
        assert_eq!(
            consumer.consume(
                new.owner(),
                24_000,
                [1; 16],
                Some(DayOffer {
                    run_id: [1; 16],
                    pulse
                })
            ),
            0
        );
        let own = new.advance(1, 24_000).unwrap();
        assert_eq!(
            consumer.consume(
                new.owner(),
                1,
                [1; 16],
                Some(DayOffer {
                    run_id: [1; 16],
                    pulse: own
                })
            ),
            1
        );
    }
}
