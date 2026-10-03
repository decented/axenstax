//! Rig Studio (#19 dynamic/animated asset authoring — the in-game authoring UX).
//! Solo Buildout Session 3.
//!
//! The authoring surface the skeleton + anim-set foundation was waiting for:
//! pick a standard skeleton, assign a block to each of its named parts (the
//! "build your creature from blocks" do-ocracy MVP — the per-part micro-model
//! *shell* is a later upgrade), then spawn it as a standing, animated display
//! that inherits the skeleton's gait. Pure render: returns an action the game
//! loop performs (the actual assign reads the player's held block; the spawn
//! creates a `RiggedDisplay` entity).

use egui::RichText;

use crate::anim_set::AnimClip;
use crate::block::{BlockId, BlockRegistry, AIR};
use crate::skeleton::SkeletonKind;

/// The Rig Studio's authoring state (lives on `GameState`). `assigned[i]` is the
/// block bound to skeleton part `i` (`AIR` = unassigned), re-sized whenever the
/// skeleton changes.
pub struct RigStudioState {
    pub skeleton: SkeletonKind,
    pub assigned: Vec<BlockId>,
    pub name: String,
    /// Which inherited clip the spawned rig plays. Walk is the default (and what
    /// every rig placed before the picker existed plays).
    pub clip: AnimClip,
}

impl Default for RigStudioState {
    fn default() -> Self {
        let skeleton = SkeletonKind::Biped;
        Self {
            skeleton,
            assigned: vec![AIR; skeleton.parts().len()],
            name: "My Creature".to_string(),
            clip: AnimClip::Walk,
        }
    }
}

impl RigStudioState {
    /// Switch skeleton, clearing the per-part assignments (part counts differ).
    pub fn set_skeleton(&mut self, kind: SkeletonKind) {
        self.skeleton = kind;
        self.assigned = vec![AIR; kind.parts().len()];
    }

    /// Assign `block` to part `idx` (no-op if out of range).
    pub fn assign(&mut self, idx: usize, block: BlockId) {
        if let Some(slot) = self.assigned.get_mut(idx) {
            *slot = block;
        }
    }

    /// Build a `RiggedModel` from the current assignments (parts left as `AIR`
    /// are simply omitted). `None` if nothing is assigned yet.
    pub fn to_rig(&self) -> Option<crate::skeleton::RiggedModel> {
        let mut rig = crate::skeleton::RiggedModel::new(self.skeleton, self.name.clone());
        let parts = self.skeleton.parts();
        for (i, &block) in self.assigned.iter().enumerate() {
            if block != AIR
                && let Some(sp) = parts.get(i) {
                    rig.attach(sp.name, block);
                }
        }
        if rig.parts.is_empty() {
            None
        } else {
            Some(rig)
        }
    }
}

/// What the panel is asking the game loop to do this frame.
#[derive(Default)]
pub struct RigStudioResult {
    /// Switch to this skeleton (clears assignments).
    pub select: Option<SkeletonKind>,
    /// Play this clip on the rig (and on every rig spawned from here after).
    pub set_clip: Option<AnimClip>,
    /// Assign the player's held block to this part index.
    pub set_part: Option<usize>,
    /// Spawn the authored rig in the world.
    pub spawn: bool,
    pub close_requested: bool,
}

/// Render the Rig Studio. `held_name` is the player's currently-selected hotbar
/// block's display name (for the "Set from held" hint), or `None` if empty-handed.
pub fn show_rig_studio(
    ctx: &egui::Context,
    state: &RigStudioState,
    registry: &BlockRegistry,
    held_name: Option<&str>,
) -> RigStudioResult {
    let mut result = RigStudioResult::default();
    let mut window_open = true;

    egui::Window::new("Rig Studio")
        .id(egui::Id::new("rig_studio"))
        .open(&mut window_open)
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(RichText::new("Build an animated creature").size(16.0).strong());
            ui.label(
                RichText::new("Pick a skeleton, assign a block to each part, then Spawn. Press Y to close.")
                    .weak()
                    .size(11.0),
            );
            ui.add_space(8.0);

            // Skeleton picker.
            ui.horizontal_wrapped(|ui| {
                ui.label("Skeleton:");
                for kind in SkeletonKind::ALL {
                    let selected = kind == state.skeleton;
                    if ui.selectable_label(selected, kind.name()).clicked() && !selected {
                        result.select = Some(kind);
                    }
                }
            });
            ui.add_space(4.0);

            // Motion picker — which of the skeleton's inherited clips it plays.
            ui.horizontal_wrapped(|ui| {
                ui.label("Motion:");
                for clip in AnimClip::PICKER {
                    let selected = clip == state.clip;
                    if ui.selectable_label(selected, clip.name()).clicked() && !selected {
                        result.set_clip = Some(clip);
                    }
                }
            });
            ui.add_space(8.0);

            // Per-part assignment rows.
            let parts = state.skeleton.parts();
            for (i, sp) in parts.iter().enumerate() {
                let assigned = state.assigned.get(i).copied().unwrap_or(AIR);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(sp.name).strong());
                    let shown = if assigned == AIR {
                        "—".to_string()
                    } else {
                        registry.get(assigned).name.to_string()
                    };
                    ui.label(RichText::new(shown).color(GOLD));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = match held_name {
                            Some(n) => format!("Set: {n}"),
                            None => "Set (hold a block)".to_string(),
                        };
                        if ui.add_enabled(held_name.is_some(), egui::Button::new(label)).clicked() {
                            result.set_part = Some(i);
                        }
                    });
                });
            }

            ui.add_space(10.0);
            let any_assigned = state.assigned.iter().any(|&b| b != AIR);
            ui.horizontal(|ui| {
                if ui.add_enabled(any_assigned, egui::Button::new("🐾 Spawn rig")).clicked() {
                    result.spawn = true;
                }
                if ui.button("Close").clicked() {
                    result.close_requested = true;
                }
            });
            if !any_assigned {
                ui.label(RichText::new("Assign at least one part to spawn.").weak().size(11.0));
            }
        });

    if !window_open {
        result.close_requested = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result.close_requested = true;
    }
    result
}

const GOLD: egui::Color32 = egui::Color32::from_rgb(255, 210, 120);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_skeleton_resizes_assignments() {
        let mut s = RigStudioState::default();
        assert_eq!(s.assigned.len(), SkeletonKind::Biped.parts().len());
        s.assign(0, crate::block::STONE);
        s.set_skeleton(SkeletonKind::Quadruped);
        assert_eq!(s.assigned.len(), SkeletonKind::Quadruped.parts().len());
        assert!(s.assigned.iter().all(|&b| b == AIR), "switching clears assignments");
    }

    #[test]
    fn clip_defaults_to_walk_and_is_pickable() {
        let mut s = RigStudioState::default();
        assert_eq!(s.clip, AnimClip::Walk, "a fresh studio walks");
        // The picker offers exactly the three display-friendly clips.
        assert_eq!(AnimClip::PICKER, [AnimClip::Walk, AnimClip::Idle, AnimClip::Bounce]);
        s.clip = AnimClip::Bounce;
        assert_eq!(s.clip, AnimClip::Bounce);
        // Switching skeleton keeps the chosen motion (only geometry is cleared).
        s.set_skeleton(SkeletonKind::Bird);
        assert_eq!(s.clip, AnimClip::Bounce);
    }

    #[test]
    fn to_rig_attaches_only_assigned_parts() {
        let mut s = RigStudioState::default(); // biped
        assert!(s.to_rig().is_none(), "nothing assigned yet");
        // Assign head + body.
        s.assign(0, crate::block::STONE);
        s.assign(1, crate::block::OAK_PLANKS);
        let rig = s.to_rig().expect("two parts assigned");
        assert_eq!(rig.parts.len(), 2);
        assert_eq!(rig.skeleton, SkeletonKind::Biped);
    }
}
