//! Pure paint engine for the 64×64 avatar skin buffer (Skin Studio, Phase 1b).
//! Every op takes the row-major RGBA buffer (`idx = (y*64 + x)*4`) and a
//! half-open `PixelRect` clamp so a brush/fill can never bleed out of the hit
//! face's UV island into a neighbouring body part.
//!
//! No rendering/GPU/egui here — given a tool + a target pixel + the face's rect,
//! it mutates the buffer. The 3D wiring (ray → pixel) lives in `skin_hit.rs` +
//! the render plan.

/// Pixel bounds of one paintable region, half-open: covers `x0 <= x < x1`,
/// `y0 <= y < y1`. (Same convention as `skin_uv::texel_for`'s interior clamp.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl PixelRect {
    fn contains(&self, x: i64, y: i64) -> bool {
        x >= self.x0 as i64 && x < self.x1 as i64 && y >= self.y0 as i64 && y < self.y1 as i64
    }
}

const W: u32 = 64;

#[inline]
fn idx(x: u32, y: u32) -> usize {
    ((y * W + x) * 4) as usize
}

/// Read the RGBA at `(x, y)`. Caller guarantees `x, y < 64`.
pub fn get_pixel(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = idx(x, y);
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

/// Write the RGBA at `(x, y)`. Caller guarantees `x, y < 64`.
pub fn set_pixel(buf: &mut [u8], x: u32, y: u32, c: [u8; 4]) {
    let i = idx(x, y);
    buf[i..i + 4].copy_from_slice(&c);
}

/// Paint a square brush of side `brush` (>=1) centred on `(cx, cy)`, writing
/// `color`. Pixels outside `rect` are skipped — the brush never bleeds across a
/// UV-island boundary. For `brush > 1` the square is centred (top-left offset by
/// `brush/2`).
pub fn pencil(buf: &mut [u8], cx: u32, cy: u32, brush: u32, color: [u8; 4], rect: &PixelRect) {
    let r = brush.max(1) as i64;
    let half = r / 2;
    for dy in 0..r {
        for dx in 0..r {
            let x = cx as i64 - half + dx;
            let y = cy as i64 - half + dy;
            if rect.contains(x, y) {
                set_pixel(buf, x as u32, y as u32, color);
            }
        }
    }
}

/// OVERLAY-layer eraser: paint fully-transparent black so the base skin shows
/// through (spec §6.5). This is correct ONLY for the overlay layer. The BASE
/// layer is never transparent — a base-layer "erase" must instead RESTORE the
/// default-skin pixel via `pencil(buf, .., default_rgba, rect)`, NOT this
/// function (calling `erase` on the base layer would punch a transparent hole).
/// The render layer holds the active layer + the default-skin buffer and routes
/// base-vs-overlay erase accordingly.
pub fn erase(buf: &mut [u8], cx: u32, cy: u32, brush: u32, rect: &PixelRect) {
    pencil(buf, cx, cy, brush, [0, 0, 0, 0], rect);
}

/// Sample `(x, y)` for the eyedropper tool: `Some(rgba)` when there is
/// something to pick (alpha > 0), `None` on a bare texel (alpha == 0 — e.g. an
/// unpainted clothes-layer texel). Bounds contract is the same as `get_pixel`
/// (caller guarantees `x, y < 64`) — this adds no extra clamping of its own.
///
/// Why this exists: `eyedropper`/`get_pixel` read the buffer verbatim, so
/// grabbing a colour off a bare clothes texel hands back `[0,0,0,0]`, and
/// `bind_to_layer` then forces that onto the BASE layer as **opaque black**
/// (alpha forced to 255) the next time it's used to paint — a surprising
/// "why did my skin turn black" bug, not the erase the player reached for.
/// The explicit eraser tools remain the only way to erase; a transparent
/// sample here just means "nothing to pick", so the caller leaves
/// `picked_color` unchanged and tells the player instead.
pub fn sample_for_eyedropper(buf: &[u8], x: u32, y: u32) -> Option<[u8; 4]> {
    let c = get_pixel(buf, x, y);
    if c[3] > 0 { Some(c) } else { None }
}

/// The base-layer eraser (spec §6.5: base is never transparent). Copies the
/// brush footprint from `source` (e.g. `texture_gen::default_skin_rgba()`) into
/// `buf`, per-pixel and clamped to `rect`. `source` must be the same 64×64 RGBA
/// layout as `buf`.
pub fn restore_from(buf: &mut [u8], cx: u32, cy: u32, brush: u32, source: &[u8], rect: &PixelRect) {
    let r = brush.max(1) as i64;
    let half = r / 2;
    for dy in 0..r {
        for dx in 0..r {
            let x = cx as i64 - half + dx;
            let y = cy as i64 - half + dy;
            if rect.contains(x, y) {
                let i = idx(x as u32, y as u32);
                if i + 4 <= buf.len() && i + 4 <= source.len() {
                    buf[i..i + 4].copy_from_slice(&source[i..i + 4]);
                }
            }
        }
    }
}

/// Bind a paint colour to `layer`, enforcing the module invariant that the BASE
/// layer is never transparent: a colour heading for the base always gets full
/// alpha. The overlay keeps whatever alpha it was handed — transparent there is
/// legitimate, it IS the clothes eraser.
///
/// The eyedropper is why this exists. It samples the working buffer verbatim, so
/// grabbing a colour off the (unpainted, fully transparent) clothes shell yields
/// `[0, 0, 0, 0]`, and `picked_color` survives a clothes/layer toggle. Without
/// this guard the next base stroke pencils alpha-0 texels into the body and
/// `fs_avatar` (`shader.wgsl`, `alpha < 0.5 -> discard`) renders a see-through
/// hole. Reported by Axolittle 2026-07-31: "the back of the arms on the skins
/// are trasparent".
pub fn bind_to_layer(color: [u8; 4], layer: crate::skin_uv::SkinLayer) -> [u8; 4] {
    match layer {
        crate::skin_uv::SkinLayer::Base => [color[0], color[1], color[2], 255],
        crate::skin_uv::SkinLayer::Overlay => color,
    }
}

/// Repair an existing skin that already carries the transparent-base defect:
/// force alpha 255 on every texel inside the 36 BASE face rects, leaving colour
/// and the whole overlay layer untouched. A skin that already honours the
/// invariant is unchanged.
///
/// Applied when a skin is opened in the painter, so a body punctured before
/// [`bind_to_layer`] existed heals on edit and Pin writes it back solid. Only
/// the editor does this — worn and imported skins are never rewritten behind
/// the player's back.
///
/// CLASSIC rects, whatever the skin's arm model: every slim arm rect sits
/// inside the union of that arm's classic ones, so healing the classic island
/// is a superset that also covers the slim one (and additionally solidifies the
/// 1-px column slim doesn't use, which nothing reads). Same reasoning as
/// `skin_layers::clothes_layer_has_content` — see its docs.
pub fn heal_base_opacity(buf: &mut [u8]) {
    const SIDE: u32 = 64;
    if buf.len() != (SIDE * SIDE * 4) as usize {
        return;
    }
    for part in 0..6 {
        for face in 0..6 {
            let Some((x0, y0, x1, y1)) = crate::skin_uv::face_rect_px(
                part,
                face,
                crate::skin_uv::SkinLayer::Base,
                crate::skin_uv::ArmModel::Classic,
            ) else {
                continue;
            };
            for y in y0..y1.min(SIDE) {
                for x in x0..x1.min(SIDE) {
                    buf[idx(x, y) + 3] = 255;
                }
            }
        }
    }
}

/// 4-connected flood fill: replace the contiguous region of the start pixel's
/// colour, reachable from `(x, y)` without leaving `rect`, with `color`. A no-op
/// when the start colour already equals `color` (also avoids an infinite loop).
pub fn fill(buf: &mut [u8], x: u32, y: u32, color: [u8; 4], rect: &PixelRect) {
    if !rect.contains(x as i64, y as i64) {
        return;
    }
    let target = get_pixel(buf, x, y);
    if target == color {
        return;
    }
    let mut stack: Vec<(u32, u32)> = vec![(x, y)];
    while let Some((px, py)) = stack.pop() {
        if !rect.contains(px as i64, py as i64) {
            continue;
        }
        if get_pixel(buf, px, py) != target {
            continue;
        }
        set_pixel(buf, px, py, color);
        if px + 1 < rect.x1 {
            stack.push((px + 1, py));
        }
        if px > rect.x0 {
            stack.push((px - 1, py));
        }
        if py + 1 < rect.y1 {
            stack.push((px, py + 1));
        }
        if py > rect.y0 {
            stack.push((px, py - 1));
        }
    }
}

// ── Skindex-parity tool set (2026-09-06) ──────────────────────────────────
//
// One enum, one meaning per click. The eraser used to be a `bool` alongside the
// brush; it is a TOOL, and so are fill and the three shading brushes, so they
// all live in one place and no two can be "on" at once.

/// Which tool the next click applies. `brush` (1..3) sizes the footprint of
/// every member of the brush family (Brush/Eraser/Lighten/Darken/Noise); Fill
/// ignores it and floods the aimed face rect instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintTool {
    /// Paint the active colour. With nothing held and no colour picked this
    /// still ERASES — "empty hand rubs out" predates the eraser tool and stays.
    Brush,
    /// Flood the aimed face rect's matching region with the active colour.
    Fill,
    /// Rub out: overlay → transparent, base → back to the default skin.
    Eraser,
    /// Nudge each texel 10% toward white.
    Lighten,
    /// Nudge each texel 10% toward black.
    Darken,
    /// Jitter each texel's brightness by up to ±12%.
    Noise,
}

impl PaintTool {
    /// The panel button / toast label.
    pub fn label(self) -> &'static str {
        match self {
            PaintTool::Brush => "Brush",
            PaintTool::Fill => "Fill",
            PaintTool::Eraser => "Eraser",
            PaintTool::Lighten => "Lighten",
            PaintTool::Darken => "Darken",
            PaintTool::Noise => "Noise",
        }
    }

    /// B walks the shading brushes: Brush → Lighten → Darken → Noise → Brush.
    /// From Fill or the Eraser it means "back to painting" rather than
    /// dropping the player into the middle of the cycle.
    pub fn cycle_shading(self) -> PaintTool {
        match self {
            PaintTool::Brush => PaintTool::Lighten,
            PaintTool::Lighten => PaintTool::Darken,
            PaintTool::Darken => PaintTool::Noise,
            PaintTool::Noise | PaintTool::Fill | PaintTool::Eraser => PaintTool::Brush,
        }
    }

    /// True for the tools that MODIFY the texel already there (rather than
    /// replacing it), which is exactly the set that must apply once per texel
    /// per stroke — see [`TouchedSet`].
    pub fn is_shading(self) -> bool {
        matches!(self, PaintTool::Lighten | PaintTool::Darken | PaintTool::Noise)
    }
}

/// Texels already shaded during the current stroke. Lighten/Darken/Noise
/// *modify* what is there, so without this holding the button on one spot
/// compounds to black/white and a mirrored hit that folds onto the same texel
/// (e.g. the centre column of a front face) applies twice in one click. The
/// session clears it on the mouse-down edge; every shading op consults it.
pub type TouchedSet = ahash::AHashSet<(u32, u32)>;

/// A tiny deterministic RNG for the Noise brush. Deliberately NOT the `rand`
/// crate: this must build for `wasm32-unknown-unknown` with no extra
/// dependency, and a seeded LCG makes the brush reproducible in tests.
#[derive(Clone, Copy, Debug)]
pub struct Lcg {
    state: u32,
}

impl Lcg {
    /// Numerical Recipes' LCG constants — plenty for texture jitter.
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// The next value in `0.0..1.0`.
    pub fn next_unit(&mut self) -> f32 {
        self.state = self.state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        // Take the high 24 bits so the low-order-bit weakness of an LCG
        // doesn't show up as a visible pattern on a 4x4 limb face.
        (self.state >> 8) as f32 / 16_777_216.0
    }
}

/// Walk the brush footprint, honouring `rect` and the per-stroke `touched`
/// guard, applying `f` to each surviving texel. Fully-transparent texels are
/// skipped: there is no colour there to modify, and shading `[0,0,0,0]` would
/// smear grey ghosts across the bare clothes layer.
fn shade_footprint(
    buf: &mut [u8],
    cx: u32,
    cy: u32,
    brush: u32,
    rect: &PixelRect,
    touched: &mut TouchedSet,
    mut f: impl FnMut([u8; 4]) -> [u8; 4],
) {
    let r = brush.max(1) as i64;
    let half = r / 2;
    for dy in 0..r {
        for dx in 0..r {
            let x = cx as i64 - half + dx;
            let y = cy as i64 - half + dy;
            if !rect.contains(x, y) {
                continue;
            }
            let (x, y) = (x as u32, y as u32);
            if !touched.insert((x, y)) {
                continue;
            }
            let c = get_pixel(buf, x, y);
            if c[3] == 0 {
                continue;
            }
            set_pixel(buf, x, y, f(c));
        }
    }
}

/// One lighten step on a single channel: `c + (255 - c) * 0.10`.
fn lift(c: u8) -> u8 {
    (c as f32 + (255.0 - c as f32) * 0.10).round().clamp(0.0, 255.0) as u8
}

/// One darken step on a single channel: `c * 0.90`.
fn drop_(c: u8) -> u8 {
    (c as f32 * 0.90).round().clamp(0.0, 255.0) as u8
}

/// Lighten brush — move every texel in the footprint 10% toward white. Alpha is
/// untouched and alpha-0 texels are skipped; at most once per texel per stroke.
pub fn lighten(
    buf: &mut [u8],
    cx: u32,
    cy: u32,
    brush: u32,
    rect: &PixelRect,
    touched: &mut TouchedSet,
) {
    shade_footprint(buf, cx, cy, brush, rect, touched, |c| {
        [lift(c[0]), lift(c[1]), lift(c[2]), c[3]]
    });
}

/// Darken brush — move every texel in the footprint 10% toward black. Same
/// alpha + once-per-stroke rules as [`lighten`].
pub fn darken(
    buf: &mut [u8],
    cx: u32,
    cy: u32,
    brush: u32,
    rect: &PixelRect,
    touched: &mut TouchedSet,
) {
    shade_footprint(buf, cx, cy, brush, rect, touched, |c| {
        [drop_(c[0]), drop_(c[1]), drop_(c[2]), c[3]]
    });
}

/// Maximum brightness jitter the Noise brush applies, as a fraction.
const NOISE_SPREAD: f32 = 0.12;

/// Noise brush — jitter each texel's brightness uniformly within ±12%. One
/// factor per texel (all three channels scale together, so the hue survives and
/// only the shade wobbles). Same alpha + once-per-stroke rules as [`lighten`].
pub fn noise(
    buf: &mut [u8],
    cx: u32,
    cy: u32,
    brush: u32,
    rect: &PixelRect,
    touched: &mut TouchedSet,
    rng: &mut Lcg,
) {
    shade_footprint(buf, cx, cy, brush, rect, touched, |c| {
        let k = 1.0 + (rng.next_unit() * 2.0 - 1.0) * NOISE_SPREAD;
        let jit = |v: u8| (v as f32 * k).round().clamp(0.0, 255.0) as u8;
        [jit(c[0]), jit(c[1]), jit(c[2]), c[3]]
    });
}

/// Bresenham line between two texels, inclusive of both ends, in order from
/// `a` to `b`. Shift+click paints along this so a kid can rule a straight edge
/// instead of trying to drag one freehand across a 4-pixel-wide arm.
///
/// Bounds are the caller's job — the paint pass clamps each texel to the face
/// rect, so a line can never leave the UV island it started in.
///
/// Symmetric by construction: plain Bresenham breaks its ties in the direction
/// of travel, so A→B and B→A can pick DIFFERENT texels on a shallow diagonal.
/// The run is always computed from the lexicographically smaller endpoint and
/// reversed if needed, so re-ruling the same edge backwards lands on the same
/// pixels instead of a second, slightly-offset line.
pub fn line_texels(a: (u32, u32), b: (u32, u32)) -> Vec<(u32, u32)> {
    if a > b {
        let mut v = line_texels(b, a);
        v.reverse();
        return v;
    }
    let (mut x, mut y) = (a.0 as i64, a.1 as i64);
    let (x1, y1) = (b.0 as i64, b.1 as i64);
    let dx = (x1 - x).abs();
    let sx = if x < x1 { 1 } else { -1 };
    let dy = -(y1 - y).abs();
    let sy = if y < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = Vec::new();
    loop {
        out.push((x as u32, y as u32));
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

/// Parse `#rrggbb` (or bare `rrggbb`, either case, surrounding space allowed)
/// into an RGB triple. `None` for anything else — the hex field leaves the
/// colour alone rather than fighting the player halfway through typing.
pub fn parse_hex_rgb(s: &str) -> Option<[u8; 3]> {
    let t = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Render a colour as the `#rrggbb` the hex field shows (alpha is not part of
/// the notation — the base layer is opaque and the overlay's transparency is
/// the eraser's job, not a colour you type).
pub fn hex_of(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// How many recently-used colours the panel keeps.
pub const RECENT_CAP: usize = 8;

/// Remember a colour the player just chose: most-recent-first, no duplicates
/// (re-picking moves it to the front), capped at [`RECENT_CAP`].
pub fn push_recent(recent: &mut Vec<[u8; 4]>, c: [u8; 4]) {
    recent.retain(|r| *r != c);
    recent.insert(0, c);
    recent.truncate(RECENT_CAP);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Skindex-parity tool set (2026-09-06) ───────────────────────────────

    #[test]
    fn b_cycles_the_shading_brushes_and_returns_to_brush() {
        assert_eq!(PaintTool::Brush.cycle_shading(), PaintTool::Lighten);
        assert_eq!(PaintTool::Lighten.cycle_shading(), PaintTool::Darken);
        assert_eq!(PaintTool::Darken.cycle_shading(), PaintTool::Noise);
        assert_eq!(PaintTool::Noise.cycle_shading(), PaintTool::Brush);
    }

    #[test]
    fn cycling_out_of_fill_or_eraser_lands_on_brush() {
        // B is the shading cycle; from a non-brush-family tool it means
        // "back to painting", not "skip to Darken".
        assert_eq!(PaintTool::Fill.cycle_shading(), PaintTool::Brush);
        assert_eq!(PaintTool::Eraser.cycle_shading(), PaintTool::Brush);
    }

    #[test]
    fn every_tool_has_a_kid_readable_label() {
        for t in [
            PaintTool::Brush,
            PaintTool::Fill,
            PaintTool::Eraser,
            PaintTool::Lighten,
            PaintTool::Darken,
            PaintTool::Noise,
        ] {
            assert!(!t.label().is_empty(), "{t:?} needs a label");
        }
    }

    // ── lighten / darken ──────────────────────────────────────────────────

    #[test]
    fn lighten_moves_a_texel_ten_percent_toward_white() {
        let mut b = blank();
        set_pixel(&mut b, 5, 5, [100, 0, 250, 255]);
        let mut touched = TouchedSet::default();
        lighten(&mut b, 5, 5, 1, &full_rect(), &mut touched);
        // c + (255 - c) * 0.10, rounded: 100→116, 0→26, 250→251
        assert_eq!(get_pixel(&b, 5, 5), [116, 26, 251, 255]);
    }

    #[test]
    fn darken_scales_a_texel_to_ninety_percent() {
        let mut b = blank();
        set_pixel(&mut b, 5, 5, [100, 10, 255, 255]);
        let mut touched = TouchedSet::default();
        darken(&mut b, 5, 5, 1, &full_rect(), &mut touched);
        // c * 0.90, rounded: 100→90, 10→9, 255→230
        assert_eq!(get_pixel(&b, 5, 5), [90, 9, 230, 255]);
    }

    #[test]
    fn shading_never_touches_alpha() {
        let mut b = blank();
        set_pixel(&mut b, 1, 1, [80, 80, 80, 137]);
        set_pixel(&mut b, 2, 1, [80, 80, 80, 137]);
        let mut touched = TouchedSet::default();
        lighten(&mut b, 1, 1, 1, &full_rect(), &mut touched);
        darken(&mut b, 2, 1, 1, &full_rect(), &mut touched);
        assert_eq!(get_pixel(&b, 1, 1)[3], 137, "lighten keeps alpha");
        assert_eq!(get_pixel(&b, 2, 1)[3], 137, "darken keeps alpha");
    }

    #[test]
    fn shading_skips_a_fully_transparent_texel() {
        // A bare clothes texel has no colour to shade — lifting [0,0,0,0]
        // toward white would smear grey ghosts over the empty overlay.
        let mut b = blank();
        let mut touched = TouchedSet::default();
        lighten(&mut b, 7, 7, 1, &full_rect(), &mut touched);
        darken(&mut b, 7, 7, 1, &full_rect(), &mut touched);
        assert_eq!(get_pixel(&b, 7, 7), [0, 0, 0, 0], "alpha-0 is left alone");
    }

    #[test]
    fn shading_is_clamped_to_the_face_rect() {
        let mut b = blank();
        for x in 6..12 {
            set_pixel(&mut b, x, 10, [100, 100, 100, 255]);
        }
        let rect = PixelRect { x0: 8, y0: 8, x1: 16, y1: 16 };
        let mut touched = TouchedSet::default();
        darken(&mut b, 8, 10, 3, &rect, &mut touched);
        assert_eq!(get_pixel(&b, 7, 10), [100, 100, 100, 255], "outside the rect untouched");
        assert_eq!(get_pixel(&b, 8, 10), [90, 90, 90, 255], "inside the rect darkened");
    }

    #[test]
    fn a_texel_is_shaded_at_most_once_per_stroke() {
        // Holding the button on one spot must not compound to black, and a
        // mirrored hit that lands on the same texel must not double-apply.
        let mut b = blank();
        set_pixel(&mut b, 5, 5, [100, 100, 100, 255]);
        let mut touched = TouchedSet::default();
        for _ in 0..10 {
            darken(&mut b, 5, 5, 1, &full_rect(), &mut touched);
        }
        assert_eq!(get_pixel(&b, 5, 5), [90, 90, 90, 255], "one step, not ten");
        // A NEW stroke clears the set and shades again.
        touched.clear();
        darken(&mut b, 5, 5, 1, &full_rect(), &mut touched);
        assert_eq!(get_pixel(&b, 5, 5), [81, 81, 81, 255], "next stroke shades again");
    }

    // ── noise ─────────────────────────────────────────────────────────────

    #[test]
    fn noise_is_reproducible_for_a_fixed_seed() {
        let mut a = blank();
        let mut b2 = blank();
        for buf in [&mut a, &mut b2] {
            set_pixel(buf, 5, 5, [128, 128, 128, 255]);
        }
        let mut ta = TouchedSet::default();
        let mut tb = TouchedSet::default();
        let mut ra = Lcg::new(7);
        let mut rb = Lcg::new(7);
        noise(&mut a, 5, 5, 1, &full_rect(), &mut ta, &mut ra);
        noise(&mut b2, 5, 5, 1, &full_rect(), &mut tb, &mut rb);
        assert_eq!(get_pixel(&a, 5, 5), get_pixel(&b2, 5, 5), "same seed → same jitter");
    }

    #[test]
    fn noise_stays_within_twelve_percent_and_keeps_alpha() {
        let mut b = blank();
        for x in 0..40u32 {
            set_pixel(&mut b, x, 3, [100, 100, 100, 200]);
        }
        let mut touched = TouchedSet::default();
        let mut rng = Lcg::new(1234);
        for x in 0..40u32 {
            noise(&mut b, x, 3, 1, &full_rect(), &mut touched, &mut rng);
        }
        let mut any_changed = false;
        for x in 0..40u32 {
            let c = get_pixel(&b, x, 3);
            assert_eq!(c[3], 200, "alpha preserved at x={x}");
            assert!((88..=112).contains(&c[0]), "x={x}: {} outside ±12%", c[0]);
            if c[0] != 100 {
                any_changed = true;
            }
        }
        assert!(any_changed, "noise must actually jitter something");
    }

    #[test]
    fn noise_skips_a_fully_transparent_texel() {
        let mut b = blank();
        let mut touched = TouchedSet::default();
        let mut rng = Lcg::new(3);
        noise(&mut b, 9, 9, 1, &full_rect(), &mut touched, &mut rng);
        assert_eq!(get_pixel(&b, 9, 9), [0, 0, 0, 0]);
    }

    #[test]
    fn noise_applies_once_per_texel_per_stroke() {
        let mut b = blank();
        set_pixel(&mut b, 4, 4, [128, 128, 128, 255]);
        let mut touched = TouchedSet::default();
        let mut rng = Lcg::new(11);
        noise(&mut b, 4, 4, 1, &full_rect(), &mut touched, &mut rng);
        let once = get_pixel(&b, 4, 4);
        for _ in 0..5 {
            noise(&mut b, 4, 4, 1, &full_rect(), &mut touched, &mut rng);
        }
        assert_eq!(get_pixel(&b, 4, 4), once, "re-hits in the same stroke are no-ops");
    }

    #[test]
    fn the_lcg_stays_in_the_unit_interval() {
        let mut rng = Lcg::new(0);
        for _ in 0..1000 {
            let v = rng.next_unit();
            assert!((0.0..1.0).contains(&v), "{v} out of 0..1");
        }
    }

    // ── straight line (Shift+click) ───────────────────────────────────────

    #[test]
    fn line_of_one_point_is_that_point() {
        assert_eq!(line_texels((3, 4), (3, 4)), vec![(3, 4)]);
    }

    #[test]
    fn horizontal_line_covers_every_texel_between() {
        assert_eq!(line_texels((2, 5), (6, 5)), vec![(2, 5), (3, 5), (4, 5), (5, 5), (6, 5)]);
    }

    #[test]
    fn vertical_line_covers_every_texel_between() {
        assert_eq!(line_texels((7, 1), (7, 4)), vec![(7, 1), (7, 2), (7, 3), (7, 4)]);
    }

    #[test]
    fn diagonal_line_steps_one_for_one() {
        assert_eq!(line_texels((0, 0), (3, 3)), vec![(0, 0), (1, 1), (2, 2), (3, 3)]);
    }

    #[test]
    fn a_line_is_symmetric_end_to_end() {
        let fwd = line_texels((1, 2), (9, 6));
        let mut back = line_texels((9, 6), (1, 2));
        back.reverse();
        assert_eq!(fwd, back, "the same texels either way round");
        assert_eq!(*fwd.first().unwrap(), (1, 2));
        assert_eq!(*fwd.last().unwrap(), (9, 6));
    }

    // ── hex entry ─────────────────────────────────────────────────────────

    #[test]
    fn hex_parses_with_and_without_the_hash() {
        assert_eq!(parse_hex_rgb("#ff8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex_rgb("FF8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex_rgb("  #00ff7f  "), Some([0, 255, 127]));
    }

    #[test]
    fn hex_rejects_anything_that_is_not_six_hex_digits() {
        for bad in ["", "#", "fff", "#12345", "#1234567", "#gg0000", "12 34 56"] {
            assert_eq!(parse_hex_rgb(bad), None, "{bad:?} must not parse");
        }
    }

    #[test]
    fn hex_of_round_trips_through_parse() {
        let s = hex_of([18, 52, 86, 255]);
        assert_eq!(s, "#123456");
        assert_eq!(parse_hex_rgb(&s), Some([18, 52, 86]));
    }

    // ── recent colours ────────────────────────────────────────────────────

    #[test]
    fn recent_colours_are_most_recent_first() {
        let mut r = Vec::new();
        push_recent(&mut r, [1, 1, 1, 255]);
        push_recent(&mut r, [2, 2, 2, 255]);
        assert_eq!(r, vec![[2, 2, 2, 255], [1, 1, 1, 255]]);
    }

    #[test]
    fn re_picking_a_colour_moves_it_to_the_front_without_duplicating() {
        let mut r = Vec::new();
        push_recent(&mut r, [1, 1, 1, 255]);
        push_recent(&mut r, [2, 2, 2, 255]);
        push_recent(&mut r, [1, 1, 1, 255]);
        assert_eq!(r, vec![[1, 1, 1, 255], [2, 2, 2, 255]], "moved to front, not duplicated");
    }

    #[test]
    fn recent_colours_are_capped() {
        let mut r = Vec::new();
        for i in 0..20u8 {
            push_recent(&mut r, [i, 0, 0, 255]);
        }
        assert_eq!(r.len(), RECENT_CAP);
        assert_eq!(r[0], [19, 0, 0, 255], "newest first");
        assert_eq!(r[RECENT_CAP - 1], [20 - RECENT_CAP as u8, 0, 0, 255], "oldest dropped");
    }


    fn blank() -> Vec<u8> {
        vec![0u8; 64 * 64 * 4]
    }

    fn full_rect() -> PixelRect {
        PixelRect { x0: 0, y0: 0, x1: 64, y1: 64 }
    }

    #[test]
    fn set_get_roundtrip() {
        let mut b = blank();
        set_pixel(&mut b, 10, 20, [1, 2, 3, 4]);
        assert_eq!(get_pixel(&b, 10, 20), [1, 2, 3, 4]);
        // neighbour untouched
        assert_eq!(get_pixel(&b, 11, 20), [0, 0, 0, 0]);
    }

    #[test]
    fn pencil_single_pixel() {
        let mut b = blank();
        pencil(&mut b, 5, 5, 1, [9, 9, 9, 255], &full_rect());
        assert_eq!(get_pixel(&b, 5, 5), [9, 9, 9, 255]);
        assert_eq!(get_pixel(&b, 6, 5), [0, 0, 0, 0]);
    }

    #[test]
    fn pencil_brush_three_is_centred_3x3() {
        let mut b = blank();
        pencil(&mut b, 5, 5, 3, [1, 1, 1, 1], &full_rect());
        // centred: covers x,y in 4..=6
        for y in 4..=6 {
            for x in 4..=6 {
                assert_eq!(get_pixel(&b, x, y), [1, 1, 1, 1], "({x},{y}) should be painted");
            }
        }
        assert_eq!(get_pixel(&b, 3, 5), [0, 0, 0, 0], "outside the 3x3");
        assert_eq!(get_pixel(&b, 7, 5), [0, 0, 0, 0], "outside the 3x3");
    }

    #[test]
    fn pencil_clamped_to_rect_no_bleed() {
        let mut b = blank();
        // rect covers only x in [8,16), y in [8,16) (a head-face-sized island).
        let rect = PixelRect { x0: 8, y0: 8, x1: 16, y1: 16 };
        // brush of 3 centred on the left edge of the rect — half spills left of x0.
        pencil(&mut b, 8, 10, 3, [5, 5, 5, 5], &rect);
        assert_eq!(get_pixel(&b, 7, 10), [0, 0, 0, 0], "must NOT paint outside the rect");
        assert_eq!(get_pixel(&b, 8, 10), [5, 5, 5, 5], "inside the rect is painted");
    }

    #[test]
    fn erase_sets_transparent() {
        let mut b = blank();
        set_pixel(&mut b, 3, 3, [200, 100, 50, 255]);
        erase(&mut b, 3, 3, 1, &full_rect());
        assert_eq!(get_pixel(&b, 3, 3), [0, 0, 0, 0]);
    }

    #[test]
    fn fill_within_rect_only() {
        let mut b = blank();
        let rect = PixelRect { x0: 0, y0: 0, x1: 4, y1: 4 };
        // Fill the 4x4 region (all transparent) with red.
        fill(&mut b, 0, 0, [255, 0, 0, 255], &rect);
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(get_pixel(&b, x, y), [255, 0, 0, 255], "({x},{y}) filled");
            }
        }
        // A pixel just outside the rect is untouched.
        assert_eq!(get_pixel(&b, 4, 0), [0, 0, 0, 0], "fill must not cross the rect boundary");
    }

    #[test]
    fn fill_stops_at_colour_boundary() {
        let mut b = blank();
        let rect = PixelRect { x0: 0, y0: 0, x1: 8, y1: 1 }; // a 1-pixel-tall strip
        // Put a different-coloured wall at x=4.
        set_pixel(&mut b, 4, 0, [1, 1, 1, 1]);
        // Fill starting at x=0 with green; should fill 0..4 but stop at the wall.
        fill(&mut b, 0, 0, [0, 255, 0, 255], &rect);
        for x in 0..4 {
            assert_eq!(get_pixel(&b, x, 0), [0, 255, 0, 255], "x={x} filled");
        }
        assert_eq!(get_pixel(&b, 4, 0), [1, 1, 1, 1], "wall pixel unchanged");
        assert_eq!(get_pixel(&b, 5, 0), [0, 0, 0, 0], "past the wall not filled");
    }

    #[test]
    fn fill_noop_when_same_colour() {
        let mut b = blank();
        let rect = PixelRect { x0: 0, y0: 0, x1: 2, y1: 2 };
        // Target colour equals the existing colour (transparent) — must not loop forever.
        fill(&mut b, 0, 0, [0, 0, 0, 0], &rect);
        assert_eq!(get_pixel(&b, 1, 1), [0, 0, 0, 0]);
    }

    #[test]
    fn restore_from_copies_source_within_rect() {
        let mut b = blank();
        // A "default" source: solid green everywhere.
        let source = vec![0u8; 64 * 64 * 4]
            .chunks(4)
            .flat_map(|_| [0u8, 255, 0, 255])
            .collect::<Vec<u8>>();
        // Paint a red pixel, then restore it from source → becomes green.
        set_pixel(&mut b, 5, 5, [255, 0, 0, 255]);
        let rect = PixelRect { x0: 0, y0: 0, x1: 64, y1: 64 };
        restore_from(&mut b, 5, 5, 1, &source, &rect);
        assert_eq!(get_pixel(&b, 5, 5), [0, 255, 0, 255], "restored from source");
    }

    // ── base-is-never-transparent (Axolittle 2026-07-31) ────────────────────

    /// A transparent colour bound for the BASE layer is forced opaque. This is
    /// the eyedropper hole-punch: grabbing a colour off the bare clothes shell
    /// hands the brush `[0,0,0,0]`, which `fs_avatar` would then discard.
    #[test]
    fn a_transparent_colour_is_forced_opaque_on_the_base_layer() {
        assert_eq!(
            bind_to_layer([12, 34, 56, 0], crate::skin_uv::SkinLayer::Base),
            [12, 34, 56, 255],
            "the base layer is never transparent — the colour survives, the hole does not"
        );
    }

    /// The overlay must keep its alpha: painting transparent there is how you
    /// rub clothes out, so the guard must not reach across and break it.
    #[test]
    fn the_overlay_layer_keeps_a_transparent_colour() {
        assert_eq!(
            bind_to_layer([12, 34, 56, 0], crate::skin_uv::SkinLayer::Overlay),
            [12, 34, 56, 0],
            "transparent on the clothes layer IS the eraser"
        );
    }

    /// An opaque colour is untouched on either layer.
    #[test]
    fn an_opaque_colour_passes_through_unchanged() {
        for layer in [crate::skin_uv::SkinLayer::Base, crate::skin_uv::SkinLayer::Overlay] {
            assert_eq!(bind_to_layer([9, 8, 7, 255], layer), [9, 8, 7, 255]);
        }
    }

    /// Every base face rect is healed, so an arm punctured before the guard
    /// existed comes back solid the moment it's opened in the painter.
    #[test]
    fn heal_restores_alpha_across_every_base_face() {
        let mut b = crate::texture_gen::default_skin_rgba();
        // Punch a hole in the middle of all 36 base faces.
        for part in 0..6 {
            for face in 0..6 {
                let (x, y) =
                    crate::skin_uv::texel_for(part, face, 0.5, 0.5, crate::skin_uv::SkinLayer::Base, crate::skin_uv::ArmModel::Classic)
                        .expect("valid part/face");
                let mut c = get_pixel(&b, x, y);
                c[3] = 0;
                set_pixel(&mut b, x, y, c);
            }
        }

        heal_base_opacity(&mut b);

        for part in 0..6 {
            for face in 0..6 {
                let (x, y) =
                    crate::skin_uv::texel_for(part, face, 0.5, 0.5, crate::skin_uv::SkinLayer::Base, crate::skin_uv::ArmModel::Classic)
                        .expect("valid part/face");
                assert_eq!(
                    get_pixel(&b, x, y)[3],
                    255,
                    "part {part} face {face} must be solid again"
                );
            }
        }
    }

    /// The specific report: the arm BACK faces. Right arm back is atlas
    /// px(52,20,4,12), left arm back px(44,52,4,12) — every texel of both.
    #[test]
    fn heal_restores_the_whole_of_both_arm_back_faces() {
        let mut b = crate::texture_gen::default_skin_rgba();
        let rects = [(52u32, 20u32), (44, 52)];
        for (x0, y0) in rects {
            for y in y0..y0 + 12 {
                for x in x0..x0 + 4 {
                    set_pixel(&mut b, x, y, [0, 0, 0, 0]);
                }
            }
        }

        heal_base_opacity(&mut b);

        for (x0, y0) in rects {
            for y in y0..y0 + 12 {
                for x in x0..x0 + 4 {
                    assert_eq!(
                        get_pixel(&b, x, y)[3],
                        255,
                        "arm back texel ({x},{y}) must be solid"
                    );
                }
            }
        }
    }

    /// Healing must not turn the (legitimately empty) clothes layer into a
    /// solid shell — that would wrap every skin in an invisible box.
    #[test]
    fn heal_leaves_the_clothes_layer_transparent() {
        let mut b = crate::texture_gen::default_skin_rgba();
        heal_base_opacity(&mut b);
        assert!(
            !crate::skin_layers::clothes_layer_has_content(&b),
            "the overlay must stay bare — healing is a base-layer repair only"
        );
    }

    /// Healing changes alpha only; the painted colours are the player's work.
    #[test]
    fn heal_preserves_colour() {
        let mut b = crate::texture_gen::default_skin_rgba();
        let (x, y) =
            crate::skin_uv::texel_for(3, 4, 0.5, 0.5, crate::skin_uv::SkinLayer::Base, crate::skin_uv::ArmModel::Classic).unwrap();
        set_pixel(&mut b, x, y, [200, 30, 40, 0]);
        heal_base_opacity(&mut b);
        assert_eq!(get_pixel(&b, x, y), [200, 30, 40, 255], "colour kept, alpha restored");
    }

    /// A wrong-sized buffer must be left alone rather than panic on a paint path.
    #[test]
    fn heal_ignores_a_wrong_sized_buffer() {
        let mut short = vec![0u8; 16];
        heal_base_opacity(&mut short);
        assert_eq!(short, vec![0u8; 16], "not a 64x64 skin — untouched");
    }

    // ── eyedropper: alpha-0 texels have nothing to pick (v0.2.19) ──────────

    /// A fully-transparent texel (the bare clothes shell) has nothing to
    /// pick — the eyedropper must not hand back `[0,0,0,0]`, which
    /// `bind_to_layer` would later force to opaque BLACK on the base layer.
    #[test]
    fn sample_for_eyedropper_returns_none_on_alpha_zero() {
        let mut b = blank();
        set_pixel(&mut b, 4, 4, [200, 30, 40, 0]);
        assert_eq!(sample_for_eyedropper(&b, 4, 4), None);
    }

    #[test]
    fn sample_for_eyedropper_returns_some_with_exact_bytes_at_alpha_one() {
        let mut b = blank();
        set_pixel(&mut b, 4, 4, [200, 30, 40, 1]);
        assert_eq!(sample_for_eyedropper(&b, 4, 4), Some([200, 30, 40, 1]));
    }

    #[test]
    fn sample_for_eyedropper_returns_some_with_exact_bytes_at_alpha_255() {
        let mut b = blank();
        set_pixel(&mut b, 4, 4, [200, 30, 40, 255]);
        assert_eq!(sample_for_eyedropper(&b, 4, 4), Some([200, 30, 40, 255]));
    }

    #[test]
    fn restore_from_respects_rect() {
        let mut b = blank();
        let source = vec![7u8; 64 * 64 * 4];
        let rect = PixelRect { x0: 8, y0: 8, x1: 16, y1: 16 };
        // brush of 3 at the rect's left edge — the spill-left pixel must stay blank.
        restore_from(&mut b, 8, 10, 3, &source, &rect);
        assert_eq!(get_pixel(&b, 7, 10), [0, 0, 0, 0], "outside the rect untouched");
        assert_eq!(get_pixel(&b, 8, 10), [7, 7, 7, 7], "inside the rect restored");
    }
}
