//! The island's layout invariants (§C.3): nothing overlaps, everything fits,
//! the same input gives the same island, and only unlocked trials get doorways.

use super::*;
use crate::trials::{TrialBests, TrialRef, TRIAL_ORDER};
use crate::trials_island_build::{inputs, layout_for};

/// Every trial open (as if all were unlocked), with its real diorama.
fn all_open() -> Vec<SlotInput> {
    let mut slots = inputs(&TrialBests::default());
    for s in &mut slots {
        s.state = SlotState::Open { title: s.token.to_string() };
    }
    slots
}

fn token_of(tref: &TrialRef) -> &'static str {
    match tref {
        TrialRef::Race(id) | TrialRef::Challenge(id) => id,
    }
}

#[test]
fn every_district_has_room_for_its_trials() {
    let slots = inputs(&TrialBests::default());
    for d in [District::Make, District::Tend, District::Tech, District::Brave] {
        let n = slots.iter().filter(|s| s.district == d).count();
        assert!(n <= district_capacity(), "{d:?} has {n} trials, room for {}", district_capacity());
    }
    assert!(slots.iter().filter(|s| s.district == District::Clock).count() <= 4);
}

#[test]
fn no_two_doorways_or_dioramas_overlap() {
    let island = layout(&all_open());
    assert_eq!(island.doorways.len(), TRIAL_ORDER.len(), "every trial has a doorway when all are open");
    let mut seen: std::collections::HashMap<[i32; 3], &str> = std::collections::HashMap::new();
    for (token, cells) in &island.footprints {
        for c in cells {
            if let Some(other) = seen.insert(*c, token) {
                assert!(other == *token, "{token} and {other} both use {c:?}");
            }
        }
    }
    // Nor does any doorway's opening or plinth stand on another's.
    for (i, a) in island.doorways.iter().enumerate() {
        for b in &island.doorways[i + 1..] {
            let apart = a.zone_max[0] < b.zone_min[0]
                || b.zone_max[0] < a.zone_min[0]
                || a.zone_max[1] < b.zone_min[1]
                || b.zone_max[1] < a.zone_min[1];
            assert!(apart, "{} and {} share ground", a.token, b.token);
        }
    }
}

#[test]
fn every_plot_stays_inside_its_district_and_off_the_plaza_and_track() {
    let island = layout(&all_open());
    for (token, cells) in island.footprints.iter().filter(|(t, _)| *t != "plaza") {
        for c in cells {
            let r = ring(c[0], c[2]);
            assert!(r <= LAND_R, "{token} reaches {c:?}, past the land");
            assert!(r > PLAZA_R, "{token} builds on the plaza at {c:?}");
        }
    }
    for (ttype, tref) in TRIAL_ORDER {
        let token = token_of(tref);
        let district = crate::trials_island_build::district_of(*ttype, *tref);
        let cells = &island.footprints.iter().find(|(t, _)| *t == token).unwrap().1;
        for c in cells {
            let (x, z) = (c[0], c[2]);
            let inside = match district {
                District::Make => x.abs() < -z,
                District::Tend => z.abs() < x,
                District::Tech => x.abs() < z,
                District::Brave => z.abs() < -x,
                District::Clock => ring(x, z) <= TRACK.1 + 1,
            };
            assert!(inside, "{token} ({district:?}) strays to {c:?}");
            if district != District::Clock {
                assert!(ring(x, z) > TRACK.1 + 1, "{token} builds on the track at {c:?}");
            }
        }
    }
}

#[test]
fn everything_fits_within_the_island() {
    let island = layout(&all_open());
    for x in -ISLAND_R - 2..=ISLAND_R + 2 {
        for z in -ISLAND_R - 2..=ISLAND_R + 2 {
            for y in MIN_Y - 1..=MAX_Y + 1 {
                if island.block_at(x, y, z) != KEEP {
                    assert!(ring(x, z) <= ISLAND_R && (MIN_Y..=MAX_Y).contains(&y), "{x},{y},{z}");
                }
            }
        }
    }
    for s in &island.signs {
        assert!(ring(s.pos[0], s.pos[2]) <= LAND_R, "sign at {:?}", s.pos);
    }
    for d in &island.doorways {
        let r = ring(d.return_pos[0].floor() as i32, d.return_pos[2].floor() as i32);
        assert!(r <= LAND_R, "{} returns off the land", d.token);
        assert!(ring(d.exhibit_at[0], d.exhibit_at[2]) <= LAND_R);
    }
}

#[test]
fn the_layout_is_deterministic() {
    let bests = TrialBests::default();
    assert_eq!(layout_for(&bests), layout_for(&bests));
    assert_eq!(layout(&all_open()), layout(&all_open()));
}

#[test]
fn a_trial_keeps_its_plot_whatever_else_unlocks() {
    let fresh = layout_for(&TrialBests::default());
    let full = layout(&all_open());
    for d in &fresh.doorways {
        assert_eq!(Some(d), full.doorway(d.token), "{} moved when others unlocked", d.token);
    }
}

#[test]
fn only_unlocked_trials_get_doorways_and_the_next_get_hint_plots() {
    let fresh = TrialBests::default();
    let island = layout_for(&fresh);
    let mut doors: Vec<&str> = island.doorways.iter().map(|d| d.token).collect();
    doors.sort_unstable();
    let mut want: Vec<&str> = TRIAL_ORDER
        .iter()
        .map(|(_, r)| token_of(r))
        .filter(|t| crate::trials_unlock::is_unlocked(t, &fresh))
        .collect();
    want.sort_unstable();
    assert_eq!(doors, want, "a fresh player sees the front shelf and the clock corner");
    // Having done Mine, the next ones show as roped-off plots, still no doorway.
    let mut started = TrialBests::default();
    started.mark_challenge_done(&crate::scenario::named_builtin_def("mine").unwrap().display_name);
    let hints = crate::trials_unlock::next_hints(&started);
    assert!(!hints.is_empty());
    let island = layout_for(&started);
    for (token, hint) in &hints {
        assert!(island.doorway(token).is_none(), "{token} is not unlocked yet");
        assert!(island.signs.iter().any(|s| s.text == *hint), "{token}'s hint sign stands");
    }
    for d in &island.doorways {
        assert!(crate::trials_unlock::is_unlocked(d.token, &started), "{} has a doorway", d.token);
    }
}

#[test]
fn signs_carry_names_and_hints_only_never_counts_or_money_words() {
    let island = layout(&all_open());
    for s in &island.signs {
        assert!(s.text.chars().count() <= crate::sign::SIGN_MAX_CHARS);
        assert!(!s.text.chars().any(|c| c.is_ascii_digit()), "a number on a sign: {:?}", s.text);
        let hits = crate::copy_lint::banned_in(&s.text, crate::copy_lint::BANNED_MONEY_WORDS);
        assert!(hits.is_empty(), "{hits:?}");
    }
    let body = include_str!("trials_island_layout.rs").split("#[cfg(test)]").next().unwrap();
    assert!(crate::copy_lint::banned_in(body, crate::copy_lint::BANNED_MONEY_WORDS).is_empty());
}

#[test]
fn a_doorway_opens_in_its_arch_and_holds_on_its_plinth_only() {
    let island = layout_for(&TrialBests::default());
    for d in &island.doorways {
        let mid = |a: [i32; 2], b: [i32; 2]| [(a[0] + b[0]) as f32 / 2.0 + 0.5, (GROUND_Y + 1) as f32, (a[1] + b[1]) as f32 / 2.0 + 0.5];
        let inside = mid(d.open_min, d.open_max);
        assert!(d.opens_at(inside) && d.holds(inside), "{}", d.token);
        assert_eq!(island.doorway_opening_at(inside).map(|o| o.token), Some(d.token));
        assert!(!d.holds(d.return_pos), "{} returns the player outside the doorway", d.token);
        assert!(!d.opens_at([inside[0], inside[1] + 4.0, inside[2]]), "standing on the arch is not walking in");
        // The opening is open air, the return spot is clear ground.
        for (x, z) in [(d.open_min[0], d.open_min[1]), (d.open_max[0], d.open_max[1])] {
            for y in GROUND_Y + 1..=GROUND_Y + 4 {
                assert_eq!(island.block_at(x, y, z), block::AIR, "{} opening blocked at {x},{y},{z}", d.token);
            }
        }
        let (rx, rz) = (d.return_pos[0].floor() as i32, d.return_pos[2].floor() as i32);
        assert_eq!(island.block_at(rx, GROUND_Y + 1, rz), block::AIR, "{} return spot blocked", d.token);
        assert!(island.block_at(rx, GROUND_Y, rz) != block::WATER);
    }
}

#[test]
fn liquids_in_a_diorama_are_cased_so_nothing_flows_or_burns() {
    let island = layout(&all_open());
    let liquid = |b| b == block::WATER || b == block::LAVA;
    for x in -LAND_R..=LAND_R {
        for z in -LAND_R..=LAND_R {
            for y in GROUND_Y..=MAX_Y {
                let b = island.block_at(x, y, z);
                if !liquid(b) {
                    continue;
                }
                let mut around = vec![(x + 1, y, z), (x - 1, y, z), (x, y, z + 1), (x, y, z - 1), (x, y - 1, z)];
                if b == block::LAVA {
                    around.push((x, y + 1, z));
                }
                for (nx, ny, nz) in around {
                    let n = island.block_at(nx, ny, nz);
                    assert!(n != block::AIR, "liquid at {x},{y},{z} touches air at {nx},{ny},{nz}");
                }
            }
        }
    }
}

#[test]
fn the_shore_is_sand_then_water_and_spawn_is_on_the_plaza() {
    let island = layout_for(&TrialBests::default());
    assert_eq!(island.block_at(0, GROUND_Y, LAND_R + 1), block::SAND);
    assert_eq!(island.block_at(ISLAND_R, GROUND_Y, 0), block::WATER);
    assert_eq!(island.block_at(ISLAND_R + 1, GROUND_Y, 0), KEEP, "the world beyond is untouched");
    let (sx, sz) = (SPAWN[0].floor() as i32, SPAWN[2].floor() as i32);
    assert_eq!(island.block_at(sx, GROUND_Y, sz), block::MARBLE);
    assert_eq!(island.block_at(sx, GROUND_Y + 1, sz), block::AIR);
    assert!(is_protected(ISLAND_R + 1, 0) && !is_protected(ISLAND_R + 2, 0));
    // A signpost to each district, and one for the track.
    assert_eq!(island.signs.iter().filter(|s| ring(s.pos[0], s.pos[2]) <= PLAZA_R).count(), 5);
}
