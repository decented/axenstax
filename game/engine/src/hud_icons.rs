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
/// atlas layer (the top face, the established icon), the layer cut to the block's
/// shape, or, for a block whose top face is no picture of it, a drawn icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockIcon {
    /// The block's `tex_top` layer, drawn full size.
    Layer(u32),
    /// A shaped block (slab, stairs, door, trapdoor, fence gate, wall, fence post): its
    /// base layer clipped to the shape's silhouette ([`icon_pieces`]), so it does not
    /// look like the full block of the same material.
    Shaped { layer: u32, shape: IconShape },
    /// The hand lantern: a frame with a lit window under a handle (`lantern`). Its
    /// top face is the opaque lid fill, which as an icon was a black square.
    Lantern,
}

/// The silhouettes a shaped block's icon is cut to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconShape {
    Slab,
    Stairs,
    Door,
    Trapdoor,
    FenceGate,
    Wall,
    /// A fence post: a post with a rail stub either side.
    Post,
}

/// A rectangle of the unit icon square, `[x0, y0, x1, y1]`, y down.
pub(crate) type UnitRect = [f32; 4];

/// The textured pieces of a shape's icon: each is the base layer's own pixels at the
/// same place (the texture is clipped to the piece, not squeezed into it), so the icon
/// is the material seen through the shape. Painted in order.
pub(crate) fn icon_pieces(shape: IconShape) -> &'static [UnitRect] {
    match shape {
        // The lower half.
        IconShape::Slab => &[[0.0, 0.5, 1.0, 1.0]],
        // The lower half and the upper back step.
        IconShape::Stairs => &[[0.0, 0.5, 1.0, 1.0], [0.5, 0.0, 1.0, 0.5]],
        // One tall narrow panel.
        IconShape::Door => &[[0.28, 0.04, 0.72, 0.96]],
        // A thin lid.
        IconShape::Trapdoor => &[[0.04, 0.60, 0.96, 0.80]],
        // Two posts with two rails between them and a gap between the rails.
        IconShape::FenceGate => &[
            [0.04, 0.16, 0.20, 0.92],
            [0.80, 0.16, 0.96, 0.92],
            [0.20, 0.30, 0.80, 0.42],
            [0.20, 0.62, 0.80, 0.74],
        ],
        // A tall post with a low wall running out either side.
        IconShape::Wall => &[[0.0, 0.50, 1.0, 0.88], [0.30, 0.14, 0.70, 0.94]],
        // A narrow post with rail stubs.
        IconShape::Post => &[[0.0, 0.26, 1.0, 0.38], [0.0, 0.60, 1.0, 0.72], [0.36, 0.04, 0.64, 0.96]],
    }
}

/// Small dark details drawn over a shape's pieces (a door's handle, a trapdoor's hinges).
pub(crate) fn icon_details(shape: IconShape) -> &'static [UnitRect] {
    match shape {
        IconShape::Door => &[[0.58, 0.48, 0.66, 0.56]],
        IconShape::Trapdoor => &[[0.10, 0.62, 0.20, 0.68], [0.80, 0.62, 0.90, 0.68]],
        IconShape::Slab | IconShape::Stairs | IconShape::FenceGate | IconShape::Wall | IconShape::Post => &[],
    }
}

/// The shape block `id`'s icon is cut to, if it is not a plain cube: the six building
/// families (every wood and stone variant: `block_shape::shape_of` knows them all) and
/// the species fence posts.
pub(crate) fn icon_shape(id: BlockId) -> Option<IconShape> {
    use crate::block_shape::BlockShape;
    if crate::block::is_fence_post(id) {
        return Some(IconShape::Post);
    }
    match crate::block_shape::shape_of(id) {
        BlockShape::Slab => Some(IconShape::Slab),
        BlockShape::Stairs => Some(IconShape::Stairs),
        BlockShape::Door => Some(IconShape::Door),
        BlockShape::Trapdoor => Some(IconShape::Trapdoor),
        BlockShape::FenceGate => Some(IconShape::FenceGate),
        BlockShape::Wall => Some(IconShape::Wall),
        _ => None,
    }
}

/// The wood species a fence post is made of.
fn fence_post_species(id: BlockId) -> Option<crate::block::WoodSpecies> {
    crate::block::ALL_WOOD_SPECIES.iter().copied().find(|s| crate::block::fence_post_for_species(*s) == id)
}

/// The icon of block `id` (the one question every slot painter asks).
pub(crate) fn block_icon(id: BlockId, registry: &BlockRegistry) -> BlockIcon {
    if id == crate::block::LANTERN {
        return BlockIcon::Lantern;
    }
    match icon_shape(id) {
        Some(shape) => {
            // Every species' fence post is built from the oak planks layer and
            // tinted; its icon shows its own species' planks instead.
            let layer = match fence_post_species(id) {
                Some(species) => registry.tex_top(crate::block::planks_block_for(species)),
                None => registry.tex_top(id),
            };
            BlockIcon::Shaped { layer, shape }
        }
        None => BlockIcon::Layer(registry.tex_top(id)),
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
        BlockIcon::Shaped { layer, shape } => {
            let unit = |r: UnitRect| Rect::from_min_max(point(rect, r[0], r[1]), point(rect, r[2], r[3]));
            let edge = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(20, 20, 24, 200));
            if let Some(tex_id) = texture(layer) {
                for piece in icon_pieces(shape) {
                    let uv = Rect::from_min_max(Pos2::new(piece[0], piece[1]), Pos2::new(piece[2], piece[3]));
                    painter.image(tex_id, unit(*piece), uv, Color32::WHITE);
                    painter.rect_stroke(unit(*piece), 0.0, edge, egui::StrokeKind::Inside);
                }
            }
            for detail in icon_details(shape) {
                painter.rect_filled(unit(*detail), 0.0, Color32::from_rgb(40, 34, 30));
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
    use crate::building_variants::{self as bv, VShape};

    #[test]
    fn the_lantern_is_drawn_and_every_plain_block_keeps_its_top_layer() {
        let reg = BlockRegistry::new();
        assert_eq!(block_icon(block::LANTERN, &reg), BlockIcon::Lantern);
        // Its top layer is the opaque lid fill (a black square as an icon): not the icon.
        assert_ne!(block_icon(block::LANTERN, &reg), BlockIcon::Layer(reg.tex_top(block::LANTERN)));
        for id in [block::STONE, block::TORCH, block::PAPER_LANTERN_YELLOW, block::GLASS, block::SPRUCE_PLANKS] {
            assert_eq!(block_icon(id, &reg), BlockIcon::Layer(reg.tex_top(id)), "block {id}");
        }
    }

    fn area(r: &UnitRect) -> f32 {
        (r[2] - r[0]) * (r[3] - r[1])
    }

    #[test]
    fn every_shape_has_its_own_silhouette_inside_the_icon() {
        let shapes = [
            IconShape::Slab,
            IconShape::Stairs,
            IconShape::Door,
            IconShape::Trapdoor,
            IconShape::FenceGate,
            IconShape::Wall,
            IconShape::Post,
        ];
        for (i, a) in shapes.iter().enumerate() {
            let pieces = icon_pieces(*a);
            assert!(!pieces.is_empty());
            let cover: f32 = pieces.iter().map(area).sum();
            assert!(cover < 0.9, "{a:?} is a shape, not the whole tile (covers {cover})");
            for r in pieces.iter().chain(icon_details(*a)) {
                assert!(r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= 1.0 && r[3] <= 1.0 && r[0] < r[2] && r[1] < r[3], "{a:?} {r:?}");
            }
            for b in &shapes[i + 1..] {
                assert_ne!(pieces, icon_pieces(*b), "{a:?} and {b:?} share a silhouette");
            }
        }
        // A slab is the lower half; a trapdoor is thinner than a slab; a door is taller than wide.
        assert_eq!(icon_pieces(IconShape::Slab), &[[0.0, 0.5, 1.0, 1.0]]);
        assert!(area(&icon_pieces(IconShape::Trapdoor)[0]) < area(&icon_pieces(IconShape::Slab)[0]) * 0.5);
        let door = icon_pieces(IconShape::Door)[0];
        assert!((door[3] - door[1]) > 2.0 * (door[2] - door[0]));
    }

    #[test]
    fn every_building_variant_is_shaped_and_not_the_icon_of_its_base_block() {
        let reg = BlockRegistry::new();
        let mut n = 0;
        for (id, material, shape) in bv::all() {
            n += 1;
            let want = match shape {
                VShape::Slab => IconShape::Slab,
                VShape::Stairs => IconShape::Stairs,
                VShape::Door => IconShape::Door,
                VShape::Trapdoor => IconShape::Trapdoor,
                VShape::FenceGate => IconShape::FenceGate,
                VShape::Wall => IconShape::Wall,
            };
            let icon = block_icon(id, &reg);
            assert_eq!(icon, BlockIcon::Shaped { layer: reg.tex_top(material.base), shape: want }, "{} ({id})", material.key);
            assert_ne!(icon, block_icon(material.base, &reg), "{} is not its base block's icon", material.key);
        }
        assert_eq!(n, 59, "the 59 slab/stair/door/trapdoor/gate/wall variants");
    }

    #[test]
    fn a_species_fence_post_shows_its_own_planks() {
        let reg = BlockRegistry::new();
        let mut layers = Vec::new();
        for s in block::ALL_WOOD_SPECIES {
            let post = block::fence_post_for_species(*s);
            let want = reg.tex_top(block::planks_block_for(*s));
            assert_eq!(block_icon(post, &reg), BlockIcon::Shaped { layer: want, shape: IconShape::Post }, "{s:?}");
            layers.push(want);
        }
        layers.sort_unstable();
        layers.dedup();
        assert_eq!(layers.len(), block::ALL_WOOD_SPECIES.len(), "seven species, seven different planks");
        assert_ne!(block_icon(block::BIRCH_FENCE_POST, &reg), block_icon(block::OAK_FENCE_POST, &reg));
    }

    #[test]
    fn spruce_planks_and_the_spruce_fence_gate_no_longer_share_an_icon() {
        let reg = BlockRegistry::new();
        let gate = bv::id_by_name("spruce_fence_gate").expect("the spruce fence gate");
        assert_ne!(block_icon(gate, &reg), block_icon(block::SPRUCE_PLANKS, &reg));
        assert_eq!(block_icon(block::OAK_FENCE_GATE, &reg), BlockIcon::Shaped { layer: reg.tex_top(block::OAK_PLANKS), shape: IconShape::FenceGate });
    }
}
