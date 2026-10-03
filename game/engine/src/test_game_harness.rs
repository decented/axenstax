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
}
