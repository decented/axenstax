//! The Workshop face-painter — the tactile 16×16 paint-grid UI (Spec 40 Phase C).
//!
//! "Blow the asset up to a comfortable working size and paint its faces." Here the
//! faces are shown large in an egui panel: six 16×16 grids (one per cube
//! direction), painted cell-by-cell from a colour palette, seeded from the asset's
//! **current** texture so you re-skin what's there. Pinning writes the painted
//! faces into the override registry — every instance of that asset updates
//! (`workshop::commit_project_override`, the same path the `/ws` command uses).
//!
//! State + paint logic live here (unit-tested); the egui rendering is
//! `draw_workshop_painter`. Blocks lead; a single mob part plugs into the same
//! six-face buffer via [`PaintTarget::MobPart`].

use crate::override_registry::{AuthoredFaces, FACE_BYTES};
use crate::workshop::{WorkshopPaint, WorkshopTarget};

/// Grid resolution — one cell per texel (16×16), the block's native texture size.
pub const GRID: usize = 16;

/// What the painter is editing. A block paints all six faces (indexed by
/// `mesh::Face::index()`); a mob paints one part's six faces (the part's
/// `tex_faces` slot order) at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaintTarget {
    Block(crate::block::BlockId),
    MobPart { mob: crate::mob::MobType, part: u8 },
}

/// The kid-friendly painter palette — 16 solid colours (RGBA). Mirrors the
/// dyed-`WALLPAPER_*` "paint-with-blocks" idea without coupling to block ids.
pub const PALETTE: [[u8; 4]; 16] = [
    [20, 20, 24, 255],    // near-black
    [120, 120, 130, 255], // grey
    [235, 235, 240, 255], // white
    [196, 64, 54, 255],   // red
    [224, 122, 40, 255],  // orange
    [240, 206, 70, 255],  // yellow
    [120, 184, 64, 255],  // lime
    [54, 132, 70, 255],   // green
    [64, 168, 176, 255],  // cyan
    [60, 110, 200, 255],  // blue
    [40, 60, 140, 255],   // deep blue
    [128, 80, 190, 255],  // purple
    [200, 110, 180, 255], // magenta
    [150, 96, 60, 255],   // brown
    [230, 170, 140, 255], // skin/tan
    [248, 168, 196, 255], // pink
];

/// The painter UI state.
pub struct WorkshopPainterUi {
    pub open: bool,
    pub target: Option<PaintTarget>,
    /// Six 16×16 RGBA face buffers (each [`FACE_BYTES`]). Index = face slot.
    pub faces: [Vec<u8>; 6],
    /// Index into [`PALETTE`] of the active colour.
    pub selected: usize,
    /// Cursor position in normalised device coords (for click mapping if
    /// needed). Set at init, never read — speculative field, no consumer yet.
    #[allow(dead_code)]
    pub mouse_ndc: [f32; 2],
}

impl Default for WorkshopPainterUi {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkshopPainterUi {
    pub fn new() -> Self {
        Self {
            open: false,
            target: None,
            faces: blank_faces(),
            selected: 3, // red — an obvious default so the first stroke shows
            mouse_ndc: [0.0, 0.0],
        }
    }

    /// Open the painter for `target`, seeding each face from the asset's current
    /// texture (so the author tweaks what's there). `textures` is
    /// `texture_gen::generate_textures()`; `registry` resolves a block's per-face
    /// layers.
    pub fn open_for(
        &mut self,
        target: PaintTarget,
        textures: &[Vec<u8>],
        registry: &crate::block::BlockRegistry,
    ) {
        self.faces = seed_faces(&target, textures, registry);
        self.target = Some(target);
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.target = None;
        self.faces = blank_faces();
    }

    /// Paint one cell of `face` (0..6) at grid `(gx, gy)` with the active colour.
    /// Out-of-range coordinates are ignored.
    pub fn paint_cell(&mut self, face: usize, gx: usize, gy: usize) {
        if face >= 6 || gx >= GRID || gy >= GRID {
            return;
        }
        let rgba = PALETTE[self.selected.min(PALETTE.len() - 1)];
        let i = (gy * GRID + gx) * 4;
        let buf = &mut self.faces[face];
        if i + 4 <= buf.len() {
            buf[i..i + 4].copy_from_slice(&rgba);
        }
    }

    /// Read the RGBA of a cell (for rendering the grid).
    pub fn cell_color(&self, face: usize, gx: usize, gy: usize) -> [u8; 4] {
        let buf = &self.faces[face];
        let i = (gy * GRID + gx) * 4;
        if i + 4 <= buf.len() {
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        } else {
            [0, 0, 0, 0]
        }
    }

    /// The authored faces as a [`WorkshopPaint`] ready for
    /// `workshop::commit_project_override`, plus the bound [`WorkshopTarget`].
    pub fn to_paint(&self) -> Option<(WorkshopTarget, WorkshopPaint)> {
        let target = self.target.clone()?;
        let authored = AuthoredFaces::from_faces([
            self.faces[0].clone(),
            self.faces[1].clone(),
            self.faces[2].clone(),
            self.faces[3].clone(),
            self.faces[4].clone(),
            self.faces[5].clone(),
        ]);
        Some(match target {
            PaintTarget::Block(id) => (WorkshopTarget::Block(id), WorkshopPaint::Block(authored)),
            PaintTarget::MobPart { mob, part } => (
                WorkshopTarget::Mob(mob),
                WorkshopPaint::Mob(vec![(part, authored)]),
            ),
        })
    }
}

/// Six blank (transparent) face buffers.
fn blank_faces() -> [Vec<u8>; 6] {
    [
        vec![0; FACE_BYTES],
        vec![0; FACE_BYTES],
        vec![0; FACE_BYTES],
        vec![0; FACE_BYTES],
        vec![0; FACE_BYTES],
        vec![0; FACE_BYTES],
    ]
}

/// Copy a texture-array layer into a fresh 16×16 RGBA buffer, or blank if absent.
fn layer_buf(textures: &[Vec<u8>], layer: u32) -> Vec<u8> {
    textures
        .get(layer as usize)
        .filter(|t| t.len() == FACE_BYTES)
        .cloned()
        .unwrap_or_else(|| vec![0; FACE_BYTES])
}

/// Seed the six face buffers from the asset's current textures.
fn seed_faces(
    target: &PaintTarget,
    textures: &[Vec<u8>],
    registry: &crate::block::BlockRegistry,
) -> [Vec<u8>; 6] {
    match target {
        PaintTarget::Block(id) => {
            // Block-face order = mesh::Face::index(): Top, Bottom, North, South,
            // East, West. A block only distinguishes top/bottom/side, so the four
            // sides seed from `tex_side`.
            let top = layer_buf(textures, registry.tex_top(*id));
            let bottom = layer_buf(textures, registry.tex_bottom(*id));
            let side = layer_buf(textures, registry.tex_side(*id));
            [
                top,
                bottom,
                side.clone(),
                side.clone(),
                side.clone(),
                side,
            ]
        }
        PaintTarget::MobPart { mob, part } => {
            let model = crate::entity_model::mob_model(*mob);
            if let Some(p) = model.get(*part as usize) {
                [
                    layer_buf(textures, p.tex_faces[0]),
                    layer_buf(textures, p.tex_faces[1]),
                    layer_buf(textures, p.tex_faces[2]),
                    layer_buf(textures, p.tex_faces[3]),
                    layer_buf(textures, p.tex_faces[4]),
                    layer_buf(textures, p.tex_faces[5]),
                ]
            } else {
                blank_faces()
            }
        }
    }
}

/// Terminal action from the painter panel for one frame.
#[derive(Debug, PartialEq, Eq)]
pub enum PainterResult {
    None,
    /// Commit the painted faces (the caller reads `to_paint()` then closes).
    Pin,
    /// Discard + close.
    Cancel,
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 220);
#[allow(dead_code)]
const PANEL_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(22, 22, 30, 255);
const GRID_BACKDROP: egui::Color32 = egui::Color32::from_rgb(44, 44, 56);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 224, 178);

/// Block-face order labels (= `mesh::Face::index()`); also reused for mob parts.
const FACE_LABELS: [&str; 6] = ["Top", "Bottom", "North", "South", "East", "West"];

/// Draw the face-painter panel. Painting mutates `ui_state.faces` directly;
/// returns [`PainterResult`] only for the terminal Pin/Cancel actions.
pub fn draw_workshop_painter(
    ctx: &egui::Context,
    _viewport: &crate::screen::ViewportRect,
    ui_state: &mut WorkshopPainterUi,
) -> PainterResult {
    let mut result = PainterResult::None;

    // CentralPanel fills the window and lays out reliably in a single (headless)
    // frame — unlike a bare fixed-pos Area, which can collapse to zero size before
    // its first measured frame. The frame fill IS the dimming overlay.
    //
    // `.show(ctx, ..)` is deprecated in favour of `.show_inside(ui, ..)`, but this
    // is a genuine top-level panel (no enclosing Ui) — egui 0.34 has no
    // non-deprecated top-level entry point for CentralPanel.
    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(OVERLAY_BG))
        .show(ctx, |ui| {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                let title = match &ui_state.target {
                    Some(PaintTarget::Block(_)) => "The Workshop — repaint this block".to_string(),
                    Some(PaintTarget::MobPart { part, .. }) => {
                        format!("The Workshop — repaint mob part {part}")
                    }
                    None => "The Workshop".to_string(),
                };
                ui.label(egui::RichText::new(title).size(22.0).color(TITLE_COLOR).strong());
                ui.add_space(10.0);

                // Palette row.
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - (16.0 * 30.0)) / 2.0);
                    for (i, c) in PALETTE.iter().enumerate() {
                        let col = egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
                        ui.painter().rect_filled(rect, 3.0, col);
                        if i == ui_state.selected {
                            ui.painter().rect_stroke(
                                rect,
                                3.0,
                                egui::Stroke::new(3.0_f32, egui::Color32::WHITE),
                                egui::StrokeKind::Outside,
                            );
                        }
                        if resp.clicked() {
                            ui_state.selected = i;
                        }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(14.0);

                // Six face grids, 3 across × 2 down.
                for row in 0..2 {
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - (3.0 * 210.0)) / 2.0);
                        for col in 0..3 {
                            let face = row * 3 + col;
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(FACE_LABELS[face])
                                        .size(12.0)
                                        .color(egui::Color32::from_rgb(150, 150, 160)),
                                );
                                draw_face_grid(ui, ui_state, face);
                            });
                            ui.add_space(10.0);
                        }
                    });
                    ui.add_space(10.0);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 320.0) / 2.0);
                    let pin = egui::Button::new(
                        egui::RichText::new("📌 Pin — apply to every one").size(15.0).strong(),
                    )
                    .min_size(egui::vec2(220.0, 38.0))
                    .fill(egui::Color32::from_rgb(54, 132, 70));
                    if ui.add(pin).clicked() {
                        result = PainterResult::Pin;
                    }
                    ui.add_space(8.0);
                    let cancel = egui::Button::new(egui::RichText::new("Cancel").size(15.0))
                        .min_size(egui::vec2(90.0, 38.0))
                        .fill(egui::Color32::from_rgb(70, 50, 50));
                    if ui.add(cancel).clicked() {
                        result = PainterResult::Cancel;
                    }
                });
            });
        });

    result
}

/// Render one 16×16 face grid and paint the cell under the pointer on click/drag.
fn draw_face_grid(ui: &mut egui::Ui, ui_state: &mut WorkshopPainterUi, face: usize) {
    let cell = 12.0;
    let size = GRID as f32 * cell;
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, GRID_BACKDROP);
    for gy in 0..GRID {
        for gx in 0..GRID {
            let c = ui_state.cell_color(face, gx, gy);
            if c[3] == 0 {
                continue; // transparent → show the backdrop
            }
            let col = egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            let cell_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(gx as f32 * cell, gy as f32 * cell),
                egui::vec2(cell, cell),
            );
            painter.rect_filled(cell_rect, 0.0, col);
        }
    }
    // Thin border so the grid reads as an editable surface.
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(80, 80, 96)),
        egui::StrokeKind::Outside,
    );
    if (resp.clicked() || resp.dragged())
        && let Some(pos) = resp.interact_pointer_pos() {
            let local = pos - rect.min;
            let gx = (local.x / cell).floor() as i32;
            let gy = (local.y / cell).floor() as i32;
            if gx >= 0 && gy >= 0 && (gx as usize) < GRID && (gy as usize) < GRID {
                ui_state.paint_cell(face, gx as usize, gy as usize);
            }
        }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paint_cell_sets_the_selected_colour() {
        let mut ui = WorkshopPainterUi::new();
        ui.selected = 5; // yellow
        ui.paint_cell(2, 3, 4);
        assert_eq!(ui.cell_color(2, 3, 4), PALETTE[5]);
        // A neighbour is untouched.
        assert_eq!(ui.cell_color(2, 4, 4), [0, 0, 0, 0]);
    }

    #[test]
    fn paint_cell_ignores_out_of_range() {
        let mut ui = WorkshopPainterUi::new();
        ui.paint_cell(6, 0, 0); // bad face
        ui.paint_cell(0, GRID, 0); // bad x
        ui.paint_cell(0, 0, GRID); // bad y
        // Nothing painted anywhere.
        for f in 0..6 {
            for y in 0..GRID {
                for x in 0..GRID {
                    assert_eq!(ui.cell_color(f, x, y), [0, 0, 0, 0]);
                }
            }
        }
    }

    #[test]
    fn seed_block_faces_from_current_textures() {
        let registry = crate::block::BlockRegistry::new();
        let textures = crate::texture_gen::generate_textures();
        let mut ui = WorkshopPainterUi::new();
        ui.open_for(PaintTarget::Block(crate::block::STONE), &textures, &registry);
        assert!(ui.open);
        // The top face was seeded from stone's top layer (16×16 RGBA).
        assert_eq!(ui.faces[0].len(), FACE_BYTES);
        assert_eq!(ui.faces[0], textures[registry.tex_top(crate::block::STONE) as usize]);
    }

    #[test]
    fn to_paint_block_yields_block_override() {
        let registry = crate::block::BlockRegistry::new();
        let textures = crate::texture_gen::generate_textures();
        let mut ui = WorkshopPainterUi::new();
        ui.open_for(PaintTarget::Block(crate::block::CORNFLOWER), &textures, &registry);
        ui.selected = 7;
        ui.paint_cell(0, 0, 0);
        let (target, paint) = ui.to_paint().expect("has a target");
        assert_eq!(target, WorkshopTarget::Block(crate::block::CORNFLOWER));
        assert!(matches!(paint, WorkshopPaint::Block(_)));
    }

    #[test]
    fn to_paint_mob_yields_per_part_override() {
        let registry = crate::block::BlockRegistry::new();
        let textures = crate::texture_gen::generate_textures();
        let mut ui = WorkshopPainterUi::new();
        ui.open_for(
            PaintTarget::MobPart { mob: crate::mob::MobType::Cow, part: 1 },
            &textures,
            &registry,
        );
        let (target, paint) = ui.to_paint().expect("has a target");
        assert_eq!(target, WorkshopTarget::Mob(crate::mob::MobType::Cow));
        match paint {
            WorkshopPaint::Mob(parts) => {
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].0, 1, "edits part index 1");
            }
            _ => panic!("expected mob paint"),
        }
    }

    #[test]
    fn close_resets() {
        let mut ui = WorkshopPainterUi::new();
        let registry = crate::block::BlockRegistry::new();
        let textures = crate::texture_gen::generate_textures();
        ui.open_for(PaintTarget::Block(crate::block::DIRT), &textures, &registry);
        ui.close();
        assert!(!ui.open);
        assert!(ui.target.is_none());
        assert!(ui.to_paint().is_none());
    }
}
