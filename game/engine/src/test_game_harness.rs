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
}
