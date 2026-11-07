//! Paints a Trials card's picture: a soft vertical gradient in the shelf
//! colour, a faint ground shadow, and the diorama's blocks as isometric cubes
//! (egui meshes of textured triangles, nearest-sampled like the block icons).
//! The pure maths (projection, sort, fit) lives in `trials_diorama`.

use crate::trials_diorama::{
    cube_bounds, cube_faces, fit, project, scene_bounds, Cube, Fit, Shape as CubeShape, CUBE_H, FLAT_LIFT,
    SHADE_LEFT, SHADE_RIGHT, SHADE_TOP, SPRITE_H, SPRITE_W,
};
use egui::epaint::Vertex;
use egui::{Color32, Mesh, Painter, Pos2, Rect, TextureId, Vec2};

/// Linear blend of two colours (opaque).
pub(crate) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A rounded rectangle filled with a top-to-bottom gradient, as one triangle
/// fan (every vertex's colour is a linear function of its height, so the fan
/// shades smoothly). `radii` runs nw, ne, se, sw.
pub(crate) fn gradient_mesh(rect: Rect, radii: [f32; 4], top: Color32, bottom: Color32) -> Mesh {
    use std::f32::consts::{FRAC_PI_2, PI};
    let mut outline: Vec<Pos2> = Vec::new();
    // (corner centre, start angle) for nw, ne, se, sw.
    let corners = [
        (Pos2::new(rect.min.x + radii[0], rect.min.y + radii[0]), PI, radii[0]),
        (Pos2::new(rect.max.x - radii[1], rect.min.y + radii[1]), 1.5 * PI, radii[1]),
        (Pos2::new(rect.max.x - radii[2], rect.max.y - radii[2]), 0.0, radii[2]),
        (Pos2::new(rect.min.x + radii[3], rect.max.y - radii[3]), FRAC_PI_2, radii[3]),
    ];
    for (c, start, r) in corners {
        if r <= 0.0 {
            outline.push(c);
            continue;
        }
        const STEPS: usize = 6;
        for i in 0..=STEPS {
            let a = start + FRAC_PI_2 * (i as f32 / STEPS as f32);
            outline.push(Pos2::new(c.x + r * a.cos(), c.y + r * a.sin()));
        }
    }
    let h = rect.height().max(1e-3);
    let colour_at = |y: f32| lerp_color(top, bottom, (y - rect.min.y) / h);
    let mut mesh = Mesh::default();
    let centre = rect.center();
    mesh.vertices.push(Vertex { pos: centre, uv: egui::epaint::WHITE_UV, color: colour_at(centre.y) });
    for p in &outline {
        mesh.vertices.push(Vertex { pos: *p, uv: egui::epaint::WHITE_UV, color: colour_at(p.y) });
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.indices.extend([0, 1 + i, 1 + (i + 1) % n]);
    }
    mesh
}

/// The card's backdrop: the gradient, `top` lighter than `bottom`.
pub(crate) fn paint_backdrop(painter: &Painter, rect: Rect, radii: [f32; 4], top: Color32, bottom: Color32) {
    painter.add(egui::Shape::mesh(gradient_mesh(rect, radii, top, bottom)));
}

/// One textured quad (corners `[x, y, u, v]` in screen pixels) as a mesh.
fn quad(tex: Option<TextureId>, corners: [[f32; 4]; 4], tint: Color32) -> egui::Shape {
    let mut mesh = match tex {
        Some(t) => Mesh::with_texture(t),
        None => Mesh::default(),
    };
    for c in corners {
        // Without a texture (icons not uploaded yet) the quad is a flat grey.
        let (uv, color) = match tex {
            Some(_) => (Pos2::new(c[2], c[3]), tint),
            None => (egui::epaint::WHITE_UV, lerp_color(Color32::from_gray(90), tint, 0.4)),
        };
        mesh.vertices.push(Vertex { pos: Pos2::new(c[0], c[1]), uv, color });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    egui::Shape::mesh(mesh)
}

fn scaled(f: &Fit, corners: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    corners.map(|c| {
        let p = f.at([c[0], c[1]]);
        [p[0], p[1], c[2], c[3]]
    })
}

fn outline(painter: &Painter, corners: &[[f32; 4]; 4], colour: Color32) {
    let pts: Vec<Pos2> = corners.iter().map(|c| Pos2::new(c[0], c[1])).collect();
    painter.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.0_f32, colour)));
}

/// Paint one block in place.
fn paint_cube(painter: &Painter, f: &Fit, c: &Cube, icons: &[TextureId]) {
    let tex = |layer: u32| icons.get(layer as usize).copied();
    match c.shape {
        CubeShape::Cube | CubeShape::Fluid => {
            let fluid = c.shape == CubeShape::Fluid;
            let [top, left, right] = cube_faces(c.pos).map(|face| scaled(f, face));
            // Liquids glow (no dark side) and let a little of the ground through.
            let shade = |s: u8| {
                if fluid {
                    let (g, a) = (s as u16 * 85 / 100 + 25, 235u16);
                    let p = (g * a / 255) as u8;
                    Color32::from_rgba_premultiplied(p, p, p, a as u8)
                } else {
                    Color32::from_gray(s)
                }
            };
            painter.add(quad(tex(c.side), left, shade(SHADE_LEFT)));
            painter.add(quad(tex(c.side), right, shade(SHADE_RIGHT)));
            painter.add(quad(tex(c.top), top, shade(SHADE_TOP)));
            // Dark seams under the sides, a light rim on top: dark textures
            // (coal, furnace, campfire) still read against the backdrop.
            outline(painter, &left, Color32::from_black_alpha(70));
            outline(painter, &right, Color32::from_black_alpha(90));
            outline(painter, &top, Color32::from_white_alpha(80));
        }
        CubeShape::Flat => {
            let mut top = cube_faces(c.pos)[0];
            for v in &mut top {
                v[1] += (1.0 - FLAT_LIFT) * CUBE_H;
            }
            painter.add(quad(tex(c.top), scaled(f, top), Color32::from_gray(SHADE_TOP)));
        }
        CubeShape::Sprite => {
            let [x, y, z] = c.pos.map(|v| v as f32);
            let [bx, by] = project(x + 0.5, y, z + 0.5);
            let (hw, h) = (SPRITE_W / 2.0, SPRITE_H);
            let corners = [
                [bx - hw, by - h, 0.0, 0.0],
                [bx + hw, by - h, 1.0, 0.0],
                [bx + hw, by, 1.0, 1.0],
                [bx - hw, by, 0.0, 1.0],
            ];
            // A small contact shadow so the sprite sits on its cell.
            let foot = f.at([bx, by]);
            painter.add(egui::epaint::EllipseShape::filled(
                Pos2::new(foot[0], foot[1]),
                Vec2::new(0.4 * f.scale, 0.16 * f.scale),
                Color32::from_black_alpha(46),
            ));
            painter.add(quad(tex(c.side), scaled(f, corners), Color32::WHITE));
        }
    }
}

/// Paint a diorama centred in `rect`: ground shadow, then the blocks back to
/// front (`cubes` must already be in painter's order). `max_scale_per_height`
/// caps the scale (pixels per unit) at that fraction of `rect`'s height, so a
/// lone block is not blown up to fill the card.
pub(crate) fn paint_diorama(
    painter: &Painter,
    rect: Rect,
    cubes: &[Cube],
    icons: &[TextureId],
    max_scale_per_height: f32,
) {
    let Some(bounds) = scene_bounds(cubes) else { return };
    let f = fit(
        bounds,
        [rect.center().x, rect.center().y],
        [rect.width(), rect.height()],
        0.05,
        rect.height() * max_scale_per_height,
    );
    // Ground shadow: a few soft ellipses under the scene's floor.
    let floor = cubes.iter().map(|c| cube_bounds(c)[3]).fold(f32::MIN, f32::max);
    let (left, right) = (bounds[0], bounds[2]);
    let mid = f.at([(left + right) * 0.5, floor]);
    let half = (right - left) * 0.5 * f.scale;
    for (grow, alpha) in [(1.0, 26u8), (0.86, 30), (0.7, 34)] {
        painter.add(egui::epaint::EllipseShape::filled(
            Pos2::new(mid[0], mid[1] - half * 0.06),
            Vec2::new(half * grow * 1.02, half * grow * 0.2),
            Color32::from_black_alpha(alpha),
        ));
    }
    for c in cubes {
        paint_cube(painter, &f, c, icons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gradient_stays_in_its_rect_and_runs_light_to_dark() {
        let rect = Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 120.0));
        let top = Color32::from_rgb(200, 200, 200);
        let bottom = Color32::from_rgb(40, 40, 40);
        let m = gradient_mesh(rect, [12.0, 12.0, 0.0, 0.0], top, bottom);
        assert!(!m.is_empty());
        for v in &m.vertices {
            assert!(rect.expand(0.01).contains(v.pos), "{:?} outside", v.pos);
            let want = lerp_color(top, bottom, (v.pos.y - rect.min.y) / rect.height());
            assert_eq!(v.color, want);
        }
        let hi = m.vertices.iter().min_by(|a, b| a.pos.y.total_cmp(&b.pos.y)).unwrap();
        let lo = m.vertices.iter().max_by(|a, b| a.pos.y.total_cmp(&b.pos.y)).unwrap();
        assert_eq!(hi.color, top);
        assert_eq!(lo.color, bottom);
        assert!(m.indices.iter().all(|i| (*i as usize) < m.vertices.len()));
        assert_eq!(m.indices.len() % 3, 0);
    }

    #[test]
    fn square_corners_add_one_point_each() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(10.0, 10.0));
        let m = gradient_mesh(rect, [0.0; 4], Color32::WHITE, Color32::BLACK);
        // centre + four corners
        assert_eq!(m.vertices.len(), 5);
    }
}
