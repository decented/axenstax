//! Spec 28d chunk 13 — test audit bundle.
//!
//! Property-shaped tests that walk the engine's data tables and assert
//! invariants. Not "did this function work for these inputs" — instead
//! "for all inputs in this space, no invariant gets violated." Catches
//! a class of bug where a new MobType / Material / Recipe lands without
//! a corresponding wire-up.

use crate::biome::{Biome, assign_biome_whittaker, classify_whittaker};
use crate::block;
use crate::crop_growth::{CropFamily, crop_family, tick_crop};
use crate::mob::{MOBS, MobType, drops_for, mob_def};
use crate::tree_shapes;

// ── Mob damage table audit ─────────────────────────────────────────

/// Every mob in MOBS must produce a non-panicking drops_for result
/// across a wide seed sweep, AND mobs marked passive should never
/// drop hostile-only signature items. Post-fantasy-roster-excision the
/// only such marker is the BrigandChieftainTrophy (Berserker boss drop).
/// Note: Bone is NOT hostile-signature any more — Phase D re-sources it
/// from livestock (Cow/Sheep/Pig) so the Bone -> Bonemeal farming path
/// survives the skeleton removal.
#[test]
fn mob_drops_table_audit() {
    use crate::item::{Item, MaterialId};
    let hostile_signature = [MaterialId::BrigandChieftainTrophy];

    for def in MOBS.iter() {
        for seed in 0u32..200 {
            let drops = drops_for(def.mob_type, seed);
            // Drops never zero count.
            for stack in &drops {
                assert!(stack.count >= 1, "zero-count drop on {:?}", def.mob_type);
            }
            // Passive mobs don't drop hostile-signature items.
            if matches!(def.category, crate::mob::MobCategory::Passive) {
                for stack in &drops {
                    if let Item::Material(m) = stack.item {
                        assert!(
                            !hostile_signature.contains(&m),
                            "passive mob {:?} dropped hostile-signature material {:?} on seed {seed}",
                            def.mob_type, m,
                        );
                    }
                }
            }
        }
    }
}

/// Every mob has a sane health/size/category triple — caught any
/// 0-HP or zero-size mob TOML.
#[test]
fn mob_definitions_pass_basic_sanity() {
    for def in MOBS.iter() {
        assert!(def.health > 0, "{:?} has 0 HP", def.mob_type);
        assert!(def.width > 0.0, "{:?} has 0 width", def.mob_type);
        assert!(def.height > 0.0, "{:?} has 0 height", def.mob_type);
        assert!(!def.name.is_empty(), "{:?} has empty name", def.mob_type);
        // Speed > 0 except possibly for purely-stationary mobs (none yet).
        assert!(def.speed > 0.0, "{:?} has 0 speed", def.mob_type);
    }
}

// ── Recipe completeness audit ──────────────────────────────────────

/// Every mob in MOBS that drops a `Raw*Meat` MaterialId should have a
/// `Cooked*` counterpart that's also reachable in the engine. Catches
/// a class of drift where a new mob ships with a raw drop but no
/// cooked variant on the food ladder.
#[test]
fn raw_meat_drops_have_cooked_counterparts() {
    use crate::item::{Item, MaterialId};
    let raw_to_cooked: &[(MaterialId, MaterialId)] = &[
        (MaterialId::RawBeef, MaterialId::CookedBeef),
        (MaterialId::RawPorkchop, MaterialId::CookedPorkchop),
        (MaterialId::RawChicken, MaterialId::CookedChicken),
        (MaterialId::RawMutton, MaterialId::CookedMutton),
        (MaterialId::RawRabbit, MaterialId::CookedRabbit),
    ];

    // Walk every mob's drops; for each raw, the cooked variant must exist.
    for def in MOBS.iter() {
        for seed in 0u32..50 {
            for stack in drops_for(def.mob_type, seed) {
                if let Item::Material(m) = stack.item {
                    if let Some((_, cooked)) = raw_to_cooked.iter().find(|(raw, _)| *raw == m) {
                        // The cooked variant must have a non-None
                        // food_value (otherwise it's unreachable food).
                        let cooked_item = Item::Material(*cooked);
                        assert!(
                            cooked_item.food_value().is_some(),
                            "raw drop {m:?} found on {:?}, but cooked counterpart {cooked:?} has no food_value",
                            def.mob_type,
                        );
                    }
                }
            }
        }
    }
}

// ── Crop growth property tests ─────────────────────────────────────

/// Across 1000 seeds, every crop family must progress from stage 0 to
/// mature within `4 × period` ticks. Catches a class of bug where the
/// jitter formula overflows / underflows and stalls growth.
#[test]
fn crop_growth_reaches_mature_within_bound() {
    let families = [
        CropFamily::Wheat,
        CropFamily::Carrot,
        CropFamily::Potato,
        CropFamily::Corn,
    ];
    for family in families {
        let initial = family.initial_stage();
        let mature = family.mature_stage();
        let period = family.growth_period_ticks();
        // Bound: 4 × period gives plenty of headroom for the ±10%
        // jitter and the 4 stage progressions.
        let max_ticks = period * (family.stages().len() as u64) * 2;
        for seed in 0u64..1000 {
            let mut current = initial;
            let mut tick = 0u64;
            while tick < max_ticks && current != mature {
                let next = tick_crop(current, false, tick, seed);
                if next != current {
                    current = next;
                }
                tick += 1;
            }
            assert_eq!(
                current, mature,
                "{:?} seed {seed}: did not mature within {max_ticks} ticks", family,
            );
        }
    }
}

/// Water adjacency must always finish at least as fast as dry growth
/// for the same seed. Property: water never slows growth.
#[test]
fn water_adjacency_never_slows_growth() {
    // Sample at the stage-1 → stage-2 boundary.
    let family = CropFamily::Wheat;
    let period = family.growth_period_ticks();

    for seed in 0u64..200 {
        let mut dry_block = family.initial_stage();
        let mut wet_block = family.initial_stage();
        let mut dry_done = None;
        let mut wet_done = None;
        for tick in 0..period * 5 {
            if dry_done.is_none() {
                let next = tick_crop(dry_block, false, tick, seed);
                if crop_family(next).map(|(_, s)| s) != crop_family(dry_block).map(|(_, s)| s) {
                    dry_done = Some(tick);
                }
                dry_block = next;
            }
            if wet_done.is_none() {
                let next = tick_crop(wet_block, true, tick, seed);
                if crop_family(next).map(|(_, s)| s) != crop_family(wet_block).map(|(_, s)| s) {
                    wet_done = Some(tick);
                }
                wet_block = next;
            }
            if dry_done.is_some() && wet_done.is_some() {
                break;
            }
        }
        let (dry, wet) = (dry_done.unwrap(), wet_done.unwrap());
        assert!(
            wet <= dry,
            "seed {seed}: water-adjacent ({wet}) slower than dry ({dry})"
        );
    }
}

// ── Tree-shape integration ─────────────────────────────────────────

/// Every wood species must produce a tree with at least one log AND
/// at least one leaf block when placed.
#[test]
fn every_wood_species_has_logs_and_leaves() {
    use crate::block::WoodSpecies as W;
    for species in [W::Oak, W::Birch, W::Spruce, W::Jungle, W::Acacia, W::DarkOak] {
        let blocks = tree_shapes::place_tree(species, 0, 0, 42);
        let log = block::log_block_for(species);
        let leaves = block::leaves_block_for(species);
        let log_count = blocks.iter().filter(|t| t.id == log).count();
        let leaf_count = blocks.iter().filter(|t| t.id == leaves).count();
        assert!(log_count >= 1, "{:?} produced no logs", species);
        assert!(leaf_count >= 1, "{:?} produced no leaves", species);
    }
}

/// Tree shapes are deterministic per seed — same seed must yield the
/// same offset list. Replay-stability property.
#[test]
fn tree_shape_is_deterministic_per_seed() {
    use crate::block::WoodSpecies as W;
    for species in [W::Oak, W::Birch, W::Spruce, W::Jungle, W::Acacia, W::DarkOak] {
        for seed in [1u32, 42, 1000, u32::MAX] {
            let a = tree_shapes::place_tree(species, 0, 0, seed);
            let b = tree_shapes::place_tree(species, 0, 0, seed);
            assert_eq!(a.len(), b.len(), "{:?} seed {seed}: nondeterministic", species);
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(x.id, y.id);
                assert_eq!(x.dx, y.dx);
                assert_eq!(x.dy, y.dy);
                assert_eq!(x.dz, y.dz);
            }
        }
    }
}

// ── Whittaker classification fuzz ──────────────────────────────────

/// Across a temperature/humidity sweep we should see at least 6 of the
/// 8 launch biomes — proves the classifier isn't collapsing to one or
/// two biomes by accident.
#[test]
fn whittaker_classifier_covers_many_biomes() {
    use ahash::AHashSet;
    let mut seen: AHashSet<Biome> = AHashSet::new();
    for ti in 0..50 {
        for hi in 0..50 {
            let t = -1.0 + (ti as f32) / 25.0;
            let h = -1.0 + (hi as f32) / 25.0;
            let biome = classify_whittaker(t, h);
            seen.insert(biome);
        }
    }
    assert!(
        seen.len() >= 6,
        "Whittaker classifier only produced {} biomes: {:?}",
        seen.len(),
        seen,
    );
}

/// World-position biome assignment also must produce diversity at
/// realistic seeds. Walk a 1024-block-radius around origin and assert
/// at least 4 biomes.
#[test]
fn biome_assignment_at_world_positions_is_diverse() {
    use ahash::AHashSet;
    let mut seen: AHashSet<Biome> = AHashSet::new();
    for x in (-1024i32..=1024).step_by(64) {
        for z in (-1024i32..=1024).step_by(64) {
            seen.insert(assign_biome_whittaker(x, z, 42));
        }
    }
    assert!(seen.len() >= 4, "only {} biomes at seed 42: {:?}", seen.len(), seen);
}

/// Same biome assignment must be deterministic per (x, z, seed).
#[test]
fn biome_assignment_is_deterministic() {
    for x in [-100, 0, 50, 500] {
        for z in [-50, 0, 75] {
            let a = assign_biome_whittaker(x, z, 1234);
            let b = assign_biome_whittaker(x, z, 1234);
            assert_eq!(a, b);
        }
    }
}

// ── Mob name uniqueness ────────────────────────────────────────────

/// Every mob's name must be unique. Catches a class of bug where two
/// TOMLs land with the same display name.
#[test]
fn mob_names_are_unique() {
    use ahash::AHashSet;
    let mut seen: AHashSet<&str> = AHashSet::new();
    for def in MOBS.iter() {
        assert!(seen.insert(&def.name), "duplicate mob name: {}", def.name);
    }
}

/// Every MobType variant has a matching MOBS entry. Catches a class
/// of bug where the enum gets a new variant but the TOML doesn't load.
#[test]
fn every_mob_type_has_a_def() {
    let all_kinds = [
        MobType::Cow, MobType::Chicken,
        MobType::Pig, MobType::Sheep,
        MobType::Villager, MobType::Peddler,
        MobType::Wolf, MobType::Horse, MobType::Rabbit, MobType::Goat,
        MobType::Bee, MobType::Squid,
        MobType::Nostrich, MobType::Bear, MobType::Hyena,
        MobType::Brigand, MobType::Marauder, MobType::Berserker,
        MobType::Knight,
    ];
    for kind in all_kinds {
        let def = mob_def(kind);
        assert_eq!(def.mob_type, kind);
    }
}
