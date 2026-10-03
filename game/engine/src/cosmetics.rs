//! Player cosmetic descriptor (F1). The avatar's appearance is read from this,
//! not hardcoded — so future cosmetics (3D model, cape, emotes) are additive
//! fields, not a renderer rewrite. Round one carries only the skin.
//!
//! INVARIANT (F2): a CosmeticDescriptor drives VISUALS ONLY. It must never feed
//! the collision hitbox / eye-height (those are fixed consts in physics.rs).

/// Where a player's skin pixels come from.
#[derive(Clone, Debug, PartialEq)]
pub enum SkinSource {
    /// The bundled original default skin (texture_gen::default_skin_rgba).
    Default,
    /// A 64x64 RGBA skin supplied at runtime (Phase 2 upload). 64*64*4 bytes.
    Rgba64(Vec<u8>),
}

/// Everything that determines how a player avatar looks.
#[derive(Clone, Debug, PartialEq)]
pub struct CosmeticDescriptor {
    /// Bumped when fields are added so older clients detect unknown versions.
    pub version: u8,
    pub skin: SkinSource,
    /// Classic (4-px arms) or Slim ("Alex", 3-px arms). Visual only, like the
    /// rest of this struct — it changes the mesh and the UV rects, never the
    /// collision hitbox (F2 invariant above).
    ///
    /// NOT on the wire: multiplayer sends only `skin_key`, so a remote player's
    /// arm model is unknown and every remote avatar is drawn Classic. That is
    /// the same limitation that already leaves remote players without custom
    /// skins at all, and it moves when the skin does.
    pub arm_model: crate::skin_uv::ArmModel,
}

impl Default for CosmeticDescriptor {
    fn default() -> Self {
        Self {
            version: 1,
            skin: SkinSource::Default,
            arm_model: crate::skin_uv::ArmModel::Classic,
        }
    }
}

impl CosmeticDescriptor {
    /// True when this avatar uses the bundled default skin. No production
    /// caller — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_default_skin(&self) -> bool {
        matches!(self.skin, SkinSource::Default)
    }
}

/// Decode uploaded image bytes (PNG) into a 64×64 RGBA8 buffer (16384 bytes).
/// Rejects anything that isn't exactly 64×64 with a kid-readable message. Used
/// both for the upload path and when restoring a stored skin.
pub fn decode_skin_64(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes)
        .map_err(|_| "That file isn't a picture we can read — try a PNG.".to_string())?;
    let rgba = img.to_rgba8();
    if rgba.width() != 64 || rgba.height() != 64 {
        return Err(format!(
            "Your skin must be 64×64 pixels (this one is {}×{}).",
            rgba.width(),
            rgba.height()
        ));
    }
    Ok(rgba.into_raw())
}

/// Encode a 64×64 RGBA buffer (16384 bytes) as a standard Minecraft skin PNG.
/// Mirrors the encoder used in this module's tests. This is the export path —
/// the bytes that get downloaded and dropped straight into the Minecraft
/// launcher. Errors (kid-readable) only when the buffer isn't 64×64.
pub fn encode_skin_png(rgba: &[u8]) -> Result<Vec<u8>, String> {
    if rgba.len() != 64 * 64 * 4 {
        return Err("That skin isn't the right size to save.".to_string());
    }
    let img = image::RgbaImage::from_raw(64, 64, rgba.to_vec())
        .ok_or_else(|| "Couldn't build the skin image.".to_string())?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("Couldn't save the skin: {e}"))?;
    Ok(png)
}

/// Decode an imported/exported skin PNG to 16384 RGBA bytes. Accepts the modern
/// 64×64 sheet (returns it as-is) and the legacy 64×32 sheet (expands it to
/// 64×64 — old pre-1.8 skins; common enough that rejecting them feels broken,
/// spec §8.4). Returns `(rgba, was_legacy)`. Any other size → kid-readable Err.
pub fn decode_skin_any(bytes: &[u8]) -> Result<(Vec<u8>, bool), String> {
    let img = image::load_from_memory(bytes)
        .map_err(|_| "That file isn't a picture we can read — try a PNG.".to_string())?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    match (w, h) {
        (64, 64) => Ok((rgba.into_raw(), false)),
        (64, 32) => Ok((crate::mc_import::expand_legacy_skin(&rgba.into_raw()), true)),
        _ => Err(format!(
            "Your skin must be 64×64 pixels (this one is {w}×{h})."
        )),
    }
}

impl CosmeticDescriptor {
    /// The 64×64 RGBA pixels this descriptor renders as: the bundled default,
    /// or the player's uploaded skin. Always returns 64*64*4 = 16384 bytes.
    pub fn skin_rgba(&self) -> Vec<u8> {
        match &self.skin {
            SkinSource::Default => crate::texture_gen::default_skin_rgba(),
            SkinSource::Rgba64(px) => px.clone(),
        }
    }

    /// A stable 64-bit content hash of this skin, for matching/diffing a player's
    /// look on the wire. Default = 0 (sentinel); a custom skin hashes its bytes
    /// (FNV-1a), forced non-zero so 0 always means "default".
    pub fn skin_key(&self) -> u64 {
        match &self.skin {
            SkinSource::Default => 0,
            SkinSource::Rgba64(px) => {
                let mut h: u64 = 0xcbf29ce484222325;
                for &b in px {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
                if h == 0 { 1 } else { h }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_version_1_default_skin() {
        let d = CosmeticDescriptor::default();
        assert_eq!(d.version, 1);
        assert!(d.is_default_skin());
        assert_eq!(
            d.arm_model,
            crate::skin_uv::ArmModel::Classic,
            "Classic arms are the default look"
        );
    }

    #[test]
    fn rgba_source_is_not_default() {
        let d = CosmeticDescriptor { version: 1, skin: SkinSource::Rgba64(vec![0u8; 64 * 64 * 4]), ..Default::default() };
        assert!(!d.is_default_skin());
    }

    #[test]
    fn decode_accepts_64x64_png() {
        let rgba = crate::texture_gen::default_skin_rgba();
        let img = image::RgbaImage::from_raw(64, 64, rgba).expect("64x64 buffer");
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).expect("encode png");
        let decoded = decode_skin_64(&png).expect("64x64 png should decode");
        assert_eq!(decoded.len(), 64 * 64 * 4);
    }

    #[test]
    fn decode_rejects_wrong_size() {
        let img = image::RgbaImage::from_raw(32, 32, vec![255u8; 32 * 32 * 4]).unwrap();
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let err = decode_skin_64(&png).unwrap_err();
        assert!(err.contains("64"), "error should mention the required size: {err}");
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_skin_64(b"not an image at all").is_err());
    }

    // ── Phase 4: native custom-skin upload contract ─────────────────────────
    // The native OS-PNG-picker poll site (`game_loop::update_and_render`) reads
    // raw file bytes off a worker thread, then runs EXACTLY this decode before
    // adding a new wardrobe entry. These pin that PNG-bytes → 16384-byte RGBA
    // contract (and the malformed-no-panic path) independently of the WASM picker,
    // which is unavailable headlessly. They overlap `decode_accepts_*`/`decode_rejects_*`
    // by design — the native upload path must stay covered if those move.

    /// Build the PNG the worker would hand the native poll site, then decode it
    /// the same way: valid 64×64 → Ok with 64*64*4 = 16384 RGBA bytes.
    #[test]
    fn native_upload_decode_yields_16384_rgba() {
        let rgba = crate::texture_gen::default_skin_rgba();
        let img = image::RgbaImage::from_raw(64, 64, rgba).expect("64x64 buffer");
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode png");
        let decoded = decode_skin_64(&png).expect("a 64x64 png must decode for the upload path");
        assert_eq!(decoded.len(), 64 * 64 * 4, "native upload must apply 16384 RGBA bytes");
    }

    /// A file that isn't a PNG (the worker still reads + forwards its bytes) must
    /// produce a non-empty friendly Err the panel can show — never a panic.
    #[test]
    fn native_upload_decode_rejects_non_png_without_panic() {
        let err = decode_skin_64(b"not a png").expect_err("garbage bytes must not decode");
        assert!(!err.is_empty(), "the panel needs a friendly message to show");
    }

    #[test]
    fn skin_rgba_default_matches_default_skin() {
        assert_eq!(CosmeticDescriptor::default().skin_rgba(), crate::texture_gen::default_skin_rgba());
    }

    #[test]
    fn skin_rgba_custom_returns_its_bytes() {
        let px = vec![7u8; 64 * 64 * 4];
        let d = CosmeticDescriptor { version: 1, skin: SkinSource::Rgba64(px.clone()), ..Default::default() };
        assert_eq!(d.skin_rgba(), px);
    }

    #[test]
    fn skin_key_default_is_zero() {
        assert_eq!(CosmeticDescriptor::default().skin_key(), 0);
    }
    #[test]
    fn skin_key_distinguishes_skins() {
        let a = CosmeticDescriptor { version: 1, skin: SkinSource::Rgba64(vec![1u8; 64 * 64 * 4]), ..Default::default() };
        let b = CosmeticDescriptor { version: 1, skin: SkinSource::Rgba64(vec![2u8; 64 * 64 * 4]), ..Default::default() };
        assert_ne!(a.skin_key(), 0);
        assert_ne!(a.skin_key(), b.skin_key(), "different skins -> different keys");
        let a2 = CosmeticDescriptor { version: 1, skin: SkinSource::Rgba64(vec![1u8; 64 * 64 * 4]), ..Default::default() };
        assert_eq!(a.skin_key(), a2.skin_key(), "same skin -> same key");
    }

    // ── Task A1: RGBA→PNG encoder + decode-any (TDD) ─────────────────────────

    #[test]
    fn encode_then_decode_roundtrips() {
        let rgba = crate::texture_gen::default_skin_rgba();
        let png = encode_skin_png(&rgba).expect("encode a 64x64 skin");
        // The exported PNG must be a real, decodable 64x64 Minecraft skin.
        let back = decode_skin_64(&png).expect("our own export must re-decode");
        assert_eq!(back, rgba, "export → import must be byte-identical");
    }

    #[test]
    fn encode_rejects_wrong_length() {
        assert!(encode_skin_png(&[0u8; 100]).is_err());
    }

    #[test]
    fn decode_any_accepts_modern_64x64() {
        let rgba = crate::texture_gen::default_skin_rgba();
        let png = encode_skin_png(&rgba).unwrap();
        let (out, legacy) = decode_skin_any(&png).expect("64x64 decodes");
        assert_eq!(out.len(), 64 * 64 * 4);
        assert!(!legacy, "64x64 is not legacy");
    }

    #[test]
    fn decode_any_expands_legacy_64x32() {
        // A 64x32 RGBA buffer (8192 bytes) → PNG → decode_any expands to 16384.
        let legacy_rgba = vec![200u8; 64 * 32 * 4];
        let img = image::RgbaImage::from_raw(64, 32, legacy_rgba).unwrap();
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (out, legacy) = decode_skin_any(&png).expect("64x32 must expand, not reject");
        assert_eq!(out.len(), 64 * 64 * 4, "expanded to a full 64x64 sheet");
        assert!(legacy, "64x32 is flagged legacy");
        // Top half preserved (row 0 of the source survives unchanged).
        assert_eq!(&out[0..64 * 4], &vec![200u8; 64 * 4][..]);
    }

    #[test]
    fn decode_any_rejects_other_sizes() {
        let img = image::RgbaImage::from_raw(32, 32, vec![1u8; 32 * 32 * 4]).unwrap();
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert!(decode_skin_any(&png).is_err());
    }
}
