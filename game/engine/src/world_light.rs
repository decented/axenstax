//! One light rule for every lit world surface (Spec 03 §4.7).
//!
//! The CPU twin of `shader.wgsl`'s `sky_shade`, `sky_term`, `face_shade`,
//! `block_term` and `world_light`, kept here so the rule is tested. A surface
//! carries two light channels, block light (torches, lava, lamps) and sky
//! light, each 0..=1. They light it in two different ways:
//!
//! - **Sky term.** Sky light, scaled by the day's brightness (`sun.w`), times
//!   the sun-and-ambient shade of the face. Night darkens it.
//! - **Block term.** Block light in a warm colour times a fixed per-face shade
//!   (top 1.0, bottom 0.5, north/south 0.8, east/west 0.6, Minecraft's), with
//!   no time input at all. A torch lights a room the same at noon and midnight.
//!
//! The surface takes the per-channel maximum of the two. Before 2026-11-12 the
//! two channels were merged first (`max(block, sky * sun.w)`) and the result
//! multiplied by the sun-and-ambient shade, so torchlight was scaled by the sun:
//! at midnight a stone floor next to a torch showed about a fifth of its colour.
//!
//! Avatars use the same rule. Their `light` channel is the fade alpha, so the
//! block level rides in bits 20..23 of `tex_layer` ([`avatar_block_bits`]) and
//! `sky_light` carries sky already scaled by the day's brightness.

// The shader is the live caller of the light rule; Rust calls only the avatar
// packing helpers, the rest is here for its tests.
#![cfg_attr(not(test), allow(dead_code))]

use glam::Vec3;

/// The shade's ambient part at night (`sun.w` = 0).
pub const SKY_AMBIENT_NIGHT: [f32; 3] = [0.12, 0.13, 0.18];
/// What the ambient part gains per unit of `sun.w`.
pub const SKY_AMBIENT_DAY: [f32; 3] = [0.28, 0.29, 0.32];
/// The sun's colour at full brightness.
pub const SUN_COLOUR: [f32; 3] = [1.0, 0.98, 0.9];
/// How much the direct sun adds on a face turned to it.
pub const SUN_WEIGHT: f32 = 0.6;
/// The lowest sky level a face shows (a sealed cave is not pitch black).
pub const LIGHT_FLOOR: f32 = 0.08;
/// Block light's warm colour.
pub const BLOCK_LIGHT_COLOUR: [f32; 3] = [1.0, 0.9, 0.7];
/// Fixed per-face shade of block light: top, bottom, north/south (Z), east/west (X).
pub const FACE_SHADE_TOP: f32 = 1.0;
pub const FACE_SHADE_BOTTOM: f32 = 0.5;
pub const FACE_SHADE_Z: f32 = 0.8;
pub const FACE_SHADE_X: f32 = 0.6;

/// Avatar vertices: the skin layer is the low 16 bits of `tex_layer`...
pub const AVATAR_LAYER_MASK: u32 = 0xFFFF;
/// ...and the block-light level (0..=15) sits at bits 20..23.
pub const AVATAR_BLOCK_SHIFT: u32 = 20;
pub const AVATAR_BLOCK_MASK: u32 = 0xF;
/// The four block-level bits in place, for clearing them.
pub const AVATAR_BLOCK_BITS: u32 = AVATAR_BLOCK_MASK << AVATAR_BLOCK_SHIFT;

fn v3(c: [f32; 3]) -> Vec3 {
    Vec3::from_array(c)
}

/// The sun-and-ambient shade of a face with normal `n` (`shader.wgsl` `sky_shade`).
/// `sun_dir` need not be normalised; `brightness` is `sun.w`.
pub fn sky_shade(n: Vec3, sun_dir: Vec3, brightness: f32) -> Vec3 {
    let ambient = v3(SKY_AMBIENT_NIGHT) + v3(SKY_AMBIENT_DAY) * brightness;
    let sun = v3(SUN_COLOUR) * brightness;
    let ndotl = n.dot(sun_dir.normalize()).max(0.0);
    ambient + sun * ndotl * SUN_WEIGHT
}

/// The sky term for a sky level already scaled by the day's brightness.
pub fn sky_term(n: Vec3, sky_level: f32, sun_dir: Vec3, brightness: f32) -> Vec3 {
    sky_shade(n, sun_dir, brightness) * sky_level.clamp(LIGHT_FLOOR, 1.0)
}

/// Fixed per-face shade of block light. Axis normals give the table values;
/// any other normal mixes them by the squares of its components (they sum to
/// one for a unit normal), and a zero normal gives 1.0.
pub fn face_shade(n: Vec3) -> f32 {
    let n2 = n * n;
    let len2 = n2.x + n2.y + n2.z;
    let y_shade = if n.y > 0.0 { FACE_SHADE_TOP } else { FACE_SHADE_BOTTOM };
    let weighted = n2.x * FACE_SHADE_X + n2.y * y_shade + n2.z * FACE_SHADE_Z;
    if len2 > 1e-6 { weighted / len2.max(1e-6) } else { 1.0 }
}

/// The block term: warm block light times the face's fixed shade. No time input.
pub fn block_term(n: Vec3, block_level: f32) -> Vec3 {
    v3(BLOCK_LIGHT_COLOUR) * (block_level.min(1.0) * face_shade(n))
}

/// The one light rule: what a face with normal `n` and channels
/// `(block_level, sky_level)` multiplies its texture colour by.
pub fn world_light(n: Vec3, block_level: f32, sky_level: f32, sun_dir: Vec3, brightness: f32) -> Vec3 {
    sky_term(n, sky_level * brightness, sun_dir, brightness).max(block_term(n, block_level))
}

/// The rule before 2026-11-12, kept for the tests that pin the change: the
/// channels merged first, then the merged value scaled by the sun-and-ambient shade.
pub fn legacy_light(n: Vec3, block_level: f32, sky_level: f32, sun_dir: Vec3, brightness: f32) -> Vec3 {
    let merged = block_level.max(sky_level * brightness);
    sky_shade(n, sun_dir, brightness) * merged.clamp(LIGHT_FLOOR, 1.0)
}

/// The bits an avatar vertex carries for a block-light level of `block_level`
/// (0..=1): the level rounded to 0..=15, shifted to bits 20..23.
pub fn avatar_block_bits(block_level: f32) -> u32 {
    let level = (block_level * 15.0).round().clamp(0.0, 15.0) as u32;
    (level & AVATAR_BLOCK_MASK) << AVATAR_BLOCK_SHIFT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::compute_sun;

    const NOON: u32 = 12000;
    const DUSK: u32 = 19000;
    const MIDNIGHT: u32 = 0;

    fn normals() -> [Vec3; 6] {
        [Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z, Vec3::X, Vec3::NEG_X]
    }

    fn luma(c: Vec3) -> f32 {
        0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z
    }

    #[test]
    fn block_light_is_the_same_at_noon_and_midnight() {
        let level = 14.0 / 15.0;
        let (noon_dir, noon_b) = compute_sun(NOON);
        let (night_dir, night_b) = compute_sun(MIDNIGHT);
        for n in normals() {
            let noon = world_light(n, level, 0.0, noon_dir, noon_b);
            let night = world_light(n, level, 0.0, night_dir, night_b);
            assert_eq!(noon, night, "normal {n}: torchlight must not change with the time of day");
            assert_eq!(noon, block_term(n, level), "normal {n}: under cover the block term decides");
            // The old rule scaled the same torchlight by the sun.
            let old_noon = legacy_light(n, level, 0.0, noon_dir, noon_b);
            let old_night = legacy_light(n, level, 0.0, night_dir, night_b);
            assert!(luma(old_night) < luma(old_noon) * 0.5, "normal {n}: the old rule dimmed torchlight at night");
        }
    }

    #[test]
    fn faces_keep_minecraft_shade_order_under_block_light() {
        let level = 14.0 / 15.0;
        let top = luma(block_term(Vec3::Y, level));
        let z = luma(block_term(Vec3::Z, level));
        let x = luma(block_term(Vec3::X, level));
        let bottom = luma(block_term(Vec3::NEG_Y, level));
        assert!(top > z && z > x && x > bottom, "top {top} > z {z} > x {x} > bottom {bottom}");
        assert_eq!(face_shade(Vec3::ZERO), 1.0, "a zero normal is unshaded");
        // A non-axis unit normal mixes by the squared components.
        let d = Vec3::new(1.0, 1.0, 0.0).normalize();
        assert!((face_shade(d) - 0.5 * (FACE_SHADE_X + FACE_SHADE_TOP)).abs() < 1e-6);
        // An unnormalised normal shades the same as its unit direction.
        assert!((face_shade(Vec3::new(0.0, 0.0, 3.0)) - FACE_SHADE_Z).abs() < 1e-6);
    }

    #[test]
    fn without_block_light_nothing_changes() {
        for t in [NOON, DUSK, MIDNIGHT] {
            let (dir, b) = compute_sun(t);
            for n in normals() {
                for sky in [0.0, 7.0 / 15.0, 1.0] {
                    let new = world_light(n, 0.0, sky, dir, b);
                    let old = legacy_light(n, 0.0, sky, dir, b);
                    assert!(
                        (new - old).abs().max_element() < 1e-6,
                        "time {t} normal {n} sky {sky}: {new} vs legacy {old}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_floor_by_a_torch_shows_most_of_its_colour_at_midnight() {
        let emission = crate::block::BlockRegistry::new().light_emission(crate::block::TORCH);
        // The air cell beside the torch holds one level less than it emits.
        let level = f32::from(emission - 1) / 15.0;
        for t in [MIDNIGHT, NOON] {
            let (dir, b) = compute_sun(t);
            // Under cover (no sky) and in the open.
            for sky in [0.0, 1.0] {
                let floor_top = world_light(Vec3::Y, level, sky, dir, b);
                assert!(luma(floor_top) >= 0.7, "time {t} sky {sky}: luma {} of the texture", luma(floor_top));
            }
        }
    }

    #[test]
    fn avatar_block_bits_round_trip_the_level() {
        for level in 0..=15u32 {
            let bits = avatar_block_bits(level as f32 / 15.0);
            assert_eq!(bits & !AVATAR_BLOCK_BITS, 0, "only bits 20..23 are set");
            assert_eq!((bits >> AVATAR_BLOCK_SHIFT) & AVATAR_BLOCK_MASK, level);
        }
        assert_eq!(avatar_block_bits(2.0), 15 << AVATAR_BLOCK_SHIFT, "above 1 clamps to 15");
        assert_eq!(avatar_block_bits(-1.0), 0, "below 0 clamps to 0");
        // The skin layer (low 16 bits) is untouched by the level.
        let packed = 3 | avatar_block_bits(1.0);
        assert_eq!(packed & AVATAR_LAYER_MASK, 3);
    }

    #[test]
    fn shader_carries_the_same_constants() {
        let src = include_str!("shader.wgsl");
        let vec3 = |c: [f32; 3]| format!("vec3<f32>({:?}, {:?}, {:?})", c[0], c[1], c[2]);
        let expect = [
            format!("const SKY_AMBIENT_NIGHT: vec3<f32> = {};", vec3(SKY_AMBIENT_NIGHT)),
            format!("const SKY_AMBIENT_DAY: vec3<f32> = {};", vec3(SKY_AMBIENT_DAY)),
            format!("const SUN_COLOUR: vec3<f32> = {};", vec3(SUN_COLOUR)),
            format!("const SUN_WEIGHT: f32 = {SUN_WEIGHT:?};"),
            format!("const LIGHT_FLOOR: f32 = {LIGHT_FLOOR:?};"),
            format!("const BLOCK_LIGHT_COLOUR: vec3<f32> = {};", vec3(BLOCK_LIGHT_COLOUR)),
            format!("const FACE_SHADE_TOP: f32 = {FACE_SHADE_TOP:?};"),
            format!("const FACE_SHADE_BOTTOM: f32 = {FACE_SHADE_BOTTOM:?};"),
            format!("const FACE_SHADE_Z: f32 = {FACE_SHADE_Z:?};"),
            format!("const FACE_SHADE_X: f32 = {FACE_SHADE_X:?};"),
            format!("const AVATAR_LAYER_MASK: u32 = 0x{AVATAR_LAYER_MASK:X}u;"),
            format!("const AVATAR_BLOCK_SHIFT: u32 = {AVATAR_BLOCK_SHIFT}u;"),
            format!("const AVATAR_BLOCK_MASK: u32 = 0x{AVATAR_BLOCK_MASK:X}u;"),
        ];
        for line in &expect {
            assert!(src.contains(line.as_str()), "shader.wgsl is missing `{line}`");
        }
        // No fragment function may keep the old merged-channel rule.
        for old in [
            "max(in.light, in.sky_light * camera.sun_dir.w)",
            "max(in.light, in.sky_light * brightness)",
            "max(in.light, 0.08)",
        ] {
            assert!(!src.contains(old), "shader.wgsl still merges the channels the old way: `{old}`");
        }
        // Every fragment function that lights a world surface goes through the rule.
        assert!(src.matches("world_light(in.normal, in.light, in.sky_light)").count() >= 4);
    }
}
