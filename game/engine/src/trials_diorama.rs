//! The Trials gallery's little block dioramas: the pure side. A trial's
//! `card_art` is either one item name or a short list of blocks on a small
//! integer grid; this module holds that data shape, the isometric projection,
//! the painter's-order sort and the fit-to-card maths. No drawing and no block
//! registry here, so all of it is unit-tested; `trials_diorama_ui` paints it.
//!
//! The view: x grows to the lower right, z to the lower left, y is up. A cube
//! shows its top, its +z face (left on screen) and its +x face (right).

use serde::{Deserialize, Serialize};

/// Most blocks one diorama may hold (`trials_lint` enforces it on bundled trials).
pub const MAX_DIORAMA_BLOCKS: usize = 8;

/// Grid coordinates stay in `-MAX_COORD..=MAX_COORD`, so a card stays one small scene.
pub const MAX_COORD: i32 = 6;

/// One block of a diorama: a `/give`-style block name and its grid cell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DioramaBlock {
    pub block: String,
    /// `[x, y, z]`: x to the lower right, z to the lower left, y up.
    pub at: [i32; 3],
}

/// A trial's `card_art`: the original single name, or a diorama.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CardArtDef {
    /// `"furnace"`: one item or block (a block is drawn as a lone cube).
    Name(String),
    /// `[{ "block": "furnace", "at": [0, 0, 0] }, ...]`
    Diorama(Vec<DioramaBlock>),
}

/// How one block is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A full cube: top, left and right faces.
    Cube,
    /// A cut-out sprite (crops, torches, doors) standing in its cell.
    Sprite,
    /// A flat tile lying on the cell floor (rails, plates).
    Flat,
    /// A liquid: a cube drawn bright, with no side shading, a little see-through.
    Fluid,
}

/// A diorama block resolved to what the painter needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cube {
    pub pos: [i32; 3],
    pub top: u32,
    pub side: u32,
    pub shape: Shape,
}

/// Face shading: top full, left (+z) 80%, right (+x) 62%.
pub const SHADE_TOP: u8 = 255;
pub const SHADE_LEFT: u8 = 204;
pub const SHADE_RIGHT: u8 = 158;

/// Cube height against its half-width (the top diamond is `1.0` tall, `2.0` wide).
pub const CUBE_H: f32 = 1.0;

/// Screen position (unit scale: a cube is 2 wide) of a grid point. Larger
/// `x + z` is nearer the viewer (lower on screen).
pub fn project(x: f32, y: f32, z: f32) -> [f32; 2] {
    [x - z, (x + z) * 0.5 - y * CUBE_H]
}

/// Back-to-front painter's order: far cells first, then lower before higher.
/// Cubes with equal depth never overlap on screen, so ties are harmless.
pub fn sort_back_to_front(cubes: &mut [Cube]) {
    cubes.sort_by_key(|c| (c.pos[0] + c.pos[1] + c.pos[2], c.pos[1], c.pos[0]));
}

/// Width of a sprite on screen, in the same unit scale.
pub const SPRITE_W: f32 = 1.0;
/// Height of a sprite on screen.
pub const SPRITE_H: f32 = 1.1;
/// How far a flat tile sits above its cell floor.
pub const FLAT_LIFT: f32 = 0.04;

/// A block's screen bounds `[min_x, min_y, max_x, max_y]` (unit scale).
pub fn cube_bounds(c: &Cube) -> [f32; 4] {
    let (x, y, z) = (c.pos[0] as f32, c.pos[1] as f32, c.pos[2] as f32);
    match c.shape {
        Shape::Cube | Shape::Fluid => [
            x - z - 1.0,
            (x + z) * 0.5 - (y + 1.0) * CUBE_H,
            x - z + 1.0,
            (x + z + 2.0) * 0.5 - y * CUBE_H,
        ],
        Shape::Flat => [
            x - z - 1.0,
            (x + z) * 0.5 - (y + FLAT_LIFT) * CUBE_H,
            x - z + 1.0,
            (x + z + 2.0) * 0.5 - (y + FLAT_LIFT) * CUBE_H,
        ],
        Shape::Sprite => {
            let [bx, by] = project(x + 0.5, y, z + 0.5);
            [bx - SPRITE_W / 2.0, by - SPRITE_H, bx + SPRITE_W / 2.0, by]
        }
    }
}

/// The union of every block's screen bounds, or `None` for an empty diorama.
pub fn scene_bounds(cubes: &[Cube]) -> Option<[f32; 4]> {
    cubes.iter().map(cube_bounds).reduce(|a, b| {
        [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
    })
}

/// Where the scene lands in a box: the scale (pixels per unit) and the screen
/// point that unit-scale `(0, 0)` maps to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f32,
    pub origin: [f32; 2],
}

/// Fit `bounds` into a box of `avail` pixels (width, height) whose centre is
/// `centre`, leaving `margin` (0..0.5) of each side empty, centred, and never
/// scaled past `max_scale` (so a lone cube is not blown up to fill a hero).
pub fn fit(bounds: [f32; 4], centre: [f32; 2], avail: [f32; 2], margin: f32, max_scale: f32) -> Fit {
    let bw = (bounds[2] - bounds[0]).max(1e-3);
    let bh = (bounds[3] - bounds[1]).max(1e-3);
    let keep = 1.0 - 2.0 * margin.clamp(0.0, 0.45);
    let scale = (avail[0] * keep / bw).min(avail[1] * keep / bh).min(max_scale).max(0.0);
    let cx = (bounds[0] + bounds[2]) * 0.5;
    let cy = (bounds[1] + bounds[3]) * 0.5;
    Fit { scale, origin: [centre[0] - cx * scale, centre[1] - cy * scale] }
}

impl Fit {
    /// A unit-scale point on screen.
    pub fn at(&self, p: [f32; 2]) -> [f32; 2] {
        [self.origin[0] + p[0] * self.scale, self.origin[1] + p[1] * self.scale]
    }
}

/// The four corners of each visible face of a cube at `pos`, in unit-scale
/// screen space, with the texture corner (u, v) each one takes. Order: top,
/// left (+z), right (+x). Each face is `[(x, y, u, v); 4]`, wound the same way.
pub fn cube_faces(pos: [i32; 3]) -> [[[f32; 4]; 4]; 3] {
    let (x, y, z) = (pos[0] as f32, pos[1] as f32, pos[2] as f32);
    let p = |px: f32, py: f32, pz: f32, u: f32, v: f32| {
        let s = project(px, py, pz);
        [s[0], s[1], u, v]
    };
    let top = [
        p(x, y + 1.0, z, 0.0, 0.0),
        p(x + 1.0, y + 1.0, z, 1.0, 0.0),
        p(x + 1.0, y + 1.0, z + 1.0, 1.0, 1.0),
        p(x, y + 1.0, z + 1.0, 0.0, 1.0),
    ];
    let left = [
        p(x, y + 1.0, z + 1.0, 0.0, 0.0),
        p(x + 1.0, y + 1.0, z + 1.0, 1.0, 0.0),
        p(x + 1.0, y, z + 1.0, 1.0, 1.0),
        p(x, y, z + 1.0, 0.0, 1.0),
    ];
    let right = [
        p(x + 1.0, y + 1.0, z + 1.0, 0.0, 0.0),
        p(x + 1.0, y + 1.0, z, 1.0, 0.0),
        p(x + 1.0, y, z, 1.0, 1.0),
        p(x + 1.0, y, z + 1.0, 0.0, 1.0),
    ];
    [top, left, right]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(x: i32, y: i32, z: i32) -> Cube {
        Cube { pos: [x, y, z], top: 0, side: 0, shape: Shape::Cube }
    }

    #[test]
    fn card_art_parses_the_single_name_form() {
        let a: CardArtDef = serde_json::from_str("\"furnace\"").unwrap();
        assert_eq!(a, CardArtDef::Name("furnace".to_string()));
    }

    #[test]
    fn card_art_parses_the_diorama_form() {
        let a: CardArtDef = serde_json::from_str(
            r#"[{"block":"furnace","at":[0,1,0]},{ "block": "stone", "at": [0, 0, -1] }]"#,
        )
        .unwrap();
        assert_eq!(
            a,
            CardArtDef::Diorama(vec![
                DioramaBlock { block: "furnace".into(), at: [0, 1, 0] },
                DioramaBlock { block: "stone".into(), at: [0, 0, -1] },
            ])
        );
    }

    #[test]
    fn card_art_rejects_a_malformed_diorama() {
        assert!(serde_json::from_str::<CardArtDef>(r#"[{"block":"stone"}]"#).is_err());
        assert!(serde_json::from_str::<CardArtDef>(r#"[{"block":"stone","at":[1,2]}]"#).is_err());
        assert!(serde_json::from_str::<CardArtDef>("7").is_err());
    }

    #[test]
    fn card_art_round_trips_in_both_forms() {
        for src in ["\"bone\"", r#"[{"block":"hay_bale","at":[1,0,2]}]"#] {
            let a: CardArtDef = serde_json::from_str(src).unwrap();
            let back: CardArtDef = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
            assert_eq!(a, back);
        }
    }

    #[test]
    fn nearer_cells_are_lower_on_screen() {
        let a = project(0.0, 0.0, 0.0);
        assert!(project(1.0, 0.0, 0.0)[1] > a[1]);
        assert!(project(0.0, 0.0, 1.0)[1] > a[1]);
        assert!(project(0.0, 1.0, 0.0)[1] < a[1]);
        // x runs right, z runs left.
        assert!(project(1.0, 0.0, 0.0)[0] > a[0]);
        assert!(project(0.0, 0.0, 1.0)[0] < a[0]);
    }

    #[test]
    fn sort_paints_far_and_low_first() {
        let mut v = vec![cube(1, 1, 1), cube(0, 0, 0), cube(1, 0, 0), cube(0, 1, 0), cube(2, 0, 2)];
        sort_back_to_front(&mut v);
        let depth: Vec<i32> = v.iter().map(|c| c.pos[0] + c.pos[1] + c.pos[2]).collect();
        assert!(depth.windows(2).all(|w| w[0] <= w[1]), "{depth:?}");
        assert_eq!(v[0].pos, [0, 0, 0]);
        assert_eq!(v.last().unwrap().pos, [2, 0, 2]);
    }

    #[test]
    fn sort_puts_a_block_on_top_after_the_block_it_sits_on() {
        let mut v = vec![cube(0, 1, 0), cube(0, 0, 0)];
        sort_back_to_front(&mut v);
        assert_eq!(v[0].pos, [0, 0, 0]);
        assert_eq!(v[1].pos, [0, 1, 0]);
    }

    #[test]
    fn a_lone_cube_is_two_wide_and_two_tall() {
        let b = scene_bounds(&[cube(0, 0, 0)]).unwrap();
        assert_eq!(b, [-1.0, -1.0, 1.0, 1.0]);
    }

    #[test]
    fn bounds_cover_every_block_and_an_empty_scene_has_none() {
        assert_eq!(scene_bounds(&[]), None);
        let b = scene_bounds(&[cube(0, 0, 0), cube(3, 2, 0)]).unwrap();
        assert_eq!((b[1], b[2]), (-1.5, 4.0), "{b:?}");
        for c in [cube(0, 0, 0), cube(3, 2, 0)] {
            let cb = cube_bounds(&c);
            assert!(b[0] <= cb[0] && b[1] <= cb[1] && b[2] >= cb[2] && b[3] >= cb[3]);
        }
    }

    #[test]
    fn fit_centres_the_scene_and_fills_the_tighter_axis() {
        let b = [-1.0, -1.0, 1.0, 1.0];
        // A wide box: the height limits the scale.
        let f = fit(b, [100.0, 50.0], [200.0, 100.0], 0.0, 1000.0);
        assert_eq!(f.scale, 50.0);
        assert_eq!(f.at([0.0, 0.0]), [100.0, 50.0]);
        // A tall box: the width limits it.
        let f = fit(b, [0.0, 0.0], [40.0, 400.0], 0.0, 1000.0);
        assert_eq!(f.scale, 20.0);
    }

    #[test]
    fn fit_keeps_a_margin_and_honours_the_scale_cap() {
        let b = [-1.0, -1.0, 1.0, 1.0];
        let f = fit(b, [0.0, 0.0], [100.0, 100.0], 0.1, 1000.0);
        assert!((f.scale - 40.0).abs() < 1e-4, "{}", f.scale);
        let f = fit(b, [0.0, 0.0], [100.0, 100.0], 0.0, 12.0);
        assert_eq!(f.scale, 12.0);
    }

    #[test]
    fn fit_centres_an_off_origin_scene() {
        let b = [2.0, 4.0, 6.0, 8.0];
        let f = fit(b, [10.0, 10.0], [100.0, 100.0], 0.0, 1000.0);
        let c = f.at([4.0, 6.0]);
        assert!((c[0] - 10.0).abs() < 1e-3 && (c[1] - 10.0).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn cube_faces_share_the_front_corner_and_use_the_full_texture() {
        let [top, left, right] = cube_faces([0, 0, 0]);
        // The near top corner is shared by all three faces.
        assert_eq!(top[2][..2], left[1][..2]);
        assert_eq!(top[2][..2], right[0][..2]);
        // The left and right faces share the near vertical edge.
        assert_eq!(left[1][..2], right[0][..2]);
        assert_eq!(left[2][..2], right[3][..2]);
        for face in [top, left, right] {
            let us: Vec<f32> = face.iter().map(|v| v[2]).collect();
            let vs: Vec<f32> = face.iter().map(|v| v[3]).collect();
            assert!(us.contains(&0.0) && us.contains(&1.0) && vs.contains(&0.0) && vs.contains(&1.0));
        }
    }

    #[test]
    fn shading_orders_top_over_left_over_right() {
        assert!(SHADE_TOP > SHADE_LEFT && SHADE_LEFT > SHADE_RIGHT);
    }
}
