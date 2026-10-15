//! NPC clothing and readable human tiers. The six legacy body slots remain;
//! attached equipment shares limb pivots. Armour is cosmetic, not combat data.
use super::*;

pub(super) fn polish(kind: MobType, mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    // AI, avatars and animal models all face -Z. These NPCs historically had
    // their face on +Z, so they appeared to walk backwards.
    m[1].tex_faces.swap(4, 5);
    let hair = match kind {
        MobType::Peddler => Purple,
        MobType::Marauder | MobType::Knight => Grey,
        _ => Brown,
    };
    // Cheek art belongs on the sides, never across the crown or rear.
    m[1].tex_faces[2] = material(hair);
    m[1].tex_faces[3] = material(Sand);
    m[1].tex_faces[4] = material(hair);
    for i in [2, 3] {
        m[i].origin.x *= 0.82;
        m[i].pivot.x = m[i].origin.x;
    }
    match kind {
        MobType::Villager => {
            m.push(cuboid([0.0, 0.99, -0.164], [0.33, 0.49, 0.025], Sand));
            m.push(cuboid([0.0, 1.23, -0.18], [0.35, 0.035, 0.035], Brown));
            m.push(cuboid([0.12, 0.92, -0.19], [0.12, 0.13, 0.025], Brown));
        }
        MobType::Peddler => {
            hood(&mut m, Purple);
            m.push(cuboid([0.0, 1.0, 0.25], [0.38, 0.50, 0.24], Brown));
            m.push(cuboid([0.0, 1.29, 0.25], [0.47, 0.12, 0.24], Sand));
            m.push(cuboid([0.12, 1.12, -0.17], [0.055, 0.53, 0.035], Gold));
            m.push(cuboid([0.0, 1.34, -0.19], [0.37, 0.12, 0.05], Purple));
            m.push(cuboid([0.0, 1.14, -0.19], [0.11, 0.36, 0.05], Purple));
        }
        MobType::Brigand => {
            dye(&mut m[0], Brown);
            for i in [2, 3, 4, 5] {
                dye(&mut m[i], Black);
            }
            m[1].tex_faces[..5].fill(material(Brown));
            hood(&mut m, Brown);
            m.push(cuboid([0.0, 1.49, -0.288], [0.51, 0.17, 0.025], Black));
            belt(&mut m, Black);
            m.push(cuboid([0.17, 0.84, -0.18], [0.16, 0.20, 0.12], Brown));
        }
        MobType::Marauder => {
            for i in [0, 2, 3] {
                dye(&mut m[i], Grey);
            }
            helmet(&mut m, Grey);
            shoulders(&mut m, Silver, 0.29);
            belt(&mut m, Brown);
            m.push(cuboid([0.11, 1.20, -0.17], [0.07, 0.52, 0.05], Brown));
            m.push(cuboid([0.0, 1.1, 0.18], [0.49, 0.72, 0.08], Grey));
        }
        MobType::Berserker => {
            m[0].size = Vec3::new(0.64, 0.90, 0.36);
            dye(&mut m[0], Red);
            for i in [2, 3] {
                m[i].origin.x *= 1.22;
                m[i].pivot.x = m[i].origin.x;
                m[i].size.x = 0.25;
                dye(&mut m[i], Brown);
            }
            shoulders(&mut m, Red, 0.35);
            belt(&mut m, Black);
            m.push(cuboid([0.0, 1.35, -0.20], [0.35, 0.20, 0.07], Black));
            m.push(cuboid([0.0, 1.84, -0.29], [0.57, 0.09, 0.04], Red));
            m.push(cuboid([0.0, 1.96, 0.0], [0.57, 0.11, 0.56], Black));
            m.push(cuboid([0.0, 1.05, 0.22], [0.64, 0.92, 0.10], Red));
        }
        MobType::Knight => {
            for i in [0, 2, 3, 4, 5] {
                dye(&mut m[i], Silver);
            }
            helmet(&mut m, Silver);
            shoulders(&mut m, Silver, 0.30);
            m.push(cuboid([0.0, 1.65, -0.29], [0.50, 0.22, 0.04], Grey));
            m.push(cuboid([0.0, 1.70, -0.315], [0.35, 0.035, 0.02], Black));
            m.push(cuboid([0.0, 2.01, 0.02], [0.08, 0.20, 0.32], Blue));
            m.push(cuboid([0.0, 1.08, -0.17], [0.31, 0.72, 0.035], Blue));
            m.push(cuboid([0.0, 1.16, -0.196], [0.035, 0.47, 0.02], Gold));
            belt(&mut m, Grey);
            let arm = &m[2];
            m.push(attached(
                arm,
                [-0.38, 0.99, -0.13],
                [0.10, 0.16, 0.10],
                Grey,
            ));
            m.push(attached(
                &m[2],
                [-0.43, 0.99, -0.19],
                [0.36, 0.53, 0.08],
                Blue,
            ));
            m.push(attached(
                &m[2],
                [-0.43, 0.99, -0.237],
                [0.04, 0.45, 0.02],
                Gold,
            ));
        }
        _ => unreachable!(),
    }
    // Hands and footwear remain joined throughout the existing walk cycle.
    for i in 2..4 {
        let p = &m[i];
        m.push(attached(
            p,
            [p.origin.x, 0.69, 0.0],
            [0.20, 0.13, 0.23],
            if kind == MobType::Knight {
                Silver
            } else {
                Sand
            },
        ));
    }
    feet(
        &mut m,
        4..6,
        if kind == MobType::Knight { Grey } else { Black },
    );
    m
}

fn hood(m: &mut Vec<ModelPart>, tone: Tone) {
    m.push(cuboid([0.0, 1.94, 0.0], [0.61, 0.09, 0.61], tone));
    for x in [-0.28, 0.28] {
        m.push(cuboid([x, 1.66, -0.01], [0.08, 0.51, 0.60], tone));
    }
}

fn helmet(m: &mut Vec<ModelPart>, tone: Tone) {
    m.push(cuboid([0.0, 1.90, 0.0], [0.60, 0.16, 0.59], tone));
    for x in [-0.28, 0.28] {
        m.push(cuboid([x, 1.64, 0.02], [0.055, 0.43, 0.55], tone));
    }
    m.push(cuboid([0.0, 1.78, -0.29], [0.57, 0.055, 0.035], tone));
}

fn shoulders(m: &mut Vec<ModelPart>, tone: Tone, width: f32) {
    for i in 2..4 {
        let p = &m[i];
        m.push(attached(
            p,
            [p.origin.x, 1.40, 0.0],
            [width, 0.23, 0.37],
            tone,
        ));
    }
}

fn belt(m: &mut Vec<ModelPart>, tone: Tone) {
    let depth = m[0].size.z + 0.025;
    m.push(cuboid(
        [0.0, 0.95, 0.0],
        [m[0].size.x + 0.025, 0.09, depth],
        tone,
    ));
    m.push(cuboid(
        [0.0, 0.95, -depth * 0.5 - 0.01],
        [0.08, 0.07, 0.025],
        Gold,
    ));
}

pub(super) fn sage(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    m[1].tex_faces.swap(4, 5);
    m[1].tex_faces[2..5].fill(material(Brown));
    m[6].origin.z = m[6].origin.z.abs();
    m[SATOSHI_GLOW_PART].origin.z = -m[SATOSHI_GLOW_PART].origin.z.abs();
    hood(&mut m, Brown);
    // The amulet keeps index 7 and is still the only emissive part.
    for x in [-0.27, 0.27] {
        m.push(cuboid([x, 0.95, 0.247], [0.035, 1.05, 0.015], Sand));
    }
    m.push(cuboid([0.0, 1.43, -0.18], [0.025, 0.13, 0.025], Gold));
    feet(&mut m, 4..6, Black);
    m
}
