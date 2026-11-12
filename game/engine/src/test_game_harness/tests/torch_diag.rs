//! Torch light diagnostic (ignored: needs a GPU adapter holding the atlas).
//! Places a torch (and a salt lamp + paper lantern for comparison) in a fresh
//! world through the real relight + remesh path, then screenshots day/night;
//! last, closes a stone box round the torch (no sky light) and shoots it at
//! noon and midnight, with and without the torch. Each shot also logs the
//! screen position of named world points (`TORCHDIAG probe ...`) so a probe can
//! sample the same surface before and after a change.
//! Run: TORCH_SHOTS_OUT=<dir> cargo test --lib -- --ignored game_harness_torch_diag --nocapture
use super::*;
use crate::block;

fn ground_y(hg: &HeadlessGame, x: i32, z: i32, from: i32) -> i32 {
    let reg = &hg.state.registry;
    let mut y = from;
    while y > 0 {
        let b = hg.state.world.get_block(x, y, z);
        if b != block::AIR && reg.is_solid(b) {
            return y;
        }
        y -= 1;
    }
    panic!("no ground at {x},{z}");
}

fn place(hg: &mut HeadlessGame, x: i32, y: i32, z: i32, id: block::BlockId) {
    let prev = hg.state.world.get_block(x, y, z);
    hg.state.world.set_block(x, y, z, id);
    hg.state.world.set_placed(x, y, z, true);
    crate::lighting::update_for_block_change(&mut hg.state.world, (x, y, z), prev, id, &hg.state.registry);
    hg.state.rebuild_chunks_for_lighting(x, y, z);
}

fn settle(hg: &mut HeadlessGame) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        hg.frames(3);
        if hg.state.dirty_mesh_chunks.is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "meshes never settled");
    }
    hg.frames(3);
}

/// Stand so the EYE is at `eye`, looking along yaw/pitch.
fn pose_eye(hg: &mut HeadlessGame, eye: [f32; 3], yaw: f32, pitch: f32) {
    let p = &mut hg.state.players[0];
    p.player.pos = glam::Vec3::new(eye[0], eye[1] - 1.62, eye[2]);
    p.player.velocity = glam::Vec3::ZERO;
    p.player.flying = true;
    p.camera.yaw = yaw;
    p.camera.pitch = pitch;
}

fn shot(hg: &mut HeadlessGame, name: &str, points: &[(&str, [f32; 3])]) {
    let Ok(dir) = std::env::var("TORCH_SHOTS_OUT") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let path = format!("{dir}/{name}.png");
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(&path, 0, |_, _| {});
    }
    eprintln!("TORCHDIAG shot {path}");
    // Project each named world point with the view the shot was drawn from.
    let vp = hg.state.renderer.player_gpu[0].last_view_proj;
    let (w, h) = (hg.state.renderer.width as f32, hg.state.renderer.height as f32);
    for (label, p) in points {
        let c = vp * glam::Vec4::new(p[0], p[1], p[2], 1.0);
        if c.w <= 0.0 {
            continue;
        }
        let (sx, sy) = ((c.x / c.w + 1.0) * 0.5 * w, (1.0 - c.y / c.w) * 0.5 * h);
        let cell = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
        eprintln!(
            "TORCHDIAG probe {name} {label} px={:.0} py={:.0} bl={} sl={}",
            sx,
            sy,
            hg.state.world.block_light_at(cell.0, cell.1, cell.2),
            hg.state.world.sky_light_at(cell.0, cell.1, cell.2),
        );
    }
}

/// Mean linear-space RGB of a 16x16 texture layer, printed so a probe can
/// express a pixel as a fraction of its texture's own colour.
fn texture_mean(tex: &[Vec<u8>], layer: u32, label: &str) {
    let t = &tex[layer as usize];
    let lin = |c: u8| {
        let s = c as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    let mut sum = [0.0f32; 3];
    for px in t.chunks_exact(4) {
        for c in 0..3 {
            sum[c] += lin(px[c]);
        }
    }
    let n = (t.len() / 4) as f32;
    eprintln!(
        "TORCHDIAG texmean {label} layer={layer} linear=[{:.4}, {:.4}, {:.4}]",
        sum[0] / n,
        sum[1] / n,
        sum[2] / n
    );
}

fn set_time(hg: &mut HeadlessGame, t: u32) {
    hg.state.write_clock(t, 0);
    hg.state.lightning_flash = 0.0;
    hg.frames(4);
    let (_, b) = crate::camera::compute_sun(hg.state.world.effective_world_time(hg.state.world_time));
    eprintln!("TORCHDIAG time={} sun_brightness={b:.3}", hg.state.world_time);
}

#[test]
#[ignore = "diagnostic; needs a GPU adapter holding the atlas"]
fn game_harness_torch_diag() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-torch-diag");
    hg.state.set_play_mode(crate::play_mode::PlayMode::Survival);

    // What the torch faces sample: the small cube's UVs run [0,dx]x[0,dy]
    // (dx=0.2, dy=0.6) => texels u 0..3, v 0..9 of the 16x16 torch layer.
    let tex = crate::texture_gen::generate_textures();
    let torch = &tex[block::TEX_TORCH as usize];
    let avg = |u0: usize, u1: usize, v0: usize, v1: usize| {
        let (mut s, mut n) = ([0u32; 3], 0u32);
        for v in v0..v1 {
            for u in u0..u1 {
                let i = (v * 16 + u) * 4;
                for c in 0..3 { s[c] += torch[i + c] as u32; }
                n += 1;
            }
        }
        [s[0] / n, s[1] / n, s[2] / n]
    };
    eprintln!("TORCHDIAG torch layer {} whole-texture avg {:?}", block::TEX_TORCH, avg(0, 16, 0, 16));
    eprintln!("TORCHDIAG side-face sampled region u0..4 v0..10 avg {:?}", avg(0, 4, 0, 10));
    eprintln!("TORCHDIAG top-face sampled region u0..4 v0..4 avg {:?}", avg(0, 4, 0, 4));
    eprintln!("TORCHDIAG flame+handle region u6..10 v2..16 avg {:?}", avg(6, 10, 2, 16));
    eprintln!("TORCHDIAG texture_count {}", crate::texture_gen::texture_count());
    texture_mean(&tex, hg.state.registry.tex_top(block::MARBLE), "marble");
    texture_mean(&tex, hg.state.registry.tex_side(block::STONE), "stone");

    // Fly in Creative so posing sticks; let the spawn area stream + mesh first.
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    hg.state.players[0].player.flying = true;
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (px, pz) = (p.x.floor() as i32, p.z.floor() as i32);
    let top = p.y.floor() as i32 + 12;

    // One stage: a 17x17 pad of MARBLE (pale, uniform) at the ground under the
    // spawn, everything cleared 9 high above it, emitter at the pad centre.
    let (tx, tz) = (px, pz);
    let gy = ground_y(&hg, tx, tz, top);
    eprintln!("TORCHDIAG stage ground y={gy} block={}", hg.state.world.get_block(tx, gy, tz));
    for dx in -8..=8 {
        for dz in -8..=8 {
            let (x, z) = (tx + dx, tz + dz);
            if hg.state.world.get_block(x, gy, z) != block::MARBLE {
                place(&mut hg, x, gy, z, block::MARBLE);
            }
            for dy in 1..=9 {
                if hg.state.world.get_block(x, gy + dy, z) != block::AIR {
                    place(&mut hg, x, gy + dy, z, block::AIR);
                }
            }
        }
    }
    let ty = gy + 1;
    settle(&mut hg);

    // Cameras, all inside the cleared volume.
    // Close-up: eye 1.8 south of the emitter, 1.0 above its base.
    let close = [tx as f32 + 0.5, ty as f32 + 1.0, tz as f32 + 0.5 + 1.8];
    let close_pitch = -(0.7f32 / 1.8).atan();
    // Wide: eye 6 south, 3.5 up — shows the light pool on the pad.
    let wide = [tx as f32 + 0.5, ty as f32 + 3.5, tz as f32 + 0.5 + 6.0];
    let wide_pitch = -(3.4f32 / 6.0).atan();
    // Named surfaces each shot reports the screen position of (top faces sit
    // at gy + 1; the torch pole spans x/z 0.4..0.6 and y 0..0.6 of its cell).
    let (fx, fy, fz) = (tx as f32, ty as f32, tz as f32);
    let pad_y = gy as f32 + 1.0;
    let points: Vec<(&str, [f32; 3])> = vec![
        ("pad-under-emitter", [fx + 0.15, pad_y, fz + 0.85]),
        ("pad-south-1", [fx + 0.5, pad_y, fz + 1.5]),
        ("pad-east-1", [fx + 1.5, pad_y, fz + 0.5]),
        ("pad-east-3", [fx + 3.5, pad_y, fz + 0.5]),
        ("pad-east-6", [fx + 6.5, pad_y, fz + 0.5]),
        ("pad-far-sw", [fx - 6.5, pad_y, fz + 3.5]),
        ("torch-flame", [fx + 0.5, fy + 0.5, fz + 0.6]),
        ("torch-handle", [fx + 0.5, fy + 0.15, fz + 0.6]),
    ];

    let stages: [(&str, block::BlockId); 4] = [
        ("none", block::AIR),
        ("torch", block::TORCH),
        ("saltlamp", block::SALT_LAMP),
        ("lantern", block::PAPER_LANTERN_WHITE),
    ];
    for (name, id) in stages {
        let cur = hg.state.world.get_block(tx, ty, tz);
        if cur != id {
            if cur != block::AIR {
                place(&mut hg, tx, ty, tz, block::AIR);
            }
            if id != block::AIR {
                place(&mut hg, tx, ty, tz, id);
            }
        }
        settle(&mut hg);
        let w = &hg.state.world;
        let ring: Vec<_> = (0..=15).map(|d| w.block_light_at(tx, ty, tz + d)).collect();
        eprintln!(
            "TORCHDIAG stage={name} id={id} emission={} cell bl={} sl={} | bl along +z (air row): {ring:?} | sky above pad={}",
            hg.state.registry.light_emission(id),
            w.block_light_at(tx, ty, tz),
            w.sky_light_at(tx, ty, tz),
            w.sky_light_at(tx, ty, tz + 3),
        );
        for (label, t) in [("day", 12000u32), ("night", 19000u32), ("midnight", 0u32)] {
            set_time(&mut hg, t);
            if name != "none" {
                pose_eye(&mut hg, close, 0.0, close_pitch);
                hg.frames(3);
                shot(&mut hg, &format!("{name}-close-{label}"), &points);
            }
            pose_eye(&mut hg, wide, 0.0, wide_pitch);
            hg.frames(3);
            shot(&mut hg, &format!("{name}-wide-{label}"), &points);
        }
    }
    eprintln!("TORCHDIAG stage centre ({tx},{ty},{tz})");

    // Enclosed: a closed stone box on the pad round the emitter cell (interior
    // x/z -2..=2, y ty..=ty+4; the marble pad is its floor), so no sky light
    // reaches inside. Shot from inside at noon and midnight, empty, then with
    // the torch. Runs after the open-sky shots so it cannot shade them.
    if hg.state.world.get_block(tx, ty, tz) != block::AIR {
        place(&mut hg, tx, ty, tz, block::AIR);
    }
    for dx in -3..=3i32 {
        for dz in -3..=3i32 {
            for dy in 0..=5 {
                let shell = dx.abs() == 3 || dz.abs() == 3 || dy == 5;
                if shell && hg.state.world.get_block(tx + dx, ty + dy, tz + dz) != block::STONE {
                    place(&mut hg, tx + dx, ty + dy, tz + dz, block::STONE);
                }
            }
        }
    }
    settle(&mut hg);
    let inside = [fx + 0.5, fy + 2.5, fz + 2.5];
    let inside_pitch = -(2.2f32 / 2.0).atan();
    let box_points: Vec<(&str, [f32; 3])> = vec![
        ("pad-under-emitter", [fx + 0.15, pad_y, fz + 0.85]),
        ("pad-south-1", [fx + 0.5, pad_y, fz + 1.5]),
        ("pad-east-1", [fx + 1.5, pad_y, fz + 0.5]),
        ("pad-west-2", [fx - 1.5, pad_y, fz - 1.5]),
        ("wall-north", [fx + 0.5, fy + 1.5, fz - 2.0]),
        ("wall-west", [fx - 2.0, fy + 1.0, fz + 0.5]),
        ("torch-flame", [fx + 0.5, fy + 0.5, fz + 0.6]),
        ("torch-handle", [fx + 0.5, fy + 0.15, fz + 0.6]),
    ];
    for (name, id) in [("none", block::AIR), ("torch", block::TORCH)] {
        if id != block::AIR {
            place(&mut hg, tx, ty, tz, id);
        }
        settle(&mut hg);
        let w = &hg.state.world;
        let sky_inside: Vec<_> = (-2..=2)
            .flat_map(|dx| (-2..=2).map(move |dz| (dx, dz)))
            .map(|(dx, dz)| w.sky_light_at(tx + dx, ty + 2, tz + dz))
            .collect();
        eprintln!(
            "TORCHDIAG enclosed={name} max sky light inside={} emitter bl={}",
            sky_inside.iter().max().copied().unwrap_or(0),
            w.block_light_at(tx, ty, tz),
        );
        for (label, t) in [("day", 12000u32), ("midnight", 0u32)] {
            set_time(&mut hg, t);
            pose_eye(&mut hg, inside, 0.0, inside_pitch);
            hg.frames(3);
            shot(&mut hg, &format!("enclosed-{name}-{label}"), &box_points);
        }
    }
}
