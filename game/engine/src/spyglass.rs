//! The Spyglass (pt1124, 0.2.43): pure rules for looking through one.
//!
//! A spyglass is a `ToolType::Spyglass` held in either hand. It replaced free
//! zoom: the Zoom key (`Action::Zoom`, C by default) now zooms only while a
//! spyglass is in a hand, and it looks exactly as a held right-click does.
//!
//! **Two triggers, one look.** A held use button puts it to the eye by the
//! shield's rule ([`crate::hands::seat_raise`]: the last claimant of the click,
//! the main hand's first, decided when the press starts and kept until it is
//! released). The Zoom key (or the touch Zoom button) held with a spyglass in
//! either hand is the second trigger. [`look_frame`] folds both into one state
//! per seat, frame by frame. While looking:
//! - the view zooms (`Camera::set_zoom`, the Zoom FOV setting);
//! - a round lens covers the view (`hud_ui::paint_spyglass_lens`) and the
//!   first-person hand hides behind it;
//! - the body walks at sneak speed with no sprint (`PlayerIntent::sighting`,
//!   on the wire as `InputPacket.spyglass_raised` so a joiner's server body
//!   agrees);
//! - nothing is mined, attacked or placed.
//!
//! **What ends it.** Letting go ends it at once. So do opening any screen and
//! pausing (the caller ends the state, [`LookState::end`]), changing the hotbar
//! slot and the hand swap (F): [`look_frame`] sees those itself, and a trigger
//! still held stays down until it is let go and pressed again (the press's
//! decision stands, as a shield's does).

use glam::{Mat4, Vec3};

use crate::hands::{Hand, Press};
use crate::item::Item;
use crate::viewmodel::HandSide;

/// Is `item` a spyglass?
pub fn is_spyglass(item: Option<&Item>) -> bool {
    matches!(item, Some(Item::Tool(t)) if t.tool_type == crate::crafting::ToolType::Spyglass)
}

/// The hand whose spyglass a look uses: the main hand's if it holds one, else the
/// off-hand's. The same order as every raise ([`crate::hands::raised_item`]).
pub fn spyglass_hand(main: Option<&Item>, off: Option<&Item>) -> Option<Hand> {
    if is_spyglass(main) {
        Some(Hand::Main)
    } else if is_spyglass(off) {
        Some(Hand::Off)
    } else {
        None
    }
}

/// One seat's look, kept between frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LookState {
    /// The Zoom key's press, classified when it goes down and kept until it is
    /// released (the shield's [`Press`]).
    pub key: Press,
    /// While looking: the hotbar slot the look began at. A change of slot ends it.
    pub anchor: Option<usize>,
}

impl LookState {
    /// The look ends with no trigger to hold down: a screen opened, the game
    /// paused, the cursor freed, the body dead or seated. A trigger still held
    /// when play comes back is a new press.
    pub fn end(&mut self) {
        *self = LookState::default();
    }

    /// Is this seat looking (as of the last [`look_frame`])?
    #[cfg(test)]
    pub fn looking(&self) -> bool {
        self.anchor.is_some()
    }
}

/// What one frame of a seat in play brings to the look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LookFrame {
    /// The Zoom key (or the touch Zoom button) is held.
    pub key_held: bool,
    /// The use press's raise is up and it lifts a spyglass
    /// (`hands::seat_raise` gave `Raise::Spyglass` when the press started).
    pub raised: bool,
    /// A spyglass is in either hand.
    pub spyglass_in_hand: bool,
    /// The selected hotbar slot.
    pub hotbar_slot: usize,
    /// The hands were swapped (F) this frame.
    pub swapped: bool,
    /// The seat cannot look this frame: a screen or panel over its view, the
    /// cursor free, dead or seated. The look ends ([`LookState::end`]) and a
    /// trigger still held afterwards is a new press.
    pub blocked: bool,
}

/// What [`look_frame`] decided.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LookOut {
    /// Looking through the spyglass this frame.
    pub looking: bool,
    /// The look is the Zoom key's (the use button is free: the caller blocks
    /// placing with it).
    pub by_key: bool,
    /// The look was interrupted while the use press held it up: the caller keeps
    /// that press down until the button is released.
    pub end_raise: bool,
}

/// One frame of a seat's look, in play (see the module doc). Pure.
pub fn look_frame(state: &mut LookState, f: LookFrame) -> LookOut {
    if f.blocked {
        state.end();
        return LookOut::default();
    }
    // A change of slot or a swap ends a look in progress; whatever still holds
    // it up stays down until it is let go.
    if let Some(anchor) = state.anchor
        && (f.swapped || anchor != f.hotbar_slot)
    {
        state.key = if f.key_held { Press::Decided { raised: false } } else { Press::Idle };
        state.anchor = None;
        return LookOut { looking: false, by_key: false, end_raise: f.raised };
    }
    let (hold, key) = crate::hands::shield_press(state.key, f.key_held, f.spyglass_in_hand, || true);
    state.key = key;
    let looking = hold.raised || f.raised;
    state.anchor = looking.then_some(f.hotbar_slot);
    LookOut { looking, by_key: hold.raised, end_raise: false }
}

// ── The raised pose (third person) ──────────────────────────────────────────

/// Third person: the angle the arm holding a raised spyglass is lifted forward
/// about its shoulder (radians), and how far it swings in toward the middle (the
/// eating raise's shape: `eating::AVATAR_ARM_ANGLE` / `AVATAR_ARM_YAW`), so the
/// fist comes up under the eye.
pub const AVATAR_ARM_ANGLE: f32 = 1.85;
pub const AVATAR_ARM_YAW: f32 = 0.40;

/// The raised arm's turn toward the middle for the hand `side`.
pub fn avatar_arm_yaw(side: HandSide) -> f32 {
    side.sign() * AVATAR_ARM_YAW
}

/// The eye the spyglass is held to, in the avatar's model space (feet at the
/// origin, facing -Z): the front of the head, on the hand's side.
pub fn avatar_eye(side: HandSide) -> Vec3 {
    Vec3::new(side.sign() * 0.12, 1.62, -0.27)
}

/// The neck joint the head (and the raised spyglass) nods about with the look
/// pitch: the head part's pivot (`entity_model::PLAYER_MODEL`).
pub const AVATAR_NECK: Vec3 = Vec3::new(0.0, 1.325, 0.0);

/// The raised spyglass's size on the avatar, as a share of the held model.
pub const AVATAR_SCALE: f32 = 0.85;

/// Where a raised spyglass mesh (`held_tool_model`'s frame: the tube along +Y,
/// its eyepiece end at [`crate::held_tool_model::SPYGLASS_EYEPIECE_Y`]) goes in
/// the avatar's model space: turned to lie along the look (+Y to -Z), its
/// eyepiece at the eye on the hand's side, then nodded with the head by `pitch`
/// about the neck, so it points where the player looks.
pub fn avatar_transform(side: HandSide, pitch: f32) -> Mat4 {
    Mat4::from_translation(AVATAR_NECK)
        * Mat4::from_rotation_x(pitch)
        * Mat4::from_translation(-AVATAR_NECK)
        * Mat4::from_translation(avatar_eye(side))
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_scale(Vec3::splat(AVATAR_SCALE))
        * Mat4::from_translation(Vec3::new(0.0, -crate::held_tool_model::SPYGLASS_EYEPIECE_Y, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};

    fn spy() -> Item {
        Item::Tool(Tool::new(ToolType::Spyglass, ToolMaterial::Wood))
    }

    fn frame(key_held: bool, raised: bool, in_hand: bool, slot: usize) -> LookFrame {
        LookFrame { key_held, raised, spyglass_in_hand: in_hand, hotbar_slot: slot, swapped: false, blocked: false }
    }

    #[test]
    fn a_spyglass_is_found_in_either_hand_main_first() {
        let pick = Item::Tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
        assert!(is_spyglass(Some(&spy())));
        assert!(!is_spyglass(Some(&pick)) && !is_spyglass(None));
        assert_eq!(spyglass_hand(Some(&spy()), Some(&spy())), Some(Hand::Main));
        assert_eq!(spyglass_hand(Some(&pick), Some(&spy())), Some(Hand::Off));
        assert_eq!(spyglass_hand(Some(&pick), None), None);
    }

    #[test]
    fn the_zoom_key_looks_only_with_a_spyglass_in_a_hand() {
        let mut s = LookState::default();
        // C with no spyglass: nothing, and picking one up while it is held does not start it.
        assert_eq!(look_frame(&mut s, frame(true, false, false, 0)), LookOut::default());
        assert!(!look_frame(&mut s, frame(true, false, true, 0)).looking, "the press was decided without one");
        assert!(!look_frame(&mut s, frame(false, false, true, 0)).looking, "released");
        // C with a spyglass in a hand: looking, by the key, until it is let go.
        let out = look_frame(&mut s, frame(true, false, true, 0));
        assert!(out.looking && out.by_key);
        assert!(look_frame(&mut s, frame(true, false, true, 0)).looking, "held");
        assert!(!look_frame(&mut s, frame(false, false, true, 0)).looking, "letting go ends it at once");
        assert!(!s.looking());
    }

    #[test]
    fn a_raised_spyglass_looks_the_same_as_the_key() {
        let mut s = LookState::default();
        let out = look_frame(&mut s, frame(false, true, true, 3));
        assert!(out.looking && !out.by_key, "the use button's raise");
        assert!(s.looking());
        assert!(!look_frame(&mut s, frame(false, false, true, 3)).looking, "lowered");
    }

    #[test]
    fn a_slot_change_or_a_swap_ends_the_look_until_the_trigger_is_let_go() {
        // The key: a slot change ends it and holding on does not bring it back.
        let mut s = LookState::default();
        assert!(look_frame(&mut s, frame(true, false, true, 0)).looking);
        assert!(!look_frame(&mut s, frame(true, false, true, 1)).looking, "slot changed");
        assert!(!look_frame(&mut s, frame(true, false, true, 1)).looking, "still held: stays down");
        assert!(!look_frame(&mut s, frame(false, false, true, 1)).looking);
        assert!(look_frame(&mut s, frame(true, false, true, 1)).looking, "a new press looks again");
        // F: the swap ends it, even with the spyglass still in a hand.
        let swapped = LookFrame { swapped: true, ..frame(true, false, true, 1) };
        assert!(!look_frame(&mut s, swapped).looking);
        assert!(!look_frame(&mut s, frame(true, false, true, 1)).looking, "still held: stays down");
        // The use button's raise: interrupted, and the caller is told to keep the press down.
        let mut s = LookState::default();
        assert!(look_frame(&mut s, frame(false, true, true, 2)).looking);
        let out = look_frame(&mut s, frame(false, true, true, 5));
        assert_eq!(out, LookOut { looking: false, by_key: false, end_raise: true });
        let out = look_frame(&mut s, LookFrame { swapped: true, ..frame(false, false, true, 5) });
        assert!(!out.looking && !out.end_raise, "nothing up, nothing to end");
    }

    #[test]
    fn a_blocked_frame_never_looks_and_a_held_key_after_it_is_a_new_press() {
        let mut s = LookState::default();
        assert!(look_frame(&mut s, frame(true, false, true, 0)).looking);
        // A screen over the view (one that keeps the cursor, too): no look, with C still held.
        for _ in 0..3 {
            let blocked = LookFrame { blocked: true, ..frame(true, true, true, 0) };
            assert_eq!(look_frame(&mut s, blocked), LookOut::default());
            assert_eq!(s, LookState::default());
        }
        // Closed with C still held and the spyglass in hand: a new press, so it looks.
        assert!(look_frame(&mut s, frame(true, false, true, 0)).looking);
    }

    #[test]
    fn a_screen_or_the_pause_ends_the_look() {
        let mut s = LookState::default();
        assert!(look_frame(&mut s, frame(true, false, true, 0)).looking);
        s.end();
        assert!(!s.looking());
        assert_eq!(s, LookState::default(), "a key still held after the screen is a new press");
    }

    #[test]
    fn the_raised_spyglass_lies_along_the_look_with_its_eyepiece_at_the_eye() {
        use crate::held_tool_model::{SPYGLASS_EYEPIECE_Y, SPYGLASS_OBJECTIVE_Y};
        for side in [HandSide::Right, HandSide::Left] {
            let m = avatar_transform(side, 0.0);
            let eye = m.transform_point3(Vec3::new(0.0, SPYGLASS_EYEPIECE_Y, 0.0));
            let lens = m.transform_point3(Vec3::new(0.0, SPYGLASS_OBJECTIVE_Y, 0.0));
            assert!((eye - avatar_eye(side)).length() < 1e-4, "{side:?}: eyepiece at the eye ({eye})");
            let axis = (lens - eye).normalize();
            assert!(axis.dot(Vec3::NEG_Z) > 0.999, "{side:?}: along the look, forward ({axis})");
            // Looking up, it points up with the head.
            let up = avatar_transform(side, 0.6);
            let axis_up = (up.transform_point3(Vec3::new(0.0, SPYGLASS_OBJECTIVE_Y, 0.0))
                - up.transform_point3(Vec3::new(0.0, SPYGLASS_EYEPIECE_Y, 0.0)))
            .normalize();
            assert!(axis_up.y > 0.5, "{side:?}: nods with the look ({axis_up})");
        }
        assert!(avatar_eye(HandSide::Right).x > 0.0 && avatar_eye(HandSide::Left).x < 0.0);
    }
}
