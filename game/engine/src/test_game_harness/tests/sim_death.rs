//! Death results reach the actual local player at the fixed-tick boundary.
use super::*;
use crate::{
    combat::{Attacker, Health, LastAttacker},
    entity::{self, Position},
    local_routing::LocalRouting,
    mob::MobType,
};

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d4_death_local_credit_drains_before_world_window_and_catch_up() {
    isolate_saves();
    for routing in [LocalRouting::Standalone, LocalRouting::SoloHosted] {
        let name = format!(
            "d4-death-local-{routing:?}-{:?}",
            std::thread::current().id()
        );
        let mut game = HeadlessGame::boot_into_world_routed(&name, routing);
        game.state.ecs.clear();
        let at = game.state.players[0].player.pos + glam::Vec3::new(8.0, 0.0, 0.0);
        for tamed in [true, false] {
            let bird = entity::spawn_mob(&mut game.state.ecs, MobType::Nostrich, at);
            if tamed {
                game.state
                    .ecs
                    .get::<&mut crate::nostrich::NostrichData>(bird)
                    .unwrap()
                    .ownership
                    .owner_pubkey = "local-player-0".into();
            }
            game.state.ecs.get::<&mut Health>(bird).unwrap().current = 0.0;
            game.state
                .ecs
                .insert_one(bird, LastAttacker(Attacker::Local(0)))
                .unwrap();
        }
        let before = game.state.world.sim_tally;
        game.state.tick();
        assert_eq!(
            game.state.players[0].kill_counter.get(&MobType::Nostrich),
            Some(&2),
            "local credit drains before the late authority window"
        );
        assert!(
            game.state.players[0].nostrich_vow.is_some(),
            "colocated wild victim retains its own Vow"
        );
        if routing == LocalRouting::SoloHosted {
            assert!(
                game.state
                    .hosted_server
                    .as_mut()
                    .unwrap()
                    .server
                    .sim_events
                    .take()
                    .is_empty()
            );
            game.state.network_send_input();
            assert!(game.state.tick_hosted_server(before).faults.is_empty());
            game.state.finish_reputation_day_tick();
        }
        assert!(game.logical_ticks(3).iter().all(|r| r.faults.is_empty()));
        assert_eq!(
            game.state.players[0].kill_counter.get(&MobType::Nostrich),
            Some(&2),
            "catch-up never replays the credit"
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d4_death_projectile_kill_waits_for_next_early_boundary() {
    isolate_saves();
    for routing in [LocalRouting::Standalone, LocalRouting::SoloHosted] {
        let name = format!(
            "d4-death-projectile-{routing:?}-{:?}",
            std::thread::current().id()
        );
        let mut game = HeadlessGame::boot_into_world_routed(&name, routing);
        game.state.ecs.clear();
        let at = game.state.players[0].player.pos + glam::Vec3::new(8.0, 2.0, 0.0);
        for x in -2..=2 {
            for y in -2..=3 {
                for z in -2..=2 {
                    let p = at.as_ivec3() + glam::IVec3::new(x, y, z);
                    game.state.world.set_block(p.x, p.y, p.z, crate::block::AIR);
                }
            }
        }
        let cow = entity::spawn_mob(&mut game.state.ecs, MobType::Cow, at);
        game.state.ecs.get::<&mut Health>(cow).unwrap().current = 1.0;
        entity::spawn_arrow(
            &mut game.state.ecs,
            at + glam::Vec3::new(-0.5, 0.5, 0.0),
            glam::Vec3::X,
            100.0,
            Some(0),
        );
        assert!(game.logical_ticks(1).iter().all(|r| r.faults.is_empty()));
        assert!(
            game.state.ecs.contains(cow),
            "projectile runs after the early death sweep"
        );
        assert!(
            game.state.ecs.get::<&Health>(cow).unwrap().is_dead(),
            "the arrow actually landed"
        );
        assert!(
            game.state.players[0]
                .kill_counter
                .get(&MobType::Cow)
                .is_none()
        );
        assert!(game.logical_ticks(1).iter().all(|r| r.faults.is_empty()));
        assert!(game.state.ecs.get::<&Position>(cow).is_err());
        assert_eq!(
            game.state.players[0].kill_counter.get(&MobType::Cow),
            Some(&1)
        );
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas"]
fn game_harness_d4_death_host_preparation_retries_after_backlog_without_admitting_early() {
    use crate::transport::ClientTransport as _;
    let name = "d4-death-host-preparation";
    let _root = crate::save::WorldsRootGuard::new(name);
    let mut game = HeadlessGame::boot_into_world(name);
    game.state.ecs.clear();
    let at = game.state.players[0].player.pos + glam::Vec3::X * 8.0;
    for _ in 0..65 {
        let mut health = Health::new(10.0);
        health.current = 0.0;
        game.state
            .ecs
            .spawn((entity::MobKind(MobType::Cow), Position(at), health));
    }
    let mut host = crate::hosted_server::HostedServer::start_host(
        1,
        name.into(),
        42,
        0,
        crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        crate::hosted_server::HostWorld::Lent,
    )
    .unwrap();
    let client = host.attach_test_remote();
    client.send_to_server(&crate::protocol::serialize_packet(
        crate::protocol::PacketType::Ping,
        &(),
    ));
    game.state.hosted_server = Some(host);
    assert!(game.hosted_ticks(1).iter().all(|r| r.faults.is_empty()));
    let refusal = game
        .state
        .hosted_server
        .as_ref()
        .unwrap()
        .local_gameplay_preparation
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap_err();
    assert!(
        refusal.contains("Deaths are still being resolved"),
        "the transient death capture refusal actually occurred: {refusal}"
    );
    let mut pong = false;
    while let Some(packet) = client.try_recv_from_server() {
        pong |= matches!(
            crate::protocol::deserialize_header(&packet),
            Some((crate::protocol::PacketType::Pong, _))
        );
    }
    assert!(pong, "waiting for deaths does not starve free control");
    client.send_to_server(&crate::protocol::serialize_packet(
        crate::protocol::PacketType::JoinRequest,
        &crate::remote_client::build_join_request_guest("Early guest", 0),
    ));
    assert!(game.hosted_ticks(1).iter().all(|r| r.faults.is_empty()));
    let mut rejected = false;
    while let Some(packet) = client.try_recv_from_server() {
        let (kind, _) = crate::protocol::deserialize_header(&packet).unwrap();
        assert_ne!(
            kind,
            crate::protocol::PacketType::JoinAccept,
            "guests must not slip through unprepared authority"
        );
        rejected |= kind == crate::protocol::PacketType::JoinReject;
    }
    assert!(rejected);
    assert!(game.hosted_ticks(2).iter().all(|r| r.faults.is_empty()));
    assert!(!crate::sim_death::has_pending(&game.state.ecs));
    assert!(
        game.state.world.quest_scope.is_some(),
        "the same host retries after its finite death backlog drains"
    );
    assert!(
        game.state
            .hosted_server
            .as_ref()
            .unwrap()
            .local_gameplay_preparation
            .as_ref()
            .unwrap()
            .is_ok()
    );
    assert_eq!(
        game.state.players[0].kill_counter.get(&MobType::Cow),
        Some(&65),
        "all current live credit survives the retry"
    );
}
