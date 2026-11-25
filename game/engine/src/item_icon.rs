//! The one inventory icon painter: what any slot (inventory grid, chest, crafting
//! grid, hotbar, off-hand box, cursor stack, recipe card, equipment column) draws for
//! an [`Item`] (Spec 03 §5.10 "Inventory icons").
//!
//! * a block: its atlas layer, or its shape cut from that layer (`hud_icons`);
//! * a tool: the held model's own shape, flattened ([`crate::held_tool_model::icon_quads`])
//!   and tinted by the material's head colour; the shield keeps its drawn board;
//! * armour: a silhouette per slot ([`armour_pieces`]) tinted by the material;
//! * a Plan: a rolled blueprint in the colour of its develop stage;
//! * a material: its own texture layer (the log materials, the tinted log).
//!
//! Everything is painted from existing layers and egui shapes: no texture layer is
//! added (the block array is full). Four painters used to carry their own copy of the
//! tool, material, Plan and armour fallback (a flat square and a letter); they all
//! call [`paint_item_icon`] now.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

use crate::armour::{ArmourMaterial, ArmourSlot};
use crate::block::BlockRegistry;
use crate::craft_ui::IconSource;
use crate::item::Item;

/// The multiplier colour of a log material's icon tint ([`crate::item::log_material_icon`]).
fn log_tint_color(tint: [f32; 3]) -> Color32 {
    to_color(tint)
}

fn to_color(c: [f32; 3]) -> Color32 {
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}

/// The colour an armour material is drawn in (the slot column, the grid and the
/// hotbar agree on it).
pub(crate) fn armour_material_color(mat: ArmourMaterial) -> [f32; 3] {
    match mat {
        ArmourMaterial::Leather => [0.55, 0.35, 0.18],
        ArmourMaterial::Iron => [0.75, 0.75, 0.78],
        ArmourMaterial::Diamond => [0.40, 0.92, 0.90],
        ArmourMaterial::Satori => [0.95, 0.55, 0.18],
        ArmourMaterial::Chainmail => [0.55, 0.55, 0.60],
        ArmourMaterial::Rubber => [0.42, 0.30, 0.22],
    }
}

/// One convex piece of an armour silhouette, in a unit square (x right, y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ArmourPiece {
    pub points: &'static [[f32; 2]],
    /// Drawn in the darker accent shade (a visor band, a belt, a cuff).
    pub accent: bool,
}

const fn piece(points: &'static [[f32; 2]]) -> ArmourPiece {
    ArmourPiece { points, accent: false }
}

const fn accent(points: &'static [[f32; 2]]) -> ArmourPiece {
    ArmourPiece { points, accent: true }
}

const HELMET: &[ArmourPiece] = &[
    piece(&[[0.20, 0.42], [0.22, 0.22], [0.32, 0.13], [0.68, 0.13], [0.78, 0.22], [0.80, 0.42]]),
    piece(&[[0.20, 0.42], [0.36, 0.42], [0.36, 0.84], [0.25, 0.84], [0.20, 0.70]]),
    piece(&[[0.64, 0.42], [0.80, 0.42], [0.80, 0.70], [0.75, 0.84], [0.64, 0.84]]),
    accent(&[[0.20, 0.36], [0.80, 0.36], [0.80, 0.44], [0.20, 0.44]]),
    accent(&[[0.46, 0.44], [0.54, 0.44], [0.54, 0.68], [0.46, 0.68]]),
];

const CHESTPLATE: &[ArmourPiece] = &[
    piece(&[[0.06, 0.24], [0.28, 0.14], [0.28, 0.46], [0.09, 0.52]]),
    piece(&[[0.94, 0.24], [0.72, 0.14], [0.72, 0.46], [0.91, 0.52]]),
    piece(&[[0.28, 0.22], [0.72, 0.22], [0.74, 0.88], [0.26, 0.88]]),
    piece(&[[0.26, 0.12], [0.42, 0.12], [0.42, 0.24], [0.26, 0.24]]),
    piece(&[[0.58, 0.12], [0.74, 0.12], [0.74, 0.24], [0.58, 0.24]]),
    accent(&[[0.26, 0.70], [0.74, 0.70], [0.74, 0.78], [0.26, 0.78]]),
    accent(&[[0.48, 0.26], [0.52, 0.26], [0.52, 0.70], [0.48, 0.70]]),
];

const LEGGINGS: &[ArmourPiece] = &[
    piece(&[[0.20, 0.12], [0.80, 0.12], [0.80, 0.32], [0.20, 0.32]]),
    piece(&[[0.20, 0.32], [0.49, 0.32], [0.47, 0.92], [0.22, 0.92]]),
    piece(&[[0.51, 0.32], [0.80, 0.32], [0.78, 0.92], [0.53, 0.92]]),
    accent(&[[0.20, 0.22], [0.80, 0.22], [0.80, 0.27], [0.20, 0.27]]),
];

const BOOTS: &[ArmourPiece] = &[
    piece(&[[0.08, 0.16], [0.40, 0.16], [0.40, 0.66], [0.08, 0.66]]),
    piece(&[[0.04, 0.66], [0.44, 0.66], [0.48, 0.80], [0.48, 0.86], [0.04, 0.86]]),
    accent(&[[0.08, 0.16], [0.40, 0.16], [0.40, 0.24], [0.08, 0.24]]),
    piece(&[[0.60, 0.16], [0.92, 0.16], [0.92, 0.66], [0.60, 0.66]]),
    piece(&[[0.96, 0.66], [0.56, 0.66], [0.52, 0.80], [0.52, 0.86], [0.96, 0.86]]),
    accent(&[[0.60, 0.16], [0.92, 0.16], [0.92, 0.24], [0.60, 0.24]]),
];

/// The silhouette of an armour slot: a helmet with a visor, a chestplate with sleeves,
/// leggings, a pair of boots. Painted in order, so a later piece lies over an earlier one.
pub(crate) fn armour_pieces(slot: ArmourSlot) -> &'static [ArmourPiece] {
    match slot {
        ArmourSlot::Helmet => HELMET,
        ArmourSlot::Chestplate => CHESTPLATE,
        ArmourSlot::Leggings => LEGGINGS,
        ArmourSlot::Boots => BOOTS,
    }
}

fn at(rect: Rect, p: [f32; 2]) -> Pos2 {
    rect.min + egui::vec2(rect.width() * p[0], rect.height() * p[1])
}

const OUTLINE: Color32 = Color32::from_rgb(24, 24, 30);

/// Paint an armour piece of `material` in `slot`: the silhouette, a dark outline round
/// it, an accent shade on its straps and bands.
pub(crate) fn paint_armour_icon(painter: &Painter, rect: Rect, slot: ArmourSlot, material: ArmourMaterial) {
    let base = armour_material_color(material);
    let shade = |k: f32| to_color([base[0] * k, base[1] * k, base[2] * k]);
    let (main, dark) = (shade(1.0), shade(0.62));
    // The outline first: every piece, dark and a pixel fatter, so the silhouette reads
    // on any slot background.
    for p in armour_pieces(slot) {
        let pts: Vec<Pos2> = p.points.iter().map(|&q| at(rect, q)).collect();
        painter.add(Shape::convex_polygon(pts, OUTLINE, Stroke::new(2.0_f32, OUTLINE)));
    }
    for p in armour_pieces(slot) {
        let pts: Vec<Pos2> = p.points.iter().map(|&q| at(rect, q)).collect();
        painter.add(Shape::convex_polygon(pts, if p.accent { dark } else { main }, Stroke::NONE));
    }
}

/// Paint a Plan: a rolled blueprint, its paper in the colour of its develop stage
/// (`colour`: [`Item::color`]), with pale lines and a roll top and bottom.
pub(crate) fn paint_plan_icon(painter: &Painter, rect: Rect, colour: [f32; 3]) {
    let paper = to_color(colour);
    let roll = to_color([colour[0] * 0.6, colour[1] * 0.6, colour[2] * 0.6]);
    let line = Color32::from_rgba_unmultiplied(235, 240, 250, 190);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(rect, [x0, y0]), at(rect, [x1, y1]));
    // The sheet, then the two rolls over its ends.
    painter.rect_filled(r(0.20, 0.14, 0.80, 0.86), 1.0, OUTLINE);
    painter.rect_filled(r(0.22, 0.16, 0.78, 0.84), 1.0, paper);
    for y in [0.32, 0.44, 0.56] {
        painter.rect_filled(r(0.30, y, 0.70, y + 0.04), 0.0, line);
    }
    painter.rect_filled(r(0.30, 0.66, 0.52, 0.70), 0.0, line);
    for (y0, y1) in [(0.08, 0.22), (0.78, 0.92)] {
        painter.rect_filled(r(0.14, y0, 0.86, y1), 3.0, OUTLINE);
        painter.rect_filled(r(0.16, y0 + 0.02, 0.84, y1 - 0.02), 3.0, roll);
    }
}

/// Paint a tool: the held model seen from the front, turned a quarter of a right angle
/// so a long shaft runs corner to corner, filling the icon. Each box of the model is
/// one polygon in its own colour (the material's head colour for the head or blade, the
/// stick's for the handle).
pub(crate) fn paint_tool_icon(painter: &Painter, rect: Rect, tool: crate::crafting::Tool) {
    let quads = tool_icon_polygons(tool.tool_type, tool.material);
    // Fit the turned shape to the rect, centred, with a small margin.
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for q in &quads {
        for c in q.corners {
            for i in 0..2 {
                lo[i] = lo[i].min(c[i]);
                hi[i] = hi[i].max(c[i]);
            }
        }
    }
    let (w, h) = ((hi[0] - lo[0]).max(1e-3), (hi[1] - lo[1]).max(1e-3));
    let side = rect.width().min(rect.height()) * 0.94;
    let scale = side / w.max(h);
    let centre = rect.center();
    let map = |c: [f32; 2]| {
        // Model y is up, the screen's down.
        Pos2::new(
            centre.x + (c[0] - (lo[0] + hi[0]) * 0.5) * scale,
            centre.y - (c[1] - (lo[1] + hi[1]) * 0.5) * scale,
        )
    };
    for q in &quads {
        let pts: Vec<Pos2> = q.corners.iter().map(|&c| map(c)).collect();
        painter.add(Shape::convex_polygon(pts, OUTLINE, Stroke::new(2.0_f32, OUTLINE)));
    }
    for q in &quads {
        let pts: Vec<Pos2> = q.corners.iter().map(|&c| map(c)).collect();
        painter.add(Shape::convex_polygon(pts, Color32::from_rgb(q.rgb[0], q.rgb[1], q.rgb[2]), Stroke::NONE));
    }
}

/// The tool's polygons, turned 45 degrees clockwise (the top toward the upper right),
/// back to front.
pub(crate) fn tool_icon_polygons(
    tool: crate::crafting::ToolType,
    material: crate::crafting::ToolMaterial,
) -> Vec<crate::held_tool_model::IconQuad> {
    let (s, c) = (-std::f32::consts::FRAC_PI_4).sin_cos();
    let mut quads = crate::held_tool_model::icon_quads(tool, material);
    // Nearer boxes over farther ones; equal depths keep the model's own order.
    quads.sort_by(|a, b| a.front.total_cmp(&b.front));
    for q in &mut quads {
        for corner in &mut q.corners {
            let [x, y] = *corner;
            *corner = [x * c - y * s, x * s + y * c];
        }
    }
    quads
}

/// Paint the icon of `item` into `rect` (the slot's icon square). The one function
/// every slot painter calls.
pub(crate) fn paint_item_icon(
    painter: &Painter,
    rect: Rect,
    item: &Item,
    registry: &BlockRegistry,
    icons: &dyn IconSource,
) {
    match item {
        Item::Block(id) => {
            crate::hud_icons::paint_block_icon(painter, rect, *id, registry, |layer| icons.block_texture(layer));
        }
        Item::Tool(tool) if tool.tool_type == crate::crafting::ToolType::Shield => icons.paint_shield(painter, rect),
        Item::Tool(tool) => paint_tool_icon(painter, rect, *tool),
        // The log materials draw as a log (the oak log icon, tinted by stage).
        Item::Material(m) if crate::item::log_material_icon(*m).is_some() => {
            let (log, tint) = crate::item::log_material_icon(*m).expect("checked by the guard");
            if let Some(tex_id) = icons.block_texture(registry.tex_top(log)) {
                painter.image(tex_id, rect, UNIT_UV, log_tint_color(tint));
            }
        }
        // Every other material has its own texture layer (many share a stand-in
        // picture, but none is a bare swatch). Only where that layer has not been
        // uploaded (a headless pass) does the flat colour stand in.
        //
        // A material that stands in with another material's layer (the buckets and
        // the copper, tin and bronze ingots on the iron ingot's, the saplings on the
        // wheat seeds) is multiplied by its own colour, as the flat swatch it
        // replaced was, so the two buckets and the three ingots stay apart.
        Item::Material(m) => match icons.block_texture(crate::entity_model::material_texture(*m)) {
            Some(tex_id) => {
                painter.image(tex_id, rect, UNIT_UV, borrowed_layer_tint(*m, item.color(registry)));
            }
            None => {
                painter.rect_filled(rect, 2.0, to_color(item.color(registry)));
            }
        },
        Item::Plan(_) => paint_plan_icon(painter, rect, item.color(registry)),
        Item::Armour(a) => paint_armour_icon(painter, rect, a.slot, a.material),
    }
}

/// The material whose picture each texture layer is: the first material (in id
/// order) that maps to it. Every later material on the same layer borrows it.
fn layer_owner(layer: u32) -> Option<u16> {
    static OWNERS: std::sync::OnceLock<std::collections::HashMap<u32, u16>> = std::sync::OnceLock::new();
    OWNERS
        .get_or_init(|| {
            let mut owners = std::collections::HashMap::new();
            for id in 0..=u16::MAX {
                let Ok(m) = crate::item::MaterialId::try_from(id) else { break };
                owners.entry(crate::entity_model::material_texture(m)).or_insert(id);
            }
            owners
        })
        .get(&layer)
        .copied()
}

/// Whether `m` draws another material's layer rather than its own.
pub(crate) fn borrows_layer(m: crate::item::MaterialId) -> bool {
    layer_owner(crate::entity_model::material_texture(m)).is_some_and(|owner| owner != m as u16)
}

/// The multiplier a material's icon is drawn with: its own colour (`color`, the
/// item's) when it borrows another material's layer, white when the layer is its own.
pub(crate) fn borrowed_layer_tint(m: crate::item::MaterialId, color: [f32; 3]) -> Color32 {
    if borrows_layer(m) { to_color(color) } else { Color32::WHITE }
}

const UNIT_UV: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armour::all_armour_combos;
    use crate::craft_ui::NoIcons;
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::item::{ItemStack, MaterialId};

    fn shapes_of(item: &Item) -> Vec<Shape> {
        let ctx = egui::Context::default();
        let reg = BlockRegistry::new();
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let rect = Rect::from_min_size(Pos2::new(10.0, 10.0), egui::vec2(48.0, 48.0));
            paint_item_icon(&painter, rect, item, &reg, &NoIcons);
        });
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    #[test]
    fn every_tool_is_a_shape_of_its_own_in_its_materials_colour() {
        use crate::held_tool_model::tests::all_tool_types;
        let sig = |t: ToolType, m: ToolMaterial| {
            tool_icon_polygons(t, m)
                .iter()
                .map(|q| (q.corners.map(|c| c.map(|v| (v * 1000.0).round() as i32)), q.rgb))
                .collect::<Vec<_>>()
        };
        let shape = |t: ToolType| {
            tool_icon_polygons(t, ToolMaterial::Iron)
                .iter()
                .map(|q| q.corners.map(|c| c.map(|v| (v * 1000.0).round() as i32)))
                .collect::<Vec<_>>()
        };
        let types: Vec<ToolType> = all_tool_types().into_iter().filter(|t| *t != ToolType::Shield).collect();
        assert_eq!(types.len(), 12, "the twelve non-shield tools");
        for (i, a) in types.iter().enumerate() {
            assert!(shape(*a).len() >= 3, "{a:?} is more than a bar");
            for b in &types[i + 1..] {
                assert_ne!(shape(*a), shape(*b), "{a:?} and {b:?} have the same icon shape");
            }
        }
        // The five tiered types change colour with the material, and keep their shape.
        let mats = [ToolMaterial::Wood, ToolMaterial::Stone, ToolMaterial::Iron, ToolMaterial::Diamond, ToolMaterial::Satori];
        for t in [ToolType::Pickaxe, ToolType::Axe, ToolType::Sword, ToolType::Shovel, ToolType::Hoe] {
            for (i, a) in mats.iter().enumerate() {
                for b in &mats[i + 1..] {
                    assert_ne!(sig(t, *a), sig(t, *b), "{t:?} in {a:?} and {b:?} is the same icon");
                }
            }
        }
    }

    #[test]
    fn a_tool_icon_is_several_polygons_not_a_swatch() {
        for t in crate::held_tool_model::tests::all_tool_types() {
            if t == ToolType::Shield {
                continue;
            }
            let n = shapes_of(&Item::Tool(Tool::new(t, ToolMaterial::Iron))).len();
            assert!(n >= 6, "{t:?}: {n} shapes (outline and fill per box)");
        }
    }

    #[test]
    fn armour_silhouettes_differ_per_slot_and_are_convex_pieces_in_the_unit_square() {
        let slots = [ArmourSlot::Helmet, ArmourSlot::Chestplate, ArmourSlot::Leggings, ArmourSlot::Boots];
        for (i, a) in slots.iter().enumerate() {
            let pieces = armour_pieces(*a);
            assert!(pieces.len() >= 4, "{a:?}: a silhouette, not a block");
            for p in pieces {
                assert!(p.points.len() >= 4);
                for q in p.points {
                    assert!((0.0..=1.0).contains(&q[0]) && (0.0..=1.0).contains(&q[1]), "{a:?} point {q:?} leaves the icon");
                }
                // Convex: every turn goes the same way.
                let n = p.points.len();
                let mut sign = 0.0_f32;
                for k in 0..n {
                    let (a0, a1, a2) = (p.points[k], p.points[(k + 1) % n], p.points[(k + 2) % n]);
                    let cross = (a1[0] - a0[0]) * (a2[1] - a1[1]) - (a1[1] - a0[1]) * (a2[0] - a1[0]);
                    if cross.abs() > 1e-6 {
                        assert!(sign == 0.0 || sign * cross > 0.0, "{a:?}: a piece is not convex");
                        sign = cross;
                    }
                }
            }
            for b in &slots[i + 1..] {
                assert_ne!(armour_pieces(*a), armour_pieces(*b), "{a:?} and {b:?} share a silhouette");
            }
        }
    }

    #[test]
    fn every_armour_combo_draws_a_silhouette_in_its_material_colour() {
        for (slot, mat) in all_armour_combos() {
            let item = ItemStack::new_armour(slot, mat).item;
            assert!(shapes_of(&item).len() >= 8, "{slot:?} {mat:?}: outline and fill per piece");
        }
        // The tint is the item's own colour (the grid, the hotbar and the column agree).
        let reg = BlockRegistry::new();
        for (slot, mat) in all_armour_combos() {
            let item = ItemStack::new_armour(slot, mat).item;
            assert_eq!(item.color(&reg), armour_material_color(mat), "{slot:?} {mat:?}");
        }
    }

    #[test]
    fn a_plan_is_a_scroll_in_its_stage_colour() {
        let plan = Item::Plan(crate::plan::PlanData::debug_3x3_stone());
        assert!(shapes_of(&plan).len() >= 8);
    }

    #[test]
    fn a_material_on_another_materials_layer_is_tinted_by_its_own_colour() {
        let reg = BlockRegistry::new();
        let tint = |m: MaterialId| borrowed_layer_tint(m, Item::Material(m).color(&reg));
        // The two buckets share the iron ingot's layer and must not look alike.
        assert_ne!(tint(MaterialId::WaterBucket), tint(MaterialId::LavaBucket));
        assert_ne!(tint(MaterialId::WaterBucket), Color32::WHITE);
        // So do the three ingots, and none of them is the iron ingot's own picture.
        let ingots = [MaterialId::CopperIngot, MaterialId::TinIngot, MaterialId::BronzeIngot].map(tint);
        assert!(ingots[0] != ingots[1] && ingots[1] != ingots[2] && ingots[0] != ingots[2]);
        // A material on its own layer is drawn as the picture is.
        assert_eq!(tint(MaterialId::IronIngot), Color32::WHITE);
        assert_eq!(tint(MaterialId::Coal), Color32::WHITE);
        assert!(!borrows_layer(MaterialId::IronIngot) && borrows_layer(MaterialId::Bucket));
    }

    #[test]
    fn every_material_has_a_texture_to_draw() {
        // With no atlas, a flat swatch stands in; with one, the material's layer. The
        // layer is always a real one (below the array size).
        for m in (0..=u16::MAX).map_while(|i| MaterialId::try_from(i).ok()) {
            let layer = crate::entity_model::material_texture(m);
            assert!(layer < 512, "{m:?} -> layer {layer}");
            // A log material draws its (tinted) log layer: nothing without the atlas.
            let want = if crate::item::log_material_icon(m).is_some() { 0 } else { 1 };
            assert_eq!(shapes_of(&Item::Material(m)).len(), want, "{m:?} falls back to one swatch headless");
        }
    }
}
