//! Spec 24 — egui dialogs for the Build Schematics feature.
//!
//! Phase 5 (this file's starting scope): the **Capture dialog** — name
//! input, licence picker, Save-As-derivative checkbox (when a parent
//! match is detected), material summary, Confirm / Cancel. Plus the
//! first-time licence-onboarding modal that explains CC-BY-SA.
//!
//! Phases 7 + 10 + 12 (Inspect / Resume / Plaque dialogs) will land in
//! this same module as they're built — they share most of the layout
//! patterns and the pure helpers (`material_summary`, etc.).
//!
//! Pure helpers (`material_summary`, `display_block_name`) live here
//! rather than in `plan.rs` because they're rendered-list-shaped and
//! used by multiple dialogs. They have no egui dependency and are
//! exhaustively unit-tested.

use crate::block::{BlockId, BlockRegistry};
use crate::plan::{PendingCapture, PlanLicense};

// ─── Pure helpers ─────────────────────────────────────────────────────

/// Aggregate a plan's captured cells into a `(block_id, count)` list
/// suitable for the Inspect dialog's material summary OR the Capture
/// dialog's "what you'll be locking in" line. Stable order (sorted by
/// block_id descending count) so the same plan always renders the same
/// list.
pub fn material_summary(data: &crate::plan::PlanData) -> Vec<(BlockId, u32)> {
    let mut counts: ahash::AHashMap<BlockId, u32> = ahash::AHashMap::new();
    for c in &data.cells {
        *counts.entry(c.block_id).or_insert(0) += 1;
    }
    let mut vec: Vec<(BlockId, u32)> = counts.into_iter().collect();
    // Sort by count descending, then by block_id ascending for ties.
    vec.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    vec
}

/// Turn a registry block name like `"genesis:oak_planks"` into a
/// human-readable label `"Oak Planks"`. Delegates to `Item::name` so
/// HP-0 historical-naming overrides (e.g. BLUEPRINT_PAPER → "Blueprint Paper")
/// apply to every UI surface consistently.
pub fn display_block_name(registry: &BlockRegistry, id: BlockId) -> String {
    crate::item::Item::Block(id).name(registry)
}

/// Human label for a licence — used in pickers, plaques, plan
/// inspect dialogs. Display-only.
pub fn license_label(lic: PlanLicense) -> &'static str {
    match lic {
        PlanLicense::AllRightsReserved => "All Rights Reserved",
        PlanLicense::CC0 => "CC-0 (Public Domain)",
        PlanLicense::Ccbysa => "CC-BY-SA (Default)",
        PlanLicense::Ccbynd => "CC-BY-ND",
    }
}

/// One-line description of what a licence means, shown under the
/// radio buttons in the capture dialog.
pub fn license_explainer(lic: PlanLicense) -> &'static str {
    match lic {
        PlanLicense::AllRightsReserved => "Your design. Others can build a Licence copy but not derive.",
        PlanLicense::CC0 => "Public domain. Anyone can use, modify, or sell.",
        PlanLicense::Ccbysa => "Anyone can use or modify — must credit you and share-alike.",
        PlanLicense::Ccbynd => "Anyone can build a copy — must credit you, no derivatives.",
    }
}

/// Render an ASCII top-down preview of a plan's footprint. Each XZ
/// column is `#` if any cell in that column is non-air, `.` otherwise.
/// `width` rows × `depth` columns, separated by newlines. Useful for
/// the Inspect dialog's at-a-glance shape preview.
pub fn ascii_footprint(data: &crate::plan::PlanData) -> String {
    let w = data.width as usize;
    let d = data.depth as usize;
    if w == 0 || d == 0 {
        return String::new();
    }
    let mut grid = vec![vec![false; d]; w];
    for c in &data.cells {
        let rx = c.rx as usize;
        let rz = c.rz as usize;
        if rx < w && rz < d {
            grid[rx][rz] = true;
        }
    }
    grid.iter()
        .map(|row| {
            row.iter()
                .map(|&filled| if filled { '#' } else { '.' })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Spec 24 Phase 7 — one row in the Inspect dialog's material list.
/// In survival, `sufficient` controls whether the "Place in world"
/// button can be clicked (all rows must be sufficient). In creative
/// the field is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialDeltaRow {
    pub block_id: BlockId,
    pub needed: u32,
    pub owned: u32,
    pub sufficient: bool,
}

/// Compute the Inspect dialog's material list with green/red inventory
/// delta info. Aggregates plan cells by block_id and counts how many
/// of each block the player currently owns across their inventory.
pub fn material_delta_summary(
    data: &crate::plan::PlanData,
    inventory: &crate::inventory::Inventory,
) -> Vec<MaterialDeltaRow> {
    let summary = material_summary(data);
    summary
        .into_iter()
        .map(|(block_id, needed)| {
            let owned: u32 = (0..36)
                .filter_map(|i| inventory.slot(i))
                .filter_map(|s| match s.item {
                    crate::item::Item::Block(b) if b == block_id => Some(s.count as u32),
                    _ => None,
                })
                .sum();
            MaterialDeltaRow {
                block_id,
                needed,
                owned,
                sufficient: owned >= needed,
            }
        })
        .collect()
}

/// Whether the player has enough materials to place the plan. In
/// creative this is always `true`; in survival it requires every row
/// in the summary to be `sufficient`.
pub fn has_sufficient_materials(rows: &[MaterialDeltaRow], is_creative: bool) -> bool {
    if is_creative {
        return true;
    }
    rows.iter().all(|r| r.sufficient)
}

// ─── Capture dialog (Phase 5) ─────────────────────────────────────────

/// Returned by `show_capture_dialog` to indicate what the player did
/// this frame. The caller commits / clears `pending_capture` based on
/// the outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureDialogOutcome {
    /// Modal still open — dialog handles state, caller does nothing.
    InProgress,
    /// Player clicked Confirm — caller commits the plan into the
    /// player's inventory + clears tile blocks.
    Confirmed,
    /// Player clicked Cancel — caller drops the candidate; tiles and
    /// built structure remain untouched in the world.
    Cancelled,
}

/// Returned by `show_license_onboarding_modal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnboardingOutcome {
    InProgress,
    Dismissed,
}

/// Render the first-time CC-BY-SA onboarding modal. Only called when
/// `WorldMeta.has_seen_license_onboarding == false`. Returns
/// `Dismissed` once the player clicks OK; caller then flips the flag.
pub fn show_license_onboarding_modal(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
) -> OnboardingOutcome {
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("plan_onboarding_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 200),
    );

    let panel_w = 520.0;
    let panel_h = 320.0;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0,
    );
    let mut outcome = OnboardingOutcome::InProgress;
    egui::Area::new(egui::Id::new(("plan_onboarding", player_index)))
        .fixed_pos(origin)
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            let bg = egui::Rect::from_min_size(origin, egui::vec2(panel_w, panel_h));
            ui.painter().rect_filled(
                bg,
                8.0,
                egui::Color32::from_rgb(36, 32, 26),
            );
            ui.painter().rect_stroke(
                bg,
                8.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(180, 150, 80)),
                egui::StrokeKind::Inside,
            );
            ui.vertical_centered(|ui| {
                ui.add_space(18.0);
                ui.label(
                    egui::RichText::new("\u{1F4DC} Your first Plan")
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(230, 200, 120)),
                );
                ui.add_space(12.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Plans are saved buildings you can rebuild anywhere — \
                             and share with others. Pick a licence so other players \
                             know what they can do with your design.",
                        )
                        .size(15.0)
                        .color(egui::Color32::from_rgb(220, 220, 220)),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Default: CC-BY-SA. Anyone can build or modify your \
                             plan — they just have to credit you and share their \
                             changes under the same licence. You can change this \
                             on every plan.",
                        )
                        .size(14.0)
                        .italics()
                        .color(egui::Color32::from_rgb(180, 180, 180)),
                    )
                    .wrap(),
                );
                ui.add_space(22.0);
                if ui
                    .add_sized(
                        [120.0, 32.0],
                        egui::Button::new(
                            egui::RichText::new("Got it").size(15.0).strong(),
                        ),
                    )
                    .clicked()
                {
                    outcome = OnboardingOutcome::Dismissed;
                }
            });
        });
    outcome
}

/// Render the Capture dialog. Mutates `pending` in-place
/// (name / licence / mark_as_derivative). Returns the player's
/// action this frame.
pub fn show_capture_dialog(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    pending: &mut PendingCapture,
    parent_name: Option<&str>,
    registry: &BlockRegistry,
) -> CaptureDialogOutcome {
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("plan_capture_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    let panel_w = 540.0;
    let panel_h = 460.0;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0,
    );

    let mut outcome = CaptureDialogOutcome::InProgress;
    let summary = material_summary(&pending.candidate.data);
    let footprint = (
        pending.candidate.data.width,
        pending.candidate.data.depth,
        pending.candidate.data.height,
    );

    egui::Area::new(egui::Id::new(("plan_capture", player_index)))
        .fixed_pos(origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            let bg = egui::Rect::from_min_size(origin, egui::vec2(panel_w, panel_h));
            ui.painter().rect_filled(
                bg,
                6.0,
                egui::Color32::from_rgb(28, 30, 40),
            );
            ui.painter().rect_stroke(
                bg,
                6.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(180, 150, 80)),
                egui::StrokeKind::Inside,
            );
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("\u{1F4DC} Plan captured")
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(230, 200, 120)),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(format!(
                        "Footprint: {}\u{00D7}{}\u{00D7}{} \u{2022} {} blocks",
                        footprint.0,
                        footprint.1,
                        footprint.2,
                        pending.candidate.data.cells.len()
                    ))
                    .size(13.0)
                    .italics()
                    .color(egui::Color32::from_rgb(170, 170, 200)),
                );
                ui.add_space(14.0);
            });

            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(egui::RichText::new("Name:").size(15.0).strong());
                ui.add_sized(
                    [380.0, 26.0],
                    egui::TextEdit::singleline(&mut pending.candidate.data.name)
                        .char_limit(64),
                );
            });
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(egui::RichText::new("Licence:").size(15.0).strong());
            });
            for lic in [
                PlanLicense::Ccbysa,
                PlanLicense::CC0,
                PlanLicense::Ccbynd,
                PlanLicense::AllRightsReserved,
            ] {
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    let selected = pending.candidate.data.license == lic;
                    if ui.radio(selected, license_label(lic)).clicked() {
                        pending.candidate.data.license = lic;
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(62.0);
                    ui.label(
                        egui::RichText::new(license_explainer(lic))
                            .size(12.0)
                            .italics()
                            .color(egui::Color32::from_rgb(160, 160, 180)),
                    );
                });
            }
            ui.add_space(10.0);

            // Save-As checkbox — only renders when a parent was detected.
            if pending.parent_match.is_some() {
                let parent_label = parent_name.unwrap_or("an existing plan");
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    if ui
                        .checkbox(
                            &mut pending.mark_as_derivative,
                            format!("Mark as derivative of «{parent_label}»"),
                        )
                        .changed()
                    {
                        // No-op — `pending.mark_as_derivative` mutates
                        // in-place via the checkbox.
                    }
                });
                ui.add_space(8.0);
            }

            // Material summary — informational only at capture time
            // (the player already gathered these blocks since this is
            // a captured building they built). Keeps the player
            // oriented about what kind of plan they're saving.
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Plan uses:")
                        .size(13.0)
                        .color(egui::Color32::from_rgb(200, 200, 200)),
                );
            });
            let preview: Vec<String> = summary
                .iter()
                .take(4)
                .map(|(id, n)| format!("{n}\u{00D7} {}", display_block_name(registry, *id)))
                .collect();
            let suffix = if summary.len() > 4 {
                format!(" + {} more", summary.len() - 4)
            } else {
                String::new()
            };
            ui.horizontal(|ui| {
                ui.add_space(34.0);
                ui.label(
                    egui::RichText::new(format!("{}{suffix}", preview.join(", ")))
                        .size(12.0)
                        .italics()
                        .color(egui::Color32::from_rgb(180, 180, 180)),
                );
            });
            ui.add_space(18.0);

            ui.horizontal(|ui| {
                let button_w = 130.0;
                let gap = 16.0;
                let centre_offset = (panel_w - (button_w * 2.0 + gap)) / 2.0;
                ui.add_space(centre_offset);
                if ui
                    .add_sized(
                        [button_w, 34.0],
                        egui::Button::new(
                            egui::RichText::new("Cancel").size(15.0),
                        ),
                    )
                    .clicked()
                {
                    outcome = CaptureDialogOutcome::Cancelled;
                }
                ui.add_space(gap);
                if ui
                    .add_sized(
                        [button_w, 34.0],
                        egui::Button::new(
                            egui::RichText::new("Confirm").size(15.0).strong(),
                        )
                        .fill(egui::Color32::from_rgb(80, 130, 80)),
                    )
                    .clicked()
                {
                    outcome = CaptureDialogOutcome::Confirmed;
                }
            });
        });

    outcome
}

// ─── Inspect dialog (Phase 7) ─────────────────────────────────────────

/// Returned by `show_inspect_dialog` to indicate what the player did
/// this frame. The caller transitions to ghost-preview mode (Phase 8,
/// deferred) OR clears the inspect target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectDialogOutcome {
    InProgress,
    /// Place button pressed — caller transitions to Phase 8 Ghost mode
    /// (when Ghost ships) or to a direct Phase 10+11 build at the
    /// player's facing position (pre-Phase-8 fallback).
    PlaceClicked,
    /// Close button pressed or dialog dismissed.
    Closed,
}

/// Render the Inspect dialog for a Plan in the player's inventory.
/// Read-only — no state mutation; the caller decides what to do with
/// PlaceClicked.
///
/// `is_creative` controls whether the material list renders the
/// inventory delta colours (survival) or informational-only (creative).
pub fn show_inspect_dialog(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    plan: &crate::plan::PlanData,
    inventory: &crate::inventory::Inventory,
    is_creative: bool,
    registry: &BlockRegistry,
) -> InspectDialogOutcome {
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("plan_inspect_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    let panel_w = 560.0;
    let panel_h = 520.0;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0,
    );

    let rows = material_delta_summary(plan, inventory);
    let can_place = has_sufficient_materials(&rows, is_creative);
    let footprint_ascii = ascii_footprint(plan);
    let mut outcome = InspectDialogOutcome::InProgress;

    egui::Area::new(egui::Id::new(("plan_inspect", player_index)))
        .fixed_pos(origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            let bg = egui::Rect::from_min_size(origin, egui::vec2(panel_w, panel_h));
            ui.painter().rect_filled(
                bg,
                6.0,
                egui::Color32::from_rgb(28, 30, 40),
            );
            ui.painter().rect_stroke(
                bg,
                6.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(180, 150, 80)),
                egui::StrokeKind::Inside,
            );
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(format!("\u{1F4DC} {}", plan.name))
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(230, 200, 120)),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} \u{2022} Footprint {}\u{00D7}{}\u{00D7}{} \u{2022} {} blocks",
                        license_label(plan.license),
                        plan.width,
                        plan.depth,
                        plan.height,
                        plan.cells.len(),
                    ))
                    .size(13.0)
                    .italics()
                    .color(egui::Color32::from_rgb(170, 170, 200)),
                );
                ui.add_space(2.0);
                // Mode badge — explicit "Authored in Creative" /
                // "Authored in Survival". Spec 24 §"Creative vs
                // Survival" amendment.
                let mode_label = match plan.authored_in.as_str() {
                    "creative" => "Authored in Creative",
                    _ => "Authored in Survival",
                };
                ui.label(
                    egui::RichText::new(mode_label)
                        .size(11.0)
                        .italics()
                        .color(egui::Color32::from_rgb(140, 140, 180)),
                );
                ui.add_space(12.0);
            });

            // ASCII top-down preview — monospace label.
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Footprint:")
                        .size(13.0)
                        .strong()
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new(&footprint_ascii)
                        .monospace()
                        .size(14.0)
                        .color(egui::Color32::from_rgb(200, 200, 200)),
                );
            });
            ui.add_space(12.0);

            // Material list. Creative = informational; survival =
            // green/red inventory delta.
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new(if is_creative {
                        "Plan contents (creative — no inventory required):"
                    } else {
                        "Materials needed:"
                    })
                    .size(13.0)
                    .strong()
                    .color(egui::Color32::from_rgb(200, 200, 220)),
                );
            });
            for row in &rows {
                let name = display_block_name(registry, row.block_id);
                let (text, colour) = if is_creative {
                    (
                        format!("{}\u{00D7} {}", row.needed, name),
                        egui::Color32::from_rgb(180, 180, 180),
                    )
                } else {
                    let symbol = if row.sufficient { "\u{2713}" } else { "\u{2717}" };
                    let label = format!(
                        "{symbol}  {}\u{00D7} {}  (have {})",
                        row.needed, name, row.owned
                    );
                    let colour = if row.sufficient {
                        egui::Color32::from_rgb(140, 200, 140)
                    } else {
                        egui::Color32::from_rgb(220, 130, 130)
                    };
                    (label, colour)
                };
                ui.horizontal(|ui| {
                    ui.add_space(40.0);
                    ui.label(egui::RichText::new(text).size(13.0).color(colour));
                });
            }
            ui.add_space(20.0);

            ui.horizontal(|ui| {
                let button_w = 130.0;
                let gap = 16.0;
                let centre_offset = (panel_w - (button_w * 2.0 + gap)) / 2.0;
                ui.add_space(centre_offset);
                if ui
                    .add_sized(
                        [button_w, 34.0],
                        egui::Button::new(egui::RichText::new("Close").size(15.0)),
                    )
                    .clicked()
                {
                    outcome = InspectDialogOutcome::Closed;
                }
                ui.add_space(gap);
                let place_button = egui::Button::new(
                    egui::RichText::new("Place in world")
                        .size(15.0)
                        .strong()
                        .color(if can_place {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_rgb(140, 140, 140)
                        }),
                )
                .fill(if can_place {
                    egui::Color32::from_rgb(80, 130, 80)
                } else {
                    egui::Color32::from_rgb(60, 60, 70)
                });
                let resp = ui.add_enabled_ui(can_place, |ui| {
                    ui.add_sized([button_w, 34.0], place_button)
                });
                if resp.inner.clicked() {
                    outcome = InspectDialogOutcome::PlaceClicked;
                }
            });
        });

    outcome
}

// ─── Plaque dialog (Phase 12) ────────────────────────────────────────

/// Returned by `show_plaque_dialog`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaqueDialogOutcome {
    InProgress,
    Closed,
    /// Spec 25 Phase 7 — player clicked the Tip button on a specific
    /// derivation-chain link. The usize is the link's index into
    /// `plaque.chain`; the caller resolves the architect's
    /// `author_npub` and routes a sats trickle through
    /// `economy::apply_sats_payout` with `PayoutKind::PlaqueTip`.
    TipLink(usize),
}

/// Render the Architect's Plaque attribution dialog. Shows the
/// derivation chain + the root plan's authored-in badge. Spec 25
/// Phase 7 adds the per-architect tip button.
///
/// `tip_eligible` is `true` when the player has Charter sats enabled
/// AND the server allows Bitcoin (and no Nostrich's Vow) — dimmed
/// otherwise. `tips_visible` ([`crate::economy::sats_ui_visible`]) hides
/// the tip buttons entirely when sats are off (review W4 S4).
pub fn show_plaque_dialog(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    plaque: &crate::plan::ArchitectPlaqueData,
    tip_eligible: bool,
    tips_visible: bool,
    procgen_source: bool,
) -> PlaqueDialogOutcome {
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("plaque_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    let panel_w = 520.0;
    let panel_h = (160.0 + (plaque.chain.len() as f32 * 44.0)).min(560.0);
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0,
    );

    let mut outcome = PlaqueDialogOutcome::InProgress;

    egui::Area::new(egui::Id::new(("plaque_dialog", player_index)))
        .fixed_pos(origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            let bg = egui::Rect::from_min_size(origin, egui::vec2(panel_w, panel_h));
            ui.painter().rect_filled(
                bg,
                6.0,
                egui::Color32::from_rgb(28, 30, 40),
            );
            ui.painter().rect_stroke(
                bg,
                6.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(180, 150, 80)),
                egui::StrokeKind::Inside,
            );
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("\u{1F3DB} Architect's Plaque")
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(230, 200, 120)),
                );
                ui.add_space(2.0);
                let mode_label = match plaque.authored_in.as_str() {
                    "creative" => "Original plan authored in Creative",
                    _ => "Original plan authored in Survival",
                };
                ui.label(
                    egui::RichText::new(mode_label)
                        .size(12.0)
                        .italics()
                        .color(egui::Color32::from_rgb(170, 170, 200)),
                );
                if procgen_source {
                    // Spec 27 Phase 8 — surface the procgen origin so
                    // a villager-village build reads as procedurally
                    // sampled rather than player-built.
                    ui.label(
                        egui::RichText::new("Sampled by village procgen")
                            .size(12.0)
                            .italics()
                            .color(egui::Color32::from_rgb(170, 200, 170)),
                    );
                }
                if let Some(credit) = plaque.builder_credit.as_ref() {
                    // Spec 26 — Builder NPC credit. Reads as
                    // "Built by {villager} of {village} for {commissioner}".
                    let village_clause = credit
                        .village_name
                        .as_deref()
                        .map(|n| format!(" of {n}"))
                        .unwrap_or_default();
                    let commissioner = if credit.commissioner_npub.is_empty() {
                        "an unsigned commissioner".to_string()
                    } else {
                        format!(
                            "{}…",
                            &credit.commissioner_npub
                                [..credit.commissioner_npub.len().min(12)]
                        )
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "Built by {}{village_clause} for {commissioner}",
                            credit.villager_name
                        ))
                        .size(12.0)
                        .italics()
                        .color(egui::Color32::from_rgb(170, 200, 170)),
                    );
                }
                ui.add_space(12.0);
            });
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Derivation chain:")
                        .size(13.0)
                        .strong()
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                );
            });
            for (i, link) in plaque.chain.iter().enumerate() {
                let arrow = if i == plaque.chain.len() - 1 {
                    "\u{25CF}" // bullet for current
                } else {
                    "\u{25CB}" // hollow circle for ancestor
                };
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    ui.label(
                        egui::RichText::new(format!("{arrow}  {}", link.plan_name))
                            .size(14.0)
                            .strong()
                            .color(egui::Color32::from_rgb(220, 220, 220)),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(56.0);
                    let author = if link.author_npub.is_empty() {
                        "(unsigned)".to_string()
                    } else {
                        format!("{}…", crate::plan::short_npub(&link.author_npub, 12))
                    };
                    ui.label(
                        egui::RichText::new(format!(
                            "{author} \u{2022} {}",
                            license_label(link.license)
                        ))
                        .size(12.0)
                        .italics()
                        .color(egui::Color32::from_rgb(160, 160, 180)),
                    );
                    // Spec 25 Phase 7 — per-architect tip button.
                    // Disabled when (a) the link is unsigned (no
                    // recipient address) or (b) the player can't
                    // send sats (Charter / server-policy off).
                    if !tips_visible {
                        return;
                    }
                    let can_tip = tip_eligible && !link.author_npub.is_empty();
                    let btn = egui::Button::new(egui::RichText::new("Tip 1 sat").size(11.0));
                    let resp = ui.add_enabled(can_tip, btn);
                    if resp.clicked() {
                        outcome = PlaqueDialogOutcome::TipLink(i);
                    }
                    if !can_tip {
                        let reason = if link.author_npub.is_empty() {
                            "Architect is unsigned — no tip target."
                        } else {
                            "Bitcoin disabled for this account / server."
                        };
                        resp.on_hover_text(reason);
                    }
                });
            }
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                if ui
                    .add_sized(
                        [120.0, 32.0],
                        egui::Button::new(egui::RichText::new("Close").size(15.0)),
                    )
                    .clicked()
                {
                    outcome = PlaqueDialogOutcome::Closed;
                }
            });
        });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::plan::{CapturedCell, PlanData};

    fn plan_with_cells(cells: Vec<CapturedCell>) -> PlanData {
        let mut p = PlanData::debug_3x3_stone();
        p.cells = cells;
        p
    }

    #[test]
    fn material_summary_aggregates_single_block() {
        let plan = PlanData::debug_3x3_stone();
        let s = material_summary(&plan);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0], (block::STONE, 9));
    }

    #[test]
    fn plaque_dialog_outcome_tip_link_variant_carries_index() {
        // Spec 25 Phase 7 — TipLink(usize) variant carries the chain
        // index. Compile-time + equality guards.
        let a = PlaqueDialogOutcome::TipLink(2);
        let b = PlaqueDialogOutcome::TipLink(2);
        assert_eq!(a, b);
        let c = PlaqueDialogOutcome::TipLink(5);
        assert_ne!(a, c);
        // Closed + InProgress are distinct from TipLink.
        assert_ne!(a, PlaqueDialogOutcome::Closed);
        assert_ne!(a, PlaqueDialogOutcome::InProgress);
    }

    #[test]
    fn material_summary_aggregates_multiple_blocks_sorted_by_count() {
        let cells = vec![
            CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::DIRT },
            CapturedCell { rx: 1, ry: 0, rz: 0, block_id: block::STONE },
            CapturedCell { rx: 0, ry: 0, rz: 1, block_id: block::STONE },
            CapturedCell { rx: 1, ry: 0, rz: 1, block_id: block::STONE },
        ];
        let plan = plan_with_cells(cells);
        let s = material_summary(&plan);
        assert_eq!(s, vec![(block::STONE, 3), (block::DIRT, 1)]);
    }

    #[test]
    fn material_summary_handles_empty_plan() {
        let plan = plan_with_cells(Vec::new());
        let s = material_summary(&plan);
        assert!(s.is_empty());
    }

    #[test]
    fn display_block_name_strips_namespace_and_title_cases() {
        let reg = BlockRegistry::new();
        assert_eq!(display_block_name(&reg, block::STONE), "Stone");
        assert_eq!(display_block_name(&reg, block::OAK_PLANKS), "Oak Planks");
    }

    #[test]
    fn display_block_name_handles_underscore_words() {
        let reg = BlockRegistry::new();
        // BLUEPRINT_PAPER → "Blueprint Paper" (HP-0 historical-naming override).
        assert_eq!(display_block_name(&reg, block::BLUEPRINT_PAPER), "Blueprint Paper");
    }

    #[test]
    fn license_label_covers_all_variants() {
        // Compiler enforces exhaustiveness via the inner match. The
        // test just guards against future enum extensions silently
        // mis-rendering one variant.
        assert!(license_label(PlanLicense::AllRightsReserved).len() > 0);
        assert!(license_label(PlanLicense::CC0).contains("Public"));
        assert!(license_label(PlanLicense::Ccbysa).contains("Default"));
        assert!(license_label(PlanLicense::Ccbynd).len() > 0);
    }

    #[test]
    fn license_explainer_returns_non_empty_for_every_variant() {
        for lic in [
            PlanLicense::AllRightsReserved,
            PlanLicense::CC0,
            PlanLicense::Ccbysa,
            PlanLicense::Ccbynd,
        ] {
            assert!(!license_explainer(lic).is_empty());
        }
    }

    // ─── Spec 24 Phase 7 — Inspect dialog pure helpers ──────────────

    #[test]
    fn ascii_footprint_renders_filled_3x3() {
        // debug_3x3_stone has stone at every (rx, 0, rz) for rx,rz ∈ 0..3.
        let plan = PlanData::debug_3x3_stone();
        let ascii = ascii_footprint(&plan);
        assert_eq!(ascii, "###\n###\n###");
    }

    #[test]
    fn ascii_footprint_renders_L_shape() {
        // Build an L-shape:
        //   #.
        //   #.
        //   ##
        let cells = vec![
            CapturedCell { rx: 0, ry: 0, rz: 0, block_id: block::STONE },
            CapturedCell { rx: 1, ry: 0, rz: 0, block_id: block::STONE },
            CapturedCell { rx: 2, ry: 0, rz: 0, block_id: block::STONE },
            CapturedCell { rx: 2, ry: 0, rz: 1, block_id: block::STONE },
        ];
        let mut plan = plan_with_cells(cells);
        plan.width = 3;
        plan.depth = 2;
        let ascii = ascii_footprint(&plan);
        assert_eq!(ascii, "#.\n#.\n##");
    }

    #[test]
    fn ascii_footprint_handles_zero_dimensions() {
        let mut plan = plan_with_cells(Vec::new());
        plan.width = 0;
        plan.depth = 0;
        assert_eq!(ascii_footprint(&plan), "");
    }

    fn inventory_with_blocks(pairs: &[(BlockId, u32)]) -> crate::inventory::Inventory {
        let mut inv = crate::inventory::Inventory::new();
        for &(bid, n) in pairs {
            // ItemStack count is u8; split into multiple stacks if >64.
            let mut remaining = n;
            while remaining > 0 {
                let take = remaining.min(64) as u8;
                let _ = inv.add_item(crate::item::ItemStack::new_block(bid, take));
                remaining -= take as u32;
            }
        }
        inv
    }

    #[test]
    fn material_delta_summary_reports_owned_and_needed() {
        let plan = PlanData::debug_3x3_stone(); // 9 stone needed
        let inv = inventory_with_blocks(&[(block::STONE, 5)]);
        let rows = material_delta_summary(&plan, &inv);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].block_id, block::STONE);
        assert_eq!(rows[0].needed, 9);
        assert_eq!(rows[0].owned, 5);
        assert!(!rows[0].sufficient);
    }

    #[test]
    fn material_delta_summary_marks_sufficient_when_owned_meets_needed() {
        let plan = PlanData::debug_3x3_stone();
        let inv = inventory_with_blocks(&[(block::STONE, 9)]);
        let rows = material_delta_summary(&plan, &inv);
        assert_eq!(rows[0].owned, 9);
        assert!(rows[0].sufficient);
    }

    #[test]
    fn material_delta_summary_treats_excess_as_sufficient() {
        let plan = PlanData::debug_3x3_stone();
        let inv = inventory_with_blocks(&[(block::STONE, 200)]);
        let rows = material_delta_summary(&plan, &inv);
        assert_eq!(rows[0].owned, 200);
        assert!(rows[0].sufficient);
    }

    #[test]
    fn has_sufficient_materials_always_true_in_creative() {
        let rows = vec![MaterialDeltaRow {
            block_id: block::STONE,
            needed: 100,
            owned: 0,
            sufficient: false,
        }];
        assert!(has_sufficient_materials(&rows, true));
    }

    #[test]
    fn has_sufficient_materials_requires_all_rows_in_survival() {
        let rows = vec![
            MaterialDeltaRow { block_id: block::STONE, needed: 5, owned: 5, sufficient: true },
            MaterialDeltaRow { block_id: block::DIRT, needed: 3, owned: 0, sufficient: false },
        ];
        assert!(!has_sufficient_materials(&rows, false));
    }

    #[test]
    fn has_sufficient_materials_returns_true_in_survival_when_all_sufficient() {
        let rows = vec![
            MaterialDeltaRow { block_id: block::STONE, needed: 5, owned: 5, sufficient: true },
            MaterialDeltaRow { block_id: block::DIRT, needed: 3, owned: 7, sufficient: true },
        ];
        assert!(has_sufficient_materials(&rows, false));
    }
}
