//! The meta a hand placement stamps on its block, hoisted out of the game
//! loop's place arm (C3d run 9) so a Strict client's typed `Edit` carries it
//! BEFORE anything is predicted, and the place arm writes the same value.
//! Pure: the placed block, the clicked face and the camera's forward. The
//! order is the place arm's: a shaped block's orientation, then a piston's,
//! dispenser's or dropper's look facing, then a directional power device's
//! axis facing (a cable or a signal input keeps what came before).
use crate::block::{self, BlockId};

/// The meta placing `placed` on `face` while looking along `fwd` stamps;
/// `None` when it stamps none (the cell keeps the meta it had, as
/// `World::set_block` leaves it).
pub fn placement_meta(placed: BlockId, face: [i32; 3], fwd: glam::Vec3) -> Option<u8> {
    let mut stamped = None;
    // F1 — shaped-block orientation (slab half / stair facing) from the
    // clicked face + camera, so it renders + collides correctly.
    if crate::block_shape::is_shaped(placed) {
        let m = match crate::block_shape::shape_of(placed) {
            // A hand-placed snow layer starts at one
            // thin layer (AUX 0 = 1/8 block).
            crate::block_shape::BlockShape::SnowLayer => 0,
            crate::block_shape::BlockShape::Slab => {
                let f = crate::block_shape::slab_placement_facing(
                    face, 0.5,
                );
                crate::meta::with_facing(0, f)
            }
            crate::block_shape::BlockShape::Stairs => {
                let (f, top) = crate::block_shape::stairs_placement(
                    fwd.x, fwd.z, face,
                );
                let base = crate::meta::with_facing(0, f);
                if top {
                    crate::meta::with_state(base, 1)
                } else {
                    base
                }
            }
            crate::block_shape::BlockShape::FenceGate => {
                // Faces the player's axis; placed closed.
                let f = crate::block_shape::facing_from_forward(
                    fwd.x, fwd.z,
                );
                crate::meta::with_facing(0, f)
            }
            crate::block_shape::BlockShape::Trapdoor => {
                // Hinges to the wall the player faces; a
                // click on a block's underside mounts it
                // at the ceiling (state bit 1).
                let f = crate::block_shape::facing_from_forward(
                    fwd.x, fwd.z,
                );
                let base = crate::meta::with_facing(0, f);
                if matches!(face, [0, -1, 0]) {
                    crate::meta::with_state(base, 0b10)
                } else {
                    base
                }
            }
            // Connecting shapes derive their
            // geometry from live neighbours, so
            // they store no orientation meta.
            crate::block_shape::BlockShape::Pane
            | crate::block_shape::BlockShape::Wall => 0,
            crate::block_shape::BlockShape::Door => {
                // Bottom half: faces the player,
                // left hinge, closed. The top half
                // is placed by the door block below.
                let f = crate::block_shape::facing_from_forward(
                    fwd.x, fwd.z,
                );
                crate::meta::with_facing(0, f)
            }
            // Buttons + levers mount on the face
            // the player clicked (protrude along
            // its outward normal); placed off.
            crate::block_shape::BlockShape::Button
            | crate::block_shape::BlockShape::Lever => {
                let f = crate::block_shape::facing_from_face_normal(
                    face,
                );
                crate::meta::with_facing(0, f)
            }
            // Pressure plates always lie on the floor.
            crate::block_shape::BlockShape::PressurePlate => 0,
            // Sign board faces across the player's
            // view so the text reads toward them.
            crate::block_shape::BlockShape::Sign => {
                let f = crate::block_shape::facing_from_forward(
                    fwd.x, fwd.z,
                );
                crate::meta::with_facing(0, f)
            }
            // Item frame mounts on the clicked wall.
            crate::block_shape::BlockShape::ItemFrame => {
                let f = crate::block_shape::facing_from_face_normal(
                    face,
                );
                crate::meta::with_facing(0, f)
            }
            crate::block_shape::BlockShape::FullCube => 0,
        };
        stamped = Some(m);
    }
    // P11 — a piston is a plain FullCube, but it must remember which way to
    // push (pitch included, so up/down pistons work); a dispenser or dropper
    // takes the same facing stamp.
    if crate::piston::is_piston(placed) || placed == block::DISPENSER || placed == block::DROPPER {
        stamped = Some(crate::meta::with_facing(0, crate::piston::facing_from_look(fwd.x, fwd.y, fwd.z)));
    }
    // Spec 48 — a directional power device faces the player's dominant
    // horizontal axis. Wave 2c — an input device (button/lever/plate) keeps
    // its clicked-face facing above.
    if block::is_power_block(placed) && !block::is_cable(placed) && !crate::block_shape::is_signal_input(placed) {
        stamped = Some(crate::meta::with_facing(0, device_facing(fwd)));
    }
    stamped
}

/// A directional power device's facing: the dominant horizontal axis of `fwd`.
pub fn device_facing(fwd: glam::Vec3) -> crate::meta::Facing {
    if fwd.x.abs() > fwd.z.abs() {
        if fwd.x > 0.0 { crate::meta::Facing::East } else { crate::meta::Facing::West }
    } else if fwd.z > 0.0 {
        crate::meta::Facing::South
    } else {
        crate::meta::Facing::North
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_blocks_stamp_nothing_and_directional_blocks_face_the_look() {
        let north = glam::Vec3::new(0.0, 0.0, -1.0);
        assert_eq!(placement_meta(block::COBBLESTONE, [0, 1, 0], north), None, "a full cube keeps the cell's meta");
        let piston = placement_meta(block::DISPENSER, [0, 1, 0], glam::Vec3::new(0.0, -1.0, 0.0));
        assert_eq!(piston, Some(crate::meta::with_facing(0, crate::piston::facing_from_look(0.0, -1.0, 0.0))), "a dispenser faces the look, pitch included");
        assert_eq!(device_facing(glam::Vec3::new(1.0, 0.0, 0.2)), crate::meta::Facing::East);
        assert_eq!(device_facing(north), crate::meta::Facing::North);
    }
}
