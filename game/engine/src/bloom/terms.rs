//! The four named terms of world bloom for adults (Tier 2 spec §1 and §11):
//! life, ideas, people together and time. World totals only: nothing here is
//! about a person, and nothing per person is kept.
//!
//! One pure function reads the field and the Plan-build table; the F3 grown-up
//! panel and the dedicated server's `ConsoleSnapshot` both call it, so the two
//! can never disagree. People goes through the field's own `P` (the bloom
//! function), so any term another slice adds to `P` flows in here.

use super::field::BloomField;
use crate::plan_builds::PlanBuilds;

/// The four world terms. `0..=1` terms are fractions, shown as percentages.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldTerms {
    /// Life: the mean eco term over the measured regions (0 when none).
    pub life: f32,
    /// Ideas: `(designs built at least once, total builds)` in this world.
    pub ideas: (u32, u32),
    /// People together: the mean `P` over the measured regions (0 when none).
    pub people: f32,
    /// Time: the mean lived-in term `u` over the regions that have a use
    /// record; `None` when there are none.
    pub time: Option<f32>,
}

/// The world's four terms. `tables` is the world's Plan-build table (empty on a
/// joined client, which the panel then reports as "—" for ideas).
pub fn world_terms(field: &BloomField, tables: &PlanBuilds) -> WorldTerms {
    let s = field.world_summary();
    WorldTerms {
        life: s.mean_eco,
        ideas: tables.ideas_total(),
        people: s.mean_people,
        time: field.mean_lived(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bloom::test_world::Harness;

    fn counts(table: &mut PlanBuilds, designs: &[(u8, u32)]) {
        for &(d, n) in designs {
            for _ in 0..n {
                table.note_completed([d; 32], &[]);
            }
        }
    }

    #[test]
    fn an_empty_world_has_all_four_terms_absent() {
        let t = world_terms(&BloomField::new(), &PlanBuilds::default());
        assert_eq!(t, WorldTerms { life: 0.0, ideas: (0, 0), people: 0.0, time: None });
    }

    #[test]
    fn the_ideas_total_is_designs_and_builds() {
        let mut table = PlanBuilds::default();
        counts(&mut table, &[(1, 3), (2, 1), (3, 5), (4, 2)]);
        let t = world_terms(&BloomField::new(), &table);
        assert_eq!(t.ideas, (4, 11));
    }

    #[test]
    fn one_region_gives_life_and_people_through_the_bloom_function_and_no_time_without_a_record() {
        let mut h = Harness::with_generator(1, crate::biome::Generator::V3);
        h.generate((0, 0));
        h.settle((0, 0));
        let (eco, people, _, _) = h.field.region_terms((0, 0)).unwrap();
        let t = world_terms(&h.field, &PlanBuilds::default());
        assert_eq!(t.life, eco);
        assert_eq!(t.people, people, "P is read through the field, not recomputed");
        assert_eq!(t.time, None, "no use record: absent, not zero");
    }

    #[test]
    fn time_is_the_mean_u_over_regions_with_a_use_record() {
        let mut h = Harness::with_generator(1, crate::biome::Generator::V3);
        h.generate((0, 0));
        h.generate((1, 0));
        h.local = vec![glam::Vec3::new(16.0, 90.0, 8.0)];
        // A place in region (0,0): sixteen placed blocks and ten lived days there.
        h.plant((0, 0), crate::bloom::test_world::first_columns(20), 200, crate::block::OAK_PLANKS);
        for day in 1..=10u32 {
            h.world.use_record.note_present_at(day, 4, 4);
        }
        // Region (1,0) has no record at all and must not drag the mean down.
        h.rescan((0, 0));
        for _ in 0..400 {
            h.tick();
            if h.field.region_terms((1, 0)).is_some() {
                break;
            }
        }
        let u0 = h.world.lived_in_term(0, 0);
        assert!(u0 > 0.1, "{u0}");
        assert!(h.field.region_terms((1, 0)).is_some(), "both regions measured");
        let t = world_terms(&h.field, &PlanBuilds::default());
        assert_eq!(t.time, Some(u0));
    }
}
