//! The Trials island (§C.3) through the real game: launched the way the
//! gallery's "Walk the island" launches it, built as its columns load, a
//! doorway walked into (the card opens, stepping out closes it), its trial
//! launched with Play, finished, and "Back to Trials" landing back at that
//! doorway; "Back to menu" from a trial goes to the lobby. With
//! `ISLAND_SHOTS_OUT=<dir>` it also writes the island's screenshots there.
use super::*;
use crate::trials_island_build as build;
use crate::trials_island_layout::{ring, GROUND_Y, ISLAND_R};

/// Queue a lobby Play of `def` and pump frames until its world is in play.
fn play_from_lobby(hg: &mut HeadlessGame, def: crate::scenario::ScenarioDef) {
    hg.state.pending_menu_action =
        Some((crate::menu::MenuAction::PlayScenario { def: Box::new(def) }, web_time::Instant::now()));
    pump_until_playing(hg);
}

fn pump_until_playing(hg: &mut HeadlessGame) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        hg.state.update_and_render();
        if matches!(hg.state.mode, GameMode::Playing) && hg.state.pending_menu_action.is_none() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "never reached Playing");
    }
}

/// Pump until the island visit has built every column and placed the player.
fn settle_island(hg: &mut HeadlessGame) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        hg.frames(5);
        let done = hg.state.island.as_ref().is_some_and(|v| {
            v.pending.is_empty() && v.arrival == crate::trials_island_flow::Arrival::Done
        });
        if done && hg.state.dirty_mesh_chunks.is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the island never finished building");
    }
}

/// Every layout cell in the island world matches the layout.
fn mismatches(hg: &HeadlessGame) -> usize {
    let v = hg.state.island.as_ref().expect("on the island");
    let cs = crate::chunk::CHUNK_SIZE as i32;
    crate::trials_island_layout::IslandLayout::columns(cs)
        .into_iter()
        .flat_map(|(cx, cz)| v.layout.column_cells(cx, cz, cs))
        .filter(|((x, y, z), b)| hg.state.world.get_block(*x, *y, *z) != *b)
        .count()
}

/// Stand player 0 at `pos` looking along `yaw`/`pitch` (flying, so it stays).
fn pose(hg: &mut HeadlessGame, pos: [f32; 3], yaw: f32, pitch: f32) {
    let p = &mut hg.state.players[0];
    p.player.pos = glam::Vec3::from(pos);
    p.player.velocity = glam::Vec3::ZERO;
    p.player.flying = true;
    p.camera.yaw = yaw;
    p.camera.pitch = pitch;
}

fn shot(hg: &mut HeadlessGame, name: &str, ui: impl Fn(&egui::Context)) {
    let Ok(dir) = std::env::var("ISLAND_SHOTS_OUT") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let path = format!("{dir}/{name}.png");
    // Twice: egui settles its sizing pass on the first.
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(&path, 0, |ctx, _| ui(ctx));
    }
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_trials_island_doorway_launches_a_trial_and_comes_back_to_it() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-island-home");
    hg.state.graphics.controls_card_seen = true;
    // A fresh player who has kept one Mine postcard.
    hg.state.trial_bests = crate::trials::TrialBests::default();
    hg.state.trial_bests.cards_opened.insert("sprint".to_string());
    let rgba = crate::postcard::fixture_rgba("mine", 0);
    assert!(crate::postcard::save("mine", &rgba, 1_000).is_some());
    assert!(hg.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));

    // Walk the island (the gallery's button sends this very action).
    play_from_lobby(&mut hg, crate::scenario::trials_island_def());
    assert_eq!(hg.state.world_name, build::island_folder());
    assert!(hg.state.world.has_flat_floor(), "the island is a flat world");
    settle_island(&mut hg);
    assert!(hg.state.scenario.is_none(), "no scenario runs on the island");
    assert_eq!(mismatches(&hg), 0, "the island is built as laid out");
    assert_eq!(hg.state.world.get_block(0, GROUND_Y, 0), crate::block::MARBLE, "the plaza has a floor");
    let p = hg.state.players[0].player.pos;
    assert!(ring(p.x.floor() as i32, p.z.floor() as i32) <= 1, "a visit starts on the plaza: {p:?}");
    let island = hg.state.island.as_ref().unwrap();
    let sprint = island.layout.doorway("sprint").expect("the clock corner has the sprint doorway").clone();
    let mine = island.layout.doorway("mine").expect("the front shelf has Mine").clone();
    assert!(island.layout.doorway("dye").is_none(), "a locked trial has no doorway");
    let near = |s: &&crate::trials_island_layout::SignSpot| {
        let d = |i: usize, j: usize| (s.pos[i] as f32 + 0.5 - mine.return_pos[j]).abs();
        d(0, 0) + d(2, 2)
    };
    let mine_sign = island.layout.signs.iter().min_by(|a, b| near(a).total_cmp(&near(b))).cloned();
    assert!(hg.state.world.exhibits.iter().any(|e| e.image_ref == build::exhibit_ref("mine")), "Mine's postcard hangs");
    assert!(!hg.state.world.exhibits.iter().any(|e| e.image_ref == build::exhibit_ref("craft")));

    // Nothing in the built area can be broken: aiming at it protects it.
    assert!(hg.state.island_protects(Some([mine.open_min[0], GROUND_Y, mine.open_min[1]])));
    assert!(!hg.state.island_protects(Some([ISLAND_R + 3, GROUND_Y, 0])));

    // Screenshots: the island from above, a doorway, and its open card.
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    pose(&mut hg, [34.0, 122.0, 34.0], std::f32::consts::FRAC_PI_4, -0.78);
    hg.frames(3);
    shot(&mut hg, "island-overview", |_| {});
    let back = [mine.return_pos[0], mine.return_pos[1] + 2.5, mine.return_pos[2] + 7.0];
    pose(&mut hg, back, mine.return_yaw + 0.2, -0.22);
    hg.frames(3);
    shot(&mut hg, "island-doorway", |ctx| {
        if let Some(s) = &mine_sign {
            let vp = crate::screen::ViewportRect { x: 0, y: 0, width: 1280, height: 720 };
            crate::hud_ui::draw_sign_text(ctx, &vp, &s.text);
        }
    });
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    hg.state.players[0].player.flying = false;

    // Walk into the Mine doorway: the card opens; step out: it closes.
    pose(&mut hg, mine.return_pos, mine.return_yaw, 0.0);
    hg.ticks(1);
    assert!(!hg.state.island_card_open(), "standing in front opens nothing");
    let inside = [mine.open_min[0] as f32 + 0.5, mine.return_pos[1], mine.open_min[1] as f32 + 0.5];
    let inside = [(inside[0] + mine.open_max[0] as f32 + 0.5) / 2.0, inside[1], (inside[2] + mine.open_max[1] as f32 + 0.5) / 2.0];
    pose(&mut hg, inside, mine.return_yaw, 0.0);
    hg.state.players[0].player.flying = false;
    hg.ticks(1);
    assert!(hg.state.island_card_open(), "walking into the doorway opens its card");
    assert!(hg.state.cursor_must_be_free(), "the card's Play can be clicked");
    if std::env::var("ISLAND_SHOTS_OUT").is_ok() {
        let textures = crate::texture_gen::generate_textures();
        let r = &mut hg.state.renderer;
        r.egui.upload_block_textures(&r.device, &r.queue, &textures, 16);
    }
    hg.frames(2);
    assert!(hg.state.island_card_open(), "the card stays open while standing in the doorway");
    {
        let st = &mut hg.state;
        let open = st.island.as_ref().unwrap().card.as_ref().unwrap();
        let (card, tex) = (&open.card, open.postcard.as_ref().map(|t| t.id()));
        if let Ok(dir) = std::env::var("ISLAND_SHOTS_OUT") {
            let path = format!("{dir}/island-hero-card.png");
            for _ in 0..2 {
                st.renderer.render_view_to_png(&path, 0, |ctx, _| {
                    crate::trials_island_ui::draw_card(ctx, card, tex);
                });
            }
        }
    }
    pose(&mut hg, mine.return_pos, mine.return_yaw, 0.0);
    hg.state.players[0].player.flying = false;
    hg.ticks(1);
    assert!(!hg.state.island_card_open(), "stepping out closes the card");

    // The sprint doorway: walk in, press Play.
    let into = [sprint.open_min[0] as f32 + 0.5, sprint.return_pos[1], sprint.open_min[1] as f32 + 0.5];
    pose(&mut hg, sprint.return_pos, sprint.return_yaw, 0.0);
    hg.state.players[0].player.flying = false;
    hg.ticks(1);
    pose(&mut hg, into, sprint.return_yaw, 0.0);
    hg.state.players[0].player.flying = false;
    hg.ticks(1);
    assert!(hg.state.island_card_open(), "the sprint card opens");
    hg.state.island_play("sprint");
    assert!(hg.state.island.is_none(), "Play leaves the island");
    pump_until_playing(&mut hg);
    assert_eq!(hg.state.world_name, "exp-the-sprint", "Play launched the sprint arena");
    assert!(hg.state.island_return.is_some(), "the trial knows its way back");
    hg.ticks(2);
    let finish = hg.state.active_trial.as_ref().expect("the race is armed").finish;
    hg.state.players[0].player.pos = glam::Vec3::from(finish);
    hg.ticks(2);
    assert!(hg.state.trial_outcome.is_some(), "the race is finished");

    // Back to Trials: to the island, at the sprint doorway.
    hg.state.back_to_trials();
    pump_until_playing(&mut hg);
    assert_eq!(hg.state.world_name, build::island_folder(), "back on the island");
    settle_island(&mut hg);
    let p = hg.state.players[0].player.pos;
    let want = glam::Vec3::from(sprint.return_pos);
    assert!(p.distance(want) < 0.6, "back at the sprint doorway: {p:?}, want {want:?}");
    assert!(!hg.state.island_card_open(), "arriving by the doorway opens nothing");
    assert_eq!(mismatches(&hg), 0, "the island is rebuilt unchanged");
    hg.ticks(200);
    assert_eq!(mismatches(&hg), 0, "nothing on the island flows or burns");

    // A trial left by "Back to menu" (the pause menu's Save & Quit) goes to the lobby.
    pose(&mut hg, sprint.return_pos, sprint.return_yaw, 0.0);
    hg.state.players[0].player.flying = false;
    hg.ticks(1);
    pose(&mut hg, into, sprint.return_yaw, 0.0);
    hg.state.players[0].player.flying = false;
    hg.ticks(1);
    hg.state.island_play("sprint");
    pump_until_playing(&mut hg);
    assert!(hg.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    assert!(hg.state.island_return.is_none(), "Back to menu forgets the island");
    hg.frames(5);
    assert!(matches!(hg.state.mode, GameMode::Menu(_)), "and stays in the lobby");
}
