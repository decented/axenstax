//! Animation-set evaluator (#19 — dynamic/animated asset authoring, Phase B).
//! Solo Buildout Wave 4.
//!
//! The "do-ocracy unlock": an author builds geometry, attaches it to a standard
//! skeleton (`skeleton.rs`), and **inherits** that skeleton's built-in animation
//! set for free — never hand-keyframing. This module is the generic, data-driven
//! version of the inline arithmetic that today drives the player avatar
//! (`entity_model::build_player_avatar_vertices`) and every mob
//! (`build_entity_model_vertices`): walk = `sin(t·freq + phase·τ)·amp` per
//! animated part; attack = `swing_angle(t)` on the swing part (overriding walk);
//! idle = a small breathing sine; jump = a static airborne pose.
//!
//! `eval_anim_set` returns a `PartPose` — the *exact* struct the renderer's
//! `build_part_vertices` already consumes — so a rig animates through the
//! shipping transform with no new motion maths. The look-pitch (`head_pitch`)
//! is added by the renderer from the entity's gaze, not by the clip.

use std::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::entity_model::{swing_angle, PartPose};
use crate::skeleton::{SkeletonKind, SkeletonPart};

/// A named motion clip a skeleton can play. Each standard skeleton supports a
/// subset (a biped walks/attacks; a plant only sways).
///
/// **Wire order is load-bearing.** A placed rig's chosen clip is persisted
/// (`WorldSave.rig_clips`) through bincode, whose enum tags are positional — so
/// variants are APPEND-ONLY here: never reorder or remove one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AnimClip {
    Idle,
    /// The Rig Studio's default, and what every rig placed before the clip
    /// picker existed plays (`WorldSave.rig_clips` defaults to Walk).
    #[default]
    Walk,
    /// Never constructed anywhere, not even a test — only appears as a
    /// match arm in `eval_anim_set`'s vertical-offset table.
    #[allow(dead_code)]
    Jump,
    Attack,
    /// Same story as `Jump` — reachable via `SkeletonKind::idle_clip`, which has
    /// no live caller yet.
    #[cfg_attr(not(test), allow(dead_code))]
    Sway,
    /// #19 Phase C (rung 2) — a gentle whole-rig squash-and-stretch idle: the
    /// creature breathes, squashing on Y and widening on X/Z on a slow sine.
    /// APPENDED variant; keep it last (bincode tags are positional).
    Bounce,
}

impl AnimClip {
    /// The clips the Rig Studio offers an author. Deliberately the three that
    /// read clearly on a standing display rig — `Attack`/`Jump`/`Sway` are driven
    /// by entity state, not picked by hand.
    pub const PICKER: [AnimClip; 3] = [AnimClip::Walk, AnimClip::Idle, AnimClip::Bounce];

    /// Short label for the authoring UI.
    pub fn name(self) -> &'static str {
        match self {
            AnimClip::Idle => "Idle",
            AnimClip::Walk => "Walk",
            AnimClip::Jump => "Jump",
            AnimClip::Attack => "Attack",
            AnimClip::Sway => "Sway",
            AnimClip::Bounce => "Bounce",
        }
    }
}

/// Walk-cycle frequency (rad/s of the gait sine) — the avatar/mob driver's 2.5.
const WALK_FREQ: f32 = 2.5;
/// Walk swing amplitude — the avatar/mob driver's 0.4 rad.
const WALK_AMP: f32 = 0.4;
/// Idle "breathing" — a much smaller, slower sway so an idle rig isn't a statue.
const IDLE_FREQ: f32 = 1.0;
const IDLE_AMP: f32 = 0.05;
/// Plant wind-sway — gentle, mid-frequency.
const SWAY_FREQ: f32 = 1.5;
const SWAY_AMP: f32 = 0.15;
/// Bounce (Phase C rung 2) — a slow breathe/squash. `BOUNCE_AMP` is the peak
/// fraction a part squashes on Y (and correspondingly widens on X/Z), kept small
/// so the rig reads as alive rather than as a wobbling jelly.
const BOUNCE_FREQ: f32 = 2.2;
const BOUNCE_AMP: f32 = 0.12;

impl SkeletonKind {
    /// The part a skeleton swings on `Attack` (the biped's right arm, a bird's
    /// beak peck, a quadruped's head bite). `None` ⇒ the skeleton has no attack.
    pub fn swing_part(self) -> Option<&'static str> {
        match self {
            SkeletonKind::Biped => Some("arm_r"),
            SkeletonKind::Quadruped => Some("head"),
            SkeletonKind::Bird => Some("beak"),
            SkeletonKind::Fish => None,
            SkeletonKind::SwayingPlant => None,
        }
    }

    /// The clip a skeleton plays at rest. No live caller — game_loop.rs only
    /// drives `AnimClip::Walk` today. Tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn idle_clip(self) -> AnimClip {
        match self {
            SkeletonKind::SwayingPlant => AnimClip::Sway,
            _ => AnimClip::Idle,
        }
    }
}

/// Evaluate one skeleton part's pose for `clip` at time `t` (seconds). Pure: the
/// same `(clip, t, part, is_swing_part)` always yields the same `PartPose`. The
/// renderer composes this with the entity's look-pitch + world transform.
pub fn eval_anim_set(clip: AnimClip, t: f32, part: &SkeletonPart, is_swing_part: bool) -> PartPose {
    // Attack: the swing part rolls the mining/placing arc, overriding its walk.
    // Its swing-arc time is `t` wrapped to [0,1] (one strike per second here).
    if clip == AnimClip::Attack && is_swing_part {
        return PartPose {
            walk_swing: 0.0,
            head_pitch: 0.0,
            arm_override: Some(swing_angle(t.fract())),
            ..Default::default()
        };
    }

    // Bounce: a whole-rig squash & stretch (rung 2) — every part, animated or
    // not, breathes about its own pivot. Squash on Y is paired with an equal
    // widen on X/Z so the creature keeps its bulk.
    if clip == AnimClip::Bounce {
        let k = (t * BOUNCE_FREQ + part.phase * TAU).sin() * BOUNCE_AMP;
        return PartPose {
            walk_swing: 0.0,
            head_pitch: 0.0,
            arm_override: None,
            scale: [1.0 + k, 1.0 - k, 1.0 + k],
        };
    }

    if !part.animated {
        return PartPose::default();
    }

    let swing = match clip {
        AnimClip::Idle => (t * IDLE_FREQ + part.phase * TAU).sin() * IDLE_AMP,
        // Non-swing parts keep walking during an attack (the avatar does this).
        AnimClip::Walk | AnimClip::Attack => (t * WALK_FREQ + part.phase * TAU).sin() * WALK_AMP,
        AnimClip::Sway => (t * SWAY_FREQ + part.phase * TAU).sin() * SWAY_AMP,
        // Jump: a single static airborne pose (no oscillation) — legs tucked
        // back, arms forward, transcribing the avatar's jump pose sign.
        AnimClip::Jump => 0.4,
        // Handled above (whole-rig scale channel, no joint rotation).
        AnimClip::Bounce => 0.0,
    };

    PartPose { walk_swing: swing, head_pitch: 0.0, arm_override: None, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &'static str) -> SkeletonPart {
        SkeletonKind::Biped
            .parts()
            .iter()
            .find(|p| p.name == name)
            .copied()
            .expect("biped part")
    }

    #[test]
    fn walk_swings_animated_parts_with_alternating_phase() {
        // arm_l (phase 0.0) and leg_l (phase 0.5) swing in opposition — the
        // diagonal gait the avatar/mob driver produces today.
        let arm_l = part("arm_l");
        let leg_l = part("leg_l");
        let t = 0.5;
        let a = eval_anim_set(AnimClip::Walk, t, &arm_l, false).walk_swing;
        let l = eval_anim_set(AnimClip::Walk, t, &leg_l, false).walk_swing;
        // Opposite phase ⇒ opposite-signed swings at a non-zero t.
        assert!(a.abs() > 0.0 && l.abs() > 0.0);
        assert!(a * l < 0.0, "phase-0 and phase-0.5 parts swing in opposition");
        // Matches the live formula exactly.
        let expected = (t * WALK_FREQ).sin() * WALK_AMP;
        assert!((a - expected).abs() < 1e-6);
    }

    #[test]
    fn static_parts_dont_swing() {
        // Body is not animated → no swing under any locomotion clip.
        let body = part("body");
        assert_eq!(eval_anim_set(AnimClip::Walk, 0.5, &body, false).walk_swing, 0.0);
        assert_eq!(eval_anim_set(AnimClip::Idle, 0.5, &body, false).walk_swing, 0.0);
    }

    #[test]
    fn attack_routes_swing_angle_to_the_swing_part_and_overrides_walk() {
        let arm_r = part("arm_r");
        let t = 0.4;
        let pose = eval_anim_set(AnimClip::Attack, t, &arm_r, true);
        assert_eq!(pose.arm_override, Some(swing_angle(t.fract())));
        // Other animated parts keep walking during the attack.
        let leg_l = part("leg_l");
        let lp = eval_anim_set(AnimClip::Attack, t, &leg_l, false);
        assert!(lp.arm_override.is_none());
        assert!(lp.walk_swing.abs() > 0.0);
    }

    #[test]
    fn idle_is_a_small_sway_not_a_statue_and_smaller_than_walk() {
        let arm_l = part("arm_l");
        let t = 0.5;
        let idle = eval_anim_set(AnimClip::Idle, t, &arm_l, false).walk_swing.abs();
        let walk = eval_anim_set(AnimClip::Walk, t, &arm_l, false).walk_swing.abs();
        assert!(idle > 0.0, "idle breathes");
        assert!(idle < walk, "idle is gentler than walk");
    }

    #[test]
    fn swing_part_mapping_per_skeleton() {
        assert_eq!(SkeletonKind::Biped.swing_part(), Some("arm_r"));
        assert_eq!(SkeletonKind::Fish.swing_part(), None);
        assert_eq!(SkeletonKind::SwayingPlant.idle_clip(), AnimClip::Sway);
        assert_eq!(SkeletonKind::Biped.idle_clip(), AnimClip::Idle);
    }

    #[test]
    fn bounce_squashes_y_and_widens_x_gently_about_the_default_scale() {
        // Phase C rung 2 — the whole-rig breathe. At t=0 the sine is zero, so the
        // clip is a no-op; a quarter-period later it is at peak squash.
        let body = part("body");
        let flat = eval_anim_set(AnimClip::Bounce, 0.0, &body, false);
        assert_eq!(flat.scale, [1.0, 1.0, 1.0], "a bounce starts from rest");
        assert_eq!(flat.walk_swing, 0.0, "bounce is a scale channel, not a rotation");

        let peak = eval_anim_set(AnimClip::Bounce, std::f32::consts::FRAC_PI_2 / BOUNCE_FREQ, &body, false);
        assert!(peak.scale[1] < 1.0, "squashes on Y");
        assert!(peak.scale[0] > 1.0 && peak.scale[2] > 1.0, "widens on X and Z");
        // Y squash and X widen are equal and opposite, and stay gentle.
        assert!((peak.scale[0] - 1.0 - (1.0 - peak.scale[1])).abs() < 1e-6);
        assert!((1.0 - peak.scale[1]) <= BOUNCE_AMP + 1e-6, "kid-pleasing, not jelly");
    }

    #[test]
    fn bounce_moves_static_parts_too_unlike_the_locomotion_clips() {
        // Walk/Idle leave a non-`animated` part alone; Bounce is a whole-creature
        // breathe, so the body scales as well.
        let body = part("body");
        assert!(!body.animated, "precondition: the biped body is a static part");
        let t = 0.4;
        assert_eq!(eval_anim_set(AnimClip::Walk, t, &body, false).scale, [1.0, 1.0, 1.0]);
        assert_ne!(eval_anim_set(AnimClip::Bounce, t, &body, false).scale, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn every_clip_defaults_to_an_identity_scale_so_rotation_clips_are_unchanged() {
        // Phase C's regression contract: only Bounce touches the scale channel.
        for &clip in &[AnimClip::Idle, AnimClip::Walk, AnimClip::Jump, AnimClip::Attack, AnimClip::Sway] {
            for &name in &["head", "body", "arm_l", "leg_r"] {
                let p = part(name);
                for &t in &[0.0, 0.37, 1.9] {
                    assert_eq!(
                        eval_anim_set(clip, t, &p, name == "arm_r").scale,
                        [1.0, 1.0, 1.0],
                        "{clip:?}/{name}@{t} must leave the scale channel alone"
                    );
                }
            }
        }
        assert_eq!(PartPose::default().scale, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn clip_wire_tags_are_append_only() {
        // A placed rig's clip persists through positional bincode
        // (`WorldSave.rig_clips`), so the variant ORDER is a wire format. Pin it.
        let tag = |c: AnimClip| bincode::serialize(&c).unwrap();
        assert_eq!(tag(AnimClip::Idle), 0u32.to_le_bytes());
        assert_eq!(tag(AnimClip::Walk), 1u32.to_le_bytes());
        assert_eq!(tag(AnimClip::Jump), 2u32.to_le_bytes());
        assert_eq!(tag(AnimClip::Attack), 3u32.to_le_bytes());
        assert_eq!(tag(AnimClip::Sway), 4u32.to_le_bytes());
        assert_eq!(tag(AnimClip::Bounce), 5u32.to_le_bytes(), "Bounce was APPENDED last");
        assert_eq!(AnimClip::default(), AnimClip::Walk);
    }

    #[test]
    fn biped_walk_matches_player_avatar_gait_formula() {
        // The regression anchor: the standard biped's walk really is the avatar's
        // motion. The avatar swing is sin(anim_time*2.5 + phase*TAU)*0.4.
        for &name in &["arm_l", "arm_r", "leg_l", "leg_r"] {
            let p = part(name);
            for &t in &[0.0, 0.3, 0.7, 1.4] {
                let got = eval_anim_set(AnimClip::Walk, t, &p, false).walk_swing;
                let want = (t * 2.5 + p.phase * TAU).sin() * 0.4;
                assert!((got - want).abs() < 1e-6, "{name}@{t}");
            }
        }
    }
}
