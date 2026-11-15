//! How a held item is carried: the one hook the third-person avatar
//! (`entity_model`) and the first-person hands (`viewmodel`) ask, per item.
//!
//! * [`HoldPose::Default`] - the item rides the hand: a tool or block turns with
//!   its arm, a sprite or cube stands up out of the fist in first person.
//! * [`HoldPose::Shield`] - the shield's own poses (`shield_pose`).
//! * [`HoldPose::Hanging`] - carried by a handle at the fist, the body hanging
//!   below it: the hand lantern. The forearm lifts a little ([`HANGING_ARM_RAISE`],
//!   third person; [`HANGING_FIST_RAISE_NDC`], first person) and the item keeps
//!   upright under its handle while the hand carries it through every arm pose.
//!
//! Only the lantern hangs today; another hanging item (a pail, a bell) joins by
//! returning `Hanging { handle }` from [`hold_pose`], with its own handle height.
//! The pose is keyed off the held item already on the avatar wire, so it needs no
//! new field, packet or save layout.

use crate::item_model::HeldItem;
use crate::protocol::ItemRef;

/// The pose a held item is carried in (see the module).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HoldPose {
    Default,
    Shield,
    /// Hung from a handle: `handle` is how far the grip sits above the item
    /// mesh's own origin, so the caller puts the grip at the fist by moving the
    /// mesh down by that much.
    Hanging { handle: f32 },
}

/// Third person: how much further the arm of a hanging item lifts forward about
/// the shoulder (radians, added to whatever the arm is doing: idle, walk, jump).
pub const HANGING_ARM_RAISE: f32 = 0.5;

/// First person: how far the fist of a hanging item is raised up the view
/// (NDC, 1 = half the view's height), so the body hangs in view and not off the
/// bottom edge.
pub const HANGING_FIST_RAISE_NDC: f32 = 0.30;

/// How the item in a hand is carried.
pub fn hold_pose(held: HeldItem) -> HoldPose {
    if held.tool == Some(crate::crafting::ToolType::Shield) {
        return HoldPose::Shield;
    }
    match held.item {
        ItemRef::Block(id) if id == crate::block::LANTERN => HoldPose::Hanging { handle: crate::lantern::HANDLE_HEIGHT },
        _ => HoldPose::Default,
    }
}

impl HoldPose {
    /// Third person: the extra lift of this hand's arm about the shoulder (radians).
    pub fn arm_raise(self) -> f32 {
        match self {
            HoldPose::Hanging { .. } => HANGING_ARM_RAISE,
            HoldPose::Default | HoldPose::Shield => 0.0,
        }
    }

    /// First person: how far the hand is raised up the view (NDC, 1 = half the view's height).
    pub fn fist_raise_ndc(self) -> f32 {
        match self {
            HoldPose::Hanging { .. } => HANGING_FIST_RAISE_NDC,
            HoldPose::Default | HoldPose::Shield => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::entity_model::{self, avatar_hand_anchor, player_model_for, swing_angle};
    use crate::protocol::ItemRef;
    use crate::item_model::HeldItem;
    use crate::protocol::HeldWire;
    use crate::skin_uv::ArmModel;
    use crate::viewmodel::{self, HandSide};
    use glam::Vec3;

    fn lantern() -> HeldItem {
        ItemRef::Block(block::LANTERN).into()
    }

    #[test]
    fn the_hook_hangs_the_lantern_and_leaves_every_other_item_alone() {
        assert!(matches!(hold_pose(lantern()), HoldPose::Hanging { handle } if handle > 0.1 && handle < 0.3));
        assert_eq!(hold_pose(ItemRef::Block(block::TORCH).into()), HoldPose::Default);
        assert_eq!(hold_pose(ItemRef::Block(block::STONE).into()), HoldPose::Default);
        assert_eq!(hold_pose(HeldItem::EMPTY), HoldPose::Default);
        let sword = crate::item::Item::Tool(Tool::new(ToolType::Sword, ToolMaterial::Iron));
        assert_eq!(hold_pose(HeldItem::of(&sword)), HoldPose::Default);
        let shield = crate::item::Item::Tool(Tool::new(ToolType::Shield, ToolMaterial::Wood));
        assert_eq!(hold_pose(HeldItem::of(&shield)), HoldPose::Shield);
        // The wire form (a peer's avatar) says the same as the local item.
        let wire = HeldWire::of(Some(&crate::item::Item::Block(block::LANTERN)));
        assert_eq!(hold_pose(HeldItem::from_wire(wire)), hold_pose(lantern()));
    }

    fn rot_x(p: Vec3, pivot: Vec3, a: f32) -> Vec3 {
        let (s, c) = a.sin_cos();
        let (dy, dz) = (p.y - pivot.y, p.z - pivot.z);
        Vec3::new(p.x, pivot.y + dy * c - dz * s, pivot.z + dy * s + dz * c)
    }

    /// The held-item vertices of an avatar at the origin with its yaw turned so the
    /// builder's model yaw is the identity, in a pose, with `item` in `side`'s hand.
    fn held_verts(side: HandSide, item: crate::item::Item, anim_state: u8, ticks: u32, swing: f32) -> Vec<crate::mesh::Vertex> {
        let wire = HeldWire::of(Some(&item));
        let mut ps = crate::protocol::PlayerState {
            player_index: 1,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            health: 20.0,
            held_kind: crate::protocol::item_kind::EMPTY,
            held_id: 0,
            anim_state,
            flags: 0,
            skin_key: 0,
            held_tool: HeldWire::NO_TOOL,
            off_kind: crate::protocol::item_kind::EMPTY,
            off_id: 0,
            off_tool: HeldWire::NO_TOOL,
        };
        match side {
            HandSide::Right => {
                ps.held_kind = wire.kind;
                ps.held_id = wire.id;
                ps.held_tool = wire.tool;
            }
            HandSide::Left => {
                ps.off_kind = wire.kind;
                ps.off_id = wire.id;
                ps.off_tool = wire.tool;
            }
        }
        let reg = BlockRegistry::new();
        entity_model::build_player_avatar_vertices(&ps, swing, ticks, 0, 1.0, &reg, ArmModel::Classic).1
    }

    #[test]
    fn a_hanging_lantern_hangs_from_the_hand_through_every_arm_pose() {
        let arms = player_model_for(ArmModel::Classic);
        let reg = BlockRegistry::new();
        let held = crate::item_model::held_item_mesh(lantern(), &reg, HandSide::Right);
        let ys = |v: &[crate::mesh::Vertex]| {
            let lo = v.iter().map(|v| v.position[1]).fold(f32::MAX, f32::min);
            let hi = v.iter().map(|v| v.position[1]).fold(f32::MIN, f32::max);
            (lo, hi)
        };
        let (lo, hi) = ys(&held);
        let height = hi - lo;
        // (label, anim_state, ticks, swing progress)
        let poses: [(&str, u8, u32, f32); 5] = [
            ("idle", 0, 0, 0.0),
            ("walk, arm back", 1, 13, 0.0),
            ("walk, arm forward", 1, 38, 0.0),
            ("airborne", 2, 10, 0.0),
            ("mid swing", 0, 0, 0.35),
        ];
        for side in [HandSide::Right, HandSide::Left] {
            let arm = &arms[side.arm_index()];
            let hand = avatar_hand_anchor(side, ArmModel::Classic);
            for (label, anim, ticks, swing) in poses {
                let swing = if side == HandSide::Right { swing } else { 0.0 };
                // The arm's own angle from the pose rules, plus the lift.
                let base = match anim {
                    _ if swing > 0.0 => swing_angle(swing),
                    1 => (ticks as f32 / 20.0 * 2.5 + arm.phase * std::f32::consts::TAU).sin() * 0.4,
                    2 => -0.4,
                    _ => 0.0,
                };
                let angle = base + HANGING_ARM_RAISE;
                let fist = rot_x(hand, arm.pivot, angle);
                let verts = held_verts(side, crate::item::Item::Block(block::LANTERN), anim, ticks, swing);
                let (vlo, vhi) = ys(&verts);
                assert!(((vhi - vlo) - height).abs() < 1e-3, "{side:?} {label}: the lantern keeps its height, upright");
                assert!((vhi - fist.y).abs() < 0.05 && vhi >= fist.y - 0.01, "{side:?} {label}: the handle's top ({vhi}) is at the fist ({})", fist.y);
                assert!(vlo < fist.y - height * 0.9, "{side:?} {label}: the body hangs below the fist (bottom {vlo}, fist {})", fist.y);
                let n = verts.len() as f32;
                let cx = verts.iter().map(|v| v.position[0]).sum::<f32>() / n;
                let cz = verts.iter().map(|v| v.position[2]).sum::<f32>() / n;
                assert!((cx - fist.x).abs() < 0.05, "{side:?} {label}: under the fist in x ({cx} vs {})", fist.x);
                assert!((cz - fist.z).abs() < 0.05, "{side:?} {label}: under the fist in z ({cz} vs {})", fist.z);
            }
        }
    }

    #[test]
    fn the_hanging_arm_is_lifted_and_a_torch_arm_is_not() {
        // A raised forearm puts the fist further forward (-Z is the front) than the
        // same pose with a torch in the hand.
        let torch = held_verts(HandSide::Right, crate::item::Item::Block(block::TORCH), 0, 0, 0.0);
        let torch_z = torch.iter().map(|v| v.position[2]).sum::<f32>() / torch.len() as f32;
        let lamp = held_verts(HandSide::Right, crate::item::Item::Block(block::LANTERN), 0, 0, 0.0);
        let lamp_z = lamp.iter().map(|v| v.position[2]).sum::<f32>() / lamp.len() as f32;
        assert!(lamp_z < torch_z - 0.15, "the lantern is carried further forward: {lamp_z} vs the torch's {torch_z}");
    }

    fn arm_half_length() -> f32 {
        entity_model::player_model()[viewmodel::RIGHT_ARM_INDEX].size.y * 0.5
    }

    /// First person: the viewmodel's held verts and arm verts in view space.
    fn first_person(held: HeldItem, side: HandSide, aspect: f32) -> (Vec<Vec3>, Vec<Vec3>) {
        let (a, h) = viewmodel::build_viewmodel_vertices(side, held, 0.0, 0.0, 0, &BlockRegistry::new(), ArmModel::Classic, aspect);
        let p = |v: Vec<crate::mesh::Vertex>| v.into_iter().map(|v| Vec3::from(v.position)).collect::<Vec<_>>();
        (p(a), p(h))
    }

    #[test]
    fn first_person_the_fist_is_raised_and_the_lantern_hangs_from_it() {
        for aspect in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0] {
            for side in [HandSide::Right, HandSide::Left] {
                let (torch_arm, _) = first_person(ItemRef::Block(block::TORCH).into(), side, aspect);
                let (arm, item) = first_person(lantern(), side, aspect);
                let mean_y = |v: &[Vec3]| v.iter().map(|p| p.y).sum::<f32>() / v.len() as f32;
                // How far the whole hand went up the view.
                let raise = mean_y(&arm) - mean_y(&torch_arm);
                let half_height = (viewmodel::VIEWMODEL_FOV_Y.to_radians() * 0.5).tan() * viewmodel::slot_depth(aspect);
                let want = HANGING_FIST_RAISE_NDC * half_height;
                assert!((raise - want).abs() < 0.02, "{side:?} @{aspect:.2}: the hand rose {raise}, wanted {want}");
                // The fist (the arm's hand end, raised) and the lantern's top bar.
                let fist = viewmodel::arm_transform(aspect, side, 0.0, 0.0).transform_point3(Vec3::new(0.0, -arm_half_length(), 0.0)) + Vec3::new(0.0, raise, 0.0);
                let top = item.iter().map(|p| p.y).fold(f32::MIN, f32::max);
                let bottom = item.iter().map(|p| p.y).fold(f32::MAX, f32::min);
                assert!((top - fist.y).abs() < 0.06, "{side:?} @{aspect:.2}: the handle's top ({top}) is at the fist ({})", fist.y);
                assert!(bottom < fist.y - 0.3, "{side:?} @{aspect:.2}: the body hangs below the fist ({bottom} vs {})", fist.y);
            }
        }
    }
}
