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
            self.state.finish_reputation_day_tick();
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
                let report = self.state.tick_hosted_server(before);
                self.state.finish_reputation_day_tick();
                report
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_arch = "wasm32"))]
    #[path = "quest_baseline.rs"]
    mod quest_baseline;
    #[path = "quest_replay.rs"]
    mod quest_replay;
    #[cfg(not(target_arch = "wasm32"))]
    #[path = "quest_native.rs"]
    mod quest_native;
    #[cfg(not(target_arch = "wasm32"))]
    #[path = "quest_preparation.rs"]
    mod quest_preparation;
    #[cfg(not(target_arch = "wasm32"))]
    #[path = "quest_preparation_tests.rs"]
    mod quest_preparation_tests;
    #[cfg(not(target_arch = "wasm32"))]
    #[path = "quest_preparation_lifecycle.rs"]
    mod quest_preparation_lifecycle;
    #[path = "quest_day.rs"]
    mod quest_day;
    #[path = "quest_day_authority.rs"]
    mod quest_day_authority;
    #[path = "quest_carry.rs"]
    mod quest_carry;
    #[cfg(not(target_arch = "wasm32"))]
    #[path = "pick_block.rs"]
    mod pick_block;
    #[cfg(not(target_arch = "wasm32"))]
    #[path = "keep_join.rs"]
    mod keep_join;
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

    /// Perf A review S1: a section vacated by a chunk push, and a column
    /// unloaded, must lose their transparent (leaves, glass) mesh too, not
    /// just the opaque, water, plant, decal and micro ones.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_dropping_a_section_or_column_clears_its_transparent_mesh() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-drop-transparent");
        hg.frames(3);
        let st = &mut hg.state;
        let pos = (0, crate::world::MAX_CHUNK_Y, 0);
        let stamp = |st: &mut crate::GameState, block| {
            st.world.set_block(5, pos.1 * 16 + 5, 5, block);
            let meshes = crate::mesh::build_chunk_meshes(pos.0, pos.1, pos.2, &st.world, &st.registry);
            st.renderer.upload_chunk(pos, &meshes);
        };
        // Section vacated.
        stamp(st, crate::block::GLASS);
        assert!(st.renderer.transparent_meshes.contains_key(&pos), "glass must mesh as transparent");
        st.drop_section_meshes(pos);
        assert!(!st.renderer.transparent_meshes.contains_key(&pos), "section drop leaked the transparent mesh");
        // Column unloaded.
        stamp(st, crate::block::OAK_LEAVES);
        assert!(st.renderer.transparent_meshes.contains_key(&pos), "leaves must mesh as transparent");
        st.drop_column_meshes((pos.0, pos.2));
        assert!(!st.renderer.transparent_meshes.contains_key(&pos), "column drop leaked the transparent mesh");
    }

    /// Perf review X2: a joined client (push-only or not) must never let the
    /// governor shorten its streamed distance; its server pushes at the
    /// distance it announces and would re-send the ring in between.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joined_client_never_has_its_distance_shortened_by_the_governor() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-governor-joined");
        hg.state.graphics.auto_quality = true;
        assert!(hg.state.governor_may_shrink_distance(), "its own world: it may");
        let mut server = crate::hosted_server::HostedServer::start(
            0,
            format!("harness-governor-joined-server-{}", std::process::id()),
            42,
            0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        )
        .expect("dedicated server starts");
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport),
            crate::remote_client::build_join_request_guest("Governed", 0),
            None,
        ));
        assert!(!hg.state.joined_push_only, "a normal joiner, not a worldgen-mismatch one");
        assert!(!hg.state.governor_may_shrink_distance());
        let rd = hg.state.graphics.render_distance;
        let max = crate::quality_governor::max_level(&hg.state.graphics, false, crate::quality_governor::NO_SCALE_CAP);
        for _ in 0..4000 {
            let dt = 0.200 * 0.8f32.powi(hg.state.governor.level() as i32);
            hg.state.governor.update(dt, 1.0 / 60.0, max);
        }
        assert!(hg.state.governor.level() > 0, "the machine is slow: it stepped down");
        assert_eq!(hg.state.effective_render_distance(), rd, "but never the distance");
    }

    /// Perf review S4: leaving a world must drop EVERY per-section mesh map,
    /// transparent (leaves, glass) included, or the last world's canopies and
    /// glass hang in the next world's sky (an all-air section never uploads or
    /// drops anything, so nothing else clears them).
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_world_change_drops_every_per_section_mesh_map() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-world-change-meshes");
        hg.frames(3);
        let all_empty = |r: &crate::renderer::Renderer| {
            r.chunk_meshes.is_empty()
                && r.water_meshes.is_empty()
                && r.transparent_meshes.is_empty()
                && r.plant_meshes.is_empty()
                && r.decal_meshes.is_empty()
                && r.micro_meshes.is_empty()
                && r.micro_billboard_meshes.is_empty()
        };
        // Stamp one section with stone, glass, leaves, water and a plant, high
        // in the world where no terrain meshes, and upload it.
        let pos = (0, crate::world::MAX_CHUNK_Y, 0);
        let st = &mut hg.state;
        for (i, b) in [
            crate::block::STONE,
            crate::block::GLASS,
            crate::block::OAK_LEAVES,
            crate::block::WATER,
            crate::block::TALL_GRASS,
        ]
        .into_iter()
        .enumerate()
        {
            st.world.set_block(2 + 2 * i as i32, pos.1 * 16 + 5, 5, b);
        }
        let meshes = crate::mesh::build_chunk_meshes(pos.0, pos.1, pos.2, &st.world, &st.registry);
        st.renderer.upload_chunk(pos, &meshes);
        assert!(st.renderer.chunk_meshes.contains_key(&pos), "opaque meshed");
        assert!(st.renderer.transparent_meshes.contains_key(&pos), "glass and leaves meshed");
        assert!(!all_empty(&st.renderer));
        // The real world-change reset.
        st.reset_for_world_change();
        assert!(st.renderer.transparent_meshes.is_empty(), "the last world's leaves and glass stayed");
        assert!(all_empty(&st.renderer), "a per-section mesh map survived the world change");
        // And the Workshop reset: re-stamp, make it a Workshop, reset the room.
        st.world.set_block(2, pos.1 * 16 + 5, 5, crate::block::GLASS);
        let meshes = crate::mesh::build_chunk_meshes(pos.0, pos.1, pos.2, &st.world, &st.registry);
        st.renderer.upload_chunk(pos, &meshes);
        assert!(st.renderer.transparent_meshes.contains_key(&pos));
        st.world.is_workshop = true;
        st.reset_workshop_space();
        assert!(all_empty(&st.renderer), "the Workshop reset left a mesh map behind");
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
                pack: false,
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
        joined_window_client_mode(tag, crate::action_identity::InventoryMode::Mirror)
    }

    fn joined_window_client_mode(tag: &str, mode: crate::action_identity::InventoryMode) -> (HeadlessGame, crate::hosted_server::HostedServer, usize) {
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
        server.select_inventory_mode(mode).expect("the fixture selects its session mode before joining");
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

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_sidecar_dead_rejoin_enters_death_screen_and_respawns() {
        use crate::transport::ClientTransport as _;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-sidecar-dead");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-sidecar-dead-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).unwrap();
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let keys = nostr::Keys::new(nostr::SecretKey::from_slice(&[66; 32]).unwrap());
        let mut saved = crate::server::ServerPlayer::new(server.server.world_spawn());
        saved.server_simulated = true;
        saved.verified_pubkey = Some(keys.public_key().to_bytes());
        saved.enter_world();
        saved.combat.health = 0.0;
        saved.combat.dead = true;
        saved.death_handled = true;
        saved.cursor = Some(crate::item::ItemStack::new_block(crate::block::DIRT, 2));
        server.server.players.push(saved);
        server.server.save_joiner(0).unwrap();
        server.server.players.pop();
        let cell = (1, 80, 1);
        server.server.world.set_block(cell.0, cell.1, cell.2, crate::block::GRAVE);
        server.server.world.insert_grave(cell, crate::grave::GraveData::from_snapshot(
            vec![Some(crate::item::ItemStack::new_block(crate::block::STONE, 3))], 0));
        let binding = Some([17; 32]);
        let transport = server.attach_test_remote_with_binding(binding);
        let pkt = transport.try_recv_from_server().expect("issued challenge");
        let (kind, bytes) = crate::protocol::deserialize_header(&pkt).unwrap();
        assert_eq!(kind, crate::protocol::PacketType::Challenge);
        let challenge: crate::protocol::ChallengePacket = crate::protocol::safe_deserialize(bytes).unwrap();
        let mut request = crate::remote_client::build_join_request_guest("Saved", 0);
        request.auth_event = Some(pollster::block_on(crate::signet::native_signer::build_join_auth_event(
            &keys, &challenge.nonce_hex, &crate::signet::join_origin(binding))).unwrap());
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(Box::new(transport), request, None));
        for _ in 0..8 { harness_step(&mut server, &mut hg); }
        assert!(server.server.players[0].combat.dead);
        assert_eq!(hg.state.players[0].crafting_ui.cursor_item.as_ref().unwrap().count, 2);
        assert!(hg.state.players[0].combat.dead, "fresh client must enter its actual death-screen state");
        assert_eq!(server.server.world.iter_graves().count(), 1);
        for _ in 0..crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN { harness_step(&mut server, &mut hg); }
        assert!(hg.state.players[0].combat.request_respawn());
        for _ in 0..40 {
            harness_step(&mut server, &mut hg);
            if !server.server.players[0].combat.dead && !hg.state.players[0].combat.dead { break; }
        }
        assert!(!server.server.players[0].combat.dead);
        assert!(!hg.state.players[0].combat.dead);
        assert_eq!(hg.state.players[0].combat.health, 20.0);
        assert_eq!(hg.state.players[0].crafting_ui.cursor_item.as_ref().unwrap().count, 2);
        assert_eq!(server.server.world.iter_graves().count(), 1);
        assert_eq!(server.server.world.grave_at(cell).unwrap().slots[0].as_ref().unwrap().count, 3);
    }

    /// C3d run 8 (run-6 review H1, LOW-1) — a Strict joiner dies with stone
    /// in its 2×2 grid, screen open. Through the real `GameState`: Esc on the
    /// frozen screen pauses (never the generic panel close), E reaches the
    /// same one gate, and nothing reaches the server (no correction is
    /// queued); a Respawn the server hasn't answered yet still lets Esc
    /// pause. Once the body lives, E closes and the grid ends in the bag on
    /// both copies, never torn down.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_strict_frozen_crafting_esc_pauses_and_e_returns_the_grid() {
        use crate::window_actions::CraftClose;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-frozen-esc");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-frozen-esc-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).expect("dedicated server starts");
        server.select_inventory_mode(crate::action_identity::InventoryMode::Strict).expect("Strict");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport), crate::remote_client::build_join_request_guest("Frozen", 0), None));
        for _ in 0..5 { harness_step(&mut server, &mut hg); }
        let slot = server.server.players.len() - 1;
        assert_eq!(hg.state.remote_client.as_ref().unwrap().inventory_mode, crate::action_identity::InventoryMode::Strict);
        let applied = hg.state.remote_client.as_ref().unwrap().applied_window_boundary();
        crate::window_events::apply_through(&mut server.server.players[slot], applied, 0);
        let stone = crate::item::ItemStack::new_block(crate::block::STONE, 8);
        let refill = hg.state.players[0].inventory.auto_refill;
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            *inv = crate::inventory::Inventory::new();
            inv.auto_refill = refill;
            inv.set_slot(0, Some(stone.clone()));
        }
        hg.state.open_crafting(0, None);
        hg.state.apply_window_click(0, &crate::window::WindowClick::Slot { slot: 0, right: false });
        hg.state.apply_window_click(0, &crate::window::WindowClick::Grid { row: 0, col: 0, right: false });
        for _ in 0..3 { harness_step(&mut server, &mut hg); }
        assert_eq!(server.server.players[slot].craft_grid[0][0].as_ref().map(|s| s.count), Some(8), "the server's copy has the grid");
        let sp = &mut server.server.players[slot];
        sp.combat.health = 0.0;
        sp.combat.dead = true;
        for _ in 0..8 {
            harness_step(&mut server, &mut hg);
            if hg.state.players[0].is_dead() { break; }
        }
        assert!(hg.state.players[0].is_dead(), "the client heard of its death");
        for _ in 0..3 { harness_step(&mut server, &mut hg); }
        let events = server.server.players[slot].window_events.last_number();
        assert!(hg.state.players[0].crafting_ui.open, "the screen is open at death");
        hg.state.esc_crafting();
        assert!(matches!(hg.state.mode, crate::GameMode::Paused { .. }), "Esc on a frozen crafting screen pauses");
        assert!(hg.state.players[0].crafting_ui.open && hg.state.players[0].crafting_ui.grid_holds_items(), "the screen and its grid stay");
        assert_eq!(hg.state.close_crafting(0), CraftClose::Frozen, "E reaches the same gate");
        hg.state.mode = crate::GameMode::Playing;
        for _ in 0..3 { harness_step(&mut server, &mut hg); }
        assert_eq!(server.server.players[slot].window_events.last_number(), events, "no Close reached the server: no correction queued");
        // The Respawn, unanswered: the client asks, the server hasn't run.
        for _ in 0..crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN { harness_step(&mut server, &mut hg); }
        assert!(hg.state.players[0].combat.request_respawn());
        for _ in 0..3 {
            hg.frames(1);
            hg.ticks(1);
            if hg.state.remote_client.as_ref().unwrap().respawn_unanswered() { break; }
        }
        assert!(hg.state.remote_client.as_ref().unwrap().respawn_unanswered(), "a Respawn waits for its answer");
        hg.state.esc_crafting();
        assert!(matches!(hg.state.mode, crate::GameMode::Paused { .. }), "an unanswered Respawn never traps the player: Esc pauses");
        hg.state.mode = crate::GameMode::Playing;
        for _ in 0..40 {
            harness_step(&mut server, &mut hg);
            if !server.server.players[slot].combat.dead && !hg.state.players[0].is_dead()
                && !hg.state.remote_client.as_ref().unwrap().respawn_unanswered() { break; }
        }
        assert!(!hg.state.players[0].is_dead() && !hg.state.remote_client.as_ref().unwrap().respawn_unanswered(), "respawned");
        assert_eq!(hg.state.close_crafting(0), CraftClose::Closed, "E closes once the body lives");
        for _ in 0..3 { harness_step(&mut server, &mut hg); }
        assert!(matches!(hg.state.remote_client.as_ref().unwrap().state, crate::remote_client::ConnectionState::Connected { .. }), "never torn down");
        let applied = hg.state.remote_client.as_ref().unwrap().applied_window_boundary();
        let sp = &mut server.server.players[slot];
        crate::window_events::apply_through(sp, applied, 0);
        assert!(sp.craft_grid.iter().flatten().all(Option::is_none), "the server copy's grid went back");
        assert_eq!(sp.inventory.slot(0).map(|s| s.count), Some(8), "into the server copy's bag");
        assert!(!hg.state.players[0].crafting_ui.grid_holds_items(), "the client's grid went back");
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(8), "into the client's bag");
    }

    /// C3d run 9 — a Strict joiner's survival placement and mined break
    /// through the REAL game-loop arms (a right-click on the floor, the held
    /// left button on the placed block): each is ONE reserved typed `Edit`
    /// the arm reserves itself, and nothing rides the movement packet (a
    /// Strict server closes the session on any edit there). The server
    /// accepts both; the windows, worlds and the yield agree; the tool wore
    /// once on both copies.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_strict_place_and_break_go_typed() {
        use crate::item::{Item, ItemStack};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-strict-edit");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-strict-edit-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).expect("dedicated server starts");
        server.select_inventory_mode(crate::action_identity::InventoryMode::Strict).expect("Strict");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport), crate::remote_client::build_join_request_guest("Typed", 0), None));
        for _ in 0..5 { harness_step(&mut server, &mut hg); }
        let slot = server.server.players.len() - 1;
        assert_eq!(hg.state.remote_client.as_ref().unwrap().inventory_mode, crate::action_identity::InventoryMode::Strict);
        let applied = hg.state.remote_client.as_ref().unwrap().applied_window_boundary();
        crate::window_events::apply_through(&mut server.server.players[slot], applied, 0);
        let feet = clear_pad(&mut hg, Some(&mut server));
        let pickaxe = crate::crafting::Tool::new(crate::crafting::ToolType::Pickaxe, crate::crafting::ToolMaterial::Stone);
        let refill = hg.state.players[0].inventory.auto_refill;
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            *inv = crate::inventory::Inventory::new();
            inv.auto_refill = refill;
            inv.set_slot(0, Some(ItemStack::new_block(crate::block::COBBLESTONE, 4)));
            inv.set_slot(1, Some(ItemStack { item: Item::Tool(pickaxe), count: 1 }));
        }
        hg.state.players[0].hotbar_slot = 0;
        for _ in 0..2 { harness_step(&mut server, &mut hg); }
        let through = |server: &crate::hosted_server::HostedServer| server.server.players[slot].action_frontier.as_ref().map_or(0, |f| f.through());
        let before = through(&server);
        // The place arm: aim at the top of the floor two cells ahead.
        let cell = [feet[0], feet[1], feet[2] - 2];
        aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 - 0.02, cell[2] as f32 + 0.5));
        right_click(&mut hg);
        assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), crate::block::COBBLESTONE, "the client placed it");
        let reserved_edit = |hg: &HeadlessGame| hg.state.remote_client.as_ref().unwrap().action_journal.as_ref().unwrap().actions().any(|a| {
            matches!(crate::action_wire::ActionPacket::decode(&a.intent).map(|p| p.intent), Ok(crate::action_wire::ActionIntent::Edit { change, .. }) if [change.x, change.y, change.z] == cell)
        });
        assert!(reserved_edit(&hg), "the place arm reserved a typed Edit of the cell");
        assert!(hg.state.pending_block_changes.is_empty() && hg.state.pending_mined.is_empty(), "nothing waits for the movement packet");
        for _ in 0..4 { harness_step(&mut server, &mut hg); }
        assert_eq!(server.server.world.get_block(cell[0], cell[1], cell[2]), crate::block::COBBLESTONE, "the server accepted the placement");
        // The break arm: the pickaxe, the button held until the block breaks.
        hg.state.players[0].hotbar_slot = 1;
        harness_step(&mut server, &mut hg);
        // Run 11 (run-9b review LOW-3) — checked in the frame the block
        // broke, before any later frame's tick could send (and so empty) the
        // movement packet's queues: a Legacy break would still be waiting.
        mine_joined_to_break(&mut hg, &mut server, slot, cell);
        assert!(hg.state.pending_block_changes.is_empty() && hg.state.pending_mined.is_empty(), "the break: nothing waits for the movement packet");
        for _ in 0..10 {
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
            server.tick();
        }
        for _ in 0..4 { harness_step(&mut server, &mut hg); }
        let rc = hg.state.remote_client.as_ref().unwrap();
        let reason = match &rc.state { crate::remote_client::ConnectionState::Failed(reason) => reason.clone(), _ => String::new() };
        assert!(matches!(rc.state, crate::remote_client::ConnectionState::Connected { .. }), "never torn down: nothing rode the movement packet ({reason})");
        assert_eq!(rc.action_journal.as_ref().map(|j| j.len()), Some(0), "every reservation retired");
        assert!(through(&server) >= before + 2, "two typed edits accepted (through {} from {before})", through(&server));
        assert_eq!(server.server.world.get_block(cell[0], cell[1], cell[2]), crate::block::AIR, "the server broke it");
        assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), crate::block::AIR);
        let applied = rc.applied_window_boundary();
        let sp = &mut server.server.players[slot];
        crate::window_events::apply_through(sp, applied, 0);
        let p = &hg.state.players[0];
        let client = crate::window::digest_parts(&p.inventory, &p.armour_slots, &p.crafting_ui.cursor_item, &p.crafting_ui.grid, p.crafting_ui.station());
        let server_digest = crate::window::digest_parts(&sp.inventory, &sp.armour, &sp.cursor, &sp.craft_grid, sp.station);
        assert_eq!(client, server_digest, "the windows agree");
        assert_eq!(p.inventory.slot(0).map(|s| s.count), Some(4), "the yield came back: four cobblestone");
        let worn = |inv: &crate::inventory::Inventory| match inv.slot(1).map(|s| &s.item) { Some(Item::Tool(t)) => t.durability, _ => 0 };
        assert_eq!((worn(&p.inventory), worn(&sp.inventory)), (pickaxe.durability - 1, pickaxe.durability - 1), "the pickaxe wore once on both copies");
    }

    /// OC (v88) — the full client path: a saved joiner owing Stone 1 joins
    /// by its real signed join; the client's own `apply_window_inbox` (the
    /// `Sync` arm) installs the debt the `InventorySync` carries, the server's
    /// copy holds the same; the next identical correction give, sent as a
    /// served op's is, then pays it on both copies — no free Stone. And an
    /// owed list the client can't decode whole ends the session (A6),
    /// applying nothing.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_oc_saved_debt_reaches_the_client_and_pays_the_next_give() {
        use crate::transport::ClientTransport as _;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-oc-debt");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-oc-debt-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).unwrap();
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let keys = nostr::Keys::new(nostr::SecretKey::from_slice(&[67; 32]).unwrap());
        let stone = crate::item::Item::Block(crate::block::STONE);
        let one_stone = crate::item::ItemStack::new_block(crate::block::STONE, 1);
        let mut saved = crate::server::ServerPlayer::new(server.server.world_spawn());
        saved.server_simulated = true;
        saved.verified_pubkey = Some(keys.public_key().to_bytes());
        saved.enter_world();
        saved.inventory.set_slot(5, Some(crate::item::ItemStack::new_block(crate::block::DIRT, 4)));
        let take = crate::container_window::ItemDelta { take: vec![(0, one_stone.clone())], give: vec![] };
        saved.window_events.debt.apply(&mut saved.inventory, &mut saved.craft_grid, &mut saved.cursor, &mut saved.armour, &take);
        assert_eq!(saved.window_events.debt.owed(&stone), 1);
        server.server.players.push(saved);
        server.server.save_joiner(0).unwrap();
        server.server.players.pop();
        let binding = Some([18; 32]);
        let transport = server.attach_test_remote_with_binding(binding);
        let pkt = transport.try_recv_from_server().expect("issued challenge");
        let (kind, bytes) = crate::protocol::deserialize_header(&pkt).unwrap();
        assert_eq!(kind, crate::protocol::PacketType::Challenge);
        let challenge: crate::protocol::ChallengePacket = crate::protocol::safe_deserialize(bytes).unwrap();
        let mut request = crate::remote_client::build_join_request_guest("Owing", 0);
        request.auth_event = Some(pollster::block_on(crate::signet::native_signer::build_join_auth_event(
            &keys, &challenge.nonce_hex, &crate::signet::join_origin(binding))).unwrap());
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(Box::new(transport), request, None));
        for _ in 0..8 { harness_step(&mut server, &mut hg); }
        let has_stone = |inv: &crate::inventory::Inventory| inv.slots_iter().flatten().any(|s| s.item == stone);
        assert_eq!(hg.state.players[0].inventory.slot(5).map(|s| s.count), Some(4), "the saved window arrived");
        assert_eq!(hg.state.window_inbox.debt.owed(&stone), 1, "the client's Sync apply installed the carried debt");
        assert_eq!(server.server.players[0].window_events.debt.owed(&stone), 1, "the server copy holds it too");
        let give = crate::container_window::ItemDelta { take: vec![], give: vec![one_stone] };
        let n = server.send_test_correction(0, crate::window_ops::Correction {
            client: give.clone(), own: Some(give),
            view: crate::window_ops::ViewEvent::Sets { sets: Vec::new(), furnace: None },
        });
        for _ in 0..8 { harness_step(&mut server, &mut hg); }
        assert_eq!(server.server.players[0].window_events.waiting(), 0, "event {n} was applied on both copies");
        assert!(!has_stone(&server.server.players[0].inventory), "the give paid the carried debt on the server copy");
        assert!(!has_stone(&hg.state.players[0].inventory), "and on the client copy");
        assert!(hg.state.players[0].crafting_ui.cursor_item.is_none());
        assert!(server.server.players[0].window_events.debt.is_empty() && hg.state.window_inbox.debt.is_empty());
        // A6 — an undecodable owed entry: a protocol error, nothing applied.
        let bad = crate::protocol::InventorySyncPacket {
            window_event: n + 1,
            slots: vec![(crate::protocol::WireWindowSlot::Inv(5), None)],
            owed: vec![(crate::protocol::WireStack {
                item_kind: crate::protocol::item_kind::BLOCK, item_id: u16::MAX, count: 1,
                full_item: crate::protocol::WireItem::None,
            }, 1)],
            station: None,
            locks: 0,
        };
        hg.state.window_inbox.add(vec![crate::remote_client::RequestOutcome::Sync(bad)], Vec::new(), Vec::new());
        hg.state.apply_window_inbox();
        assert_eq!(hg.state.players[0].inventory.slot(5).map(|s| s.count), Some(4), "nothing of the refused Sync applied");
        assert!(matches!(
            &hg.state.remote_client.as_ref().unwrap().state,
            crate::remote_client::ConnectionState::Failed(reason) if reason == crate::remote_client::HOST_BAD_WORLD_DATA
        ), "the client leaves with a reason");
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
        hg.state.tick_accumulator = web_time::Duration::ZERO;
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

    /// D2c-1: drive only logical ticks with held keys, so steering cannot
    /// accidentally depend on the frame loop. Death must release the seat.
    fn tick_ridden_mob(kind: crate::mob::MobType, tag: &str) {
        use crate::entity::{Position, Ridden};
        use winit::keyboard::KeyCode;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world(tag);
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        clear_pad(&mut hg, None);
        let at = hg.state.players[0].player.pos;
        let mount = crate::entity::spawn_mob(&mut hg.state.ecs, kind, at);
        crate::riding::mount(&mut hg.state.ecs, mount, &mut hg.state.players[0].ride);
        hg.state.players[0].riding = Some(mount);
        hg.state.players[0].camera.yaw = 0.0;
        hg.state.input.cursor_captured = true;
        hg.state.input.key_pressed(KeyCode::KeyW);
        hg.ticks(4);
        let end = hg.state.ecs.get::<&Position>(mount).unwrap().0;
        assert!(at.z - end.z > 0.8, "held forward steers from the tick: {at} -> {end}");
        let body = hg.state.players[0].player.pos;
        assert!((body.x - end.x).abs() < 1e-4 && (body.z - end.z).abs() < 1e-4);
        assert!(hg.state.ecs.get::<&Ridden>(mount).is_ok());
        hg.state.players[0].combat.take_damage_from(100.0, crate::survival::DamageCause::Generic);
        hg.ticks(1);
        assert!(hg.state.players[0].riding.is_none(), "death releases the seat before respawn");
        assert!(hg.state.ecs.get::<&Ridden>(mount).is_err());
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_riding_horse_steering_runs_from_ticks_and_death_releases_it() {
        tick_ridden_mob(crate::mob::MobType::Horse, "harness-riding-horse");
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_riding_nostrich_steering_runs_from_ticks_and_death_releases_it() {
        tick_ridden_mob(crate::mob::MobType::Nostrich, "harness-riding-nostrich");
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_riding_cart_follows_from_ticks_and_jump_releases_it() {
        use crate::entity::{Position, Ridden};
        use winit::keyboard::KeyCode;
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-riding-cart");
        let feet = clear_pad(&mut hg, None);
        let cell = (feet[0], feet[1], feet[2]);
        for z in feet[2] - 4..=feet[2] {
            hg.state.world.set_block(feet[0], feet[1], z, crate::rail::TRACK);
        }
        let cart = crate::cart::spawn_cart(&mut hg.state.ecs, cell);
        assert_eq!(crate::cart::dispatch_cart(&mut hg.state.ecs, &mut hg.state.world, cell, -glam::Vec3::Z), Some(cart));
        crate::riding::mount(&mut hg.state.ecs, cart, &mut hg.state.players[0].ride);
        hg.state.players[0].riding = Some(cart);
        let before = hg.state.ecs.get::<&Position>(cart).unwrap().0;
        hg.ticks(4);
        let after = hg.state.ecs.get::<&Position>(cart).unwrap().0;
        assert!(before.distance(after) > 0.1, "cart rolls from the tick");
        assert!(hg.state.players[0].riding.is_some());
        let body = hg.state.players[0].player.pos;
        assert!((body.x - after.x).abs() < 1e-4 && (body.z - after.z).abs() < 1e-4);
        hg.state.input.cursor_captured = true;
        hg.state.input.key_pressed(KeyCode::Space);
        hg.ticks(1);
        assert!(hg.state.players[0].riding.is_none());
        assert!(hg.state.ecs.get::<&Ridden>(cart).is_err());
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_riding_joiner_clicks_a_server_horse_follows_its_mirror_and_sneaks_off() {
        use crate::entity::{Position, ProtocolId};
        use winit::keyboard::KeyCode;
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("riding-joiner");
        clear_pad(&mut hg, Some(&mut server));
        let at = server.server.players[slot].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
        let horse = crate::entity::spawn_mob(&mut server.server.ecs, crate::mob::MobType::Horse, at);
        for _ in 0..3 { harness_step(&mut server, &mut hg); }
        let id = server.server.ecs.get::<&ProtocolId>(horse).unwrap().0;
        let at = server.server.ecs.get::<&Position>(horse).unwrap().0;
        aim_at(&mut hg, at + glam::Vec3::Y);
        right_click(&mut hg);
        settle(&mut server, &mut hg);
        assert_eq!(server.server.players[slot].riding, Some(horse));
        assert_eq!(hg.state.players[0].mirror_mount, Some(id));
        hg.state.input.cursor_captured = true;
        hg.state.input.key_pressed(KeyCode::KeyW);
        // Real frames can auto-send ten inputs before our explicit send.
        // Wait for the server's packet budget to reach the held forward
        // input rather than assuming four fast rounds consumed its backlog.
        for _ in 0..40 {
            harness_step(&mut server, &mut hg);
            if server.server.players[slot].seat_input.forward == 1.0 { break; }
        }
        assert_eq!(server.server.players[slot].seat_input.forward, 1.0, "the server consumed held W");
        let start = server.server.ecs.get::<&Position>(horse).unwrap().0;
        for _ in 0..4 { harness_step(&mut server, &mut hg); }
        let end = server.server.ecs.get::<&Position>(horse).unwrap().0;
        assert!(end.distance(start) > 0.5, "four server steering ticks: {start:?} -> {end:?}");
        let mirrored = hg.state.remote_mobs.position_of(id).unwrap();
        let body = hg.state.players[0].player.pos;
        assert!((body.x - mirrored.x).abs() < 1e-4 && (body.z - mirrored.z).abs() < 1e-4);
        hg.state.input.key_released(KeyCode::KeyW);
        hg.state.input.key_pressed(KeyCode::ShiftLeft);
        for _ in 0..40 {
            harness_step(&mut server, &mut hg);
            if hg.state.players[0].mirror_mount.is_none() { break; }
        }
        assert!(server.server.players[slot].riding.is_none());
        assert!(hg.state.players[0].mirror_mount.is_none(), "the server's dismount reached the client");
        assert_eq!(hg.state.own_prediction.visual_offset(), glam::Vec3::ZERO, "first off-seat state snaps without smoothing");
        assert!(hg.state.players[0].player.pos.distance(server.server.players[slot].player.pos) < 1e-4, "first off-seat body agrees with the server");
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
        copy_holds(&mut server, &mut hg, slot, &crate::item::Item::Block(crate::block::DIAMOND_BLOCK), 0);
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

    // ─── C3c-2-fix ─────────────────────────────────────────────────────────

    /// C3c-2-fix (M1) — hold right-click through the REAL click arms for
    /// `ticks` game ticks at `fps` frames a second (`fps / 20` frames a tick,
    /// spread evenly), with no wall-clock tick inside a frame so the game
    /// tick is the test's; a joined client's `server` steps once a tick, as
    /// `harness_step`. Returns the game ticks elapsed.
    fn hold_right_click(hg: &mut HeadlessGame, mut server: Option<&mut crate::hosted_server::HostedServer>, fps: u32, ticks: u32) -> u64 {
        let start = hg.state.tick_counter;
        hg.state.players[0].place_cooldown = 0;
        hg.state.input.cursor_captured = true;
        hg.state.input.right_held = true;
        let mut owed = 0u32;
        for _ in 0..ticks {
            if let Some(s) = server.as_deref_mut() {
                s.tick();
            }
            owed += fps;
            while owed >= 20 {
                owed -= 20;
                hg.state.tick_accumulator = web_time::Duration::ZERO;
                hg.state.last_tick = web_time::Instant::now();
                hg.frames(1);
            }
            hg.ticks(1);
            if server.is_some() {
                hg.state.network_send_input();
            }
        }
        hg.state.input.right_held = false;
        hg.state.tick_counter - start
    }

    /// A bow in hotbar slot 0 and `arrows` arrows in slot 9 of player 0's
    /// (and, joined, the server's) window, aimed high over the pad.
    fn arm_bow(hg: &mut HeadlessGame, server: Option<(&mut crate::hosted_server::HostedServer, usize)>, arrows: u8) {
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::{ItemStack, MaterialId};
        let bow = ItemStack::new_tool(Tool::new(ToolType::Bow, ToolMaterial::Wood));
        let ammo = ItemStack::new_material(MaterialId::Arrow, arrows);
        let inv = &mut hg.state.players[0].inventory;
        inv.set_slot(0, Some(bow.clone()));
        inv.set_slot(9, Some(ammo.clone()));
        if let Some((server, slot)) = server {
            let inv = &mut server.server.players[slot].inventory;
            inv.set_slot(0, Some(bow));
            inv.set_slot(9, Some(ammo));
        }
        hg.state.players[0].hotbar_slot = 0;
        let eye = hg.state.players[0].player.eye_pos();
        aim_at(hg, eye + glam::Vec3::new(0.0, 30.0, -4.0));
    }

    /// C3c-2-fix (M1) — single-player's held bow fires on the TICK cadence
    /// (`shot::SHOT_COOLDOWN_TICKS`, 2.5 shots a second) whatever the frame
    /// rate: it used to fire every 8 FRAMES (7.5 a second at 60 fps, 18 at
    /// 144).
    fn single_player_bow_cadence(fps: u32) {
        use crate::item::{Item, MaterialId};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world(&format!("harness-bow-cadence-{fps}"));
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        hg.frames(5);
        clear_pad(&mut hg, None);
        hg.state.players[0].inventory = crate::inventory::Inventory::new();
        arm_bow(&mut hg, None, 64);
        let elapsed = hold_right_click(&mut hg, None, fps, 160);
        let shots = 64 - held_units(&hg.state.players[0].inventory, &Item::Material(MaterialId::Arrow));
        let want = elapsed.div_ceil(crate::shot::SHOT_COOLDOWN_TICKS);
        assert!(u64::from(shots).abs_diff(want) <= 1, "{fps} fps: {shots} shots in {elapsed} ticks (want {want})");
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_held_bow_fires_two_and_a_half_a_second_at_60_fps() {
        single_player_bow_cadence(60);
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_held_bow_fires_two_and_a_half_a_second_at_144_fps() {
        single_player_bow_cadence(144);
    }

    /// C3c-2-fix (M1) — a joiner holding right-click with its bow at 60 fps
    /// paces its `Shoot`s on the same tick cadence the server holds it to:
    /// every shot it sends is accepted (none `TooSoon`), 2.5 a second, each
    /// paid from both copies. It used to send one every 8 frames and have
    /// two in three silently refused.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_holding_its_bow_has_every_shot_accepted() {
        use crate::item::{Item, MaterialId};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("bow-cadence");
        clear_pad(&mut hg, Some(&mut server));
        arm_bow(&mut hg, Some((&mut server, slot)), 64);
        harness_step(&mut server, &mut hg);
        let elapsed = hold_right_click(&mut hg, Some(&mut server), 60, 160);
        settle(&mut server, &mut hg);
        let arrow = Item::Material(MaterialId::Arrow);
        let client_shots = 64 - held_units(&hg.state.players[0].inventory, &arrow);
        let sp = &server.server.players[slot];
        assert_eq!(sp.possession.shot_too_soon, 0, "no honest shot refused as too soon");
        assert_eq!(64 - held_units(&sp.inventory, &arrow), client_shots, "both copies paid each shot");
        let want = elapsed.div_ceil(crate::shot::SHOT_COOLDOWN_TICKS);
        assert!(u64::from(client_shots).abs_diff(want) <= 1, "{client_shots} shots in {elapsed} ticks (want {want})");
        assert_eq!((sp.possession.ammo_believed, sp.possession.mismatched), (0, 0));
    }

    /// C3d run 13 — a STRICT joiner holding right-click with its bow sends
    /// each shot as ONE typed action through the real `GameState::send_shot`
    /// (`strict_requests::route_shot`), never the legacy request: never torn
    /// down, every reservation retired, no Strict refusal, and each shot paid
    /// on both copies from the server's numbered Take (the client predicts
    /// nothing).
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_strict_joiner_holding_its_bow_is_served_typed() {
        use crate::item::{Item, MaterialId};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-strict-bow");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-strict-bow-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).expect("dedicated server starts");
        server.select_inventory_mode(crate::action_identity::InventoryMode::Strict).expect("Strict");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport), crate::remote_client::build_join_request_guest("Archer", 0), None));
        for _ in 0..5 { harness_step(&mut server, &mut hg); }
        let slot = server.server.players.len() - 1;
        assert_eq!(hg.state.remote_client.as_ref().unwrap().inventory_mode, crate::action_identity::InventoryMode::Strict);
        let applied = hg.state.remote_client.as_ref().unwrap().applied_window_boundary();
        crate::window_events::apply_through(&mut server.server.players[slot], applied, 0);
        let refill = hg.state.players[0].inventory.auto_refill;
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            *inv = crate::inventory::Inventory::new();
            inv.auto_refill = refill;
        }
        clear_pad(&mut hg, Some(&mut server));
        arm_bow(&mut hg, Some((&mut server, slot)), 16);
        harness_step(&mut server, &mut hg);
        hold_right_click(&mut hg, Some(&mut server), 60, 40);
        settle(&mut server, &mut hg);
        let client = hg.state.remote_client.as_ref().unwrap();
        assert!(client.is_connected(), "an honest Strict archer is never torn down");
        assert_eq!(client.action_journal.as_ref().map(|j| j.len()), Some(0), "every typed shot retired");
        let arrow = Item::Material(MaterialId::Arrow);
        let client_shots = 16 - held_units(&hg.state.players[0].inventory, &arrow);
        let sp = &server.server.players[slot];
        assert!(client_shots >= 2, "the held bow fired: {client_shots}");
        assert_eq!(16 - held_units(&sp.inventory, &arrow), client_shots, "both copies paid each shot");
        // C3d run 18 (run-17 review LOW-2) — world-fact refusals too.
        let t = sp.strict_refusals;
        assert_eq!((t.stale_world, t.other, t.world_fact), (0, 0, 0), "no Strict refusal");
        // Run-17 review LOW-1 — the game loop's shot bracket really compared.
        // Run-18 review M2: the guard is debug-only (a release run skips this).
        #[cfg(debug_assertions)]
        assert!(crate::strict_requests::checked::seen().contains(&"shot"), "the shot's NoPrediction bracket ran");
    }

    /// C3d run 16 — a STRICT joiner's swing through its REAL client
    /// (`GameState::send_entity_attack` → `strict_requests::route_attack`):
    /// ONE typed action, served from the server copy's sword; the hit lands
    /// (the chicken dies and the client's kill attribution counts it), the
    /// sword wears once on both copies from the server's numbered Wear, every
    /// reservation retired, no Strict refusal, never torn down.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_strict_joiners_swing_is_served_typed() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-strict-swing");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-strict-swing-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).expect("dedicated server starts");
        server.select_inventory_mode(crate::action_identity::InventoryMode::Strict).expect("Strict");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport), crate::remote_client::build_join_request_guest("Swinger", 0), None));
        for _ in 0..5 { harness_step(&mut server, &mut hg); }
        let slot = server.server.players.len() - 1;
        assert_eq!(hg.state.remote_client.as_ref().unwrap().inventory_mode, crate::action_identity::InventoryMode::Strict);
        let applied = hg.state.remote_client.as_ref().unwrap().applied_window_boundary();
        crate::window_events::apply_through(&mut server.server.players[slot], applied, 0);
        let refill = hg.state.players[0].inventory.auto_refill;
        let hot = hg.state.players[0].hotbar_slot;
        let iron = crate::crafting::Tool::new(crate::crafting::ToolType::Sword, crate::crafting::ToolMaterial::Iron);
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            *inv = crate::inventory::Inventory::new();
            inv.auto_refill = refill;
            inv.set_slot(hot, Some(crate::item::ItemStack { item: crate::item::Item::Tool(iron), count: 1 }));
        }
        clear_pad(&mut hg, Some(&mut server));
        harness_step(&mut server, &mut hg);
        let wear = |inv: &crate::inventory::Inventory| match inv.slot(hot).map(|s| &s.item) {
            Some(crate::item::Item::Tool(t)) => t.durability,
            other => panic!("the sword is gone: {other:?}"),
        };
        // A chicken one hit from death, right in front of the joiner's body.
        let sp = &server.server.players[slot];
        let at = sp.player.pos + crate::camera::forward_from(sp.yaw, 0.0) * 1.5;
        let chicken = crate::entity::spawn_mob(&mut server.server.ecs, crate::mob::MobType::Chicken, at);
        server.server.ecs.get::<&mut crate::combat::Health>(chicken).unwrap().current = 1.0;
        harness_step(&mut server, &mut hg);
        let id = server.server.ecs.get::<&crate::entity::ProtocolId>(chicken).expect("broadcast").0;
        server.server.ecs.get::<&mut crate::entity::Position>(chicken).unwrap().0 = at;
        hg.state.send_entity_attack(
            0,
            crate::remote_mobs::MirrorTarget { id, kind: crate::mob::MobType::Chicken, tamed: false, pack: false, baby: false, tethered: false, product_not_ready: false },
            false,
            false,
        );
        settle(&mut server, &mut hg);
        let client = hg.state.remote_client.as_ref().unwrap();
        assert!(client.is_connected(), "an honest Strict swing is never torn down");
        assert_eq!(client.action_journal.as_ref().map(|j| j.len()), Some(0), "the typed swing retired");
        assert_eq!(wear(&hg.state.players[0].inventory), iron.durability - 1, "the server's Wear applied on the client");
        let sp = &server.server.players[slot];
        assert_eq!(wear(&sp.inventory), iron.durability - 1, "and on the server copy");
        assert_eq!(
            hg.state.players[0].kill_counter.get(&crate::mob::MobType::Chicken).copied(),
            Some(1),
            "the hit landed: the server's KillEvent ran the joiner's kill attribution"
        );
        let t = sp.strict_refusals;
        assert_eq!((t.stale_world, t.other, t.world_fact), (0, 0, 0), "no Strict refusal");
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
        mine_joined_to_break(hg, server, slot, cell);
        for _ in 0..10 {
            hg.state.tick_accumulator = crate::TICK_DURATION;
            hg.frames(1);
            server.tick();
        }
    }

    /// [`mine_joined`] up to and including the frame the block breaks in,
    /// without the trailing frames: that frame's tick (and its movement
    /// packet) ran BEFORE its break arm, so whatever the break arm left for
    /// the movement packet is still waiting when this returns (run 11,
    /// run-9b review LOW-3).
    fn mine_joined_to_break(hg: &mut HeadlessGame, server: &mut crate::hosted_server::HostedServer, slot: usize, cell: [i32; 3]) {
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

    /// C3d run 19 — a lending host opens its door (the REAL use arm, a
    /// right-click): both halves reach a joiner open. (Before, the toggle
    /// arm wrote its own world and recorded nothing: a joiner kept the door
    /// shut.)
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_lending_hosts_door_toggle_reaches_a_joiner() {
        use crate::transport::ClientTransport;
        isolate_saves();
        let name = "harness-lend-door-toggle";
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
        hg.state.tick_accumulator = crate::TICK_DURATION;
        right_click(&mut hg);
        for c in [bottom, top] {
            assert!(crate::block_shape::is_open(hg.state.world.meta_at(c[0], c[1], c[2])), "the host opened its half at {c:?}");
        }
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
                seen.iter().any(|b| (b.x, b.y, b.z, b.new_block) == (c[0], c[1], c[2], crate::block::OAK_DOOR) && crate::block_shape::is_open(b.meta)),
                "the joiner hears the half at {c:?} open: {seen:?}"
            );
        }
    }

    /// C3d run 19 — a Mirror joiner opens a door through the REAL use arm (a
    /// right-click): ONE typed `ToggleBlock` reserved, its own door opened
    /// (`FlipEach`, as before), and the dedicated server's door opens too,
    /// served and retired.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_mirror_joiners_door_toggle_opens_the_servers_door() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-mirror-toggle");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-mirror-toggle-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport), crate::remote_client::build_join_request_guest("Doorman", 0), None));
        for _ in 0..5 { harness_step(&mut server, &mut hg); }
        assert_eq!(hg.state.remote_client.as_ref().unwrap().inventory_mode, crate::action_identity::InventoryMode::Mirror);
        let feet = clear_pad(&mut hg, Some(&mut server));
        let bottom = [feet[0], feet[1], feet[2] - 2];
        let top = [bottom[0], bottom[1] + 1, bottom[2]];
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_block(bottom[0], bottom[1], bottom[2], crate::block::OAK_DOOR);
            w.set_meta((bottom[0], bottom[1], bottom[2]), 0);
            w.set_block(top[0], top[1], top[2], crate::block::OAK_DOOR);
            w.set_meta((top[0], top[1], top[2]), crate::use_edits::door_top_meta(0));
        }
        for _ in 0..2 { harness_step(&mut server, &mut hg); }
        aim_at(&mut hg, glam::Vec3::new(bottom[0] as f32 + 0.5, bottom[1] as f32 + 0.5, bottom[2] as f32 + 0.5));
        right_click(&mut hg);
        let toggles = |hg: &HeadlessGame| hg.state.remote_client.as_ref().unwrap().action_journal.as_ref().unwrap().actions()
            .filter(|a| matches!(crate::action_wire::ActionPacket::decode(&a.intent).map(|p| p.intent), Ok(crate::action_wire::ActionIntent::ToggleBlock { cell, .. }) if cell == bottom))
            .count();
        assert_eq!(toggles(&hg), 1, "one ToggleBlock reserved");
        for c in [bottom, top] {
            assert!(crate::block_shape::is_open(hg.state.world.meta_at(c[0], c[1], c[2])), "the joiner opened its half at {c:?}");
        }
        for _ in 0..6 { harness_step(&mut server, &mut hg); }
        for c in [bottom, top] {
            assert!(crate::block_shape::is_open(server.server.world.meta_at(c[0], c[1], c[2])), "the server's half at {c:?} is open");
        }
        assert_eq!(toggles(&hg), 0, "the toggle retired");
        assert!(hg.state.remote_client.as_ref().unwrap().is_connected(), "never torn down");
    }

    /// C3d run 22 — a Mirror joiner places a door and clicks it in the same
    /// frame, its placement still waiting to be sent: the door opens on the
    /// clicker's screen at once, the toggle waits in the one ordered client
    /// queue behind the placement, and the server applies the placement
    /// THEN the toggle (were it sent first, the server would refuse it: no
    /// door there yet). A second joiner hears both. No toast, no lost click.
    /// (Run 21 refused the click while edits were unsent; on `bdd4fe87` the
    /// door always opened.) Run-21 review LOW-3 — every branch of
    /// `edits_unsent` is driven for real: an earlier edit, then a request
    /// queued behind it (`has_queued_requests`), then the door placement,
    /// which the order cut leaves in the input carry-over
    /// (`has_carry_over`) while the request goes; the toggle waits for that
    /// carry-over.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_mirror_door_click_queues_behind_unsent_edits() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-mirror-toggle-wait");
        hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
        let mut server = crate::hosted_server::HostedServer::start(
            0, format!("harness-mirror-toggle-wait-server-{}", std::process::id()), 42, 0,
            crate::hosted_server::RemoteTransport::WebSocket { port: 0 },
        ).expect("dedicated server starts");
        server.server.difficulty = crate::survival::Difficulty::Peaceful;
        let transport = server.attach_test_remote();
        hg.state.remote_client = Some(crate::remote_client::RemoteClient::from_transport(
            Box::new(transport), crate::remote_client::build_join_request_guest("Doorman", 0), None));
        let mut neighbour = crate::remote_client::RemoteClient::from_transport(
            Box::new(server.attach_test_remote()), crate::remote_client::build_join_request_guest("Neighbour", 0), None);
        for _ in 0..5 { harness_step(&mut server, &mut hg); neighbour.poll(); }
        assert_eq!(hg.state.remote_client.as_ref().unwrap().inventory_mode, crate::action_identity::InventoryMode::Mirror);
        let n_slot = neighbour.player_index().expect("the neighbour joined") as usize;
        let feet = clear_pad(&mut hg, Some(&mut server));
        let bottom = [feet[0], feet[1], feet[2] - 2];
        let top = [bottom[0], bottom[1] + 1, bottom[2]];
        let cs = crate::chunk::CHUNK_SIZE as i32;
        server.hold_column_for_test(n_slot, (bottom[0].div_euclid(cs), bottom[2].div_euclid(cs)));
        for _ in 0..2 { harness_step(&mut server, &mut hg); neighbour.poll(); }
        neighbour.pending_block_changes.clear();
        // An earlier edit (the floor it stands on, as it is), then a request
        // made behind it (a device click on a cell with no device: the
        // server ignores it), both waiting; then the placement: both halves
        // on the joiner's screen and waiting to be sent (the click frame runs
        // no tick, so no input goes before the use arm, which runs after the
        // frame's ticks).
        let floor = [feet[0], feet[1] - 1, feet[2]];
        let standing = hg.state.world.get_block(floor[0], floor[1], floor[2]);
        hg.state.pending_block_changes.push(crate::protocol::BlockChange { x: floor[0], y: floor[1], z: floor[2], new_block: standing, meta: 0 });
        hg.state.remote_client.as_mut().unwrap().queue_request(crate::window_ops::order_stamp(), crate::remote_client::Request::Device((floor[0], floor[1], floor[2])));
        let metas = [(bottom, 0u8), (top, crate::use_edits::door_top_meta(0))];
        for (c, m) in metas {
            hg.state.world.set_block(c[0], c[1], c[2], crate::block::OAK_DOOR);
            hg.state.world.set_meta((c[0], c[1], c[2]), m);
            hg.state.pending_block_changes.push(crate::protocol::BlockChange { x: c[0], y: c[1], z: c[2], new_block: crate::block::OAK_DOOR, meta: m });
        }
        assert_eq!(server.server.world.get_block(bottom[0], bottom[1], bottom[2]), crate::block::AIR, "the server has no door yet");
        aim_at(&mut hg, glam::Vec3::new(bottom[0] as f32 + 0.5, bottom[1] as f32 + 0.5, bottom[2] as f32 + 0.5));
        let toggles = |hg: &HeadlessGame| hg.state.remote_client.as_ref().unwrap().action_journal.as_ref().unwrap().actions()
            .filter(|a| matches!(crate::action_wire::ActionPacket::decode(&a.intent).map(|p| p.intent), Ok(crate::action_wire::ActionIntent::ToggleBlock { cell, .. }) if cell == bottom))
            .map(|a| a.sent)
            .collect::<Vec<_>>();
        hg.state.last_tick = std::time::Instant::now();
        hg.state.tick_accumulator = std::time::Duration::ZERO;
        let toast_before = toast_text(&hg);
        right_click(&mut hg);
        assert!(!hg.state.pending_block_changes.is_empty(), "the placement was still waiting at the click");
        for c in [bottom, top] {
            assert!(crate::block_shape::is_open(hg.state.world.meta_at(c[0], c[1], c[2])), "the door opens on the clicker's screen at once ({c:?})");
        }
        assert_eq!(toggles(&hg), vec![false], "one ToggleBlock reserved, not yet sent");
        assert!(hg.state.remote_client.as_ref().unwrap().has_queued_requests(), "it waits in the ordered queue behind the placement");
        assert_eq!(toast_text(&hg), toast_before, "no toast");
        // The first input carries the floor edit; the order cut keeps the
        // door placement (made after the queued request) in the carry-over,
        // the request goes, and the toggle still waits behind the placement.
        hg.state.network_send_input();
        let rc = hg.state.remote_client.as_ref().unwrap();
        assert!(rc.has_carry_over(), "the placement is carried over to the next input");
        assert!(rc.has_queued_requests(), "the toggle still waits behind it");
        assert_eq!(toggles(&hg), vec![false], "not sent ahead of the carried placement");
        // The next input carries the placement; the toggle goes right after.
        hg.state.network_send_input();
        let rc = hg.state.remote_client.as_ref().unwrap();
        assert!(!rc.has_carry_over() && !rc.has_queued_requests(), "the carry-over and the queue drained");
        assert_eq!(toggles(&hg), vec![true], "sent right after the input carrying the placement");
        for _ in 0..6 { harness_step(&mut server, &mut hg); neighbour.poll(); }
        for c in [bottom, top] {
            assert_eq!(server.server.world.get_block(c[0], c[1], c[2]), crate::block::OAK_DOOR, "the server applied the placement ({c:?})");
            assert!(crate::block_shape::is_open(server.server.world.meta_at(c[0], c[1], c[2])), "then the toggle: the server's half at {c:?} is open");
            assert!(crate::block_shape::is_open(hg.state.world.meta_at(c[0], c[1], c[2])), "and the clicker's stays open ({c:?})");
            let heard = neighbour.pending_block_changes.iter().rev().find(|b| [b.x, b.y, b.z] == c).map(|b| (b.new_block, crate::block_shape::is_open(b.meta)));
            assert_eq!(heard, Some((crate::block::OAK_DOOR, true)), "the neighbour hears the door, open, at {c:?}");
        }
        assert!(toggles(&hg).is_empty(), "the toggle retired");
        assert!(hg.state.remote_client.as_ref().unwrap().is_connected(), "never torn down");
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

    /// C3c-3b (decision 4; replaces C3c-3r's break-arm test, whose joiner
    /// still granted itself the wallpaper — the dupe) — a joiner breaks a
    /// block carrying a laid Blueprint and a wallpaper (on faces it didn't
    /// strike) through the REAL break arm: its client grants itself NOTHING
    /// (no wallpaper, no Plan, nothing on its own ground); the server spills
    /// each attachment once as a real ground item (one wallpaper block, one
    /// Plan with the server's body) and its stream takes both out of the
    /// client's copy. Both the survival and the creative break arm.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiners_break_grants_nothing_and_the_server_spills_its_attachments() {
        use crate::item::Item;
        use crate::world::FaceAttachment;
        for creative in [false, true] {
            isolate_saves();
            let tag = if creative { "break-att-creative" } else { "break-att-survival" };
            let (mut hg, mut server, slot) = joined_window_client(tag);
            if creative {
                hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
            }
            let feet = clear_pad(&mut hg, Some(&mut server));
            hold_feet_column(&mut server, slot, feet);
            // Three ahead: past the server body's pickup reach of what spills.
            let cell = [feet[0], feet[1], feet[2] - 3];
            let pos = (cell[0], cell[1], cell[2]);
            let (top, west) = (crate::mesh::Face::Top.index(), crate::mesh::Face::West.index());
            let plan = crate::plan::PlanData::debug_3x3_stone();
            for w in [&mut hg.state.world, &mut server.server.world] {
                w.set_block(cell[0], cell[1], cell[2], crate::block::STONE);
                w.set_face_attachment(pos, west, FaceAttachment::Wallpaper(crate::block::OAK_PLANKS));
            }
            // The server holds the real Blueprint; the client its render stub.
            server.server.world.set_face_attachment(pos, top, FaceAttachment::Blueprint(Box::new(plan.clone())));
            hg.state.world.set_face_attachment(pos, top, FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::render_stub(false))));
            harness_step(&mut server, &mut hg);
            mine_joined(&mut hg, &mut server, slot, cell);
            for _ in 0..4 {
                harness_step(&mut server, &mut hg);
            }
            assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), crate::block::AIR, "the block broke ({tag})");
            assert_eq!(server.server.world.get_block(cell[0], cell[1], cell[2]), crate::block::AIR, "on the server too ({tag})");
            let inv = &hg.state.players[0].inventory;
            assert_eq!(held_units(inv, &Item::Block(crate::block::OAK_PLANKS)), 0, "the client granted itself no wallpaper ({tag})");
            assert!(!inv.slots_iter().flatten().any(|s| matches!(s.item, Item::Plan(_))), "nor a Plan ({tag})");
            assert_eq!(ground_units(&hg.state.ecs, &Item::Block(crate::block::OAK_PLANKS)), 0, "nor dropped one of its own ({tag})");
            assert_eq!(ground_units(&server.server.ecs, &Item::Block(crate::block::OAK_PLANKS)), 1, "one wallpaper on the server's ground ({tag})");
            let ground_plans: Vec<_> = server
                .server
                .ecs
                .query::<&crate::entity::ItemEntity>()
                .iter()
                .filter_map(|(_, it)| match &it.stack.item {
                    Item::Plan(p) => Some(p.cells.len()),
                    _ => None,
                })
                .collect();
            assert_eq!(ground_plans, vec![plan.cells.len()], "one ground Plan with the server's body ({tag})");
            for face in [top, west] {
                assert!(server.server.world.face_attachment_at(pos, face).is_none(), "gone from the server ({tag})");
                assert!(hg.state.world.face_attachment_at(pos, face).is_none(), "and from the client's copy, by the stream ({tag})");
            }
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

    /// C3c-3-fix (M1) — fuelling a Steam Generator while joined: the fuel
    /// stays in hand, no private device is made in the client's copy, nothing
    /// is asked of the server, and the toast says so. Alone, the same click
    /// fuels it.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_cannot_fuel_a_steam_generator() {
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("gen-fuel");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let cell = (feet[0], feet[1], feet[2] - 2);
        let aim = glam::Vec3::new(cell.0 as f32 + 0.5, cell.1 as f32 + 0.3, cell.2 as f32 + 0.5);
        hg.state.world.set_block(cell.0, cell.1, cell.2, crate::block::STEAM_GENERATOR);
        server.server.world.set_block(cell.0, cell.1, cell.2, crate::block::STEAM_GENERATOR);
        let coal = crate::item::ItemStack::new_material(crate::item::MaterialId::Coal, 5);
        hg.state.players[0].inventory.set_slot(0, Some(coal.clone()));
        hg.state.players[0].hotbar_slot = 0;
        harness_step(&mut server, &mut hg);
        hg.state.toast = None;
        aim_at(&mut hg, aim);
        right_click(&mut hg);
        assert_eq!(hg.state.players[0].inventory.slot(0), Some(&coal), "the coal is still in hand");
        assert!(hg.state.world.power_device_at(cell).is_none(), "no private device made");
        assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_MACHINE_TOAST));
        assert_eq!(hg.state.joiner_actions.len(), 0, "nothing asked of the server");
        harness_step(&mut server, &mut hg);
        assert!(server.server.world.power_device_at(cell).is_none_or(|d| d.fuel.is_none()), "the real generator got nothing");
        // Alone, the same click fuels it.
        hg.state.remote_client = None;
        hg.state.toast = None;
        right_click(&mut hg);
        assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(4), "alone, one coal goes in");
        assert!(hg.state.world.power_device_at(cell).is_some_and(|d| d.fuel.is_some()), "and the device holds it");
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
        // C3c-3b — and in the server's world: the commit spends it there too.
        for w in [&mut hg.state.world, &mut server.server.world] {
            w.set_face_attachment(tile, top, FaceAttachment::BlueprintBlank);
            w.set_block(tile.0, tile.1 + 1, tile.2, crate::block::OAK_PLANKS);
        }
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
        assert_eq!(server.server.world.face_attachment_at(tile, top), Some(&FaceAttachment::BlueprintBlank), "the server's too");
        // 2. Room (slot 5): the Plan lands on both sides.
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            inv.set_slot(5, None);
        }
        hg.state.players[0].pending_capture = Some(pending());
        hg.state.confirm_capture(0);
        let plan = plan_in(&hg.state.players[0].inventory, 5).expect("the client's Plan");
        assert_eq!(hg.state.world.face_attachment_at(tile, top), Some(&FaceAttachment::BlueprintBlank), "joined paper waits for authority");
        for _ in 0..3 {
            harness_step(&mut server, &mut hg);
        }
        let sp = &server.server.players[slot];
        assert_eq!(plan_in(&sp.inventory, 5).and_then(|p| p.marker), Some(crate::plan::marker(&plan)));
        assert_eq!(sp.inventory.slots_iter().flatten().filter(|s| s.item == stone.item).count(), 35, "nothing spent");
        assert_eq!((sp.possession.plan_minted, sp.possession.plan_mismatch), (1, 0));
        // C3c-3b (decision 5) — the server spent the capture's paper, so a
        // second capture over that spot finds none.
        assert!(server.server.world.face_attachment_at(tile, top).is_none(), "the server's tile paper is spent");
        assert!(crate::plan::capture(&server.server.world, tile, "survival").is_err(), "nothing to capture twice");
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
        assert!(hg.state.players[0].inventory.slot(0).is_none(), "the client paid for the framed paper");
        copy_holds(&mut server, &mut hg, slot, &Item::Block(crate::block::BLUEPRINT_PAPER), 0);
        let sp = &server.server.players[slot];
        assert_eq!(sp.possession.plan_minted, 0, "nothing reported");
        assert!(!has_plan(&sp.inventory) && !has_plan(&hg.state.players[0].inventory));
        assert!(hg.state.players[0].inventory.slot(0).is_none() && sp.inventory.slot(0).is_none(), "the paper is in the frame");
    }

    // ─── C3c-3b ────────────────────────────────────────────────────────────

    /// C3c-3b — the server holds the column of `feet` as pushed to joiner
    /// `slot`, so the attachment stream reaches it.
    fn hold_feet_column(server: &mut crate::hosted_server::HostedServer, slot: usize, feet: [i32; 3]) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        server.hold_column_for_test(slot, (feet[0].div_euclid(cs), feet[2].div_euclid(cs)));
    }

    /// C3c-3b — step until the server's copy of joiner `slot`'s window holds
    /// `n` of `item` (its window events wait for the client's report, which
    /// the real-time loopback can delay past a fixed number of steps).
    fn copy_holds(server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame, slot: usize, item: &crate::item::Item, n: u32) {
        for _ in 0..40 {
            if held_units(&server.server.players[slot].inventory, item) == n {
                return;
            }
            harness_step(server, hg);
        }
        assert_eq!(held_units(&server.server.players[slot].inventory, item), n, "the server's copy of the window");
    }

    /// C3c-3b (decisions 2, 3) — through the REAL arms, a joiner paints a
    /// wallpaper on a wall, lays blank paper on the floor and peels the
    /// wallpaper back. Each click changes nothing in its own world or bag:
    /// it asks the server (`Attach`, `Detach`). Once answered, the server's
    /// world has each change, the client's copy shows it through the stream,
    /// and its bag pays for each lay and gets the peeled block back as a
    /// grant — the server's copy of its window in step.
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
    fn game_harness_a_joiner_paints_lays_and_peels_by_request() {
        use crate::item::{Item, ItemStack};
        use crate::world::FaceAttachment;
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("attach");
        let feet = clear_pad(&mut hg, Some(&mut server));
        hold_feet_column(&mut server, slot, feet);
        let wp = Item::Block(crate::block::WALLPAPER_RED);
        let paper = Item::Block(crate::block::BLUEPRINT_PAPER);
        for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
            inv.set_slot(0, Some(ItemStack { item: wp.clone(), count: 2 }));
            inv.set_slot(1, Some(ItemStack { item: paper.clone(), count: 2 }));
        }
        hg.state.players[0].hotbar_slot = 0;
        let wall = wall_ahead(&mut hg, &mut server, feet);
        let pos = (wall[0], wall[1], wall[2]);
        let south = crate::mesh::Face::South.index();
        harness_step(&mut server, &mut hg);
        // 1. Paint: asked, nothing changed here yet.
        aim_at_wall(&mut hg, wall);
        right_click(&mut hg);
        assert!(hg.state.world.face_attachment_at(pos, south).is_none(), "the client paints nothing itself");
        assert_eq!(held_units(&hg.state.players[0].inventory, &wp), 2, "nor pays before the answer");
        assert_eq!(hg.state.joiner_actions.len(), 1, "one Attach in flight");
        settle(&mut server, &mut hg);
        assert_eq!(server.server.world.face_attachment_at(pos, south), Some(&FaceAttachment::Wallpaper(crate::block::WALLPAPER_RED)), "the server's world");
        assert_eq!(hg.state.world.face_attachment_at(pos, south), Some(&FaceAttachment::Wallpaper(crate::block::WALLPAPER_RED)), "the painter's copy, by the stream");
        assert_eq!(held_units(&hg.state.players[0].inventory, &wp), 1, "paid one");
        // The take is a window event the server's copy applies once the
        // client's input says it paid (the loopback is real-time: step for it).
        copy_holds(&mut server, &mut hg, slot, &wp, 1);
        // 2. Lay blank paper on the floor's Top face.
        hg.state.players[0].hotbar_slot = 1;
        let floor = (feet[0], feet[1] - 1, feet[2] - 2);
        let top = crate::mesh::Face::Top.index();
        aim_at(&mut hg, glam::Vec3::new(floor.0 as f32 + 0.5, floor.1 as f32 + 0.98, floor.2 as f32 + 0.5));
        right_click(&mut hg);
        assert!(hg.state.world.face_attachment_at(floor, top).is_none(), "the client lays nothing itself");
        settle(&mut server, &mut hg);
        assert_eq!(server.server.world.face_attachment_at(floor, top), Some(&FaceAttachment::BlueprintBlank));
        assert_eq!(hg.state.world.face_attachment_at(floor, top), Some(&FaceAttachment::BlueprintBlank));
        assert_eq!(held_units(&hg.state.players[0].inventory, &paper), 1);
        // 3. Peel the wallpaper: one strike asks once and breaks nothing.
        strike(&mut hg, glam::Vec3::new(wall[0] as f32 + 0.5, wall[1] as f32 + 0.5, wall[2] as f32 + 0.9), 20);
        assert_eq!(hg.state.world.get_block(wall[0], wall[1], wall[2]), crate::block::STONE, "the strike broke nothing");
        assert!(hg.state.world.face_attachment_at(pos, south).is_some(), "and peeled nothing itself");
        settle(&mut server, &mut hg);
        assert!(server.server.world.face_attachment_at(pos, south).is_none(), "peeled on the server");
        assert!(hg.state.world.face_attachment_at(pos, south).is_none(), "and in the client's copy, by the stream");
        assert_eq!(held_units(&hg.state.players[0].inventory, &wp), 2, "the block came back as a grant");
        copy_holds(&mut server, &mut hg, slot, &wp, 2);
        assert_eq!(server.server.world.get_block(wall[0], wall[1], wall[2]), crate::block::STONE);
        let sp = &server.server.players[slot];
        assert_eq!(sp.possession.mismatched, 0, "every take paid from the server's copy");
    }
    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_smallfix_refused_plan_is_visible_on_first_closed_and_book_frame() {
        use crate::item::{Item, ItemStack};
        use crate::plan::PlanData;
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("smallfix-cursor");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let plan = PlanData { name: "Smallfix cursor identity 92741".to_string(), ..PlanData::debug_3x3_stone() };
        let stone = ItemStack::new_block(crate::block::STONE, 64);
        for k in 1..36 {
            hg.state.players[0].inventory.set_slot(k, Some(stone.clone()));
            server.server.players[slot].inventory.set_slot(k, Some(stone.clone()));
        }
        hg.state.players[0].inventory.set_slot(0, Some(ItemStack { item: Item::Plan(plan.clone()), count: 1 }));
        server.server.players[slot].inventory.set_slot(0, Some(ItemStack {
            item: Item::Plan(PlanData::marker_placeholder(crate::plan::marker(&plan), true)), count: 1 }));
        hg.state.players[0].hotbar_slot = 0;
        let wall = wall_ahead(&mut hg, &mut server, feet);
        foreign_plot_on_server(&mut server, wall);
        harness_step(&mut server, &mut hg);
        aim_at_wall(&mut hg, wall);
        right_click(&mut hg);
        assert!(hg.state.players[0].inventory.slot(0).is_none(), "the real hang spent its Plan");
        hg.state.players[0].inventory.set_slot(0, Some(stone));
        assert!(!hg.state.players[0].crafting_ui.open);
        let captured = hg.state.input.cursor_captured;
        let mut found = false;
        for _ in 0..60 {
            harness_step(&mut server, &mut hg);
            if hg.state.players[0].crafting_ui.cursor_item.is_some() { found = true; break; }
        }
        assert!(found, "the actual refusal returned the full Plan onto the cursor");
        let cursor = hg.state.players[0].crafting_ui.cursor_item.as_ref().unwrap();
        assert_eq!(cursor, &ItemStack { item: Item::Plan(plan.clone()), count: 1 });
        assert_eq!(hg.state.input.cursor_captured, captured, "painting does not release capture");
        let texts = |shapes: &[egui::epaint::ClippedShape]| shapes.iter().filter_map(|s| {
            if let egui::Shape::Text(t) = &s.shape { Some(t.galley.text()) } else { None }
        }).filter(|text| *text == plan.name).count();
        assert_eq!(texts(&hg.state.renderer.egui.last_shapes), 1, "first refusal frame paints its unique carried name once");
        let centre = egui::pos2(668.0, 388.0);
        assert!(hg.state.renderer.egui.last_shapes.iter().any(|s| match &s.shape {
            egui::Shape::Rect(r) => (r.rect.center() - centre).length() < 0.1 && (r.rect.width() - 38.4).abs() < 0.1,
            _ => false,
        }), "the actual carried icon uses the pointer-free viewport anchor");
        hg.state.players[0].crafting_ui.open = true;
        hg.state.players[0].crafting_ui.book_open = true;
        hg.frames(1);
        assert_eq!(texts(&hg.state.renderer.egui.last_shapes), 1, "book paints the cursor exactly once");
        hg.state.players[0].crafting_ui.book_open = false;
        hg.state.players[0].crafting_ui.pad_focus = Some(crate::craft_ui::PadSlot::Inv(5));
        hg.state.players[0].inventory.set_slot(5, None);
        hg.frames(1);
        assert_eq!(texts(&hg.state.renderer.egui.last_shapes), 1, "grid paints the cursor exactly once");
        // A newly opened egui Area has sizing/fade frames before its slot
        // border is opaque. The first carried paint assertion above remains
        // immediate; seating waits for the actual visible panel geometry.
        for _ in 0..20 {
            if hg.state.renderer.egui.last_shapes.iter().any(|s| matches!(&s.shape,
                egui::Shape::Rect(r) if r.stroke.color == egui::Color32::from_rgb(255, 210, 80))) { break; }
            hg.frames(1);
        }
        let target_rect = hg.state.renderer.egui.last_shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.stroke.color == egui::Color32::from_rgb(255, 210, 80) => Some(r.rect),
            _ => None,
        }).expect("the actual focused slot has its visible controller border");
        let viewport = crate::screen::ViewportRect::full(1280, 720);
        let ctx = hg.state.renderer.egui.ctx.clone();
        let mut actual_click = crate::craft_ui::ClickTarget::None;
        for pressed in [true, false] {
            ctx.begin_pass(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                events: vec![egui::Event::PointerMoved(target_rect.center()), egui::Event::PointerButton {
                    pos: target_rect.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() }],
                ..Default::default() });
            let p = &mut hg.state.players[0];
            p.crafting_ui.pad_focus = None;
            actual_click = crate::craft_ui::draw_crafting_ui(&ctx, &viewport, 0, &mut p.crafting_ui,
                &p.inventory, &p.armour_slots, &hg.state.registry, &hg.state.renderer.egui);
            let _ = ctx.end_pass();
        }
        assert!(matches!(actual_click, crate::craft_ui::ClickTarget::InventorySlot(5, false)),
            "the carried painter over the slot lets its real pointer click through");
        let p = &mut hg.state.players[0];
        assert!(p.crafting_ui.click_inventory_slot(5, &mut p.inventory, false), "ordinary slot seating succeeds");
        assert_eq!(plan_in(&p.inventory, 5), Some(plan));
        assert!(p.crafting_ui.cursor_item.is_none());
        hg.frames(1);
        assert_eq!(hg.state.players[0].inventory.slots_iter().flatten().filter(|s| matches!(s.item, Item::Plan(_))).count(), 1);
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_smallfix_four_economy_placements_cost_once_and_explain_unavailable_setup() {
        use crate::block;
        use crate::item::{Item, ItemStack};
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("smallfix-economy-place");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let floor = [feet[0], feet[1] - 1, feet[2] - 3];
        let cell = [floor[0], floor[1] + 1, floor[2]];
        for kind in [block::VENDOR_BLOCK, block::TIP_JAR, block::MARKET_BELL, block::AUCTION_BLOCK] {
            for w in [&mut hg.state.world, &mut server.server.world] {
                w.set_block(cell[0], cell[1], cell[2], block::AIR);
            }
            let item = Item::Block(kind);
            for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
                inv.set_slot(0, Some(ItemStack { item: item.clone(), count: 2 }));
            }
            hg.state.players[0].hotbar_slot = 0;
            harness_step(&mut server, &mut hg);
            aim_at(&mut hg, glam::Vec3::new(floor[0] as f32 + 0.5, floor[1] as f32 + 0.98, floor[2] as f32 + 0.5));
            right_click(&mut hg);
            assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), kind, "placed {kind}");
            assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_ECONOMY_TOAST));
            settle(&mut server, &mut hg);
            copy_holds(&mut server, &mut hg, slot, &item, 1);
            assert_eq!(server.server.world.get_block(cell[0], cell[1], cell[2]), kind, "authority placed {kind}");
            assert_eq!(held_units(&hg.state.players[0].inventory, &item), 1, "one cost {kind}");
            hg.state.toast = None;
            aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.3, cell[2] as f32 + 0.5));
            right_click(&mut hg);
            let p = &hg.state.players[0];
            assert!(p.open_vendor.is_none() && p.open_tip_jar.is_none() && p.open_market_hub.is_none() && p.open_auction.is_none());
            assert_eq!(toast_text(&hg).as_deref(), Some(crate::remote_mobs::JOINED_ECONOMY_TOAST));
            assert_eq!(held_units(&p.inventory, &item), 1, "refused setup costs nothing");
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_smallfix_eraser_and_place_over_leave_attachment_copies_until_authority() {
        use crate::block;
        use crate::item::{Item, ItemStack};
        use crate::world::FaceAttachment;
        isolate_saves();
        let (mut hg, mut server, slot) = joined_window_client("smallfix-attachment-copies");
        let feet = clear_pad(&mut hg, Some(&mut server));
        let floor = [feet[0], feet[1] - 1, feet[2] - 2];
        let cell = [floor[0], floor[1] + 1, floor[2]];
        let pos = (cell[0], cell[1], cell[2]);
        let face = crate::mesh::Face::South.index();
        for erase in [true, false] {
            for refuse in [true, false] {
                server.server.world.plots.clear();
                let old = if erase { block::BLUEPRINT_PAPER } else { block::AIR };
                for w in [&mut hg.state.world, &mut server.server.world] {
                    w.set_block(cell[0], cell[1], cell[2], old);
                    w.set_face_attachment(pos, face, FaceAttachment::Wallpaper(block::WALLPAPER_RED));
                }
                if refuse { foreign_plot_on_server(&mut server, cell); }
                let item = if erase { Item::Tool(crate::crafting::Tool::new(crate::crafting::ToolType::Eraser, crate::crafting::ToolMaterial::Wood)) } else { Item::Block(block::OAK_PLANKS) };
                for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
                    inv.set_slot(0, Some(ItemStack { item: item.clone(), count: if erase { 1 } else { 2 } }));
                }
                hg.state.players[0].hotbar_slot = 0;
                harness_step(&mut server, &mut hg);
                let aim = if erase { glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5) }
                    else { glam::Vec3::new(floor[0] as f32 + 0.5, floor[1] as f32 + 0.98, floor[2] as f32 + 0.5) };
                aim_at(&mut hg, aim); right_click(&mut hg);
                assert_eq!(hg.state.world.face_attachment_at(pos, face), Some(&FaceAttachment::Wallpaper(block::WALLPAPER_RED)),
                    "the real joined arm leaves paper copies immediately (erase {erase}, refused {refuse})");
                let predicted = if erase { block::AIR } else { block::OAK_PLANKS };
                assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), predicted, "the actual arm made its optimistic edit");
                let authoritative = if refuse { old } else { predicted };
                let expected = refuse.then_some(FaceAttachment::Wallpaper(block::WALLPAPER_RED));
                // These are block edits, not ItemActions. Wait for their actual
                // correction/attachment stream rather than an empty request list.
                for _ in 0..40 {
                    harness_step(&mut server, &mut hg);
                    if server.server.world.get_block(cell[0], cell[1], cell[2]) == authoritative
                        && hg.state.world.get_block(cell[0], cell[1], cell[2]) == authoritative
                        && hg.state.world.face_attachment_at(pos, face) == expected.as_ref()
                        && (!refuse || !erase || hg.state.sent_uses.len() == 0) { break; }
                }
                assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), authoritative, "the client received authority correction (erase {erase}, refused {refuse})");
                if refuse && erase {
                    assert_eq!(hg.state.sent_uses.len(), 0, "the Eraser refusal reached its actual undo callback");
                }
                assert_eq!(server.server.world.face_attachment_at(pos, face), expected.as_ref(), "erase {erase}, refused {refuse}: actual authority");
                assert_eq!(hg.state.world.face_attachment_at(pos, face), expected.as_ref(), "only the actual stream clears accepted edits");
                assert_eq!(server.server.world.get_block(cell[0], cell[1], cell[2]), if refuse { old } else if erase { block::AIR } else { block::OAK_PLANKS });
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_smallfix_carried_paint_stays_in_viewport_and_below_death_priority() {
        use crate::item::{Item, ItemStack};
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("smallfix-carried-viewports");
        let plan = crate::plan::PlanData { name: "Smallfix viewport cursor 19037".to_string(), ..crate::plan::PlanData::debug_3x3_stone() };
        let stack = ItemStack { item: Item::Plan(plan.clone()), count: 1 };
        let viewport = crate::screen::ViewportRect { x: 640, y: 0, width: 640, height: 720 };
        let clip = egui::Rect::from_min_size(egui::pos2(640.0, 0.0), egui::vec2(640.0, 720.0));
        let ctx = hg.state.renderer.egui.ctx.clone();
        for controller in [false, true] {
            ctx.begin_pass(egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0))], ..Default::default() });
            crate::craft_ui::paint_carried_item(&ctx, &viewport, 1, Some(&stack), None, controller, &hg.state.registry, &hg.state.renderer.egui);
            let output = ctx.end_pass();
            assert!(output.shapes.iter().all(|s| s.clip_rect == clip));
            assert!(output.shapes.iter().any(|s| match &s.shape {
                egui::Shape::Rect(r) => (r.rect.center() - egui::pos2(988.0, 388.0)).length() < 0.1 && (r.rect.width() - 38.4).abs() < 0.1,
                _ => false,
            }), "foreign pointer or missing controller uses its own viewport fallback");
        }
        hg.state.players[0].crafting_ui.cursor_item = Some(stack);
        hg.state.players[0].combat.dead = true;
        hg.frames(1);
        assert!(hg.state.renderer.egui.last_shapes.iter().all(|s| match &s.shape {
            egui::Shape::Text(t) => t.galley.text() != plan.name,
            _ => true,
        }), "death screen keeps priority over carried paint");
        hg.state.players[0].combat.dead = false;
        hg.state.controls_card_open = true;
        hg.frames(1);
        assert!(hg.state.renderer.egui.last_shapes.iter().all(|s| match &s.shape {
            egui::Shape::Text(t) => t.galley.text() != plan.name,
            _ => true,
        }), "the blocking Controls card also keeps priority");
    }

    /// Drive this fixture with the existing schedule until its actual state
    /// changes; do not assume a packet or receipt fits one frame/tick.
    fn pack_wait(server: &mut crate::hosted_server::HostedServer, hg: &mut HeadlessGame, mut ready: impl FnMut(&crate::hosted_server::HostedServer, &HeadlessGame) -> bool) {
        for _ in 0..200 {
            if ready(server, hg) { return; }
            harness_step(server, hg);
            assert!(hg.state.remote_client.as_ref().is_some_and(|remote| remote.is_connected()), "pack state wait lost its connection");
        }
        assert!(ready(server, hg), "bounded pack state wait did not converge");
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_d2c2_joined_pack_round_trip_and_current_authority_view_in_both_modes() {
        use crate::{action_identity::InventoryMode, container_window::{ContainerClick, ContainerTarget}, pack_wire::PackOperation};
        use winit::keyboard::KeyCode;
        isolate_saves();
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let (mut hg, mut server, slot) = joined_window_client_mode(&format!("pack-{mode:?}"), mode);
            clear_pad(&mut hg, Some(&mut server));
            let at = server.server.players[slot].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
            let steed = crate::entity::spawn_mob(&mut server.server.ecs, crate::mob::MobType::Donkey, at);
            server.server.ecs.get::<&mut crate::horse_ai::HorseData>(steed).unwrap().keeper = Some(crate::horse_ai::SteedKeeper::LegacyLocal(0));
            pack_wait(&mut server, &mut hg, |server, hg| server.server.ecs.get::<&crate::entity::ProtocolId>(steed).is_ok_and(|key| hg.state.remote_mobs.position_of(key.0).is_some()));
            let id = server.server.ecs.get::<&crate::entity::ProtocolId>(steed).unwrap().0;
            let target = ContainerTarget::Entity(id);
            for (place, stack) in [(0, crate::item::ItemStack::new_block(crate::block::CHEST, 1)), (1, crate::item::ItemStack::new_block(crate::block::STONE, 4))] {
                hg.state.players[0].inventory.set_slot(place, Some(stack.clone()));
                server.server.players[slot].inventory.set_slot(place, Some(stack));
            }
            hg.state.players[0].hotbar_slot = 0;
            aim_at(&mut hg, at + glam::Vec3::Y * 0.7);
            pack_wait(&mut server, &mut hg, |server, hg| (server.server.players[slot].yaw - hg.state.players[0].camera.yaw).abs() < 0.01);
            right_click(&mut hg); // Actual kept-unpacked Chest interaction.
            assert!(hg.state.pack_window_frozen(0));
            let before = crate::staged_window::MarkerWindow::capture_parts(&hg.state.players[0].inventory, &hg.state.players[0].armour_slots, &hg.state.players[0].crafting_ui.cursor_item, &hg.state.players[0].crafting_ui.grid, &hg.state.registry).slots();
            hg.state.send_drop_request(0, 0);
            hg.state.apply_window_click(0, &crate::window::WindowClick::Sort);
            let after = crate::staged_window::MarkerWindow::capture_parts(&hg.state.players[0].inventory, &hg.state.players[0].armour_slots, &hg.state.players[0].crafting_ui.cursor_item, &hg.state.players[0].crafting_ui.grid, &hg.state.registry).slots();
            assert_eq!(after, before, "held physical mutation is refused in either mode");
            pack_wait(&mut server, &mut hg, |server, hg| server.server.ecs.get::<&crate::horse_ai::HorseData>(steed).is_ok_and(|data| data.pack.is_some()) && !hg.state.pack_window_frozen(0) && server.server.players[slot].pack_flight.is_none());
            assert!(hg.state.players[0].inventory.slot(0).is_none(), "one actual Chest was consumed");
            hg.state.input.key_pressed(KeyCode::ShiftLeft);
            right_click(&mut hg); // Actual sneak/empty existing-pack interaction.
            hg.state.input.key_released(KeyCode::ShiftLeft);
            pack_wait(&mut server, &mut hg, |server, hg| hg.state.players[0].shared_container.as_ref().is_some_and(|mirror| mirror.target == target) && !hg.state.pack_window_frozen(0) && server.server.players[slot].pack_flight.is_none());
            let session = hg.state.players[0].shared_container.as_ref().unwrap().pack_session.unwrap();
            assert!(hg.state.request_pack_action(0, target, PackOperation::Click { incarnation: session.incarnation, revision: session.revision, click: ContainerClick::Deposit { slot: 1, all: true } }));
            pack_wait(&mut server, &mut hg, |server, hg| !hg.state.pack_window_frozen(0) && server.server.players[slot].pack_flight.is_none() && server.server.ecs.get::<&crate::horse_ai::HorseData>(steed).is_ok_and(|data| data.pack.as_ref().unwrap().slots[0].as_ref().is_some_and(|stack| stack.count == 4)));
            assert!(hg.state.players[0].inventory.slot(1).is_none());
            // Later authoritative stock is independently pushed to the
            // already-open client; the actual lent host click is tested below.
            server.server.ecs.get::<&mut crate::horse_ai::HorseData>(steed).unwrap().pack.as_mut().unwrap().slots[1] = Some(crate::item::ItemStack::new_block(crate::block::STONE, 2));
            let revision = crate::pack_state::revision(&server.server.ecs, steed) + 1;
            server.server.ecs.insert_one(steed, crate::pack_state::PackRevision(revision)).unwrap();
            pack_wait(&mut server, &mut hg, |_, hg| hg.state.players[0].shared_container.as_ref().is_some_and(|mirror| mirror.pack_session.is_some_and(|session| session.revision == revision)));
            let session = hg.state.players[0].shared_container.as_ref().unwrap().pack_session.unwrap();
            assert!(hg.state.request_pack_action(0, target, PackOperation::Click { incarnation: session.incarnation, revision: session.revision, click: ContainerClick::TakeAll }));
            pack_wait(&mut server, &mut hg, |server, hg| !hg.state.pack_window_frozen(0) && server.server.players[slot].pack_flight.is_none());
            let stone = hg.state.players[0].inventory.slots_iter().flatten().filter(|stack| stack.item == crate::item::Item::Block(crate::block::STONE)).map(|stack| u32::from(stack.count)).sum::<u32>();
            assert_eq!(stone, 6);
            let session = hg.state.players[0].shared_container.as_ref().unwrap().pack_session.unwrap();
            assert!(hg.state.request_pack_action(0, target, PackOperation::Remove { incarnation: session.incarnation, revision: session.revision }));
            pack_wait(&mut server, &mut hg, |server, hg| !hg.state.pack_window_frozen(0) && server.server.players[slot].pack_flight.is_none() && server.server.ecs.get::<&crate::horse_ai::HorseData>(steed).is_ok_and(|data| data.pack.is_none()));
            assert!(hg.state.players[0].shared_container.is_none());
            assert_eq!(hg.state.players[0].inventory.slots_iter().flatten().filter(|stack| stack.item == crate::item::Item::Block(crate::block::CHEST)).map(|stack| u32::from(stack.count)).sum::<u32>(), 1);
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_d2c2_actual_overflowing_prefix_feedback_survives_pack_hold_and_returns_once() {
        use crate::{action_identity::InventoryMode, container_window::ContainerTarget, pack_wire::PackOperation};
        isolate_saves();
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let (mut hg, mut server, slot) = joined_window_client_mode(&format!("pack-unfit-{mode:?}"), mode);
            clear_pad(&mut hg, Some(&mut server));
            let at = server.server.players[slot].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
            let steed = crate::entity::spawn_mob(&mut server.server.ecs, crate::mob::MobType::Donkey, at);
            {
                let mut data = server.server.ecs.get::<&mut crate::horse_ai::HorseData>(steed).unwrap();
                data.keeper = Some(crate::horse_ai::SteedKeeper::LegacyLocal(0));
                data.pack = Some(crate::chest::ChestData::new());
            }
            pack_wait(&mut server, &mut hg, |server, hg| server.server.ecs.get::<&crate::entity::ProtocolId>(steed).is_ok_and(|key| hg.state.remote_mobs.position_of(key.0).is_some()));
            aim_at(&mut hg, at + glam::Vec3::Y * 0.7);
            pack_wait(&mut server, &mut hg, |server, hg| (server.server.players[slot].yaw - hg.state.players[0].camera.yaw).abs() < 0.01);
            for place in 0..36 {
                let stack = crate::item::ItemStack::new_block(crate::block::DIRT, 64);
                hg.state.players[0].inventory.set_slot(place, Some(stack.clone()));
                server.server.players[slot].inventory.set_slot(place, Some(stack));
            }
            let id = server.server.ecs.get::<&crate::entity::ProtocolId>(steed).unwrap().0;
            server.grant_to_joiner(slot, [crate::item::ItemStack::new_block(crate::block::STONE, 1)]);
            let grant_event = server.server.players[slot].window_events.last_number();
            assert!(hg.state.request_pack_action(0, ContainerTarget::Entity(id), PackOperation::Open));
            assert!(hg.state.pack_window_frozen(0));
            let floor = |server: &crate::hosted_server::HostedServer| server.server.ecs.query::<&crate::entity::ItemEntity>().iter().filter(|(_, drop)| drop.stack.item == crate::item::Item::Block(crate::block::STONE)).map(|(_, drop)| u32::from(drop.stack.count)).sum::<u32>();
            pack_wait(&mut server, &mut hg, |server, hg| !hg.state.pack_window_frozen(0) && server.server.players[slot].pack_flight.is_none() && floor(server) == 1);
            assert_eq!(hg.state.players[0].inventory.slots_iter().flatten().filter(|stack| stack.item == crate::item::Item::Block(crate::block::STONE)).map(|stack| u32::from(stack.count)).sum::<u32>(), 0);
            assert_eq!(server.server.players[slot].window_events.tally.unfit_returned, 1);
            hg.state.remote_client.as_mut().unwrap().send_item_action(&crate::protocol::ItemActionPacket { seq: 0, action: crate::protocol::ItemAction::GrantUnfit { event: grant_event, count: 1 }, events_applied: 0 });
            harness_step(&mut server, &mut hg);
            assert!(hg.state.remote_client.as_ref().unwrap().is_connected());
            assert!(server.server.players[slot].is_in_world());
            assert_eq!(floor(&server), 1, "actual repeated feedback cannot return the overflow twice");
            assert_eq!(server.server.players[slot].window_events.tally.unfit_returned, 1);
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_d2c2_local_pack_click_uses_live_cargo_without_snapshot_writeback() {
        isolate_saves();
        let mut hg = HeadlessGame::boot_into_world("harness-local-live-pack");
        let steed = crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Mule, hg.state.players[0].player.pos + glam::Vec3::new(0.0, 0.0, -2.0));
        {
            let mut data = hg.state.ecs.get::<&mut crate::horse_ai::HorseData>(steed).unwrap();
            data.keeper = Some(crate::horse_ai::SteedKeeper::LegacyLocal(0));
            data.pack = Some(crate::chest::ChestData::new());
        }
        hg.state.players[0].inventory = crate::inventory::Inventory::new();
        hg.state.players[0].inventory.set_slot(1, Some(crate::item::ItemStack::new_block(crate::block::STONE, 3)));
        // The current cargo changes after a rendering snapshot could have
        // been drawn: a click must leave that newly arrived stock intact.
        hg.state.ecs.get::<&mut crate::horse_ai::HorseData>(steed).unwrap().pack.as_mut().unwrap().slots[8] = Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Bread, 2));
        hg.state.apply_local_pack_clicks(0, steed, &[crate::container_window::ContainerClick::Deposit { slot: 1, all: true }]);
        assert!(hg.state.players[0].inventory.slot(1).is_none());
        assert_eq!(hg.state.ecs.get::<&crate::horse_ai::HorseData>(steed).unwrap().pack.as_ref().unwrap().slots[8].as_ref().unwrap().count, 2, "current unrelated stock survives the local click");
        hg.state.apply_local_pack_clicks(0, steed, &[crate::container_window::ContainerClick::TakeAll]);
        assert!(hg.state.ecs.get::<&crate::horse_ai::HorseData>(steed).unwrap().pack.as_ref().unwrap().slots.iter().all(Option::is_none));
        assert_eq!(hg.state.players[0].inventory.count_material(crate::item::MaterialId::Bread), 2);
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_d2c2_lent_hosts_live_pack_click_pushes_to_its_open_joiner() {
        use crate::transport::ClientTransport as _;
        use crate::{action_wire::{ActionIntent, ActionPacket, AppliedActionAck}, container_window::{ContainerClick, ContainerTarget}, pack_wire::{PackOperation, PackRequest, PackResult, PackViewPush}};
        isolate_saves();
        let name = "harness-lent-live-pack";
        let mut hg = HeadlessGame::boot_into_world(name);
        let seed = hg.state.biome_gen.seed;
        hg.state.hosted_server = Some(crate::hosted_server::HostedServer::start_host(1, name.to_string(), seed, 0, crate::hosted_server::RemoteTransport::WebSocket { port: 0 }, crate::hosted_server::HostWorld::Lent).unwrap());
        assert!(hg.state.sim_lent());
        let client = hg.state.hosted_server.as_mut().unwrap().attach_test_remote();
        client.send_to_server(&crate::protocol::serialize_packet(crate::protocol::PacketType::JoinRequest, &crate::remote_client::build_join_request_guest("Visitor", 0)));
        let mut accept = None;
        for _ in 0..100 {
            hg.hosted_ticks(1);
            while let Some(bytes) = client.try_recv_from_server() {
                if let Some((crate::protocol::PacketType::JoinAccept, payload)) = crate::protocol::deserialize_header(&bytes) { accept = Some(crate::protocol::safe_deserialize::<crate::protocol::JoinAcceptPacket>(payload).unwrap()); }
            }
            if accept.is_some() { break; }
        }
        let accept = accept.expect("the real lent join reached acceptance");
        clear_pad(&mut hg, None);
        let at = hg.state.players[0].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
        let steed = crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Mule, at);
        {
            let mut data = hg.state.ecs.get::<&mut crate::horse_ai::HorseData>(steed).unwrap();
            data.keeper = Some(crate::horse_ai::SteedKeeper::LegacyLocal(0));
            data.pack = Some(crate::chest::ChestData::new());
        }
        let slot = hg.state.hosted_server.as_ref().unwrap().server.players.len() - 1;
        // The lent authority chooses the guest's spawn independently of the
        // native host's saved position. Put this interaction on the same pad.
        let host_pos = hg.state.players[0].player.pos;
        let original = &hg.state.hosted_server.as_ref().unwrap().server.players[slot];
        eprintln!("lent pack setup: original guest {:?}, target {:?}, eye distance {}, reach {}, alive {}, actor {}", original.player.pos, at, (at + glam::Vec3::Y * 0.7 - original.player.eye_pos()).length(), crate::combat::ATTACK_REACH + crate::hosted_server::ATTACK_REACH_TOLERANCE, original.is_present_and_alive(), original.respawns);
        {
            let sp = &mut hg.state.hosted_server.as_mut().unwrap().server.players[slot];
            sp.player.pos = host_pos;
            sp.player.velocity = glam::Vec3::ZERO;
        }
        let eye = hg.state.hosted_server.as_ref().unwrap().server.players[slot].player.eye_pos();
        assert!((at + glam::Vec3::Y * 0.7 - eye).length() <= crate::combat::ATTACK_REACH + crate::hosted_server::ATTACK_REACH_TOLERANCE);
        let forward = (at + glam::Vec3::Y * 0.7 - eye).normalize();
        {
            let hs = hg.state.hosted_server.as_mut().unwrap();
            hs.server.players[slot].yaw = (-forward.x).atan2(-forward.z);
            hs.server.players[slot].pitch = forward.y.asin();
            let boundary = hs.server.players[slot].window_events.last_number();
            crate::window_events::apply_through(&mut hs.server.players[slot], boundary, hs.server.tick_counter);
        }
        for _ in 0..100 {
            hg.hosted_ticks(1);
            if hg.state.ecs.get::<&crate::entity::ProtocolId>(steed).is_ok() { break; }
        }
        let id = hg.state.ecs.get::<&crate::entity::ProtocolId>(steed).unwrap().0;
        let target = ContainerTarget::Entity(id);
        let hs = hg.state.hosted_server.as_ref().unwrap();
        let sp = &hs.server.players[slot];
        let raw = crate::window_events::pack_raw_marker(sp, &hg.state.registry).unwrap();
        let action = sp.action_frontier.as_ref().unwrap().header(1, sp.window_events.last_number());
        let request = PackRequest { actor_respawns: sp.respawns, target, hotbar_slot: 0, before: raw.digest_with(sp.station, None), operation: PackOperation::Open };
        client.send_to_server(&crate::protocol::serialize_packet(crate::protocol::PacketType::Action, &ActionPacket { action, intent: ActionIntent::Pack(request) }));
        let mut opened = None;
        for _ in 0..100 {
            hg.hosted_ticks(1);
            while let Some(bytes) = client.try_recv_from_server() {
                if let Some((crate::protocol::PacketType::PackResult, payload)) = crate::protocol::deserialize_header(&bytes) { opened = Some(PackResult::decode(payload).unwrap()); }
            }
            if opened.is_some() { break; }
        }
        let opened = opened.expect("real entity Open through the lent transport");
        assert!(opened.view.is_some(), "live lent Open refused with {:?}; guest {:?}, target {:?}", opened.note, hg.state.hosted_server.as_ref().unwrap().server.players[slot].player.pos, hg.state.ecs.get::<&crate::entity::Position>(steed).unwrap().0);
        client.send_to_server(&crate::protocol::serialize_packet(crate::protocol::PacketType::AppliedActionAck, &AppliedActionAck { session: accept.action_session, epoch: action.epoch, life: action.life, through: action.seq, included_events: opened.window_event }));
        hg.hosted_ticks(1);
        hg.state.players[0].inventory = crate::inventory::Inventory::new();
        hg.state.players[0].inventory.set_slot(1, Some(crate::item::ItemStack::new_block(crate::block::STONE, 3)));
        hg.state.apply_local_pack_clicks(0, steed, &[ContainerClick::Deposit { slot: 1, all: true }]);
        assert!(hg.state.players[0].inventory.slot(1).is_none(), "actual host window paid the live deposit");
        let mut changed = None;
        for _ in 0..100 {
            hg.hosted_ticks(1);
            while let Some(bytes) = client.try_recv_from_server() {
                if let Some((crate::protocol::PacketType::PackView, payload)) = crate::protocol::deserialize_header(&bytes) { changed = Some(crate::protocol::safe_deserialize::<PackViewPush>(payload).unwrap()); }
            }
            if changed.as_ref().is_some_and(|push| push.view.as_ref().is_some_and(|view| view.slots.as_slice()[0].as_ref().is_some_and(|stack| stack.count == 3))) { break; }
        }
        let changed = changed.expect("the open joiner received the host's changed live pack");
        assert!(changed.window_event > opened.window_event);
        assert_eq!(changed.view.unwrap().slots.as_slice()[0].as_ref().unwrap().count, 3);
        assert_eq!(hg.state.ecs.get::<&crate::horse_ai::HorseData>(steed).unwrap().pack.as_ref().unwrap().slots[0].as_ref().unwrap().count, 3);
    }

    #[test]
    #[ignore = "needs a GPU adapter holding the 506-layer atlas"]
    fn game_harness_d2c2_old_decoded_result_cannot_install_window_or_view_after_respawn() {
        use crate::{action_identity::InventoryMode, container_window::ContainerTarget, pack_wire::PackOperation};
        isolate_saves();
        for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
            let (mut hg, mut server, slot) = joined_window_client_mode(&format!("pack-old-life-{mode:?}"), mode);
            clear_pad(&mut hg, Some(&mut server));
            let at = server.server.players[slot].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
            let steed = crate::entity::spawn_mob(&mut server.server.ecs, crate::mob::MobType::Donkey, at);
            {
                let mut data = server.server.ecs.get::<&mut crate::horse_ai::HorseData>(steed).unwrap();
                data.keeper = Some(crate::horse_ai::SteedKeeper::LegacyLocal(0));
                data.pack = Some(crate::chest::ChestData::new());
            }
            pack_wait(&mut server, &mut hg, |server, hg| server.server.ecs.get::<&crate::entity::ProtocolId>(steed).is_ok_and(|key| hg.state.remote_mobs.position_of(key.0).is_some()));
            let id = server.server.ecs.get::<&crate::entity::ProtocolId>(steed).unwrap().0;
            aim_at(&mut hg, at + glam::Vec3::Y * 0.7);
            pack_wait(&mut server, &mut hg, |server, hg| (server.server.players[slot].yaw - hg.state.players[0].camera.yaw).abs() < 0.01);
            assert!(hg.state.request_pack_action(0, ContainerTarget::Entity(id), PackOperation::Open));
            // Decode from the real host, but deliberately delay GameState's
            // world/window intake across an actually-applied life boundary.
            let mut decoded = None;
            for _ in 0..100 {
                server.tick();
                let remote = hg.state.remote_client.as_mut().unwrap();
                remote.poll();
                if let Some(at) = remote.chunk_queue.iter().position(|(_, item)| matches!(item, crate::chunk_intake::StreamItem::PackResult(_))) {
                    let (_, crate::chunk_intake::StreamItem::PackResult(packet)) = remote.chunk_queue.remove(at) else { unreachable!() };
                    decoded = Some(packet);
                    break;
                }
            }
            let decoded = decoded.expect("a real decoded old-body Open result");
            assert!(decoded.view.is_some(), "the stale carrier could otherwise reinstall a view");
            let new_life = hg.state.remote_client.as_ref().unwrap().applied_actor_respawns() + 1;
            assert!(hg.state.remote_client.as_mut().unwrap().apply_actor_respawn(new_life));
            assert!(!hg.state.remote_client.as_mut().unwrap().apply_actor_respawn(new_life), "duplicate lifecycle delivery is inert");
            hg.state.players[0].inventory = crate::inventory::Inventory::new();
            hg.state.players[0].inventory.set_slot(4, Some(crate::item::ItemStack::new_block(crate::block::STONE, 7)));
            assert!(hg.state.remote_client.as_mut().unwrap().apply_pack_result_intake(&decoded).unwrap());
            hg.state.apply_pack_result(&decoded).unwrap();
            assert_eq!(hg.state.players[0].inventory.slot(4).unwrap().count, 7, "old canonical state cannot replace the current body's window");
            assert!(hg.state.players[0].shared_container.is_none(), "old Open cannot restore a stale view");
            hg.state.remote_client.as_mut().unwrap().pack_carrier_applied(decoded.owned_bytes());
            hg.state.remote_client.as_mut().unwrap().note_window_event(decoded.window_event);
            hg.frames(1); // Real world FIFO receipt intake retires the metadata.
            assert!(!hg.state.pack_window_frozen(0), "stale terminal metadata still retires its exact receipt");
        }
    }

}
