//! Trial postcards: finishing a trial leaves a small picture of the player's
//! result, kept in a local album. Design: `docs/vision/trials-two-acts.md` Part C,
//! §C.1 principle 2 and §C.2 "Postcards" / "Album" (internal repo).
//!
//! Local only. Native keeps the newest [`KEEP_PER_TRIAL`] PNGs per trial under
//! `profile/postcards/<token>/<unix-ms>.png`, next to `trials.json`; the web
//! keeps the newest one per trial in localStorage, sized so every trial's card
//! fits well inside the browser's quota. Nothing here is sent anywhere, and the
//! postcards are not part of any cloud sync, Stash or profile bundle.
//!
//! The pure parts (token resolution, file names, pruning, ordering, the web
//! size budget, cropping and the small base64 codec) are unit-tested; the GPU
//! capture lives in `renderer/postcard_capture.rs` and the flow in
//! `postcard_flow.rs`.

use crate::trials::{TrialRef, TRIAL_ORDER};
use crate::trials_gallery::{row_kind_of, row_specs, RowKind};

/// A postcard's stored size on native (and what the capture is reduced to).
pub const POSTCARD_W: u32 = 320;
pub const POSTCARD_H: u32 = 180;

/// How many postcards native keeps per trial (the newest).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const KEEP_PER_TRIAL: usize = 5;

/// The folder under the profile folder.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const DIR_NAME: &str = "postcards";

/// One kept postcard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Postcard {
    /// The trial's token (a challenge token or a race id).
    pub token: String,
    /// When it was taken, unix milliseconds.
    pub ms: u64,
    /// The picture, PNG-encoded.
    pub png: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Pure: keys
// ---------------------------------------------------------------------------

fn token_of(tref: TrialRef) -> &'static str {
    match tref {
        TrialRef::Race(id) | TrialRef::Challenge(id) => id,
    }
}

/// The token of a bundled race, if `race_id` is one.
pub fn token_for_race(race_id: &str) -> Option<&'static str> {
    TRIAL_ORDER.iter().find_map(|(_, r)| match r {
        TrialRef::Race(id) if *id == race_id => Some(*id),
        _ => None,
    })
}

/// The token of the bundled challenge whose display name is `display`, read
/// from the challenge listing (the name the scenario runs under).
pub fn token_for_challenge(display: &str) -> Option<&'static str> {
    let listing = crate::scenario::challenge_listing();
    let token = listing.iter().find(|(_, d)| d == display).map(|(t, _)| *t)?;
    TRIAL_ORDER.iter().find_map(|(_, r)| match r {
        TrialRef::Challenge(t) if *t == token => Some(*t),
        _ => None,
    })
}

/// Whether a token is safe to use as a folder or storage-key part: short,
/// lower-case letters, digits, `-` and `_` only.
pub fn is_safe_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 48
        && token.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Whether `token` is a bundled trial's.
pub fn is_known_token(token: &str) -> bool {
    TRIAL_ORDER.iter().any(|(_, r)| token_of(*r) == token)
}

// ---------------------------------------------------------------------------
// Pure: file names, pruning, ordering
// ---------------------------------------------------------------------------

/// The file a postcard taken at `ms` is stored as.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn file_name(ms: u64) -> String {
    format!("{ms}.png")
}

/// The time back out of a postcard file name; `None` for any other file.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn parse_file_name(name: &str) -> Option<u64> {
    let stem = name.strip_suffix(".png")?;
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

/// Of the times kept for one trial, the ones to delete so only the newest
/// `keep` remain (oldest first).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn to_prune(times: &[u64], keep: usize) -> Vec<u64> {
    let mut sorted = times.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted.dedup();
    let mut out: Vec<u64> = sorted.into_iter().skip(keep).collect();
    out.sort_unstable();
    out
}

/// The newest postcard first.
pub fn sort_newest_first(cards: &mut [Postcard]) {
    cards.sort_by(|a, b| b.ms.cmp(&a.ms).then_with(|| a.token.cmp(&b.token)));
}

/// Which gallery row a token's trial sits in (`None` for an unknown token).
pub fn row_of(token: &str) -> Option<RowKind> {
    TRIAL_ORDER
        .iter()
        .find(|(_, r)| token_of(*r) == token)
        .map(|(t, r)| row_kind_of(*t, *r))
}

/// The album's groups: one per gallery row in the gallery's order (Make, Tend,
/// Tech, Brave, Against the clock), each holding indices into `cards`, newest
/// first. Rows with no postcard, and unknown tokens, are left out.
pub fn album_groups(cards: &[Postcard]) -> Vec<(RowKind, Vec<usize>)> {
    row_specs()
        .into_iter()
        .filter_map(|spec| {
            let mut idx: Vec<usize> =
                (0..cards.len()).filter(|&i| row_of(&cards[i].token) == Some(spec.kind)).collect();
            idx.sort_by(|&a, &b| cards[b].ms.cmp(&cards[a].ms).then(a.cmp(&b)));
            (!idx.is_empty()).then_some((spec.kind, idx))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pure: pixels
// ---------------------------------------------------------------------------

/// The centred crop of a `w`×`h` frame that has the aspect `out_w`:`out_h`,
/// as (x, y, width, height).
pub fn crop_rect(w: u32, h: u32, out_w: u32, out_h: u32) -> (u32, u32, u32, u32) {
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 {
        return (0, 0, w, h);
    }
    // Compare w/h with out_w/out_h without floats.
    let (w64, h64, ow, oh) = (w as u64, h as u64, out_w as u64, out_h as u64);
    if w64 * oh > h64 * ow {
        // Too wide: keep the full height.
        let cw = ((h64 * ow) / oh).max(1) as u32;
        ((w - cw) / 2, 0, cw, h)
    } else {
        let ch = ((w64 * oh) / ow).max(1) as u32;
        (0, (h - ch) / 2, w, ch)
    }
}

/// Centre-crop an RGBA frame to the aspect of `out_w`×`out_h` and reduce it
/// to that size by averaging each output pixel's source area. Alpha is set
/// opaque. Returns `None` if `rgba` is not `w`×`h`×4 bytes.
pub fn crop_and_reduce(rgba: &[u8], w: u32, h: u32, out_w: u32, out_h: u32) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 || rgba.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let (cx, cy, cw, ch) = crop_rect(w, h, out_w, out_h);
    let mut out = Vec::with_capacity((out_w * out_h * 4) as usize);
    for oy in 0..out_h {
        let y0 = cy + (oy as u64 * ch as u64 / out_h as u64) as u32;
        let y1 = (cy + ((oy as u64 + 1) * ch as u64 / out_h as u64) as u32).max(y0 + 1).min(h);
        for ox in 0..out_w {
            let x0 = cx + (ox as u64 * cw as u64 / out_w as u64) as u32;
            let x1 = (cx + ((ox as u64 + 1) * cw as u64 / out_w as u64) as u32).max(x0 + 1).min(w);
            let mut sum = [0u32; 3];
            let mut n = 0u32;
            for y in y0..y1 {
                let row = (y as usize) * (w as usize) * 4;
                for x in x0..x1 {
                    let p = row + (x as usize) * 4;
                    sum[0] += rgba[p] as u32;
                    sum[1] += rgba[p + 1] as u32;
                    sum[2] += rgba[p + 2] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            out.extend_from_slice(&[
                ((sum[0] + n / 2) / n) as u8,
                ((sum[1] + n / 2) / n) as u8,
                ((sum[2] + n / 2) / n) as u8,
                255,
            ]);
        }
    }
    Some(out)
}

/// Encode an RGBA picture as an RGB PNG. `posterize_bits` (1..=8) keeps that
/// many bits per channel (8 = untouched); fewer bits shrink the file.
pub fn encode_png(rgba: &[u8], w: u32, h: u32, posterize_bits: u8) -> Option<Vec<u8>> {
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let bits = posterize_bits.clamp(1, 8);
    let mask: u8 = !((1u16 << (8 - bits)) as u8).wrapping_sub(1);
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0] & mask, p[1] & mask, p[2] & mask]).collect();
    let img = image::RgbImage::from_raw(w, h, rgb)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// Decode a stored postcard to (width, height, RGBA).
pub fn decode_png(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    Some((img.width(), img.height(), img.into_raw()))
}

// ---------------------------------------------------------------------------
// Pure: the web budget
// ---------------------------------------------------------------------------

/// The localStorage quota the budget is planned against (the common 5 MB).
#[cfg_attr(not(test), allow(dead_code))]
pub const WEB_QUOTA_BYTES: usize = 5_000_000;

/// localStorage counts characters; browsers hold them as UTF-16, two bytes each.
#[cfg_attr(not(test), allow(dead_code))]
pub const WEB_BYTES_PER_CHAR: usize = 2;

/// The longest stored value one postcard may take, in characters. With one
/// postcard per trial this keeps every trial's card under half the quota.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const WEB_MAX_VALUE_CHARS: usize = 24_000;

/// The sizes the web tries, largest first, and the bits per channel kept at
/// each: the first encoding that fits [`WEB_MAX_VALUE_CHARS`] is stored.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const WEB_LADDER: [(u32, u32, u8); 4] =
    [(192, 108, 8), (192, 108, 5), (160, 90, 5), (128, 72, 4)];

/// The localStorage key prefix (one key per trial).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const WEB_KEY_PREFIX: &str = "axenstax_postcard_";

/// Characters of standard base64 for `n` bytes.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn base64_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

/// The stored value: `<unix-ms>:<base64 png>`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn web_value(ms: u64, png: &[u8]) -> String {
    format!("{ms}:{}", base64_encode(png))
}

/// A stored value back to (ms, png).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn parse_web_value(value: &str) -> Option<(u64, Vec<u8>)> {
    let (ms, b64) = value.split_once(':')?;
    Some((ms.parse().ok()?, base64_decode(b64)?))
}

/// Encode a native-size postcard (RGBA, `POSTCARD_W`×`POSTCARD_H`) for the web:
/// down the ladder until the stored value fits. `None` when none fits.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn web_encode(rgba: &[u8], ms: u64) -> Option<String> {
    for (w, h, bits) in WEB_LADDER {
        let small = crop_and_reduce(rgba, POSTCARD_W, POSTCARD_H, w, h)?;
        let png = encode_png(&small, w, h, bits)?;
        let value = web_value(ms, &png);
        if value.len() <= WEB_MAX_VALUE_CHARS {
            return Some(value);
        }
    }
    None
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (the `base64` crate is native-only).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(base64_len(bytes.len()));
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Decode standard base64 (padding required); `None` on anything malformed.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let val = |c: u8| -> Option<u32> { B64.iter().position(|&x| x == c).map(|p| p as u32) };
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for (i, q) in s.chunks(4).enumerate() {
        let last = i == s.len() / 4 - 1;
        let pad = q.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (j, &c) in q.iter().enumerate() {
            let v = if j >= 4 - pad { 0 } else { val(c)? };
            n = (n << 6) | v;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Storage (best-effort: a failure means the postcard is not kept, never a crash)
// ---------------------------------------------------------------------------

/// Unix milliseconds now.
pub fn now_ms() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Keep a postcard for `token`. `rgba` is `POSTCARD_W`×`POSTCARD_H` RGBA.
/// Returns the stored PNG (what the album will show), or `None` if it was not
/// kept (unknown token, encode failure, a full disk or a full quota).
pub fn save(token: &str, rgba: &[u8], ms: u64) -> Option<Vec<u8>> {
    if !is_safe_token(token) || !is_known_token(token) {
        return None;
    }
    save_impl(token, rgba, ms)
}

/// The newest postcard's PNG for a trial token: what the Trials island reads.
/// Read next by the Trials island (§C.3); nothing in the game calls it yet.
#[allow(dead_code)]
pub fn newest_png(token: &str) -> Option<Vec<u8>> {
    if !is_safe_token(token) {
        return None;
    }
    newest_impl(token)
}

/// Every kept postcard, newest first.
pub fn load_all() -> Vec<Postcard> {
    let mut all: Vec<Postcard> = TRIAL_ORDER
        .iter()
        .map(|(_, r)| token_of(*r))
        .filter(|t| is_safe_token(t))
        .flat_map(load_token)
        .collect();
    sort_newest_first(&mut all);
    all
}

#[cfg(not(target_arch = "wasm32"))]
fn token_dir(token: &str) -> std::path::PathBuf {
    crate::data_dir::profile_dir().join(DIR_NAME).join(token)
}

#[cfg(not(target_arch = "wasm32"))]
fn times_in(dir: &std::path::Path) -> Vec<u64> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().to_str().and_then(parse_file_name))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn save_impl(token: &str, rgba: &[u8], ms: u64) -> Option<Vec<u8>> {
    let png = encode_png(rgba, POSTCARD_W, POSTCARD_H, 8)?;
    let dir = token_dir(token);
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(file_name(ms));
    let tmp = dir.join(format!("{ms}.tmp"));
    std::fs::write(&tmp, &png).ok()?;
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    for old in to_prune(&times_in(&dir), KEEP_PER_TRIAL) {
        let _ = std::fs::remove_file(dir.join(file_name(old)));
    }
    Some(png)
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn newest_impl(token: &str) -> Option<Vec<u8>> {
    let dir = token_dir(token);
    let newest = times_in(&dir).into_iter().max()?;
    std::fs::read(dir.join(file_name(newest))).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn load_token(token: &str) -> Vec<Postcard> {
    let dir = token_dir(token);
    let mut times = times_in(&dir);
    times.sort_unstable_by(|a, b| b.cmp(a));
    times
        .into_iter()
        .take(KEEP_PER_TRIAL)
        .filter_map(|ms| {
            let png = std::fs::read(dir.join(file_name(ms))).ok()?;
            Some(Postcard { token: token.to_string(), ms, png })
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
fn save_impl(token: &str, rgba: &[u8], ms: u64) -> Option<Vec<u8>> {
    let value = web_encode(rgba, ms)?;
    let store = local_storage()?;
    // A full quota is an `Err`: the postcard is simply not kept.
    store.set_item(&format!("{WEB_KEY_PREFIX}{token}"), &value).ok()?;
    parse_web_value(&value).map(|(_, png)| png)
}

#[cfg(target_arch = "wasm32")]
fn newest_impl(token: &str) -> Option<Vec<u8>> {
    load_token(token).into_iter().next().map(|p| p.png)
}

#[cfg(target_arch = "wasm32")]
fn load_token(token: &str) -> Vec<Postcard> {
    let Some(value) = local_storage()
        .and_then(|s| s.get_item(&format!("{WEB_KEY_PREFIX}{token}")).ok().flatten())
    else {
        return Vec::new();
    };
    parse_web_value(&value)
        .map(|(ms, png)| vec![Postcard { token: token.to_string(), ms, png }])
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Fixtures (the `--shot-lobby --fixture-postcards` screenshots and tests only;
// never written to any store)
// ---------------------------------------------------------------------------

/// A made-up postcard picture for `token`: a sky, a sun, ground and a few
/// blocks in the shelf's colour. Deterministic per (token, variant).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn fixture_rgba(token: &str, variant: u32) -> Vec<u8> {
    let seed = token.bytes().fold(variant.wrapping_mul(2_654_435_761), |h, b| {
        h.wrapping_mul(31).wrapping_add(b as u32)
    });
    let shelf = row_of(token).map(|r| r.color_rgb()).unwrap_or([180, 160, 120]);
    let (w, h) = (POSTCARD_W as i32, POSTCARD_H as i32);
    let horizon = 108 + (seed % 24) as i32;
    let sun = (40 + (seed % 240) as i32, 30 + ((seed >> 8) % 30) as i32);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let px: [u8; 3] = if (x - sun.0).pow(2) + (y - sun.1).pow(2) < 14 * 14 {
                [255, 236, 170]
            } else if y < horizon {
                let t = y as f32 / horizon as f32;
                [(120.0 + 60.0 * t) as u8, (170.0 + 50.0 * t) as u8, (230.0 + 10.0 * t) as u8]
            } else {
                let checker = ((x / 16) + (y / 16)) % 2 == 0;
                if checker { [92, 150, 70] } else { [84, 138, 64] }
            };
            out.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    // A little stack of blocks in the shelf colour.
    let blocks = 2 + (seed % 4) as i32;
    let x0 = 90 + ((seed >> 4) % 120) as i32;
    for b in 0..blocks {
        let (bx, by) = (x0 + (b % 3) * 26, horizon - 24 - (b / 3) * 26);
        for y in by.max(0)..(by + 24).min(h) {
            for x in bx.max(0)..(bx + 24).min(w) {
                let edge = x == bx || y == by || x == bx + 23 || y == by + 23;
                let c = if edge {
                    [shelf[0] / 2, shelf[1] / 2, shelf[2] / 2]
                } else {
                    shelf
                };
                let p = ((y * w + x) * 4) as usize;
                out[p..p + 3].copy_from_slice(&c);
            }
        }
    }
    out
}

/// A set of fixture postcards: one or two for a handful of trials across the
/// shelves, as if they had been played.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn fixtures() -> Vec<Postcard> {
    let picks: [(&str, u32); 10] = [
        ("craft", 1),
        ("build", 2),
        ("mine", 1),
        ("harvest", 1),
        ("fish", 1),
        ("power", 1),
        ("kill", 1),
        ("sprint", 2),
        ("smelt", 1),
        ("scavenger", 1),
    ];
    let base: u64 = 1_790_000_000_000;
    let mut out = Vec::new();
    let mut step = 0u64;
    for (token, n) in picks {
        if !is_known_token(token) {
            continue;
        }
        for v in 0..n {
            step += 1;
            let rgba = fixture_rgba(token, v);
            if let Some(png) = encode_png(&rgba, POSTCARD_W, POSTCARD_H, 8) {
                out.push(Postcard { token: token.to_string(), ms: base + step * 3_600_000, png });
            }
        }
    }
    sort_newest_first(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(token: &str, ms: u64) -> Postcard {
        Postcard { token: token.to_string(), ms, png: Vec::new() }
    }

    #[test]
    fn every_trial_token_is_safe_and_unique_across_races_and_challenges() {
        let mut seen = std::collections::HashSet::new();
        for (_, r) in TRIAL_ORDER {
            let t = token_of(*r);
            assert!(is_safe_token(t), "{t} is not a safe postcard key");
            assert!(seen.insert(t), "{t} names two trials: their postcards would mix");
        }
    }

    #[test]
    fn tokens_resolve_from_a_race_id_and_a_challenge_display_name() {
        assert_eq!(token_for_race("sprint"), Some("sprint"));
        assert_eq!(token_for_race("not-a-race"), None);
        for (token, display) in crate::scenario::challenge_listing() {
            if is_known_token(token) {
                assert_eq!(token_for_challenge(&display), Some(token), "{display}");
            }
        }
        assert_eq!(token_for_challenge("No Such Trial"), None);
    }

    #[test]
    fn unsafe_tokens_are_refused() {
        for bad in ["", "../etc", "a/b", "Sprint", "ch:smelt", "a b", &"x".repeat(49)] {
            assert!(!is_safe_token(bad), "{bad:?}");
        }
        assert!(is_safe_token("cross-country"));
    }

    #[test]
    fn file_names_round_trip_and_ignore_strangers() {
        assert_eq!(file_name(1_790_000_000_123), "1790000000123.png");
        assert_eq!(parse_file_name("1790000000123.png"), Some(1_790_000_000_123));
        for other in ["x.png", ".png", "12.tmp", "12.PNG", "-3.png", "12.png.bak"] {
            assert_eq!(parse_file_name(other), None, "{other}");
        }
    }

    #[test]
    fn pruning_keeps_the_newest_five() {
        let times = [5, 1, 9, 3, 7, 2, 8];
        assert_eq!(to_prune(&times, KEEP_PER_TRIAL), vec![1, 2]);
        assert!(to_prune(&[4, 2], KEEP_PER_TRIAL).is_empty());
        assert_eq!(to_prune(&[1, 1, 2], 1), vec![1]);
    }

    #[test]
    fn the_album_groups_by_shelf_in_gallery_order_newest_first() {
        let cards = vec![
            card("sprint", 10),
            card("craft", 5),
            card("craft", 50),
            card("mystery", 99),
            card("mine", 20),
        ];
        let groups = album_groups(&cards);
        let kinds: Vec<RowKind> = groups.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds.first(), Some(&row_of("craft").unwrap()));
        assert_eq!(kinds.last(), Some(&RowKind::Clock));
        let make = &groups[0].1;
        let times: Vec<u64> = make.iter().map(|&i| cards[i].ms).collect();
        let mut sorted = times.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(times, sorted, "newest first within a shelf");
        assert!(groups.iter().all(|(_, idx)| idx.iter().all(|&i| cards[i].token != "mystery")));
        // The gallery's order: each group's row comes after the previous one.
        let order: Vec<RowKind> = row_specs().into_iter().map(|r| r.kind).collect();
        let pos: Vec<usize> = kinds.iter().map(|k| order.iter().position(|o| o == k).unwrap()).collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn newest_first() {
        let mut cards = vec![card("a", 1), card("b", 3), card("a", 2)];
        sort_newest_first(&mut cards);
        assert_eq!(cards.iter().map(|c| c.ms).collect::<Vec<_>>(), vec![3, 2, 1]);
    }

    #[test]
    fn crop_keeps_the_middle_at_the_postcard_aspect() {
        assert_eq!(crop_rect(1920, 1080, 320, 180), (0, 0, 1920, 1080));
        assert_eq!(crop_rect(1280, 800, 320, 180), (0, 40, 1280, 720));
        assert_eq!(crop_rect(1000, 400, 320, 180), (144, 0, 711, 400));
    }

    #[test]
    fn reduce_averages_and_rejects_a_wrong_buffer() {
        // 4×2 frame: left half black, right half white → 2×1 = black, white.
        let mut rgba = Vec::new();
        for _y in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                rgba.extend_from_slice(&[v, v, v, 7]);
            }
        }
        // 4×2 is 2:1, the output 2×1 is 2:1: no crop.
        let out = crop_and_reduce(&rgba, 4, 2, 2, 1).unwrap();
        assert_eq!(out, vec![0, 0, 0, 255, 255, 255, 255, 255]);
        assert!(crop_and_reduce(&rgba[..8], 4, 2, 2, 1).is_none());
    }

    #[test]
    fn base64_round_trips_and_refuses_junk() {
        for n in 0..20usize {
            let bytes: Vec<u8> = (0..n as u8).map(|b| b.wrapping_mul(37)).collect();
            let enc = base64_encode(&bytes);
            assert_eq!(enc.len(), base64_len(n));
            assert_eq!(base64_decode(&enc).unwrap(), bytes);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert!(base64_decode("abc").is_none());
        assert!(base64_decode("ab=c").is_none());
        assert!(base64_decode("a=b=").is_none());
        assert!(base64_decode("@@@@").is_none());
        let v = web_value(42, b"png!");
        assert_eq!(parse_web_value(&v), Some((42, b"png!".to_vec())));
    }

    #[test]
    fn the_web_budget_keeps_every_trial_well_inside_the_quota() {
        let trials = TRIAL_ORDER.len();
        let longest_key = TRIAL_ORDER.iter().map(|(_, r)| token_of(*r).len()).max().unwrap()
            + WEB_KEY_PREFIX.len();
        let worst = trials * (WEB_MAX_VALUE_CHARS + longest_key) * WEB_BYTES_PER_CHAR;
        assert!(trials >= 45, "the budget is planned for at least 45 trials");
        let at_45 = 45 * (WEB_MAX_VALUE_CHARS + longest_key) * WEB_BYTES_PER_CHAR;
        assert!(at_45 * 2 <= WEB_QUOTA_BYTES, "45 postcards take {at_45} bytes: over half the quota");
        assert!(worst * 2 <= WEB_QUOTA_BYTES + WEB_QUOTA_BYTES / 10, "{trials} postcards take {worst} bytes");
        // The ladder runs largest first and ends small.
        assert!(WEB_LADDER.windows(2).all(|w| w[0].0 * w[0].1 >= w[1].0 * w[1].1));
    }

    #[test]
    fn a_busy_postcard_still_fits_the_web_budget() {
        // Noise is the worst case for PNG: the ladder must still land one.
        let mut x = 12345u32;
        let rgba: Vec<u8> = (0..POSTCARD_W * POSTCARD_H * 4)
            .map(|i| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                if i % 4 == 3 { 255 } else { (x >> 16) as u8 }
            })
            .collect();
        // Pure noise may not fit at all; a fixture (a real-looking scene) must.
        let scene = fixture_rgba("craft", 0);
        let v = web_encode(&scene, 7).expect("a scene fits the web budget");
        assert!(v.len() <= WEB_MAX_VALUE_CHARS);
        let (ms, png) = parse_web_value(&v).unwrap();
        assert_eq!(ms, 7);
        let (w, h, _) = decode_png(&png).unwrap();
        assert!(WEB_LADDER.iter().any(|(lw, lh, _)| (*lw, *lh) == (w, h)));
        if let Some(v) = web_encode(&rgba, 1) {
            assert!(v.len() <= WEB_MAX_VALUE_CHARS);
        }
    }

    #[test]
    fn native_store_saves_prunes_and_reads_back() {
        let token = "smelt";
        let dir = token_dir(token);
        let _ = std::fs::remove_dir_all(&dir);
        let rgba = fixture_rgba(token, 0);
        for ms in 1..=7u64 {
            assert!(save(token, &rgba, ms).is_some());
        }
        let mut kept = times_in(&dir);
        kept.sort_unstable();
        assert_eq!(kept, vec![3, 4, 5, 6, 7], "only the newest five stay");
        let png = newest_png(token).expect("newest");
        assert_eq!(png, std::fs::read(dir.join(file_name(7))).unwrap());
        let (w, h, _) = decode_png(&png).unwrap();
        assert_eq!((w, h), (POSTCARD_W, POSTCARD_H));
        let all: Vec<Postcard> = load_all().into_iter().filter(|p| p.token == token).collect();
        assert_eq!(all.iter().map(|p| p.ms).collect::<Vec<_>>(), vec![7, 6, 5, 4, 3]);
        assert!(save("../evil", &rgba, 1).is_none());
        assert!(save("not-a-trial", &rgba, 1).is_none());
        assert!(newest_png("../evil").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fixtures_cover_several_shelves() {
        let f = fixtures();
        assert_eq!(album_groups(&f).len(), 5, "every shelf and the clock row");
        assert!(f.windows(2).all(|w| w[0].ms >= w[1].ms));
    }
}
