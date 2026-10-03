//! Local "press A to join" decision logic for split-screen.
//!
//! Cross-platform after spec 15 — the layout/routing maths is pure Rust,
//! and the gamepad backend works on both native (gilrs) and WASM
//! (`navigator.getGamepads()`). Extracted as a free function so it can be
//! unit-tested without standing up a renderer + gamepad runtime.

use glam::Vec3;

use crate::player_intent::PlayerIntent;

/// What the caller needs to seat a new local player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JoinDecision {
    /// 0-indexed slot for the new player (== current `players.len()` at call time).
    pub new_player_index: usize,
    /// Index into `GamepadSystem::gamepads` that this player owns. Bound to the
    /// formula in [`expected_controller_index`] so per-tick input routing
    /// reads the same controller.
    pub gp_idx: usize,
    /// Spawn position — fanned along +X from P1 so successive joiners don't
    /// stack on top of each other.
    pub spawn: Vec3,
}

/// The controller index a given player slot is bound to under the shared
/// routing formula.
///
/// - `player_index == 0` returns `p1_gamepad` (P1 is either KB+M = None or
///   bound to a specific controller via [`Some`]).
/// - `player_index >= 1` returns `Some(base + (player_index - 1))` where
///   `base = 1` if P1 owns a controller, else `0`. So P2/P3/P4 fill the
///   first three unowned slots in index order.
pub fn expected_controller_index(player_index: usize, p1_gamepad: Option<usize>) -> Option<usize> {
    if player_index == 0 {
        return p1_gamepad;
    }
    let base = if p1_gamepad.is_some() { 1 } else { 0 };
    Some(base + (player_index - 1))
}

/// The controller a player's *UI* (crafting slot-cursor etc.) listens to.
///
/// Same formula as [`expected_controller_index`], plus the solo fallback
/// from [`route_intents`]: a solo KB+M P1 with an unowned `gamepads[0]`
/// gets that pad as bonus input, so their UI must listen to it too.
pub fn ui_pad_index(
    player_index: usize,
    p1_gamepad: Option<usize>,
    num_players: usize,
) -> Option<usize> {
    if player_index == 0 && p1_gamepad.is_none() && num_players == 1 {
        return Some(0);
    }
    expected_controller_index(player_index, p1_gamepad)
}

/// Decide whether a new player should join this frame.
///
/// Returns `Some(JoinDecision)` iff:
/// - The lobby has fewer than 4 local players, and
/// - The next-expected controller (per [`expected_controller_index`]) is
///   connected and just pressed A.
///
/// The caller is responsible for actually pushing the slot, allocating GPU
/// resources, and recomputing the screen layout — this function is pure and
/// describes *what* should happen, not *how*.
pub fn try_join_next_player(
    current_player_count: usize,
    p1_gamepad: Option<usize>,
    gamepad_press: impl Fn(usize) -> Option<(bool, bool)>,
    p0_pos: Vec3,
) -> Option<JoinDecision> {
    if current_player_count == 0 || current_player_count >= 4 {
        return None;
    }
    let new_player_index = current_player_count;
    let gp_idx = expected_controller_index(new_player_index, p1_gamepad)?;
    let (connected, a_pressed) = gamepad_press(gp_idx)?;
    if !connected || !a_pressed {
        return None;
    }
    let spawn = Vec3::new(
        p0_pos.x + 3.0 * (new_player_index as f32),
        p0_pos.y,
        p0_pos.z,
    );
    Some(JoinDecision { new_player_index, gp_idx, spawn })
}

/// Build the per-player input intents for one frame given the current
/// keyboard+mouse intent, the per-gamepad intent producer, and the seated
/// player count.
///
/// Mapping (matches [`expected_controller_index`]):
/// - Player 0 (P1) always gets `kbm_intent`. If P1 owns a controller
///   (`p1_gamepad = Some(idx)`), that controller's intent is merged into
///   P1's so console-mode play continues to work and the KB+M user can
///   pair a controller for analog sticks if they want to.
/// - Players 1..N (P2/P3/P4) get the intent for their bound gamepad index.
///   If that controller is disconnected the producer returns `None` and the
///   player gets [`PlayerIntent::default`] — the character stops moving
///   rather than crashing.
pub fn route_intents(
    num_players: usize,
    p1_gamepad: Option<usize>,
    kbm_intent: PlayerIntent,
    gamepad_intent: impl Fn(usize) -> Option<PlayerIntent>,
) -> Vec<PlayerIntent> {
    let mut intents = Vec::with_capacity(num_players);
    for player_idx in 0..num_players {
        let intent = if player_idx == 0 {
            let mut p1 = kbm_intent.clone();
            if let Some(gp_idx) = p1_gamepad {
                // Console mode: P1's chosen controller is theirs.
                if let Some(gp_intent) = gamepad_intent(gp_idx) {
                    p1.merge(&gp_intent);
                }
            } else if num_players == 1 {
                // Solo + KB+M: a connected controller acts as bonus input
                // (analog sticks, triggers) even if the user clicked Play
                // with the mouse. P2 isn't seated so Gamepad[0] is unowned.
                if let Some(gp_intent) = gamepad_intent(0) {
                    p1.merge(&gp_intent);
                }
            }
            p1
        } else {
            let gp_idx = expected_controller_index(player_idx, p1_gamepad)
                .expect("non-zero player index always has an expected controller");
            gamepad_intent(gp_idx).unwrap_or_default()
        };
        intents.push(intent);
    }
    intents
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(connected: bool, a: bool) -> impl Fn(usize) -> Option<(bool, bool)> {
        move |_idx| Some((connected, a))
    }

    /// Build a press-table where index `idx` returns `(connected, a)` from
    /// the slice (panic if out of bounds — tests should size it deliberately).
    fn press_table(table: Vec<(bool, bool)>) -> impl Fn(usize) -> Option<(bool, bool)> {
        move |idx| table.get(idx).copied()
    }

    #[test]
    fn ui_pad_solo_kbm_p1_listens_to_unowned_pad_0() {
        // Mirrors route_intents' solo fallback: pad 0 is unowned, so it
        // drives P1's UI navigation too.
        assert_eq!(ui_pad_index(0, None, 1), Some(0));
    }

    #[test]
    fn ui_pad_multiplayer_kbm_p1_does_not_steal_p2s_pad() {
        // With P2 seated, pad 0 belongs to P2 — P1's UI has no pad.
        assert_eq!(ui_pad_index(0, None, 2), None);
        assert_eq!(ui_pad_index(1, None, 2), Some(0));
    }

    #[test]
    fn ui_pad_console_mode_matches_intent_routing() {
        assert_eq!(ui_pad_index(0, Some(0), 1), Some(0));
        assert_eq!(ui_pad_index(0, Some(0), 3), Some(0));
        assert_eq!(ui_pad_index(1, Some(0), 3), Some(1));
        assert_eq!(ui_pad_index(2, Some(0), 3), Some(2));
    }

    #[test]
    fn p1_on_keyboard_p2_takes_gamepad_0() {
        assert_eq!(expected_controller_index(0, None), None);
        assert_eq!(expected_controller_index(1, None), Some(0));
        assert_eq!(expected_controller_index(2, None), Some(1));
        assert_eq!(expected_controller_index(3, None), Some(2));
    }

    #[test]
    fn p1_on_controller_p2_takes_gamepad_1() {
        assert_eq!(expected_controller_index(0, Some(0)), Some(0));
        assert_eq!(expected_controller_index(1, Some(0)), Some(1));
        assert_eq!(expected_controller_index(2, Some(0)), Some(2));
        assert_eq!(expected_controller_index(3, Some(0)), Some(3));
    }

    #[test]
    fn no_join_when_player_count_at_cap() {
        let decision = try_join_next_player(4, None, press(true, true), Vec3::ZERO);
        assert!(decision.is_none());
    }

    #[test]
    fn no_join_when_player_count_zero() {
        // Guards a logic bug — without P1 seated there's no spawn anchor.
        let decision = try_join_next_player(0, None, press(true, true), Vec3::ZERO);
        assert!(decision.is_none());
    }

    #[test]
    fn no_join_when_expected_controller_disconnected() {
        let decision = try_join_next_player(1, None, press(false, true), Vec3::ZERO);
        assert!(decision.is_none());
    }

    #[test]
    fn no_join_when_a_not_pressed() {
        let decision = try_join_next_player(1, None, press(true, false), Vec3::ZERO);
        assert!(decision.is_none());
    }

    #[test]
    fn p2_joins_on_gamepad_0_when_p1_on_kbm() {
        let table = press_table(vec![(true, true)]);
        let decision = try_join_next_player(1, None, table, Vec3::new(10.0, 80.0, 5.0))
            .expect("P2 should join");
        assert_eq!(decision.new_player_index, 1);
        assert_eq!(decision.gp_idx, 0);
        assert_eq!(decision.spawn, Vec3::new(13.0, 80.0, 5.0));
    }

    #[test]
    fn p3_joins_on_gamepad_1_when_p1_on_kbm() {
        // P1 + P2 already seated; P3 takes gamepad 1.
        let table = press_table(vec![(true, false), (true, true)]);
        let decision = try_join_next_player(2, None, table, Vec3::new(0.0, 80.0, 0.0))
            .expect("P3 should join");
        assert_eq!(decision.new_player_index, 2);
        assert_eq!(decision.gp_idx, 1);
        assert_eq!(decision.spawn, Vec3::new(6.0, 80.0, 0.0));
    }

    #[test]
    fn p3_joins_on_gamepad_2_when_p1_on_controller() {
        // P1 owns gamepad 0, P2 owns gamepad 1; P3 takes gamepad 2.
        let table = press_table(vec![(true, false), (true, false), (true, true)]);
        let decision = try_join_next_player(2, Some(0), table, Vec3::new(0.0, 80.0, 0.0))
            .expect("P3 should join");
        assert_eq!(decision.new_player_index, 2);
        assert_eq!(decision.gp_idx, 2);
    }

    #[test]
    fn p4_joins_on_gamepad_2_when_p1_on_kbm() {
        let table = press_table(vec![(true, false), (true, false), (true, true)]);
        let decision = try_join_next_player(3, None, table, Vec3::ZERO)
            .expect("P4 should join");
        assert_eq!(decision.new_player_index, 3);
        assert_eq!(decision.gp_idx, 2);
        assert_eq!(decision.spawn, Vec3::new(9.0, 0.0, 0.0));
    }

    #[test]
    fn fifth_player_is_a_no_op() {
        // Even with everything pressed, count >= 4 short-circuits.
        let table = press_table(vec![(true, true); 8]);
        assert!(try_join_next_player(4, None, table, Vec3::ZERO).is_none());
    }

    // ── route_intents (Phase 4) ───────────────────────────────────────

    /// Build a deliberately-distinct PlayerIntent so equality checks can
    /// confirm "which gamepad's intent landed in which slot".
    fn marker_intent(marker: f32) -> PlayerIntent {
        PlayerIntent {
            move_forward: marker,
            ..Default::default()
        }
    }

    /// KB+M intent with a distinct movement value so we can verify it
    /// reaches P1 (and only P1).
    fn kbm() -> PlayerIntent {
        marker_intent(-1.0)
    }

    /// Build a gamepad-intent producer from a slice of optional intents.
    /// `None` represents a disconnected controller.
    fn gp_producer(
        intents: Vec<Option<PlayerIntent>>,
    ) -> impl Fn(usize) -> Option<PlayerIntent> {
        move |idx| intents.get(idx).cloned().flatten()
    }

    #[test]
    fn intent_routing_4_players_3_gamepads() {
        // P1 on KB+M; gamepads 0..=2 provide P2/P3/P4 inputs.
        let intents = route_intents(
            4,
            None,
            kbm(),
            gp_producer(vec![
                Some(marker_intent(0.1)),
                Some(marker_intent(0.2)),
                Some(marker_intent(0.3)),
            ]),
        );
        assert_eq!(intents.len(), 4);
        assert_eq!(intents[0].move_forward, -1.0, "P1 must reflect KB+M");
        assert_eq!(intents[1].move_forward, 0.1, "P2 must reflect Gamepad[0]");
        assert_eq!(intents[2].move_forward, 0.2, "P3 must reflect Gamepad[1]");
        assert_eq!(intents[3].move_forward, 0.3, "P4 must reflect Gamepad[2]");
    }

    #[test]
    fn intent_routing_4_players_p1_on_controller() {
        // P1 owns Gamepad[0]; P2/P3/P4 take Gamepad[1..=3].
        let mut p1_pad_intent = marker_intent(0.5);
        p1_pad_intent.sprint = true;
        let intents = route_intents(
            4,
            Some(0),
            kbm(),
            gp_producer(vec![
                Some(p1_pad_intent),
                Some(marker_intent(0.6)),
                Some(marker_intent(0.7)),
                Some(marker_intent(0.8)),
            ]),
        );
        assert_eq!(intents.len(), 4);
        // P1 = KB+M merged with Gamepad[0]. merge() overrides movement if
        // the gamepad has non-zero movement, so P1.move_forward should
        // come from the gamepad (0.5), not the KB+M value (-1.0). Sprint
        // should be the OR — true from the gamepad.
        assert_eq!(intents[0].move_forward, 0.5, "P1 gamepad movement wins over KB+M");
        assert!(intents[0].sprint, "P1 sprint OR'd in from Gamepad[0]");
        assert_eq!(intents[1].move_forward, 0.6, "P2 = Gamepad[1]");
        assert_eq!(intents[2].move_forward, 0.7, "P3 = Gamepad[2]");
        assert_eq!(intents[3].move_forward, 0.8, "P4 = Gamepad[3]");
    }

    #[test]
    fn intent_routing_disconnected_controller_yields_default() {
        // P3's gamepad (Gamepad[1]) disconnects mid-tick. The other three
        // players' intents must be unaffected, and P3 gets a default
        // (no-movement, no-action) intent so the character just stands.
        let intents = route_intents(
            4,
            None,
            kbm(),
            gp_producer(vec![
                Some(marker_intent(0.1)),
                None, // disconnected
                Some(marker_intent(0.3)),
            ]),
        );
        assert_eq!(intents[0].move_forward, -1.0);
        assert_eq!(intents[1].move_forward, 0.1);
        assert_eq!(
            intents[2].move_forward, 0.0,
            "disconnected gamepad → default intent (no movement)"
        );
        assert!(!intents[2].jump_held);
        assert!(!intents[2].break_block);
        assert_eq!(intents[3].move_forward, 0.3);
    }

    #[test]
    fn intent_routing_solo_player_kbm_only() {
        let intents = route_intents(1, None, kbm(), gp_producer(vec![]));
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].move_forward, -1.0);
    }

    #[test]
    fn intent_routing_solo_kbm_with_controller_plugged_in_merges_gamepad() {
        // Existing UX: a solo KB+M player can pick up a controller and have
        // it act as bonus input — they don't have to relaunch via Play-A.
        // Gamepad[0] is unowned (no P2 seated), so it falls through to P1.
        let mut kbm_intent = PlayerIntent::default();
        kbm_intent.move_forward = 0.0; // idle KB
        let intents = route_intents(
            1,
            None,
            kbm_intent,
            gp_producer(vec![Some(marker_intent(0.6))]),
        );
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].move_forward, 0.6, "Gamepad[0] should merge into P1");
    }

    #[test]
    fn intent_routing_multi_kbm_does_not_steal_p2s_gamepad() {
        // The solo-mode fallback must NOT trigger in multi-player — if P2
        // is seated and bound to Gamepad[0], P1 shouldn't get a copy of
        // Gamepad[0]'s intent merged into theirs.
        let mut kbm_intent = PlayerIntent::default();
        kbm_intent.move_forward = -0.5;
        let intents = route_intents(
            2,
            None,
            kbm_intent,
            gp_producer(vec![Some(marker_intent(0.9))]),
        );
        assert_eq!(intents.len(), 2);
        assert_eq!(intents[0].move_forward, -0.5, "P1 stays on KB+M only");
        assert_eq!(intents[1].move_forward, 0.9, "P2 gets Gamepad[0]");
    }

    #[test]
    fn intent_routing_solo_player_console_mode_merges_gamepad() {
        // P1 in console mode: KB+M (idle) + Gamepad[0]. Movement should
        // come from the gamepad since KB+M is idle (move_forward=0).
        let idle_kbm = PlayerIntent::default();
        let intents = route_intents(
            1,
            Some(0),
            idle_kbm,
            gp_producer(vec![Some(marker_intent(0.4))]),
        );
        assert_eq!(intents[0].move_forward, 0.4);
    }

    #[test]
    fn spawn_fans_along_x_from_p1() {
        // Successive players should not stack at the same coords. The X
        // offset = 3.0 * new_player_index — so each new player is 3 units
        // further out than the last.
        let press_a_on = |target_idx: usize| {
            move |idx: usize| Some(if idx == target_idx { (true, true) } else { (true, false) })
        };
        let p0 = Vec3::new(100.0, 70.0, 50.0);
        let p2 = try_join_next_player(1, None, press_a_on(0), p0).unwrap();
        let p3 = try_join_next_player(2, None, press_a_on(1), p0).unwrap();
        let p4 = try_join_next_player(3, None, press_a_on(2), p0).unwrap();
        assert_eq!(p2.spawn.x, 103.0);
        assert_eq!(p3.spawn.x, 106.0);
        assert_eq!(p4.spawn.x, 109.0);
        // Y and Z inherit from P1.
        for d in &[p2, p3, p4] {
            assert_eq!(d.spawn.y, 70.0);
            assert_eq!(d.spawn.z, 50.0);
        }
    }
}
