//! The life term ("eco") of a patch of ground (living-world spec §1.2).
//!
//! A patch is a set of columns. [`EcoTally`] collects what the scan saw there;
//! [`EcoTally::eco`] turns it into a number in 0..=1. Everything here is pure:
//! the scan that fills a tally lives in `scan.rs`.

use super::kinds::FAMILIES;

/// Value of a column whose top block is alive.
pub const LIVING_VALUE: f32 = 1.0;
/// Value of an untouched natural surface (sand in a desert, rock on a cliff,
/// the wall of a ravine): not green, but not wounded either.
pub const NATURAL_VALUE: f32 = 0.8;
/// Value of bare dirt or tilled soil with nothing on it.
pub const SOIL_VALUE: f32 = 0.4;

/// `cover = (1 - e^(-COVER_RATE f)) / (1 - e^(-COVER_RATE))`.
const COVER_RATE: f32 = 3.0;
/// `div = 1 - e^(-kinds / DIVERSITY_SCALE)`.
const DIVERSITY_SCALE: f32 = 4.0;
/// Diversity weight: `0.5 + 0.5 div`.
const DIV_BASE: f32 = 0.5;
/// A monoculture needs at least this many crop cells...
const MONO_MIN_CELLS: u32 = 24;
/// ...with one family at least this share of them.
const MONO_SHARE: f32 = 0.85;
/// The penalty for a monoculture.
const MONO_FACTOR: f32 = 0.6;
/// Walls only start to count against a region past this built share.
const WALL_FREE: f32 = 0.6;
/// How hard walls count past it.
const WALL_SLOPE: f32 = 1.5;
/// With every column built, the living share is taken as this.
const ALL_BUILT_LIVING: f32 = 0.5;

/// What a column's top block is, for the life term.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopClass {
    Living,
    Natural,
    Soil,
    /// Not alive, not placed by a player, and not what the ground naturally is.
    Bare,
    /// A non-living block a player placed: left out of the average.
    Built,
}

/// Classify a column from its top block.
///
/// `living`: the top block is alive. `placed`: the player-placed bit. `natural`:
/// the block is what worldgen left there (same block at the same height), or a
/// weather cover such as snow. `soil`: dirt or tilled soil.
pub fn classify(living: bool, placed: bool, natural: bool, soil: bool) -> TopClass {
    if living {
        TopClass::Living
    } else if placed {
        TopClass::Built
    } else if natural {
        TopClass::Natural
    } else if soil {
        TopClass::Soil
    } else {
        TopClass::Bare
    }
}

/// What a scan saw in a patch of ground.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EcoTally {
    /// Columns that held any block at all (loaded columns).
    pub columns: u32,
    pub built: u32,
    pub value_sum: f32,
    /// Bit-or of the kind bits of every living block seen.
    pub kinds: u64,
    /// Crop cells (columns topped by a crop) per family.
    pub crop_cells: [u16; FAMILIES],
}

impl EcoTally {
    /// Add one column.
    pub fn add_column(&mut self, class: TopClass) {
        self.columns += 1;
        match class {
            TopClass::Living => self.value_sum += LIVING_VALUE,
            TopClass::Natural => self.value_sum += NATURAL_VALUE,
            TopClass::Soil => self.value_sum += SOIL_VALUE,
            TopClass::Bare => {}
            TopClass::Built => self.built += 1,
        }
    }

    /// Note a living kind found while walking down a column.
    pub fn add_kind(&mut self, kind_bit: u8) {
        self.kinds |= 1u64 << kind_bit;
    }

    /// Note a crop cell of `family`.
    pub fn add_crop_cell(&mut self, family: u8) {
        let c = &mut self.crop_cells[usize::from(family)];
        *c = c.saturating_add(1);
    }

    /// Fold another tally in (a probe box spans several regions).
    pub fn merge(&mut self, other: &EcoTally) {
        self.columns += other.columns;
        self.built += other.built;
        self.value_sum += other.value_sum;
        self.kinds |= other.kinds;
        for (a, b) in self.crop_cells.iter_mut().zip(other.crop_cells) {
            *a = a.saturating_add(b);
        }
    }

    /// `true` once a scan has seen any ground.
    pub fn has_data(&self) -> bool {
        self.columns > 0
    }

    /// The share of non-built columns that is alive, weighted by class.
    pub fn living_frac(&self) -> f32 {
        let open = self.columns - self.built;
        if open == 0 {
            ALL_BUILT_LIVING
        } else {
            self.value_sum / open as f32
        }
    }

    /// Distinct living kinds seen, plus `wildlife_kinds` passive species.
    pub fn kind_count(&self, wildlife_kinds: u32) -> u32 {
        self.kinds.count_ones() + wildlife_kinds
    }

    fn mono(&self) -> f32 {
        let total: u32 = self.crop_cells.iter().map(|&c| u32::from(c)).sum();
        let top = self.crop_cells.iter().copied().max().map_or(0, u32::from);
        if total >= MONO_MIN_CELLS && top as f32 >= MONO_SHARE * total as f32 {
            MONO_FACTOR
        } else {
            1.0
        }
    }

    fn wall(&self) -> f32 {
        if self.columns == 0 {
            return 1.0;
        }
        let built_frac = self.built as f32 / self.columns as f32;
        1.0 - WALL_SLOPE * (built_frac - WALL_FREE).max(0.0)
    }

    /// The life term, 0..=1. `wildlife_kinds` is the number of distinct passive
    /// species standing in the patch.
    pub fn eco(&self, wildlife_kinds: u32) -> f32 {
        let f = self.living_frac().clamp(0.0, 1.0);
        let cover = (1.0 - (-COVER_RATE * f).exp()) / (1.0 - (-COVER_RATE).exp());
        let div = 1.0 - (-(self.kind_count(wildlife_kinds) as f32) / DIVERSITY_SCALE).exp();
        (cover.powf(0.7) * (DIV_BASE + (1.0 - DIV_BASE) * div) * self.wall() * self.mono())
            .clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(living: u32, soil: u32, bare: u32, kinds: u32) -> EcoTally {
        let mut t = EcoTally::default();
        for _ in 0..living {
            t.add_column(TopClass::Living);
        }
        for _ in 0..soil {
            t.add_column(TopClass::Soil);
        }
        for _ in 0..bare {
            t.add_column(TopClass::Bare);
        }
        for k in 0..kinds {
            t.add_kind(k as u8);
        }
        t
    }

    #[test]
    fn a_stripped_patch_is_zero_and_a_full_varied_one_is_near_one() {
        assert_eq!(tally(0, 0, 256, 0).eco(0), 0.0);
        let full = tally(256, 0, 0, 9).eco(0);
        assert!(full > 0.85, "{full}");
        assert!(full <= 1.0);
    }

    #[test]
    fn more_cover_and_more_kinds_both_raise_the_term() {
        let a = tally(40, 0, 216, 3).eco(0);
        let b = tally(120, 0, 136, 3).eco(0);
        let c = tally(120, 0, 136, 9).eco(0);
        assert!(a < b && b < c, "{a} {b} {c}");
    }

    #[test]
    fn wildlife_adds_to_diversity() {
        let t = tally(100, 0, 156, 2);
        assert!(t.eco(2) > t.eco(0));
    }

    #[test]
    fn a_monoculture_is_penalised_only_past_the_cell_threshold() {
        let mut one = tally(0, 0, 0, 0);
        let mut mixed = one.clone();
        for i in 0..64u32 {
            one.add_column(TopClass::Living);
            one.add_crop_cell(0);
            mixed.add_column(TopClass::Living);
            mixed.add_crop_cell((i % 3) as u8);
        }
        assert!((one.mono() - MONO_FACTOR).abs() < 1e-6);
        assert_eq!(mixed.mono(), 1.0);
        let mut few = tally(0, 0, 0, 0);
        for _ in 0..(MONO_MIN_CELLS - 1) {
            few.add_column(TopClass::Living);
            few.add_crop_cell(0);
        }
        assert_eq!(few.mono(), 1.0);
    }

    #[test]
    fn built_columns_are_left_out_until_walls_cover_most_of_the_region() {
        let mut t = tally(100, 0, 0, 4);
        let base = t.eco(0);
        for _ in 0..100 {
            t.add_column(TopClass::Built);
        }
        // Half built: the living share of what is left is unchanged, no wall penalty.
        assert!((t.eco(0) - base).abs() < 1e-5);
        for _ in 0..156 {
            t.add_column(TopClass::Built);
        }
        // 256 of 356 built (72%): the wall term bites.
        assert!(t.eco(0) < base);
        // Every column built: 0.5 is assumed, not zero.
        let mut all = EcoTally::default();
        for _ in 0..256 {
            all.add_column(TopClass::Built);
        }
        assert_eq!(all.living_frac(), ALL_BUILT_LIVING);
    }

    #[test]
    fn classify_follows_the_spec_order() {
        assert_eq!(classify(true, true, true, true), TopClass::Living);
        assert_eq!(classify(false, true, true, true), TopClass::Built);
        assert_eq!(classify(false, false, true, true), TopClass::Natural);
        assert_eq!(classify(false, false, false, true), TopClass::Soil);
        assert_eq!(classify(false, false, false, false), TopClass::Bare);
    }
}
