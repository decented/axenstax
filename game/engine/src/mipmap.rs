//! CPU mip-chain generation for the block texture array (Spec 39 A6, Spec 03 §9.4b).
//!
//! The block atlas is a `D2Array` texture with one layer per registered block
//! texture. Each layer is an independent res×res image, so each layer gets its
//! **own** mip chain — there is no cross-texture bleeding to guard against
//! (Spec 03 §3.5, "texture arrays" bullet).
//!
//! Generation is done on the CPU at upload time and pushed level-by-level with
//! `Queue::write_texture`. That is deliberately the simplest correct option and
//! is byte-identical on native and on wasm32/WebGPU — a GPU downsample pass
//! would need a render-attachment usage, a blit pipeline and per-level views on
//! both backends for no visible gain at 16–128 px per layer.
//!
//! ## Why alpha-weighted, and why linear space
//!
//! Block textures are cut-outs (leaves, glass panes, rails, plants): a plain
//! 2×2 RGBA average pulls the *colour* of fully-transparent texels (usually
//! black) into the neighbours, so distant leaves darken into soot. The 2×2
//! filter here weights colour by alpha, so transparent texels contribute
//! nothing to colour; alpha itself is a straight mean, and an all-transparent
//! quad stays fully transparent.
//!
//! The atlas format is `Rgba8UnormSrgb`, so the stored bytes are sRGB-encoded.
//! Averaging the encoded bytes darkens gradients; we decode to linear, average,
//! and re-encode.

/// Number of mip levels for a square texture of side `size`:
/// `floor(log2(size)) + 1` (16 → 5, 32 → 6, 128 → 8, 1 → 1).
pub fn mip_level_count_for(size: u32) -> u32 {
    if size == 0 {
        return 1;
    }
    32 - size.leading_zeros()
}

/// The level count the renderer actually allocates for the atlas: 1 (mip 0 only,
/// today's exact behaviour) when mipmaps are off, and 1 as a safety net for a
/// non-power-of-two atlas side (packs declare 16/32/64/128 — see
/// `texture_registry::VALID_RESOLUTIONS` — but a hand-made pack must never be
/// able to produce a half-sized level the halving filter can't express).
pub fn atlas_mip_levels(size: u32, enabled: bool) -> u32 {
    if !enabled || !size.is_power_of_two() {
        return 1;
    }
    mip_level_count_for(size)
}

/// sRGB byte → linear float in `0.0..=1.0` (IEC 61966-2-1).
fn srgb_to_linear(b: u8) -> f32 {
    let c = b as f32 / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear float → sRGB byte (inverse of [`srgb_to_linear`], round-to-nearest).
fn linear_to_srgb(v: f32) -> u8 {
    let c = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// Box-filter one RGBA level of side `size` down to `size / 2`.
///
/// Colour is averaged in linear space **weighted by alpha** (a transparent
/// texel neither darkens nor tints its neighbours); alpha is a straight mean of
/// the four bytes. A 2×2 quad that is fully transparent stays fully transparent
/// (`[0, 0, 0, 0]`) — those texels are alpha-cut in the shader, so their colour
/// is never seen.
///
/// Returns an empty vec if `size < 2` or `src` is not `size * size * 4` bytes.
pub fn downsample_rgba(src: &[u8], size: u32) -> Vec<u8> {
    if size < 2 || src.len() != (size as usize * size as usize * 4) {
        return Vec::new();
    }
    let half = size / 2;
    let row = size as usize * 4;
    let mut out = Vec::with_capacity(half as usize * half as usize * 4);
    for y in 0..half as usize {
        for x in 0..half as usize {
            let (mut lr, mut lg, mut lb) = (0.0f32, 0.0f32, 0.0f32);
            let mut alpha_weight = 0.0f32;
            let mut alpha_sum = 0u32;
            for dy in 0..2usize {
                for dx in 0..2usize {
                    let i = (y * 2 + dy) * row + (x * 2 + dx) * 4;
                    let a = src[i + 3];
                    let w = a as f32 / 255.0;
                    alpha_sum += a as u32;
                    alpha_weight += w;
                    lr += srgb_to_linear(src[i]) * w;
                    lg += srgb_to_linear(src[i + 1]) * w;
                    lb += srgb_to_linear(src[i + 2]) * w;
                }
            }
            // Round-to-nearest mean of the four alpha bytes.
            let a_out = ((alpha_sum + 2) / 4) as u8;
            if alpha_weight <= 0.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                out.push(linear_to_srgb(lr / alpha_weight));
                out.push(linear_to_srgb(lg / alpha_weight));
                out.push(linear_to_srgb(lb / alpha_weight));
                out.push(a_out);
            }
        }
    }
    out
}

/// Full mip chain for one square RGBA layer, **including level 0** at index 0.
/// `chain[l]` is the level-`l` image, of side `size >> l`, down to 1×1.
///
/// Returns just level 0 for a non-power-of-two side (see [`atlas_mip_levels`]).
pub fn mip_chain(level0: &[u8], size: u32) -> Vec<Vec<u8>> {
    let levels = atlas_mip_levels(size, true);
    let mut chain = Vec::with_capacity(levels as usize);
    chain.push(level0.to_vec());
    let mut cur = size;
    for _ in 1..levels {
        let next = downsample_rgba(chain.last().expect("chain is non-empty"), cur);
        if next.is_empty() {
            break;
        }
        chain.push(next);
        cur /= 2;
    }
    chain
}

/// Upload one atlas layer, mip level 0 first and then every generated level.
/// With `levels == 1` this is exactly the single `write_texture` the engine did
/// before mipmaps existed (no chain is built at all).
pub fn write_layer_with_mips(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    pixels: &[u8],
    size: u32,
    levels: u32,
) {
    let write = |level: u32, data: &[u8], side: u32| {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
        );
    };
    if levels <= 1 {
        write(0, pixels, size);
        return;
    }
    for (level, data) in mip_chain(pixels, size).iter().enumerate() {
        write(level as u32, data, size >> level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid RGBA layer of `size`×`size`.
    fn solid(size: u32, px: [u8; 4]) -> Vec<u8> {
        px.iter().copied().cycle().take(size as usize * size as usize * 4).collect()
    }

    #[test]
    fn level_counts_match_floor_log2_plus_one() {
        assert_eq!(mip_level_count_for(1), 1);
        assert_eq!(mip_level_count_for(2), 2);
        assert_eq!(mip_level_count_for(16), 5);
        assert_eq!(mip_level_count_for(32), 6);
        assert_eq!(mip_level_count_for(64), 7);
        assert_eq!(mip_level_count_for(128), 8);
    }

    #[test]
    fn atlas_levels_are_one_when_disabled_or_not_power_of_two() {
        assert_eq!(atlas_mip_levels(16, false), 1, "dial off ⇒ today's mip_level_count: 1");
        assert_eq!(atlas_mip_levels(48, true), 1, "non-power-of-two atlas ⇒ no chain");
        assert_eq!(atlas_mip_levels(16, true), 5);
        assert_eq!(atlas_mip_levels(128, true), 8);
    }

    #[test]
    fn srgb_round_trips_for_every_byte() {
        for b in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(b)), b, "byte {b} failed round trip");
        }
    }

    #[test]
    fn opaque_2x2_averages_to_one_texel() {
        // Four identical opaque texels average to themselves.
        let src = solid(2, [120, 60, 200, 255]);
        assert_eq!(downsample_rgba(&src, 2), vec![120, 60, 200, 255]);
    }

    #[test]
    fn one_transparent_texel_does_not_darken_the_colour() {
        // Three opaque light-grey texels + one transparent BLACK texel. A naive
        // average would drag the colour down to ~150; the alpha-weighted filter
        // must keep the colour and only lower alpha.
        let mut src = solid(2, [200, 200, 200, 255]);
        src[12..16].copy_from_slice(&[0, 0, 0, 0]);
        let out = downsample_rgba(&src, 2);
        assert_eq!(&out[0..3], &[200, 200, 200], "transparent texel must not tint colour");
        assert_eq!(out[3], 191, "alpha is the plain mean: (255*3)/4 = 191.25 → 191");
    }

    #[test]
    fn all_transparent_2x2_stays_transparent() {
        let src = solid(2, [17, 34, 51, 0]);
        assert_eq!(downsample_rgba(&src, 2), vec![0, 0, 0, 0]);
    }

    #[test]
    fn downsample_rejects_bad_input() {
        assert!(downsample_rgba(&solid(1, [1, 2, 3, 4]), 1).is_empty());
        assert!(downsample_rgba(&[0, 0, 0, 0], 4).is_empty(), "byte length must match size");
    }

    #[test]
    fn chain_from_16px_has_the_expected_level_sizes() {
        let level0 = solid(16, [80, 90, 100, 255]);
        let chain = mip_chain(&level0, 16);
        assert_eq!(chain.len(), 5);
        for (l, data) in chain.iter().enumerate() {
            let side = 16u32 >> l;
            assert_eq!(
                data.len(),
                (side * side * 4) as usize,
                "level {l} should be {side}×{side}"
            );
        }
        // Uniform input ⇒ every level is that same colour, 1×1 included.
        assert_eq!(chain[4], vec![80, 90, 100, 255]);
    }

    #[test]
    fn chain_1x1_level_is_the_alpha_weighted_mean() {
        // Half the 16×16 layer is opaque red, half is transparent black. The
        // 1×1 level must be red at half alpha — not a half-darkened red.
        let mut level0 = solid(16, [255, 0, 0, 255]);
        for y in 8..16usize {
            for x in 0..16usize {
                let i = (y * 16 + x) * 4;
                level0[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
        let chain = mip_chain(&level0, 16);
        let last = chain.last().expect("chain has levels");
        assert_eq!(last.len(), 4);
        assert_eq!(&last[0..3], &[255, 0, 0], "colour must survive the transparent half");
        assert_eq!(last[3], 128, "alpha halves each way: 255/2 → 128");
    }

    #[test]
    fn chain_is_level_zero_only_for_a_non_power_of_two_layer() {
        let level0 = solid(3, [1, 2, 3, 255]);
        let chain = mip_chain(&level0, 3);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0], level0);
    }
}
