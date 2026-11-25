//! Code-native survival silhouettes, scaled inside existing HUD icon bounds.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

use crate::block::{BlockId, BlockRegistry};

fn point(rect: Rect, x: f32, y: f32) -> Pos2 {
    rect.min + egui::vec2(rect.width() * x, rect.height() * y)
}

/// Two convex halves keep the cleft crisp and preserve the half-heart reading.
pub(crate) fn heart(
    painter: &Painter,
    rect: Rect,
    empty: Color32,
    fill: Option<Color32>,
    half: bool,
) {
    let left: Vec<_> = [
        (0.5, 0.32),
        (0.35, 0.14),
        (0.17, 0.14),
        (0.05, 0.28),
        (0.05, 0.45),
        (0.17, 0.63),
        (0.5, 0.94),
    ]
    .into_iter()
    .map(|(x, y)| point(rect, x, y))
    .collect();
    let right: Vec<_> = left
        .iter()
        .map(|p| Pos2::new(2.0 * rect.center().x - p.x, p.y))
        .collect();
    painter.add(Shape::convex_polygon(
        left.clone(),
        fill.unwrap_or(empty),
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        right.clone(),
        if half { empty } else { fill.unwrap_or(empty) },
        Stroke::NONE,
    ));
    let mut outline = left;
    outline.extend(right.into_iter().rev().skip(1).take(5));
    painter.add(Shape::closed_line(
        outline,
        Stroke::new(1.0_f32, Color32::from_rgb(145, 103, 96)),
    ));
}

/// Drumstick with a separate bone; a depleted icon retains its visible outline.
pub(crate) fn drumstick(
    painter: &Painter,
    rect: Rect,
    empty: Color32,
    fill: Option<Color32>,
    half: bool,
) {
    let body: Vec<_> = [
        (0.10, 0.28),
        (0.28, 0.10),
        (0.52, 0.10),
        (0.72, 0.27),
        (0.72, 0.48),
        (0.47, 0.72),
        (0.24, 0.69),
        (0.10, 0.51),
    ]
    .into_iter()
    .map(|(x, y)| point(rect, x, y))
    .collect();
    let bone = if fill.is_some() && !half {
        Color32::from_rgb(235, 222, 194)
    } else {
        Color32::from_rgb(139, 127, 110)
    };
    let a = point(rect, 0.57, 0.57);
    let b = point(rect, 0.81, 0.81);
    let width = rect.width() * 0.17;
    painter.line_segment(
        [a, b],
        Stroke::new(width + 2.0, Color32::from_rgb(25, 29, 26)),
    );
    painter.line_segment([a, b], Stroke::new(width, bone));
    for (x, y) in [(0.85, 0.76), (0.76, 0.85)] {
        painter.circle(
            point(rect, x, y),
            rect.width() * 0.10,
            bone,
            Stroke::new(1.0_f32, Color32::from_rgb(25, 29, 26)),
        );
    }
    painter.add(Shape::convex_polygon(body.clone(), empty, Stroke::NONE));
    if let Some(colour) = fill {
        let clip = if half {
            Rect::from_min_max(rect.min, Pos2::new(rect.center().x, rect.bottom()))
        } else {
            rect
        };
        painter
            .with_clip_rect(painter.clip_rect().intersect(clip))
            .add(Shape::convex_polygon(body.clone(), colour, Stroke::NONE));
    }
    painter.add(Shape::closed_line(
        body,
        Stroke::new(1.0_f32, Color32::from_rgb(158, 129, 90)),
    ));
}

/// What an inventory slot, the hotbar and the off-hand box show for a block: its
/// atlas layer (the top face, the established icon) or, for a block whose top face
/// is no picture of it, a drawn icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockIcon {
    /// The block's `tex_top` layer, drawn full size.
    Layer(u32),
    /// The hand lantern: a frame with a lit window under a handle (`lantern`). Its
    /// top face is the opaque lid fill, which as an icon was a black square.
    Lantern,
    /// An atlas layer drawn tinted by `[r, g, b]` (multiplied): Cobbled Deepslate,
    /// whose faces are the cobblestone layer the shader darkens (`mesh::DARK_FLAG`).
    Tinted(u32, [u8; 3]),
}

/// The slot tint matching the shader's `DEEPSLATE_SHADE` (0.42, 0.42, 0.47).
pub(crate) const DEEPSLATE_ICON_TINT: [u8; 3] = [107, 107, 120];

/// The icon of block `id` (the one question every slot painter asks).
pub(crate) fn block_icon(id: BlockId, registry: &BlockRegistry) -> BlockIcon {
    if id == crate::block::LANTERN {
        BlockIcon::Lantern
    } else if crate::mesh::layer_flags(id) & crate::mesh::DARK_FLAG != 0 {
        BlockIcon::Tinted(registry.tex_top(id), DEEPSLATE_ICON_TINT)
    } else {
        BlockIcon::Layer(registry.tex_top(id))
    }
}

/// Paint block `id`'s icon into `rect`. `texture` is where an atlas layer's egui
/// texture is (`None` when it has not been uploaded: a headless pass draws nothing
/// for a layer icon).
pub(crate) fn paint_block_icon(
    painter: &Painter,
    rect: Rect,
    id: BlockId,
    registry: &BlockRegistry,
    texture: impl Fn(u32) -> Option<egui::TextureId>,
) {
    match block_icon(id, registry) {
        BlockIcon::Layer(layer) => {
            if let Some(tex_id) = texture(layer) {
                painter.image(
                    tex_id,
                    rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        BlockIcon::Tinted(layer, [r, g, b]) => {
            if let Some(tex_id) = texture(layer) {
                painter.image(
                    tex_id,
                    rect,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::from_rgb(r, g, b),
                );
            }
        }
        BlockIcon::Lantern => lantern(painter, rect),
    }
}

/// The hand lantern: a bail over a grey cap, an iron frame round a lit window, a
/// base plate. Code-drawn from the model's own colours (black frame, grey cap,
/// yellow core: `lantern::body`), so it needs no atlas layer and reads at hotbar size.
pub(crate) fn lantern(painter: &Painter, rect: Rect) {
    let frame = Color32::from_rgb(92, 86, 78);
    let frame_dark = Color32::from_rgb(40, 38, 36);
    let cap = Color32::from_rgb(150, 152, 160);
    let glow = Color32::from_rgb(255, 214, 92);
    let core = Color32::from_rgb(255, 242, 178);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(point(rect, x0, y0), point(rect, x1, y1));
    let h = rect.height();
    // The bail: two uprights and a bar across them.
    let bail = Stroke::new((h * 0.07).max(1.5), frame);
    painter.add(Shape::line(
        vec![point(rect, 0.36, 0.26), point(rect, 0.36, 0.08), point(rect, 0.64, 0.08), point(rect, 0.64, 0.26)],
        bail,
    ));
    // Cap, then the frame body with the lit window and its brighter core, then the base.
    painter.rect_filled(r(0.26, 0.24, 0.74, 0.34), 1.0, cap);
    painter.rect_filled(r(0.20, 0.34, 0.80, 0.88), 1.5, frame);
    painter.rect_filled(r(0.30, 0.40, 0.70, 0.82), 1.0, glow);
    painter.rect_filled(r(0.38, 0.48, 0.62, 0.74), 1.0, core);
    painter.rect_filled(r(0.22, 0.86, 0.78, 0.94), 1.0, frame_dark);
    painter.rect_stroke(r(0.20, 0.34, 0.80, 0.88), 1.5, Stroke::new(1.0_f32, frame_dark), egui::StrokeKind::Inside);
}

#[cfg(test)]
mod block_icon_tests {
    use super::*;
    use crate::block::{self, BlockRegistry};

    #[test]
    fn the_lantern_is_drawn_and_every_other_block_keeps_its_top_layer() {
        let reg = BlockRegistry::new();
        assert_eq!(block_icon(block::LANTERN, &reg), BlockIcon::Lantern);
        // Its top layer is the opaque lid fill (a black square as an icon): not the icon.
        assert_ne!(block_icon(block::LANTERN, &reg), BlockIcon::Layer(reg.tex_top(block::LANTERN)));
        for id in [block::STONE, block::TORCH, block::PAPER_LANTERN_YELLOW, block::GLASS, block::COBBLESTONE] {
            assert_eq!(block_icon(id, &reg), BlockIcon::Layer(reg.tex_top(id)), "block {id}");
        }
        // pt1124 — Cobbled Deepslate: the cobblestone layer, tinted as the shader darkens it.
        assert_eq!(
            block_icon(block::COBBLED_DEEPSLATE, &reg),
            BlockIcon::Tinted(block::TEX_COBBLESTONE, DEEPSLATE_ICON_TINT)
        );
    }
}
