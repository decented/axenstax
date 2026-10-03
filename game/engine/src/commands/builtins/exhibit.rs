//! `/exhibit` (Creator Gallery, Spec 2026-06-19 §9 Phase 1c) — author 2D art
//! exhibits in-world: place / move / resize / re-orient / set-image / label /
//! delete. Edits the live `World.exhibits` list, which is snapshotted to the
//! world save (and `.axeworld`). Rendering is Phase 1b; this command stores
//! `image_ref` as an opaque non-empty string and never decodes pixels.
//!
//! Authoring is creative power, so editing subcommands are `OpLevel::Op` and
//! flag the World Integrity Ledger (`is_cheat`). `/exhibit list` is read-only.

use crate::commands::dispatch::{CommandContext, CommandResult};
use crate::commands::registry::{Command, OpLevel};
use crate::exhibit::{self, EditError, Exhibit, Presentation};

pub struct ExhibitCommand;

const USAGE: &str = "usage: /exhibit place <image> <wall|standing> [w] [h] | list | \
move <i> | resize <i> <w> <h> | yaw <i> <deg> | image <i> <ref> | label <i> <text> | delete <i>";

fn parse_index(s: Option<&String>) -> Option<usize> {
    s.and_then(|v| v.parse::<usize>().ok())
}

fn err(ctx: &mut CommandContext, msg: impl Into<String>) -> CommandResult {
    let m = msg.into();
    ctx.error(m.clone());
    CommandResult::Error(m)
}

/// Map an `EditError` to player-facing text.
fn explain(e: EditError) -> String {
    match e {
        EditError::NoTarget => "look at a block first (no target)".to_string(),
        EditError::BadIndex => "no exhibit with that index (try /exhibit list)".to_string(),
        EditError::EmptyImageRef => "image reference can't be empty".to_string(),
        EditError::NonPositiveSize => "width and height must be greater than zero".to_string(),
        EditError::WallNeedsWallFace => {
            "a wall exhibit must face a vertical wall, not a floor or ceiling".to_string()
        }
    }
}

impl Command for ExhibitCommand {
    fn name(&self) -> &'static str {
        "exhibit"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["ex"]
    }
    fn help(&self) -> &'static str {
        "Author gallery exhibits: place/move/resize/orient/image/label/delete"
    }
    fn usage(&self) -> &'static str {
        USAGE
    }
    // `list` is read-only; everything else is creative authoring. We gate the
    // whole command at Op so a survival visitor can't edit a gallery, and let
    // `list` early-return before any cheat marking.
    fn min_op_level(&self) -> OpLevel {
        OpLevel::Op
    }
    fn is_cheat(&self) -> bool {
        true
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> CommandResult {
        let sub = match args.first() {
            Some(s) => s.to_lowercase(),
            None => return err(ctx, USAGE),
        };

        match sub.as_str() {
            "list" => {
                if ctx.world.exhibits.is_empty() {
                    ctx.info("No exhibits placed yet. Use /exhibit place.".to_string());
                } else {
                    let lines: Vec<String> = ctx
                        .world
                        .exhibits
                        .iter()
                        .enumerate()
                        .map(|(i, e)| {
                            let kind = match e.presentation {
                                Presentation::Wall => "wall",
                                Presentation::Standing => "standing",
                            };
                            format!(
                                "#{i} [{kind}] '{}' {}×{} @({}, {}, {}) — \"{}\"",
                                e.image_ref, e.width, e.height, e.x, e.y, e.z, e.label
                            )
                        })
                        .collect();
                    for l in lines {
                        ctx.info(l);
                    }
                }
                // Read-only: no cheat flag.
                CommandResult::Silent
            }

            "place" => {
                let image_ref = match args.get(1) {
                    Some(s) if !s.trim().is_empty() => s.clone(),
                    _ => return err(ctx, "usage: /exhibit place <image> <wall|standing> [w] [h]"),
                };
                let presentation = match args.get(2).map(|s| s.to_lowercase()).as_deref() {
                    Some("wall") | Some("w") => Presentation::Wall,
                    Some("standing") | Some("stand") | Some("s") => Presentation::Standing,
                    _ => return err(ctx, "presentation must be 'wall' or 'standing'"),
                };
                // Optional size; default to a 2×2 m piece.
                let width = args.get(3).and_then(|s| s.parse::<f32>().ok()).unwrap_or(2.0);
                let height = args.get(4).and_then(|s| s.parse::<f32>().ok()).unwrap_or(2.0);
                if width <= 0.0 || height <= 0.0 {
                    return err(ctx, explain(EditError::NonPositiveSize));
                }

                // Read the dispatcher's last-frame raycast + facing.
                let slot = &ctx.players[ctx.player_idx];
                let target = slot.target_block;
                let face = slot.target_face;
                let yaw = slot.camera.yaw;

                let anchor = match exhibit::resolve_placement(target, face, yaw, presentation) {
                    Ok(a) => a,
                    Err(e) => return err(ctx, explain(e)),
                };

                ctx.world.exhibits.push(Exhibit {
                    x: anchor.x,
                    y: anchor.y,
                    z: anchor.z,
                    presentation,
                    image_ref,
                    width,
                    height,
                    yaw: anchor.yaw,
                    label: String::new(),
                    link: None,
                    sku: None,
                    price: None,
                });
                let idx = ctx.world.exhibits.len() - 1;
                ctx.success(format!(
                    "Placed exhibit #{idx} at ({}, {}, {}).",
                    anchor.x, anchor.y, anchor.z
                ));
                CommandResult::Success
            }

            "move" => {
                let idx = match parse_index(args.get(1)) {
                    Some(i) => i,
                    None => return err(ctx, "usage: /exhibit move <index>"),
                };
                let slot = &ctx.players[ctx.player_idx];
                let target = slot.target_block;
                let face = slot.target_face;
                let yaw = slot.camera.yaw;
                // Re-resolve using the moved piece's own presentation.
                let presentation = match ctx.world.exhibits.get(idx) {
                    Some(e) => e.presentation,
                    None => return err(ctx, explain(EditError::BadIndex)),
                };
                let anchor = match exhibit::resolve_placement(target, face, yaw, presentation) {
                    Ok(a) => a,
                    Err(e) => return err(ctx, explain(e)),
                };
                match exhibit::apply_move(&mut ctx.world.exhibits, idx, anchor.x, anchor.y, anchor.z) {
                    Ok(()) => {
                        let _ = exhibit::apply_yaw(&mut ctx.world.exhibits, idx, anchor.yaw);
                        ctx.success(format!(
                            "Moved exhibit #{idx} to ({}, {}, {}).",
                            anchor.x, anchor.y, anchor.z
                        ));
                        CommandResult::Success
                    }
                    Err(e) => err(ctx, explain(e)),
                }
            }

            "resize" => {
                let idx = parse_index(args.get(1));
                let w = args.get(2).and_then(|s| s.parse::<f32>().ok());
                let h = args.get(3).and_then(|s| s.parse::<f32>().ok());
                match (idx, w, h) {
                    (Some(i), Some(w), Some(h)) => {
                        match exhibit::apply_resize(&mut ctx.world.exhibits, i, w, h) {
                            Ok(()) => {
                                ctx.success(format!("Exhibit #{i} resized to {w}×{h}."));
                                CommandResult::Success
                            }
                            Err(e) => err(ctx, explain(e)),
                        }
                    }
                    _ => err(ctx, "usage: /exhibit resize <index> <width> <height>"),
                }
            }

            "yaw" => {
                let idx = parse_index(args.get(1));
                let deg = args.get(2).and_then(|s| s.parse::<f32>().ok());
                match (idx, deg) {
                    (Some(i), Some(d)) => {
                        let rad = d.to_radians();
                        match exhibit::apply_yaw(&mut ctx.world.exhibits, i, rad) {
                            Ok(()) => {
                                ctx.success(format!("Exhibit #{i} oriented to {d}°."));
                                CommandResult::Success
                            }
                            Err(e) => err(ctx, explain(e)),
                        }
                    }
                    _ => err(ctx, "usage: /exhibit yaw <index> <degrees>"),
                }
            }

            "image" => {
                let idx = parse_index(args.get(1));
                let image_ref = args.get(2).cloned();
                match (idx, image_ref) {
                    (Some(i), Some(r)) => {
                        match exhibit::apply_set_image(&mut ctx.world.exhibits, i, r.clone()) {
                            Ok(()) => {
                                ctx.success(format!("Exhibit #{i} image set to '{r}'."));
                                CommandResult::Success
                            }
                            Err(e) => err(ctx, explain(e)),
                        }
                    }
                    _ => err(ctx, "usage: /exhibit image <index> <image_ref>"),
                }
            }

            "label" => {
                let idx = match parse_index(args.get(1)) {
                    Some(i) => i,
                    None => return err(ctx, "usage: /exhibit label <index> <text>"),
                };
                // Everything after the index is the label (case + spaces preserved).
                let label = args[2..].join(" ");
                match exhibit::apply_set_label(&mut ctx.world.exhibits, idx, label.clone()) {
                    Ok(()) => {
                        ctx.success(format!("Exhibit #{idx} labelled \"{label}\"."));
                        CommandResult::Success
                    }
                    Err(e) => err(ctx, explain(e)),
                }
            }

            "delete" | "remove" | "del" => {
                let idx = match parse_index(args.get(1)) {
                    Some(i) => i,
                    None => return err(ctx, "usage: /exhibit delete <index>"),
                };
                match exhibit::apply_delete(&mut ctx.world.exhibits, idx) {
                    Ok(removed) => {
                        ctx.success(format!("Deleted exhibit #{idx} ('{}').", removed.image_ref));
                        CommandResult::Success
                    }
                    Err(e) => err(ctx, explain(e)),
                }
            }

            other => err(ctx, format!("unknown subcommand '{other}'. {USAGE}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::dispatch::ChatLine;
    use crate::commands::registry::{CommandRegistry, OpLevel};
    use crate::player_slot::PlayerSlot;
    use crate::world::World;

    /// Run `/exhibit <args>` against a world whose dispatcher (player 0) is
    /// looking at `target` with hit-face `face`, facing `yaw`. Returns the
    /// result and the chat lines emitted.
    fn run(
        args: &[&str],
        world: &mut World,
        target: Option<[i32; 3]>,
        face: [i32; 3],
        yaw: f32,
    ) -> (CommandResult, Vec<String>) {
        let cmd = ExhibitCommand;
        let mut creative = true;
        let mut wt = 0u32;
        let mut ts = 1u32;
        let mut pm = crate::play_mode::PlayMode::Creative;
        let mut players = vec![PlayerSlot::new(0, glam::Vec3::ZERO, 0.5)];
        players[0].target_block = target;
        players[0].target_face = face;
        players[0].camera.yaw = yaw;
        let mut log: Vec<ChatLine> = Vec::new();
        let mut ch = false;
        let mut ev = false;
        let mut ps = false;
        let reg = CommandRegistry::new();
        let r = {
            let mut ctx = CommandContext {
                world,
                world_time: &mut wt,
                world_time_step: &mut ts,
                is_creative: &mut creative,
                play_mode: &mut pm,
                seed: 1,
                world_name: "t",
                players: &mut players,
                player_idx: 0,
                op_level: OpLevel::Op,
                current_tick: 0,
                log: &mut log,
                registry: &reg,
                cheats_used_marker: &mut ch,
                ever_creative_marker: &mut ev,
                pure_survival_broken_marker: &mut ps,
            };
            let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            cmd.execute(&mut ctx, &owned)
        };
        let lines = log.iter().map(|l| l.text.clone()).collect();
        (r, lines)
    }

    #[test]
    fn place_wall_against_a_wall_face_adds_one_exhibit() {
        let mut w = World::new();
        let (r, lines) = run(
            &["place", "poster.png", "wall", "2", "1.5"],
            &mut w,
            Some([5, 64, 10]),
            [0, 0, 1], // +Z wall face
            0.0,
        );
        assert_eq!(r, CommandResult::Success);
        assert_eq!(w.exhibits.len(), 1);
        let e = &w.exhibits[0];
        assert_eq!((e.x, e.y, e.z), (5, 64, 11));
        assert_eq!(e.presentation, Presentation::Wall);
        assert_eq!(e.image_ref, "poster.png");
        assert!(lines.iter().any(|l| l.contains("Placed exhibit #0")));
    }

    #[test]
    fn place_wall_against_floor_is_rejected() {
        let mut w = World::new();
        let (r, _) = run(&["place", "x.png", "wall"], &mut w, Some([0, 0, 0]), [0, 1, 0], 0.0);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(w.exhibits.is_empty());
    }

    #[test]
    fn place_without_target_is_rejected() {
        let mut w = World::new();
        let (r, _) = run(&["place", "x.png", "standing"], &mut w, None, [0, 0, 0], 0.0);
        assert!(matches!(r, CommandResult::Error(_)));
        assert!(w.exhibits.is_empty());
    }

    #[test]
    fn place_standing_sits_on_top() {
        let mut w = World::new();
        let (r, _) = run(&["place", "statue.png", "standing"], &mut w, Some([2, 30, -4]), [0, 1, 0], 0.0);
        assert_eq!(r, CommandResult::Success);
        assert_eq!((w.exhibits[0].x, w.exhibits[0].y, w.exhibits[0].z), (2, 31, -4));
        assert_eq!(w.exhibits[0].presentation, Presentation::Standing);
    }

    #[test]
    fn resize_yaw_image_label_delete_round() {
        let mut w = World::new();
        run(&["place", "a.png", "wall"], &mut w, Some([0, 64, 0]), [0, 0, 1], 0.0);
        assert_eq!(run(&["resize", "0", "4", "3"], &mut w, None, [0; 3], 0.0).0, CommandResult::Success);
        assert!((w.exhibits[0].width - 4.0).abs() < 1e-6);
        assert_eq!(run(&["yaw", "0", "90"], &mut w, None, [0; 3], 0.0).0, CommandResult::Success);
        assert!((w.exhibits[0].yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert_eq!(run(&["image", "0", "b.png"], &mut w, None, [0; 3], 0.0).0, CommandResult::Success);
        assert_eq!(w.exhibits[0].image_ref, "b.png");
        assert_eq!(run(&["label", "0", "My", "Best", "Work"], &mut w, None, [0; 3], 0.0).0, CommandResult::Success);
        assert_eq!(w.exhibits[0].label, "My Best Work");
        assert_eq!(run(&["delete", "0"], &mut w, None, [0; 3], 0.0).0, CommandResult::Success);
        assert!(w.exhibits.is_empty());
    }

    #[test]
    fn edits_on_bad_index_error_without_panicking() {
        let mut w = World::new();
        assert!(matches!(run(&["resize", "9", "1", "1"], &mut w, None, [0; 3], 0.0).0, CommandResult::Error(_)));
        assert!(matches!(run(&["delete", "9"], &mut w, None, [0; 3], 0.0).0, CommandResult::Error(_)));
        assert!(matches!(run(&["yaw", "9", "0"], &mut w, None, [0; 3], 0.0).0, CommandResult::Error(_)));
    }

    #[test]
    fn list_is_silent_and_reports_count() {
        let mut w = World::new();
        let (r, lines) = run(&["list"], &mut w, None, [0; 3], 0.0);
        assert_eq!(r, CommandResult::Silent);
        assert!(lines.iter().any(|l| l.contains("No exhibits")));
    }

    #[test]
    fn unknown_subcommand_errors() {
        let mut w = World::new();
        assert!(matches!(run(&["frobnicate"], &mut w, None, [0; 3], 0.0).0, CommandResult::Error(_)));
    }
}
