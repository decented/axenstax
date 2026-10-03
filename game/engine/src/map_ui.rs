//! Full-screen explorable map UI (#6). Opened with **M** outside the Workshop;
//! shows the explored-tile composite centred on `MapScreen.centre`, zoomable
//! (scroll / ± buttons) and pannable (drag), with waypoint markers + a side
//! list (teleport-in-creative, remove, pin-the-centre). All world/state
//! mutations are returned as [`MapActions`] and applied by the game loop, so this
//! function stays a pure view over borrowed data.
//!
//! Feel (pan/zoom ergonomics, marker density) = playtest boundary.

use crate::minimap::MAP_TEX_PX;
use crate::waypoint::{Waypoint, WaypointKind};

/// Outcomes the map UI requests this frame; the game loop applies them against
/// the live world / player / map state.
#[derive(Default)]
pub struct MapActions {
    pub close: bool,
    /// Drop a manual pin at the current map centre.
    pub pin_here: bool,
    /// Teleport to this waypoint id (creative only — gated by the game loop).
    pub tp: Option<u32>,
    /// Remove this waypoint id.
    pub remove: Option<u32>,
    /// Multiplicative zoom (1.0 = unchanged; <1 zooms in, >1 zooms out).
    pub zoom_factor: f32,
    /// World-block delta to add to the map centre.
    pub pan: (f32, f32),
}

#[allow(clippy::too_many_arguments)]
pub fn draw_map_screen(
    ctx: &egui::Context,
    screen_w_px: f32,
    screen_h_px: f32,
    texture: Option<&egui::TextureHandle>,
    centre: (f32, f32),
    bpp: f32,
    player_pos: glam::Vec3,
    waypoints: &[Waypoint],
    is_creative: bool,
) -> MapActions {
    let mut act = MapActions {
        zoom_factor: 1.0,
        ..Default::default()
    };
    let ppp = ctx.pixels_per_point().max(1.0);
    let (sw, sh) = (screen_w_px / ppp, screen_h_px / ppp);

    // Dim the world behind the map.
    egui::Area::new(egui::Id::new("map_backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(egui::pos2(0.0, 0.0))
        .show(ctx, |ui| {
            let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(sw, sh));
            ui.painter()
                .rect_filled(r, 0.0, egui::Color32::from_black_alpha(180));
        });

    egui::Window::new("Map")
        .id(egui::Id::new("full_map"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                // ---- the map image ------------------------------------------
                let disp = (sh * 0.8).min(sw * 0.6).max(220.0);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(disp, disp), egui::Sense::click_and_drag());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(18, 20, 26));
                if let Some(tex) = texture {
                    painter.image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                painter.rect_stroke(
                    rect,
                    4.0,
                    egui::Stroke::new(1.5_f32, egui::Color32::from_gray(60)),
                    egui::StrokeKind::Inside,
                );

                let pts_per_px = disp / MAP_TEX_PX as f32;
                let world_per_pt = bpp / pts_per_px;

                // Pan by dragging the map.
                if resp.dragged() {
                    let d = resp.drag_delta();
                    act.pan.0 -= d.x * world_per_pt;
                    act.pan.1 -= d.y * world_per_pt;
                }
                // Zoom by scrolling over the map (up = zoom in ⇒ smaller bpp).
                if resp.hovered() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll > 0.0 {
                        act.zoom_factor *= 0.9;
                    } else if scroll < 0.0 {
                        act.zoom_factor *= 1.0 / 0.9;
                    }
                }

                // World (x,z) → on-screen point inside the map rect.
                let to_screen = |wx: f32, wz: f32| -> egui::Pos2 {
                    let (dpx, dpz) =
                        crate::minimap::project_to_map(wx, wz, centre.0, centre.1, bpp);
                    rect.center() + egui::vec2(dpx * pts_per_px, dpz * pts_per_px)
                };

                // Player marker.
                let pp = to_screen(player_pos.x, player_pos.z);
                if rect.contains(pp) {
                    painter.circle_filled(pp, 4.0, egui::Color32::WHITE);
                    painter.circle_stroke(pp, 4.0, egui::Stroke::new(1.0_f32, egui::Color32::BLACK));
                }
                // Waypoint markers + labels.
                for w in waypoints {
                    let p = to_screen(w.pos[0] as f32, w.pos[2] as f32);
                    if !rect.contains(p) {
                        continue;
                    }
                    let col = egui::Color32::from_rgb(w.colour[0], w.colour[1], w.colour[2]);
                    painter.circle_filled(p, 4.0, col);
                    painter.circle_stroke(p, 4.0, egui::Stroke::new(1.0_f32, egui::Color32::BLACK));
                    painter.text(
                        p + egui::vec2(6.0, -6.0),
                        egui::Align2::LEFT_BOTTOM,
                        &w.name,
                        egui::FontId::proportional(11.0),
                        egui::Color32::WHITE,
                    );
                }

                // ---- side panel ---------------------------------------------
                ui.vertical(|ui| {
                    ui.set_min_width(190.0);
                    ui.label(format!("Centre: {:.0}, {:.0}", centre.0, centre.1));
                    ui.label(format!("Zoom: {bpp:.1} blocks/px"));
                    ui.horizontal(|ui| {
                        if ui.button("−").clicked() {
                            act.zoom_factor *= 1.0 / 0.8;
                        }
                        if ui.button("+").clicked() {
                            act.zoom_factor *= 0.8;
                        }
                        if ui.button("Recentre").clicked() {
                            act.pan.0 = player_pos.x - centre.0;
                            act.pan.1 = player_pos.z - centre.1;
                        }
                    });
                    if ui.button("📍 Pin map centre").clicked() {
                        act.pin_here = true;
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Waypoints").strong());
                    egui::ScrollArea::vertical()
                        .max_height((disp - 140.0).max(80.0))
                        .show(ui, |ui| {
                            if waypoints.is_empty() {
                                ui.label(egui::RichText::new("none yet").weak());
                            }
                            for w in waypoints {
                                ui.horizontal(|ui| {
                                    let icon = match w.kind {
                                        WaypointKind::Manual => "📌",
                                        WaypointKind::Death => "💀",
                                    };
                                    ui.label(format!("{icon} {}", w.name));
                                    if ui.small_button("✕").clicked() {
                                        act.remove = Some(w.id);
                                    }
                                    if is_creative && ui.small_button("TP").clicked() {
                                        act.tp = Some(w.id);
                                    }
                                });
                            }
                        });
                    ui.separator();
                    if ui.button("Close (M)").clicked() {
                        act.close = true;
                    }
                });
            });
        });

    act
}
