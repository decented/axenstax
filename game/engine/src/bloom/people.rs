//! The people-together term (living-world spec §1.3), runtime only.
//!
//! Three parts, each 0..=1, combined as `P = 1 - (1-c)(1-f)(1-k)`:
//! co-presence `c`, the Feast `f` and Commons `k`. A solo player with no Feast
//! and no Commons has `P = 0`: absent, never a penalty.
//!
//! Tier 2 S3 adds a fifth, `u`, lived in (`use_record::u_term`): the days a
//! player was present and the nights slept, in a place with at least 16 placed
//! blocks, fading over 14 to 28 lived days of disuse. It is read like `b`.
//!
//! Tier 2 S2 adds a fourth, `b`, built together (`P = 1 - (1-c)(1-f)(1-k)(1-b)`):
//! the number of distinct recorded authors of the placed blocks in a region
//! (`authorship::built_together`). It is read from the world's authorship table
//! on the region scan cadence, so only a machine that owns the world sees it.
//!
//! The Feast and Commons blocks come with slice S3. The field takes them as
//! plain inputs ([`FeastMarker`], [`CommonsArea`]) so S3 only has to feed it.

/// Seconds for co-presence to close the gap for one extra player.
const CO_RISE_SECS: f32 = 180.0;
/// Seconds for co-presence to fade when a place is empty or solo.
const CO_FADE_SECS: f32 = 1800.0;
/// Seconds for a Feast surge to fade.
const SURGE_FADE_SECS: f32 = 1200.0;
/// Guests at which a Feast reaches 63 % of its strength.
const FEAST_GUEST_SCALE: f32 = 3.0;
/// A Feast Marker's lasting floor, per marker, at full strength.
const MARKER_FLOOR: f32 = 0.15;
/// Cap on the summed floors in one region.
const MARKER_FLOOR_CAP: f32 = 0.3;
/// What a Commons area adds.
pub const COMMONS_K: f32 = 0.1;
/// Players this close to a region (blocks) count as present in it.
pub const PRESENCE_MARGIN: f32 = 8.0;
/// Blocks per region side.
const REGION: f32 = 16.0;

/// A Feast Marker block: where it stands and how many guests were fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeastMarker {
    pub x: i32,
    pub z: i32,
    pub guests: u8,
}

impl From<(i32, i32, u8)> for FeastMarker {
    fn from((x, z, guests): (i32, i32, u8)) -> Self {
        Self { x, z, guests }
    }
}

/// A Commons footprint, inclusive block bounds in x and z.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommonsArea {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
}

impl CommonsArea {
    /// A Commons Stone's footprint: the same 33 x 33 as a Plot, marker +-16.
    pub fn from_marker(x: i32, z: i32) -> Self {
        let h = crate::plot::PLOT_HALF_EXTENT;
        Self { min_x: x - h, max_x: x + h, min_z: z - h, max_z: z + h }
    }

    /// Does this footprint overlap the block rectangle (inclusive)?
    pub fn overlaps(&self, min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> bool {
        self.min_x <= max_x && self.max_x >= min_x && self.min_z <= max_z && self.max_z >= min_z
    }
}

/// The strength of a Feast for `guests` guests, 0..1.
pub fn feast_strength(guests: u8) -> f32 {
    1.0 - (-f32::from(guests) / FEAST_GUEST_SCALE).exp()
}

/// The lasting floor given by the Feast Markers in a patch (summed, capped).
pub fn marker_floor(guests: impl IntoIterator<Item = u8>) -> f32 {
    guests.into_iter().map(|g| MARKER_FLOOR * feast_strength(g)).sum::<f32>().min(MARKER_FLOOR_CAP)
}

/// The people state of one region or probe.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PeopleState {
    /// Co-presence.
    pub c: f32,
    /// The Feast surge, fading.
    pub surge: f32,
}

impl PeopleState {
    /// Advance by `dt` seconds with `present` distinct players near.
    pub fn step(&mut self, present: usize, in_commons: bool, dt: f32) {
        if present >= 2 {
            let boost = if in_commons { 2.0 } else { 1.0 };
            let rate = ((present - 1) as f32 * boost * dt / CO_RISE_SECS).min(1.0);
            self.c += (1.0 - self.c) * rate;
        } else {
            self.c -= self.c * (dt / CO_FADE_SECS).min(1.0);
        }
        self.surge -= self.surge * (dt / SURGE_FADE_SECS).min(1.0);
        if self.c < 1e-6 {
            self.c = 0.0;
        }
        if self.surge < 1e-6 {
            self.surge = 0.0;
        }
    }

    /// A Feast was held here for `guests` guests.
    pub fn note_feast(&mut self, guests: u8) {
        self.surge = self.surge.max(feast_strength(guests));
    }

    /// `P`, given the Markers' lasting `floor` and whether a Commons overlaps
    /// (no built-together term: probes, which have no authorship to read).
    pub fn value(&self, floor: f32, in_commons: bool) -> f32 {
        self.value_with(floor, in_commons, 0.0)
    }

    /// `P` with the built-together term `b` (0..=1) as well.
    pub fn value_with(&self, floor: f32, in_commons: bool, b: f32) -> f32 {
        self.value_full(floor, in_commons, b, 0.0)
    }

    /// `P` with the built-together term `b` and the lived-in term `u` (both
    /// 0..=1): `1 - (1-c)(1-f)(1-k)(1-b)(1-u)` (Tier 2 S3 adds `u`).
    pub fn value_full(&self, floor: f32, in_commons: bool, b: f32, u: f32) -> f32 {
        let f = self.surge.max(floor.min(MARKER_FLOOR_CAP));
        let k = if in_commons { COMMONS_K } else { 0.0 };
        1.0 - (1.0 - self.c)
            * (1.0 - f)
            * (1.0 - k)
            * (1.0 - b.clamp(0.0, 1.0))
            * (1.0 - u.clamp(0.0, 1.0))
    }
}

/// The region keys a player at `(x, z)` counts as present in: the region it
/// stands in and any within [`PRESENCE_MARGIN`] blocks of it (up to four).
pub fn regions_near(x: f32, z: f32) -> impl Iterator<Item = (i32, i32)> {
    let span = |p: f32| {
        let lo = ((p - REGION - PRESENCE_MARGIN) / REGION).floor() as i32 + 1;
        let hi = ((p + PRESENCE_MARGIN) / REGION).floor() as i32;
        lo..=hi
    };
    let zs = span(z);
    span(x).flat_map(move |cx| zs.clone().map(move |cz| (cx, cz)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_never_lifts_and_pairs_do() {
        let mut p = PeopleState::default();
        for _ in 0..2000 {
            p.step(1, false, 0.05);
        }
        assert_eq!(p.c, 0.0);
        assert_eq!(p.value(0.0, false), 0.0);
        for _ in 0..(180 * 20) {
            p.step(2, false, 0.05);
        }
        assert!(p.c > 0.55 && p.c < 0.75, "{}", p.c);
    }

    #[test]
    fn built_together_is_a_fourth_independent_factor_of_p() {
        let p = PeopleState { c: 0.5, surge: 0.0 };
        let (floor, b) = (0.0, 0.3935);
        let base = p.value_with(floor, true, 0.0);
        assert_eq!(base, p.value(floor, true), "b = 0 is the Tier 1 value");
        let with_b = p.value_with(floor, true, b);
        // 1 - (1-c)(1-k)(1-b)
        let want = 1.0 - (1.0 - 0.5) * (1.0 - COMMONS_K) * (1.0 - b);
        assert!((with_b - want).abs() < 1e-6, "{with_b} vs {want}");
        assert!(with_b > base);
        // Alone, it lifts a place nobody is in: the people built it together.
        let alone = PeopleState::default().value_with(0.0, false, b);
        assert!((alone - b).abs() < 1e-6);
        assert_eq!(PeopleState::default().value_with(0.0, false, 2.0), 1.0, "clamped");
    }

    #[test]
    fn commons_doubles_the_rise_and_adds_its_own_lift() {
        let (mut a, mut b) = (PeopleState::default(), PeopleState::default());
        for _ in 0..400 {
            a.step(2, false, 0.05);
            b.step(2, true, 0.05);
        }
        assert!(b.c > a.c * 1.8, "{} vs {}", b.c, a.c);
        assert!((PeopleState::default().value(0.0, true) - COMMONS_K).abs() < 1e-6);
    }

    #[test]
    fn co_presence_fades_slowly_when_people_leave() {
        let mut p = PeopleState { c: 1.0, surge: 0.0 };
        for _ in 0..(60 * 20) {
            p.step(0, false, 0.05);
        }
        assert!(p.c > 0.9 && p.c < 1.0, "{}", p.c);
    }

    #[test]
    fn a_feast_surges_then_fades_and_markers_leave_a_capped_floor() {
        let mut p = PeopleState::default();
        p.note_feast(6);
        assert!((p.surge - (1.0 - (-2.0f32).exp())).abs() < 1e-6);
        p.note_feast(1);
        assert!(p.surge > 0.8, "a smaller feast never lowers the surge");
        for _ in 0..(1200 * 20) {
            p.step(0, false, 0.05);
        }
        assert!(p.surge < 0.4);
        assert!((marker_floor([255u8; 5]) - MARKER_FLOOR_CAP).abs() < 1e-6);
        assert!(marker_floor([]) == 0.0);
        let floor = marker_floor([6u8]);
        assert!((PeopleState::default().value(floor, false) - floor).abs() < 1e-6);
    }

    #[test]
    fn regions_near_is_the_region_stood_in_and_its_neighbours_within_eight_blocks() {
        // The margin (8) is half a region, so a player always counts in a 2 x 2 block of regions.
        for (x, z) in [(0.0, 0.0), (8.0, 8.0), (15.9, 3.0), (-0.1, 40.0)] {
            assert_eq!(regions_near(x, z).count(), 4, "{x},{z}");
        }
        let v: Vec<_> = regions_near(3.0, 12.0).collect();
        assert!(v.contains(&(0, 0)) && v.contains(&(-1, 0)) && v.contains(&(0, 1)) && v.contains(&(-1, 1)), "{v:?}");
        // 10 blocks into region 0: region 1 is 6 away, region -1 is 10 away.
        let v: Vec<_> = regions_near(10.0, 8.0).collect();
        assert!(v.contains(&(1, 0)) && !v.contains(&(-1, 0)), "{v:?}");
    }
}
