//! Workshop Phase 5 Task 6 — Wardrobe panel (K key / `/ws gallery`).
//!
//! Shows the per-block design libraries stored in the world's `OverrideRegistry`.
//! Left column: one selectable row per block that has at least one design.
//! Right pane: "Stock (original)" + one row per `NamedDesign` with Set-active /
//! Rename / Delete controls.
//!
//! The panel is draw-only: it never mutates the registry directly. Instead it
//! returns a `WardrobeOutcome` that the game loop applies AFTER the immutable
//! borrow is released (resolving the borrow-checker constraint).

use crate::block::{BlockId, BlockRegistry};
use crate::override_registry::{DesignId, DesignLibrary, NamedDesign};

// ── Constants ───────────────────────────────────────────────────────────────

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 195, 100);
const TICK_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 220, 80);
const CHIP_SIZE: f32 = 18.0;
const PANEL_WIDTH: f32 = 520.0;
const LEFT_COL_WIDTH: f32 = 140.0;

// ── Outcome ──────────────────────────────────────────────────────────────────

/// What the Wardrobe panel asks the caller to do this frame. The caller applies
/// the mutation and calls `apply_active_designs` where needed.
#[derive(Clone, Debug, PartialEq)]
pub enum WardrobeOutcome {
    /// No user action this frame.
    None,
    /// Player closed the panel (Esc or the ✕ button).
    Closed,
    /// Player clicked a block in the left column — update `wardrobe_block`.
    Select(BlockId),
    /// Player clicked "Set active" on a design — set it active + rebuild textures.
    SetActive(BlockId, DesignId),
    /// Player clicked "Use original" — clear active + rebuild textures.
    UseOriginal(BlockId),
    /// Player committed a rename — update the name (no rebuild needed).
    Rename(BlockId, DesignId, String),
    /// Player clicked "Delete" — remove the design + rebuild textures.
    Delete(BlockId, DesignId),
    /// Player toggled "Remember my designs when entering a world".
    SetRemember(bool),
}

// ── Panel ────────────────────────────────────────────────────────────────────

/// Draw the Wardrobe panel. Returns an outcome that the caller applies AFTER the
/// frame (so there is no `&mut self` borrow conflict with the draw closure).
///
/// `block_designs` is `&OverrideRegistry::set().block_designs`.
/// `rename` is `&mut GameState::wardrobe_rename` — the in-progress edit buffer.
pub fn draw_wardrobe_ui(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    block_designs: &[(BlockId, DesignLibrary)],
    registry: &BlockRegistry,
    selected: Option<BlockId>,
    remember: bool,
    rename: &mut Option<(BlockId, DesignId, String)>,
) -> WardrobeOutcome {
    let mut outcome = WardrobeOutcome::None;

    // Dim background covering the player's viewport.
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("wardrobe_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    // Position the panel centred horizontally, near the top of the viewport.
    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - PANEL_WIDTH / 2.0,
        viewport.y as f32 + 70.0,
    );

    egui::Area::new(egui::Id::new(("wardrobe_panel", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(PANEL_WIDTH);
            ui.set_max_width(PANEL_WIDTH);

            // ── Title row ──────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Wardrobe")
                        .size(22.0)
                        .color(TITLE_COLOR)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").clicked() {
                        outcome = WardrobeOutcome::Closed;
                    }
                });
            });
            ui.separator();
            ui.add_space(4.0);

            // ── Remember-on-entry preference ───────────────────────────────
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let mut remember_now = remember;
                if ui
                    .checkbox(&mut remember_now, "Remember my designs when I enter a world")
                    .on_hover_text("Off = enter worlds at the standard look. Your saved designs are kept either way.")
                    .changed()
                {
                    outcome = WardrobeOutcome::SetRemember(remember_now);
                }
            });
            ui.add_space(4.0);

            // ── Two-column layout ──────────────────────────────────────────
            ui.horizontal_top(|ui| {
                // ── LEFT: block list ──────────────────────────────────────
                ui.vertical(|ui| {
                    ui.set_min_width(LEFT_COL_WIDTH);
                    ui.set_max_width(LEFT_COL_WIDTH);
                    ui.label(
                        egui::RichText::new("Blocks")
                            .size(14.0)
                            .color(egui::Color32::LIGHT_GRAY)
                            .strong(),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt(("wardrobe_blocks", player_index))
                        .max_height(350.0)
                        .show(ui, |ui| {
                            if block_designs.is_empty() {
                                ui.label(
                                    egui::RichText::new("No designs yet.\nPin a design to see\nyour wardrobe here.")
                                        .size(12.0)
                                        .color(egui::Color32::GRAY),
                                );
                            }
                            for (block_id, _lib) in block_designs {
                                let block_id = *block_id;
                                // Use the block registry name if available; fall back
                                // to "Block <id>" (acceptable v1 fallback as noted in spec).
                                // `plan_ui::display_block_name` calls `Item::Block(id).name(registry)`,
                                // which title-cases the registry entry's `.name` field and strips
                                // the namespace — the same helper used in the Blueprint inspect dialog.
                                let block_name =
                                    crate::plan_ui::display_block_name(registry, block_id);
                                let is_selected = selected == Some(block_id);
                                let resp = ui.selectable_label(is_selected, &block_name);
                                if resp.clicked() {
                                    outcome = WardrobeOutcome::Select(block_id);
                                }
                            }
                        });
                });

                ui.separator();

                // ── RIGHT: design list for selected block ─────────────────
                ui.vertical(|ui| {
                    let right_width = PANEL_WIDTH - LEFT_COL_WIDTH - 24.0;
                    ui.set_min_width(right_width);
                    ui.set_max_width(right_width);

                    let selected_block = selected;
                    let lib_opt = selected_block.and_then(|bid| {
                        block_designs.iter().find(|(b, _)| *b == bid).map(|(_, l)| l)
                    });

                    match (selected_block, lib_opt) {
                        (Some(bid), Some(lib)) => {
                            ui.label(
                                egui::RichText::new("Designs")
                                    .size(14.0)
                                    .color(egui::Color32::LIGHT_GRAY)
                                    .strong(),
                            );
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .id_salt(("wardrobe_designs", player_index))
                                .max_height(350.0)
                                .show(ui, |ui| {
                                    // ── Stock (original) row ──────────────
                                    let is_original_active = lib.active.is_none();
                                    ui.horizontal(|ui| {
                                        // Grey chip for "original"
                                        colour_chip(ui, [120, 120, 120, 255]);
                                        ui.label("Stock (original)");
                                        if is_original_active {
                                            ui.label(
                                                egui::RichText::new("✓")
                                                    .color(TICK_COLOR)
                                                    .strong(),
                                            );
                                        }
                                        if ui.button("Use original").clicked() {
                                            outcome = WardrobeOutcome::UseOriginal(bid);
                                        }
                                    });
                                    ui.separator();

                                    // ── One row per named design ──────────
                                    for design in &lib.designs {
                                        let is_active =
                                            lib.active == Some(design.id);
                                        let is_renaming = rename
                                            .as_ref()
                                            .map(|(b, id, _)| *b == bid && *id == design.id)
                                            .unwrap_or(false);

                                        ui.horizontal(|ui| {
                                            // Colour chip — sample face 4 (south/+z)
                                            // first texel as the swatch. For a
                                            // shape-only design we show neutral grey.
                                            let chip_rgba = design_chip_colour(design);
                                            colour_chip(ui, chip_rgba);

                                            if is_active {
                                                ui.label(
                                                    egui::RichText::new("✓")
                                                        .color(TICK_COLOR)
                                                        .strong(),
                                                );
                                            } else {
                                                ui.add_space(14.0);
                                            }

                                            if is_renaming {
                                                // In-progress rename field.
                                                if let Some((_, _, buf)) = rename {
                                                    let resp = ui.text_edit_singleline(buf);
                                                    if resp.lost_focus()
                                                        && ui.input(|i| {
                                                            i.key_pressed(egui::Key::Enter)
                                                        })
                                                    {
                                                        let new_name = buf.trim().to_string();
                                                        if !new_name.is_empty() {
                                                            outcome = WardrobeOutcome::Rename(
                                                                bid,
                                                                design.id,
                                                                new_name,
                                                            );
                                                        } else {
                                                            // Cancel empty rename silently.
                                                            outcome = WardrobeOutcome::None;
                                                        }
                                                    }
                                                }
                                            } else {
                                                ui.label(&design.name);
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            ui.add_space(CHIP_SIZE + 18.0);
                                            if !is_active && ui.button("Set active").clicked() {
                                                outcome = WardrobeOutcome::SetActive(
                                                    bid, design.id,
                                                );
                                            }
                                            if is_renaming {
                                                if ui.button("Cancel").clicked() {
                                                    *rename = None;
                                                }
                                            } else if ui.button("Rename").clicked() {
                                                *rename = Some((bid, design.id, design.name.clone()));
                                            }
                                            if ui.button("Delete").clicked() {
                                                outcome = WardrobeOutcome::Delete(bid, design.id);
                                            }
                                        });
                                        ui.add_space(2.0);
                                    }
                                });
                        }
                        _ => {
                            ui.label(
                                egui::RichText::new("Select a block on the left\nto manage its designs.")
                                    .size(13.0)
                                    .color(egui::Color32::GRAY),
                            );
                        }
                    }
                });
            });

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Esc or ✕ to close.")
                    .size(12.0)
                    .color(egui::Color32::DARK_GRAY),
            );
        });

    // Esc closes the panel.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = WardrobeOutcome::Closed;
    }

    outcome
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Draw a small filled square colour chip inline with the current row.
fn colour_chip(ui: &mut egui::Ui, rgba: [u8; 4]) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(CHIP_SIZE, CHIP_SIZE),
        egui::Sense::hover(),
    );
    let colour = egui::Color32::from_rgba_premultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
    ui.painter().rect_filled(rect, 2.0, colour);
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0_f32, egui::Color32::DARK_GRAY),
        egui::StrokeKind::Inside,
    );
}

/// Pick a representative RGBA colour for the design's swatch.
///
/// For a paint design we sample face 4 (south / +z side face) first texel.
/// For a shape-only design (no `faces`) we return a neutral mid-grey.
fn design_chip_colour(design: &NamedDesign) -> [u8; 4] {
    if let Some(authored) = &design.faces {
        // Face 4 = south (+z), consistent with the mesh::Face ordering used throughout.
        // Each face is a 16×16 RGBA buffer; first four bytes = top-left texel.
        let face = &authored.faces[4];
        if face.len() >= 4 {
            return [face[0], face[1], face[2], face[3]];
        }
    }
    // Shape-only or invalid — neutral grey chip.
    [140, 140, 140, 255]
}
