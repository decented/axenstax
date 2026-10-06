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
        let mut frames = 0u32;
        while !matches!(state.mode, GameMode::Playing) {
            state.update_and_render();
            frames += 1;
            assert!(
                frames < 20_000,
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
}
