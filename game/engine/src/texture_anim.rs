//! Animated textures (Spec 03 §3.4 / §11.3) — vertical-strip frames advanced by
//! **game time** (so they pause when the game pauses), uploaded one atlas layer
//! at a time via `queue.write_texture`.
//!
//! This module is the pure, cross-platform core: parse the `<key>.png.anim`
//! sidecar, slice a vertical strip into square frames, build a playback schedule,
//! and select the active frame from a game-tick clock. The renderer holds the
//! resulting [`AnimatedTexture`]s and re-uploads the selected frame each render
//! frame. No GPU or filesystem here — the native pack loader and the WASM fetch
//! path both feed decoded RGBA in.
//!
//! Texture-pack spec PHASE 5 (`docs/foundations/2026-06-18-texture-pack-authoring.md`).

use serde::Deserialize;

fn default_frame_time() -> u32 {
    2 // ticks per frame (0.1s @ 20 TPS) when a frame declares no explicit time
}

/// One entry in an explicit animation schedule (Minecraft-style): which strip
/// frame shows, and (optionally) for how many game ticks — overriding the
/// pack-wide [`AnimMeta::frame_time`].
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AnimFrameSpec {
    pub index: u32,
    #[serde(default)]
    pub time: Option<u32>,
}

/// `<key>.png.anim` sidecar (Spec 03 §3.4). Every field defaults, so a minimal
/// `{}` (or an absent/garbage file) yields a sane animation: each strip frame in
/// order, [`AnimMeta::frame_time`] ticks each.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AnimMeta {
    /// Ticks per frame for frames with no explicit `time`. Sanitised to ≥ 1.
    #[serde(default = "default_frame_time")]
    pub frame_time: u32,
    /// Explicit frame order + per-frame timing. Empty ⇒ `0..frame_count` in order.
    #[serde(default)]
    pub frames: Vec<AnimFrameSpec>,
    /// Cross-fade between frames. Parsed for forward-compat; v1 renders the
    /// nearest frame (interpolation deferred).
    #[serde(default)]
    pub interpolate: bool,
}

impl Default for AnimMeta {
    fn default() -> Self {
        Self { frame_time: default_frame_time(), frames: Vec::new(), interpolate: false }
    }
}

impl AnimMeta {
    /// Parse a `.anim` sidecar leniently: bad/empty JSON yields the default
    /// animation rather than failing the pack load.
    pub fn parse(text: &str) -> Self {
        serde_json::from_str::<AnimMeta>(text).unwrap_or_default()
    }
}

/// Frames in a vertical strip: `strip_height / frame_width` when the strip is a
/// clean stack of squares, else 1 (a square or ragged image is a single frame).
pub fn frame_count(strip_height: u32, frame_width: u32) -> u32 {
    if frame_width == 0 || !strip_height.is_multiple_of(frame_width) {
        return 1;
    }
    (strip_height / frame_width).max(1)
}

/// Slice a `frame_width`-wide vertical strip RGBA buffer into `count` square
/// `frame_width × frame_width` frames, top to bottom. Trailing bytes that don't
/// complete a frame are ignored; a short buffer yields as many whole frames as fit.
pub fn slice_vertical_strip(rgba: &[u8], frame_width: u32, count: u32) -> Vec<Vec<u8>> {
    let frame_bytes = (frame_width * frame_width * 4) as usize;
    if frame_bytes == 0 {
        return Vec::new();
    }
    (0..count as usize)
        .filter_map(|i| rgba.get(i * frame_bytes..(i + 1) * frame_bytes).map(<[u8]>::to_vec))
        .collect()
}

/// The first (top) `frame_width × frame_width` frame of a strip — the static
/// image the base atlas layer holds before animation kicks in.
pub fn first_frame(rgba: &[u8], frame_width: u32) -> Vec<u8> {
    let frame_bytes = (frame_width * frame_width * 4) as usize;
    rgba.get(..frame_bytes).map(<[u8]>::to_vec).unwrap_or_default()
}

/// Build the playback schedule `(frame_index, ticks)` from the meta and the
/// number of sliced frames. Explicit frames with an out-of-range `index` are
/// dropped; per-frame `time` (when > 0) overrides `frame_time`. An empty or
/// fully-invalid `frames` list falls back to `0..frame_count`, `frame_time` each.
pub fn build_schedule(meta: &AnimMeta, frame_count: u32) -> Vec<(u32, u32)> {
    let ft = meta.frame_time.max(1);
    let explicit: Vec<(u32, u32)> = meta
        .frames
        .iter()
        .filter(|f| f.index < frame_count)
        .map(|f| (f.index, f.time.filter(|t| *t > 0).unwrap_or(ft)))
        .collect();
    if explicit.is_empty() {
        return (0..frame_count).map(|i| (i, ft)).collect();
    }
    explicit
}

/// An animated atlas texture: a stack of equal-size RGBA frames bound to one
/// texture-array layer, with a game-tick playback schedule. Pure data — the
/// renderer re-uploads `pixels_at(tick)` to `layer_index` when the frame changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimatedTexture {
    pub layer_index: u32,
    /// Each frame is `res × res` RGBA at the atlas resolution.
    pub frames: Vec<Vec<u8>>,
    /// `(index-into-frames, ticks)` playback steps, cycled.
    pub schedule: Vec<(u32, u32)>,
}

impl AnimatedTexture {
    /// The frame index (into [`Self::frames`]) active at game tick `tick`, cycling
    /// over the schedule. Returns 0 for an empty/degenerate animation.
    pub fn frame_index_at(&self, tick: u64) -> u32 {
        let total: u64 = self.schedule.iter().map(|(_, t)| *t as u64).sum();
        if total == 0 || self.schedule.is_empty() {
            return 0;
        }
        let mut t = tick % total;
        for (frame, ticks) in &self.schedule {
            let ticks = *ticks as u64;
            if t < ticks {
                return *frame;
            }
            t -= ticks;
        }
        self.schedule.last().map(|(f, _)| *f).unwrap_or(0)
    }

    /// The RGBA pixels of the frame active at `tick`. Empty slice if degenerate.
    pub fn pixels_at(&self, tick: u64) -> &[u8] {
        self.frames
            .get(self.frame_index_at(tick) as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// True only when there's something to animate (≥ 2 frames and ≥ 2 steps).
    /// No production caller yet — the renderer always re-uploads
    /// `pixels_at(tick)` unconditionally; exercised by tests here and in
    /// `texture_registry`'s pack-loading tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1 && self.schedule.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `w`-wide vertical strip from per-frame solid colours.
    fn strip(frame_w: u32, colours: &[[u8; 4]]) -> Vec<u8> {
        let mut out = Vec::new();
        for c in colours {
            for _ in 0..(frame_w * frame_w) {
                out.extend_from_slice(c);
            }
        }
        out
    }

    #[test]
    fn anim_meta_parses_defaults_and_is_lenient() {
        let d = AnimMeta::parse("{}");
        assert_eq!(d.frame_time, 2);
        assert!(d.frames.is_empty());
        assert!(!d.interpolate);
        // Garbage / empty → defaults, never a parse failure.
        assert_eq!(AnimMeta::parse("not json"), AnimMeta::default());
        assert_eq!(AnimMeta::parse(""), AnimMeta::default());
    }

    #[test]
    fn anim_meta_parses_explicit_fields() {
        let m = AnimMeta::parse(
            r#"{ "frame_time": 4, "interpolate": true,
                 "frames": [ {"index":0,"time":2}, {"index":1}, {"index":2} ] }"#,
        );
        assert_eq!(m.frame_time, 4);
        assert!(m.interpolate);
        assert_eq!(m.frames.len(), 3);
        assert_eq!(m.frames[0].time, Some(2));
        assert_eq!(m.frames[1].time, None);
    }

    #[test]
    fn frame_count_divides_strip_else_one() {
        assert_eq!(frame_count(80, 16), 5, "clean stack of 5");
        assert_eq!(frame_count(16, 16), 1, "square is a single frame");
        assert_eq!(frame_count(17, 16), 1, "ragged → single frame");
        assert_eq!(frame_count(80, 0), 1, "zero width guarded");
    }

    #[test]
    fn slice_vertical_strip_yields_each_frame_in_order() {
        let s = strip(1, &[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]);
        let frames = slice_vertical_strip(&s, 1, 3);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0], vec![255, 0, 0, 255], "top frame = red");
        assert_eq!(frames[2], vec![0, 0, 255, 255], "bottom frame = blue");
    }

    #[test]
    fn first_frame_is_the_top_square() {
        let s = strip(1, &[[9, 9, 9, 9], [1, 1, 1, 1]]);
        assert_eq!(first_frame(&s, 1), vec![9, 9, 9, 9]);
    }

    #[test]
    fn build_schedule_defaults_to_all_frames_in_order() {
        let meta = AnimMeta { frame_time: 3, frames: vec![], interpolate: false };
        assert_eq!(build_schedule(&meta, 4), vec![(0, 3), (1, 3), (2, 3), (3, 3)]);
    }

    #[test]
    fn build_schedule_honours_explicit_order_drops_invalid_indices() {
        let meta = AnimMeta {
            frame_time: 3,
            frames: vec![
                AnimFrameSpec { index: 2, time: Some(5) },
                AnimFrameSpec { index: 9, time: None }, // out of range → dropped
                AnimFrameSpec { index: 0, time: None }, // no time → frame_time
            ],
            interpolate: false,
        };
        assert_eq!(build_schedule(&meta, 3), vec![(2, 5), (0, 3)]);
    }

    #[test]
    fn build_schedule_all_invalid_falls_back_to_default() {
        let meta = AnimMeta {
            frame_time: 4,
            frames: vec![AnimFrameSpec { index: 7, time: Some(1) }],
            interpolate: false,
        };
        assert_eq!(build_schedule(&meta, 2), vec![(0, 4), (1, 4)]);
    }

    #[test]
    fn frame_index_cycles_over_the_schedule_by_game_tick() {
        let anim = AnimatedTexture {
            layer_index: 11,
            frames: vec![vec![0; 4], vec![1; 4]],
            schedule: vec![(0, 2), (1, 3)], // total 5
        };
        assert_eq!(anim.frame_index_at(0), 0);
        assert_eq!(anim.frame_index_at(1), 0);
        assert_eq!(anim.frame_index_at(2), 1, "second step starts at tick 2");
        assert_eq!(anim.frame_index_at(4), 1);
        assert_eq!(anim.frame_index_at(5), 0, "cycle wraps at the total");
        assert_eq!(anim.frame_index_at(7), 1, "7 % 5 == 2 → second step");
        assert_eq!(anim.pixels_at(2), &[1, 1, 1, 1]);
        assert!(anim.is_animated());
    }

    #[test]
    fn degenerate_animation_is_safe() {
        let one = AnimatedTexture { layer_index: 0, frames: vec![vec![7; 4]], schedule: vec![(0, 1)] };
        assert_eq!(one.frame_index_at(123), 0);
        assert!(!one.is_animated(), "single frame is not animated");
        let empty = AnimatedTexture { layer_index: 0, frames: vec![], schedule: vec![] };
        assert_eq!(empty.frame_index_at(1), 0);
        assert_eq!(empty.pixels_at(1), &[] as &[u8]);
    }
}
