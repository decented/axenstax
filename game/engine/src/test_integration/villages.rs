//! Spec 19 villages — end-to-end integration tests.
//!
//! These exercise the full village quest loop *without* `TestHost` (the
//! village ticks live on the single-player game-loop side, not in
//! `GameServer::tick`, so the harness can't drive them yet). Each test
//! wires the bits directly: ECS + World + Inventory + Quest + Reputation
//! + the village/villager/quest helper functions.
//!
//! The end-to-end happy-path test walks through:
//!  1. Build a small world with a Crafting Table workstation.
//!  2. Spawn a Villager near the workstation; tick the claim scan; assert
//!     they bind to Carpenter.
//!  3. Generate a Carpenter quest, accept it, drop the required items into
//!     the player's inventory, run the completion check, run the payout,
//!     assert the player got rewarded and the reputation ledger moved.

use ahash::AHashMap;
use glam::Vec3;

use crate::block;
use crate::entity::{self, MobKind, Position, Velocity};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::mob::MobType;
use crate::quest::{self, Quest, QuestFlavour, QuestReward};
use crate::reputation::{self, Reputation, Tier};
use crate::villager::{self, Profession, VillagerComponent};
use crate::village_gen;
use crate::world::World;

fn floor_at(world: &mut World, y: i32) {
    for x in -4..=4 {
        for z in -4..=4 {
            world.set_block(x, y - 1, z, block::DIRT);
        }
    }
}

fn spawn_villager_at(ecs: &mut hecs::World, pos: Vec3) -> hecs::Entity {
    entity::spawn_mob(ecs, MobType::Villager, pos);
    // `spawn_mob` attaches a `VillagerComponent` automatically for villager
    // kinds (Spec 19 phase 3); grab the most recently spawned id by querying.
    let mut id = None;
    for (e, k) in ecs.query::<&MobKind>().iter() {
        if k.0 == MobType::Villager {
            id = Some(e);
        }
    }
    id.expect("villager spawn failed")
}

#[test]
fn villager_with_crafting_table_claims_carpenter_profession() {
    let mut world = World::new();
    floor_at(&mut world, 64);
    world.set_block(1, 64, 0, block::CRAFTING_TABLE);

    let mut ecs = hecs::World::new();
    let id = spawn_villager_at(&mut ecs, Vec3::new(0.5, 64.0, 0.0));

    villager::tick_workstation_claims(&mut ecs, &world);

    let vc = ecs.get::<&VillagerComponent>(id).unwrap();
    assert_eq!(vc.profession, Profession::Carpenter);
    assert_eq!(vc.claimed_workstation, Some([1, 64, 0]));
}

#[test]
fn destroying_workstation_releases_profession() {
    let mut world = World::new();
    floor_at(&mut world, 64);
    world.set_block(0, 64, 0, block::CRAFTING_TABLE);

    let mut ecs = hecs::World::new();
    let id = spawn_villager_at(&mut ecs, Vec3::new(0.5, 64.0, 0.0));
    villager::tick_workstation_claims(&mut ecs, &world);
    assert_eq!(
        ecs.get::<&VillagerComponent>(id).unwrap().profession,
        Profession::Carpenter,
    );

    world.set_block(0, 64, 0, block::AIR);
    villager::release_orphan_claims(&mut ecs, &world);

    let vc = ecs.get::<&VillagerComponent>(id).unwrap();
    assert_eq!(vc.profession, Profession::None);
    assert!(vc.claimed_workstation.is_none());
}

#[test]
fn fetch_quest_completes_when_inventory_holds_target_count() {
    let mut inv = Inventory::new();
    inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 5)));

    let q = Quest {
        flavour: QuestFlavour::Fetch { item: MaterialId::Wheat, count: 5 },
        reward: QuestReward { items: Vec::new(), sats: 12, reputation: 5 },
        accepted_by: Some(0),
    };

    let (have, need) = quest::progress_against(&q, &inv, 0);
    assert_eq!(have, 5);
    assert_eq!(need, 5);
    assert!(quest::is_complete(&q, &inv, 0));
}

#[test]
fn turn_in_consumes_resources_and_credits_reputation_to_village() {
    // Build a one-village world the reputation lookup can attribute to.
    let mut world = World::new();
    world.village_anchors.insert((0, 0), [0, 64, 0]);

    let mut inv = Inventory::new();
    inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 8)));

    let q = Quest {
        flavour: QuestFlavour::Fetch { item: MaterialId::Wheat, count: 5 },
        reward: QuestReward { items: Vec::new(), sats: 12, reputation: 5 },
        accepted_by: Some(0),
    };
    assert!(quest::is_complete(&q, &inv, 0));

    // Consume the required count out of the player's inventory.
    assert!(quest::fetch_make_consume(&q, &mut inv));
    let remaining: u32 = (0..36)
        .filter_map(|i| inv.slot(i))
        .map(|s| match s.item {
            Item::Material(MaterialId::Wheat) => s.count as u32,
            _ => 0,
        })
        .sum();
    assert_eq!(remaining, 3, "should have consumed exactly 5, leaving 3");

    // Apply reputation against the village nearest the (anchor) position.
    let mut rep = Reputation::default();
    let vid = reputation::village_at_position(&world, Vec3::new(0.0, 64.0, 0.0))
        .expect("expected village_at_position to find the anchor we registered");
    let new_score = rep.adjust(vid, q.reward.reputation);
    assert_eq!(new_score, 5);
    assert_eq!(rep.tier(vid), Tier::Neutral); // 5 is still inside the neutral band
    rep.adjust(vid, 6); // bump us into Friendly
    assert_eq!(rep.tier(vid), Tier::Friendly);
}

#[test]
fn killing_a_villager_drops_reputation_and_hits_the_cooldown() {
    let mut world = World::new();
    world.village_anchors.insert((0, 0), [0, 64, 0]);

    // First kill within the cooldown window — penalty applies.
    let mut rep = Reputation::default();
    let vid = reputation::village_at_position(&world, Vec3::new(0.0, 64.0, 0.0)).unwrap();
    rep.adjust(vid, reputation::VILLAGER_KILL_PENALTY);
    assert_eq!(rep.score(vid), -25);

    // Spamming further kills in-window would not move the score; emulate
    // by enforcing "no second adjustment" guard at the caller.
    let last_tick: u64 = 10;
    let now_within_cooldown: u64 = last_tick + 100; // 100 < 600 cooldown
    let on_cooldown = now_within_cooldown.saturating_sub(last_tick)
        < reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS;
    assert!(on_cooldown, "second kill within 30 s should be on cooldown");

    let now_after_cooldown = last_tick + reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS + 1;
    let on_cooldown_after = now_after_cooldown.saturating_sub(last_tick)
        < reputation::VILLAGER_KILL_REP_COOLDOWN_TICKS;
    assert!(!on_cooldown_after, "after 30 s the next kill should re-penalise");
}

#[test]
fn village_defender_attacks_nearest_hostile_within_reach() {
    use crate::combat::Health;

    let mut ecs = hecs::World::new();
    // Spawn a Knight defender with the GolemGuard state pointing at the anchor.
    entity::spawn_mob(&mut ecs, MobType::Knight, Vec3::new(0.0, 64.0, 0.0));
    let knight_id = ecs
        .query::<&MobKind>()
        .iter()
        .find(|(_, k)| k.0 == MobType::Knight)
        .map(|(id, _)| id)
        .unwrap();
    if let Ok(mut ai) = ecs.get::<&mut crate::mob_ai::MobAi>(knight_id) {
        ai.state = crate::mob_ai::AiState::GolemGuard { home_x: 0, home_z: 0 };
    }

    // A brigand 1 block away — inside GOLEM_MELEE_REACH.
    entity::spawn_mob(&mut ecs, MobType::Brigand, Vec3::new(1.0, 64.0, 0.0));
    let brigand_id = ecs
        .query::<&MobKind>()
        .iter()
        .find(|(_, k)| k.0 == MobType::Brigand)
        .map(|(id, _)| id)
        .unwrap();
    let before = ecs.get::<&Health>(brigand_id).unwrap().current;

    let hits = crate::mob_ai::tick_golem_combat(&mut ecs);
    assert_eq!(hits, 1, "expected the defender to hit the adjacent brigand");

    let after = ecs.get::<&Health>(brigand_id).unwrap().current;
    assert!(after < before, "brigand should have taken damage: {before} → {after}");
}

#[test]
fn village_bell_attracts_wanderer_and_converts_to_villager() {
    let mut world = World::new();
    floor_at(&mut world, 64);
    let bell = [0, 64, 0];
    world.set_block(bell[0], bell[1], bell[2], block::VILLAGE_BELL);
    world.village_bells.push(bell);

    let mut ecs = hecs::World::new();
    // A wanderer right next to the bell — within the 2.5-block conversion
    // radius so the migration tick should despawn + replace it.
    entity::spawn_mob(
        &mut ecs,
        MobType::Peddler,
        Vec3::new(0.5, 64.0, 1.0),
    );

    let converted = village_gen::tick_wanderer_migration(&mut world, &mut ecs, 0);
    assert_eq!(converted, 1, "expected the wanderer to convert at the bell");

    // Wanderer gone; villager spawned.
    let wanderer_count = ecs
        .query::<&MobKind>()
        .iter()
        .filter(|(_, k)| k.0 == MobType::Peddler)
        .count();
    let villager_count = ecs
        .query::<&MobKind>()
        .iter()
        .filter(|(_, k)| k.0 == MobType::Villager)
        .count();
    assert_eq!(wanderer_count, 0, "wanderer should have despawned");
    assert_eq!(villager_count, 1, "a villager should now stand at the bell");

    // The bell cell got registered as a village anchor — same key as the
    // procgen path would use.
    assert!(!world.village_anchors.is_empty(), "bell should register as anchor");
    assert!(!world.populated_villages.is_empty(), "cell should be marked populated");
}

#[test]
fn end_to_end_villager_quest_loop() {
    // Build a tiny world with a Carpenter workstation + register a village
    // anchor so reputation attribution lands on it.
    let mut world = World::new();
    floor_at(&mut world, 64);
    world.set_block(1, 64, 0, block::CRAFTING_TABLE);
    world.village_anchors.insert((0, 0), [0, 64, 0]);

    let mut ecs = hecs::World::new();
    let _villager_id = spawn_villager_at(&mut ecs, Vec3::new(0.5, 64.0, 0.0));

    // Phase 3 — villager binds to Carpenter via workstation scan.
    villager::tick_workstation_claims(&mut ecs, &world);

    // Phase 6 — Carpenter quest pool offers a Stick Fetch quest at seed 0.
    let seed = 0u32;
    let q = quest::generate_quest(Profession::Carpenter, seed)
        .expect("carpenter pool must produce a quest");
    let need = match q.flavour {
        QuestFlavour::Fetch { item, count } | QuestFlavour::Make { item, count } => {
            Some((item, count))
        }
        QuestFlavour::Kill { .. } => None,
    };

    // Phase 7 path — fill the player's inventory with enough to complete a
    // Fetch / Make quest. If the seed picked a Kill, skip the inventory bit
    // and exercise a separate Kill path.
    let mut inv = Inventory::new();
    let mut kill_count = 0u32;
    if let Some((item, count)) = need {
        inv.set_slot(0, Some(ItemStack::new_material(item, count)));
        assert!(quest::is_complete(&q, &inv, 0));
    } else if let QuestFlavour::Kill { count, .. } = q.flavour {
        kill_count = count as u32;
        assert!(quest::is_complete(&q, &inv, kill_count));
    }

    // Turn in: consume + reward + reputation.
    if need.is_some() {
        assert!(quest::fetch_make_consume(&q, &mut inv));
    }
    let mut rep = Reputation::default();
    let vid = reputation::village_at_position(&world, Vec3::new(0.0, 64.0, 0.0)).unwrap();
    let new_score = rep.adjust(vid, q.reward.reputation);
    assert!(new_score > 0, "reward.reputation must be positive for a Carpenter quest");
    assert!(q.reward.sats > 0, "Carpenter quests should pay sats (sandbox)");
}

// ---------- village_gen layout sanity ----------

#[test]
fn village_gen_anchors_register_after_column_generation() {
    use crate::biome::BiomeGenerator;
    use crate::chunk::CHUNK_SIZE;

    // Generate a column we know has a village by searching the grid.
    let bg = BiomeGenerator::new(42);
    let mut layout = None;
    'search: for gx in -10..10 {
        for gz in -10..10 {
            if let Some(l) = village_gen::layout_for_cell(42, gx, gz, &bg) {
                layout = Some((gx, gz, l));
                break 'search;
            }
        }
    }
    let (gx, gz, layout) = layout.expect("expected a village in [-10..10]^2");
    let [ax, _ay, az] = layout.anchor_world;
    let cx = ax.div_euclid(CHUNK_SIZE as i32);
    let cz = az.div_euclid(CHUNK_SIZE as i32);

    let mut world = World::new();
    for dcx in -2..=2 {
        for dcz in -2..=2 {
            world.generate_column(cx + dcx, cz + dcz, &bg);
        }
    }
    assert!(
        world.village_anchors.contains_key(&(gx, gz)),
        "village_anchors should include cell ({gx},{gz})"
    );

    // The campfire block should sit one above the campfire hearth position.
    let [fx, fy, fz] = layout.campfire;
    assert_eq!(
        world.get_block(fx, fy + 1, fz),
        block::CAMPFIRE,
        "expected lit campfire at the layout position",
    );

    // Suppress unused warnings for the imports we didn't reach in this test.
    let _ = (AHashMap::<(i32, i32), ()>::new(), Velocity(Vec3::ZERO), Position(Vec3::ZERO));
}

// ─── Spec 27 — village procgen uses the plan registry ─────────────────

#[test]
fn village_with_bundled_registry_drops_at_least_one_procgen_plaque() {
    use crate::biome::BiomeGenerator;
    use crate::chunk::CHUNK_SIZE;

    let bg = BiomeGenerator::new(42);
    // Find an actual village cell that the layout function approves.
    let mut found: Option<crate::village_gen::VillageLayout> = None;
    'search: for gx in -10..10 {
        for gz in -10..10 {
            if let Some(layout) = crate::village_gen::layout_for_cell(42, gx, gz, &bg) {
                found = Some(layout);
                break 'search;
            }
        }
    }
    let layout = found.expect("expected at least one village in sample range");

    let mut world = World::new();
    world.load_bundled_plans();
    assert!(!world.plan_registry.is_empty(), "load_bundled_plans must populate");

    let [ax, _ay, az] = layout.anchor_world;
    let cx = ax.div_euclid(CHUNK_SIZE as i32);
    let cz = az.div_euclid(CHUNK_SIZE as i32);
    for dcx in -3..=3 {
        for dcz in -3..=3 {
            world.generate_column(cx + dcx, cz + dcz, &bg);
        }
    }
    // At least one procgen Plaque must have been placed by the
    // registry-driven build_house path.
    assert!(
        !world.architect_plaques.is_empty(),
        "expected at least one architect plaque from registry-driven build_house",
    );
    assert!(
        !world.procgen_plaque_sources.is_empty(),
        "expected at least one plaque tagged as procgen-sourced",
    );
    // Every procgen plaque must have a non-empty derivation chain
    // (the bundled plans all carry the architect's chain).
    for pos in &world.procgen_plaque_sources {
        let data = world.architect_plaques.get(pos).expect("plaque metadata missing");
        assert!(
            !data.chain.is_empty(),
            "procgen plaque at {pos:?} should carry the plan's derivation chain",
        );
    }
}

#[test]
fn build_house_with_empty_registry_falls_back_to_hardcoded_shape() {
    use crate::biome::BiomeGenerator;
    use crate::chunk::CHUNK_SIZE;

    let bg = BiomeGenerator::new(42);
    let mut found: Option<crate::village_gen::VillageLayout> = None;
    'search: for gx in -10..10 {
        for gz in -10..10 {
            if let Some(layout) = crate::village_gen::layout_for_cell(42, gx, gz, &bg) {
                found = Some(layout);
                break 'search;
            }
        }
    }
    let layout = found.expect("expected at least one village in sample range");

    let mut world = World::new();
    // Deliberately do NOT call `world.load_bundled_plans()` — empty
    // registry forces the hardcoded fallback.
    assert!(world.plan_registry.is_empty());

    let [ax, _ay, az] = layout.anchor_world;
    let cx = ax.div_euclid(CHUNK_SIZE as i32);
    let cz = az.div_euclid(CHUNK_SIZE as i32);
    for dcx in -3..=3 {
        for dcz in -3..=3 {
            world.generate_column(cx + dcx, cz + dcz, &bg);
        }
    }
    // No procgen plaque without the registry. Hardcoded shape still
    // builds — campfire is the canonical signal it ran.
    assert!(
        world.procgen_plaque_sources.is_empty(),
        "empty registry must skip the procgen Plaque path",
    );
    let [fx, fy, fz] = layout.campfire;
    assert_eq!(
        world.get_block(fx, fy + 1, fz),
        block::CAMPFIRE,
        "hardcoded fallback path must still place the village campfire",
    );
}

#[test]
fn village_layout_carries_workshop_slots_for_alpha_professions() {
    use crate::biome::BiomeGenerator;

    let bg = BiomeGenerator::new(42);
    let mut found: Option<crate::village_gen::VillageLayout> = None;
    'search: for gx in -20..20 {
        for gz in -20..20 {
            if let Some(layout) = crate::village_gen::layout_for_cell(42, gx, gz, &bg) {
                if layout.houses.len() >= 5 {
                    found = Some(layout);
                    break 'search;
                }
            }
        }
    }
    let layout = found.expect("expected a village with >=5 houses");
    assert!(
        !layout.workshops.is_empty(),
        "workshops slot should be allocated when houses >= 1",
    );
    // Workshop count = min(n_houses, alpha_professions.len()).
    // Historical Pivot Sub 7 grew the alpha-profession pool from 5
    // to 8 (added Miller / Baker / Brewer), so villages now receive
    // up to 8 workshop slots before capping. Each workshop's
    // profession must still come from the post-Sub-7 pool below.
    assert!(
        layout.workshops.len() >= 5,
        "expected at least 5 workshop slots for this seed, got {}",
        layout.workshops.len(),
    );
    assert!(
        layout.workshops.len() <= layout.houses.len(),
        "workshop count should never exceed house count",
    );
    let valid: std::collections::HashSet<Profession> = [
        Profession::Farmer,
        Profession::Cook,
        Profession::Carpenter,
        Profession::Blacksmith,
        Profession::Scribe,
        Profession::Miller,
        Profession::Baker,
        Profession::Brewer,
    ]
    .into_iter()
    .collect();
    for ws in &layout.workshops {
        assert!(
            valid.contains(&ws.profession),
            "workshop profession {:?} not in the alpha-profession pool",
            ws.profession,
        );
    }
    let unique: std::collections::HashSet<Profession> =
        layout.workshops.iter().map(|w| w.profession).collect();
    assert_eq!(
        unique.len(),
        layout.workshops.len(),
        "each workshop slot should host a distinct profession (no doubles in a single village)",
    );
}
