//! How items look on the GPU (ignored: needs a GPU adapter holding the atlas).
//!
//! One stage on a marble pad: a torch in an item frame on a stone pillar (shot
//! from the front, the side and above, each also with the frame empty), a
//! torch and a stone block held in the hand (first person mid-swing, when the
//! hand comes into view, and third person from behind, each also empty-handed),
//! a dropped torch and a dropped stone block on the ground, a blighted crop
//! next to a mature wheat crop at noon and by torchlight at midnight, and a
//! placed torch at noon and midnight (it must look the same before and after
//! an item change).
//!
//! Each measurement is logged as `ITEMDIAG ...`: near-black pixels, and the
//! pixels that changed against the item-free shot (the item's own pixels, and
//! how many of those are near-black). The checks run last, so a failing run
//! still writes every shot and number first:
//! - the framed torch shows (changed pixels from the front and the side), and
//!   seen from the side it is not a dark box (few near-black pixels where the
//!   framed item sits);
//! - the torch held in the hand, seen from behind in third person, shows and
//!   has no near-black pixels (not a dark cube);
//! - the blighted crop is grey: its pixels are far less saturated than the
//!   healthy wheat's, at noon and by torchlight.
//!
//! Run: ITEM_SHOTS_OUT=<dir> cargo test --lib -- --ignored --test-threads=1 --nocapture game_harness_item_looks
use super::torch_diag::{ground_y, place, pose_eye, set_time, settle};
use super::*;
use crate::block;
use crate::item::ItemStack;

/// A pixel counts as near-black when its brightest sRGB channel is below this.
const NEAR_BLACK: u8 = 40;
/// A pixel differs between two shots when the sum of its three sRGB channel
/// differences is above this.
const CHANGED: i32 = 36;

type Img = image::RgbaImage;
type Rect = [u32; 4];

/// Yaw and pitch that look from `eye` at `target` (yaw 0 looks along -Z).
fn aim(eye: [f32; 3], target: [f32; 3]) -> (f32, f32) {
    let d = glam::Vec3::from_array(target) - glam::Vec3::from_array(eye);
    let h = (d.x * d.x + d.z * d.z).sqrt();
    ((-d.x).atan2(-d.z), d.y.atan2(h))
}

/// Pose the eye at `eye` looking at `target`, let a few frames upload the
/// view, and save the shot as `<out>/<name>.png`. Returns the image.
fn shot(hg: &mut HeadlessGame, out: &str, name: &str, eye: [f32; 3], target: [f32; 3]) -> Img {
    let (yaw, pitch) = aim(eye, target);
    pose_eye(hg, eye, yaw, pitch);
    hg.frames(3);
    snap(hg, out, name, eye)
}

/// Save the current view (already posed at `eye`) as `<out>/<name>.png`.
fn snap(hg: &mut HeadlessGame, out: &str, name: &str, eye: [f32; 3]) -> Img {
    let cam = hg.state.players[0].camera.position;
    assert!(
        (cam - glam::Vec3::from_array(eye)).length() < 1e-3,
        "{name}: the camera is at {cam}, not the posed eye {eye:?}"
    );
    let path = format!("{out}/{name}.png");
    for _ in 0..2 {
        hg.state.renderer.render_view_to_png(&path, 0, |_, _| {});
    }
    eprintln!("ITEMDIAG shot {path}");
    image::open(&path).unwrap_or_else(|e| panic!("{path}: {e}")).to_rgba8()
}

/// The screen rectangle `[x0, y0, x1, y1)` covering a world-space box, through
/// the view the last shot was drawn from.
fn screen_rect(hg: &HeadlessGame, min: [f32; 3], max: [f32; 3]) -> Rect {
    let vp = hg.state.renderer.player_gpu[0].last_view_proj;
    let (w, h) = (hg.state.renderer.width as f32, hg.state.renderer.height as f32);
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for i in 0..8 {
        let p = [
            if i & 1 == 0 { min[0] } else { max[0] },
            if i & 2 == 0 { min[1] } else { max[1] },
            if i & 4 == 0 { min[2] } else { max[2] },
        ];
        let c = vp * glam::Vec4::new(p[0], p[1], p[2], 1.0);
        assert!(c.w > 0.0, "box corner {p:?} is behind the camera");
        let (sx, sy) = ((c.x / c.w + 1.0) * 0.5 * w, (1.0 - c.y / c.w) * 0.5 * h);
        r = [r[0].min(sx), r[1].min(sy), r[2].max(sx), r[3].max(sy)];
    }
    [
        r[0].floor().clamp(0.0, w) as u32,
        r[1].floor().clamp(0.0, h) as u32,
        r[2].ceil().clamp(0.0, w) as u32,
        r[3].ceil().clamp(0.0, h) as u32,
    ]
}

/// Every pixel of `rect` except the crosshair (a small cross at the centre).
fn pixels(img: &Img, rect: Rect) -> impl Iterator<Item = (u32, u32)> + '_ {
    let (cx, cy) = (img.width() / 2, img.height() / 2);
    (rect[1]..rect[3])
        .flat_map(move |y| (rect[0]..rect[2]).map(move |x| (x, y)))
        .filter(move |&(x, y)| !(x.abs_diff(cx) <= 12 && y.abs_diff(cy) <= 12))
}

/// Near-black pixels in `rect`, and the rect's area (crosshair left out).
fn near_black(img: &Img, rect: Rect) -> (u32, u32) {
    let (mut dark, mut area) = (0, 0);
    for (x, y) in pixels(img, rect) {
        area += 1;
        let p = img.get_pixel(x, y).0;
        if p[0].max(p[1]).max(p[2]) < NEAR_BLACK {
            dark += 1;
        }
    }
    (dark, area)
}

/// Pixels in `rect` that differ between `a` and `b`.
fn changed(a: &Img, b: &Img, rect: Rect) -> u32 {
    pixels(a, rect).filter(|&(x, y)| differs(a.get_pixel(x, y).0, b.get_pixel(x, y).0)).count() as u32
}

fn differs(a: [u8; 4], b: [u8; 4]) -> bool {
    (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).sum::<i32>() > CHANGED
}

/// Pixels in `rect` that differ between `base` and `img` and are near-black
/// in `img`: the dark part of whatever appeared.
fn dark_changed(base: &Img, img: &Img, rect: Rect) -> u32 {
    pixels(img, rect)
        .filter(|&(x, y)| {
            let p = img.get_pixel(x, y).0;
            differs(base.get_pixel(x, y).0, p) && p[0].max(p[1]).max(p[2]) < NEAR_BLACK
        })
        .count() as u32
}

/// HSV saturation and value of an sRGB pixel, each 0..=1.
fn hsv_sv(p: [u8; 4]) -> (f32, f32) {
    let max = p[0].max(p[1]).max(p[2]) as f32 / 255.0;
    let min = p[0].min(p[1]).min(p[2]) as f32 / 255.0;
    (if max > 0.0 { (max - min) / max } else { 0.0 }, max)
}

/// The plant's own pixels in `rect` (those that differ from the plant-free
/// shot `base`): count, mean HSV saturation and value, mean sRGB.
fn plant_stats(base: &Img, img: &Img, rect: Rect) -> (u32, f32, f32, [f32; 3]) {
    let (mut n, mut s, mut v, mut rgb) = (0u32, 0.0f32, 0.0f32, [0.0f32; 3]);
    for (x, y) in pixels(img, rect) {
        let b = img.get_pixel(x, y).0;
        if !differs(base.get_pixel(x, y).0, b) {
            continue;
        }
        let (ps, pv) = hsv_sv(b);
        n += 1;
        s += ps;
        v += pv;
        for c in 0..3 {
            rgb[c] += b[c] as f32;
        }
    }
    let k = n.max(1) as f32;
    (n, s / k, v / k, [rgb[0] / k, rgb[1] / k, rgb[2] / k])
}

/// Hold `stack` in hotbar slot 0 (selected), with the rest of the hotbar empty.
fn hold(hg: &mut HeadlessGame, stack: Option<ItemStack>) {
    let p = &mut hg.state.players[0];
    for i in 0..9 {
        p.inventory.set_slot(i, None);
    }
    p.inventory.set_slot(0, stack);
    p.hotbar_slot = 0;
}

/// Measure the crops shot `img` against its plant-free `base`; returns the
/// mean saturation of (wheat, blight).
fn crop_saturation(hg: &HeadlessGame, label: &str, base: &Img, img: &Img, crops: [(i32, i32, i32); 2]) -> [f32; 2] {
    let mut sat = [0.0f32; 2];
    for (i, (kind, at)) in [("wheat", crops[0]), ("blight", crops[1])].into_iter().enumerate() {
        let (x, y, z) = (at.0 as f32, at.1 as f32, at.2 as f32);
        let rect = screen_rect(hg, [x - 0.1, y, z - 0.1], [x + 1.1, y + 1.0, z + 1.1]);
        let (n, s, v, rgb) = plant_stats(base, img, rect);
        eprintln!(
            "ITEMDIAG {label}-{kind} rect={rect:?} plant_px={n} mean_S={s:.3} mean_V={v:.3} mean_rgb=[{:.0}, {:.0}, {:.0}]",
            rgb[0], rgb[1], rgb[2]
        );
        assert!(n > 50, "{label}: the {kind} crop shows on screen ({n} plant pixels)");
        sat[i] = s;
    }
    sat
}

#[test]
#[ignore = "diagnostic + regression; needs a GPU adapter holding the atlas"]
fn game_harness_item_looks() {
    isolate_saves();
    let out = std::env::var("ITEM_SHOTS_OUT").unwrap_or_else(|_| {
        std::env::temp_dir().join(format!("axenstax-item-shots-{}", std::process::id())).display().to_string()
    });
    std::fs::create_dir_all(&out).unwrap();
    let mut hg = HeadlessGame::boot_into_world("harness-item-looks");

    // Fly in Creative so posing sticks; empty hand; let the spawn area stream.
    hg.state.set_play_mode(crate::play_mode::PlayMode::Creative);
    hg.state.players[0].player.flying = true;
    hold(&mut hg, None);
    settle(&mut hg);
    let p = hg.state.players[0].player.pos;
    let (tx, tz) = (p.x.floor() as i32, p.z.floor() as i32);
    let top = p.y.floor() as i32 + 12;

    // The stage: a 17x17 marble pad, everything cleared 9 high above it.
    let gy = ground_y(&hg, tx, tz, top);
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
    let (fx, fy, fz) = (tx as f32, ty as f32, tz as f32);
    eprintln!("ITEMDIAG stage centre ({tx},{ty},{tz})");

    // A two-high stone pillar with an empty item frame on its south face. The
    // frame cell is (tx, ty+1, tz+1); its plate lies against z = tz+1.
    place(&mut hg, tx, ty, tz, block::STONE);
    place(&mut hg, tx, ty + 1, tz, block::STONE);
    let frame_at = (tx, ty + 1, tz + 1);
    place(&mut hg, frame_at.0, frame_at.1, frame_at.2, block::ITEM_FRAME);
    hg.state.world.set_meta(frame_at, crate::meta::with_facing(0, crate::meta::Facing::South));
    hg.state.world.insert_item_frame(frame_at, crate::item_frame::ItemFrameData::new());
    hg.state.rebuild_chunks_for_lighting(frame_at.0, frame_at.1, frame_at.2);

    // A placed torch, well away from everything else.
    let placed = (tx - 6, ty, tz - 6);
    place(&mut hg, placed.0, placed.1, placed.2, block::TORCH);

    // Tilled soil for the two crops (the plants go in after a plant-free shot).
    let (wheat_at, blight_at) = ((tx + 3, ty, tz - 3), (tx + 5, ty, tz - 3));
    place(&mut hg, wheat_at.0, gy, wheat_at.2, block::TILLED_SOIL);
    place(&mut hg, blight_at.0, gy, blight_at.2, block::TILLED_SOIL);
    settle(&mut hg);
    set_time(&mut hg, 12000);

    // ── Framed torch: front, side, above; empty frame first ─────────────────
    // The region measured is the framed item's space: the old item cube (x/y
    // 0.25..0.75 of the cell, 0.0625..0.25 off the plate) with a margin.
    let (ox, oy, oz) = (frame_at.0 as f32, frame_at.1 as f32, frame_at.2 as f32);
    let item_min = [ox + 0.2, oy + 0.2, oz + 0.0625];
    let item_max = [ox + 0.8, oy + 0.8, oz + 0.3];
    let item_centre = [ox + 0.5, oy + 0.5, oz + 0.16];
    let views = [
        ("front", [ox + 0.5, fy + 1.7, oz + 1.5]),
        ("side", [ox + 1.8, fy + 1.7, oz + 0.16]),
        ("above", [ox + 0.5, fy + 2.7, oz + 0.4]),
    ];
    let empties: Vec<Img> =
        views.iter().map(|(v, eye)| shot(&mut hg, &out, &format!("frame-{v}-empty"), *eye, item_centre)).collect();
    let mut frame = crate::item_frame::ItemFrameData::new();
    assert!(frame.try_insert(ItemStack::new_block(block::TORCH, 1)), "an empty frame takes a torch");
    hg.state.world.insert_item_frame(frame_at, frame);
    hg.state.rebuild_chunks_for_lighting(frame_at.0, frame_at.1, frame_at.2);
    settle(&mut hg);
    set_time(&mut hg, 12000);
    let (mut side_frac, mut side_shown, mut front_shown) = (1.0f32, 0u32, 0u32);
    let (mut held_3p_shown, mut held_3p_dark) = (0u32, u32::MAX);
    for ((v, eye), empty) in views.iter().zip(&empties) {
        let name = format!("frame-{v}");
        let img = shot(&mut hg, &out, &name, *eye, item_centre);
        let rect = screen_rect(&hg, item_min, item_max);
        let (dark, area) = near_black(&img, rect);
        let frac = dark as f32 / area.max(1) as f32;
        let shown = changed(empty, &img, rect);
        eprintln!(
            "ITEMDIAG {name} rect={rect:?} near_black={dark} area={area} frac={frac:.4} changed_vs_empty={shown} dark_changed={}",
            dark_changed(empty, &img, rect)
        );
        match *v {
            "side" => (side_frac, side_shown) = (frac, shown),
            "front" => front_shown = shown,
            _ => {}
        }
    }

    // ── Placed torch at noon and midnight (must not change) ─────────────────
    let torch_eye = [fx - 5.5, fy + 1.7, fz - 6.5 + 1.8];
    let torch_mid = [fx - 5.5, fy + 0.3, fz - 5.5];
    for (label, t) in [("day", 12000u32), ("midnight", 0u32)] {
        set_time(&mut hg, t);
        shot(&mut hg, &out, &format!("placed-torch-{label}"), torch_eye, torch_mid);
        let (x, y, z) = (placed.0 as f32, placed.1 as f32, placed.2 as f32);
        let rect = screen_rect(&hg, [x + 0.3, y, z + 0.3], [x + 0.7, y + 0.7, z + 0.7]);
        eprintln!("ITEMDIAG placed-torch-{label} rect={rect:?}");
    }
    set_time(&mut hg, 12000);

    // ── Held: empty hand, a torch, a stone block, over the bright pad ───────
    // First person: at rest the hand end of the arm sits below the screen, so
    // each shot is taken mid-swing (the local swing window part-way down,
    // `SWING_TICKS` 6), when the held item comes into view. Third person: the
    // orbit camera behind the player, who looks along +Z over the pad.
    let held_eye = [fx + 5.5, fy + 1.7, fz + 5.5];
    let held_target = [fx + 5.5, fy, fz + 3.5];
    let back_eye = [fx + 5.5, fy + 1.7, fz + 1.5];
    for view in ["held", "held-3p"] {
        let mut empty: Option<Img> = None;
        for (what, stack) in [
            ("empty", None),
            ("torch", Some(ItemStack::new_block(block::TORCH, 1))),
            ("stone", Some(ItemStack::new_block(block::STONE, 1))),
        ] {
            let name = format!("{view}-{what}");
            hold(&mut hg, stack);
            let img = if view == "held" {
                let (yaw, pitch) = aim(held_eye, held_target);
                pose_eye(&mut hg, held_eye, yaw, pitch);
                hg.frames(2);
                hg.state.local_swing.insert(0, 4);
                hg.frames(1);
                snap(&mut hg, &out, &name, held_eye)
            } else {
                hg.state.players[0].camera.mode = crate::camera::CameraMode::OrbitBehind;
                pose_eye(&mut hg, back_eye, std::f32::consts::PI, -0.2);
                hg.frames(3);
                let path = format!("{out}/{name}.png");
                for _ in 0..2 {
                    hg.state.renderer.render_view_to_png(&path, 0, |_, _| {});
                }
                eprintln!("ITEMDIAG shot {path}");
                image::open(&path).unwrap().to_rgba8()
            };
            let whole = [0, 0, img.width(), img.height()];
            let (dark, _) = near_black(&img, whole);
            let (shown, dark_shown) =
                empty.as_ref().map_or((0, 0), |e| (changed(e, &img, whole), dark_changed(e, &img, whole)));
            eprintln!("ITEMDIAG {name} near_black={dark} changed_vs_empty={shown} dark_changed={dark_shown}");
            if name == "held-3p-torch" {
                (held_3p_shown, held_3p_dark) = (shown, dark_shown);
            }
            if empty.is_none() {
                empty = Some(img);
            }
        }
    }
    hg.state.players[0].camera.mode = crate::camera::CameraMode::FirstPerson;
    hold(&mut hg, None);

    // ── Dropped: a torch and a stone block on the pad ───────────────────────
    let drop_eye = [fx - 2.5, fy + 1.7, fz + 3.5 + 2.4];
    let drop_target = [fx - 2.5, fy + 0.25, fz + 3.5];
    let drops_base = shot(&mut hg, &out, "drops-base", drop_eye, drop_target);
    let torch_drop = glam::Vec3::new(fx - 3.5, fy + 0.3, fz + 3.5);
    let stone_drop = glam::Vec3::new(fx - 1.5, fy + 0.3, fz + 3.5);
    crate::entity::spawn_item(&mut hg.state.ecs, torch_drop, ItemStack::new_block(block::TORCH, 1), 0);
    crate::entity::spawn_item(&mut hg.state.ecs, stone_drop, ItemStack::new_block(block::STONE, 1), 0);
    // Put each just above its spot with no hop, and let it land: 3 s of ticks.
    let ours = |item: &crate::item::Item| match item {
        crate::item::Item::Block(id) if *id == block::TORCH => Some(torch_drop),
        crate::item::Item::Block(id) if *id == block::STONE => Some(stone_drop),
        _ => None,
    };
    for (_, (pos, vel, it)) in hg
        .state
        .ecs
        .query::<(&mut crate::entity::Position, &mut crate::entity::Velocity, &crate::entity::ItemEntity)>()
        .iter()
    {
        if let Some(at) = ours(&it.stack.item) {
            pos.0 = at;
            vel.0 = glam::Vec3::ZERO;
        }
    }
    hg.ticks(60);
    let drops: Vec<(crate::item::Item, glam::Vec3)> = hg
        .state
        .ecs
        .query::<(&crate::entity::Position, &crate::entity::ItemEntity)>()
        .iter()
        .filter(|(_, (_, it))| ours(&it.stack.item).is_some())
        .map(|(_, (pos, it))| (it.stack.item.clone(), pos.0))
        .collect();
    eprintln!("ITEMDIAG drops {drops:?}");
    // A drop spins with `falling_tick_counter` (0.045 rad a tick). Pin it so
    // the drops turn about 0.3 rad from facing the camera (keeping it rising).
    let c0 = hg.state.falling_tick_counter;
    hg.state.falling_tick_counter = (c0..c0 + 200)
        .min_by(|a, b| {
            let off = |c: &u32| ((*c as f32 / 20.0 * 0.9).rem_euclid(std::f32::consts::TAU) - 0.3).abs();
            off(a).total_cmp(&off(b))
        })
        .unwrap();
    let (yaw, pitch) = aim(drop_eye, drop_target);
    pose_eye(&mut hg, drop_eye, yaw, pitch);
    hg.frames(1);
    eprintln!("ITEMDIAG drops spin counter={} (was {c0})", hg.state.falling_tick_counter);
    let img = snap(&mut hg, &out, "drops", drop_eye);
    for (label, id) in [("torch", block::TORCH), ("stone", block::STONE)] {
        let pos = drops
            .iter()
            .find(|d| d.0 == crate::item::Item::Block(id))
            .map(|d| d.1)
            .unwrap_or_else(|| panic!("the dropped {label} is in the world"));
        let rect = screen_rect(&hg, [pos.x - 0.3, pos.y, pos.z - 0.3], [pos.x + 0.3, pos.y + 0.6, pos.z + 0.3]);
        let (dark, area) = near_black(&img, rect);
        eprintln!(
            "ITEMDIAG drop-{label} at={pos} rect={rect:?} near_black={dark} area={area} frac={:.4} changed_vs_base={} dark_changed={}",
            dark as f32 / area.max(1) as f32,
            changed(&drops_base, &img, rect),
            dark_changed(&drops_base, &img, rect),
        );
    }

    // ── Blight: a blighted crop beside a mature wheat crop ──────────────────
    let crop_eye = [fx + 4.5, fy + 1.7, fz - 2.5 + 2.0];
    let crop_target = [fx + 4.5, fy + 0.3, fz - 2.5];
    let crops = [wheat_at, blight_at];
    let plant = |hg: &mut HeadlessGame, on: bool| {
        let (w, b) = if on { (block::WHEAT_STAGE_3, block::BLIGHTED_CROP) } else { (block::AIR, block::AIR) };
        place(hg, wheat_at.0, wheat_at.1, wheat_at.2, w);
        place(hg, blight_at.0, blight_at.1, blight_at.2, b);
        settle(hg);
    };
    // At noon.
    let base = shot(&mut hg, &out, "crops-noon-base", crop_eye, crop_target);
    plant(&mut hg, true);
    set_time(&mut hg, 12000);
    let img = shot(&mut hg, &out, "crops-noon", crop_eye, crop_target);
    let noon = crop_saturation(&hg, "crops-noon", &base, &img, crops);
    // At midnight, lit by a torch standing between them.
    place(&mut hg, tx + 4, ty, tz - 3, block::TORCH);
    plant(&mut hg, false);
    set_time(&mut hg, 0);
    let base = shot(&mut hg, &out, "crops-torch-base", crop_eye, crop_target);
    plant(&mut hg, true);
    set_time(&mut hg, 0);
    let img = shot(&mut hg, &out, "crops-torch", crop_eye, crop_target);
    let w = &hg.state.world;
    eprintln!(
        "ITEMDIAG crops-torch block light wheat={} blight={}",
        w.block_light_at(wheat_at.0, wheat_at.1, wheat_at.2),
        w.block_light_at(blight_at.0, blight_at.1, blight_at.2),
    );
    let torchlit = crop_saturation(&hg, "crops-torch", &base, &img, crops);

    // ── The checks ──────────────────────────────────────────────────────────
    assert!(front_shown >= 200, "the framed torch shows from the front ({front_shown} changed pixels)");
    assert!(side_shown >= 20, "the framed torch shows from the side ({side_shown} changed pixels)");
    assert!(
        side_frac < 0.10,
        "the framed torch seen from the side is a dark box: {:.1}% of its space is near-black",
        side_frac * 100.0
    );
    assert!(
        held_3p_shown >= 600 && held_3p_dark == 0,
        "the held torch seen from behind: {held_3p_shown} changed pixels, {held_3p_dark} of them near-black"
    );
    for (label, [wheat_s, blight_s]) in [("noon", noon), ("torchlight", torchlit)] {
        assert!(
            blight_s < 0.2 && blight_s < 0.5 * wheat_s,
            "{label}: the blighted crop is not grey: saturation {blight_s:.3} against the wheat's {wheat_s:.3}"
        );
    }
}
