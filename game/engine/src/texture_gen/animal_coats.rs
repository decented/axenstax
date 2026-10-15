//! Bespoke coats baked into the existing species slots. No extra atlas layers,
//! texture keys or pack migration; MobDef.color remains the base coat colour.

use super::*;
use crate::entity_model::*;
use crate::mob::MobType;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    End,
    Side,
    Top,
    Face,
    Cheek,
    Crown,
    Feet,
    Wing,
}

pub(super) fn generate(kind: MobType, donor: u32, colour: [f32; 3]) -> Option<Vec<u8>> {
    use MobType::*;
    use Surface::*;
    if !matches!(kind, Fox | Cat | Crab | Reindeer | Donkey | Mule | Parrot) {
        return None;
    }
    let surface = match donor {
        TEX_SHEEP_BODY_END | TEX_COW_BODY_END => End,
        TEX_SHEEP_BODY_SIDE | TEX_COW_BODY_SIDE | TEX_CHICKEN_BODY => Side,
        TEX_SHEEP_BODY_TOP | TEX_COW_BODY_TOP => Top,
        TEX_SHEEP_HEAD_FRONT | TEX_COW_HEAD_FRONT | TEX_CHICKEN_HEAD => Face,
        TEX_SHEEP_HEAD_SIDE | TEX_COW_HEAD_SIDE => Cheek,
        TEX_SHEEP_HEAD_TOP | TEX_COW_HEAD_TOP => Crown,
        TEX_CHICKEN_LEG => Feet,
        TEX_CHICKEN_WING => Wing,
        _ => return None,
    };
    let base = colour.map(|c| (c.clamp(0.0, 1.0) * 255.0) as i32);
    let mut pixels = vec![0; PIXELS];
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Low contrast, two-pixel clusters avoid the speckled donor wool.
            let n = (px_hash(x / 2, y / 2, 7331) % 9) as i32 - 4;
            let mut rgb = base.map(|v| v + n);
            match kind {
                Fox => {
                    if surface == End || (surface == Face && y >= 9) || (surface == Side && y >= 13)
                    {
                        rgb = [237 + n, 224 + n, 198 + n];
                    }
                    if surface == Crown && (x <= 3 || x >= 12) {
                        rgb = [55 + n, 38 + n, 31 + n];
                    }
                }
                Cat => {
                    if matches!(surface, Side | Top | Crown) && (x + y / 4) % 6 <= 1 {
                        rgb = base.map(|v| v * 3 / 5 + n);
                    }
                    if surface == End || (surface == Face && y >= 10) {
                        rgb = [210 + n, 201 + n, 180 + n];
                    }
                    if surface == Face && (5..=10).contains(&x) && y <= 3 && x % 3 == y % 3 {
                        rgb = base.map(|v| v / 2);
                    }
                }
                Crab => {
                    if surface == Crown {
                        rgb = [28 + n, 25 + n, 23 + n];
                    }
                    if surface == Top && (x == 2 || x == 13 || y == 2 || y == 13) {
                        rgb = base.map(|v| v * 3 / 4 + n);
                    }
                    if surface == Cheek {
                        rgb = base.map(|v| v + 15 + n);
                    }
                }
                Reindeer => {
                    if surface == End {
                        rgb = [205 + n, 187 + n, 150 + n];
                    }
                    if surface == Side && y >= 11 {
                        rgb = [164 + n, 146 + n, 115 + n];
                    }
                    if surface == Face && y >= 10 {
                        rgb = [67 + n, 52 + n, 40 + n];
                    }
                }
                Donkey | Mule => {
                    if surface == End {
                        rgb = [53 + n, 43 + n, 37 + n];
                    }
                    if (surface == Face && y >= 9) || (surface == Side && y >= 12) {
                        rgb = [195 + n, 183 + n, 163 + n];
                    }
                    if surface == Top && (7..=8).contains(&x) {
                        rgb = [55 + n, 44 + n, 37 + n];
                    }
                }
                Parrot => {
                    if surface == Wing {
                        rgb = if y < 5 {
                            [225 + n, 180 + n, 48 + n]
                        } else if y < 10 {
                            [48 + n, 133 + n, 77 + n]
                        } else {
                            [40 + n, 91 + n, 167 + n]
                        };
                        if x.is_multiple_of(4) {
                            rgb = rgb.map(|v| v * 4 / 5);
                        }
                    }
                    if surface == Feet {
                        rgb = [64 + n, 56 + n, 50 + n];
                    }
                    if surface == Face && (2..=13).contains(&x) && (3..=10).contains(&y) {
                        rgb = [230 + n, 219 + n, 195 + n];
                    }
                }
                _ => unreachable!(),
            }
            // Faces have distinct eye whites/irises, dark pupils and small
            // highlights. Body, crown and rear faces never acquire eyes.
            if surface == Face && !matches!(kind, Crab | Parrot) {
                let eye = (3..=5).contains(&x) || (10..=12).contains(&x);
                if eye && (5..=7).contains(&y) {
                    rgb = if kind == Cat {
                        [143, 177, 82]
                    } else {
                        [35, 29, 25]
                    };
                    if x == 4 || x == 11 {
                        rgb = [22, 24, 20];
                    }
                    if (x == 3 || x == 10) && y == 5 {
                        rgb = [242, 237, 218];
                    }
                }
                if (7..=8).contains(&x) && (10..=11).contains(&y) {
                    rgb = if kind == Cat {
                        [184, 117, 111]
                    } else {
                        [39, 30, 27]
                    };
                }
            }
            // Birds have one eye on each cheek. The front is plain red and
            // carries the separate beak; a mammal's two-eye mask repeats eyes.
            if surface == Face && kind == Parrot && (7..=9).contains(&x) && (5..=8).contains(&y) {
                rgb = if x == 7 && y == 5 {
                    [245, 238, 220]
                } else {
                    [24, 27, 25]
                };
            }
            let [r, g, b] = rgb.map(clamp_u8);
            set_px(&mut pixels, x, y, r, g, b, 255);
        }
    }
    Some(pixels)
}
