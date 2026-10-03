//! Minecraft-username skin import (download only, spec §8.4). Pure parse +
//! legacy-format conversion here; the web path runs through our FastAPI proxy
//! (no CORS on Mojang), the native path fetches directly off a worker thread.
//!
//! Phase A delivers only `expand_legacy_skin` (needed by `cosmetics::decode_skin_any`).
//! Phase B adds the parse helpers, HTTP worker, and full `McImportOutcome` types.

/// Copy a `w×h` rect from `src` at `(sx,sy)` to `dst` at `(dx,dy)`, mirrored
/// horizontally (pixel column `c` of the source lands at column `w-1-c`).
fn blit_mirror_x(
    src: &[u8],
    dst: &mut [u8],
    sx: u32,
    sy: u32,
    dx: u32,
    dy: u32,
    w: u32,
    h: u32,
) {
    const STRIDE: usize = 64 * 4;
    for row in 0..h {
        for col in 0..w {
            let s = ((sy + row) as usize) * STRIDE + ((sx + col) as usize) * 4;
            let d = ((dy + row) as usize) * STRIDE + ((dx + (w - 1 - col)) as usize) * 4;
            dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
}

// ─── base64 decoder ─────────────────────────────────────────────────────────
// Native (Task B4) uses the `base64` crate — it's already pulled in for the
// native HTTP import path, so reuse it. WASM keeps the hand-rolled pure decoder
// below so the wasm graph stays dep-free (no `base64`/`ureq` leak into the
// bundle-size gate). Both only ever see Mojang's standard-alphabet textures
// value. Same signature so `parse_textures_value` is platform-agnostic.

#[cfg(not(target_arch = "wasm32"))]
fn decode_base64(input: &str) -> Result<Vec<u8>, ()> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .map_err(|_| ())
}

#[cfg(target_arch = "wasm32")]
fn decode_base64(input: &str) -> Result<Vec<u8>, ()> {
    let input = input.trim();
    let mut out = Vec::with_capacity((input.len() * 3) / 4 + 2);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for b in input.bytes() {
        let val: u32 = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\r' | b'\n' | b' ' => continue,
            _ => return Err(()),
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

// ─── legacy skin expander ────────────────────────────────────────────────────

/// Expand a legacy 64×32 skin (8192 RGBA bytes) to the modern 64×64 sheet
/// (16384 bytes). Copies the top half verbatim, then mirror-copies the right
/// arm/leg into the new left arm/leg slots (the 1.8 mapping). Overlay rows stay
/// transparent (no overlay existed in 64×32). Coordinates from skin_uv.rs.
pub fn expand_legacy_skin(rgba_64x32: &[u8]) -> Vec<u8> {
    // Normalise the input to a full 64×32 sheet (8192 bytes) up front: a short
    // or truncated input must NOT panic — neither the top-half copy below nor
    // the limb blits (which read `src` at fixed offsets) can then read past the
    // end. Padding bytes stay zero (transparent), which is the right default.
    let half = 64 * 32 * 4;
    let src = if rgba_64x32.len() == half {
        std::borrow::Cow::Borrowed(rgba_64x32)
    } else {
        let mut padded = vec![0u8; half];
        let n = half.min(rgba_64x32.len());
        padded[..n].copy_from_slice(&rgba_64x32[..n]);
        std::borrow::Cow::Owned(padded)
    };
    let src: &[u8] = &src;

    let mut out = vec![0u8; 64 * 64 * 4];
    // Top half (rows 0..32) verbatim.
    out[..half].copy_from_slice(src);

    // (sx, sy, dx, dy, w, h) — face order R, L, T, B, back, front.
    // Left ARM from right arm (right-arm base block 40,16; left-arm 32,48).
    let arm: [(u32, u32, u32, u32, u32, u32); 6] = [
        (48, 20, 32, 52, 4, 12), // R ← src L
        (40, 20, 40, 52, 4, 12), // L ← src R
        (44, 16, 36, 48, 4, 4),  // T
        (48, 16, 40, 48, 4, 4),  // B
        (52, 20, 44, 52, 4, 12), // back
        (44, 20, 36, 52, 4, 12), // front
    ];
    // Left LEG from right leg (right-leg base block 0,16; left-leg 16,48).
    let leg: [(u32, u32, u32, u32, u32, u32); 6] = [
        (8, 20, 16, 52, 4, 12),  // R ← src L
        (0, 20, 24, 52, 4, 12),  // L ← src R
        (4, 16, 20, 48, 4, 4),   // T
        (8, 16, 24, 48, 4, 4),   // B
        (12, 20, 28, 52, 4, 12), // back
        (4, 20, 20, 52, 4, 12),  // front
    ];
    for (sx, sy, dx, dy, w, h) in arm.into_iter().chain(leg) {
        blit_mirror_x(src, &mut out, sx, sy, dx, dy, w, h);
    }
    out
}

// ─── types ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McImportError {
    NotFound,
    NoCustomSkin,
    Offline,
    RateLimited,
    BadName,
    /// The downloaded skin decoded to an unusual size we can't use yet (not a
    /// 64×64 or legacy 64×32 sheet). Distinct from `Offline` so the kid sees a
    /// "we can't use this picture" message, not "can't reach Minecraft".
    BadImage,
}

impl McImportError {
    /// Kid-readable message (never "Error"/"404"; spec §11 + kid-UX §2c).
    pub fn user_message(&self) -> &'static str {
        match self {
            McImportError::NotFound =>
                "We couldn't find anyone by that name. Check the spelling — Minecraft usernames are case-sensitive, and this is Java Edition only (not Xbox/Bedrock).",
            McImportError::NoCustomSkin =>
                "That player is using a default skin (Steve or Alex) — there's no custom skin to grab. Try a different username!",
            McImportError::Offline =>
                "Can't reach Minecraft right now. This might be temporary — try again in a minute!",
            McImportError::RateLimited =>
                "Slow down a bit! We've looked up too many skins just now. Try again in about a minute.",
            McImportError::BadName =>
                "That doesn't look like a Minecraft username. Use letters, numbers, and _ (1–16 characters).",
            McImportError::BadImage =>
                "That skin is an unusual size we can't use yet (try a player whose skin is the normal 64×64 size).",
        }
    }
}

/// Should an async import result be applied, or discarded as stale? Each kick is
/// tagged with the generation token (`mc_import_token`) it started under; if the
/// player has since cancelled, closed the panel, or kicked a different request,
/// the live token has moved on and a late result must be dropped — otherwise it
/// could overwrite (and persist) the WRONG wardrobe entry. Pure so it's unit-
/// tested without the renderer; called at both drain sites (native + web).
pub fn mc_import_result_is_fresh(result_token: u32, current_token: u32) -> bool {
    result_token == current_token
}

#[derive(Clone, Debug)]
pub enum McImportOutcome {
    Ok { png: Vec<u8>, uuid: String, name: String, slim: bool },
    Err(McImportError),
}

#[derive(Clone, Debug)]
pub enum McQuery {
    ByName(String),
    ByUuid(String),
}

// ─── validators ──────────────────────────────────────────────────────────────

/// Mojang Java usernames: 1–16 of [A-Za-z0-9_].
pub fn validate_username(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.len() <= 16 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Mojang profile UUIDs as used in the sessionserver URL: 32 ASCII hex chars
/// (dash-less). Validated before building the URL so a tampered stored value
/// can't reshape the request (SSRF parity with the web proxy, spec §11).
pub fn validate_uuid(uuid: &str) -> bool {
    let u = uuid.trim();
    u.len() == 32 && u.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The skin URL must be on Mojang's texture CDN (host == textures.minecraft.net),
/// not a relay. Guards against a tampered/unexpected profile blob (spec §11 #4).
pub fn texture_url_is_mojang(url: &str) -> bool {
    // host is between "://" and the next "/".
    let rest = match url.split_once("://") {
        Some((_, r)) => r,
        None => return false,
    };
    let host = rest.split('/').next().unwrap_or("");
    host.eq_ignore_ascii_case("textures.minecraft.net")
}

// ─── profile parse ───────────────────────────────────────────────────────────

/// Parse the outer sessionserver profile JSON → (uuid, name). The textures
/// property `value` is base64 JSON parsed separately by `parse_textures_value`.
pub fn parse_profile_json(body: &str) -> Result<(String, String), McImportError> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| McImportError::Offline)?;
    let uuid = v.get("id").and_then(|x| x.as_str()).ok_or(McImportError::NotFound)?;
    let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("Player");
    Ok((uuid.to_string(), name.to_string()))
}

/// Extract the base64 `textures` property value from the outer profile.
pub fn extract_textures_value(body: &str) -> Result<String, McImportError> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| McImportError::Offline)?;
    let props = v
        .get("properties")
        .and_then(|p| p.as_array())
        .ok_or(McImportError::NoCustomSkin)?;
    for p in props {
        if p.get("name").and_then(|x| x.as_str()) == Some("textures")
            && let Some(val) = p.get("value").and_then(|x| x.as_str()) {
                return Ok(val.to_string());
            }
    }
    Err(McImportError::NoCustomSkin)
}

/// Decode the base64 textures blob → (skin_url, is_slim, profile_name).
/// `textures.SKIN` absent → `NoCustomSkin`. `metadata.model == "slim"` → slim
/// (absent metadata == Classic; code `!= "slim"`, never `== "steve"`).
pub fn parse_textures_value(b64_value: &str) -> Result<(String, bool, String), McImportError> {
    let raw = decode_base64(b64_value).map_err(|_| McImportError::Offline)?;
    let v: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| McImportError::Offline)?;
    let name = v
        .get("profileName")
        .and_then(|x| x.as_str())
        .unwrap_or("Player")
        .to_string();
    let skin = v
        .get("textures")
        .and_then(|t| t.get("SKIN"))
        .ok_or(McImportError::NoCustomSkin)?;
    let url = skin
        .get("url")
        .and_then(|x| x.as_str())
        .ok_or(McImportError::NoCustomSkin)?;
    let slim = skin
        .get("metadata")
        .and_then(|m| m.get("model"))
        .and_then(|x| x.as_str())
        == Some("slim");
    Ok((url.to_string(), slim, name))
}

// ─── native HTTP worker ────────────────────────────────────────────────────────
// Browsers go through our /mc-skin proxy (Mojang has no CORS); native fetches
// Mojang directly off a worker thread (mirrors native_file_dialog's off-thread +
// mpsc pattern). NATIVE-ONLY — wasm never compiles this (no ureq in the graph).

#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, channel};

/// Spawn a worker thread that performs the import (3 blocking GETs for a name,
/// 2 for a UUID refresh) and sends exactly one `McImportOutcome` back. Mirrors
/// the native_file_dialog off-thread + mpsc pattern. The PNG is returned RAW
/// (the caller runs `decode_skin_any` to handle the legacy 64×32 case).
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_mc_import(query: McQuery) -> Receiver<McImportOutcome> {
    let (tx, rx) = channel::<McImportOutcome>();
    std::thread::spawn(move || {
        let _ = tx.send(run_mc_import(query));
    });
    rx
}

#[cfg(not(target_arch = "wasm32"))]
fn http_get_text(url: &str) -> Result<(u16, String), McImportError> {
    // redirects(0): never auto-follow a 3xx — SSRF parity with the web httpx
    // proxy, which doesn't follow either. A redirect surfaces as a non-200
    // status and is treated as a failure by the callers.
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(4))
        .redirects(0)
        .build();
    match agent.get(url).call() {
        Ok(resp) => {
            let code = resp.status();
            let body = resp.into_string().map_err(|_| McImportError::Offline)?;
            Ok((code, body))
        }
        Err(ureq::Error::Status(code, _resp)) => Ok((code, String::new())),
        Err(_) => Err(McImportError::Offline),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn http_get_png(url: &str) -> Result<Vec<u8>, McImportError> {
    // redirects(0): SSRF parity with the web path (see `http_get_text`).
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(4))
        .redirects(0)
        .build();
    let resp = agent.get(url).call().map_err(|_| McImportError::Offline)?;
    if resp.content_type() != "image/png" {
        return Err(McImportError::Offline);
    }
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf)
        .map_err(|_| McImportError::Offline)?;
    Ok(buf)
}

#[cfg(not(target_arch = "wasm32"))]
fn run_mc_import(query: McQuery) -> McImportOutcome {
    // Step 1 (name only): username → UUID.
    let (uuid, fallback_name) = match query {
        McQuery::ByName(name) => {
            if !validate_username(&name) {
                return McImportOutcome::Err(McImportError::BadName);
            }
            let url = format!("https://api.mojang.com/users/profiles/minecraft/{name}");
            match http_get_text(&url) {
                Ok((200, body)) => match parse_profile_json(&body) {
                    Ok((id, n)) => (id, n),
                    Err(_) => return McImportOutcome::Err(McImportError::NotFound),
                },
                Ok((404, _)) => return McImportOutcome::Err(McImportError::NotFound),
                Ok((429, _)) => return McImportOutcome::Err(McImportError::RateLimited),
                Ok(_) => return McImportOutcome::Err(McImportError::Offline),
                Err(e) => return McImportOutcome::Err(e),
            }
        }
        McQuery::ByUuid(id) => {
            // Validate before building the URL (SSRF parity with the web proxy).
            if !validate_uuid(&id) {
                return McImportOutcome::Err(McImportError::BadName);
            }
            (id, "Player".to_string())
        }
    };

    // Step 2: UUID → profile (textures blob).
    let purl = format!("https://sessionserver.mojang.com/session/minecraft/profile/{uuid}");
    let profile_body = match http_get_text(&purl) {
        Ok((200, body)) => body,
        Ok((204, _)) => return McImportOutcome::Err(McImportError::NotFound),
        Ok((429, _)) => return McImportOutcome::Err(McImportError::RateLimited),
        Ok(_) => return McImportOutcome::Err(McImportError::Offline),
        Err(e) => return McImportOutcome::Err(e),
    };
    let value = match extract_textures_value(&profile_body) {
        Ok(v) => v,
        Err(e) => return McImportOutcome::Err(e),
    };
    let (skin_url, slim, profile_name) = match parse_textures_value(&value) {
        Ok(t) => t,
        Err(e) => return McImportOutcome::Err(e),
    };
    if !texture_url_is_mojang(&skin_url) {
        return McImportOutcome::Err(McImportError::Offline);
    }
    let name = if profile_name == "Player" { fallback_name } else { profile_name };

    // Step 3: fetch the PNG (force https).
    let png_url = skin_url.replacen("http://", "https://", 1);
    match http_get_png(&png_url) {
        Ok(png) => McImportOutcome::Ok { png, uuid, name, slim },
        Err(e) => McImportOutcome::Err(e),
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn set_px(buf: &mut [u8], stride: usize, x: usize, y: usize, rgba: [u8; 4]) {
        let off = y * stride * 4 + x * 4;
        buf[off..off + 4].copy_from_slice(&rgba);
    }

    fn get_px(buf: &[u8], stride: usize, x: usize, y: usize) -> [u8; 4] {
        let off = y * stride * 4 + x * 4;
        buf[off..off + 4].try_into().unwrap()
    }

    /// Minimal standard-base64 encoder (test-only; prod decode is in the module).
    fn base64_encode_test(input: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let combined = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((combined >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((combined >> 12) & 0x3f) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[((combined >> 6) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(combined & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    /// Extract the base64 textures value from an outer profile JSON (test helper).
    fn extract_textures_value_test(body: &str) -> String {
        extract_textures_value(body).expect("test profile must have a textures property")
    }

    #[test]
    fn legacy_expand_preserves_top_and_fills_size() {
        let src = vec![123u8; 64 * 32 * 4];
        let out = expand_legacy_skin(&src);
        assert_eq!(out.len(), 64 * 64 * 4);
        // Top half (rows 0..32) is copied verbatim.
        assert_eq!(&out[0..64 * 32 * 4], &src[..]);
    }

    #[test]
    fn legacy_expand_writes_left_limbs() {
        // Paint a distinctive value into the right-leg FRONT source rect (4,20,4,12)
        // and assert the mirrored left-leg FRONT dst rect (20,52,4,12) received it.
        let mut src = vec![0u8; 64 * 32 * 4];
        set_px(&mut src, 64, 5, 21, [10, 20, 30, 255]); // inside (4,20,4,12)
        let out = expand_legacy_skin(&src);
        // mirror_x within the 4-wide rect: src col offset 1 (x=5 → off 1) → dst off 2.
        let dst = get_px(&out, 64, 20 + 2, 52 + 1);
        assert_eq!(dst, [10, 20, 30, 255], "left-leg front is the mirrored right-leg front");
    }

    #[test]
    fn username_validation() {
        assert!(validate_username("Notch"));
        assert!(validate_username("a_B9"));
        assert!(!validate_username(""));
        assert!(!validate_username("has space"));
        assert!(!validate_username("toolongusername17")); // 17 chars
        assert!(!validate_username("bad-dash"));
    }

    #[test]
    fn texture_url_allowlist() {
        assert!(texture_url_is_mojang("http://textures.minecraft.net/texture/abc"));
        assert!(texture_url_is_mojang("https://textures.minecraft.net/texture/abc"));
        assert!(!texture_url_is_mojang("https://crafatar.com/skins/x"));
        assert!(!texture_url_is_mojang("https://evil.example/textures.minecraft.net"));
    }

    #[test]
    fn parse_profile_extracts_id_and_name() {
        // Outer profile shape (sessionserver). textures value is base64 JSON.
        let inner = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/abc"}}}"#;
        let b64 = base64_encode_test(inner.as_bytes());
        let outer = format!(
            r#"{{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[{{"name":"textures","value":"{b64}"}}]}}"#
        );
        let (uuid, name) = parse_profile_json(&outer).unwrap();
        assert_eq!(uuid, "069a79f444e94726a5befca90e38aaf5");
        assert_eq!(name, "Notch");
        // The textures value parses to a Mojang skin URL, classic (no metadata).
        let value = extract_textures_value_test(&outer);
        let (url, slim, _n) = parse_textures_value(&value).unwrap();
        assert!(texture_url_is_mojang(&url));
        assert!(!slim, "no metadata.model → Classic, not slim");
    }

    #[test]
    fn parse_textures_slim_flag() {
        let inner = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/abc","metadata":{"model":"slim"}}}}"#;
        let b64 = base64_encode_test(inner.as_bytes());
        let (_url, slim, _n) = parse_textures_value(&b64).unwrap();
        assert!(slim, "metadata.model == slim → slim");
    }

    #[test]
    fn parse_textures_no_custom_skin() {
        let inner = r#"{"textures":{}}"#; // default-skin account: no SKIN key
        let b64 = base64_encode_test(inner.as_bytes());
        assert!(matches!(parse_textures_value(&b64), Err(McImportError::NoCustomSkin)));
    }

    #[test]
    fn legacy_expand_handles_short_input_without_panic() {
        // A truncated (< 8192-byte) input must not panic — the copy length is
        // matched and the rest stays zero-filled.
        let short = vec![7u8; 100];
        let out = expand_legacy_skin(&short);
        assert_eq!(out.len(), 64 * 64 * 4);
        assert_eq!(&out[..100], &short[..]);
        assert!(out[100..].iter().all(|&b| b == 0), "remainder stays zero-filled");
    }

    #[test]
    fn uuid_validation() {
        assert!(validate_uuid("069a79f444e94726a5befca90e38aaf5")); // 32 hex
        assert!(!validate_uuid("069a79f4-44e9-4726-a5be-fca90e38aaf5")); // dashed
        assert!(!validate_uuid("069a79f444e94726a5befca90e38aaf")); // 31 chars
        assert!(!validate_uuid("069a79f444e94726a5befca90e38aaf5x")); // 33 chars
        assert!(!validate_uuid("069a79f444e94726a5befca90e38aaZZ")); // non-hex
        assert!(!validate_uuid(""));
    }

    #[test]
    fn import_token_discard_if_stale() {
        // Same generation → fresh; any mismatch (superseded / cancelled) → stale.
        assert!(mc_import_result_is_fresh(5, 5));
        assert!(!mc_import_result_is_fresh(5, 6)); // a later kick bumped the token
        assert!(!mc_import_result_is_fresh(6, 5)); // out-of-order late result
        assert!(!mc_import_result_is_fresh(0, 1)); // cancelled after kick
    }

    #[test]
    fn bad_image_has_distinct_message() {
        // BadImage must read differently from Offline (kid-UX §2c).
        assert_ne!(
            McImportError::BadImage.user_message(),
            McImportError::Offline.user_message()
        );
    }
}
