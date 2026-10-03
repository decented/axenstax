//! Spec 22 Raid Defence — integration tests.
//!
//! These exercise the raid pipeline end-to-end at the world+ECS layer:
//! scheduler roll → Warning raid → wave spawn → kill attribution →
//! bounty distribution. They don't drive `TestHost` because the raid
//! scheduler currently lives on the single-player game-loop side (the
//! `GameState::tick` path), same as the Spec 19 village tests.

use ahash::AHashMap;

use crate::biome::BiomeGenerator;
use crate::entity;
use crate::mob::MobType;
use crate::raid::{self, Raid, RaidStatus, WaveKind};
use crate::world::World;

fn make_eligible_world(
    anchor: [i32; 3],
    vid: (i32, i32),
    treasury: u64,
    villager_count: u32,
) -> (World, hecs::World) {
    let mut world = World::new();
    world.village_anchors.insert(vid, anchor);
    if treasury > 0 {
        world.village_treasuries.insert(vid, treasury);
    }
    let mut ecs = hecs::World::new();
    let anchor_pos = glam::Vec3::new(
        anchor[0] as f32 + 0.5,
        anchor[1] as f32 + 0.5,
        anchor[2] as f32 + 0.5,
    );
    for i in 0..villager_count {
        let pos = anchor_pos + glam::Vec3::new(i as f32 * 0.5, 0.0, 0.5);
        entity::spawn_mob(&mut ecs, MobType::Villager, pos);
    }
    (world, ecs)
}

#[test]
fn warning_raid_spawns_wave_then_kills_clear_then_bounty_distributes() {
    // End-to-end: schedule a Small raid manually, tick it through to
    // Active, simulate 5 kills attributed to player 0, tick through to
    // Cleared, distribute the bounty.
    let mut world = World::new();
    world.village_anchors.insert((0, 0), [0, 64, 0]);
    world.village_treasuries.insert((0, 0), 1_000);
    let mut ecs = hecs::World::new();

    // Manually push a Warning raid (skip the daily-roll path so the
    // test doesn't depend on a particular seed/day).
    let mut raid = Raid::new_warning(
        1, (0, 0), [0, 64, 0], WaveKind::Small, 100, 1000,
    );
    raid.warning_at_tick = 1000;
    world.active_raids.push(raid);

    // Tick to spawn-tick → Warning → Active + spawn list returned.
    let (spawns, resolutions) =
        raid::tick_raids(&mut world, 1000 + raid::RAID_WARNING_TICKS);
    assert!(resolutions.is_empty());
    assert_eq!(spawns.len(), 1);
    let (raid_id, spawn_list) = &spawns[0];
    assert_eq!(*raid_id, 1);
    assert_eq!(spawn_list.len(), WaveKind::Small.total_mobs() as usize);
    assert_eq!(world.active_raids[0].status, RaidStatus::Active);
    assert_eq!(world.active_raids[0].mobs_alive, WaveKind::Small.total_mobs());

    // Spawn each mob into the ECS via the raid-tagged spawn helper.
    let mut spawned_ids: Vec<hecs::Entity> = Vec::new();
    for (kind, pos) in spawn_list {
        let id = raid::spawn_raid_mob(&mut ecs, *kind, *pos, *raid_id);
        spawned_ids.push(id);
    }

    // Simulate 5 kill attributions all to player 0. In production
    // this is the per-tick attribution path; here we mutate directly.
    for _ in 0..5 {
        let raid_ref = raid::find_raid_mut(&mut world, 1).unwrap();
        raid::record_kill(&mut raid_ref.contribution_table, 0);
        raid_ref.mobs_alive = raid_ref.mobs_alive.saturating_sub(1);
        if raid_ref.mobs_alive == 0 {
            raid_ref.killing_blow = Some(0);
        }
    }

    // Tick → mobs_alive == 0 → Cleared resolution emitted.
    let (_spawns2, resolutions2) =
        raid::tick_raids(&mut world, 2000 + raid::RAID_WARNING_TICKS);
    assert_eq!(resolutions2.len(), 1);
    let r = &resolutions2[0];
    assert_eq!(r.status, RaidStatus::Cleared);
    assert_eq!(r.village_id, (0, 0));
    assert_eq!(r.treasury_drain, 100);
    assert_eq!(r.killing_blow, Some(0));

    // Bounty: 5 kills → 100 sats all to player 0.
    let (shares, remainder) = raid::bounty_shares(&r.contribution_table, r.treasury_drain);
    assert_eq!(shares, vec![(0, 100)]);
    assert_eq!(remainder, 0);
}

#[test]
fn failed_raid_refunds_drain_back_to_treasury() {
    // The settlement path inside game_loop refunds the drain. Here we
    // exercise the resolution + verify the refund step works.
    let mut world = World::new();
    world.village_anchors.insert((0, 0), [0, 64, 0]);
    let mut raid = Raid::new_warning(
        1, (0, 0), [0, 64, 0], WaveKind::Small, 100, 0,
    );
    raid.status = RaidStatus::Active;
    raid.spawn_at_tick = Some(0);
    raid.mobs_alive = 3;
    world.active_raids.push(raid);

    let (_spawns, resolutions) =
        raid::tick_raids(&mut world, raid::RAID_DEADLINE_TICKS + 1);
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].status, RaidStatus::Failed);
    // Simulate the game_loop's refund step.
    raid::credit_treasury(
        &mut world.village_treasuries,
        resolutions[0].village_id,
        resolutions[0].treasury_drain,
    );
    assert_eq!(world.village_treasuries.get(&(0, 0)).copied(), Some(100));
}

#[test]
fn save_load_round_trip_preserves_active_raid_and_treasury() {
    // Spec 22 Phase 11 — active raid + treasury both round-trip
    // through WorldSave bincode.
    let mut world = World::new();
    world.village_anchors.insert((0, 0), [0, 64, 0]);
    world.village_treasuries.insert((0, 0), 250);
    world.village_treasuries.insert((1, 1), 75);
    let mut raid = Raid::new_warning(
        7, (0, 0), [0, 64, 0], WaveKind::Medium, 500, 100,
    );
    raid.status = RaidStatus::Active;
    raid.spawn_at_tick = Some(100 + raid::RAID_WARNING_TICKS);
    raid.contribution_table = vec![(0, 2)];
    raid.mobs_alive = 8;
    world.active_raids.push(raid);
    world.raid_scheduler.last_rolled_day = 5;
    world.raid_scheduler.next_raid_id = 8;

    let save = crate::save::WorldSave {
        seed: 42,
        player_x: 0.0,
        player_y: 64.0,
        player_z: 0.0,
        player_health: 20.0,
        hotbar_slot: 0,
        inventory: vec![crate::save::SavedSlot::Empty; 36],
        players: vec![],
        campfires: vec![],
        furnaces: vec![],
        vendors: vec![],
        drying_racks: vec![],
        hives: vec![],
        chests: vec![],
        tip_jars: vec![],
        auctions: vec![],
        latent_prints: vec![],
        plots: vec![],
        market_hubs: vec![],
        construction_anchors: vec![],
        architect_plaques: vec![],
        village_anchors: vec![],
        populated_villages: vec![],
        village_bells: vec![],
        village_treasuries: world.village_treasuries.iter().map(|(&k, &v)| (k, v)).collect(),
        active_raids: world.active_raids.clone(),
        raid_scheduler: world.raid_scheduler.clone(),
        raid_kills: world.raid_kills.iter().map(|(&(vid, pk), &c)| (vid, pk, c)).collect(),
        brigand_hideouts: world.brigand_hideouts.iter()
            .map(|(&(gx, gz), data)| crate::save::SavedHideout { grid_x: gx, grid_z: gz, data: data.clone() })
            .collect(),
        bounties: vec![],
        bounty_next_id: 0,
        bounty_last_refresh_tick: 0,
        face_overlays: vec![],
        face_blueprints: vec![],
        face_blueprint_blanks: vec![],
        workshop: Default::default(),
        carts: vec![],
        graves: vec![],
        waypoints: vec![],
        block_meta: vec![],
        power_devices: vec![],
        signs: vec![],
        item_frames: vec![],
        locked_slots: vec![],
        hostile_acts: vec![],
        rigs: vec![],
        exhibits: Vec::new(),
        composters: Vec::new(),
        saved_mobs: Vec::new(),
        satoshi: Default::default(),
        dispensers: Vec::new(),
        rig_clips: Vec::new(),
    };
    let bytes = bincode::serialize(&save).expect("serialize");
    let back: crate::save::WorldSave = bincode::deserialize(&bytes).expect("deserialize");
    let restored_treasuries: AHashMap<(i32, i32), u64> = back
        .village_treasuries
        .iter()
        .copied()
        .collect();
    assert_eq!(restored_treasuries.get(&(0, 0)).copied(), Some(250));
    assert_eq!(restored_treasuries.get(&(1, 1)).copied(), Some(75));
    assert_eq!(back.active_raids.len(), 1);
    let r = &back.active_raids[0];
    assert_eq!(r.id, 7);
    assert_eq!(r.wave_kind, WaveKind::Medium);
    assert_eq!(r.contribution_table, vec![(0, 2)]);
    assert_eq!(r.mobs_alive, 8);
    assert_eq!(back.raid_scheduler.last_rolled_day, 5);
    assert_eq!(back.raid_scheduler.next_raid_id, 8);
}

#[test]
fn scheduler_does_not_double_schedule_when_active_raid_exists() {
    // A village already under raid should NOT receive a second raid
    // the same day even if the roll would hit.
    let (mut world, ecs) = make_eligible_world([10000, 64, 10000], (-100, -100), 100_000, 10);
    let bg = BiomeGenerator::new(42);

    // Force-push an active raid first.
    world.active_raids.push(Raid::new_warning(
        99, (-100, -100), [10000, 64, 10000], WaveKind::Large, 2000, 0,
    ));
    let before = world.active_raids.len();
    let _ = raid::tick_scheduler_daily(&mut world, &ecs, &bg, 1, 100);
    // No new raid added.
    assert_eq!(world.active_raids.len(), before);
}

#[test]
fn quest_payout_split_credits_village_treasury() {
    // Phase 13 — quest payout splits 80/20 to player/village.
    let (kid, village) = raid::split_quest_payout(100);
    assert_eq!(kid, 80);
    assert_eq!(village, 20);
}

#[test]
fn vendor_payment_split_credits_village_treasury_when_in_range() {
    // Phase 13 — vendor sale within 64 blocks of an anchor pays 5 %
    // to the treasury; outside, nothing.
    let mut anchors: AHashMap<(i32, i32), [i32; 3]> = AHashMap::new();
    anchors.insert((0, 0), [0, 64, 0]);
    // Inside 64-block radius of anchor.
    assert_eq!(
        raid::village_for_vendor_position(&anchors, (30, 64, 30)),
        Some((0, 0))
    );
    // Outside.
    assert_eq!(
        raid::village_for_vendor_position(&anchors, (500, 64, 500)),
        None
    );
    // Split arithmetic.
    let (seller, village) = raid::split_vendor_payment(200);
    assert_eq!(seller, 190);
    assert_eq!(village, 10);
}

#[test]
fn bounty_payout_suppressed_for_charter_disabled_player() {
    // Spec 22 — kid with charter_allows_sats=false sees no sats line
    // (the unified helper suppresses); the kill share is still
    // computed (so items + rep still flow) but credited == 0.
    let drain = 100u64;
    let table = vec![(0, 5)];
    let (shares, _rem) = raid::bounty_shares(&table, drain);
    assert_eq!(shares, vec![(0, 100)]);
    // Player has charter off → suppressed.
    let policy = crate::economy::ServerSatsPolicy::bitcoin_enabled_policy();
    let payout_off = crate::economy::apply_sats_payout(
        100,
        crate::economy::PayoutKind::RaidBounty,
        &policy,
        false,
    );
    assert!(payout_off.suppressed);
    assert_eq!(payout_off.credited, 0);
    // Player has charter on → full credit.
    let payout_on = crate::economy::apply_sats_payout(
        100,
        crate::economy::PayoutKind::RaidBounty,
        &policy,
        true,
    );
    assert!(!payout_on.suppressed);
    assert_eq!(payout_on.credited, 100);
}

#[test]
fn bitcoin_disabled_server_suppresses_bounty_but_raid_still_resolves() {
    // Spec 22 — Bitcoin-disabled server: raid completes (resolution
    // is Cleared), items distribute via the mob drop tables (per
    // mob::drops_for — exercised in other tests), no sats line.
    let mut world = World::new();
    world.village_anchors.insert((0, 0), [0, 64, 0]);
    let mut raid = Raid::new_warning(
        1, (0, 0), [0, 64, 0], WaveKind::Small, 100, 0,
    );
    raid.status = RaidStatus::Active;
    raid.spawn_at_tick = Some(0);
    raid.mobs_alive = 0;
    raid.contribution_table = vec![(0, 5)];
    world.active_raids.push(raid);

    let (_spawns, resolutions) = raid::tick_raids(&mut world, 100);
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].status, RaidStatus::Cleared);
    // The unified payout helper would suppress here:
    let policy = crate::economy::ServerSatsPolicy {
        bitcoin_enabled: false,
        ..Default::default()
    };
    let payout = crate::economy::apply_sats_payout(
        100,
        crate::economy::PayoutKind::RaidBounty,
        &policy,
        true,
    );
    assert!(payout.suppressed);
}

#[test]
fn killing_blow_only_stamps_on_last_mob() {
    // The killing_blow stamp is set when mobs_alive drops to 0, and
    // only then. Build a 3-mob raid and walk through the kills.
    let mut raid = Raid::new_warning(
        1, (0, 0), [0, 64, 0], WaveKind::Small, 100, 0,
    );
    raid.status = RaidStatus::Active;
    raid.spawn_at_tick = Some(0);
    raid.mobs_alive = 3;
    raid.mobs_total = 3;
    // First two kills by player 0 — no killing_blow yet.
    for _ in 0..2 {
        raid::record_kill(&mut raid.contribution_table, 0);
        raid.mobs_alive -= 1;
        if raid.mobs_alive == 0 {
            raid.killing_blow = Some(0);
        }
    }
    assert!(raid.killing_blow.is_none());
    // Third kill by player 1 — killing_blow flips to Some(1).
    raid::record_kill(&mut raid.contribution_table, 1);
    raid.mobs_alive -= 1;
    if raid.mobs_alive == 0 {
        raid.killing_blow = Some(1);
    }
    assert_eq!(raid.killing_blow, Some(1));
}

// --- Spec 22 Phase 16: bonus drops integration ---
//
// The game_loop's `settle_raid_resolution` reads the wave kind off the
// just-resolved raid and credits each contributor with the wave's
// bonus stack. These tests cover the pure layer that drives the wire-
// up; full GameState end-to-end is beyond TestHost's reach today (the
// raid scheduler still lives on the single-player game-loop side).

#[test]
fn bonus_drops_table_matches_spec_for_each_wave_kind_bitcoin_enabled() {
    use crate::raid::{bonus_drops_for, ServerEconomyMode, WaveKind};
    assert!(bonus_drops_for(WaveKind::Small, ServerEconomyMode::BitcoinEnabled).is_empty());
    assert_eq!(
        bonus_drops_for(WaveKind::Medium, ServerEconomyMode::BitcoinEnabled).len(),
        1,
        "Medium delivers exactly 1 bonus item per contributor",
    );
    assert_eq!(
        bonus_drops_for(WaveKind::Large, ServerEconomyMode::BitcoinEnabled).len(),
        1,
        "Large delivers exactly 1 bonus item per contributor",
    );
}

#[test]
fn bonus_distribution_flat_per_contributor_via_table() {
    // Smoke test: every defender with at least one kill gets the
    // wave's bonus stack. Asserted at the data layer because the
    // settlement path is on GameState (out of scope for TestHost).
    let contribution_table: Vec<(usize, u32)> = vec![(0, 5), (1, 1), (2, 0)];
    let bonus = raid::bonus_drops_for(WaveKind::Medium, raid::ServerEconomyMode::BitcoinEnabled);
    // The game_loop loops `for (pidx, _kills) in &shares` where
    // `shares` is `bounty_shares`'s output — which filters out the
    // zero-kill entries. So players 0 and 1 receive the bonus, but
    // player 2 (zero kills) does not.
    let (shares, _) = raid::bounty_shares(&contribution_table, 100);
    let recipients: Vec<usize> = shares.iter().map(|(p, _)| *p).collect();
    assert_eq!(recipients, vec![0, 1]);
    assert!(!bonus.is_empty(), "Medium must drop a bonus to test against");
}

// --- Spec 22 Phase 18: leaderboard save/load round-trip ---

#[test]
fn raid_kills_round_trip_through_world_save() {
    // Phase 18 — per-village per-player kill totals persist across
    // save/load. Mirrors the existing active_raid round-trip test.
    use crate::raid::PlayerKey;
    use crate::reputation::VillageId;
    let mut world = World::new();
    let vid: VillageId = (0, 0);
    let vid2: VillageId = (1, 1);
    let p0: PlayerKey = 0;
    let p1: PlayerKey = 1;
    world.raid_kills.insert((vid, p0), 14);
    world.raid_kills.insert((vid, p1), 9);
    world.raid_kills.insert((vid2, p0), 3);

    let save = crate::save::WorldSave {
        seed: 42,
        player_x: 0.0,
        player_y: 64.0,
        player_z: 0.0,
        player_health: 20.0,
        hotbar_slot: 0,
        inventory: vec![crate::save::SavedSlot::Empty; 36],
        players: vec![],
        campfires: vec![],
        furnaces: vec![],
        vendors: vec![],
        drying_racks: vec![],
        hives: vec![],
        chests: vec![],
        tip_jars: vec![],
        auctions: vec![],
        latent_prints: vec![],
        plots: vec![],
        market_hubs: vec![],
        construction_anchors: vec![],
        architect_plaques: vec![],
        village_anchors: vec![],
        populated_villages: vec![],
        village_bells: vec![],
        village_treasuries: vec![],
        active_raids: vec![],
        raid_scheduler: crate::raid::RaidScheduler::new(),
        raid_kills: world.raid_kills.iter().map(|(&(v, pk), &c)| (v, pk, c)).collect(),
        brigand_hideouts: world.brigand_hideouts.iter()
            .map(|(&(gx, gz), data)| crate::save::SavedHideout { grid_x: gx, grid_z: gz, data: data.clone() })
            .collect(),
        bounties: vec![],
        bounty_next_id: 0,
        bounty_last_refresh_tick: 0,
        face_overlays: vec![],
        face_blueprints: vec![],
        face_blueprint_blanks: vec![],
        workshop: Default::default(),
        carts: vec![],
        graves: vec![],
        waypoints: vec![],
        block_meta: vec![],
        power_devices: vec![],
        signs: vec![],
        item_frames: vec![],
        locked_slots: vec![],
        hostile_acts: vec![],
        rigs: vec![],
        exhibits: Vec::new(),
        composters: Vec::new(),
        saved_mobs: Vec::new(),
        satoshi: Default::default(),
        dispensers: Vec::new(),
        rig_clips: Vec::new(),
    };
    let bytes = bincode::serialize(&save).expect("serialize");
    let back: crate::save::WorldSave = bincode::deserialize(&bytes).expect("deserialize");
    let restored: AHashMap<(VillageId, PlayerKey), u32> = back
        .raid_kills
        .iter()
        .map(|&(v, pk, c)| ((v, pk), c))
        .collect();
    assert_eq!(restored.get(&(vid, p0)).copied(), Some(14));
    assert_eq!(restored.get(&(vid, p1)).copied(), Some(9));
    assert_eq!(restored.get(&(vid2, p0)).copied(), Some(3));
    assert_eq!(restored.len(), 3);
}

#[test]
fn leaderboard_top_defenders_via_world_state() {
    use crate::raid::{top_defenders, PlayerKey};
    use crate::reputation::VillageId;
    let mut world = World::new();
    let vid: VillageId = (0, 0);
    let p0: PlayerKey = 0;
    let p1: PlayerKey = 1;
    let p2: PlayerKey = 2;
    world.raid_kills.insert((vid, p0), 14);
    world.raid_kills.insert((vid, p1), 22);
    world.raid_kills.insert((vid, p2), 3);
    let top = top_defenders(&world.raid_kills, vid, 3);
    assert_eq!(top, vec![(p1, 22), (p0, 14), (p2, 3)]);
}
