//! Land animals retain their original Workshop slots and gain species anatomy.
use super::*;

pub(super) fn polish(kind: MobType, m: Vec<ModelPart>) -> Vec<ModelPart> {
    match kind {
        MobType::Bear | MobType::PolarBear => bear(kind, m),
        MobType::Wolf | MobType::Hyena => canine(kind, m),
        MobType::Horse => horse(m),
        MobType::Rabbit => rabbit(m),
        MobType::Goat => goat(m),
        MobType::Nostrich => nostrich(m),
        _ => unreachable!(),
    }
}

fn bear(kind: MobType, mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    let polar = kind == MobType::PolarBear;
    let tone = if polar { White } else { Brown };
    if !polar {
        for p in &mut m {
            dye(p, tone);
        }
    }
    m[0].origin.y = 0.78;
    m[0].size = Vec3::new(0.88, 0.72, if polar { 1.25 } else { 1.16 });
    m[1].origin = Vec3::new(0.0, 1.0, if polar { -0.73 } else { -0.67 });
    m[1].size = Vec3::new(if polar { 0.52 } else { 0.60 }, 0.45, 0.52);
    if polar {
        // Keep the six registered species surfaces and existing leg alias.
        m[1].tex_faces[4] = crate::texture_gen::tinted_layer(kind, TEX_COW_HEAD_TOP).unwrap();
    }
    for p in &mut m[2..6] {
        p.origin.y = 0.30;
        p.size.y = 0.60;
        p.size.z = 0.30;
        p.pivot.y = 0.60;
    }
    let front = m[1].origin.z - 0.26;
    for x in [-0.22, 0.22] {
        m.push(cuboid(
            [x, 1.25, m[1].origin.z + 0.05],
            [0.16, 0.18, 0.13],
            tone,
        ));
        m.push(cuboid(
            [x, 1.26, m[1].origin.z - 0.02],
            [0.08, 0.08, 0.018],
            if polar { Silver } else { Sand },
        ));
    }
    m.push(cuboid(
        [0.0, 0.91, front - 0.06],
        [0.29, 0.18, 0.20],
        if polar { White } else { Sand },
    ));
    m.push(cuboid([0.0, 0.95, front - 0.17], [0.13, 0.07, 0.04], Black));
    eyes(&mut m, 1.09, front - 0.008, 0.17, 0.055);
    m.push(cuboid(
        [0.0, 0.73, if polar { 0.65 } else { 0.61 }],
        [0.13, 0.13, 0.14],
        tone,
    ));
    feet(&mut m, 2..6, if polar { Silver } else { Black });
    m
}

fn canine(kind: MobType, mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    let wolf = kind == MobType::Wolf;
    let tone = if wolf { Silver } else { Sand };
    for p in &mut m {
        dye(p, tone);
    }
    if wolf {
        m[0].origin.y = 0.47;
        m[0].size = Vec3::new(0.34, 0.36, 0.72);
        m[1].origin = Vec3::new(0.0, 0.60, -0.44);
        m[1].size = Vec3::new(0.30, 0.28, 0.30);
        m[2] = cuboid([0.0, 0.54, -0.65], [0.17, 0.13, 0.20], White);
        for i in [3, 4] {
            m[i].origin.y = 0.78;
            m[i].origin.z = -0.43;
            m[i].size = Vec3::new(0.08, 0.16, 0.075);
            m[i].tex_faces[5] = material(Pink);
        }
        m[5] = cuboid([0.0, 0.48, 0.46], [0.15, 0.15, 0.30], Grey);
        for p in &mut m[6..10] {
            p.origin.y = 0.20;
            p.size.y = 0.40;
            p.pivot.y = 0.40;
        }
        m.push(cuboid([0.0, 0.55, -0.756], [0.12, 0.065, 0.025], Black));
        m.push(cuboid([0.0, 0.45, -0.32], [0.26, 0.25, 0.14], White));
        eyes(&mut m, 0.64, -0.596, 0.095, 0.045);
        feet(&mut m, 6..10, Grey);
    } else {
        // High shoulders, dark mane, blunt muzzle and round ears distinguish
        // the hyena from the wolf's level back and pointed ears.
        m[0].size = Vec3::new(0.43, 0.38, 0.80);
        m[1].origin = Vec3::new(0.0, 0.73, -0.48);
        dye(&mut m[1], Sand);
        m.push(cuboid([0.0, 0.70, -0.23], [0.46, 0.28, 0.35], Sand));
        m.push(cuboid([0.0, 0.81, 0.0], [0.12, 0.14, 0.68], Brown));
        for x in [-0.14, 0.14] {
            m.push(cuboid([x, 0.97, -0.44], [0.12, 0.15, 0.09], Brown));
        }
        m.push(cuboid([0.0, 0.65, -0.71], [0.24, 0.18, 0.20], Brown));
        m.push(cuboid([0.0, 0.68, -0.82], [0.13, 0.07, 0.035], Black));
        eyes(&mut m, 0.79, -0.66, 0.11, 0.045);
        m.push(cuboid([0.0, 0.48, 0.46], [0.11, 0.13, 0.23], Brown));
        for x in [-0.22, 0.22] {
            for (y, z) in [(0.57, -0.03), (0.48, 0.19), (0.64, 0.28)] {
                m.push(cuboid([x, y, z], [0.018, 0.055, 0.065], Brown));
            }
        }
        feet(&mut m, 2..6, Brown);
    }
    m
}

fn horse(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    for p in &mut m {
        dye(p, Brown);
    }
    m[0].origin.y = 1.0;
    m[0].size = Vec3::new(0.48, 0.50, 1.05);
    m[1].origin = Vec3::new(0.0, 1.28, -0.43);
    m[2].origin = Vec3::new(0.0, 1.47, -0.63);
    m[2].size = Vec3::new(0.30, 0.29, 0.46);
    for p in &mut m[3..7] {
        p.origin.y = 0.425;
        p.size.y = 0.85;
        p.pivot.y = 0.85;
    }
    for x in [-0.10, 0.10] {
        m.push(cuboid([x, 1.67, -0.52], [0.075, 0.18, 0.10], Brown));
    }
    m.push(cuboid([0.0, 1.31, -0.25], [0.10, 0.53, 0.13], Black));
    m.push(cuboid([0.0, 1.60, -0.49], [0.13, 0.09, 0.28], Black));
    m.push(cuboid([0.0, 1.40, -0.83], [0.31, 0.13, 0.12], Sand));
    m.push(cuboid([0.0, 1.53, -0.865], [0.065, 0.15, 0.012], White));
    eyes(&mut m, 1.52, -0.868, 0.10, 0.04);
    m.push(cuboid([0.0, 0.84, 0.61], [0.10, 0.49, 0.10], Black));
    feet(&mut m, 3..7, Black);
    m
}

fn rabbit(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    for p in &mut m {
        dye(p, Sand);
    }
    for i in [2, 3] {
        m[i].tex_faces[5] = material(Pink);
    }
    dye(&mut m[4], White);
    m.push(cuboid([0.0, 0.26, -0.278], [0.11, 0.065, 0.035], White));
    m.push(cuboid([0.0, 0.28, -0.302], [0.035, 0.025, 0.02], Pink));
    eyes(&mut m, 0.33, -0.276, 0.065, 0.03);
    for i in 7..9 {
        let p = &m[i];
        m.push(attached(
            p,
            [p.origin.x, 0.04, 0.10],
            [0.10, 0.08, 0.23],
            Sand,
        ));
    }
    m
}

fn goat(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    for p in &mut m {
        dye(p, White);
    }
    for (i, x) in [(2, -0.10), (3, 0.10)] {
        m[i] = cuboid([x, 0.95, -0.35], [0.07, 0.22, 0.07], Brown);
        m.push(cuboid([x, 1.07, -0.30], [0.06, 0.08, 0.15], Brown));
    }
    dye(&mut m[4], Silver);
    for x in [-0.20, 0.20] {
        m.push(cuboid([x, 0.78, -0.43], [0.15, 0.075, 0.12], Sand));
    }
    m.push(cuboid([0.0, 0.66, -0.61], [0.21, 0.13, 0.08], Sand));
    eyes(&mut m, 0.77, -0.618, 0.095, 0.04);
    m.push(cuboid([0.0, 0.74, 0.37], [0.10, 0.18, 0.13], White));
    feet(&mut m, 5..9, Black);
    m
}

fn nostrich(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    for p in &mut m {
        dye(p, Purple);
    }
    dye(&mut m[1], Pink);
    dye(&mut m[2], Pink);
    dye(&mut m[3], Gold);
    dye(&mut m[4], White);
    for p in &mut m[5..7] {
        dye(p, Sand);
    }
    // Close the old neck-to-head gap. Feather wings frame the purple body.
    m[1].size.y = 0.78;
    m[1].origin.y = 1.39;
    m[1].origin.z = -0.15; // Inset from the torso front to avoid coplanar flicker.
    for x in [-0.22, 0.22] {
        m.push(cuboid([x, 1.0, 0.03], [0.09, 0.34, 0.48], Purple));
        m.push(cuboid([x, 0.89, 0.13], [0.10, 0.12, 0.30], White));
    }
    for x in [-0.103, 0.103] {
        m.push(cuboid([x, 1.88, -0.30], [0.018, 0.06, 0.07], White));
        m.push(cuboid(
            [x * 1.10, 1.88, -0.31],
            [0.012, 0.035, 0.035],
            Black,
        ));
    }
    for i in 5..7 {
        let p = &m[i];
        m.push(attached(
            p,
            [p.origin.x, 0.035, -0.03],
            [0.13, 0.07, 0.26],
            Sand,
        ));
    }
    m
}
