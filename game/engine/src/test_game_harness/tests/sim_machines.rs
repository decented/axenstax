//! Machine authority through the actual standalone and local-hosted game.
use super::*;
use crate::{block, local_routing::LocalRouting};

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_d4_machines_share_cadence_and_drain_before_the_world_window() {
    isolate_saves();
    for route in [LocalRouting::Standalone, LocalRouting::SoloHosted] {
        let name = format!("d4-machines-{route:?}-{:?}", std::thread::current().id());
        let mut game = HeadlessGame::boot_into_world_routed(&name, route);
        game.state.ecs.clear();
        let at = game.state.players[0].player.pos.as_ivec3();
        let fire = (at.x + 3, at.y + 2, at.z);
        let rack = (at.x + 5, at.y + 2, at.z);
        let furnace = (at.x + 7, at.y + 2, at.z);
        game.state
            .world
            .set_block(fire.0, fire.1, fire.2, block::CAMPFIRE);
        game.state.world.insert_campfire(
            fire,
            crate::campfire::CampfireData {
                fuel_ticks: 2,
                ..Default::default()
            },
        );
        game.state
            .world
            .set_block(rack.0, rack.1, rack.2, block::DRYING_RACK);
        game.state
            .world
            .set_block(rack.0, rack.1 + 1, rack.2, block::AIR);
        let mut data = crate::drying_rack::DryingRackData::default();
        assert!(data.try_place_green(crate::drying_rack::LogSpecies::Oak));
        game.state.world.drying_racks.insert(rack, data);
        game.state
            .world
            .set_block(furnace.0, furnace.1, furnace.2, block::FURNACE);
        game.state.world.insert_furnace(
            furnace,
            crate::furnace::FurnaceData {
                input: Some(crate::item::ItemStack::new_material(
                    crate::item::MaterialId::Copper,
                    1,
                )),
                smelt_progress: 199,
                smelt_total: 200,
                fuel_ticks_remaining: 10,
                ..Default::default()
            },
        );
        // Host cadence can differ from a server created in mid-world. The
        // early authority must use the actual client phase, not server%4.
        game.state.falling_tick_counter = 3;
        let before = game.state.world.sim_tally;
        game.state.tick();
        assert_eq!(game.state.world.campfire_at(fire).unwrap().fuel_ticks, 1);
        assert_eq!(
            game.state.world.drying_racks[&rack].slots[0].seasoning_ticks,
            1
        );
        assert_eq!(
            game.state
                .world
                .furnace_at(furnace)
                .unwrap()
                .output
                .as_ref()
                .unwrap()
                .count,
            1
        );
        assert_eq!(
            game.state
                .world
                .sim_tally
                .get(crate::sim_lend::SimSystem::Machines),
            before.get(crate::sim_lend::SimSystem::Machines) + 1
        );
        if route == LocalRouting::SoloHosted {
            let hs = game.state.hosted_server.as_mut().unwrap();
            assert!(
                hs.server.sim_events.take().is_empty(),
                "local policy facts drain before the later world window"
            );
            let queued = hs
                .server
                .pending_block_changes
                .iter()
                .filter(|bc| (bc.x, bc.y, bc.z) == furnace && bc.new_block == block::FURNACE_LIT)
                .count();
            assert_eq!(
                queued, 0,
                "machine results do not enter the power transition channel"
            );
            assert!(
                hs.queued_broadcasts() >= 1,
                "already-applied changes are queued for peers"
            );
            assert!(
                !game
                    .state
                    .pending_block_changes
                    .iter()
                    .any(|bc| (bc.x, bc.y, bc.z) == furnace),
                "authority result is not replayed as a client edit"
            );
            game.state.network_send_input();
            assert!(game.state.tick_hosted_server(before).faults.is_empty());
            game.state.finish_reputation_day_tick();
        }
        assert!(
            game.logical_ticks(3)
                .iter()
                .all(|report| report.faults.is_empty())
        );
        assert_eq!(
            game.state.world.campfire_at(fire).unwrap().fuel_ticks,
            1,
            "one pass across four logical ticks"
        );
        assert_eq!(
            game.state.world.drying_racks[&rack].slots[0].seasoning_ticks,
            1
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_d4_machines_local_blast_lands_once_and_updates_authority_shadow() {
    isolate_saves();
    for route in [LocalRouting::Standalone, LocalRouting::SoloHosted] {
        let name = format!(
            "d4-machine-blast-{route:?}-{:?}",
            std::thread::current().id()
        );
        let mut game = HeadlessGame::boot_into_world_routed(&name, route);
        game.state.ecs.clear();
        game.state.play_mode = crate::play_mode::PlayMode::Survival;
        game.state.is_creative = false;
        game.state.explosives_enabled = true;
        let at = game.state.players[0].player.pos.as_ivec3();
        let keg = (at.x + 2, at.y, at.z);
        game.state
            .world
            .set_block(keg.0, keg.1, keg.2, block::BLASTING_KEG);
        let mut data = crate::power::PowerDeviceData::new(
            crate::power::PowerDeviceKind::BlastingKeg,
            crate::meta::Facing::Up,
        );
        data.charge = 1;
        game.state.world.insert_power_device(keg, data);
        let health = game.state.players[0].combat.health;
        game.state.falling_tick_counter = 3;
        let before = game.state.world.sim_tally;
        game.state.tick();
        let after = game.state.players[0].combat.health;
        assert!(
            after < health,
            "the authoritative blast reaches the actual local PlayerSlot"
        );
        assert_eq!(game.state.world.get_block(keg.0, keg.1, keg.2), block::AIR);
        if route == LocalRouting::SoloHosted {
            game.state.network_send_input();
            assert!(game.state.tick_hosted_server(before).faults.is_empty());
            game.state.finish_reputation_day_tick();
            assert_eq!(
                game.state.players[0].combat.health, after,
                "the world window cannot replay local blast damage"
            );
            assert_eq!(
                game.state.hosted_server.as_ref().unwrap().server.players[0]
                    .combat
                    .health,
                after
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_d4_machines_construction_does_not_fake_a_power_transition() {
    isolate_saves();
    let mut game = HeadlessGame::boot_into_world_routed(
        &format!("d4-machine-power-{:?}", std::thread::current().id()),
        LocalRouting::SoloHosted,
    );
    game.state.ecs.clear();
    let at = game.state.players[0].player.pos.as_ivec3();
    let anchor = (at.x + 5, at.y + 2, at.z);
    game.state
        .world
        .set_block(anchor.0, anchor.1, anchor.2, block::STONE);
    let mut plan = crate::plan::PlanData::debug_3x3_stone();
    plan.width = 1;
    plan.depth = 1;
    plan.cells = vec![crate::plan::CapturedCell {
        rx: 0,
        ry: 0,
        rz: 0,
        block_id: block::ELECTRIC_LAMP_LIT,
    }];
    game.state.world.construction_anchors.insert(
        anchor,
        crate::plan::ConstructionAnchorData {
            plan,
            rotations: 0,
            anchor,
            placed_index: 0,
            locked_materials: Vec::new(),
            is_creative_build: true,
            pace_divider: 1,
            pace_counter: 0,
            builder_credit: None,
        },
    );
    let objective = crate::scenario::Objective::Action {
        event: crate::scenario::ChallengeEvent::PowerDevice,
        count: 1,
    };
    let def = crate::scenario::ScenarioDef {
        kind: crate::scenario::ScenarioKind::Test,
        display_name: "Power source control".into(),
        kit: Vec::new(),
        objective,
        scoring: crate::scenario::Scoring::Work,
        arena_seed: None,
        tuning: crate::scenario::ScenarioTuning::default(),
        lock_creative: false,
        arena_mode: crate::scenario::ArenaMode::Reuse,
        world_type: None,
        game_mode: None,
        time_lock: None,
        weather_lock: None,
        mobs_enabled: None,
        trial_race: None,
        arena: None,
    };
    game.state.scenario = Some(crate::scenario::ScenarioState::new(def));
    game.state.falling_tick_counter = 3;
    assert!(
        game.logical_ticks(1)
            .iter()
            .all(|report| report.faults.is_empty())
    );
    assert_eq!(
        game.state.world.get_block(anchor.0, anchor.1 + 1, anchor.2),
        block::ARCHITECT_PLAQUE
    );
    assert!(
        !game.state.scenario.as_ref().unwrap().is_ended(),
        "stamping a lit device is not a power transition"
    );

    // Positive control: the very same objective still observes a real
    // server power transition at a different cell in the next logical tick.
    let lamp = (anchor.0 + 3, anchor.1, anchor.2);
    let lever = (lamp.0 - 1, lamp.1, lamp.2);
    for (pos, block_id, kind) in [
        (
            lamp,
            block::ELECTRIC_LAMP,
            crate::power::PowerDeviceKind::ElectricLamp,
        ),
        (lever, block::LEVER, crate::power::PowerDeviceKind::Lever),
    ] {
        game.state.world.set_block(pos.0, pos.1, pos.2, block_id);
        let mut data = crate::power::PowerDeviceData::new(kind, crate::meta::Facing::Up);
        data.on = pos == lever;
        game.state.world.insert_power_device(pos, data);
        game.state.world.mark_dirty(pos);
        game.state.world.notify_neighbours(pos);
    }
    assert!(
        game.logical_ticks(1)
            .iter()
            .all(|report| report.faults.is_empty())
    );
    assert_eq!(
        game.state.world.get_block(lamp.0, lamp.1, lamp.2),
        block::ELECTRIC_LAMP_LIT
    );
    assert!(
        game.state.scenario.as_ref().unwrap().is_ended(),
        "genuine power transition still counts"
    );
}
