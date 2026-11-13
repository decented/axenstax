//! Carried light (off-hand plan, slice 4; Spec 03 §4.7a): the glow of a light a
//! player holds in either hand.
//!
//! **Presentation only.** Nothing here touches the voxel light arrays, so a
//! carried light causes no re-mesh, no mob spawning, no crop growth and nothing on
//! the wire: each client works the lights out from what it already knows of every
//! visible player (the local slots, and `PlayerState.held_*` / `off_*` for a
//! peer). A carried light is not blocked by walls (it only skips surfaces turned
//! away from it), so it can glow faintly through a thin wall: accepted.
//!
//! **Sources.** Any held light, in either hand: a block by its `light_emission`
//! (torch, lantern, paper lantern, glowstone-class and every other emitter) and
//! the two handheld materials, Sparkler (12) and Flare (15). Each hand that holds a
//! light is one light, at that hand: [`HAND_SIDE`] out to its own side and
//! [`HAND_HEIGHT`] up from the holder's feet, turned by its yaw. (The nearest
//! eight over all holders' hands win the camera's eight slots.)
//!
//! **Method.** At most [`MAX_LIGHTS`] point lights ride in the camera uniform
//! (`CameraUniform::carried`), the nearest to the camera first. Each falls off one
//! level per block, like voxel light, and the shader and its CPU twin
//! (`world_light::carried_block`) take the max of the voxel block light and the
//! carried light before the one light rule runs.

use glam::Vec3;

use crate::block::BlockRegistry;
use crate::item::MaterialId;
use crate::protocol::{HeldWire, ItemRef, PlayerState};
use crate::viewmodel::HandSide;

/// The most lights one camera uniform carries (the nearest eight win).
pub const MAX_LIGHTS: usize = 8;
/// Level of a held Sparkler.
pub const SPARKLER_LEVEL: u8 = 12;
/// Level of a held Flare.
pub const FLARE_LEVEL: u8 = 15;
/// How far above a holder's feet a hand's light sits (the avatar's hand anchor is
/// 0.70 up and a held item sits a little above it).
pub const HAND_HEIGHT: f32 = 0.8;
/// How far out to its own side of the body axis a hand's light sits.
pub const HAND_SIDE: f32 = 0.4;
/// How far a first-person hand's light is from the eye, for the viewmodel's own
/// (CPU) lighting: the fist is about this far in front of and below the eye.
pub const VIEWMODEL_DISTANCE: f32 = 0.7;

/// One carried light: where it is and how bright (0..=15, like voxel light).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarriedLight {
    pub pos: Vec3,
    pub level: f32,
}

/// The light level of one held item: 0 for anything that does not shine.
pub fn item_level(item: ItemRef, registry: &BlockRegistry) -> u8 {
    match item {
        ItemRef::Block(id) if registry.is_known(id) => registry.light_emission(id),
        ItemRef::Material(id) => match MaterialId::try_from(id) {
            Ok(MaterialId::Sparkler) => SPARKLER_LEVEL,
            Ok(MaterialId::Flare) => FLARE_LEVEL,
            _ => 0,
        },
        _ => 0,
    }
}

/// The light level of what a hand holds, as the avatar wire carries it.
pub fn wire_level(hand: HeldWire, registry: &BlockRegistry) -> u8 {
    item_level(ItemRef::from_wire(hand.kind, hand.id), registry)
}

/// Where a holder's hand is in the world: `HAND_SIDE` out to the hand's own side
/// (right is +X in the avatar's own frame) and `HAND_HEIGHT` up from `foot`, turned
/// by `yaw` the way the avatar's parts are (`entity_model`: `x' = x cos - z sin`,
/// `z' = x sin + z cos`).
pub fn hand_position(foot: Vec3, yaw: f32, side: HandSide) -> Vec3 {
    let (sin_y, cos_y) = yaw.sin_cos();
    let x = side.sign() * HAND_SIDE;
    foot + Vec3::new(x * cos_y, HAND_HEIGHT, x * sin_y)
}

/// The lights one holder gives: one per hand that holds a light, at that hand.
/// `main` and `off` are the two hands' levels. A non-finite position gives none.
pub fn holder_lights(foot: Vec3, yaw: f32, main: u8, off: u8) -> Vec<CarriedLight> {
    if !(foot.x.is_finite() && foot.y.is_finite() && foot.z.is_finite() && yaw.is_finite()) {
        return Vec::new();
    }
    [(HandSide::Right, main), (HandSide::Left, off)]
        .into_iter()
        .filter(|(_, level)| *level > 0)
        .map(|(side, level)| CarriedLight { pos: hand_position(foot, yaw, side), level: f32::from(level) })
        .collect()
}

/// Every light the visible players carry: each state's two hands (the local
/// slots' own states and a peer's wire alike). The main hand is the right hand,
/// the off-hand the left.
pub fn sources(states: &[PlayerState], registry: &BlockRegistry) -> Vec<CarriedLight> {
    let mut out = Vec::new();
    for ps in states {
        let main = wire_level(HeldWire { kind: ps.held_kind, id: ps.held_id, tool: ps.held_tool }, registry);
        let off = wire_level(HeldWire { kind: ps.off_kind, id: ps.off_id, tool: ps.off_tool }, registry);
        out.extend(holder_lights(Vec3::new(ps.x, ps.y, ps.z), ps.yaw, main, off));
    }
    out
}

/// The block level (0..=1) a first-person hand's light gives the viewmodel (the
/// arm and the held item, lit on the CPU from the eye cell because the viewmodel
/// pass carries no world-space lights): the brighter hand's level less
/// [`VIEWMODEL_DISTANCE`] blocks, through the same twin as the world's.
pub fn viewmodel_level(main: u8, off: u8) -> f32 {
    let level = main.max(off);
    if level == 0 {
        return 0.0;
    }
    let light = CarriedLight { pos: Vec3::ZERO, level: f32::from(level) };
    crate::world_light::carried_block(&[light], Vec3::new(VIEWMODEL_DISTANCE, 0.0, 0.0), 0.0)
}

/// The lights a camera at `camera` is given: the nearest [`MAX_LIGHTS`] of
/// `lights`, nearest first. Ties keep their input order.
pub fn nearest(camera: Vec3, mut lights: Vec<CarriedLight>) -> Vec<CarriedLight> {
    lights.sort_by(|a, b| {
        (a.pos - camera).length_squared().total_cmp(&(b.pos - camera).length_squared())
    });
    lights.truncate(MAX_LIGHTS);
    lights
}

/// The uniform's carried-light block: element 0 holds the count in `x`, elements
/// 1..=8 the lights as `(x, y, z, level)`. Unused slots are zero.
pub fn pack(lights: &[CarriedLight]) -> [[f32; 4]; MAX_LIGHTS + 1] {
    let mut out = crate::camera::CARRIED_LIGHTS_OFF;
    let n = lights.len().min(MAX_LIGHTS);
    out[0][0] = n as f32;
    for (slot, l) in out[1..].iter_mut().zip(lights.iter().take(n)) {
        *slot = [l.pos.x, l.pos.y, l.pos.z, l.level];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    fn reg() -> BlockRegistry {
        BlockRegistry::new()
    }

    #[test]
    fn a_light_emitting_block_is_a_source_at_its_emission() {
        let r = reg();
        assert_eq!(item_level(ItemRef::Block(block::TORCH), &r), 14);
        assert_eq!(item_level(ItemRef::Block(block::LANTERN), &r), 15);
        assert_eq!(item_level(ItemRef::Block(block::PAPER_LANTERN_RED), &r), 12);
        assert_eq!(item_level(ItemRef::Block(block::SALT_LAMP), &r), 14);
        assert_eq!(item_level(ItemRef::Block(block::LAVA), &r), 15);
    }

    #[test]
    fn every_emitting_block_the_registry_knows_is_a_source() {
        // "Any other emitting block" is the rule, not a list: whatever the
        // registry says emits, a hand holding it carries.
        let r = reg();
        let mut emitters = 0;
        for id in 1..=block::LANTERN {
            let e = r.light_emission(id);
            assert_eq!(item_level(ItemRef::Block(id), &r), e, "block {id}");
            if e > 0 {
                emitters += 1;
            }
        }
        assert!(emitters >= 20, "the 16 paper lanterns, the torch, the lantern, lava, fire...: {emitters}");
    }

    #[test]
    fn the_handheld_materials_are_sources_and_other_items_are_not() {
        let r = reg();
        assert_eq!(item_level(ItemRef::Material(MaterialId::Sparkler as u16), &r), 12);
        assert_eq!(item_level(ItemRef::Material(MaterialId::Flare as u16), &r), 15);
        assert_eq!(item_level(ItemRef::Material(MaterialId::Stick as u16), &r), 0);
        assert_eq!(item_level(ItemRef::Material(MaterialId::Bread as u16), &r), 0);
        assert_eq!(item_level(ItemRef::Block(block::STONE), &r), 0, "a non-emitting block");
        assert_eq!(item_level(ItemRef::Tool(2), &r), 0, "a tool");
        assert_eq!(item_level(ItemRef::Empty, &r), 0, "an empty hand");
        assert_eq!(item_level(ItemRef::Block(60_000), &r), 0, "an unknown block id");
        assert_eq!(item_level(ItemRef::Material(60_000), &r), 0, "an unknown material");
    }

    fn state(x: f32, y: f32, z: f32, yaw: f32, main: Option<crate::item::Item>, off: Option<crate::item::Item>) -> PlayerState {
        let (m, o) = (HeldWire::of(main.as_ref()), HeldWire::of(off.as_ref()));
        PlayerState {
            player_index: 1,
            x,
            y,
            z,
            yaw,
            pitch: 0.0,
            health: 20.0,
            held_kind: m.kind,
            held_id: m.id,
            anim_state: 0,
            flags: 0,
            skin_key: 0,
            held_tool: m.tool,
            off_kind: o.kind,
            off_id: o.id,
            off_tool: o.tool,
        }
    }

    #[test]
    fn a_remote_players_held_torch_is_a_source_in_either_hand_and_an_empty_hand_is_not() {
        // The wire form of a peer's hands (`PlayerState.held_*`, `off_*`).
        let r = reg();
        let torch = crate::item::Item::Block(block::TORCH);
        let stone = crate::item::Item::Block(block::STONE);
        let bread = crate::item::Item::Material(MaterialId::Bread);
        let main = sources(&[state(10.0, 64.0, -3.0, 0.0, Some(torch.clone()), None)], &r);
        assert_eq!(main.len(), 1, "a main-hand torch");
        assert_eq!(main[0].level, 14.0);
        let off = sources(&[state(10.0, 64.0, -3.0, 0.0, None, Some(torch.clone()))], &r);
        assert_eq!(off.len(), 1, "an off-hand torch");
        assert_eq!(off[0].level, 14.0);
        assert!(sources(&[state(0.0, 0.0, 0.0, 0.0, None, None)], &r).is_empty(), "empty hands");
        assert!(sources(&[state(0.0, 0.0, 0.0, 0.0, Some(stone), Some(bread))], &r).is_empty(), "non-emitting items");
        let both = sources(&[state(0.0, 0.0, 0.0, 0.0, Some(torch), Some(crate::item::Item::Block(block::LANTERN)))], &r);
        assert_eq!(both.iter().map(|l| l.level).collect::<Vec<_>>(), vec![14.0, 15.0], "each lit hand is its own light");
        let foot = Vec3::new(10.0, 64.0, -3.0);
        assert!(holder_lights(Vec3::new(f32::NAN, 0.0, 0.0), 0.0, 14, 0).is_empty(), "a non-finite position gives no light");
        assert!(holder_lights(foot, f32::INFINITY, 14, 0).is_empty());
    }

    #[test]
    fn a_light_sits_at_its_hand_out_to_its_own_side_and_turns_with_the_holder() {
        let foot = Vec3::new(10.0, 64.0, -3.0);
        let right = hand_position(foot, 0.0, HandSide::Right);
        let left = hand_position(foot, 0.0, HandSide::Left);
        assert_eq!(right, foot + Vec3::new(HAND_SIDE, HAND_HEIGHT, 0.0));
        assert_eq!(left, foot + Vec3::new(-HAND_SIDE, HAND_HEIGHT, 0.0));
        // A quarter turn swings the sides round the body axis, as the avatar's parts turn.
        let q = std::f32::consts::FRAC_PI_2;
        let r = hand_position(foot, q, HandSide::Right) - foot;
        assert!(r.x.abs() < 1e-5 && (r.z - HAND_SIDE).abs() < 1e-5 && (r.y - HAND_HEIGHT).abs() < 1e-4, "{r}");
        // The two hands are always a body width apart and equally far from the axis.
        for yaw in [0.0, 0.7, 2.0, -1.3] {
            let (a, b) = (hand_position(foot, yaw, HandSide::Right), hand_position(foot, yaw, HandSide::Left));
            assert!(((a - b).length() - 2.0 * HAND_SIDE).abs() < 1e-5);
            assert!((((a.x + b.x) * 0.5) - foot.x).abs() < 1e-5 && (((a.z + b.z) * 0.5) - foot.z).abs() < 1e-5);
        }
        // Only a lit hand gives a light, at its own side.
        let lights = holder_lights(foot, 0.0, 0, 14);
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].pos, left);
        assert!(holder_lights(foot, 0.0, 0, 0).is_empty());
    }

    #[test]
    fn the_viewmodel_is_lit_by_its_own_hands_on_the_cpu() {
        // A held lantern or torch lights your own hand: one level less per block
        // of `VIEWMODEL_DISTANCE`, the brighter hand deciding.
        assert_eq!(viewmodel_level(0, 0), 0.0);
        let torch = viewmodel_level(14, 0);
        assert!((torch - (14.0 - VIEWMODEL_DISTANCE) / 15.0).abs() < 1e-6);
        assert_eq!(viewmodel_level(0, 14), torch, "the off-hand lights it too");
        assert!(viewmodel_level(15, 0) > torch && viewmodel_level(14, 15) == viewmodel_level(15, 0));
        assert!(torch > 0.85, "a held torch is bright in its own hand: {torch}");
    }

    #[test]
    fn nine_lights_keep_the_eight_nearest_the_camera_nearest_first() {
        let cam = Vec3::new(0.0, 70.0, 0.0);
        // Distances 1..=9 along x, shuffled.
        let mut lights: Vec<CarriedLight> = [5.0, 9.0, 2.0, 7.0, 1.0, 8.0, 3.0, 6.0, 4.0]
            .iter()
            .map(|d| CarriedLight { pos: Vec3::new(*d, 70.0, 0.0), level: 14.0 })
            .collect();
        let kept = nearest(cam, std::mem::take(&mut lights));
        assert_eq!(kept.len(), MAX_LIGHTS);
        let xs: Vec<f32> = kept.iter().map(|l| l.pos.x).collect();
        assert_eq!(xs, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0], "the ninth, farthest light is dropped");
    }

    #[test]
    fn the_uniform_block_holds_the_count_and_the_lights() {
        let lights = [
            CarriedLight { pos: Vec3::new(1.0, 2.0, 3.0), level: 14.0 },
            CarriedLight { pos: Vec3::new(-4.0, 5.0, 6.0), level: 12.0 },
        ];
        let u = pack(&lights);
        assert_eq!(u[0][0], 2.0);
        assert_eq!(u[1], [1.0, 2.0, 3.0, 14.0]);
        assert_eq!(u[2], [-4.0, 5.0, 6.0, 12.0]);
        assert_eq!(u[3], [0.0; 4]);
        assert_eq!(pack(&[]), crate::camera::CARRIED_LIGHTS_OFF);
        assert_eq!(crate::camera::CARRIED_LIGHTS_OFF[0][0], 0.0, "the off block has count 0");
        // More than eight is capped, never overrun.
        let many = vec![lights[0]; 20];
        assert_eq!(pack(&many)[0][0], MAX_LIGHTS as f32);
    }
}
