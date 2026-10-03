//! Standard-skeleton rig data model (#19 — dynamic/animated asset authoring,
//! Phase A). Solo Buildout Wave 4.
//!
//! The motion machinery already ships: `entity_model::{ModelPart, PartPose,
//! build_part_vertices, swing_angle}` rotate cuboid parts about a pivot and one
//! walk-cycle drives ~17 mob skeletons today. #19 does two things on top:
//!   1. make the **skeleton** (named parts + pivots + gait phase) *authored
//!      data* instead of a hardcoded Rust `*_model()` builder, and
//!   2. let an author **attach baked #18 micro-models** to named parts and
//!      **inherit** the skeleton's animation set (`anim_set.rs`, Phase B).
//!
//! This module is the data layer: the five **standard skeletons** as
//! `LazyLock` data (mirroring `entity_model::MODEL_CACHE`), the serialisable
//! `RiggedModel` an author builds, and a bridge to `ModelPart` so a rig drives
//! the existing renderer. The build-geometry / tag-parts / place-pivots
//! **authoring UX** and the micro-model-geometry emit are the Test Session 3
//! playtest boundary — NOT in this module.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::entity_model::ModelPart;

/// One named joint of a standard skeleton. A superset of `ModelPart` with a
/// `name` (the author's attach target) and an optional `parent` (hierarchy, for
/// the swaying-plant chain + future bends).
#[derive(Clone, Copy, Debug)]
pub struct SkeletonPart {
    pub name: &'static str,
    /// Index into `Skeleton.parts` of this part's parent, or `None` for a root.
    /// Read only by the tests below (the parent-chain-terminates + hierarchy
    /// assertions) — the render/attach path (Phase B, not yet built) is the
    /// production consumer this awaits.
    #[cfg_attr(not(test), allow(dead_code))]
    pub parent: Option<usize>,
    pub origin: Vec3,
    pub size: Vec3,
    pub pivot: Vec3,
    /// Gait offset (0.0 / 0.5 for alternating limbs).
    pub phase: f32,
    /// Does this part swing with the walk cycle?
    pub animated: bool,
    /// Does this part pitch with the entity's look (the head)?
    pub pitch_tracks_look: bool,
}

impl SkeletonPart {
    /// Bridge to the renderer's `ModelPart` so a rig drives the existing
    /// `build_part_vertices` path. `tex_faces` is a placeholder — a rigged part
    /// draws its attached micro-model's geometry (Phase A render integration,
    /// the playtest boundary), not a flat-textured cuboid.
    pub fn to_model_part(self) -> ModelPart {
        ModelPart {
            origin: self.origin,
            size: self.size,
            pivot: self.pivot,
            animated: self.animated,
            phase: self.phase,
            tex_faces: [0; 6],
            pitch_tracks_look: self.pitch_tracks_look,
        }
    }
}

/// Which standard skeleton (and thus which built-in animation set, Phase B) a
/// rig uses. Stored on the serialisable `RiggedModel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkeletonKind {
    Biped,
    Quadruped,
    Bird,
    Fish,
    SwayingPlant,
}

impl SkeletonKind {
    pub const ALL: [SkeletonKind; 5] = [
        SkeletonKind::Biped,
        SkeletonKind::Quadruped,
        SkeletonKind::Bird,
        SkeletonKind::Fish,
        SkeletonKind::SwayingPlant,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SkeletonKind::Biped => "biped",
            SkeletonKind::Quadruped => "quadruped",
            SkeletonKind::Bird => "bird",
            SkeletonKind::Fish => "fish",
            SkeletonKind::SwayingPlant => "swaying_plant",
        }
    }

    /// The skeleton's ordered named parts.
    pub fn parts(self) -> &'static [SkeletonPart] {
        match self {
            SkeletonKind::Biped => &BIPED,
            SkeletonKind::Quadruped => &QUADRUPED,
            SkeletonKind::Bird => &BIRD,
            SkeletonKind::Fish => &FISH,
            SkeletonKind::SwayingPlant => &SWAYING_PLANT,
        }
    }

    /// Index of the part named `name`, if any.
    pub fn part_index(self, name: &str) -> Option<usize> {
        self.parts().iter().position(|p| p.name == name)
    }
}

/// An author-built rig: a standard skeleton + a baked micro-model attached to
/// some of its parts. Serialisable so it rides Stash like `PlanData`. The
/// micro-model is referenced by the placeable block id it was baked onto (#18
/// `MicroModelRegistry` keys by `BlockId`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiggedModel {
    pub skeleton: SkeletonKind,
    pub name: String,
    pub parts: Vec<RiggedPart>,
}

/// One attached part: which skeleton part, which baked micro-model, and an
/// optional pivot nudge off the skeleton's default.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiggedPart {
    /// Index into the skeleton's `parts()`.
    pub skeleton_part: usize,
    /// The baked micro-model's block id (#18). 0/AIR ⇒ part is geometry-less.
    pub micro_model_block: BlockId,
    /// Override the skeleton's default pivot for this part, if the author moved it.
    pub pivot_override: Option<[f32; 3]>,
}

impl RiggedModel {
    /// A fresh rig on `skeleton` with no geometry attached yet.
    pub fn new(skeleton: SkeletonKind, name: impl Into<String>) -> Self {
        Self { skeleton, name: name.into(), parts: Vec::new() }
    }

    /// Attach (or replace) a baked micro-model on the named skeleton part.
    /// Returns false if the name isn't part of this skeleton.
    pub fn attach(&mut self, part_name: &str, micro_model_block: BlockId) -> bool {
        let Some(idx) = self.skeleton.part_index(part_name) else {
            return false;
        };
        match self.parts.iter_mut().find(|p| p.skeleton_part == idx) {
            Some(p) => p.micro_model_block = micro_model_block,
            None => self.parts.push(RiggedPart {
                skeleton_part: idx,
                micro_model_block,
                pivot_override: None,
            }),
        }
        true
    }

    /// The effective pivot for a rigged part (override if set, else the
    /// skeleton's default).
    pub fn pivot_of(&self, rigged: &RiggedPart) -> Vec3 {
        match rigged.pivot_override {
            Some([x, y, z]) => Vec3::new(x, y, z),
            None => self.skeleton.parts()[rigged.skeleton_part].pivot,
        }
    }
}

// ── The five standard skeletons (data, defined once) ──────────────────────────
//
// Biped is a direct transcription of `entity_model::PLAYER_MODEL`; the others
// transcribe the existing mob builders' part layouts (cow / chicken / squid).
// Pivots are non-degenerate joints; gait phases alternate diagonally.

const BIPED: [SkeletonPart; 6] = [
    SkeletonPart { name: "head", parent: Some(1), origin: Vec3::new(0.0, 1.575, 0.0), size: Vec3::new(0.5, 0.5, 0.5), pivot: Vec3::new(0.0, 1.325, 0.0), phase: 0.0, animated: false, pitch_tracks_look: true },
    SkeletonPart { name: "body", parent: None, origin: Vec3::new(0.0, 0.975, 0.0), size: Vec3::new(0.5, 0.7, 0.25), pivot: Vec3::ZERO, phase: 0.0, animated: false, pitch_tracks_look: false },
    SkeletonPart { name: "arm_l", parent: Some(1), origin: Vec3::new(-0.375, 1.05, 0.0), size: Vec3::new(0.25, 0.7, 0.25), pivot: Vec3::new(-0.375, 1.4, 0.0), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "arm_r", parent: Some(1), origin: Vec3::new(0.375, 1.05, 0.0), size: Vec3::new(0.25, 0.7, 0.25), pivot: Vec3::new(0.375, 1.4, 0.0), phase: 0.5, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "leg_l", parent: Some(1), origin: Vec3::new(-0.125, 0.3125, 0.0), size: Vec3::new(0.25, 0.625, 0.25), pivot: Vec3::new(-0.125, 0.625, 0.0), phase: 0.5, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "leg_r", parent: Some(1), origin: Vec3::new(0.125, 0.3125, 0.0), size: Vec3::new(0.25, 0.625, 0.25), pivot: Vec3::new(0.125, 0.625, 0.0), phase: 0.0, animated: true, pitch_tracks_look: false },
];

const QUADRUPED: [SkeletonPart; 7] = [
    SkeletonPart { name: "body", parent: None, origin: Vec3::new(0.0, 0.75, 0.0), size: Vec3::new(0.55, 0.5, 1.0), pivot: Vec3::ZERO, phase: 0.0, animated: false, pitch_tracks_look: false },
    SkeletonPart { name: "head", parent: Some(0), origin: Vec3::new(0.0, 0.95, -0.6), size: Vec3::new(0.4, 0.4, 0.4), pivot: Vec3::new(0.0, 0.85, -0.45), phase: 0.0, animated: false, pitch_tracks_look: true },
    SkeletonPart { name: "leg_fl", parent: Some(0), origin: Vec3::new(-0.2, 0.25, -0.35), size: Vec3::new(0.18, 0.5, 0.18), pivot: Vec3::new(-0.2, 0.5, -0.35), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "leg_fr", parent: Some(0), origin: Vec3::new(0.2, 0.25, -0.35), size: Vec3::new(0.18, 0.5, 0.18), pivot: Vec3::new(0.2, 0.5, -0.35), phase: 0.5, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "leg_bl", parent: Some(0), origin: Vec3::new(-0.2, 0.25, 0.35), size: Vec3::new(0.18, 0.5, 0.18), pivot: Vec3::new(-0.2, 0.5, 0.35), phase: 0.5, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "leg_br", parent: Some(0), origin: Vec3::new(0.2, 0.25, 0.35), size: Vec3::new(0.18, 0.5, 0.18), pivot: Vec3::new(0.2, 0.5, 0.35), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "tail", parent: Some(0), origin: Vec3::new(0.0, 0.85, 0.55), size: Vec3::new(0.1, 0.1, 0.3), pivot: Vec3::new(0.0, 0.85, 0.5), phase: 0.0, animated: true, pitch_tracks_look: false },
];

const BIRD: [SkeletonPart; 7] = [
    SkeletonPart { name: "body", parent: None, origin: Vec3::new(0.0, 0.4, 0.0), size: Vec3::new(0.3, 0.35, 0.4), pivot: Vec3::ZERO, phase: 0.0, animated: false, pitch_tracks_look: false },
    SkeletonPart { name: "head", parent: Some(0), origin: Vec3::new(0.0, 0.62, -0.18), size: Vec3::new(0.25, 0.25, 0.25), pivot: Vec3::new(0.0, 0.55, -0.1), phase: 0.0, animated: false, pitch_tracks_look: true },
    SkeletonPart { name: "beak", parent: Some(1), origin: Vec3::new(0.0, 0.6, -0.33), size: Vec3::new(0.1, 0.08, 0.12), pivot: Vec3::new(0.0, 0.6, -0.28), phase: 0.0, animated: false, pitch_tracks_look: false },
    SkeletonPart { name: "wing_l", parent: Some(0), origin: Vec3::new(-0.2, 0.45, 0.0), size: Vec3::new(0.08, 0.3, 0.35), pivot: Vec3::new(-0.15, 0.5, 0.0), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "wing_r", parent: Some(0), origin: Vec3::new(0.2, 0.45, 0.0), size: Vec3::new(0.08, 0.3, 0.35), pivot: Vec3::new(0.15, 0.5, 0.0), phase: 0.5, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "leg_l", parent: Some(0), origin: Vec3::new(-0.1, 0.12, 0.0), size: Vec3::new(0.06, 0.25, 0.06), pivot: Vec3::new(-0.1, 0.24, 0.0), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "leg_r", parent: Some(0), origin: Vec3::new(0.1, 0.12, 0.0), size: Vec3::new(0.06, 0.25, 0.06), pivot: Vec3::new(0.1, 0.24, 0.0), phase: 0.5, animated: true, pitch_tracks_look: false },
];

const FISH: [SkeletonPart; 4] = [
    SkeletonPart { name: "body", parent: None, origin: Vec3::new(0.0, 0.5, 0.0), size: Vec3::new(0.22, 0.3, 0.6), pivot: Vec3::ZERO, phase: 0.0, animated: false, pitch_tracks_look: false },
    SkeletonPart { name: "tail", parent: Some(0), origin: Vec3::new(0.0, 0.5, 0.42), size: Vec3::new(0.05, 0.3, 0.25), pivot: Vec3::new(0.0, 0.5, 0.3), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "fin_l", parent: Some(0), origin: Vec3::new(-0.15, 0.5, -0.05), size: Vec3::new(0.12, 0.12, 0.18), pivot: Vec3::new(-0.1, 0.5, -0.05), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "fin_r", parent: Some(0), origin: Vec3::new(0.15, 0.5, -0.05), size: Vec3::new(0.12, 0.12, 0.18), pivot: Vec3::new(0.1, 0.5, -0.05), phase: 0.5, animated: true, pitch_tracks_look: false },
];

// Swaying plant: a static base + a chain of segments, each parented to the one
// below (rung-3 segmented bend via Phase B's parent composition). Phase rises up
// the chain so the wind-sway ripples toward the tip.
const SWAYING_PLANT: [SkeletonPart; 4] = [
    SkeletonPart { name: "base", parent: None, origin: Vec3::new(0.0, 0.15, 0.0), size: Vec3::new(0.2, 0.3, 0.2), pivot: Vec3::ZERO, phase: 0.0, animated: false, pitch_tracks_look: false },
    SkeletonPart { name: "segment_1", parent: Some(0), origin: Vec3::new(0.0, 0.45, 0.0), size: Vec3::new(0.18, 0.3, 0.18), pivot: Vec3::new(0.0, 0.3, 0.0), phase: 0.0, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "segment_2", parent: Some(1), origin: Vec3::new(0.0, 0.75, 0.0), size: Vec3::new(0.16, 0.3, 0.16), pivot: Vec3::new(0.0, 0.6, 0.0), phase: 0.25, animated: true, pitch_tracks_look: false },
    SkeletonPart { name: "segment_3", parent: Some(2), origin: Vec3::new(0.0, 1.05, 0.0), size: Vec3::new(0.14, 0.3, 0.14), pivot: Vec3::new(0.0, 0.9, 0.0), phase: 0.5, animated: true, pitch_tracks_look: false },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_standard_skeleton_has_named_parts_with_real_pivots() {
        for k in SkeletonKind::ALL {
            let parts = k.parts();
            assert!(!parts.is_empty(), "{} has parts", k.name());
            // Names are unique within a skeleton.
            for (i, p) in parts.iter().enumerate() {
                assert!(!p.name.is_empty());
                assert!(
                    parts.iter().skip(i + 1).all(|q| q.name != p.name),
                    "duplicate part name {} in {}",
                    p.name,
                    k.name()
                );
                // A parent index, if present, is a valid non-self index, and
                // following the parent chain terminates (no cycle). Parents need
                // NOT precede children — the biped head (index 0) parents the body
                // (index 1), matching PLAYER_MODEL's [head, body, …] order.
                if let Some(par) = p.parent {
                    assert_ne!(par, i, "{}::{} can't parent itself", k.name(), p.name);
                    assert!(par < parts.len(), "{}::{} parent index in range", k.name(), p.name);
                    let mut cur = p.parent;
                    let mut hops = 0;
                    while let Some(c) = cur {
                        hops += 1;
                        assert!(hops <= parts.len(), "{}::{} parent chain cycles", k.name(), p.name);
                        cur = parts[c].parent;
                    }
                }
                // Animated parts have a pivot off the origin centre (a real joint).
                if p.animated {
                    assert!(p.pivot != Vec3::ZERO || p.name == "body");
                }
            }
        }
    }

    #[test]
    fn biped_matches_player_model_layout() {
        // The biped standard skeleton is a transcription of PLAYER_MODEL — same
        // part count, same head/arm/leg pivots (the regression anchor the spec
        // calls for).
        let pm = crate::entity_model::player_model();
        assert_eq!(BIPED.len(), pm.len());
        for (sk, mp) in BIPED.iter().zip(pm.iter()) {
            assert_eq!(sk.pivot, mp.pivot, "pivot mismatch for {}", sk.name);
            assert_eq!(sk.origin, mp.origin, "origin mismatch for {}", sk.name);
            assert_eq!(sk.animated, mp.animated);
            assert_eq!(sk.phase, mp.phase);
            assert_eq!(sk.pitch_tracks_look, mp.pitch_tracks_look);
        }
        // The head is the look-tracking part; arms/legs alternate diagonally.
        assert_eq!(SkeletonKind::Biped.part_index("head"), Some(0));
        assert!(BIPED[SkeletonKind::Biped.part_index("head").unwrap()].pitch_tracks_look);
    }

    #[test]
    fn to_model_part_bridges_to_renderer() {
        let head = &BIPED[0];
        let mp = head.to_model_part();
        assert_eq!(mp.pivot, head.pivot);
        assert_eq!(mp.pitch_tracks_look, head.pitch_tracks_look);
    }

    #[test]
    fn rigged_model_attach_and_serde_round_trip() {
        let mut rig = RiggedModel::new(SkeletonKind::Quadruped, "my cow");
        assert!(rig.attach("leg_fl", crate::block::OAK_PLANKS));
        assert!(rig.attach("head", crate::block::STONE));
        // Re-attaching the same part replaces, doesn't duplicate.
        assert!(rig.attach("leg_fl", crate::block::COBBLESTONE));
        assert_eq!(rig.parts.len(), 2);
        // Unknown part name is refused.
        assert!(!rig.attach("wing_l", crate::block::STONE));

        let json = serde_json::to_string(&rig).unwrap();
        let back: RiggedModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back.skeleton, SkeletonKind::Quadruped);
        assert_eq!(back.name, "my cow");
        assert_eq!(back.parts.len(), 2);
    }

    #[test]
    fn pivot_override_wins_over_skeleton_default() {
        let mut rig = RiggedModel::new(SkeletonKind::Biped, "x");
        rig.attach("head", crate::block::STONE);
        let default_pivot = rig.pivot_of(&rig.parts[0]);
        assert_eq!(default_pivot, BIPED[0].pivot);
        rig.parts[0].pivot_override = Some([1.0, 2.0, 3.0]);
        assert_eq!(rig.pivot_of(&rig.parts[0]), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn swaying_plant_is_a_parented_chain() {
        let parts = SkeletonKind::SwayingPlant.parts();
        // Each segment parents the one below — a chain for the segmented bend.
        assert_eq!(parts[1].parent, Some(0));
        assert_eq!(parts[2].parent, Some(1));
        assert_eq!(parts[3].parent, Some(2));
        // Phase rises up the chain so the sway ripples toward the tip.
        assert!(parts[1].phase < parts[3].phase);
    }
}
