//! Placement guards — pure rules for whether a block placement is allowed.
//!
//! Extracted from the inline checks in `game_loop.rs` so the rules are
//! unit-testable. Eventual crate home: `genesis_client`.

/// Player collision half-width used by the placement overlap check
/// (matches the player AABB in `physics.rs`).
pub const PLAYER_HALF_WIDTH: f32 = 0.3;
/// Player collision height used by the placement overlap check.
pub const PLAYER_HEIGHT: f32 = 1.8;

/// Would placing a block at `place` be blocked because this player is
/// standing in that cell?
///
/// Solid blocks can't materialise inside a player's collision box — that
/// would push them out or trap them. Non-solid blocks (cable, track,
/// torch, sapling…) have no collision at all, so a player standing in the
/// cell is no obstacle: laying a cable at your own feet is the natural
/// "put it on the ground" gesture and must work ("you cant put the cables
/// on the ground", #cable-ground).
pub fn placement_blocked_by_player(
    place: (i32, i32, i32),
    player_pos: (f32, f32, f32),
    block_solid: bool,
) -> bool {
    if !block_solid {
        return false;
    }
    let (bx, by, bz) = (place.0 as f32, place.1 as f32, place.2 as f32);
    let (px, py, pz) = player_pos;
    bx + 1.0 > px - PLAYER_HALF_WIDTH
        && bx < px + PLAYER_HALF_WIDTH
        && by + 1.0 > py
        && by < py + PLAYER_HEIGHT
        && bz + 1.0 > pz - PLAYER_HALF_WIDTH
        && bz < pz + PLAYER_HALF_WIDTH
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Player standing at the centre of cell (0, 0, 0).
    const STANDING: (f32, f32, f32) = (0.5, 0.0, 0.5);

    #[test]
    fn solid_block_in_players_cell_is_blocked() {
        assert!(placement_blocked_by_player((0, 0, 0), STANDING, true));
        // Head-height cell of the same column is inside the 1.8-tall AABB too.
        assert!(placement_blocked_by_player((0, 1, 0), STANDING, true));
    }

    #[test]
    fn solid_block_away_from_player_is_allowed() {
        assert!(!placement_blocked_by_player((3, 0, 0), STANDING, true));
        // Two above the head is clear of the 1.8-tall AABB.
        assert!(!placement_blocked_by_player((0, 2, 0), STANDING, true));
    }

    #[test]
    fn non_solid_block_in_players_cell_is_allowed() {
        // The cable-on-the-ground gesture: aim at your feet, place. A block
        // with no collision can share the player's cell.
        assert!(!placement_blocked_by_player((0, 0, 0), STANDING, false));
    }
}
