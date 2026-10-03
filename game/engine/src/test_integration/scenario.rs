//! Goal 1 — scenario runner + per-world stats integration.
//!
//! Two surfaces are covered here:
//!   * The **per-world stats** (`total_work` / `total_ticks`) live on `World`
//!     and are reachable through `TestHost` (which drives the real
//!     `GameServer`) — exercised end-to-end below.
//!   * The **scenario runner** itself lives on `GameState` (the client), so its
//!     per-tick / score / objective logic is driven directly on `ScenarioState`
//!     with REAL `block_work` values (TestHost drives the server, not a
//!     GameState). Together these prove "work accrues + timer ends".

use crate::block;
use crate::crafting::block_work;
use crate::item::MaterialId;
use crate::scenario::{Objective, ScenarioDef, ScenarioKind, ScenarioState, ScenarioTuning, Scoring};
use crate::test_harness::{TestConfig, TestHost};

#[test]
fn world_clock_accrues_through_server_tick() {
    // The world-clock (total_ticks) advances once per real GameServer tick —
    // the production accrual GameState::tick mirrors for single-player.
    let mut host = TestHost::start_with(TestConfig::default());
    assert_eq!(host.server.world.total_ticks, 0);
    host.tick(50);
    assert_eq!(host.server.world.total_ticks, 50);
}

#[test]
fn mining_a_harvestable_block_accrues_world_work() {
    // Bare-handed dirt IS harvestable → its block_work tallies into total_work,
    // through the real can_harvest + block_work + World::add_work path.
    let mut host = TestHost::start_with(TestConfig::default());
    host.set_block(0, 50, 0, block::DIRT);
    let added = host.mine_block(0, 50, 0);
    assert_eq!(added, block_work(block::DIRT));
    assert_eq!(host.server.world.total_work, block_work(block::DIRT));
    assert_eq!(host.get_block(0, 50, 0), block::AIR);
}

#[test]
fn mining_unharvestable_block_bare_handed_does_no_work() {
    // Bare hands can't harvest stone → the block breaks but NO work accrues
    // (work-based-hashing: "you simply can't do it, so no hashes").
    let mut host = TestHost::start_with(TestConfig::default());
    host.set_block(0, 50, 0, block::STONE);
    let added = host.mine_block(0, 50, 0);
    assert_eq!(added, 0);
    assert_eq!(host.server.world.total_work, 0);
}

#[test]
fn re_mining_a_player_placed_block_earns_no_work() {
    // Spec 06 §2.2 anti-farming — the reported Hash Dash cheese. A natural
    // block earns work; the SAME block placed by the player earns none when
    // re-mined. The block still breaks and the item is recoverable — only the
    // proof-of-play work/hash credit is withheld.
    let mut host = TestHost::start_with(TestConfig::default());

    host.set_block(0, 50, 0, block::DIRT); // natural
    assert_eq!(host.mine_block(0, 50, 0), block_work(block::DIRT));
    let baseline = host.server.world.total_work;

    host.place_block(1, 50, 0, block::DIRT); // player-placed
    assert!(host.server.world.is_placed(1, 50, 0));
    assert_eq!(
        host.mine_block(1, 50, 0),
        0,
        "re-mining a player-placed block must earn no work"
    );
    assert_eq!(
        host.server.world.total_work, baseline,
        "world work must not move for a placed-block break"
    );
    assert_eq!(host.get_block(1, 50, 0), block::AIR, "the block still breaks");
}

#[test]
fn place_break_loop_cannot_farm_work() {
    // Chop / replace / re-break the same cell ten times — the boring win the
    // playtest flagged. Total work must stay at zero.
    let mut host = TestHost::start_with(TestConfig::default());
    for _ in 0..10 {
        host.place_block(2, 50, 0, block::DIRT);
        assert_eq!(host.mine_block(2, 50, 0), 0);
    }
    assert_eq!(
        host.server.world.total_work, 0,
        "a place→break loop must not accrue any work"
    );
}

#[test]
fn breaking_clears_the_placed_flag_so_natural_refill_earns_work() {
    // After a placed block is mined the cell is natural again: a *natural*
    // block that later occupies it earns work normally (no stale exclusion).
    let mut host = TestHost::start_with(TestConfig::default());
    host.place_block(3, 50, 0, block::DIRT);
    host.mine_block(3, 50, 0);
    assert!(
        !host.server.world.is_placed(3, 50, 0),
        "breaking must clear the placed flag"
    );
    host.set_block(3, 50, 0, block::DIRT); // natural refill
    assert_eq!(
        host.mine_block(3, 50, 0),
        block_work(block::DIRT),
        "a natural block in a previously-placed cell earns work"
    );
}

fn timed_work_def(ticks: u32) -> ScenarioDef {
    ScenarioDef {
        kind: ScenarioKind::Test,
        display_name: "Integration".to_string(),
        lock_creative: false,
        arena_mode: crate::scenario::ArenaMode::Reuse,
        kit: vec![],
        objective: Objective::Timed { ticks },
        scoring: Scoring::Work,
        arena_seed: None,
        tuning: ScenarioTuning::default(),
        world_type: None,
        game_mode: None,
        time_lock: None,
        weather_lock: None,
        mobs_enabled: None,
        trial_race: None,
        arena: None,
    }
}

#[test]
fn scenario_work_score_and_timer_end_together() {
    // The runner's headline behaviour: in a timed work score-attack, mining
    // accrues score and the timer ends the run — with REAL block_work values.
    let mut s = ScenarioState::new(timed_work_def(5));
    let expected =
        block_work(block::STONE) + block_work(block::DIRT) + block_work(block::OAK_LEAVES);
    s.on_block_broken(block_work(block::STONE));
    s.on_block_broken(block_work(block::DIRT));
    s.on_block_broken(block_work(block::OAK_LEAVES));
    assert_eq!(s.score(), expected);
    assert!(expected > 0, "mining real blocks must produce a positive score");

    for _ in 0..4 {
        assert!(!s.tick(), "must not end before the timer");
    }
    assert!(s.tick(), "5th tick ends the timed run");
    assert!(s.is_ended());

    // Post-end mining is ignored — the score is frozen at the final total.
    s.on_block_broken(block_work(block::STONE));
    assert_eq!(s.score(), expected);
}

#[test]
fn satori_rush_scenario_ends_on_first_satori_with_world_clock_result() {
    // The Satori-Rush shape: untimed, ends on the first Satori, result tick is
    // the elapsed world-clock (the speedrun time).
    let def = ScenarioDef {
        kind: ScenarioKind::SatoriRush,
        display_name: "Satori Rush".to_string(),
        lock_creative: false,
        arena_mode: crate::scenario::ArenaMode::Reuse,
        kit: vec![],
        objective: Objective::FirstSatori,
        scoring: Scoring::None,
        arena_seed: None,
        tuning: ScenarioTuning::default(),
        world_type: None,
        game_mode: None,
        time_lock: None,
        weather_lock: None,
        mobs_enabled: None,
        trial_race: None,
        arena: None,
    };
    let mut s = ScenarioState::new(def);
    for _ in 0..123 {
        s.tick();
    }
    assert!(!s.is_ended(), "untimed run does not end on its own");
    s.on_material_gained(MaterialId::Satori);
    assert!(s.is_ended(), "first Satori ends the run");
    assert_eq!(s.result_tick(), Some(123), "result is the world-clock at genesis");
}

#[test]
fn satori_rush_resumes_from_persisted_world_meta() {
    // Goal 4 local resume: a Satori Rush world persists its def + world-clock to
    // WorldMeta; after a save→load round-trip the run reconstructs with the clock
    // continuing — exactly what the game-loop load path does.
    let mut meta = crate::save::WorldMeta::new("satori-run");
    meta.scenario_def = Some(crate::scenario::satori_rush_def().to_json().unwrap());
    meta.total_ticks = 2400; // 2 minutes in, genesis not yet found

    let bytes = bincode::serialize(&meta).unwrap();
    let loaded: crate::save::WorldMeta = bincode::deserialize(&bytes).unwrap();

    let def = crate::scenario::load_scenario_def(loaded.scenario_def.unwrap().as_bytes()).unwrap();
    let resumed = ScenarioState::resume(
        def,
        loaded.total_ticks as u32,
        loaded.genesis_block_found,
        loaded.genesis_found_at_tick.map(|t| t as u32),
    );
    assert!(!resumed.is_ended(), "in-progress run resumes active");
    assert_eq!(resumed.elapsed_ticks(), 2400, "clock continues from the world-clock");
    assert_eq!(resumed.kind(), ScenarioKind::SatoriRush);
}
