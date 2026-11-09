//! Living world Tier 2 S7 — crew moods (spec
//! `docs/superpowers/specs/2026-11-09-living-world-tier2.md` §7, call T10).
//!
//! A little of how a place is doing shows on the villagers in it, as a mote now
//! and then:
//!
//! * where the bloom stage is Alive, or a Feast surge is running, a happy mote
//!   (confetti for a surge, a green poof for a thriving place);
//! * where the place is Grey and still, a grey puff;
//! * anywhere else, nothing.
//!
//! It is PRESENTATION ONLY (T10): it reads the client's bloom field and its
//! villagers (the real ones, or a joiner's mirrored ones in `remote_mobs`) and
//! spawns particles. It changes nothing in the simulation, saves nothing and
//! sends nothing, so every path (single-player, a lending host, a joiner)
//! draws it from its own copy.
//!
//! Cheap by construction: a pass runs every [`PASS_EVERY_TICKS`], looks at no more
//! than [`MAX_VILLAGERS`] (the nearest ones within [`RADIUS`] of a camera) and
//! spawns no more than [`MAX_BURSTS`]. "Occasionally" is a hash of the villager
//! and the pass number ([`is_occasional`]), so there is no random state and the
//! same villager at the same beat always makes the same choice.

use glam::Vec3;

use crate::bloom::Stage;

/// Villagers farther than this from every camera are left alone.
pub const RADIUS: f32 = 32.0;
/// Logical ticks between passes (every two seconds at 20 TPS).
pub const PASS_EVERY_TICKS: u64 = 40;
/// Most villagers one pass looks at (the nearest ones).
pub const MAX_VILLAGERS: usize = 24;
/// Most motes one pass spawns.
pub const MAX_BURSTS: usize = 6;
/// A villager makes a mote on about one pass in this many.
pub const ONE_IN: u64 = 6;
/// The level from which a place reads as Alive (the top stage word).
pub const ALIVE_LEVEL: f32 = 0.7;
/// A fading Feast surge above this counts as running.
pub const SURGE_RUNNING: f32 = 0.05;

/// How a villager looks today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// A Feast surge is running: confetti.
    Celebrating,
    /// The place is Alive: a green poof.
    Thriving,
    /// The place is Grey and still: a grey puff.
    Grey,
}

/// The mood a region's `level` and Feast `surge` give, or `None` for the stages
/// in between. A surge wins over everything (a feast lifts even a grey place).
pub fn mood_of(level: f32, surge: f32) -> Option<Mood> {
    if surge >= SURGE_RUNNING {
        return Some(Mood::Celebrating);
    }
    match Stage::of(level, ALIVE_LEVEL, 0.0, 0.0) {
        Stage::Alive => Some(Mood::Thriving),
        Stage::GreyAndStill => Some(Mood::Grey),
        Stage::Stirring | Stage::WakingUp => None,
    }
}

/// A 64-bit mix of a villager's key and the pass number (splitmix64's finaliser).
pub fn mix(key: u64, beat: u64) -> u64 {
    let mut z = key.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ beat.wrapping_mul(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Does villager `key` make a mote on pass `beat`? About one in [`ONE_IN`].
pub fn is_occasional(key: u64, beat: u64) -> bool {
    mix(key, beat).is_multiple_of(ONE_IN)
}

/// One mote to spawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mote {
    pub at: Vec3,
    pub mood: Mood,
    /// A seed for the particle's own jitter.
    pub seed: u64,
}

/// One pass: of `villagers` (a stable key and a position each), those within
/// [`RADIUS`] of any of `eyes`, nearest [`MAX_VILLAGERS`] only, the ones that are
/// occasional on `beat` and whose place has a mood (`mood_at` the villager's
/// position), at most [`MAX_BURSTS`] of them, in order of distance.
pub fn pick(
    villagers: &[(u64, Vec3)],
    eyes: &[Vec3],
    beat: u64,
    mood_at: impl Fn(Vec3) -> Option<Mood>,
) -> Vec<Mote> {
    let mut near: Vec<(f32, u64, Vec3)> = villagers
        .iter()
        .filter_map(|&(key, at)| {
            let d2 = eyes.iter().map(|e| e.distance_squared(at)).fold(f32::INFINITY, f32::min);
            (d2 <= RADIUS * RADIUS).then_some((d2, key, at))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    near.truncate(MAX_VILLAGERS);
    let mut out = Vec::new();
    for (_, key, at) in near {
        if out.len() >= MAX_BURSTS {
            break;
        }
        if !is_occasional(key, beat) {
            continue;
        }
        if let Some(mood) = mood_at(at) {
            out.push(Mote { at, mood, seed: mix(key, beat ^ 0xA5A5) });
        }
    }
    out
}

impl crate::GameState {
    /// Draw this pass's crew moods (every [`PASS_EVERY_TICKS`]). Reads the bloom
    /// field and the villagers in this client's ECS; spawns particles only.
    pub(crate) fn tick_crew_moods(&mut self) {
        if !self.tick_counter.is_multiple_of(PASS_EVERY_TICKS) || !self.particles.is_enabled() {
            return;
        }
        let eyes: Vec<Vec3> = self.players.iter().map(|p| p.player.pos).collect();
        let villagers: Vec<(u64, Vec3)> = crate::feast::villagers_in(&self.ecs)
            .into_iter()
            .map(|(e, at)| (e.to_bits().get(), at))
            .collect();
        if eyes.is_empty() || villagers.is_empty() {
            return;
        }
        let beat = self.tick_counter / PASS_EVERY_TICKS;
        let bloom = &self.bloom;
        let motes = pick(&villagers, &eyes, beat, |at| {
            bloom.mood_inputs_at(at.x, at.z).and_then(|(level, surge)| mood_of(level, surge))
        });
        for m in motes {
            let head = m.at + Vec3::new(0.0, 2.1, 0.0);
            match m.mood {
                Mood::Celebrating => self.particles.burst_confetti(head, 8, m.seed),
                Mood::Thriving => self.particles.poof_green(head, 4, m.seed),
                Mood::Grey => self.particles.puff(head, Vec3::new(0.0, 0.3, 0.0), 3, m.seed),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mood_picker_follows_the_stage_and_a_surge_wins() {
        // Grey and still (below 0.25), no surge: a grey puff.
        assert_eq!(mood_of(0.0, 0.0), Some(Mood::Grey));
        assert_eq!(mood_of(0.24, 0.0), Some(Mood::Grey));
        // Stirring and waking up: nothing.
        assert_eq!(mood_of(0.25, 0.0), None);
        assert_eq!(mood_of(0.44, 0.0), None);
        assert_eq!(mood_of(0.5, 0.0), None);
        assert_eq!(mood_of(0.69, 0.0), None);
        // Alive: a green poof.
        assert_eq!(mood_of(0.7, 0.0), Some(Mood::Thriving));
        assert_eq!(mood_of(1.0, 0.0), Some(Mood::Thriving));
        // A running surge is confetti whatever the stage, even grey.
        assert_eq!(mood_of(0.0, 0.5), Some(Mood::Celebrating));
        assert_eq!(mood_of(0.5, SURGE_RUNNING), Some(Mood::Celebrating));
        assert_eq!(mood_of(1.0, 0.9), Some(Mood::Celebrating));
        // A faded surge is no surge.
        assert_eq!(mood_of(0.5, SURGE_RUNNING / 2.0), None);
    }

    #[test]
    fn occasional_is_a_pure_hash_near_one_in_n() {
        // The same villager and beat always decide the same way.
        for key in [1u64, 7, 4242, u64::MAX] {
            for beat in [0u64, 1, 99] {
                assert_eq!(is_occasional(key, beat), is_occasional(key, beat));
            }
        }
        // And over many passes it is about one in ONE_IN.
        let hits = (0..6_000u64).filter(|&beat| is_occasional(12_345, beat)).count();
        let expected = 6_000 / ONE_IN as usize;
        assert!(hits > expected * 3 / 4 && hits < expected * 5 / 4, "{hits} of 6000 (about {expected})");
        // Different villagers do not all move together.
        let together = (0..200u64).filter(|&beat| is_occasional(1, beat) == is_occasional(2, beat)).count();
        assert!(together < 200, "two villagers make different choices on some pass");
    }

    fn crowd(n: usize) -> Vec<(u64, Vec3)> {
        (0..n).map(|i| (i as u64 + 1, Vec3::new(1.0 + i as f32 * 0.1, 64.0, 0.0))).collect()
    }

    #[test]
    fn a_pass_spawns_at_most_the_cap_and_looks_only_near_the_camera() {
        let eyes = [Vec3::new(0.0, 64.0, 0.0)];
        let always = |_: Vec3| Some(Mood::Thriving);
        // Across many beats, however many villagers stand round, never more than the cap.
        let villagers = crowd(200);
        let mut most = 0;
        let mut any = false;
        for beat in 0..400u64 {
            let motes = pick(&villagers, &eyes, beat, always);
            assert!(motes.len() <= MAX_BURSTS, "beat {beat}: {} motes", motes.len());
            most = most.max(motes.len());
            any |= !motes.is_empty();
        }
        assert!(any, "some pass does draw a mote");
        assert!(most > 0);
        // Only the nearest MAX_VILLAGERS are looked at: villagers past that are never picked.
        let far_keys: Vec<u64> = (MAX_VILLAGERS as u64 + 1..=200).collect();
        for beat in 0..400u64 {
            for m in pick(&villagers, &eyes, beat, always) {
                let key = villagers.iter().find(|(_, at)| *at == m.at).unwrap().0;
                assert!(!far_keys.contains(&key), "villager {key} is past the pass's cap");
            }
        }
        // Beyond the radius: nothing, whatever the beat.
        let distant = vec![(1u64, Vec3::new(RADIUS + 1.0, 64.0, 0.0)), (2, Vec3::new(0.0, 64.0, -RADIUS - 5.0))];
        for beat in 0..400u64 {
            assert!(pick(&distant, &eyes, beat, always).is_empty());
        }
        // No eyes, no villagers: nothing.
        assert!(pick(&villagers, &[], 3, always).is_empty());
        assert!(pick(&[], &eyes, 3, always).is_empty());
    }

    #[test]
    fn a_place_with_no_mood_draws_nothing() {
        let eyes = [Vec3::ZERO];
        let villagers = crowd(30);
        for beat in 0..200u64 {
            assert!(pick(&villagers, &eyes, beat, |_| None).is_empty());
        }
    }

    #[test]
    fn the_same_inputs_give_the_same_motes() {
        let eyes = [Vec3::new(0.0, 64.0, 0.0), Vec3::new(5.0, 64.0, 5.0)];
        let villagers = crowd(40);
        let mood = |at: Vec3| if at.x < 2.0 { Some(Mood::Grey) } else { Some(Mood::Celebrating) };
        for beat in 0..50u64 {
            assert_eq!(pick(&villagers, &eyes, beat, mood), pick(&villagers, &eyes, beat, mood));
        }
    }

    #[test]
    fn each_mote_carries_the_mood_of_its_own_place() {
        let eyes = [Vec3::ZERO];
        let villagers = vec![(1u64, Vec3::new(3.0, 0.0, 0.0)), (2, Vec3::new(-3.0, 0.0, 0.0))];
        let mood = |at: Vec3| if at.x > 0.0 { Some(Mood::Grey) } else { Some(Mood::Thriving) };
        let mut grey = false;
        let mut thriving = false;
        for beat in 0..400u64 {
            for m in pick(&villagers, &eyes, beat, mood) {
                match m.mood {
                    Mood::Grey => {
                        assert!(m.at.x > 0.0);
                        grey = true;
                    }
                    Mood::Thriving => {
                        assert!(m.at.x < 0.0);
                        thriving = true;
                    }
                    Mood::Celebrating => panic!("no surge here"),
                }
            }
        }
        assert!(grey && thriving);
    }
}
