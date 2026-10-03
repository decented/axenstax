//! PlayerIntent — input abstraction layer.
//!
//! The game loop and physics read PlayerIntent, never raw keys or buttons.
//! Each input source (keyboard/mouse, gamepad, future touch, network) produces
//! one. This enables split-screen (multiple intents), controller-only play, and
//! (Task 1d) server-side physics for remote players — `from_input_packet`
//! reconstructs an intent from the wire representation.

/// All gameplay-relevant input for one frame/tick.
#[derive(Clone, Default)]
pub struct PlayerIntent {
    // --- Continuous (held) ---

    /// Movement direction (XZ plane). Length 0.0-1.0.
    /// Keyboard: digital (0 or 1 per axis, normalized).
    /// Gamepad: analog from left stick.
    pub move_forward: f32,
    pub move_right: f32,

    /// Look delta (yaw, pitch) in radians this frame.
    /// Mouse: raw mouse delta * sensitivity.
    /// Gamepad: right stick * sensitivity * dt.
    pub look_dx: f64,
    pub look_dy: f64,

    /// Sprint modifier. 0.0 = walk, 1.0 = sprint.
    /// Keyboard: 1.0 when Ctrl held. Gamepad: left stick click toggle or trigger.
    pub sprint: bool,

    /// Sneak modifier.
    pub sneak: bool,

    /// Jump held (for swimming up, flying up).
    pub jump_held: bool,

    // --- Per-frame triggers (consumed once) ---

    /// Jump pressed this frame (for ground jump, flight toggle detection).
    pub jump_pressed: bool,

    /// Double-tap jump detected — toggle flight.
    pub toggle_flight: bool,

    /// Break block / attack (left click / left trigger).
    pub break_block: bool,

    /// Place block / use (right click / right trigger).
    pub place_block: bool,

    /// Toggle inventory/crafting UI.
    pub toggle_inventory: bool,

    /// Drop one of the currently-held hotbar item (Q on keyboard).
    pub drop_item: bool,

    /// Pause / escape.
    pub pause: bool,

    /// Hotbar slot selection (0-8), None if unchanged.
    pub hotbar_select: Option<usize>,

    /// Scroll wheel or bumper: positive = prev slot, negative = next slot.
    pub scroll_delta: f32,

    /// Debug overlay toggle (F3).
    pub toggle_debug: bool,

    /// Whether this player has a captured cursor (mouse look active).
    pub cursor_captured: bool,

    /// Spec 24 Phase 8 — rotate the active plan ghost 90° counter-clockwise.
    /// Q on keyboard; shares the key with `drop_item` but the ghost-mode
    /// handler in `game_loop` consumes it and suppresses drop_item when a
    /// ghost is active.
    pub rotate_ghost_ccw: bool,

    /// Spec 24 Phase 8 — rotate the active plan ghost 90° clockwise.
    /// E on keyboard; shares the key with `toggle_inventory` under the
    /// same context-overload rule as `rotate_ghost_ccw`.
    pub rotate_ghost_cw: bool,

    /// Spec 28f — toggle the Inventory Explorer overlay (B key).
    pub toggle_explorer: bool,

    /// Workshop Phase 3: grab the colour off the crosshair cell (eyedropper, G key).
    pub workshop_eyedropper: bool,
    /// Workshop Phase 3: cycle the edit-symmetry mode (M key).
    pub workshop_cycle_symmetry: bool,
    /// Workshop Phase 4: P press → pin the locked ×4 working copy as a global appearance
    /// override, then collapse it ×4→×1. Edge-triggered (once/press).
    pub workshop_pin: bool,
    /// Workshop Phase 5 Task 6: K press → open/close the Wardrobe panel (design gallery).
    /// Edge-triggered (once/press).
    pub workshop_gallery: bool,
    /// Workshop §6 (2026-06-18): V press → toggle the Paint/Sculpt edit mode.
    /// Edge-triggered (once/press). Client-local render concern — never over the wire.
    pub workshop_toggle_mode: bool,
    /// Skin-paint (2026-06-30): Z press → undo the last avatar paint stroke.
    /// Edge-triggered (once/press). Client-local — never over the wire.
    pub workshop_undo: bool,
    /// Campaign S — Shift+Z: redo an undone avatar paint stroke.
    pub workshop_redo: bool,
    /// Campaign S — C: toggle the skin-paint colour picker window.
    pub workshop_picker: bool,
    /// Tab: toggle the skin paint panel (colour wheel + every option). Consumed
    /// ONLY while an avatar paint session is open, so Tab stays free for a
    /// server player list everywhere else. Client-local — never over the wire.
    pub workshop_paint_panel: bool,
    /// R: separate / rejoin the mannequin's limbs so every face can be reached.
    /// Client-local — never over the wire.
    pub workshop_limbs: bool,
    /// B: cycle the skin-paint shading brushes (Brush → Lighten → Darken →
    /// Noise). Shares B with `toggle_explorer` under the same context-overload
    /// rule as `rotate_ghost_cw`. Client-local — never over the wire.
    pub workshop_tool_cycle: bool,
    /// F: toggle the skin-paint Fill tool on/off. Client-local.
    pub workshop_fill: bool,
    /// L: show/hide the skin painter's texel grid + hover footprint.
    /// Client-local — never over the wire.
    pub workshop_grid: bool,
    /// Either Shift held — the skin painter's straight-line modifier
    /// (Shift+click rules a line from the last painted texel). A held
    /// modifier, not an edge; client-local, never over the wire.
    pub shift_down: bool,
    /// Third-person camera (Phase 1): F5 / gamepad-X / touch-View → cycle this
    /// player's perspective (FP → over-shoulder → orbit → FP). Edge-triggered
    /// (once/press). Client-local render concern — never sent over the wire.
    pub camera_cycle: bool,
}

impl PlayerIntent {
    /// Build an intent from a wire `InputPacket` for server-side physics.
    ///
    /// `look_dx`/`look_dy` are zero: absolute yaw/pitch live on the camera,
    /// not on the intent. The server keeps the ServerPlayer's yaw/pitch from
    /// the packet and feeds them into a transient Camera at tick time.
    pub fn from_input_packet(pkt: &crate::protocol::InputPacket) -> Self {
        Self {
            move_forward: pkt.move_forward.clamp(-1.0, 1.0),
            move_right: pkt.move_right.clamp(-1.0, 1.0),
            look_dx: 0.0,
            look_dy: 0.0,
            sprint: pkt.sprint,
            sneak: pkt.sneak,
            jump_held: pkt.jump,
            // jump_pressed is an edge event. Treating the held flag as a press
            // every tick would mis-fire double-tap-jump → flight toggle while
            // jump is held. Leave edge detection to the client side of the
            // wire; trust `toggle_flight` as the authoritative flight toggle.
            jump_pressed: false,
            toggle_flight: pkt.toggle_flight,
            break_block: pkt.break_block,
            place_block: pkt.place_block,
            toggle_inventory: pkt.toggle_inventory,
            drop_item: pkt.drop_item,
            pause: false,
            // Hotbar has 9 slots (0..=8). Reject out-of-range indices rather
            // than panic or silently index-wrap downstream.
            hotbar_select: pkt.hotbar_slot.and_then(|s| (s < 9).then_some(s as usize)),
            scroll_delta: 0.0,
            toggle_debug: false,
            cursor_captured: true,
            // Ghost rotation is purely client-local — the server doesn't
            // need to know about an in-progress placement preview.
            rotate_ghost_ccw: false,
            rotate_ghost_cw: false,
            // Explorer is a client-local overlay; the server doesn't care.
            toggle_explorer: false,
            // Workshop editor keys are client-local; the server doesn't care.
            workshop_eyedropper: false,
            workshop_cycle_symmetry: false,
            workshop_pin: false,
            workshop_gallery: false,
            workshop_toggle_mode: false,
            workshop_undo: false,
            workshop_redo: false,
            workshop_picker: false,
            workshop_paint_panel: false,
            workshop_limbs: false,
            workshop_tool_cycle: false,
            workshop_fill: false,
            workshop_grid: false,
            shift_down: false,
            // Camera perspective is purely client-side render; the server never
            // toggles it on a player's behalf.
            camera_cycle: false,
        }
    }

    /// Merge another intent into this one. The `other` intent overrides movement
    /// if non-zero, accumulates look deltas, and ORs boolean actions.
    pub fn merge(&mut self, other: &PlayerIntent) {
        if other.move_forward.abs() > 0.01 || other.move_right.abs() > 0.01 {
            self.move_forward = other.move_forward;
            self.move_right = other.move_right;
        }
        if other.look_dx.abs() > 0.001 || other.look_dy.abs() > 0.001 {
            self.look_dx += other.look_dx;
            self.look_dy += other.look_dy;
        }
        self.sprint = self.sprint || other.sprint;
        self.sneak = self.sneak || other.sneak;
        self.jump_held = self.jump_held || other.jump_held;
        self.jump_pressed = self.jump_pressed || other.jump_pressed;
        self.toggle_flight = self.toggle_flight || other.toggle_flight;
        self.break_block = self.break_block || other.break_block;
        self.place_block = self.place_block || other.place_block;
        self.toggle_inventory = self.toggle_inventory || other.toggle_inventory;
        self.drop_item = self.drop_item || other.drop_item;
        self.pause = self.pause || other.pause;
        self.toggle_debug = self.toggle_debug || other.toggle_debug;
        self.rotate_ghost_ccw = self.rotate_ghost_ccw || other.rotate_ghost_ccw;
        self.rotate_ghost_cw = self.rotate_ghost_cw || other.rotate_ghost_cw;
        self.toggle_explorer = self.toggle_explorer || other.toggle_explorer;
        self.workshop_eyedropper = self.workshop_eyedropper || other.workshop_eyedropper;
        self.workshop_cycle_symmetry = self.workshop_cycle_symmetry || other.workshop_cycle_symmetry;
        self.workshop_pin = self.workshop_pin || other.workshop_pin;
        self.workshop_gallery = self.workshop_gallery || other.workshop_gallery;
        self.workshop_undo = self.workshop_undo || other.workshop_undo;
        self.workshop_redo = self.workshop_redo || other.workshop_redo;
        self.workshop_picker = self.workshop_picker || other.workshop_picker;
        self.workshop_paint_panel = self.workshop_paint_panel || other.workshop_paint_panel;
        self.workshop_limbs = self.workshop_limbs || other.workshop_limbs;
        self.workshop_tool_cycle = self.workshop_tool_cycle || other.workshop_tool_cycle;
        self.workshop_fill = self.workshop_fill || other.workshop_fill;
        self.workshop_grid = self.workshop_grid || other.workshop_grid;
        self.shift_down = self.shift_down || other.shift_down;
        self.camera_cycle = self.camera_cycle || other.camera_cycle;
        if other.hotbar_select.is_some() {
            self.hotbar_select = other.hotbar_select;
        }
        self.scroll_delta += other.scroll_delta;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::InputPacket;

    fn base() -> InputPacket {
        InputPacket::default()
    }

    #[test]
    fn hotbar_slot_out_of_range_drops_to_none() {
        let mut p = base();
        p.hotbar_slot = Some(99);
        let intent = PlayerIntent::from_input_packet(&p);
        assert_eq!(intent.hotbar_select, None);
    }

    #[test]
    fn hotbar_slot_in_range_passes_through() {
        for slot in 0u8..9 {
            let mut p = base();
            p.hotbar_slot = Some(slot);
            let intent = PlayerIntent::from_input_packet(&p);
            assert_eq!(intent.hotbar_select, Some(slot as usize));
        }
    }

    #[test]
    fn movement_analog_is_clamped_to_unit_square() {
        let mut p = base();
        p.move_forward = 25.0;
        p.move_right = -7.5;
        let intent = PlayerIntent::from_input_packet(&p);
        assert_eq!(intent.move_forward, 1.0);
        assert_eq!(intent.move_right, -1.0);
    }

    #[test]
    fn real_intent_round_trips_through_the_wire() {
        // Guards the contract `network_send_input` now relies on: a local intent
        // packed into an InputPacket (the way the client send path does) must be
        // recoverable by the server's `from_input_packet`, so a server-simulated
        // remote avatar actually moves/animates instead of staying frozen.
        // Field-for-field mirror of the InputPacket the client builds.
        let mut p = base();
        p.move_forward = 0.8;
        p.move_right = -0.5;
        p.sprint = true;
        p.sneak = true;
        p.jump = true;
        p.break_block = true;
        p.place_block = false;
        p.hotbar_slot = Some(3);

        let intent = PlayerIntent::from_input_packet(&p);
        assert_eq!(intent.move_forward, 0.8);
        assert_eq!(intent.move_right, -0.5);
        assert!(intent.sprint);
        assert!(intent.sneak);
        assert!(intent.jump_held, "wire `jump` maps to intent `jump_held`");
        assert!(intent.break_block);
        assert!(!intent.place_block);
        assert_eq!(intent.hotbar_select, Some(3));
        // The whole point: non-zero motion survives the round-trip, so the
        // server's remote-player physics receives real movement.
        assert!(
            intent.move_forward.abs() + intent.move_right.abs() > 0.0,
            "server-simulated avatar must receive non-zero movement intent"
        );
    }

    #[test]
    fn jump_held_does_not_fire_edge_pressed_repeatedly() {
        let mut p = base();
        p.jump = true;
        let intent = PlayerIntent::from_input_packet(&p);
        // jump_held mirrors the wire, jump_pressed must NOT be derived from the
        // held flag — otherwise double-tap detection would misfire every tick.
        assert!(intent.jump_held);
        assert!(!intent.jump_pressed);
    }
}
