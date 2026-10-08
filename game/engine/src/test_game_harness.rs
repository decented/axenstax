//! Headless GameState harness (2026-07-11) — the wave-hardening backlog's
//! "high-leverage infra" item. Boots the FULL client game — world, chunk
//! streaming, 20 TPS ticks, input dispatch, ECS, egui UI logic — with no
//! display server, by pumping the real per-frame entry
//! (`update_and_render`) over a `GameWindow::Headless` + headless renderer.
//! Only surface paints are skipped (see `game_window.rs`); everything the
//! playtest exercises short of pixels runs for real.
//!
//! GPU note: `Renderer::new_headless` needs an adapter whose
//! `max_texture_array_layers` holds the 506-layer block atlas — a real GPU
//! (this dev box's Intel iGPU: 2048) or a capable software stack. CI's
//! llvmpipe caps at 256, so harness tests are `#[ignore]` and run locally:
//! `cargo test --lib -- --ignored game_harness`.

use crate::GameMode;

pub(crate) struct HeadlessGame {
    pub(crate) state: crate::GameState,
}

impl HeadlessGame {
    /// Boot the full game headless and drive the REAL world-entry flow
    /// (`GameMode::Loading` → `begin_load` → `step_load` frames) into the
    /// named world — fresh worldgen when no save exists. Point
    /// `AXENSTAX_WORLDS_DIR` at a temp dir first so tests never touch real
    /// saves.
    pub(crate) fn boot_into_world(world_name: &str) -> Self {
        let mut state = pollster::block_on(crate::GameState::new_headless(1280, 720));
        state.world_name = world_name.to_string();
        // No first-spawn controls card in headless runs: it is a modal that
        // freezes player 0 and frees the cursor, and the persisted
        // `controls_card_seen` flag is whatever the dev machine's settings.json
        // says. (In-memory only — never `save()`d.)
        state.graphics.controls_card_seen = true;
        state.mode = GameMode::Loading(crate::loading_screen::LoadingState::new(
            world_name.to_string(),
        ));
        // Bounded by wall-clock time, not a frame count: the loading screen
        // stays up for `loading_screen::MIN_DISPLAY_SECS` of real time, and
        // headless frames with nothing to paint can run past 4,000 a second,
        // so a 20,000-frame cap flaked on a fast frame rate.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        let mut frames = 0u32;
        while !matches!(state.mode, GameMode::Playing) {
            state.update_and_render();
            frames += 1;
            assert!(
                std::time::Instant::now() < deadline,
                "world load never reached Playing (mode stuck after {frames} frames)"
            );
        }
        Self { state }
    }

    /// Pump `n` real frames (UI logic + time-based ticks, no paint).
    pub(crate) fn frames(&mut self, n: u32) {
        for _ in 0..n {
            self.state.update_and_render();
        }
    }

    /// Run `n` deterministic game ticks directly (bypasses the wall-clock
    /// accumulator — same entry the accumulator drives).
    pub(crate) fn ticks(&mut self, n: u32) {
        for _ in 0..n {
            self.state.tick();
        }
    }

    /// Run `n` logical ticks the way the main loop does while HOSTING: the
    /// client's tick, its input send, then the embedded server's tick (on the
    /// host's lent world, D1). One report per tick.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn hosted_ticks(&mut self, n: u32) -> Vec<crate::game_loop::LentTickReport> {
        (0..n)
            .map(|_| {
                let before = self.state.world.sim_tally;
                self.state.tick();
                self.state.network_send_input();
                self.state.tick_hosted_server(before)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point `AXENSTAX_WORLDS_DIR` at one pid-scoped temp dir shared by every
    /// harness test — same value from any test thread, so concurrently
    /// running harness tests can't race the env var, and nothing ever
    /// touches the user's real saves. Left for the OS to clean (deleting it
    /// from one test could yank a sibling test's world mid-run).
    fn isolate_saves() {
        let tmp = std::env::temp_dir().join(format!(
            "axenstax-harness-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        // SAFETY: tests in this binary run in-process; every caller sets the
        // same value, so a concurrent read can never see a torn/foreign dir.
        unsafe { std::env::set_var("AXENSTAX_WORLDS_DIR", &tmp) };
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_boots_a_real_world_and_ticks_the_live_sim() {
        isolate_saves();

        let mut hg = HeadlessGame::boot_into_world("harness-proof");

        // The real load flow produced terrain under the spawn column.
        let spawn = hg.state.players[0].player.pos;
        let (sx, sz) = (spawn.x.floor() as i32, spawn.z.floor() as i32);
        let mut solid_below = false;
        for y in (0..spawn.y as i32).rev() {
            if hg.state.world.get_block(sx, y, sz) != crate::block::AIR {
                solid_below = true;
                break;
            }
        }
        assert!(solid_below, "fresh worldgen must put ground under the spawn");

        // The live sim advances deterministically through the real tick entry.
        let t0 = hg.state.world_time;
        hg.ticks(40);
        assert_eq!(
            hg.state.world_time,
            (t0 + 40) % 24000,
            "40 ticks advance world time by 40"
        );

        // And the full frame path (UI logic incl. egui, no paint) survives
        // being pumped — the exact loop a real session runs.
        hg.frames(30);
    }

    /// Spec 39 A6 — the mipmaps dial rebuilds the block atlas + sampler live.
    /// This is the only place the *GPU* side of A6 can be exercised without a
    /// display: a bad mip level count, an out-of-range `write_texture` level, or
    /// an illegal sampler (e.g. anisotropy with a Nearest filter) is an
    /// uncaptured wgpu validation error, which panics the test.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_mipmap_dial_rebuilds_the_atlas_live() {
        isolate_saves();

        let mut hg = HeadlessGame::boot_into_world("harness-mipmaps");
        assert!(!hg.state.graphics.mipmaps, "ships off");
        hg.frames(3);

        // On: full atlas rebuild with a CPU-generated chain + a new sampler,
        // then keep drawing through it.
        hg.state.graphics.mipmaps = true;
        hg.state.sync_graphics_to_engine(); // no `save()` — never touch settings.json
        hg.frames(10);

        // Off again: back to `mip_level_count: 1` + the Nearest sampler.
        hg.state.graphics.mipmaps = false;
        hg.state.sync_graphics_to_engine();
        hg.frames(10);

        // And a redundant apply must be a no-op, not a second rebuild.
        hg.state.sync_graphics_to_engine();
        hg.frames(3);
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_raid_kill_attribution_settles_through_the_live_game_loop() {
        // Spec 22 Phases 5+18 end-to-end, through the REAL game_loop tick:
        // dying RaidMember mobs → nearest in-radius defender credited →
        // out-of-radius kills decrement mobs_alive without credit → last
        // kill stamps killing_blow → Cleared resolution settles reputation
        // + the per-village leaderboard and drains the raid. Until now this
        // chain was playtest-only (the raids.rs integration tests mutate
        // the Raid struct by hand instead of killing mobs in the sim).
        isolate_saves();

        let mut hg = HeadlessGame::boot_into_world("raid-attribution-harness");

        // Stage: a village anchored at the player's feet with an ACTIVE
        // 3-mob raid, mobs spawned through the real raid spawn helper.
        let ppos = hg.state.players[0].player.pos;
        let anchor = [
            ppos.x.floor() as i32,
            ppos.y.floor() as i32,
            ppos.z.floor() as i32,
        ];
        let vid: crate::reputation::VillageId = (7, 7);
        hg.state.world.village_anchors.insert(vid, anchor);
        hg.state.world.village_treasuries.insert(vid, 1_000);
        let mut raid = crate::raid::Raid::new_warning(
            1,
            vid,
            anchor,
            crate::raid::WaveKind::Small,
            100,
            hg.state.tick_counter,
        );
        raid.status = crate::raid::RaidStatus::Active;
        raid.spawn_at_tick = Some(hg.state.tick_counter);
        raid.mobs_alive = 3;
        raid.mobs_total = 3;
        hg.state.world.active_raids.push(raid);
        let mob_pos = ppos + glam::Vec3::new(2.0, 0.0, 0.0);
        let mobs: Vec<hecs::Entity> = (0..3)
            .map(|i| {
                crate::raid::spawn_raid_mob(
                    &mut hg.state.ecs,
                    crate::mob::MobType::Brigand,
                    mob_pos + glam::Vec3::new(0.0, 0.0, i as f32),
                    1,
                )
            })
            .collect();

        let kill = |hg: &mut HeadlessGame, e: hecs::Entity| {
            hg.state
                .ecs
                .get::<&mut crate::combat::Health>(e)
                .expect("raid mob alive")
                .current = 0.0;
        };

        // Kill 1 — player inside the 24-block defender radius → credited.
        kill(&mut hg, mobs[0]);
        hg.ticks(1);
        {
            let r = &hg.state.world.active_raids[0];
            assert_eq!(r.mobs_alive, 2, "kill 1 decrements mobs_alive");
            assert_eq!(
                r.contribution_table,
                vec![(0usize, 1u32)],
                "in-radius kill credits the defender"
            );
        }

        // Kill 2 — player teleported far outside the radius → the mob
        // still dies and mobs_alive ticks down, but NO contribution.
        let far = ppos + glam::Vec3::new(200.0, 40.0, 0.0);
        hg.state.players[0].player.pos = far;
        kill(&mut hg, mobs[1]);
        hg.ticks(1);
        {
            let r = &hg.state.world.active_raids[0];
            assert_eq!(r.mobs_alive, 1, "out-of-radius kill still decrements");
            assert_eq!(
                r.contribution_table,
                vec![(0usize, 1u32)],
                "out-of-radius kill earns no credit"
            );
            assert!(r.killing_blow.is_none());
        }

        // Kill 3 — back in radius; the last mob stamps killing_blow, the
        // next tick resolves Cleared, settles, and drains the raid.
        hg.state.players[0].player.pos = ppos;
        kill(&mut hg, mobs[2]);
        hg.ticks(2);

        assert!(
            hg.state.world.active_raids.is_empty(),
            "cleared raid settles and drains from active_raids"
        );
        assert_eq!(
            hg.state.world.raid_kills.get(&(vid, 0)).copied(),
            Some(2),
            "leaderboard tallies the two credited kills (not the stray one)"
        );
        let rep = hg.state.players[0].reputation.score(vid);
        assert_eq!(
            rep,
            crate::raid::DEFENDER_REP_GAIN + crate::raid::KILLING_BLOW_REP_BONUS,
            "defender rep = +{} share + {} killing-blow bonus",
            crate::raid::DEFENDER_REP_GAIN,
            crate::raid::KILLING_BLOW_REP_BONUS
        );
    }

    /// Spec 02 §8.4 — the REAL client world entry never replaces a world that
    /// failed to load: the Loading state's `begin_load` refuses it, the player is
    /// back in the lobby with the notice, the world was never marked live (so no
    /// autosave, Save & Quit or close-save can write it), and every file in its
    /// folder is byte-for-byte as it was. (Before: a warning, a fresh world, and
    /// the first save deleted/overwrote the real one.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_refuses_a_world_that_fails_to_load() {
        isolate_saves();
        let name = format!("harness-refused-{:?}", std::thread::current().id())
            .replace(|c: char| !c.is_ascii_alphanumeric(), "-");
        let dir = crate::save::world_dir(&name);
        let _ = std::fs::remove_dir_all(&dir);
        let mut w = crate::world::World::new();
        w.set_block(3, 64, 5, crate::block::BEDROCK);
        crate::save::write_world_folder(
            &name,
            &crate::save::WorldMeta::new(&name),
            &crate::save::minimal_world_save_for_tests(7),
            &w,
        )
        .unwrap();
        std::fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();
        let snapshot = || {
            let mut files: Vec<(String, Vec<u8>)> = Vec::new();
            let mut stack = vec![dir.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap().flatten() {
                    if e.path().is_dir() {
                        stack.push(e.path());
                    } else {
                        files.push((e.path().display().to_string(), std::fs::read(e.path()).unwrap()));
                    }
                }
            }
            files.sort();
            files
        };
        let before = snapshot();

        let mut state = pollster::block_on(crate::GameState::new_headless(1280, 720));
        state.world_name = name.clone();
        state.graphics.controls_card_seen = true;
        state.mode = GameMode::Loading(crate::loading_screen::LoadingState::new(name.clone()));
        for _ in 0..20 {
            state.update_and_render();
            if !matches!(state.mode, GameMode::Loading(_)) {
                break;
            }
        }
        let GameMode::Menu(menu) = &state.mode else {
            panic!("a world that fails to load must land back in the lobby");
        };
        let notice = menu.notice.clone().unwrap_or_default();
        assert!(notice.starts_with("This world couldn't be opened: world.dat is damaged"), "{notice}");
        assert!(notice.ends_with("Nothing was changed."), "{notice}");
        assert!(state.live_world.is_none(), "never marked live");

        // Lobby frames, then every exit that saves a live world: nothing writes.
        for _ in 0..5 {
            state.update_and_render();
        }
        state.leave_world(
            crate::world_exit::SaveChoice::Save,
            crate::world_exit::ExitTo::Lobby,
        );
        assert_eq!(snapshot(), before, "the refused world's files must be untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// D1 — a host lends its world to its embedded server, through the REAL
    /// game-loop tick: every shared sim system runs once per logical tick
    /// across the two halves (the tripwire), the clock advances once, a
    /// server-made block change is remeshed on the host, and a joiner's diff
    /// carries the host's own mobs.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_lending_host_runs_one_sim_that_joiners_see() {
        use crate::transport::ClientTransport;
        isolate_saves();
        let name = "harness-lend";
        let mut hg = HeadlessGame::boot_into_world(name);
        let seed = hg.state.biome_gen.seed;
        let hs = crate::hosted_server::HostedServer::start_host(
            1,
            name.to_string(),
            seed,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
            crate::hosted_server::HostWorld::Lent,
        )
        .expect("lending host starts");
        hg.state.hosted_server = Some(hs);
        assert!(hg.state.sim_lent());

        // A guest joins over the in-process transport.
        let client = hg.state.hosted_server.as_mut().unwrap().attach_test_remote();
        let req = crate::remote_client::build_join_request_guest("Visitor", 0);
        client.send_to_server(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::JoinRequest,
            &req,
        ));

        // Sand over air beside the host, in the host's world.
        let p = hg.state.players[0].player.pos;
        let sand = (p.x.floor() as i32, p.y.floor() as i32 + 8, p.z.floor() as i32);
        hg.state.world.set_block(sand.0, sand.1, sand.2, crate::block::SAND);
        hg.state.world.set_block(sand.0, sand.1 - 1, sand.2, crate::block::AIR);

        let t0 = hg.state.world_time;
        let reports = hg.hosted_ticks(8);
        for r in &reports {
            assert!(r.faults.is_empty(), "a shared system ran the wrong number of times: {:?}", r.faults);
        }
        assert_eq!(hg.state.world_time, (t0 + 8) % 24000, "the clock advances once a tick");
        // Two 4-tick falling passes in 8 ticks: the sand left its cell and is
        // one or two cells down, in the host's own world.
        assert_eq!(
            hg.state.world.get_block(sand.0, sand.1, sand.2),
            crate::block::AIR,
            "the server's falling-block pass ran on the host's world"
        );
        assert!(
            (1..=2).any(|d| hg.state.world.get_block(sand.0, sand.1 - d, sand.2) == crate::block::SAND),
            "the sand fell, once per pass"
        );
        let sand_chunk = crate::world::World::block_to_chunk(sand.0, sand.1, sand.2);
        assert!(
            reports.iter().any(|r| r.remeshed.contains(&sand_chunk)),
            "the host remeshes the server's change"
        );

        // The joiner's diff is the host's own ECS.
        let mut spawned = std::collections::HashSet::new();
        while let Some(pkt) = client.try_recv_from_server() {
            if let Some((ptype, payload)) = crate::protocol::deserialize_header(&pkt)
                && ptype == crate::protocol::PacketType::StateUpdate
                && let Ok(s) =
                    crate::protocol::safe_deserialize::<crate::protocol::StateUpdatePacket>(payload)
            {
                spawned.extend(s.entity_spawns.iter().map(|e| e.id));
            }
        }
        // MP-D2a — every host mob near the joiner's body reached it (a
        // joiner hears about the entities inside its interest radius).
        let joiner = hg.state.hosted_server.as_ref().unwrap().server.players.last().unwrap().player.pos;
        let mut host_mobs = 0;
        for (_e, (pid, pos, _kind)) in hg
            .state
            .ecs
            .query::<(&crate::entity::ProtocolId, &crate::entity::Position, &crate::entity::MobKind)>()
            .iter()
        {
            let d = glam::Vec2::new(pos.0.x - joiner.x, pos.0.z - joiner.z).length();
            if d <= crate::entity_broadcast::INTEREST_ENTER_RADIUS {
                host_mobs += 1;
                assert!(spawned.contains(&pid.0), "host mob {} never reached the joiner", pid.0);
            }
        }
        assert!(host_mobs > 0, "a fresh world has mobs around the host (and its joiner)");
        let hs = hg.state.hosted_server.as_ref().unwrap();
        assert_eq!(hs.server.ecs.len(), 0, "the server keeps no population of its own");
    }

    /// D1 review fix 1 through the real `GameState::stream_chunks`: on a
    /// lending host whose player walks far from a joiner, the host client's
    /// streamer keeps the joiner's column (and its neighbours) loaded — the
    /// GPU-free half lives in `lent_world` / `chunk_stream`'s tests.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_lending_hosts_streamer_keeps_a_joiners_columns() {
        use crate::chunk_stream::{column_of, UNLOAD_HYSTERESIS};
        use crate::transport::ClientTransport;
        isolate_saves();
        let name = "harness-lend-anchors";
        let mut hg = HeadlessGame::boot_into_world(name);
        let seed = hg.state.biome_gen.seed;
        let hs = crate::hosted_server::HostedServer::start_host(
            1,
            name.to_string(),
            seed,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
            crate::hosted_server::HostWorld::Lent,
        )
        .expect("lending host starts");
        hg.state.hosted_server = Some(hs);
        let client = hg.state.hosted_server.as_mut().unwrap().attach_test_remote();
        let req = crate::remote_client::build_join_request_guest("Visitor", 0);
        client.send_to_server(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::JoinRequest,
            &req,
        ));
        hg.hosted_ticks(2);
        let joiners = hg.state.hosted_server.as_ref().unwrap().lent_joiner_columns();
        assert_eq!(joiners.len(), 1, "the guest is seated");
        let joiner_col = joiners[0];
        assert!(hg.state.loaded_columns.contains(&joiner_col));

        // The host walks off well past its render distance + hysteresis.
        let rd = hg.state.graphics.render_distance;
        let away = (rd + UNLOAD_HYSTERESIS + 3) as f32 * crate::chunk::CHUNK_SIZE as f32;
        hg.state.players[0].player.pos.x += away;
        assert!(column_of(hg.state.players[0].player.pos).0 - joiner_col.0 > rd + UNLOAD_HYSTERESIS);
        for _ in 0..4 {
            hg.state.stream_chunks();
        }
        for dx in -1..=1 {
            for dz in -1..=1 {
                let col = (joiner_col.0 + dx, joiner_col.1 + dz);
                assert!(hg.state.loaded_columns.contains(&col), "joiner's column {col:?} unloaded");
            }
        }
        let reports = hg.hosted_ticks(4);
        assert!(reports.iter().all(|r| r.faults.is_empty()));
    }

    /// MP-D2a — a client joined to someone else's server keeps no mobs of
    /// its own (its spawners are off and anything that slips in is purged),
    /// draws the server's mobs from its mirror, and shows the health the
    /// server holds for its body.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_mirrors_the_servers_mobs_and_runs_none_of_its_own() {
        use crate::entity::{MobKind, ProtocolId};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-joiner-mirror");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-joiner-mirror-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Mirror", 0),
            None,
        ));
        // A private mob that slipped into the joiner's own sim (the booted
        // world's scatter already put some there).
        crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Brigand, glam::Vec3::ZERO);
        let step = |server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame| {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        };
        for _ in 0..5 {
            step(&mut server, &mut hg);
        }
        let body = server.server.players.last().expect("the joiner is seated").player.pos;
        let cow = crate::entity::spawn_mob(
            &mut server.server.ecs,
            crate::mob::MobType::Cow,
            body + glam::Vec3::new(3.0, 0.0, 0.0),
        );
        for _ in 0..20 {
            step(&mut server, &mut hg);
        }
        let id = server.server.ecs.get::<&ProtocolId>(cow).expect("broadcast").0;
        assert!(hg.state.remote_mobs.drawn(id).is_some(), "the joiner mirrors the server's cow");
        assert_eq!(
            hg.state.ecs.query::<&MobKind>().iter().count(),
            0,
            "the joiner keeps no mobs of its own"
        );

        // Its health is the server's.
        server.server.players.last_mut().unwrap().combat.health = 11.0;
        for _ in 0..3 {
            step(&mut server, &mut hg);
        }
        let hp = hg.state.players[0].combat.health;
        assert!((10.0..=11.5).contains(&hp), "the joiner shows the server's health, got {hp}");
    }

    /// Review D2b LOW-8 — a joiner's swing through its REAL client: the
    /// request leaves `send_entity_attack`, the server lands it, and the
    /// client applies the answers in `network_receive` — the accepted
    /// `InteractOutcome` wears its sword (`apply_interact_outcome`) and the
    /// `KillEvent` runs its kill attribution (`apply_kill_event` →
    /// `credit_kill`: the kill counter).
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_swing_wears_its_sword_and_its_kill_is_counted() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-joiner-swing");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-joiner-swing-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Swinger", 0),
            None,
        ));
        let step = |server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame| {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        };
        for _ in 0..5 {
            step(&mut server, &mut hg);
        }
        let sword = crate::item::Item::Tool(crate::crafting::Tool::new(
            crate::crafting::ToolType::Sword,
            crate::crafting::ToolMaterial::Iron,
        ));
        let hot = hg.state.players[0].hotbar_slot;
        hg.state.players[0]
            .inventory
            .set_slot(hot, Some(crate::item::ItemStack { item: sword, count: 1 }));
        let durability = |hg: &HeadlessGame| match &hg.state.players[0].inventory.hotbar_slot(hot).unwrap().item {
            crate::item::Item::Tool(t) => t.durability,
            _ => unreachable!(),
        };
        let before = durability(&hg);

        // A chicken one hit from death, right in front of the joiner's body.
        let sp = server.server.players.last().expect("the joiner is seated");
        let ahead = crate::camera::forward_from(sp.yaw, 0.0);
        let at = sp.player.pos + ahead * 1.5;
        let chicken = crate::entity::spawn_mob(&mut server.server.ecs, crate::mob::MobType::Chicken, at);
        server.server.ecs.get::<&mut crate::combat::Health>(chicken).unwrap().current = 1.0;
        step(&mut server, &mut hg);
        let id = server.server.ecs.get::<&crate::entity::ProtocolId>(chicken).expect("broadcast").0;
        server.server.ecs.get::<&mut crate::entity::Position>(chicken).unwrap().0 = at;
        hg.state.send_entity_attack(
            0,
            crate::remote_mobs::MirrorTarget {
                id,
                kind: crate::mob::MobType::Chicken,
                tamed: false,
                baby: false,
                tethered: false,
                product_not_ready: false,
            },
            false,
            false,
        );
        for _ in 0..4 {
            step(&mut server, &mut hg);
        }
        assert_eq!(durability(&hg), before - 1, "the confirmed swing wore the sword");
        assert_eq!(
            hg.state.players[0].kill_counter.get(&crate::mob::MobType::Chicken).copied(),
            Some(1),
            "the server's KillEvent ran the joiner's kill attribution"
        );
    }

    /// C3a-2a — a joined client's window ops through its REAL send path:
    /// what its crafting screen applies (`CraftingUi::apply_click`,
    /// `open_player_crafting`, `close`) is logged and sent by the flush at the
    /// start of `network_send_input`, its auto-refill setting first; the
    /// server's copy of its window follows slot for slot, the 2×2 craft
    /// included, with no mismatch.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_inventory_clicks_reach_the_servers_window() {
        use crate::item::ItemStack;
        use crate::window::{WindowClick, WindowSlot};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-joiner-window");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-joiner-window-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Clicker", 0),
            None,
        ));
        let step = |server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame| {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        };
        for _ in 0..5 {
            step(&mut server, &mut hg);
        }
        // The same window on both sides to start (the arrival inventory is
        // not on the wire yet), except the setting: the client's is off.
        let slot = server.server.players.len() - 1;
        let start = |inv: &mut crate::inventory::Inventory| {
            *inv = crate::inventory::Inventory::new();
            inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 10)));
            inv.set_slot(1, Some(ItemStack::new_block(crate::block::OAK_PLANKS, 4)));
        };
        start(&mut server.server.players[slot].inventory);
        // The join already sent the client's setting (on) with the digest of
        // the window it arrived with, which the server never had: start the
        // tally from the matched windows.
        server.server.players[slot].possession = Default::default();
        let p = &mut hg.state.players[0];
        start(&mut p.inventory);
        p.inventory.auto_refill = false;
        p.armour_slots = [None; 4];
        p.crafting_ui.open_player_crafting(&p.inventory, &p.armour_slots);
        let eye = p.player.eye_pos();
        let clicks = [
            WindowClick::Slot { slot: 1, right: false },
            WindowClick::DragDistribute { slots: vec![WindowSlot::Grid(0, 0)] },
            WindowClick::DragDistribute { slots: vec![WindowSlot::Grid(0, 1)] },
            WindowClick::DragDistribute { slots: vec![WindowSlot::Grid(1, 0)] },
            WindowClick::DragDistribute { slots: vec![WindowSlot::Grid(1, 1)] },
            WindowClick::Result,
            WindowClick::Slot { slot: 20, right: false },
            WindowClick::Slot { slot: 0, right: false },
            WindowClick::Slot { slot: 9, right: true },
        ];
        for click in &clicks {
            p.crafting_ui.apply_click(&mut p.inventory, &mut p.armour_slots, click, false, eye, |_| crate::block::AIR);
        }
        assert!(p.crafting_ui.close(&mut p.inventory, &mut p.armour_slots), "the stone goes back");
        for _ in 0..3 {
            step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        let p = &hg.state.players[0];
        let slots = |inv: &crate::inventory::Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
        assert_eq!(slots(&sp.inventory), slots(&p.inventory), "the server's 36 slots follow the client's");
        assert_eq!(
            sp.inventory.slot(20).map(|s| s.item.clone()),
            Some(crate::item::Item::Block(crate::block::CRAFTING_TABLE)),
            "the 2×2 craft was the server's too"
        );
        assert!(!sp.inventory.auto_refill, "the client's setting arrived first");
        assert!(sp.cursor.is_none() && sp.craft_grid.iter().flatten().all(Option::is_none));
        // SetAutoRefill, OpenPlayer, nine clicks, the close.
        assert_eq!(sp.possession.window_ops, 12);
        assert_eq!(sp.possession.window_mismatch, 0);
        assert_eq!(sp.possession.crafts_ignored, 0, "no ItemAction::Craft was sent");
    }

    /// C3a-fix-1 — a joined client on a dedicated server, joined and settled,
    /// with the same window on both sides (the setting included) and the
    /// server's tally from there: `(game, server, joiner's slot)`.
    fn joined_window_client(tag: &str) -> (HeadlessGame, crate::hosted_server::HostedServer, usize) {
        let mut hg = HeadlessGame::boot_into_world(&format!("harness-{tag}"));
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-{tag}-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Orderly", 0),
            None,
        ));
        for _ in 0..5 {
            harness_step(&mut server, &mut hg);
        }
        let slot = server.server.players.len() - 1;
        let p = &mut hg.state.players[0];
        p.inventory = crate::inventory::Inventory::new();
        p.armour_slots = [None; 4];
        server.server.players[slot].inventory = crate::inventory::Inventory::new();
        server.server.players[slot].inventory.auto_refill = p.inventory.auto_refill;
        server.server.players[slot].possession = Default::default();
        (hg, server, slot)
    }

    /// One logical tick of a joined client and its dedicated server.
    fn harness_step(server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame) {
        server.tick();
        hg.frames(1);
        hg.ticks(1);
        hg.state.network_send_input();
    }

    /// C3a-fix-1 (B-L1) — a Close then a Q-drop in one tick, through the REAL
    /// send path: the close put the cursor's stack back in hotbar slot 0 and
    /// the drop takes one from there. The drop goes out mid-frame, but the
    /// Close logged before it goes first (`flush_ops_before_edits`), so the
    /// server's copy has the stack back when it pays the drop: no shortfall,
    /// no mismatch. (It used to read the Drop first and find the stack still
    /// on its cursor.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_close_then_a_drop_in_one_tick_reach_the_server_in_order() {
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("close-then-drop");
        let sticks = crate::item::ItemStack::new_material(crate::item::MaterialId::Stick, 5);
        hg.state.players[0].inventory.set_slot(0, Some(sticks.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(sticks));
        hg.state.players[0].hotbar_slot = 0;
        let p = &mut hg.state.players[0];
        p.crafting_ui.open_player_crafting(&p.inventory, &p.armour_slots);
        let eye = p.player.eye_pos();
        let pick_up = crate::window::WindowClick::Slot { slot: 0, right: false };
        p.crafting_ui.apply_click(&mut p.inventory, &mut p.armour_slots, &pick_up, false, eye, |_| crate::block::AIR);
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        assert!(server.server.players[slot].cursor.is_some(), "the server's cursor holds the sticks");
        // One tick: Close, then Q.
        let p = &mut hg.state.players[0];
        assert!(p.crafting_ui.close(&mut p.inventory, &mut p.armour_slots));
        hg.state.players[0].drop_ready_tick = 0;
        hg.state.send_drop_request(0, 0);
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        assert_eq!(sp.possession.drops, 1, "the drop was spawned");
        assert_eq!(sp.possession.mismatched, 0, "no shortfall: the Close came first");
        assert_eq!(sp.possession.window_mismatch, 0);
        assert_eq!(sp.inventory.slot(0).map(|s| s.count), Some(4));
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(4), "the same on the client");
    }

    /// C3a-fix-1 (B-L1) — a placement then E in one tick, through the REAL
    /// send path: the ops logged after the input's first edit go after the
    /// input, so the server places (and charges the stone) before it opens
    /// the screen, as the client did: the `OpenPlayer` digest matches.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_placement_then_e_in_one_tick_reach_the_server_in_order() {
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("place-then-e");
        let stone = crate::item::ItemStack::new_block(crate::block::STONE, 3);
        hg.state.players[0].inventory.set_slot(0, Some(stone.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(stone));
        hg.state.players[0].hotbar_slot = 0;
        harness_step(&mut server, &mut hg);
        // The cell over the server body's head: air, and in reach.
        let body = server.server.players[slot].player.pos;
        let cell = (body.x.floor() as i32, body.y.floor() as i32 + 2, body.z.floor() as i32);
        assert_eq!(server.server.world.get_block(cell.0, cell.1, cell.2), crate::block::AIR);
        // One tick: the place arm's edit (one stone from slot 0), then E.
        let p = &mut hg.state.players[0];
        p.inventory.take_placeable_from_hotbar(0);
        hg.state.world.set_block(cell.0, cell.1, cell.2, crate::block::STONE);
        hg.state.pending_block_changes.push(crate::protocol::BlockChange {
            x: cell.0,
            y: cell.1,
            z: cell.2,
            new_block: crate::block::STONE,
            meta: 0,
        });
        let p = &mut hg.state.players[0];
        p.crafting_ui.open_player_crafting(&p.inventory, &p.armour_slots);
        hg.state.network_send_input();
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        assert_eq!(server.server.world.get_block(cell.0, cell.1, cell.2), crate::block::STONE, "placed");
        assert_eq!((sp.possession.matched, sp.possession.mismatched), (1, 0), "charged to slot 0");
        assert_eq!(sp.possession.window_mismatch, 0, "the edit came first: OpenPlayer matched");
        assert_eq!(sp.inventory.slot(0).map(|s| s.count), Some(2));
    }

    /// C3b-fix-b (B-M1) — a placement then a Q-drop in one tick, through the
    /// REAL send path, with auto-refill on: the drop is queued behind the
    /// unsent edit and goes right after the input carrying it, so the server
    /// places (its auto-refill moves the bag's stack in), then drops: its 36
    /// slots are the client's. (It used to read the drop first, find the
    /// slot's last dirt, and never refill.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_placement_then_q_in_one_tick_reach_the_server_in_order() {
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("place-then-q");
        let dirt = crate::item::ItemStack::new_block(crate::block::DIRT, 1);
        let bag = crate::item::ItemStack::new_block(crate::block::DIRT, 64);
        for (inv, s) in [(&mut hg.state.players[0].inventory, 0), (&mut server.server.players[slot].inventory, 0)] {
            inv.set_slot(s, Some(dirt.clone()));
            inv.set_slot(20, Some(bag.clone()));
            inv.auto_refill = true;
        }
        hg.state.players[0].hotbar_slot = 0;
        harness_step(&mut server, &mut hg);
        let body = server.server.players[slot].player.pos;
        let cell = (body.x.floor() as i32, body.y.floor() as i32 + 2, body.z.floor() as i32);
        assert_eq!(server.server.world.get_block(cell.0, cell.1, cell.2), crate::block::AIR);
        // One tick: the place arm's edit, then Q.
        hg.state.players[0].inventory.take_placeable_from_hotbar(0);
        hg.state.world.set_block(cell.0, cell.1, cell.2, crate::block::DIRT);
        hg.state.pending_block_changes.push(crate::protocol::BlockChange {
            x: cell.0,
            y: cell.1,
            z: cell.2,
            new_block: crate::block::DIRT,
            meta: 0,
        });
        hg.state.players[0].drop_ready_tick = 0;
        hg.state.send_drop_request(0, 0);
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(63), "refilled, then one thrown");
        for _ in 0..4 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        let slots = |inv: &crate::inventory::Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
        assert_eq!(slots(&sp.inventory), slots(&hg.state.players[0].inventory), "the same 36 slots");
        assert_eq!(sp.possession.drops, 1, "the drop was spawned");
        assert_eq!((sp.possession.matched, sp.possession.mismatched), (1, 0), "the placement matched its slot");
    }

    /// `n` tagged breaks of air cells high over the server body (out of
    /// reach: the server refuses each and changes nothing), as one tick's
    /// edits: 16 tags ride an input (`MAX_MINED_PER_INPUT`), so a long run
    /// waits in the carry-over over several inputs.
    fn queue_tagged_breaks(hg: &mut HeadlessGame, server: &crate::hosted_server::HostedServer, slot: usize, n: i32) {
        let body = server.server.players[slot].player.pos;
        let (x0, y, z0) = (body.x.floor() as i32, body.y.floor() as i32 + 20, body.z.floor() as i32);
        for k in 0..n {
            let (x, z) = (x0 + k % 10, z0 + k / 10);
            hg.state.pending_block_changes.push(crate::protocol::BlockChange { x, y, z, new_block: crate::block::AIR, meta: 0 });
            hg.state.pending_mined.push(crate::protocol::MinedBlock { x, y, z, tool: crate::protocol::WireItem::None });
        }
    }

    /// One input a step: [`harness_step`] with the client's wall-clock tick
    /// accumulator emptied first, so the frame runs no ticks (and sends no
    /// inputs) of its own.
    fn paced_step(server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame) {
        hg.state.last_tick = std::time::Instant::now();
        hg.state.tick_accumulator = std::time::Duration::ZERO;
        harness_step(server, hg);
    }

    /// Let the server read every input sent so far (the join steps' frames
    /// may have sent several a step), then one paced step.
    fn catch_up(server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame) {
        for _ in 0..20 {
            server.tick();
        }
        paced_step(server, hg);
    }

    /// C3b-fix-d (A-L1) — an Eat queued behind a long carry-over (160 tagged
    /// breaks in one tick, over ten inputs) claims its bread until it is
    /// SENT, then until the server acknowledges the input after it. The
    /// acknowledgements of the inputs it waited behind are applied as it goes
    /// out and end nothing, so a Q-drop of that last bread is refused: it
    /// can't be both eaten and thrown. (The claim used to end with the second
    /// input's acknowledgement, while the Eat still waited.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_an_eat_queued_behind_a_long_carry_over_claims_its_bread_until_it_is_sent() {
        use crate::item::{ItemStack, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("eat-behind-carry-over");
        let bread = ItemStack::new_material(MaterialId::Bread, 1);
        hg.state.players[0].inventory.set_slot(0, Some(bread.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(bread));
        hg.state.players[0].hotbar_slot = 0;
        server.server.players[slot].combat.hunger = 9;
        catch_up(&mut server, &mut hg);
        queue_tagged_breaks(&mut hg, &server, slot, 160);
        let made_before = hg.state.remote_client.as_ref().expect("joined").next_input_seq();
        hg.state.send_eat_request(0);
        let queued = |hg: &HeadlessGame| hg.state.remote_client.as_ref().expect("joined").has_queued_requests();
        assert!(queued(&hg), "the Eat waits behind the breaks");
        let mut steps = 0;
        while queued(&hg) {
            paced_step(&mut server, &mut hg);
            steps += 1;
            assert!(steps < 40, "the carry-over drains");
        }
        assert!(
            server.server.players[slot].last_applied_input > made_before,
            "the server read inputs made after the Eat before it went (the acknowledgements it waited behind): \
             made before input {made_before}, {steps} steps, server at {}, client at {}",
            server.server.players[slot].last_applied_input,
            hg.state.remote_client.as_ref().expect("joined").next_input_seq()
        );
        // Sent just now, not yet read by the server: still spoken for.
        assert!(hg.state.joiner_actions.eat_in_flight(), "the Eat still claims its bread");
        let breads = |hg: &HeadlessGame| hg.state.players[0].inventory.count_material(MaterialId::Bread);
        hg.state.players[0].drop_ready_tick = 0;
        hg.state.send_drop_request(0, 0);
        assert_eq!(breads(&hg), 1, "the Q-drop of the claimed bread is refused");
        // The server reads the Eat after the edits it deferred (four a tick).
        let mut steps = 0;
        while server.server.players[slot].combat.hunger == 9 {
            paced_step(&mut server, &mut hg);
            steps += 1;
            assert!(steps < 120, "the server reads the Eat");
            assert!(hg.state.joiner_actions.eat_in_flight() || breads(&hg) == 0, "claimed until it is answered");
        }
        for _ in 0..4 {
            paced_step(&mut server, &mut hg);
        }
        let fed = 9 + crate::item::Item::Material(MaterialId::Bread).food_value().unwrap() as u8;
        let sp = &server.server.players[slot];
        assert_eq!(sp.combat.hunger, fed, "the server ate it");
        assert_eq!(sp.possession.drops, 0, "and threw nothing");
        assert_eq!(breads(&hg), 0, "the accepted outcome took it here too");
        assert!(!hg.state.joiner_actions.eat_in_flight());
    }

    /// C3b-fix-d (A-L1) — a request queued behind a carry-over and then
    /// discarded unsent (the link closed while edits still waited) releases
    /// its claim: its bread is spendable again, not locked for the rest of
    /// the session.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_queued_request_discarded_unsent_releases_its_claim() {
        use crate::item::{Item, ItemStack, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("discarded-unsent");
        hg.state.players[0].inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 1)));
        hg.state.players[0].hotbar_slot = 0;
        catch_up(&mut server, &mut hg);
        queue_tagged_breaks(&mut hg, &server, slot, 80);
        hg.state.send_eat_request(0);
        paced_step(&mut server, &mut hg);
        let client = hg.state.remote_client.as_mut().expect("joined");
        assert!(client.has_carry_over() && client.has_queued_requests(), "edits still wait, and the Eat behind them");
        assert!(hg.state.joiner_actions.eat_in_flight());
        client.state = crate::remote_client::ConnectionState::Failed("the link went".to_string());
        hg.state.network_send_input();
        assert!(
            !hg.state.remote_client.as_ref().expect("still held").has_queued_requests(),
            "discarded: nothing will send it"
        );
        assert!(!hg.state.joiner_actions.eat_in_flight(), "its claim is released");
        let p = &hg.state.players[0];
        let bread = Item::Material(MaterialId::Bread);
        assert!(hg.state.joiner_actions.can_spend(&p.inventory, &p.crafting_ui, &bread, 1), "the bread is free");
    }

    /// C1 — a joiner's break through its REAL client: the survival break arm
    /// mines the block under its feet, tags it (`InputPacket.mined`) and takes
    /// nothing itself (`break_drops::take_yield`); the server yields the break
    /// and grants it (`InventoryGrant`). The client ends with exactly one
    /// stack of the drop — two if it still granted itself, none if the server
    /// didn't.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_break_is_granted_once_by_the_server() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-joiner-break");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-joiner-break-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Miner", 0),
            None,
        ));
        for _ in 0..5 {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        }
        assert!(hg.state.joined());

        // Dirt under the joiner's feet in both worlds, its server body where
        // its client stands, an empty hand, looking straight down.
        let p = hg.state.players[0].player.pos;
        let cell = (p.x.floor() as i32, (p.y - 0.5).floor() as i32, p.z.floor() as i32);
        hg.state.world.set_block(cell.0, cell.1, cell.2, crate::block::DIRT);
        server.server.world.set_block(cell.0, cell.1, cell.2, crate::block::DIRT);
        let cs = crate::chunk::CHUNK_SIZE as i32;
        server.server.loaded_columns.insert((cell.0.div_euclid(cs), cell.2.div_euclid(cs)));
        let slot = server.server.players.len() - 1;
        hg.state.players[0].inventory = crate::inventory::Inventory::new();
        hg.state.players[0].camera.pitch = -std::f32::consts::FRAC_PI_2 + 0.01;
        hg.state.input.cursor_captured = true;
        hg.state.input.left_held = true;
        let dirt = crate::item::Item::Block(crate::block::DIRT);
        let held = |hg: &HeadlessGame| -> u32 {
            hg.state.players[0]
                .inventory
                .slots_iter()
                .flatten()
                .filter(|s| s.item == dirt)
                .map(|s| u32::from(s.count))
                .sum()
        };

        let mut broke = false;
        for _ in 0..400 {
            let body = &mut server.server.players[slot].player;
            body.pos = p;
            body.velocity = glam::Vec3::ZERO;
            // One tick per frame, whatever the wall clock did.
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
            server.tick();
            if hg.state.world.get_block(cell.0, cell.1, cell.2) != crate::block::DIRT {
                broke = true;
                break;
            }
        }
        assert!(broke, "the joiner's client mined the dirt");
        hg.state.input.left_held = false;
        for _ in 0..10 {
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
            server.tick();
        }
        assert_eq!(
            server.server.world.get_block(cell.0, cell.1, cell.2),
            crate::block::AIR,
            "the server took the break"
        );
        assert_eq!(held(&hg), 1, "exactly one dirt: the server's grant, not the client's own");
        assert_eq!(server.server.players[slot].possession.breaks, 1);
    }

    /// Review D2b LOW-8 — the lending host's REAL death sweep, breeding step
    /// and species dispatch (`GameState::tick`, where they run while the host
    /// lends): a mob a joiner hit dies beside the host's own player and the
    /// kill goes to the joiner as a `KillEvent`, not to the host; two cows a
    /// joiner fed breed and the joiner is told (`Bred`); and a bee whose sting
    /// is due at the joiner's slot lands on its server body (the
    /// `species_bodies` index and the `target_slot >= num_local` routing).
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_lending_hosts_sweep_and_breeding_credit_the_joiner() {
        use crate::transport::ClientTransport;
        isolate_saves();
        let name = "harness-lend-credit";
        let mut hg = HeadlessGame::boot_into_world(name);
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let seed = hg.state.biome_gen.seed;
        let hs = crate::hosted_server::HostedServer::start_host(
            1,
            name.to_string(),
            seed,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
            crate::hosted_server::HostWorld::Lent,
        )
        .expect("lending host starts");
        hg.state.hosted_server = Some(hs);
        let client = hg.state.hosted_server.as_mut().unwrap().attach_test_remote();
        let req = crate::remote_client::build_join_request_guest("Visitor", 0);
        client.send_to_server(&crate::protocol::serialize_packet(
            crate::protocol::PacketType::JoinRequest,
            &req,
        ));
        hg.hosted_ticks(8);
        let server = &hg.state.hosted_server.as_ref().unwrap().server;
        let slot = server.players.len() - 1;
        let joiner = crate::combat::Attacker::Remote {
            slot,
            generation: server.players[slot].attach_gen,
        };

        // A chicken the joiner hit dies right beside the host's own player.
        let host = hg.state.players[0].player.pos;
        let chicken = crate::entity::spawn_mob(
            &mut hg.state.ecs,
            crate::mob::MobType::Chicken,
            host + glam::Vec3::new(0.5, 0.0, 0.0),
        );
        hg.state.ecs.insert_one(chicken, crate::combat::LastAttacker(joiner)).unwrap();
        hg.state.ecs.get::<&mut crate::combat::Health>(chicken).unwrap().current = 0.0;
        // Two cows the joiner fed, side by side.
        for dx in [2.0, 2.5] {
            let cow = crate::entity::spawn_mob(
                &mut hg.state.ecs,
                crate::mob::MobType::Cow,
                host + glam::Vec3::new(dx, 0.0, 2.0),
            );
            hg.state
                .ecs
                .insert_one(cow, crate::breeding::InLove { until_tick: u64::MAX, fed_by: Some(joiner) })
                .unwrap();
        }
        // A bee whose sting at the joiner's slot is due now.
        hg.state.difficulty = "normal".to_string();
        let bee = crate::entity::spawn_mob(
            &mut hg.state.ecs,
            crate::mob::MobType::Bee,
            host + glam::Vec3::new(-3.0, 1.0, 0.0),
        );
        hg.state.ecs.get::<&mut crate::bee_ai::BeeData>(bee).unwrap().state =
            crate::bee_ai::BeeAiState::Sting { target_id: slot as u64, until_tick: 0 };
        let joiner_hp = hg.state.hosted_server.as_ref().unwrap().server.players[slot].combat.health;
        let before = hg.state.players[0].kill_counter.get(&crate::mob::MobType::Chicken).copied();
        hg.hosted_ticks(3);
        assert!(hg.state.ecs.get::<&crate::bee_ai::BeeData>(bee).is_err(), "the bee stung and died");
        assert!(
            hg.state.hosted_server.as_ref().unwrap().server.players[slot].combat.health < joiner_hp,
            "the sting landed on the joiner's server body"
        );
        assert!(hg.state.ecs.get::<&crate::combat::Health>(chicken).is_err(), "swept");
        assert_eq!(
            hg.state.players[0].kill_counter.get(&crate::mob::MobType::Chicken).copied(),
            before,
            "the host's player is not credited with the joiner's kill"
        );
        let (mut kills, mut bred) = (0, false);
        while let Some(pkt) = client.try_recv_from_server() {
            let Some((ptype, payload)) = crate::protocol::deserialize_header(&pkt) else { continue };
            match ptype {
                crate::protocol::PacketType::KillEvent => kills += 1,
                crate::protocol::PacketType::PlayerEvent => {
                    let e: crate::protocol::PlayerEventPacket =
                        crate::protocol::safe_deserialize(payload).unwrap();
                    bred |= matches!(e.event, crate::protocol::PlayerEventType::Bred { .. });
                }
                _ => {}
            }
        }
        assert_eq!(kills, 1, "the joiner is told of its kill");
        assert!(bred, "the joiner is told of the breed it fed");
    }

    /// Review D2b B1 — through the REAL per-frame player loop: a bucket held
    /// with a ready cow in the crosshair for many frames WITHOUT a right-click
    /// milks nothing (before the fix it milked on the first such frame), and
    /// companion food held on a wild cat rolls no tame and is never eaten. One
    /// right-click then milks the cow — so it really was in reach all along.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_milking_and_companion_taming_wait_for_a_right_click() {
        use crate::item::{ItemStack, MaterialId};
        use crate::mob::MobType;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-b1-click");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        hg.frames(5);
        hg.state.players[0].camera.yaw = 0.0;
        hg.state.players[0].camera.pitch = 0.0;
        // Held 1.5 blocks straight ahead of the player, at its feet, every
        // frame (mob AI would walk it out of the crosshair).
        let pin = |hg: &mut HeadlessGame, e: hecs::Entity| {
            let slot = &hg.state.players[0];
            let at = slot.player.pos + slot.camera.forward() * 1.5;
            hg.state.ecs.get::<&mut crate::entity::Position>(e).unwrap().0 = at;
            hg.state.ecs.get::<&mut crate::entity::Velocity>(e).unwrap().0 = glam::Vec3::ZERO;
        };
        let count = |hg: &HeadlessGame, m: MaterialId| hg.state.players[0].inventory.count_material(m);
        let hot = hg.state.players[0].hotbar_slot;
        hg.state.input.right_held = false;

        hg.state.players[0].inventory.set_slot(hot, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        let cow = crate::entity::spawn_mob(&mut hg.state.ecs, MobType::Cow, glam::Vec3::ZERO);
        for _ in 0..120 {
            pin(&mut hg, cow);
            hg.frames(1);
        }
        assert_eq!(count(&hg, MaterialId::Bucket), 1, "no click: the bucket stays");
        assert_eq!(count(&hg, MaterialId::MilkBucket), 0, "no click: no milk");

        hg.state.players[0].inventory.set_slot(hot, Some(ItemStack::new_material(MaterialId::RawFish, 16)));
        let _ = hg.state.ecs.despawn(cow);
        let cat = crate::entity::spawn_mob(&mut hg.state.ecs, MobType::Cat, glam::Vec3::ZERO);
        for _ in 0..120 {
            pin(&mut hg, cat);
            hg.frames(1);
        }
        assert_eq!(count(&hg, MaterialId::RawFish), 16, "no click: no food eaten");
        assert!(crate::tameable::pet_owner_of(&hg.state.ecs, cat).is_none(), "and no tame");

        // The control: one right-click milks a cow held where the first was.
        let _ = hg.state.ecs.despawn(cat);
        hg.state.players[0].inventory.set_slot(hot, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        let cow = crate::entity::spawn_mob(&mut hg.state.ecs, MobType::Cow, glam::Vec3::ZERO);
        pin(&mut hg, cow);
        hg.state.players[0].place_cooldown = 0;
        hg.state.input.cursor_captured = true;
        hg.state.input.right_held = true;
        hg.frames(1);
        hg.state.input.right_held = false;
        assert_eq!(count(&hg, MaterialId::MilkBucket), 1, "the click milks it");
        assert_eq!(count(&hg, MaterialId::Bucket), 0);
    }

    /// FU3 (FU1 verify N2) — a bucket on a tethered cow that isn't ready to
    /// milk: the refusal skips every later MOB arm for that click, so the
    /// Lead-detach arm no longer unties the cow (it did: a refused milk fell
    /// through to it), and the click goes on to the block behind — here a
    /// fence gate at eye height, which opens. (Since FU4b a bucket aimed at a
    /// pond fills from it instead — see the Plumber test below — but a gate
    /// stops the bucket's fluid ray as it stops any other.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_refused_milk_leaves_a_tethered_cow_tied_and_reaches_the_block() {
        use crate::item::{ItemStack, MaterialId};
        use crate::mob::MobType;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-n2-refused-milk");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        hg.frames(5);
        hg.state.players[0].camera.yaw = 0.0;
        hg.state.players[0].camera.pitch = 0.0;
        let hot = hg.state.players[0].hotbar_slot;
        hg.state.players[0].inventory.set_slot(hot, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        // A closed gate at eye height, three blocks ahead, in clear air.
        let (eye, fwd) = (hg.state.players[0].player.eye_pos(), hg.state.players[0].camera.forward());
        let g = eye + fwd * 3.0;
        let gate = (g.x.floor() as i32, g.y.floor() as i32, g.z.floor() as i32);
        for k in 1..=2 {
            let c = eye + fwd * k as f32;
            hg.state.world.set_block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32, crate::block::AIR);
        }
        hg.state.world.set_block(gate.0, gate.1, gate.2, crate::block::OAK_FENCE_GATE);
        hg.state.world.set_meta(gate, 0);
        // A cow just milked, on a Lead, held 1.5 blocks ahead at the feet.
        let cow = crate::entity::spawn_mob(&mut hg.state.ecs, MobType::Cow, glam::Vec3::ZERO);
        let now = hg.state.tick_counter;
        hg.state.ecs.get::<&mut crate::animal_products::AnimalProductState>(cow).unwrap().last_action_tick = Some(now);
        hg.state
            .ecs
            .insert_one(cow, crate::tether::Tethered { target: crate::tether::TetherTarget::Player(0) })
            .unwrap();
        let pin = |hg: &mut HeadlessGame| {
            let slot = &hg.state.players[0];
            let at = slot.player.pos + slot.camera.forward() * 1.5;
            hg.state.ecs.get::<&mut crate::entity::Position>(cow).unwrap().0 = at;
            hg.state.ecs.get::<&mut crate::entity::Velocity>(cow).unwrap().0 = glam::Vec3::ZERO;
        };
        hg.state.input.right_held = false;
        pin(&mut hg);
        hg.frames(1);
        assert_eq!(hg.state.players[0].target_block, Some([gate.0, gate.1, gate.2]), "aimed at the gate");

        pin(&mut hg);
        hg.state.players[0].place_cooldown = 0;
        hg.state.input.cursor_captured = true;
        hg.state.input.right_held = true;
        hg.frames(1);
        hg.state.input.right_held = false;
        assert!(
            hg.state.toast.as_ref().is_some_and(|(msg, _)| msg.contains("needs time")),
            "the click reached the cow and was refused: {:?}",
            hg.state.toast.as_ref().map(|(m, _)| m)
        );
        assert!(
            hg.state.ecs.get::<&crate::tether::Tethered>(cow).is_ok(),
            "the refused milk did not untie the cow"
        );
        let count = |m| hg.state.players[0].inventory.count_material(m);
        assert_eq!((count(MaterialId::Bucket), count(MaterialId::MilkBucket), count(MaterialId::Lead)), (1, 0, 0));
        assert!(
            crate::block_shape::is_open(hg.state.world.meta_at(gate.0, gate.1, gate.2)),
            "the click went on to the block: the gate opened"
        );
    }

    /// FU4b (FU3 verify M3) — an empty bucket aimed at a pond fills from it:
    /// the aim ray passes through water for every other item, so a bucket
    /// could never target the water it was meant to fill and The Plumber Trial
    /// (kit: one bucket, arena: only water) could not be finished. Runs the
    /// Trial's real arena and checks its first step completes.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_an_empty_bucket_fills_from_a_pond_and_the_plumbers_first_step_completes() {
        use crate::item::MaterialId;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-bucket-pond");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        hg.frames(5);
        let def = crate::scenario::load_scenario_def(include_bytes!("../assets/scenarios/explorer-bucket.json"))
            .expect("the Plumber parses");
        let arena = def.arena.clone().expect("the Plumber has an arena");
        hg.state.start_scenario(def);
        hg.state.apply_arena_setup(&arena);
        let count = |hg: &HeadlessGame, m: MaterialId| hg.state.players[0].inventory.count_material(m);
        assert_eq!(count(&hg, MaterialId::Bucket), 1, "the kit is one empty bucket");
        assert_eq!(hg.state.scenario.as_ref().unwrap().objective_progress(), Some((0, 2)));

        // The arena's pond is a source two blocks east of the player's feet.
        let feet = hg.state.players[0].player.pos;
        let pond = (feet.x.floor() as i32 + 2, feet.y.floor() as i32, feet.z.floor() as i32);
        assert_eq!(hg.state.world.get_block(pond.0, pond.1, pond.2), crate::block::WATER);
        assert!(hg.state.water.is_source(pond.0, pond.1, pond.2), "the arena registers it as a source");
        // Look at it from the eye.
        let eye = hg.state.players[0].player.eye_pos();
        let d = glam::Vec3::new(pond.0 as f32 + 0.5, pond.1 as f32 + 0.5, pond.2 as f32 + 0.5) - eye;
        hg.state.players[0].camera.yaw = (-d.x).atan2(-d.z);
        hg.state.players[0].camera.pitch = d.y.atan2(d.x.hypot(d.z));
        let hot = hg.state.players[0].hotbar_slot;
        assert!(
            hg.state.players[0].inventory.hotbar_slot(hot).is_some_and(|s| s.item == crate::item::Item::Material(MaterialId::Bucket)),
            "the bucket is in the hand"
        );
        hg.state.input.right_held = false;
        hg.frames(1);
        assert_ne!(
            hg.state.players[0].target_block,
            Some([pond.0, pond.1, pond.2]),
            "the ordinary aim ray still passes through water"
        );

        hg.state.players[0].place_cooldown = 0;
        hg.state.input.cursor_captured = true;
        hg.state.input.right_held = true;
        hg.frames(1);
        hg.state.input.right_held = false;
        assert_eq!(
            (count(&hg, MaterialId::Bucket), count(&hg, MaterialId::WaterBucket)),
            (0, 1),
            "the bucket filled from the pond"
        );
        assert_eq!(hg.state.world.get_block(pond.0, pond.1, pond.2), crate::block::AIR, "the source is taken");
        assert!(!hg.state.water.is_source(pond.0, pond.1, pond.2));
        assert_eq!(
            hg.state.scenario.as_ref().unwrap().objective_progress(),
            Some((1, 2)),
            "The Plumber's first step (a fill) is done"
        );
    }

    /// FU4b (FU3 verify Q9 row 3) — a JOINED client's own machine sims push no
    /// edits to the server: its power tick, dispensers, pistons and lightning
    /// fire are the server's to run. C3b-1 — and it runs no furnace sweep or
    /// hopper tick either (its furnace screen draws the server's furnace). A
    /// fuelled steam generator beside a lamp, a loaded furnace and a powered
    /// dispenser in the joiner's own world, ticked: nothing queued, the lamp
    /// stays dark (no power tick), the furnace stays unlit (no sweep; FU4b
    /// let it light locally), and a hopper between two chests moves nothing.
    /// The control: the same rig on a client that has joined nobody queues
    /// the lamp, the generator and the furnace.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joined_clients_machines_push_no_edits() {
        use crate::item::{ItemStack, MaterialId};
        isolate_saves();

        // Rig the machines in front of the player, on the world's surface.
        fn rig(hg: &mut HeadlessGame) -> ((i32, i32, i32), (i32, i32, i32), (i32, i32, i32)) {
            let p = hg.state.players[0].player.pos;
            let (x, y, z) = (p.x.floor() as i32 + 6, p.y.floor() as i32 + 3, p.z.floor() as i32 + 6);
            let (gen_pos, lamp, furnace) = ((x, y, z), (x + 1, y, z), (x + 3, y, z));
            let w = &mut hg.state.world;
            for dx in -1..=5 {
                for dz in -1..=1 {
                    w.set_block(x + dx, y - 1, z + dz, crate::block::STONE);
                    for dy in 0..=1 {
                        w.set_block(x + dx, y + dy, z + dz, crate::block::AIR);
                    }
                }
            }
            for (pos, blk, kind) in [
                (gen_pos, crate::block::STEAM_GENERATOR, crate::power::PowerDeviceKind::SteamGenerator),
                (lamp, crate::block::ELECTRIC_LAMP, crate::power::PowerDeviceKind::ElectricLamp),
            ] {
                w.set_block(pos.0, pos.1, pos.2, blk);
                w.block_entities.insert(
                    pos,
                    crate::world::BlockEntityData::PowerDevice(crate::power::PowerDeviceData::new(kind, crate::meta::Facing::Up)),
                );
            }
            if let Some(d) = w.power_device_at_mut(gen_pos) {
                d.fuel = Some(crate::furnace::FurnaceData {
                    fuel: Some(ItemStack::new_material(MaterialId::Coal, 8)),
                    ..Default::default()
                });
            }
            w.set_block(furnace.0, furnace.1, furnace.2, crate::block::FURNACE);
            w.insert_furnace(
                furnace,
                crate::furnace::FurnaceData {
                    input: Some(ItemStack::new_material(MaterialId::Copper, 1)),
                    fuel: Some(ItemStack::new_material(MaterialId::GreenLog, 1)),
                    ..Default::default()
                },
            );
            (gen_pos, lamp, furnace)
        }
        let queued = |hg: &HeadlessGame, cell: (i32, i32, i32)| {
            hg.state.pending_block_changes.iter().any(|bc| (bc.x, bc.y, bc.z) == cell)
        };

        // The control: a client that joined nobody runs and queues all three.
        let mut solo = HeadlessGame::boot_into_world("harness-q9-solo");
        solo.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        solo.frames(3);
        let (gen_pos, lamp, furnace) = rig(&mut solo);
        solo.state.pending_block_changes.clear();
        solo.ticks(8);
        assert!(queued(&solo, lamp), "control: the lamp lights and is queued");
        assert!(queued(&solo, gen_pos), "control: the generator lights and is queued");
        assert!(queued(&solo, furnace), "control: the furnace lights and is queued");

        // A joined client.
        let mut hg = HeadlessGame::boot_into_world("harness-q9-joined");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-q9-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Sparky", 0),
            None,
        ));
        for _ in 0..5 {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        }
        assert!(hg.state.joined());
        let (gen_pos, lamp, furnace) = rig(&mut hg);
        hg.state.pending_block_changes.clear();
        hg.ticks(8);
        assert_eq!(
            hg.state.world.get_block(lamp.0, lamp.1, lamp.2),
            crate::block::ELECTRIC_LAMP,
            "no power tick on a joined client: the lamp stays dark in its own copy"
        );
        assert_eq!(hg.state.world.get_block(gen_pos.0, gen_pos.1, gen_pos.2), crate::block::STEAM_GENERATOR);
        assert!(
            !queued(&hg, lamp) && !queued(&hg, gen_pos),
            "the joiner pushed power changes: {:?}",
            hg.state.pending_block_changes.iter().map(|b| (b.x, b.y, b.z)).collect::<Vec<_>>()
        );
        // C3b-1 — a joined client runs no furnace sweep at all now: its
        // furnace screen draws the server's furnace (was: "its own furnace
        // still cooks", FU4b).
        assert_eq!(
            hg.state.world.get_block(furnace.0, furnace.1, furnace.2),
            crate::block::FURNACE,
            "no furnace sweep on a joined client: its own copy doesn't cook"
        );
        assert!(!queued(&hg, furnace), "and no lit flip is an edit for the server");

        // C3b-1 — nor a hopper tick: a hopper between two chests of its own
        // copy moves nothing.
        let p = hg.state.players[0].player.pos;
        let (x, y, z) = (p.x.floor() as i32 - 6, p.y.floor() as i32 + 3, p.z.floor() as i32 - 6);
        let w = &mut hg.state.world;
        w.set_block(x, y, z, crate::block::CHEST);
        w.set_block(x, y + 1, z, crate::block::HOPPER);
        w.set_block(x, y + 2, z, crate::block::CHEST);
        let mut above = crate::chest::ChestData::new();
        above.slots[0] = Some(ItemStack::new_block(crate::block::STONE, 4));
        w.insert_chest((x, y + 2, z), above);
        w.insert_chest((x, y, z), crate::chest::ChestData::new());
        hg.ticks(crate::hopper::HOPPER_INTERVAL_TICKS as u32 * 3);
        assert!(
            hg.state.world.chest_at((x, y, z)).is_some_and(|c| c.slots.iter().all(Option::is_none)),
            "no hopper tick on a joined client"
        );
    }

    /// C3b-1 — a joiner's right-click on a chest asks the server
    /// (`OpenContainer`) and creates no private copy; the screen opens on the
    /// server's answer, drawn from a mirror of the server's chest; closing it
    /// (as Esc does) tells the server, which closes it too.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_opens_the_servers_chest_not_its_own() {
        use crate::item::ItemStack;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-joiner-chest");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-joiner-chest-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Keeper", 0),
            None,
        ));
        let step = |server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame| {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        };
        for _ in 0..5 {
            step(&mut server, &mut hg);
        }
        let slot = server.server.players.len() - 1;
        let body = server.server.players[slot].player.pos;
        let cell = [body.x.floor() as i32 + 1, body.y.floor() as i32, body.z.floor() as i32];
        let key = (cell[0], cell[1], cell[2]);
        server.server.world.set_block(cell[0], cell[1], cell[2], crate::block::CHEST);
        hg.state.world.set_block(cell[0], cell[1], cell[2], crate::block::CHEST);
        let mut real = crate::chest::ChestData::new();
        real.slots[2] = Some(ItemStack::new_block(crate::block::STONE, 6));
        server.server.world.insert_chest(key, real);
        hg.state.players[0].player.pos = body;
        hg.state.request_shared_open(0, cell);
        assert!(hg.state.world.chest_at(key).is_none(), "the right-click created no private copy");
        // Step until the answer is in (a request can wait behind the
        // client's edits on a loaded machine), at most 20 rounds.
        for _ in 0..20 {
            step(&mut server, &mut hg);
            if hg.state.players[0].shared_container.is_some() {
                break;
            }
        }
        let p = &hg.state.players[0];
        assert_eq!(p.open_chest, Some(key), "the screen opened on the server's answer");
        let mirror = p.shared_container.as_ref().expect("a mirror of the server's chest");
        let crate::container_window::ContainerData::Chest(m) = &mirror.contents else { panic!("a chest") };
        assert_eq!(m.slots[2], Some(ItemStack::new_block(crate::block::STONE, 6)), "the server's contents");
        assert!(hg.state.world.chest_at(key).is_none(), "still no private copy");
        assert_eq!(server.server.players[slot].open_container, Some(cell));

        // Close it as Esc does: the server closes it too.
        hg.state.players[0].open_chest = None;
        for _ in 0..20 {
            step(&mut server, &mut hg);
            if server.server.players[slot].open_container.is_none() {
                break;
            }
        }
        assert!(hg.state.players[0].shared_container.is_none(), "the mirror went with the screen");
        assert_eq!(server.server.players[slot].open_container, None, "the close reached the server");
    }

    /// C2a — a joiner's hunger, eating and sleep through its REAL client: the
    /// joined slot runs no metabolism of its own (its drain timer never
    /// moves), it shows the hunger the server sends (`own_hunger`), and a
    /// right-click with bread sends an `ItemAction::Eat` and eats, feeds and
    /// heals nothing locally — the bread goes and the hunger rises only when
    /// the server's answer and state arrive. A sleep request the server
    /// accepts sets the client's spawn point at the bed.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_hunger_is_the_servers_and_it_eats_and_sleeps_by_asking() {
        use crate::item::{ItemStack, MaterialId};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-joiner-eat");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-joiner-eat-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Eater", 0),
            None,
        ));
        let step = |server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame| {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        };
        for _ in 0..5 {
            step(&mut server, &mut hg);
        }
        assert!(hg.state.joined());

        // No metabolism of its own.
        hg.state.players[0].combat.hunger_drain_ticks = 0;
        for _ in 0..30 {
            step(&mut server, &mut hg);
        }
        assert_eq!(hg.state.players[0].combat.hunger_drain_ticks, 0, "a joined slot runs no hunger drain");

        // Its hunger is the server's.
        server.server.players.last_mut().unwrap().combat.hunger = 9;
        for _ in 0..3 {
            step(&mut server, &mut hg);
        }
        assert_eq!(hg.state.players[0].combat.hunger, 9, "own_hunger is applied");

        // A right-click with bread: a request, nothing local.
        let hot = hg.state.players[0].hotbar_slot;
        hg.state.players[0].inventory.set_slot(hot, Some(ItemStack::new_material(MaterialId::Bread, 2)));
        let health = hg.state.players[0].combat.health;
        hg.state.players[0].place_cooldown = 0;
        hg.state.input.cursor_captured = true;
        hg.state.input.right_held = true;
        hg.frames(1);
        hg.ticks(1);
        hg.state.input.right_held = false;
        let bread = |hg: &HeadlessGame| hg.state.players[0].inventory.count_material(MaterialId::Bread);
        assert_eq!(bread(&hg), 2, "nothing eaten before the server answers");
        assert_eq!(hg.state.players[0].combat.hunger, 9, "no local feed");
        assert_eq!(hg.state.players[0].combat.health, health, "no local heal");
        for _ in 0..4 {
            step(&mut server, &mut hg);
        }
        let fed = 9 + crate::item::Item::Material(MaterialId::Bread).food_value().unwrap() as u8;
        assert_eq!(server.server.players.last().unwrap().combat.hunger, fed, "the server fed its body");
        assert_eq!(bread(&hg), 1, "the accepted outcome took one bread");
        assert_eq!(hg.state.players[0].combat.hunger, fed, "and the server's hunger arrived");

        // A sleep the server accepts sets our spawn point at the bed.
        server.server.world_time = 0;
        let body = server.server.players.last().unwrap().player.pos;
        let bed = [body.x.floor() as i32 + 1, body.y.floor() as i32, body.z.floor() as i32];
        server.server.world.set_block(bed[0], bed[1], bed[2], crate::block::BED);
        hg.state.send_sleep_request(0, bed);
        for _ in 0..4 {
            step(&mut server, &mut hg);
        }
        assert_eq!(
            hg.state.players[0].spawn_pos,
            crate::item_actions::bed_spawn(bed),
            "the accepted sleep set our spawn"
        );
        assert_eq!(
            server.server.players.last().unwrap().spawn_pos,
            crate::item_actions::bed_spawn(bed),
            "and the server's"
        );
    }

    /// C3b-2 — a joiner's block use through its REAL client: the server's
    /// hive reaches the joiner's world as a view; a bucket on it asks the
    /// server (`send_block_use`) and changes nothing locally — the bucket
    /// goes, the Honey Jar comes and the hive's honey drops only when the
    /// server's outcome, grant and view arrive.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_scoops_the_servers_hive_by_asking() {
        use crate::item::{ItemStack, MaterialId};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-joiner-hive");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-joiner-hive-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Beekeeper", 0),
            None,
        ));
        let step = |server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame| {
            server.tick();
            hg.frames(1);
            hg.ticks(1);
            hg.state.network_send_input();
        };
        for _ in 0..5 {
            step(&mut server, &mut hg);
        }
        assert!(hg.state.joined());

        let body = server.server.players.last().unwrap().player.pos;
        let cell = [body.x.floor() as i32 + 1, body.y.floor() as i32, body.z.floor() as i32];
        let pos = (cell[0], cell[1], cell[2]);
        server.server.world.set_block(cell[0], cell[1], cell[2], crate::block::BEE_HIVE);
        server.server.world.insert_hive(pos, crate::bee_hive::HiveData { bees_inside: 0, honey_level: 2 });
        // The joiner holds the hive block too (as its column's push would
        // give it: the server's `set_block` above broadcasts nothing). Since
        // C3b-2-fix (M4) a view applies only where the client's block is its
        // kind.
        hg.state.world.set_block(cell[0], cell[1], cell[2], crate::block::BEE_HIVE);
        for _ in 0..6 {
            step(&mut server, &mut hg);
        }
        assert_eq!(hg.state.world.hive_at(pos).map(|h| h.honey_level), Some(2), "the server's hive, shown to the joiner");

        let hot = hg.state.players[0].hotbar_slot;
        hg.state.players[0].inventory.set_slot(hot, Some(ItemStack::new_material(MaterialId::Bucket, 1)));
        hg.state.send_block_use(0, cell, crate::block_use::UseKind::Hive);
        let count = |hg: &HeadlessGame, m| hg.state.players[0].inventory.count_material(m);
        assert_eq!(count(&hg, MaterialId::Bucket), 1, "nothing taken before the server answers");
        assert_eq!(hg.state.world.hive_at(pos).unwrap().honey_level, 2, "its copy untouched");
        for _ in 0..4 {
            step(&mut server, &mut hg);
        }
        assert_eq!(server.server.world.hive_at(pos).unwrap().honey_level, 1, "the server scooped its real hive");
        assert_eq!(count(&hg, MaterialId::Bucket), 0, "the accepted outcome took the bucket");
        assert_eq!(count(&hg, MaterialId::HoneyBottle), 1, "the grant brought the jar");
        assert_eq!(hg.state.world.hive_at(pos).unwrap().honey_level, 1, "and the hive's view came back");
    }

    // ─── C3b-2-fix ─────────────────────────────────────────────────────────

    /// Aim player 0's camera from its eye at `at` (`camera::forward_from`'s
    /// convention: yaw 0 looks along -z).
    fn aim_at(hg: &mut HeadlessGame, at: glam::Vec3) {
        let d = (at - hg.state.players[0].player.eye_pos()).normalize();
        let cam = &mut hg.state.players[0].camera;
        cam.yaw = (-d.x).atan2(-d.z);
        cam.pitch = d.y.asin();
    }

    /// Step a joined client and its server until every request it sent is
    /// answered (at most 20 steps), then two more for the grants behind.
    fn settle(server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame) {
        for _ in 0..20 {
            harness_step(server, hg);
            if hg.state.joiner_actions.len() == 0 {
                break;
            }
        }
        assert_eq!(hg.state.joiner_actions.len(), 0, "every request answered");
        harness_step(server, hg);
        harness_step(server, hg);
    }

    /// One right-click through the REAL click arms (one frame).
    fn right_click(hg: &mut HeadlessGame) {
        hg.state.players[0].place_cooldown = 0;
        hg.state.input.cursor_captured = true;
        hg.state.input.right_held = true;
        hg.frames(1);
        hg.state.input.right_held = false;
    }

    /// Player 0's feet cell, and a clear stone-floored pad around it in its
    /// own world (and in `server`'s, when joined).
    fn clear_pad(hg: &mut HeadlessGame, server: Option<&mut crate::hosted_server::HostedServer>) -> [i32; 3] {
        let p = hg.state.players[0].player.pos;
        let feet = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
        let mut worlds: Vec<&mut crate::world::World> = vec![&mut hg.state.world];
        if let Some(server) = server {
            worlds.push(&mut server.server.world);
        }
        for w in worlds {
            for x in feet[0] - 4..=feet[0] + 4 {
                for z in feet[2] - 4..=feet[2] + 4 {
                    w.set_block(x, feet[1] - 1, z, crate::block::STONE);
                    for y in feet[1]..feet[1] + 4 {
                        w.set_block(x, y, z, crate::block::AIR);
                    }
                }
            }
        }
        feet
    }

    /// C3b-2-fix (M1) — a joiner right-clicks an empty item frame with its
    /// ONLY diamond block (the REAL frame arm: the request claims it), then,
    /// inside the round trip, right-clicks the floor with the same slot (the
    /// REAL place arm): the placement waits for the claim and does nothing.
    /// The server ends with one diamond block, in the frame. (It used to place
    /// it too: one diamond in the frame and one in the world.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_placement_waits_for_the_claim_of_a_frame_use_in_flight() {
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("frame-then-place");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let body = server.server.players[slot].player.pos;
        assert!(body.distance(hg.state.players[0].player.pos) < 1.0, "the server body stands where the client does");
        let diamond = crate::item::ItemStack::new_block(crate::block::DIAMOND_BLOCK, 1);
        hg.state.players[0].inventory.set_slot(0, Some(diamond.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(diamond));
        hg.state.players[0].hotbar_slot = 0;
        // An empty frame two blocks to the +x at eye height, in both worlds.
        let frame = [feet[0] + 2, feet[1] + 1, feet[2]];
        hg.state.world.set_block(frame[0], frame[1], frame[2], crate::block::ITEM_FRAME);
        server.server.world.set_block(frame[0], frame[1], frame[2], crate::block::ITEM_FRAME);
        harness_step(&mut server, &mut hg);
        // 1. The frame, through the real arm.
        aim_at(&mut hg, glam::Vec3::new(frame[0] as f32 + 0.5, frame[1] as f32 + 0.5, frame[2] as f32 + 0.5));
        right_click(&mut hg);
        // 2. Inside the round trip: the floor two blocks ahead (-z), top face.
        let place = [feet[0], feet[1], feet[2] - 2];
        aim_at(&mut hg, glam::Vec3::new(place[0] as f32 + 0.5, place[1] as f32 + 0.02, place[2] as f32 + 0.5));
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(place[0], place[1], place[2]), crate::block::AIR, "nothing placed: the diamond is claimed");
        let diamonds = |inv: &crate::inventory::Inventory| -> u32 {
            let diamond = crate::item::Item::Block(crate::block::DIAMOND_BLOCK);
            inv.slots_iter().flatten().filter(|s| s.item == diamond).map(|s| u32::from(s.count)).sum()
        };
        assert_eq!(diamonds(&hg.state.players[0].inventory), 1, "still in hand until the frame's outcome");
        settle(&mut server, &mut hg);
        let pos = (frame[0], frame[1], frame[2]);
        let framed = server.server.world.item_frame_at(pos).and_then(|f| f.item.as_ref().map(|s| s.item.clone()));
        assert_eq!(framed, Some(crate::item::Item::Block(crate::block::DIAMOND_BLOCK)), "the frame has it");
        assert_eq!(server.server.world.get_block(place[0], place[1], place[2]), crate::block::AIR, "and the world doesn't");
        assert_eq!(diamonds(&hg.state.players[0].inventory), 0);
        assert_eq!(diamonds(&server.server.players[slot].inventory), 0);
    }

    /// C3b-2-fix (L1) — single-player feeds a campfire the last plank of a
    /// hotbar slot with auto-refill on: the slot is refilled from the bag, as
    /// it was before C3b-2 (a block fuel is taken as a placement is).
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_the_last_plank_fed_to_a_campfire_refills_the_slot() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-campfire-refill");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        hg.frames(5);
        let feet = clear_pad(&mut hg, None);
        let fire = [feet[0], feet[1] + 1, feet[2] - 2];
        hg.state.world.set_block(fire[0], fire[1], fire[2], crate::block::CAMPFIRE_UNLIT);
        let inv = &mut hg.state.players[0].inventory;
        *inv = crate::inventory::Inventory::new();
        inv.auto_refill = true;
        inv.set_slot(0, Some(crate::item::ItemStack::new_block(crate::block::OAK_PLANKS, 1)));
        inv.set_slot(20, Some(crate::item::ItemStack::new_block(crate::block::OAK_PLANKS, 10)));
        hg.state.players[0].hotbar_slot = 0;
        aim_at(&mut hg, glam::Vec3::new(fire[0] as f32 + 0.5, fire[1] as f32 + 0.3, fire[2] as f32 + 0.5));
        right_click(&mut hg);
        let fuel = hg.state.world.campfire_at((fire[0], fire[1], fire[2])).map(|c| c.fuel_ticks);
        assert!(fuel.is_some_and(|f| f > 0), "the plank burns: {fuel:?}");
        let inv = &hg.state.players[0].inventory;
        assert_eq!(inv.slot(0).map(|s| s.count), Some(10), "refilled from the bag");
        assert!(inv.slot(20).is_none());
    }

    /// C3b-2-fix (L8) — each of the five blocks right-clicked through a
    /// joined client's REAL click arm: the arm asks the server and goes no
    /// further — its own copy of the block changes nothing and the hand pays
    /// nothing until the outcome — and the server's real block takes the use.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_click_on_each_of_the_five_blocks_asks_the_server() {
        use crate::item::{Item, ItemStack, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("five-uses");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let cell = [feet[0], feet[1] + 1, feet[2] - 2];
        let pos = (cell[0], cell[1], cell[2]);
        let cases = [
            (crate::block::COMPOSTER, Item::Material(MaterialId::WheatSeeds)),
            (crate::block::DRYING_RACK, Item::Material(MaterialId::GreenLog)),
            (crate::block::CAMPFIRE_UNLIT, Item::Material(MaterialId::Coal)),
            (crate::block::ITEM_FRAME, Item::Material(MaterialId::Stick)),
            (crate::block::BEE_HIVE, Item::Material(MaterialId::Bucket)),
        ];
        for (b, held) in cases {
            for w in [&mut hg.state.world, &mut server.server.world] {
                w.set_block(cell[0], cell[1], cell[2], b);
            }
            if b == crate::block::BEE_HIVE {
                server.server.world.insert_hive(pos, crate::bee_hive::HiveData { bees_inside: 0, honey_level: 2 });
            }
            let stack = ItemStack { item: held.clone(), count: 2 };
            hg.state.players[0].inventory.set_slot(0, Some(stack.clone()));
            server.server.players[slot].inventory.set_slot(0, Some(stack));
            hg.state.players[0].hotbar_slot = 0;
            for _ in 0..3 {
                harness_step(&mut server, &mut hg);
            }
            let ours = |hg: &HeadlessGame| {
                let w = &hg.state.world;
                (w.composter_at(pos).map(|c| c.input.is_some()), w.drying_racks.get(&pos).map(|r| r.occupied_slots()), w.campfire_at(pos).map(|c| c.fuel_ticks), w.item_frame_at(pos).map(|f| f.is_empty()))
            };
            let before = ours(&hg);
            aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.3, cell[2] as f32 + 0.5));
            right_click(&mut hg);
            let held_now = |hg: &HeadlessGame| hg.state.players[0].inventory.slot(0).map_or(0, |s| s.count);
            assert_eq!(held_now(&hg), 2, "{b}: the hand pays nothing before the server answers");
            assert_eq!(ours(&hg), before, "{b}: the joiner's own copy is untouched (the arm went no further)");
            settle(&mut server, &mut hg);
            let w = &server.server.world;
            let took = match b {
                crate::block::COMPOSTER => w.composter_at(pos).is_some_and(|c| c.input.is_some()),
                crate::block::DRYING_RACK => w.drying_racks.get(&pos).is_some_and(|r| r.occupied_slots() == 1),
                crate::block::CAMPFIRE_UNLIT => w.campfire_at(pos).is_some_and(|c| c.fuel_ticks > 0),
                crate::block::ITEM_FRAME => w.item_frame_at(pos).is_some_and(|f| !f.is_empty()),
                _ => w.hive_at(pos).is_some_and(|h| h.honey_level == 1),
            };
            assert!(took, "{b}: the server's real block took the use");
            assert_eq!(held_now(&hg), 1, "{b}: and the outcome took one from the hand");
            // Clear the cell for the next case (both worlds).
            for w in [&mut hg.state.world, &mut server.server.world] {
                w.set_block(cell[0], cell[1], cell[2], crate::block::AIR);
                let _ = crate::block_use::take_on_break(w, pos, b, false);
                w.block_entities.remove(&pos);
            }
        }
    }

    // ─── C3c-1 ─────────────────────────────────────────────────────────────

    /// The ground items in an ECS that hold `item`, units.
    fn ground_units(ecs: &hecs::World, item: &crate::item::Item) -> u32 {
        ecs.query::<&crate::entity::ItemEntity>()
            .iter()
            .filter(|(_, it)| &it.stack.item == item)
            .map(|(_, it)| u32::from(it.stack.count))
            .sum()
    }

    /// C3c-1 — a joiner with a full bag fills a bucket from a pond through
    /// the REAL fill arm (`try_bucket_fill`): the use rides its edit with its
    /// tag, the server mirrors it on its copy (one bucket spent, the water
    /// bucket made), and the water bucket that fits neither side is spilled
    /// by the SERVER, from its copy, as one real ground item the joiner sees —
    /// the joined client spills nothing of its own. The world's buckets are
    /// conserved, and the copy is the client's window.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_full_joiners_bucket_fill_is_spilled_by_the_server_alone() {
        use crate::item::{Item, ItemStack, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("full-fill");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let pond = [feet[0] + 2, feet[1], feet[2]];
        hg.state.world.set_block(pond[0], pond[1], pond[2], crate::block::WATER);
        hg.state.water.add_source(pond[0], pond[1], pond[2]);
        server.server.world.set_block(pond[0], pond[1], pond[2], crate::block::WATER);
        server.server.water.add_source(pond[0], pond[1], pond[2]);
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Bucket, 2)));
            for k in 1..36 {
                inv.set_slot(k, Some(ItemStack::new_block(crate::block::STONE, 64)));
            }
        }
        hg.state.players[0].hotbar_slot = 0;
        harness_step(&mut server, &mut hg);
        let water_bucket = Item::Material(MaterialId::WaterBucket);
        aim_at(&mut hg, glam::Vec3::new(pond[0] as f32 + 0.5, pond[1] as f32 + 0.5, pond[2] as f32 + 0.5));
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(pond[0], pond[1], pond[2]), crate::block::AIR, "the client filled its bucket");
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(1), "one bucket spent");
        assert_eq!(ground_units(&hg.state.ecs, &water_bucket), 0, "a joined client spills nothing of its own");
        for _ in 0..4 {
            harness_step(&mut server, &mut hg);
        }
        assert_eq!(server.server.world.get_block(pond[0], pond[1], pond[2]), crate::block::AIR);
        let sp = &server.server.players[slot];
        assert_eq!((sp.possession.use_mirrored, sp.possession.use_mismatch), (1, 0), "the fill mirrored");
        let slots = |inv: &crate::inventory::Inventory| inv.slots_iter().map(|s| s.cloned()).collect::<Vec<_>>();
        assert_eq!(slots(&sp.inventory), slots(&hg.state.players[0].inventory), "the copy is the client's window");
        assert_eq!(ground_units(&server.server.ecs, &water_bucket), 1, "the server spilled one real water bucket");
        assert_eq!(ground_units(&hg.state.ecs, &water_bucket), 0);
        let (kind, id) = crate::inventory::item_to_ref(&water_bucket).to_wire();
        assert!(
            hg.state.remote_items.iter().any(|it| it.item.to_wire() == (kind, id)),
            "the joiner sees the server's water bucket"
        );
        let inv = &hg.state.players[0].inventory;
        let buckets = u32::from(inv.count_material(MaterialId::Bucket))
            + u32::from(inv.count_material(MaterialId::WaterBucket))
            + ground_units(&server.server.ecs, &water_bucket);
        assert_eq!(buckets, 2, "the world's buckets are conserved");
    }

    /// C3c-1 — a joiner places a door through the REAL place arm. With no
    /// headroom it is refused BEFORE the door is taken: nothing taken,
    /// nothing placed, nothing sent (it used to be placed, undone and
    /// refunded on the client alone). With headroom both halves reach the
    /// server — the top half as the door's tagged use — and the server's copy
    /// paid for one door.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_door_is_checked_first_and_reaches_the_server_whole() {
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("door");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let door = crate::item::ItemStack::new_block(crate::block::OAK_DOOR, 2);
        hg.state.players[0].inventory.set_slot(0, Some(door.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(door));
        hg.state.players[0].hotbar_slot = 0;
        let place = [feet[0], feet[1], feet[2] - 2];
        let top = [place[0], place[1] + 1, place[2]];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(top[0], top[1], top[2], crate::block::STONE);
        }
        harness_step(&mut server, &mut hg);
        let floor_top = glam::Vec3::new(place[0] as f32 + 0.5, place[1] as f32 + 0.02, place[2] as f32 + 0.5);
        aim_at(&mut hg, floor_top);
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(place[0], place[1], place[2]), crate::block::AIR, "no room: nothing placed");
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(2), "and nothing taken");
        assert!(hg.state.toast.as_ref().is_some_and(|(t, _)| t.starts_with("No room for the door")));
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        assert_eq!(server.server.world.get_block(place[0], place[1], place[2]), crate::block::AIR);
        assert_eq!(sp.inventory.slot(0).map(|s| s.count), Some(2), "the server's copy paid nothing");
        assert_eq!((sp.possession.matched, sp.possession.mismatched, sp.possession.unchecked), (0, 0, 0), "nothing was sent");
        // Headroom: a whole door.
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(top[0], top[1], top[2], crate::block::AIR);
        }
        harness_step(&mut server, &mut hg);
        aim_at(&mut hg, floor_top);
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(top[0], top[1], top[2]), crate::block::OAK_DOOR, "the client's door is whole");
        for _ in 0..4 {
            harness_step(&mut server, &mut hg);
        }
        let w = &server.server.world;
        assert_eq!(w.get_block(place[0], place[1], place[2]), crate::block::OAK_DOOR, "the bottom half");
        assert_eq!(w.get_block(top[0], top[1], top[2]), crate::block::OAK_DOOR, "the top half reached the server");
        assert!(crate::block_shape::door_is_top(w.meta_at(top[0], top[1], top[2])));
        let sp = &server.server.players[slot];
        assert_eq!((sp.possession.matched, sp.possession.mismatched), (1, 0), "the bottom half paid for the door");
        assert_eq!((sp.possession.use_mirrored, sp.possession.use_mismatch), (1, 0), "the top half is the door's use");
        assert_eq!(sp.inventory.slot(0).map(|s| s.count), Some(1));
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(1));
    }

    /// C3c-1 — single-player: a Plot Marker whose claim would overlap another
    /// player's plot is refused BEFORE it is taken (it used to be placed,
    /// undone and refunded): the toast, and the marker still in hand.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_conflicting_plot_marker_is_refused_before_it_is_taken() {
        use crate::item::{ItemStack, MaterialId};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-plot-check-first");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        hg.frames(5);
        let feet = clear_pad(&mut hg, None);
        let place = [feet[0], feet[1], feet[2] - 2];
        // Someone else's plot, its edge four blocks to the +x: the new claim
        // would overlap it, though the marker's own cell is outside it.
        let foreign = crate::plot::PlotData::from_marker(crate::plot::PlotOwner::LocalPlayer(1), place[0] + 20, place[1], place[2]);
        hg.state.world.plots.push(foreign);
        let inv = &mut hg.state.players[0].inventory;
        *inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::PlotMarkerItem, 1)));
        hg.state.players[0].hotbar_slot = 0;
        aim_at(&mut hg, glam::Vec3::new(place[0] as f32 + 0.5, place[1] as f32 + 0.02, place[2] as f32 + 0.5));
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(place[0], place[1], place[2]), crate::block::AIR, "nothing placed");
        assert_eq!(hg.state.players[0].inventory.count_material(MaterialId::PlotMarkerItem), 1, "nothing taken");
        assert_eq!(hg.state.world.plots.len(), 1, "no claim");
        assert!(hg.state.toast.as_ref().is_some_and(|(t, _)| t == "Too close to another player's plot."));
    }

    // ─── C3c-2 ─────────────────────────────────────────────────────────────

    /// C3c-2 — a joined client's bow, cart, rod and flint and steel, each
    /// through its REAL click arm: the arm asks the server and changes
    /// nothing of its own — no projectile or cart in its own ECS, no ammo,
    /// cart or wear spent, no fire lit, no catch rolled — until the outcome;
    /// the server's world holds the real arrow (owned by the joiner), the
    /// real cart and the lit fire, and the outcome pays on both copies. The
    /// joined client sees the arrow through its projectile mirror.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_bow_cart_rod_and_flint_ask_the_server() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::{Item, ItemStack, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("c3c2-uses");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let give = |hg: &mut HeadlessGame, server: &mut crate::hosted_server::HostedServer, k: usize, stack: ItemStack| {
            hg.state.players[0].inventory.set_slot(k, Some(stack.clone()));
            server.server.players[slot].inventory.set_slot(k, Some(stack));
        };
        let count = |hg: &HeadlessGame, m: MaterialId| -> u32 {
            hg.state.players[0].inventory.slots_iter().flatten().filter(|s| s.item == Item::Material(m)).map(|s| u32::from(s.count)).sum()
        };
        let durability = |hg: &HeadlessGame, k: usize| match hg.state.players[0].inventory.slot(k).map(|s| &s.item) {
            Some(Item::Tool(t)) => t.durability,
            _ => 0,
        };
        let own_projectiles = |hg: &HeadlessGame| hg.state.ecs.query::<&crate::entity::ProjectileEntity>().iter().count();

        // The bow, aimed high over the pad.
        let bow = Tool::new(ToolType::Bow, ToolMaterial::Wood);
        give(&mut hg, &mut server, 0, ItemStack::new_tool(bow));
        give(&mut hg, &mut server, 9, ItemStack::new_material(MaterialId::Arrow, 2));
        hg.state.players[0].hotbar_slot = 0;
        aim_at(&mut hg, glam::Vec3::new(feet[0] as f32 + 0.5, feet[1] as f32 + 30.0, feet[2] as f32 - 6.0));
        harness_step(&mut server, &mut hg);
        right_click(&mut hg);
        assert_eq!(own_projectiles(&hg), 0, "a joined client spawns no arrow of its own");
        assert_eq!(count(&hg, MaterialId::Arrow), 2, "nor spends one before the server answers");
        settle(&mut server, &mut hg);
        let owners: Vec<_> = server
            .server
            .ecs
            .query::<&crate::entity::ProjectileEntity>()
            .iter()
            .map(|(_, p)| p.owner.as_ref().map(|s| s.who))
            .collect();
        assert_eq!(owners.len(), 1, "the server's real arrow: {owners:?}");
        assert!(matches!(owners[0], Some(crate::combat::Attacker::Remote { slot: s, .. }) if s == slot));
        assert_eq!(own_projectiles(&hg), 0);
        assert!(!hg.state.remote_projectiles.is_empty(), "the joiner sees it through its mirror");
        assert_eq!(count(&hg, MaterialId::Arrow), 1, "the outcome took the arrow");
        assert_eq!(durability(&hg, 0), bow.durability - 1, "and wore the bow");

        // A cart on a rail.
        let rail = [feet[0], feet[1], feet[2] - 2];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(rail[0], rail[1], rail[2], crate::rail::TRACK);
        }
        give(&mut hg, &mut server, 1, ItemStack::new_material(MaterialId::WoodCart, 1));
        hg.state.players[0].hotbar_slot = 1;
        aim_at(&mut hg, glam::Vec3::new(rail[0] as f32 + 0.5, rail[1] as f32 + 0.1, rail[2] as f32 + 0.5));
        harness_step(&mut server, &mut hg);
        right_click(&mut hg);
        assert_eq!(hg.state.ecs.query::<&crate::cart::CartData>().iter().count(), 0, "no cart of its own");
        assert_eq!(count(&hg, MaterialId::WoodCart), 1);
        settle(&mut server, &mut hg);
        assert!(crate::cart::cart_here(&server.server.ecs, (rail[0], rail[1], rail[2])), "the server's real cart");
        assert_eq!(count(&hg, MaterialId::WoodCart), 0, "the outcome took the cart");

        // A rod at a pond: the line waits for the server's bite.
        let eye = hg.state.players[0].player.eye_pos();
        let pond = [feet[0] + 3, eye.y.floor() as i32, feet[2]];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(pond[0], pond[1], pond[2], crate::block::WATER);
        }
        server.server.water.add_source(pond[0], pond[1], pond[2]);
        let rod = Tool::new(ToolType::FishingRod, ToolMaterial::Wood);
        give(&mut hg, &mut server, 2, ItemStack::new_tool(rod));
        hg.state.players[0].hotbar_slot = 2;
        aim_at(&mut hg, glam::Vec3::new(pond[0] as f32 + 0.5, pond[1] as f32 + 0.5, pond[2] as f32 + 0.5));
        harness_step(&mut server, &mut hg);
        right_click(&mut hg);
        assert_eq!(hg.state.players[0].fishing.map(|l| l.catch_at_tick), Some(u64::MAX), "the line waits for the server's word");
        settle(&mut server, &mut hg);
        let line = hg.state.players[0].fishing.expect("the server accepted the cast");
        assert!(line.catch_at_tick < u64::MAX && !line.hooked, "its bite is the server's: {line:?}");
        assert!(server.server.players[slot].fishing.is_some(), "the server holds the cast");
        right_click(&mut hg);
        assert!(hg.state.players[0].fishing.is_none(), "an early reel takes the line in");
        settle(&mut server, &mut hg);
        assert!(server.server.players[slot].fishing.is_none());
        assert_eq!(durability(&hg, 2), rod.durability, "an early reel costs nothing");

        // Flint and steel on a fuelled unlit campfire.
        let fire = [feet[0] - 2, feet[1], feet[2]];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(fire[0], fire[1], fire[2], crate::block::CAMPFIRE_UNLIT);
            w.insert_campfire((fire[0], fire[1], fire[2]), crate::campfire::CampfireData { fuel_ticks: 4_000, ..Default::default() });
        }
        let flint = Tool::new(ToolType::FlintAndSteel, ToolMaterial::Iron);
        give(&mut hg, &mut server, 3, ItemStack::new_tool(flint));
        hg.state.players[0].hotbar_slot = 3;
        aim_at(&mut hg, glam::Vec3::new(fire[0] as f32 + 0.5, fire[1] as f32 + 0.3, fire[2] as f32 + 0.5));
        harness_step(&mut server, &mut hg);
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(fire[0], fire[1], fire[2]), crate::block::CAMPFIRE_UNLIT, "nothing lit here");
        assert_eq!(durability(&hg, 3), flint.durability, "nothing worn before the outcome");
        settle(&mut server, &mut hg);
        assert_eq!(server.server.world.get_block(fire[0], fire[1], fire[2]), crate::block::CAMPFIRE, "the server lit its fire");
        assert_eq!(hg.state.world.get_block(fire[0], fire[1], fire[2]), crate::block::CAMPFIRE, "and the joiner sees it");
        assert_eq!(durability(&hg, 3), flint.durability - 1, "the outcome wore the flint");
        let sp = &server.server.players[slot];
        assert_eq!(sp.possession.mismatched, 0, "the server's copy paid every take");
        assert_eq!(sp.possession.wear_mismatch, 0, "and wore every tool");
    }

    /// C3c-2 — single-player's reel through the REAL rod arm: a rod at 0
    /// durability breaks on a catch (it used to stay at 0 for ever), and a
    /// catch a full bag can't hold drops at the player (it used to vanish).
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_rod_at_zero_breaks_and_a_full_bags_catch_drops() {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::{Item, ItemStack, MaterialId};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-c3c2-reel");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        hg.frames(5);
        let feet = clear_pad(&mut hg, None);
        let eye = hg.state.players[0].player.eye_pos();
        let pond = [feet[0] + 3, eye.y.floor() as i32, feet[2]];
        hg.state.world.set_block(pond[0], pond[1], pond[2], crate::block::WATER);
        hg.state.water.add_source(pond[0], pond[1], pond[2]);
        let inv = &mut hg.state.players[0].inventory;
        *inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_tool(Tool { durability: 0, ..Tool::new(ToolType::FishingRod, ToolMaterial::Wood) })));
        for k in 1..36 {
            inv.set_slot(k, Some(ItemStack::new_block(crate::block::STONE, 64)));
        }
        hg.state.players[0].hotbar_slot = 0;
        aim_at(&mut hg, glam::Vec3::new(pond[0] as f32 + 0.5, pond[1] as f32 + 0.5, pond[2] as f32 + 0.5));
        right_click(&mut hg);
        assert!(hg.state.players[0].fishing.is_some(), "cast");
        hg.state.players[0].fishing = Some(crate::fishing::FishingLine { catch_at_tick: 0, hooked: true });
        right_click(&mut hg);
        assert!(hg.state.players[0].fishing.is_none(), "reeled in");
        assert!(hg.state.players[0].inventory.slot(0).is_none(), "the rod at 0 broke");
        let dropped: u32 = hg
            .state
            .ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter(|(_, it)| {
                matches!(it.stack.item, Item::Material(MaterialId::RawFish | MaterialId::Bone | MaterialId::Leather))
            })
            .map(|(_, it)| u32::from(it.stack.count))
            .sum();
        assert!(dropped >= 1, "the catch dropped at the player");
    }

    // ─── C3c-1-fix ─────────────────────────────────────────────────────────

    /// Units of `item` in `inv`'s 36 slots.
    fn held_units(inv: &crate::inventory::Inventory, item: &crate::item::Item) -> u32 {
        inv.slots_iter().flatten().filter(|s| &s.item == item).map(|s| u32::from(s.count)).sum()
    }

    /// A water or lava source at `cell` in a client's world and the server's.
    fn fluid_source_both(hg: &mut HeadlessGame, server: &mut crate::hosted_server::HostedServer, cell: [i32; 3], b: crate::block::BlockId) {
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(cell[0], cell[1], cell[2], b);
        }
        if b == crate::block::WATER {
            hg.state.water.add_source(cell[0], cell[1], cell[2]);
            server.server.water.add_source(cell[0], cell[1], cell[2]);
        } else {
            hg.state.lava.add_source(cell[0], cell[1], cell[2]);
            server.server.lava.add_source(cell[0], cell[1], cell[2]);
        }
    }

    /// C3c-1-fix (M-1) — a joiner right-clicks a hive with honey holding its
    /// ONLY bucket (the REAL hive arm: the request claims it), then, inside the
    /// round trip, right-clicks a pond with the same slot (the REAL fill arm):
    /// the fill waits for the claim and does nothing. The server ends with one
    /// honey bottle and no water bucket. (It used to fill too: a honey bottle
    /// AND a water bucket from one bucket.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_fill_waits_for_the_claim_of_a_hive_use_in_flight() {
        use crate::item::{Item, ItemStack, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("hive-then-fill");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let bucket = ItemStack::new_material(MaterialId::Bucket, 1);
        hg.state.players[0].inventory.set_slot(0, Some(bucket.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(bucket));
        hg.state.players[0].hotbar_slot = 0;
        let hive = [feet[0] - 2, feet[1] + 1, feet[2]];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(hive[0], hive[1], hive[2], crate::block::BEE_HIVE);
        }
        server.server.world.insert_hive((hive[0], hive[1], hive[2]), crate::bee_hive::HiveData { bees_inside: 0, honey_level: 2 });
        let pond = [feet[0] + 2, feet[1], feet[2]];
        fluid_source_both(&mut hg, &mut server, pond, crate::block::WATER);
        harness_step(&mut server, &mut hg);
        // 1. The hive, through the real arm: the bucket is claimed.
        aim_at(&mut hg, glam::Vec3::new(hive[0] as f32 + 0.5, hive[1] as f32 + 0.5, hive[2] as f32 + 0.5));
        right_click(&mut hg);
        // 2. Inside the round trip: the pond.
        aim_at(&mut hg, glam::Vec3::new(pond[0] as f32 + 0.5, pond[1] as f32 + 0.5, pond[2] as f32 + 0.5));
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(pond[0], pond[1], pond[2]), crate::block::WATER, "no fill: the bucket is claimed");
        let water_bucket = Item::Material(MaterialId::WaterBucket);
        assert_eq!(held_units(&hg.state.players[0].inventory, &water_bucket), 0);
        settle(&mut server, &mut hg);
        assert_eq!(server.server.world.get_block(pond[0], pond[1], pond[2]), crate::block::WATER, "the pond is untouched on the server");
        let honey = Item::Material(MaterialId::HoneyBottle);
        let sp = &server.server.players[slot];
        assert_eq!(held_units(&sp.inventory, &honey), 1, "one honey bottle on the server");
        assert_eq!(held_units(&sp.inventory, &water_bucket), 0, "and no water bucket");
        assert_eq!(held_units(&sp.inventory, &Item::Material(MaterialId::Bucket)), 0);
        let inv = &hg.state.players[0].inventory;
        assert_eq!((held_units(inv, &honey), held_units(inv, &water_bucket)), (1, 0), "the client agrees");
    }

    /// A foreign plot (seat 1's) over the column of `cell`, on the server only:
    /// a joiner is never sent plots.
    fn foreign_plot_on_server(server: &mut crate::hosted_server::HostedServer, cell: [i32; 3]) {
        let plot = crate::plot::PlotData::from_marker(crate::plot::PlotOwner::LocalPlayer(1), cell[0], cell[1] - 3, cell[2]);
        server.server.world.plots.push(plot);
    }

    /// C3c-1-fix (M-4) — a joiner holds right-click with a bucket on a rubber
    /// log inside someone else's plot (it is never sent plots, so it can't
    /// know). Each tap is refused by the server, sent back, and the joiner is
    /// told: it undoes the tap from its own record (the rubber taken back) and
    /// shows why. It ends with no rubber and its bucket, and the log is
    /// untapped everywhere. Then the same for a bucket of a protected lava
    /// source: the bucket back, no lava bucket, the source standing.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_refused_tap_and_fill_are_undone() {
        use crate::item::{Item, ItemStack, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("refused-uses");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let log = [feet[0], feet[1] + 1, feet[2] - 2];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(log[0], log[1], log[2], crate::block::RUBBER_LOG);
        }
        foreign_plot_on_server(&mut server, log);
        let bucket = ItemStack::new_material(MaterialId::Bucket, 1);
        hg.state.players[0].inventory.set_slot(0, Some(bucket.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(bucket.clone()));
        hg.state.players[0].hotbar_slot = 0;
        harness_step(&mut server, &mut hg);
        let rubber = Item::Material(MaterialId::Rubber);
        let mut tapped = 0;
        for _ in 0..4 {
            aim_at(&mut hg, glam::Vec3::new(log[0] as f32 + 0.5, log[1] as f32 + 0.5, log[2] as f32 + 0.5));
            if hg.state.world.get_block(log[0], log[1], log[2]) == crate::block::RUBBER_LOG {
                right_click(&mut hg);
                if hg.state.world.get_block(log[0], log[1], log[2]) == crate::block::RUBBER_LOG_TAPPED {
                    tapped += 1;
                }
            }
            // Until the send-back lands (it queues behind the join's chunk
            // pushes in the joiner's stream), and the notice with it.
            for _ in 0..60 {
                harness_step(&mut server, &mut hg);
                if hg.state.world.get_block(log[0], log[1], log[2]) == crate::block::RUBBER_LOG && hg.state.sent_uses.len() == 0 {
                    break;
                }
            }
        }
        assert!(tapped >= 2, "held down, the client tapped again after each send-back: {tapped}");
        assert_eq!(hg.state.world.get_block(log[0], log[1], log[2]), crate::block::RUBBER_LOG, "untapped on the client");
        assert_eq!(server.server.world.get_block(log[0], log[1], log[2]), crate::block::RUBBER_LOG, "and on the server");
        let inv = &hg.state.players[0].inventory;
        assert_eq!(held_units(inv, &rubber), 0, "no rubber from nothing");
        assert_eq!(inv.slot(0), Some(&bucket), "its bucket kept");
        assert!(hg.state.toast.as_ref().is_some_and(|(t, _)| t == "You can't use that here."), "{:?}", hg.state.toast);
        let sp = &server.server.players[slot];
        assert_eq!(sp.possession.use_edit_refused, tapped);
        assert_eq!(held_units(&sp.inventory, &rubber), 0);

        // A protected lava source.
        let pool = [feet[0] + 2, feet[1], feet[2]];
        fluid_source_both(&mut hg, &mut server, pool, crate::block::LAVA);
        foreign_plot_on_server(&mut server, pool);
        harness_step(&mut server, &mut hg);
        aim_at(&mut hg, glam::Vec3::new(pool[0] as f32 + 0.5, pool[1] as f32 + 0.5, pool[2] as f32 + 0.5));
        right_click(&mut hg);
        let lava_bucket = Item::Material(MaterialId::LavaBucket);
        assert_eq!(held_units(&hg.state.players[0].inventory, &lava_bucket), 1, "the client filled it");
        for _ in 0..60 {
            harness_step(&mut server, &mut hg);
            if hg.state.sent_uses.len() == 0 {
                break;
            }
        }
        let inv = &hg.state.players[0].inventory;
        assert_eq!((held_units(inv, &lava_bucket), inv.slot(0)), (0, Some(&bucket)), "undone: the bucket back");
        assert_eq!(hg.state.world.get_block(pool[0], pool[1], pool[2]), crate::block::LAVA, "the source sent back");
        assert_eq!(server.server.world.get_block(pool[0], pool[1], pool[2]), crate::block::LAVA);
        assert_eq!(held_units(&server.server.players[slot].inventory, &lava_bucket), 0);
    }

    /// Hold the left button on `cell` until the client's own world breaks it
    /// (one tick a frame; the server body held where the client stands).
    fn mine_joined(hg: &mut HeadlessGame, server: &mut crate::hosted_server::HostedServer, slot: usize, cell: [i32; 3]) {
        let p = hg.state.players[0].player.pos;
        aim_at(hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
        hg.state.input.cursor_captured = true;
        hg.state.input.left_held = true;
        let before = hg.state.world.get_block(cell[0], cell[1], cell[2]);
        let mut broke = false;
        for _ in 0..600 {
            let body = &mut server.server.players[slot].player;
            body.pos = p;
            body.velocity = glam::Vec3::ZERO;
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
            server.tick();
            if hg.state.world.get_block(cell[0], cell[1], cell[2]) != before {
                broke = true;
                break;
            }
        }
        hg.state.input.left_held = false;
        assert!(broke, "the joiner's client broke {cell:?}");
        for _ in 0..10 {
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
            server.tick();
        }
    }

    /// C3c-1-fix (M-3) — a joiner breaks one half of a door through the REAL
    /// survival break arm: the other half goes too, on every seat. The server
    /// and another joiner have no half left, the breaker has one door (the
    /// server's grant for the half it mined), and there is no second door
    /// anywhere. (Before, the other half stayed on the server: a floating
    /// half door, and a second door for whoever broke it.) Both halves.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_breaks_a_door_whole_on_every_seat() {
        use crate::item::Item;
        for broken_top in [false, true] {
            isolate_saves();
            let tag = if broken_top { "door-whole-top" } else { "door-whole-bottom" };
            let (mut hg, mut server, slot) = joined_window_client(tag);
            let feet = clear_pad(&mut hg, Some(&mut server));
            // The neighbour: a second joined client, holding the door's column.
            let mut neighbour = crate::remote_client::RemoteClient::from_transport(
                Box::new(server.attach_test_remote()),
                crate::remote_client::build_join_request_guest("Neighbour", 0),
                None,
            );
            for _ in 0..5 {
                harness_step(&mut server, &mut hg);
                neighbour.poll();
            }
            let n_slot = neighbour.player_index().expect("the neighbour joined") as usize;
            let bottom = [feet[0], feet[1], feet[2] - 2];
            let top = [bottom[0], bottom[1] + 1, bottom[2]];
            let cs = crate::chunk::CHUNK_SIZE as i32;
            server.hold_column_for_test(n_slot, (bottom[0].div_euclid(cs), bottom[2].div_euclid(cs)));
            for w in [&mut hg.state.world, &mut server.server.world] {
                w.set_block(bottom[0], bottom[1], bottom[2], crate::block::OAK_DOOR);
                w.set_block(top[0], top[1], top[2], crate::block::OAK_DOOR);
                w.set_meta((top[0], top[1], top[2]), crate::use_edits::door_top_meta(0));
            }
            harness_step(&mut server, &mut hg);
            neighbour.poll();
            neighbour.pending_block_changes.clear();
            let broken = if broken_top { top } else { bottom };
            mine_joined(&mut hg, &mut server, slot, broken);
            neighbour.poll();
            for c in [bottom, top] {
                assert_eq!(hg.state.world.get_block(c[0], c[1], c[2]), crate::block::AIR, "the breaker's world ({tag})");
                assert_eq!(server.server.world.get_block(c[0], c[1], c[2]), crate::block::AIR, "the server ({tag})");
                assert!(
                    neighbour.pending_block_changes.iter().any(|b| (b.x, b.y, b.z, b.new_block) == (c[0], c[1], c[2], crate::block::AIR)),
                    "the neighbour lost {c:?} ({tag})"
                );
            }
            let door = Item::Block(crate::block::OAK_DOOR);
            assert_eq!(held_units(&hg.state.players[0].inventory, &door), 1, "one door in the breaker's bag ({tag})");
            assert_eq!(ground_units(&server.server.ecs, &door), 0, "no second door on the server's ground ({tag})");
            assert_eq!(ground_units(&hg.state.ecs, &door), 0, "nor the client's own ({tag})");
            assert_eq!(server.server.players[slot].possession.breaks, 1, "one break yielded ({tag})");
        }
    }

    /// C3c-1-fix (M-3) — a lending host breaks the bottom half of its door
    /// (the REAL break arm): both halves reach a joiner. (Before, the other
    /// half was cleared in the shared world but never broadcast: the joiner
    /// kept a stale half.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_lending_hosts_door_break_reaches_a_joiner_whole() {
        use crate::transport::ClientTransport;
        isolate_saves();
        let name = "harness-lend-door";
        let mut hg = HeadlessGame::boot_into_world(name);
        hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
        let seed = hg.state.biome_gen.seed;
        let hs = crate::hosted_server::HostedServer::start_host(
            1,
            name.to_string(),
            seed,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
            crate::hosted_server::HostWorld::Lent,
        )
        .expect("lending host starts");
        hg.state.hosted_server = Some(hs);
        assert!(hg.state.sim_lent());
        let client = hg.state.hosted_server.as_mut().unwrap().attach_test_remote();
        let req = crate::remote_client::build_join_request_guest("Visitor", 0);
        client.send_to_server(&crate::protocol::serialize_packet(crate::protocol::PacketType::JoinRequest, &req));
        hg.hosted_ticks(8);
        let feet = clear_pad(&mut hg, None);
        let bottom = [feet[0], feet[1], feet[2] - 2];
        let top = [bottom[0], bottom[1] + 1, bottom[2]];
        hg.state.world.set_block(bottom[0], bottom[1], bottom[2], crate::block::OAK_DOOR);
        hg.state.world.set_block(top[0], top[1], top[2], crate::block::OAK_DOOR);
        hg.state.world.set_meta((top[0], top[1], top[2]), crate::use_edits::door_top_meta(0));
        let hs = hg.state.hosted_server.as_mut().unwrap();
        let joiner = hs.server.players.len() - 1;
        let cs = crate::chunk::CHUNK_SIZE as i32;
        hs.hold_column_for_test(joiner, (bottom[0].div_euclid(cs), bottom[2].div_euclid(cs)));
        hg.hosted_ticks(2);
        while client.try_recv_from_server().is_some() {}
        aim_at(&mut hg, glam::Vec3::new(bottom[0] as f32 + 0.5, bottom[1] as f32 + 0.5, bottom[2] as f32 + 0.5));
        hg.state.input.cursor_captured = true;
        hg.state.input.left_held = true;
        for _ in 0..40 {
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
            if hg.state.world.get_block(bottom[0], bottom[1], bottom[2]) != crate::block::OAK_DOOR {
                break;
            }
        }
        hg.state.input.left_held = false;
        assert_eq!(hg.state.world.get_block(bottom[0], bottom[1], bottom[2]), crate::block::AIR, "the host broke the bottom half");
        assert_eq!(hg.state.world.get_block(top[0], top[1], top[2]), crate::block::AIR, "and its top half went too");
        hg.hosted_ticks(3);
        let mut seen = Vec::new();
        while let Some(pkt) = client.try_recv_from_server() {
            if let Some((crate::protocol::PacketType::StateUpdate, payload)) = crate::protocol::deserialize_header(&pkt)
                && let Ok(s) = crate::protocol::safe_deserialize::<crate::protocol::StateUpdatePacket>(payload)
            {
                seen.extend(s.block_changes);
            }
        }
        for c in [bottom, top] {
            assert!(
                seen.iter().any(|b| (b.x, b.y, b.z, b.new_block) == (c[0], c[1], c[2], crate::block::AIR)),
                "the joiner lost {c:?}: {seen:?}"
            );
        }
    }

    // ─── C3c-3r ────────────────────────────────────────────────────────────

    /// C3c-3r — the toast text on a client now, if any.
    fn toast_text(hg: &HeadlessGame) -> Option<String> {
        hg.state.toast.as_ref().map(|(t, _)| t.clone())
    }

    /// C3c-3r — a latent Plan (the lay-flat kind).
    fn latent_plan() -> crate::item::ItemStack {
        let mut data = crate::plan::PlanData::debug_3x3_stone();
        data.develop_state = crate::plan::DevelopState::Latent { exposure_ticks: 0 };
        crate::item::ItemStack { item: crate::item::Item::Plan(data), count: 1 }
    }

    /// C3c-3r (decision 1) — Q on a held Plan while joined: the Plan stays in
    /// the slot, nothing is thrown, nothing is asked of the server, and the
    /// toast says so. Alone, the same Q throws it.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_plan_is_not_dropped() {
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("plan-q");
        let plan = crate::item::ItemStack { item: crate::item::Item::Plan(crate::plan::PlanData::debug_3x3_stone()), count: 1 };
        hg.state.players[0].inventory.set_slot(0, Some(plan.clone()));
        hg.state.players[0].hotbar_slot = 0;
        harness_step(&mut server, &mut hg);
        let press_q = |hg: &mut HeadlessGame| {
            hg.state.players[0].drop_ready_tick = 0;
            hg.state.input.cursor_captured = true;
            hg.state.input.drop_item = true;
            hg.frames(1);
        };
        press_q(&mut hg);
        assert_eq!(hg.state.players[0].inventory.slot(0), Some(&plan), "the Plan is still in hand");
        assert_eq!(ground_units(&hg.state.ecs, &plan.item), 0, "nothing thrown into the client's own world");
        assert_eq!(hg.state.joiner_actions.len(), 0, "no request sent");
        assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_PLAN_DROP_TOAST));
        harness_step(&mut server, &mut hg);
        assert_eq!(ground_units(&server.server.ecs, &plan.item), 0, "and nothing on the server's ground");
        assert_eq!(server.server.players[slot].possession.drops, 0);
        // The same Q alone throws it.
        hg.state.remote_client = None;
        hg.state.toast = None;
        press_q(&mut hg);
        assert!(hg.state.players[0].inventory.slot(0).is_none(), "alone, the Plan is thrown");
        assert_eq!(ground_units(&hg.state.ecs, &plan.item), 1);
    }

    /// C3c-3r (decision 2) — right-clicking a floor with a Latent Plan while
    /// joined lays nothing: the Plan stays in hand and the floor is bare.
    /// Alone, the same click lays it.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_cannot_lay_a_plan_flat() {
        isolate_saves();
        let (mut hg, mut server, _slot) = joined_window_client("plan-lay");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let floor = (feet[0], feet[1] - 1, feet[2] - 2);
        let plan = latent_plan();
        hg.state.players[0].inventory.set_slot(0, Some(plan.clone()));
        hg.state.players[0].hotbar_slot = 0;
        harness_step(&mut server, &mut hg);
        aim_at(&mut hg, glam::Vec3::new(floor.0 as f32 + 0.5, floor.1 as f32 + 1.02, floor.2 as f32 + 0.5));
        right_click(&mut hg);
        assert_eq!(hg.state.players[0].inventory.slot(0), Some(&plan), "the Plan is still in hand");
        assert!(hg.state.world.face_attachment_at(floor, crate::mesh::Face::Top.index()).is_none(), "nothing laid");
        assert_eq!(hg.state.joiner_actions.len(), 0);
        assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_PLAN_LAY_TOAST));
        // Alone, the same click lays it.
        hg.state.remote_client = None;
        hg.state.toast = None;
        right_click(&mut hg);
        assert!(
            matches!(hg.state.world.face_attachment_at(floor, crate::mesh::Face::Top.index()), Some(crate::world::FaceAttachment::Blueprint(_))),
            "alone, the Plan is laid"
        );
        assert!(hg.state.players[0].inventory.slot(0).is_none(), "and spent");
    }

    /// Hold the left button on `cell` for `frames` frames (the server body
    /// held where the client stands), the break arm's own clock.
    fn strike(hg: &mut HeadlessGame, aim: glam::Vec3, frames: u32) {
        aim_at(hg, aim);
        hg.state.input.cursor_captured = true;
        hg.state.input.left_held = true;
        for _ in 0..frames {
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
        }
        hg.state.input.left_held = false;
        hg.frames(2);
    }

    /// C3c-3r (decision 3, peel) — a joiner's strike on a face that carries
    /// a laid Blueprint lifts nothing: the attachment stays, the block stays,
    /// the bag gains no Plan, and the toast says so. Alone, the same strike
    /// peels it into a Plan.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_cannot_peel_a_blueprint() {
        isolate_saves();
        let (mut hg, mut server, _slot) = joined_window_client("peel");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let floor = (feet[0], feet[1] - 1, feet[2] - 2);
        let top = crate::mesh::Face::Top.index();
        hg.state.world.set_face_attachment(
            floor,
            top,
            crate::world::FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::render_stub(false))),
        );
        harness_step(&mut server, &mut hg);
        let aim = glam::Vec3::new(floor.0 as f32 + 0.5, floor.1 as f32 + 1.02, floor.2 as f32 + 0.5);
        strike(&mut hg, aim, 30);
        assert!(
            matches!(hg.state.world.face_attachment_at(floor, top), Some(crate::world::FaceAttachment::Blueprint(_))),
            "the Blueprint is still laid"
        );
        assert_eq!(hg.state.world.get_block(floor.0, floor.1, floor.2), crate::block::STONE, "and the block under it");
        let plans = hg.state.players[0].inventory.slots_iter().flatten().filter(|s| matches!(s.item, crate::item::Item::Plan(_))).count();
        assert_eq!(plans, 0, "no Plan in the bag");
        assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_BLUEPRINT_LIFT_TOAST));
        // Alone, the same strike peels it.
        hg.state.remote_client = None;
        strike(&mut hg, aim, 30);
        assert!(hg.state.world.face_attachment_at(floor, top).is_none(), "alone, it is peeled");
        let plans = hg.state.players[0].inventory.slots_iter().flatten().filter(|s| matches!(s.item, crate::item::Item::Plan(_))).count();
        assert_eq!(plans, 1, "into a Plan");
    }

    /// C3c-3r (decision 3, break arms) — a joiner breaking a block that
    /// carries a laid Blueprint (on a face it didn't strike) and a wallpaper
    /// gets the wallpaper and no Plan; the Blueprint stays in its world copy.
    /// Both the survival and the creative break arm.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_break_grants_no_plan_for_a_laid_blueprint() {
        use crate::item::Item;
        for creative in [false, true] {
            isolate_saves();
            let tag = if creative { "break-bp-creative" } else { "break-bp-survival" };
            let (mut hg, mut server, slot) = joined_window_client(tag);
            if creative {
                hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
            }
            let feet = clear_pad(&mut hg, Some(&mut server));
            let cell = [feet[0], feet[1], feet[2] - 2];
            let pos = (cell[0], cell[1], cell[2]);
            for w in [&mut hg.state.world, &mut server.server.world] {
                w.set_block(cell[0], cell[1], cell[2], crate::block::STONE);
            }
            // Blueprint on the top face, wallpaper on the west face: the strike
            // is on the front (+z) face, which carries neither.
            hg.state.world.set_face_attachment(
                pos,
                crate::mesh::Face::Top.index(),
                crate::world::FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::render_stub(false))),
            );
            hg.state.world.set_face_attachment(pos, crate::mesh::Face::West.index(), crate::world::FaceAttachment::Wallpaper(crate::block::OAK_PLANKS));
            harness_step(&mut server, &mut hg);
            mine_joined(&mut hg, &mut server, slot, cell);
            assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), crate::block::AIR, "the block broke ({tag})");
            assert!(
                matches!(hg.state.world.face_attachment_at(pos, crate::mesh::Face::Top.index()), Some(crate::world::FaceAttachment::Blueprint(_))),
                "the Blueprint stands in the client's copy ({tag})"
            );
            let inv = &hg.state.players[0].inventory;
            let plans = inv.slots_iter().flatten().filter(|s| matches!(s.item, Item::Plan(_))).count();
            assert_eq!(plans, 0, "no Plan granted ({tag})");
            assert_eq!(held_units(inv, &Item::Block(crate::block::OAK_PLANKS)), 1, "the wallpaper still comes back ({tag})");
            assert!(hg.state.world.face_attachment_at(pos, crate::mesh::Face::West.index()).is_none(), "and is gone from the wall ({tag})");
        }
    }

    /// C3c-3r (decision 4) — the Plan Build panel's Auto choice while joined:
    /// refused, nothing locked, no anchor, no cells; the panel's pending
    /// choice stays so Guided is one click away. Alone it starts a build.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_cannot_auto_build() {
        isolate_saves();
        let (mut hg, mut server, _slot) = joined_window_client("auto-build");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
        let feet = clear_pad(&mut hg, Some(&mut server));
        let anchor = [feet[0], feet[1] - 1, feet[2] - 4];
        let plan = crate::plan::PlanData::debug_3x3_stone();
        hg.state.players[0].pending_build_choice = Some((plan.clone(), anchor, 0));
        let blocks_before = hg.state.world.get_block(anchor[0], anchor[1], anchor[2]);
        hg.state.choose_auto_build(0, &plan, anchor, 0);
        assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_AUTO_BUILD_TOAST));
        assert!(hg.state.world.construction_anchors.is_empty(), "no build started");
        assert_eq!(hg.state.world.get_block(anchor[0], anchor[1], anchor[2]), blocks_before);
        assert!(hg.state.players[0].pending_build_choice.is_some(), "the panel stays for Guided");
        assert_eq!(hg.state.joiner_actions.len(), 0);
        // Alone, Auto starts the build.
        hg.state.remote_client = None;
        hg.state.choose_auto_build(0, &plan, anchor, 0);
        assert_eq!(hg.state.world.construction_anchors.len(), 1, "alone, the build starts");
        assert!(hg.state.players[0].pending_build_choice.is_none());
    }

    /// C3c-3r (decision 5) — right-clicking each economy block while joined
    /// opens nothing and says so; alone, the same click opens it.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_cannot_open_an_economy_block() {
        use crate::block;
        type Open = fn(&crate::player_slot::PlayerSlot) -> bool;
        let cases: [(block::BlockId, Open); 7] = [
            (block::VENDOR_BLOCK, |p| p.open_vendor.is_some()),
            (block::BOUNTY_BOARD, |p| p.open_bounty_board.is_some()),
            (block::TIP_JAR, |p| p.open_tip_jar.is_some()),
            (block::REPAIR_BENCH, |p| p.open_repair_bench.is_some()),
            (block::MARKET_BELL, |p| p.open_market_hub.is_some()),
            (block::AUCTION_BLOCK, |p| p.open_auction.is_some()),
            (block::BAZAAR_BLOCK, |p| p.open_bazaar.is_some()),
        ];
        isolate_saves();
        let (mut hg, mut server, _slot) = joined_window_client("economy-open");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let cell = [feet[0], feet[1], feet[2] - 2];
        let aim = glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.3, cell[2] as f32 + 0.5);
        for (b, is_open) in cases {
            hg.state.world.set_block(cell[0], cell[1], cell[2], b);
            harness_step(&mut server, &mut hg);
            hg.state.toast = None;
            aim_at(&mut hg, aim);
            right_click(&mut hg);
            assert!(!is_open(&hg.state.players[0]), "{b}: nothing opens for a joiner");
            assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_ECONOMY_TOAST), "{b}");
            assert_eq!(hg.state.joiner_actions.len(), 0, "{b}: nothing asked of the server");
            assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), b);
        }
        // The drafting table (commission) too, while still joined.
        hg.state.world.set_block(cell[0], cell[1], cell[2], block::DRAFTING_TABLE);
        hg.state.toast = None;
        aim_at(&mut hg, aim);
        right_click(&mut hg);
        assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_ECONOMY_TOAST), "drafting table");
        // Alone, the same clicks open them. Each block gets a fresh game: an
        // open screen leaves egui holding the pointer, which would swallow the
        // next block's click.
        drop(server);
        drop(hg);
        for (b, is_open) in cases {
            let mut solo = HeadlessGame::boot_into_world(&format!("harness-economy-alone-{b}"));
            solo.state.set_play_mode(crate::play_mode::PlayMode::Survival);
            // The tip jar, auction, bounty board, market hub and bazaar are
            // sats-only screens: `economy::close_sats_only_uis` shuts them every
            // frame while sats are off (the default), so switch them on here.
            solo.state.sats_policy.bitcoin_enabled = true;
            solo.state.players[0].charter_allows_sats = true;
            solo.frames(5);
            let feet = clear_pad(&mut solo, None);
            let cell = [feet[0], feet[1], feet[2] - 2];
            let at = (cell[0], cell[1], cell[2]);
            solo.state.world.set_block(cell[0], cell[1], cell[2], b);
            // The screens close themselves on a block with no data behind it.
            match b {
                block::VENDOR_BLOCK => solo.state.world.insert_vendor(
                    at,
                    crate::vendor::VendorData { owner: Some(crate::vendor::VendorOwner::LocalPlayer(0)), ..Default::default() },
                ),
                block::TIP_JAR => solo.state.world.insert_tip_jar(
                    at,
                    crate::tip_jar::TipJarData {
                        owner: Some(crate::tip_jar::TipJarOwner::LocalPlayer(0)),
                        escrow_sats: 0,
                        last_tip_tick: 0,
                        lifetime_tips_received: 0,
                    },
                ),
                block::AUCTION_BLOCK => solo.state.world.insert_auction(at, crate::auction::AuctionData::new(crate::auction::AuctionOwner::LocalPlayer(0))),
                block::MARKET_BELL => solo.state.world.market_hubs.push(crate::market_hub::MarketHubData::from_bell(
                    crate::market_hub::HubOwner::LocalPlayer(0),
                    cell[0],
                    cell[1],
                    cell[2],
                )),
                _ => {}
            }
            // Some of these blocks are thin: try a few heights until the ray
            // lands on the block itself (a miss with an empty hand does nothing).
            for dy in [0.3, 0.5, 0.8, 0.1] {
                let aim = glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + dy, cell[2] as f32 + 0.5);
                aim_at(&mut solo, aim);
                right_click(&mut solo);
                if is_open(&solo.state.players[0]) {
                    break;
                }
            }
            assert!(
                is_open(&solo.state.players[0]),
                "{b}: alone, it opens (toast {:?}, target {:?}, cell {cell:?}, block there {}, cursor {}, cooldown {}, pad hit {:?})",
                toast_text(&solo),
                solo.state.players[0].target_block,
                solo.state.world.get_block(cell[0], cell[1], cell[2]),
                solo.state.input.cursor_captured,
                solo.state.players[0].place_cooldown,
                solo.state.players[0].target_face
            );
        }
    }

    /// C3c-3r (decision 6) — a laid Blueprint attachment on a joined client
    /// does not develop in the sun; alone, the same ticks advance it.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joined_clients_attachment_does_not_develop() {
        isolate_saves();
        let (mut hg, mut server, _slot) = joined_window_client("develop");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let floor = (feet[0], feet[1] - 1, feet[2] - 2);
        let top = crate::mesh::Face::Top.index();
        hg.state.world.set_face_attachment(
            floor,
            top,
            crate::world::FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::render_stub(false))),
        );
        let exposure = |hg: &HeadlessGame| match hg.state.world.face_attachment_at(floor, top) {
            Some(crate::world::FaceAttachment::Blueprint(p)) => match p.develop_state {
                crate::plan::DevelopState::Latent { exposure_ticks } => Some(exposure_ticks),
                crate::plan::DevelopState::Developed => None,
            },
            _ => None,
        };
        hg.state.world_time = 12000;
        hg.state.world_time_step = 0;
        hg.ticks(20);
        assert_eq!(exposure(&hg), Some(0), "joined: no exposure counted");
        // Alone, the same ticks advance it (so the sun reaches the cell).
        hg.state.remote_client = None;
        hg.ticks(20);
        assert!(exposure(&hg).is_some_and(|e| e > 0), "alone it develops: {:?}", exposure(&hg));
    }

    // ─── C3c-3a ────────────────────────────────────────────────────────────

    /// C3c-3a — a stone wall block two blocks ahead (-z) at eye height, in
    /// both worlds, clear of the floor (so an art capture finds it alone).
    fn wall_ahead(hg: &mut HeadlessGame, server: &mut crate::hosted_server::HostedServer, feet: [i32; 3]) -> [i32; 3] {
        let wall = [feet[0], feet[1] + 1, feet[2] - 2];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(wall[0], wall[1], wall[2], crate::block::STONE);
        }
        wall
    }

    /// C3c-3a — aim player 0's camera at `wall`'s +z face.
    fn aim_at_wall(hg: &mut HeadlessGame, wall: [i32; 3]) {
        aim_at(hg, glam::Vec3::new(wall[0] as f32 + 0.5, wall[1] as f32 + 0.5, wall[2] as f32 + 0.9));
    }

    /// C3c-3a — the Plan in `slot` of `inv`, if one is there.
    fn plan_in(inv: &crate::inventory::Inventory, slot: usize) -> Option<crate::plan::PlanData> {
        match inv.slot(slot).map(|s| &s.item) {
            Some(crate::item::Item::Plan(p)) => Some(p.clone()),
            _ => None,
        }
    }

    /// C3c-3a — a joiner's art capture through the REAL arm (Blueprint Paper
    /// on a wall): the client mints the Plan into its first empty slot and
    /// takes its last paper; the server's copy gains the Plan's marker
    /// placeholder in the same slot and loses the paper. Then the Plan moves
    /// between slots and into the hotbar: no window mismatch.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_art_capture_mints_a_plan_the_server_tracks_by_marker() {
        use crate::item::ItemStack;
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("art-capture");
        let feet = clear_pad(&mut hg, Some(&mut server));
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            inv.set_slot(0, Some(ItemStack::new_block(crate::block::BLUEPRINT_PAPER, 1)));
            inv.set_slot(1, Some(ItemStack::new_block(crate::block::STONE, 5)));
        }
        hg.state.players[0].hotbar_slot = 0;
        let wall = wall_ahead(&mut hg, &mut server, feet);
        harness_step(&mut server, &mut hg);
        aim_at_wall(&mut hg, wall);
        right_click(&mut hg);
        let p = &hg.state.players[0];
        let plan = plan_in(&p.inventory, 2).expect("the art Plan, in the first empty slot");
        assert_eq!(plan.kind, crate::plan::PlanKind::Art);
        assert!(p.inventory.slot(0).is_none(), "the last paper went");
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        let held = plan_in(&sp.inventory, 2).expect("the server's copy holds it in the same slot");
        assert_eq!(held.marker, Some(crate::plan::marker(&plan)), "as its marker placeholder");
        assert!(held.cells.is_empty(), "body-less");
        assert!(sp.inventory.slot(0).is_none(), "and lost the paper");
        assert_eq!((sp.possession.plan_minted, sp.possession.plan_mismatch), (1, 0));
        // The Plan moves: slot 2 → slot 20 → hotbar slot 7.
        let p = &mut hg.state.players[0];
        p.crafting_ui.open_player_crafting(&p.inventory, &p.armour_slots);
        let eye = p.player.eye_pos();
        for at in [2, 20, 20, 7] {
            let click = crate::window::WindowClick::Slot { slot: at, right: false };
            let r = p.crafting_ui.apply_click(&mut p.inventory, &mut p.armour_slots, &click, false, eye, |_| crate::block::AIR);
            assert!(r.ok(), "click {at}");
        }
        assert!(p.crafting_ui.close(&mut p.inventory, &mut p.armour_slots));
        // Step until the ops land (the loopback transport is real-time: a
        // loaded machine can take more than three ticks to deliver them).
        for _ in 0..40 {
            harness_step(&mut server, &mut hg);
            if server.server.players[slot].possession.window_ops >= 6 {
                break;
            }
        }
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        assert!(sp.possession.window_ops >= 6, "the open, four clicks and the close: {}", sp.possession.window_ops);
        assert_eq!(sp.possession.window_mismatch, 0, "the placeholder moved as the Plan did");
        assert_eq!(plan_in(&sp.inventory, 7).and_then(|p| p.marker), Some(crate::plan::marker(&plan)));
        assert!(plan_in(&hg.state.players[0].inventory, 7).is_some_and(|p| p == plan));
    }

    /// C3c-3a — a joiner confirms a capture (the dialog's Confirm,
    /// `confirm_capture`): with a full bag it is refused BEFORE anything
    /// happens (the toast; the tile keeps its paper; no Plan on either side);
    /// with room the client's Plan lands in its first empty slot and the
    /// server's copy gains its marker placeholder there, spending nothing.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_capture_commit_mints_a_marker_and_a_full_bag_refuses_it() {
        use crate::item::ItemStack;
        use crate::world::FaceAttachment;
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("capture-commit");
        let feet = clear_pad(&mut hg, Some(&mut server));
        // One blank-paper tile with a block built on it, in the client's world.
        let tile = (feet[0] + 2, feet[1] - 1, feet[2]);
        let top = crate::mesh::Face::Top.index();
        hg.state.world.set_face_attachment(tile, top, FaceAttachment::BlueprintBlank);
        hg.state.world.set_block(tile.0, tile.1 + 1, tile.2, crate::block::OAK_PLANKS);
        let candidate = crate::plan::capture(&hg.state.world, tile, "survival").expect("a capture");
        let pending = || crate::plan::PendingCapture { candidate: candidate.clone(), parent_match: None, mark_as_derivative: false };
        let stone = ItemStack::new_block(crate::block::STONE, 64);
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            for k in 0..36 {
                inv.set_slot(k, Some(stone.clone()));
            }
        }
        // 1. A full bag: refused before anything happens.
        hg.state.players[0].pending_capture = Some(pending());
        hg.state.confirm_capture(0);
        assert!(hg.state.toast.as_ref().is_some_and(|(t, _)| t == crate::plan_mint::MAKE_ROOM_TOAST));
        assert!(hg.state.players[0].pending_capture.is_none(), "the dialog closed");
        assert!(hg.state.players[0].inventory.slots_iter().flatten().all(|s| s.item == stone.item), "no Plan");
        assert_eq!(hg.state.world.face_attachment_at(tile, top), Some(&FaceAttachment::BlueprintBlank), "the tile keeps its paper");
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        assert_eq!(server.server.players[slot].possession.plan_minted, 0, "nothing reported");
        // 2. Room (slot 5): the Plan lands on both sides.
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            inv.set_slot(5, None);
        }
        hg.state.players[0].pending_capture = Some(pending());
        hg.state.confirm_capture(0);
        let plan = plan_in(&hg.state.players[0].inventory, 5).expect("the client's Plan");
        assert!(hg.state.world.face_attachment_at(tile, top).is_none(), "the tile's paper was consumed");
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        assert_eq!(plan_in(&sp.inventory, 5).and_then(|p| p.marker), Some(crate::plan::marker(&plan)));
        assert_eq!(sp.inventory.slots_iter().flatten().filter(|s| s.item == stone.item).count(), 35, "nothing spent");
        assert_eq!((sp.possession.plan_minted, sp.possession.plan_mismatch), (1, 0));
    }

    /// C3c-3a — a joiner hangs a developed Plan through the REAL arm: the
    /// print reaches the server as a use edit tagged `HangPrint` (never a
    /// placement), and the server's copy loses THAT Plan's placeholder by
    /// marker — two Plans in the copy, the other one in the hand's slot (a
    /// drifted copy): the right one goes.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_hung_print_takes_its_plan_by_marker() {
        use crate::item::{Item, ItemStack};
        use crate::plan::PlanData;
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("hang-print");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let hung = PlanData { name: "Hung".to_string(), ..PlanData::debug_3x3_stone() };
        let kept = PlanData { name: "Kept".to_string(), ..PlanData::debug_3x3_stone() };
        let placeholder = |p: &PlanData| Item::Plan(PlanData::marker_placeholder(crate::plan::marker(p), true));
        let one = |item: Item| Some(ItemStack { item, count: 1 });
        hg.state.players[0].inventory.set_slot(0, one(Item::Plan(hung.clone())));
        hg.state.players[0].inventory.set_slot(1, one(Item::Plan(kept.clone())));
        server.server.players[slot].inventory.set_slot(0, one(placeholder(&kept)));
        server.server.players[slot].inventory.set_slot(1, one(placeholder(&hung)));
        hg.state.players[0].hotbar_slot = 0;
        let wall = wall_ahead(&mut hg, &mut server, feet);
        harness_step(&mut server, &mut hg);
        aim_at_wall(&mut hg, wall);
        right_click(&mut hg);
        let print = [wall[0], wall[1], wall[2] + 1];
        assert_eq!(hg.state.world.get_block(print[0], print[1], print[2]), crate::block::CYANOTYPE_PRINT, "hung");
        assert!(hg.state.players[0].inventory.slot(0).is_none(), "the Plan is spent");
        for _ in 0..4 {
            harness_step(&mut server, &mut hg);
        }
        assert_eq!(server.server.world.get_block(print[0], print[1], print[2]), crate::block::CYANOTYPE_PRINT, "the edit is applied");
        let sp = &server.server.players[slot];
        assert_eq!((sp.possession.use_mirrored, sp.possession.use_mismatch), (1, 0), "one use mirrored");
        assert_eq!((sp.possession.matched, sp.possession.mismatched), (0, 0), "never a placement");
        assert!(sp.inventory.slot(1).is_none(), "the hung Plan's placeholder went, by marker");
        assert_eq!(plan_in(&sp.inventory, 0).and_then(|p| p.marker), Some(crate::plan::marker(&kept)), "the other stayed");
    }

    /// C3c-3a — a joiner right-clicks an empty item frame with its ONLY
    /// Blueprint Paper (the frame use's request claims it), then, inside the
    /// round trip, a wall with it (the REAL art-capture arm): nothing happens
    /// — no Plan, the paper still in hand, nothing reported. The server ends
    /// with the paper in the frame and no Plan.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_art_capture_waits_for_the_claim_on_its_paper() {
        use crate::item::{Item, ItemStack};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("art-claimed");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let paper = ItemStack::new_block(crate::block::BLUEPRINT_PAPER, 1);
        hg.state.players[0].inventory.set_slot(0, Some(paper.clone()));
        server.server.players[slot].inventory.set_slot(0, Some(paper));
        hg.state.players[0].hotbar_slot = 0;
        let frame = [feet[0] + 2, feet[1] + 1, feet[2]];
        hg.state.world.set_block(frame[0], frame[1], frame[2], crate::block::ITEM_FRAME);
        server.server.world.set_block(frame[0], frame[1], frame[2], crate::block::ITEM_FRAME);
        let wall = wall_ahead(&mut hg, &mut server, feet);
        harness_step(&mut server, &mut hg);
        // 1. The frame, through the real arm: the request claims the paper.
        aim_at(&mut hg, glam::Vec3::new(frame[0] as f32 + 0.5, frame[1] as f32 + 0.5, frame[2] as f32 + 0.5));
        right_click(&mut hg);
        // 2. Inside the round trip: the wall, through the real art-capture arm.
        aim_at_wall(&mut hg, wall);
        right_click(&mut hg);
        let has_plan = |inv: &crate::inventory::Inventory| inv.slots_iter().flatten().any(|s| matches!(s.item, Item::Plan(_)));
        assert!(!has_plan(&hg.state.players[0].inventory), "no Plan: the paper is claimed");
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(1), "still in hand until the frame's outcome");
        settle(&mut server, &mut hg);
        let pos = (frame[0], frame[1], frame[2]);
        let framed = server.server.world.item_frame_at(pos).and_then(|f| f.item.as_ref().map(|s| s.item.clone()));
        assert_eq!(framed, Some(Item::Block(crate::block::BLUEPRINT_PAPER)), "the frame has it");
        let sp = &server.server.players[slot];
        assert_eq!(sp.possession.plan_minted, 0, "nothing reported");
        assert!(!has_plan(&sp.inventory) && !has_plan(&hg.state.players[0].inventory));
        assert!(hg.state.players[0].inventory.slot(0).is_none() && sp.inventory.slot(0).is_none(), "the paper is in the frame");
    }
}
