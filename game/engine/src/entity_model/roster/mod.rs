//! Cosmetic anatomy for the remaining roster. Original body/head/limb slots
//! stay in place for Workshop overrides; accessories are appended. Materials
//! reuse the existing paint palette, keeping the 511-layer atlas unchanged.

use super::*;
use crate::block as b;

mod farm;
mod people;
#[cfg(test)]
mod tests;
mod water;
mod wildlife;

#[derive(Clone, Copy)]
enum Tone {
    Black,
    White,
    Grey,
    Silver,
    Brown,
    Sand,
    Pink,
    Purple,
    Gold,
    Red,
    Blue,
}
use Tone::*;

fn material(tone: Tone) -> u32 {
    let colour = match tone {
        Black => b::WALLPAPER_BLACK,
        White => b::WALLPAPER_WHITE,
        Grey => b::WALLPAPER_GREY,
        Silver => b::WALLPAPER_LIGHT_GREY,
        Brown => b::WALLPAPER_BROWN,
        Pink => b::WALLPAPER_PINK,
        Purple => b::WALLPAPER_PURPLE,
        Gold => b::WALLPAPER_YELLOW,
        Red => b::WALLPAPER_RED,
        Blue => b::WALLPAPER_BLUE,
        // A fine-grained, warm neutral surface for muzzles, horn and leather.
        Sand => return b::TEX_SAND,
    };
    b::wallpaper_solid_layer(colour).expect("existing paint colour")
}

fn cuboid(centre: [f32; 3], size: [f32; 3], tone: Tone) -> ModelPart {
    box_part(centre.into(), size.into(), [material(tone); 6])
}

fn dye(part: &mut ModelPart, tone: Tone) {
    part.tex_faces = [material(tone); 6];
}

/// Boots, paws and cuffs rotate around their parent's joint, rather than
/// gaining their own pivot and separating from the limb during a walk.
fn attached(parent: &ModelPart, centre: [f32; 3], size: [f32; 3], tone: Tone) -> ModelPart {
    let mut detail = cuboid(centre, size, tone);
    detail.animated = parent.animated;
    detail.pivot = parent.pivot;
    detail.phase = parent.phase;
    detail
}

fn feet(model: &mut Vec<ModelPart>, limbs: std::ops::Range<usize>, tone: Tone) {
    for i in limbs {
        let p = &model[i];
        let y = p.origin.y - p.size.y * 0.5 + 0.045;
        model.push(attached(
            p,
            [p.origin.x, y, p.origin.z - 0.015],
            [p.size.x + 0.015, 0.09, p.size.z + 0.045],
            tone,
        ));
    }
}

fn eyes(model: &mut Vec<ModelPart>, y: f32, z: f32, separation: f32, width: f32) {
    for x in [-separation, separation] {
        model.push(cuboid([x, y, z], [width, width, 0.014], Black));
        model.push(cuboid(
            [x - width * 0.18, y + width * 0.18, z - 0.009],
            [width * 0.27, width * 0.27, 0.007],
            White,
        ));
    }
}

pub(super) fn polish(kind: MobType, model: Vec<ModelPart>) -> Vec<ModelPart> {
    use MobType::*;
    match kind {
        Cow | Pig | Sheep | Chicken => farm::polish(kind, model),
        Bear | PolarBear | Wolf | Hyena | Horse | Rabbit | Goat | Nostrich => {
            wildlife::polish(kind, model)
        }
        Bee | Squid | GlowSquid | Fish | Shark => water::polish(kind, model),
        Villager | Peddler | Brigand | Marauder | Berserker | Knight => people::polish(kind, model),
        _ => model,
    }
}

pub(super) fn sage(model: Vec<ModelPart>) -> Vec<ModelPart> {
    people::sage(model)
}
