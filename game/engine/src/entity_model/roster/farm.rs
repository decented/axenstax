//! Farmyard silhouettes: characteristic ears, muzzle, feet and tail feathers.
use super::*;

pub(super) fn polish(kind: MobType, mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    match kind {
        MobType::Cow => {
            m[1].tex_faces[4] = TEX_COW_HEAD_TOP;
            // Ears and short cream horns are separate from the face artwork.
            for x in [-0.29, 0.29] {
                m.push(cuboid([x, 1.02, -0.48], [0.16, 0.08, 0.13], Brown));
                m.push(cuboid([x * 0.67, 1.20, -0.43], [0.06, 0.16, 0.07], Sand));
            }
            m.push(cuboid([0.0, 0.74, -0.71], [0.32, 0.14, 0.09], Pink));
            for x in [-0.09, 0.09] {
                m.push(cuboid([x, 0.74, -0.761], [0.045, 0.025, 0.015], Black));
            }
            m.push(cuboid([0.0, 0.53, 0.42], [0.055, 0.50, 0.055], Brown));
            m.push(cuboid([0.0, 0.27, 0.42], [0.09, 0.12, 0.09], Black));
            feet(&mut m, 2..6, Black);
        }
        MobType::Pig => {
            m[1].tex_faces[4] = TEX_PIG_HEAD_TOP;
            for x in [-0.18, 0.18] {
                m.push(cuboid([x, 0.80, -0.46], [0.14, 0.15, 0.07], Pink));
            }
            m.push(cuboid([0.0, 0.47, -0.73], [0.23, 0.15, 0.12], Pink));
            for x in [-0.055, 0.055] {
                m.push(cuboid([x, 0.48, -0.797], [0.035, 0.035, 0.014], Brown));
            }
            // Three joined steps make the little curl readable from the side.
            for (p, s) in [
                ([0.0, 0.53, 0.42], [0.05, 0.05, 0.12]),
                ([0.0, 0.58, 0.49], [0.05, 0.12, 0.05]),
                ([0.0, 0.63, 0.46], [0.05, 0.05, 0.08]),
            ] {
                m.push(cuboid(p, s, Pink));
            }
            feet(&mut m, 2..6, Grey);
        }
        MobType::Sheep => {
            m[1].tex_faces[4] = TEX_SHEEP_HEAD_TOP;
            for x in [-0.24, 0.24] {
                m.push(cuboid([x, 0.92, -0.46], [0.15, 0.09, 0.13], Sand));
            }
            // Wool cap and shoulder locks soften the otherwise square fleece.
            m.push(box_part(
                Vec3::new(0.0, 1.04, -0.46),
                Vec3::new(0.44, 0.09, 0.34),
                [TEX_SHEEP_BODY_TOP; 6],
            ));
            for x in [-0.28, 0.28] {
                m.push(box_part(
                    Vec3::new(x, 0.57, -0.23),
                    Vec3::new(0.12, 0.21, 0.28),
                    [TEX_SHEEP_BODY_SIDE; 6],
                ));
            }
            m.push(box_part(
                Vec3::new(0.0, 0.57, 0.47),
                Vec3::new(0.13, 0.20, 0.15),
                [TEX_SHEEP_BODY_SIDE; 6],
            ));
        }
        MobType::Chicken => {
            // Folded wings join the body instead of floating beside it.
            m[2].origin.x = -0.19;
            m[3].origin.x = 0.19;
            m.push(cuboid([0.0, 0.67, -0.14], [0.055, 0.10, 0.15], Red));
            m.push(cuboid([0.0, 0.51, -0.27], [0.10, 0.065, 0.10], Gold));
            m.push(cuboid([0.0, 0.45, -0.24], [0.055, 0.10, 0.04], Red));
            for (x, y) in [(-0.055, 0.40), (0.0, 0.44), (0.055, 0.40)] {
                m.push(cuboid([x, y, 0.20], [0.055, 0.22, 0.12], White));
            }
            for i in 4..6 {
                let p = &m[i];
                m.push(attached(
                    p,
                    [p.origin.x, 0.025, -0.035],
                    [0.09, 0.05, 0.16],
                    Gold,
                ));
            }
        }
        _ => unreachable!(),
    }
    m
}
