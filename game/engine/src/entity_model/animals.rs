//! Animal silhouettes. Existing part indices retain their body/head/limb roles
//! so saved Workshop face overrides still address the same areas; detail is
//! appended. All parts use existing species coat layers and the live mob path.

use super::*;

fn sheep_body() -> [u32; 6] {
    coat(TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_END)
}

fn sheep_head() -> [u32; 6] {
    // +Z is the back of the head; only -Z carries the face.
    [
        TEX_SHEEP_HEAD_SIDE,
        TEX_SHEEP_HEAD_SIDE,
        TEX_SHEEP_HEAD_TOP,
        TEX_SHEEP_HEAD_SIDE,
        TEX_SHEEP_HEAD_SIDE,
        TEX_SHEEP_HEAD_FRONT,
    ]
}

/// Small feline: short muzzle, upright ears and a slender raised tail.
pub(super) fn cat() -> Vec<ModelPart> {
    let body = sheep_body();
    let head = sheep_head();
    vec![
        box_part(
            Vec3::new(0.0, 0.29, 0.02),
            Vec3::new(0.24, 0.22, 0.45),
            body,
        ),
        box_part(
            Vec3::new(0.0, 0.37, -0.24),
            Vec3::new(0.23, 0.21, 0.20),
            head,
        ),
        box_part(
            Vec3::new(0.0, 0.325, -0.35),
            Vec3::new(0.10, 0.07, 0.04),
            [TEX_SHEEP_BODY_END; 6],
        ),
        box_part(
            Vec3::new(-0.075, 0.485, -0.23),
            Vec3::new(0.07, 0.09, 0.07),
            [TEX_SHEEP_HEAD_TOP; 6],
        ),
        box_part(
            Vec3::new(0.075, 0.485, -0.23),
            Vec3::new(0.07, 0.09, 0.07),
            [TEX_SHEEP_HEAD_TOP; 6],
        ),
        box_part(
            Vec3::new(0.0, 0.35, 0.29),
            Vec3::new(0.055, 0.16, 0.07),
            body,
        ),
        leg_part(
            Vec3::new(-0.075, 0.105, -0.13),
            Vec3::new(0.065, 0.21, 0.075),
            0.0,
            TEX_SHEEP_LEG,
        ),
        leg_part(
            Vec3::new(0.075, 0.105, -0.13),
            Vec3::new(0.065, 0.21, 0.075),
            0.5,
            TEX_SHEEP_LEG,
        ),
        leg_part(
            Vec3::new(-0.075, 0.105, 0.17),
            Vec3::new(0.065, 0.21, 0.075),
            0.5,
            TEX_SHEEP_LEG,
        ),
        leg_part(
            Vec3::new(0.075, 0.105, 0.17),
            Vec3::new(0.065, 0.21, 0.075),
            0.0,
            TEX_SHEEP_LEG,
        ),
        box_part(
            Vec3::new(0.0, 0.47, 0.315),
            Vec3::new(0.055, 0.14, 0.055),
            body,
        ),
        box_part(
            Vec3::new(0.0, 0.55, 0.29),
            Vec3::new(0.055, 0.055, 0.10),
            body,
        ),
        box_part(
            Vec3::new(0.0, 0.34, -0.375),
            Vec3::new(0.04, 0.025, 0.02),
            [crate::block::wallpaper_solid_layer(crate::block::WALLPAPER_PINK).unwrap(); 6],
        ),
    ]
}

/// Low fox with a tapered muzzle and broad, cream-tipped brush.
pub(super) fn fox() -> Vec<ModelPart> {
    let body = sheep_body();
    let head = sheep_head();
    vec![
        box_part(Vec3::new(0.0, 0.34, 0.0), Vec3::new(0.29, 0.27, 0.57), body),
        box_part(
            Vec3::new(0.0, 0.45, -0.36),
            Vec3::new(0.28, 0.23, 0.26),
            head,
        ),
        box_part(
            Vec3::new(0.0, 0.40, -0.515),
            Vec3::new(0.14, 0.10, 0.10),
            [TEX_SHEEP_BODY_END; 6],
        ),
        box_part(
            Vec3::new(-0.09, 0.60, -0.31),
            Vec3::new(0.09, 0.14, 0.07),
            [TEX_SHEEP_HEAD_TOP; 6],
        ),
        box_part(
            Vec3::new(0.09, 0.60, -0.31),
            Vec3::new(0.09, 0.14, 0.07),
            [TEX_SHEEP_HEAD_TOP; 6],
        ),
        box_part(
            Vec3::new(0.0, 0.31, 0.43),
            Vec3::new(0.20, 0.20, 0.38),
            body,
        ),
        leg_part(
            Vec3::new(-0.09, 0.12, -0.19),
            Vec3::new(0.075, 0.24, 0.09),
            0.0,
            TEX_SHEEP_LEG,
        ),
        leg_part(
            Vec3::new(0.09, 0.12, -0.19),
            Vec3::new(0.075, 0.24, 0.09),
            0.5,
            TEX_SHEEP_LEG,
        ),
        leg_part(
            Vec3::new(-0.09, 0.12, 0.19),
            Vec3::new(0.075, 0.24, 0.09),
            0.5,
            TEX_SHEEP_LEG,
        ),
        leg_part(
            Vec3::new(0.09, 0.12, 0.19),
            Vec3::new(0.075, 0.24, 0.09),
            0.0,
            TEX_SHEEP_LEG,
        ),
        box_part(
            Vec3::new(0.0, 0.31, 0.65),
            Vec3::new(0.16, 0.17, 0.14),
            [TEX_SHEEP_BODY_END; 6],
        ),
        box_part(
            Vec3::new(0.0, 0.425, -0.57),
            Vec3::new(0.065, 0.045, 0.025),
            [crate::block::wallpaper_solid_layer(crate::block::WALLPAPER_BLACK).unwrap(); 6],
        ),
    ]
}

/// Broad shell, paired eye stalks, eight walking legs and open pincers.
pub(super) fn crab() -> Vec<ModelPart> {
    let body = sheep_body();
    let mut parts = vec![
        box_part(
            Vec3::new(0.0, 0.16, 0.025),
            Vec3::new(0.40, 0.19, 0.31),
            body,
        ),
        box_part(
            Vec3::new(0.0, 0.17, -0.14),
            Vec3::new(0.25, 0.11, 0.08),
            [TEX_SHEEP_HEAD_SIDE; 6],
        ),
        box_part(
            Vec3::new(-0.09, 0.29, -0.125),
            Vec3::new(0.045, 0.13, 0.045),
            [TEX_SHEEP_HEAD_TOP; 6],
        ),
        box_part(
            Vec3::new(0.09, 0.29, -0.125),
            Vec3::new(0.045, 0.13, 0.045),
            [TEX_SHEEP_HEAD_TOP; 6],
        ),
        box_part(
            Vec3::new(0.0, 0.12, 0.17),
            Vec3::new(0.26, 0.09, 0.07),
            body,
        ),
    ];
    // Retain the rabbit donor's four limb indices, then append the other legs.
    for (x, z, phase) in [
        (-0.225, -0.08, 0.0),
        (0.225, -0.08, 0.5),
        (-0.225, 0.10, 0.5),
        (0.225, 0.10, 0.0),
        (-0.245, -0.02, 0.5),
        (0.245, -0.02, 0.0),
        (-0.23, 0.16, 0.0),
        (0.23, 0.16, 0.5),
    ] {
        let mut leg = leg_part(
            Vec3::new(x, 0.065, z),
            Vec3::new(0.14, 0.055, 0.045),
            phase,
            TEX_SHEEP_LEG,
        );
        leg.pivot = Vec3::new(x * 0.7, 0.09, z);
        parts.push(leg);
    }
    for side in [-1.0, 1.0] {
        let claw = [TEX_SHEEP_HEAD_SIDE; 6];
        parts.push(box_part(
            Vec3::new(side * 0.235, 0.17, -0.20),
            Vec3::new(0.11, 0.09, 0.18),
            body,
        ));
        parts.push(box_part(
            Vec3::new(side * 0.25, 0.18, -0.31),
            Vec3::new(0.14, 0.12, 0.12),
            claw,
        ));
        for offset in [-0.045, 0.045] {
            parts.push(box_part(
                Vec3::new(side * 0.25 + offset, 0.18, -0.405),
                Vec3::new(0.045, 0.085, 0.10),
                claw,
            ));
        }
    }
    parts
}

/// Reindeer keeps the equine body/neck/head/leg indices, adding a branched rack.
pub(super) fn reindeer() -> Vec<ModelPart> {
    let body = coat(TEX_COW_BODY_SIDE, TEX_COW_BODY_TOP, TEX_COW_BODY_END);
    let head = [
        TEX_COW_HEAD_SIDE,
        TEX_COW_HEAD_SIDE,
        TEX_COW_HEAD_TOP,
        TEX_COW_HEAD_SIDE,
        TEX_COW_HEAD_SIDE,
        TEX_COW_HEAD_FRONT,
    ];
    let mut parts = vec![
        box_part(
            Vec3::new(0.0, 0.91, 0.05),
            Vec3::new(0.43, 0.47, 0.90),
            body,
        ),
        box_part(
            Vec3::new(0.0, 1.16, -0.36),
            Vec3::new(0.28, 0.47, 0.29),
            body,
        ),
        box_part(
            Vec3::new(0.0, 1.34, -0.53),
            Vec3::new(0.26, 0.25, 0.41),
            head,
        ),
        leg_part(
            Vec3::new(-0.14, 0.36, -0.28),
            Vec3::new(0.105, 0.72, 0.12),
            0.0,
            TEX_COW_LEG,
        ),
        leg_part(
            Vec3::new(0.14, 0.36, -0.28),
            Vec3::new(0.105, 0.72, 0.12),
            0.5,
            TEX_COW_LEG,
        ),
        leg_part(
            Vec3::new(-0.14, 0.36, 0.36),
            Vec3::new(0.105, 0.72, 0.12),
            0.5,
            TEX_COW_LEG,
        ),
        leg_part(
            Vec3::new(0.14, 0.36, 0.36),
            Vec3::new(0.105, 0.72, 0.12),
            0.0,
            TEX_COW_LEG,
        ),
    ];
    for side in [-1.0, 1.0] {
        parts.push(box_part(
            Vec3::new(side * 0.18, 1.43, -0.40),
            Vec3::new(0.17, 0.07, 0.12),
            body,
        ));
        let antler = [TEX_COW_BODY_END; 6];
        parts.push(box_part(
            Vec3::new(side * 0.13, 1.57, -0.43),
            Vec3::new(0.065, 0.29, 0.065),
            antler,
        ));
        parts.push(box_part(
            Vec3::new(side * 0.23, 1.68, -0.39),
            Vec3::new(0.25, 0.065, 0.08),
            antler,
        ));
        parts.push(box_part(
            Vec3::new(side * 0.33, 1.77, -0.39),
            Vec3::new(0.055, 0.20, 0.065),
            antler,
        ));
        parts.push(box_part(
            Vec3::new(side * 0.20, 1.76, -0.39),
            Vec3::new(0.05, 0.16, 0.06),
            antler,
        ));
        parts.push(box_part(
            Vec3::new(side * 0.13, 1.62, -0.52),
            Vec3::new(0.05, 0.065, 0.18),
            antler,
        ));
    }
    parts.push(box_part(
        Vec3::new(0.0, 1.0, 0.55),
        Vec3::new(0.13, 0.16, 0.19),
        [TEX_COW_BODY_END; 6],
    ));
    parts
}

/// Upright parrot with folded wings, hooked beak and long stepped tail feathers.
pub(super) fn parrot() -> Vec<ModelPart> {
    let body = [TEX_CHICKEN_BODY; 6];
    let wing = [TEX_CHICKEN_WING; 6];
    vec![
        box_part(
            Vec3::new(0.0, 0.30, 0.01),
            Vec3::new(0.21, 0.29, 0.23),
            body,
        ),
        box_part(
            Vec3::new(0.0, 0.50, -0.075),
            Vec3::new(0.23, 0.22, 0.23),
            [
                TEX_CHICKEN_HEAD,
                TEX_CHICKEN_HEAD,
                TEX_CHICKEN_BODY,
                TEX_CHICKEN_BODY,
                TEX_CHICKEN_BODY,
                TEX_CHICKEN_BODY,
            ],
        ),
        box_part(
            Vec3::new(-0.13, 0.30, 0.035),
            Vec3::new(0.065, 0.25, 0.21),
            wing,
        ),
        box_part(
            Vec3::new(0.13, 0.30, 0.035),
            Vec3::new(0.065, 0.25, 0.21),
            wing,
        ),
        leg_part(
            Vec3::new(-0.065, 0.085, 0.0),
            Vec3::new(0.045, 0.17, 0.065),
            0.0,
            TEX_CHICKEN_LEG,
        ),
        leg_part(
            Vec3::new(0.065, 0.085, 0.0),
            Vec3::new(0.045, 0.17, 0.065),
            0.5,
            TEX_CHICKEN_LEG,
        ),
        box_part(
            Vec3::new(0.0, 0.46, -0.215),
            Vec3::new(0.09, 0.11, 0.07),
            [TEX_CHICKEN_LEG; 6],
        ),
        box_part(
            Vec3::new(0.0, 0.405, -0.23),
            Vec3::new(0.06, 0.055, 0.045),
            [TEX_CHICKEN_LEG; 6],
        ),
        box_part(
            Vec3::new(0.0, 0.195, 0.21),
            Vec3::new(0.15, 0.12, 0.27),
            wing,
        ),
        box_part(
            Vec3::new(0.0, 0.13, 0.37),
            Vec3::new(0.095, 0.10, 0.16),
            wing,
        ),
    ]
}

fn pack_equine(is_mule: bool) -> Vec<ModelPart> {
    let mut parts = horse_model();
    // Keep the horse's seven Workshop indices while giving the pack animals
    // shorter legs, a sturdy trunk and their own ears, mane and switch tail.
    let scale = if is_mule {
        Vec3::new(1.05, 0.78, 0.90)
    } else {
        Vec3::new(1.0, 0.72, 0.84)
    };
    for part in &mut parts {
        part.origin *= scale;
        part.size *= scale;
        part.pivot *= scale;
    }
    let body = coat(TEX_COW_BODY_SIDE, TEX_COW_BODY_TOP, TEX_COW_BODY_END);
    parts[2].tex_faces[4] = TEX_COW_HEAD_SIDE;
    // The dark end coat is a mane/tail detail, not a black neck bib.
    parts[1].tex_faces[4] = TEX_COW_BODY_SIDE;
    parts[1].tex_faces[5] = TEX_COW_BODY_SIDE;
    let ears_h = if is_mule { 0.24 } else { 0.30 };
    let head_top = parts[2].origin.y + parts[2].size.y * 0.5;
    let ears_y = head_top + ears_h * 0.5 - 0.02;
    for side in [-1.0, 1.0] {
        parts.push(box_part(
            Vec3::new(side * 0.10, ears_y, -0.47),
            Vec3::new(0.075, ears_h, 0.095),
            body,
        ));
    }
    parts.push(box_part(
        Vec3::new(0.0, 1.27 * scale.y / 0.72, -0.28),
        Vec3::new(0.075, 0.36, 0.10),
        [TEX_COW_BODY_END; 6],
    ));
    parts.push(box_part(
        Vec3::new(0.0, 0.79, 0.57 * scale.z / 0.84),
        Vec3::new(0.065, 0.46, 0.075),
        body,
    ));
    parts.push(box_part(
        Vec3::new(0.0, 0.54, 0.57 * scale.z / 0.84),
        Vec3::new(0.12, 0.13, 0.12),
        [TEX_COW_BODY_END; 6],
    ));
    parts
}

pub(super) fn donkey() -> Vec<ModelPart> {
    pack_equine(false)
}

pub(super) fn mule() -> Vec<ModelPart> {
    pack_equine(true)
}

#[cfg(test)]
mod tests;
