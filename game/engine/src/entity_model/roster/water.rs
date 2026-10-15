//! Insects and aquatic silhouettes. Eyes and fins use geometry so a face is
//! never accidentally tiled across an abdomen, tentacle or tail.
use super::*;

pub(super) fn polish(kind: MobType, m: Vec<ModelPart>) -> Vec<ModelPart> {
    match kind {
        MobType::Bee => bee(m),
        MobType::Squid | MobType::GlowSquid => squid(kind, m),
        MobType::Fish => fish(m),
        MobType::Shark => shark(m),
        _ => unreachable!(),
    }
}

fn bee(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    dye(&mut m[1], Black);
    dye(&mut m[4], Black);
    for (i, x) in [(2, -0.245), (3, 0.245)] {
        m[i] = cuboid([x, 0.72, 0.0], [0.29, 0.025, 0.25], White);
    }
    eyes(&mut m, 0.58, -0.368, 0.057, 0.045);
    for x in [-0.065, 0.065] {
        m.push(cuboid([x, 0.69, -0.30], [0.025, 0.13, 0.025], Black));
        m.push(cuboid([x, 0.75, -0.33], [0.04, 0.03, 0.07], Black));
        for z in [-0.10, 0.03, 0.14] {
            m.push(cuboid([x * 1.9, 0.37, z], [0.025, 0.09, 0.025], Black));
        }
    }
    m
}

fn squid(kind: MobType, mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    let layer = if kind == MobType::GlowSquid {
        crate::texture_gen::tinted_layer(kind, TEX_SQUID_BODY).unwrap()
    } else {
        TEX_SQUID_BODY
    };
    let coat = [layer; 6];
    m[0] = box_part(Vec3::new(0.0, 0.64, 0.0), Vec3::new(0.42, 0.42, 0.42), coat);
    m[1] = box_part(Vec3::new(0.0, 0.41, 0.0), Vec3::new(0.46, 0.12, 0.46), coat);
    // The original four arm slots remain 2..=5; four more fill the sides.
    let roots = [
        (-0.14, -0.14),
        (0.14, -0.14),
        (-0.14, 0.14),
        (0.14, 0.14),
        (-0.20, 0.0),
        (0.20, 0.0),
        (0.0, -0.20),
        (0.0, 0.20),
    ];
    for (i, (x, z)) in roots.into_iter().enumerate() {
        let mut arm = leg_part(
            Vec3::new(x, 0.19, z),
            Vec3::new(0.065, 0.38, 0.065),
            if i % 2 == 0 { 0.0 } else { 0.5 },
            layer,
        );
        arm.pivot.y = 0.38;
        if i < 4 {
            m[i + 2] = arm;
        } else {
            m.push(arm);
        }
    }
    m.push(box_part(
        Vec3::new(0.0, 0.88, 0.0),
        Vec3::new(0.30, 0.10, 0.30),
        coat,
    ));
    for x in [-0.235_f32, 0.235] {
        m.push(cuboid([x, 0.44, -0.07], [0.026, 0.11, 0.13], White));
        m.push(cuboid(
            [x * 1.07, 0.44, -0.09],
            [0.014, 0.055, 0.055],
            Black,
        ));
    }
    // Small lateral mantle fins join the upper body.
    for x in [-0.22, 0.22] {
        m.push(box_part(
            Vec3::new(x, 0.72, 0.02),
            Vec3::new(0.10, 0.08, 0.25),
            coat,
        ));
    }
    m
}

fn fish(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    // All models face -Z, including fish. The former disconnected tail at
    // -Z was on the nose side; slot 1 now trails the body at +Z.
    m[0] = cuboid([0.0, 0.17, 0.0], [0.12, 0.16, 0.32], Silver);
    m[1] = cuboid([0.0, 0.17, 0.25], [0.035, 0.21, 0.10], Gold);
    m.push(cuboid([0.0, 0.17, 0.18], [0.055, 0.07, 0.09], Silver));
    m.push(cuboid([0.0, 0.17, -0.16], [0.10, 0.11, 0.07], Grey));
    m.push(cuboid([0.0, 0.27, 0.02], [0.025, 0.08, 0.15], Grey));
    m.push(cuboid([0.0, 0.105, -0.01], [0.10, 0.04, 0.25], White));
    for x in [-0.08, 0.08] {
        m.push(cuboid([x, 0.14, 0.015], [0.065, 0.025, 0.12], Gold));
        m.push(cuboid(
            [x * 0.79, 0.20, -0.115],
            [0.014, 0.045, 0.045],
            Black,
        ));
    }
    m
}

fn shark(mut m: Vec<ModelPart>) -> Vec<ModelPart> {
    m[0] = cuboid([0.0, 0.43, 0.0], [0.42, 0.34, 1.05], Grey);
    m[1] = cuboid([0.0, 0.69, 0.06], [0.055, 0.23, 0.25], Grey);
    m[2] = cuboid([0.0, 0.46, 0.77], [0.065, 0.40, 0.16], Grey);
    m.push(cuboid([0.0, 0.43, 0.59], [0.17, 0.16, 0.26], Grey));
    m.push(cuboid([0.0, 0.43, -0.59], [0.31, 0.23, 0.24], Grey));
    m.push(cuboid([0.0, 0.31, -0.05], [0.39, 0.12, 0.93], Silver));
    m.push(cuboid([0.0, 0.34, -0.59], [0.24, 0.025, 0.18], Black));
    m.push(cuboid([0.0, 0.32, -0.60], [0.27, 0.025, 0.17], White));
    for x in [-0.26_f32, 0.26] {
        m.push(cuboid([x, 0.34, 0.05], [0.26, 0.055, 0.31], Grey));
        m.push(cuboid([x * 0.61, 0.48, -0.60], [0.024, 0.055, 0.06], Black));
        for z in [-0.30, -0.23, -0.16] {
            m.push(cuboid([x * 0.82, 0.45, z], [0.016, 0.13, 0.022], Black));
        }
    }
    m
}
