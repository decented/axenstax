//! HUD rendering via egui — hotbar, hearts, block name, debug overlay.
//!
//! All HUD elements use egui's modern, anti-aliased rendering.
//! Block textures in hotbar slots are displayed as egui managed textures.

use std::collections::HashMap;

use crate::block::BlockRegistry;
use crate::egui_integration::EguiIntegration;
use crate::inventory::Inventory;
use crate::item::Item;
use crate::remote_client::RemoteIdentity;
use crate::reserve::{ReserveState, RichnessTier};
use crate::screen::ViewportRect;

/// Theme colours for the HUD.
const SLOT_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(20, 20, 28, 200);
const SLOT_BORDER: egui::Color32 = egui::Color32::from_rgba_premultiplied(60, 60, 80, 150);
const SLOT_SELECTED_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(40, 50, 65, 230);
const SLOT_SELECTED_BORDER: egui::Color32 = egui::Color32::from_rgba_premultiplied(120, 170, 255, 255);
const HEART_FULL: egui::Color32 = egui::Color32::from_rgb(220, 30, 30);
const HEART_HALF: egui::Color32 = egui::Color32::from_rgb(160, 20, 20);
const HEART_EMPTY: egui::Color32 = egui::Color32::from_rgba_premultiplied(50, 40, 40, 200);
const DRUMSTICK_FULL: egui::Color32 = egui::Color32::from_rgb(180, 130, 60);
const DRUMSTICK_HALF: egui::Color32 = egui::Color32::from_rgb(140, 100, 45);
const DRUMSTICK_EMPTY: egui::Color32 = egui::Color32::from_rgba_premultiplied(45, 35, 25, 200);
const GOLD_TEXT: egui::Color32 = egui::Color32::from_rgb(212, 160, 68);

/// Minimum viewport width (px) the crafting panel can fit into without
/// truncating. The crafting Area is laid out at 600 px wide; below this
/// the inventory-toggle handler refuses to open it and toasts instead.
pub const MIN_VIEWPORT_FOR_CRAFTING: u32 = 600;

/// Inspect view (Phase 4) — a collapsible top-right panel listing present remote
/// players with their full, **copyable** npub, so a specific person can be
/// verified beyond the grindable collision suffix. Call only when `roster` is
/// non-empty (multiplayer). Default-collapsed (just a title bar) so it stays
/// unobtrusive until the user expands it. A guest / unverified joiner shows no
/// npub. Fixed window id so the count in the title doesn't reset its state.
pub fn draw_player_inspect(
    ctx: &egui::Context,
    roster: &HashMap<u32, RemoteIdentity>,
    cursor_captured: bool,
) {
    if roster.is_empty() {
        return;
    }
    let mut entries: Vec<(&u32, &RemoteIdentity)> = roster.iter().collect();
    entries.sort_by_key(|(slot, _)| **slot);
    egui::Window::new(format!("Players ({})", entries.len()))
        .id(egui::Id::new("player_inspect"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
        .collapsible(true)
        .default_open(false)
        .resizable(false)
        // Task 16 — while the cursor is captured (mouse-look/mining) the
        // invisible pointer may rest over this window; if it hit-tested then,
        // `wants_pointer_input()` would freeze gameplay input. The Copy /
        // expand affordances are only usable with a free cursor anyway.
        .interactable(!cursor_captured)
        .show(ctx, |ui| {
            for (_slot, id) in entries {
                ui.label(egui::RichText::new(&id.handle).strong().color(GOLD_TEXT));
                if id.npub.is_empty() {
                    ui.label(egui::RichText::new("guest — no verified npub").small().weak());
                } else {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&id.npub).small().monospace());
                        if ui.small_button("Copy").clicked() {
                            ctx.copy_text(id.npub.clone());
                        }
                    });
                }
                ui.separator();
            }
        });
}

/// Wave 2c — read-on-look sign panel. When the player targets a Sign with
/// text, its lines surface in a small parchment popup near the top-centre of
/// the viewport (the in-world board text is baked-render polish for later).
pub fn draw_sign_text(ctx: &egui::Context, viewport: &ViewportRect, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let ppp = ctx.pixels_per_point();
    let vp = ViewportPts::from_physical(viewport, ppp);
    egui::Area::new(egui::Id::new("sign_readout"))
        .order(egui::Order::Foreground)
        // Task 16 — pure-display HUD: must not hit-test, or the (invisible,
        // confined) cursor resting over it makes `wants_pointer_input()` true
        // and the egui focus gate freezes gameplay input.
        .interactable(false)
        .fixed_pos(egui::pos2(vp.x + vp.width * 0.5 - 110.0, vp.y + vp.height * 0.18))
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style()).show(ui, |ui| {
                ui.set_max_width(220.0);
                for line in text.split('\n') {
                    ui.label(egui::RichText::new(line).color(GOLD_TEXT).size(15.0));
                }
            });
        });
}

/// #6 — corner minimap (north-up). Draws the last baked map texture in the
/// top-right of this player's viewport with a heading arrow at the centre.
/// `texture` is `None` until the first bake (or when the minimap is disabled /
/// in the Workshop), in which case nothing draws. Waypoint markers land in a
/// later increment. Feel (size, rotate-vs-north-up) = playtest.
/// P8 — rain overlay. A faint overcast tint + animated diagonal streaks drawn
/// in screen space (the engine has no 3D particle system yet) plus a small
/// "Rain" indicator. `tick` drives the fall animation. Drawn over the world but
/// under the HUD widgets (Background layer).
/// TB-10 ("i dont get it") — pick the held-Plan guidance line. Pure so the
/// wall-vs-floor overload is unit-tested: right-click on a WALL face hangs
/// the plan as a picture (and consumes it!), on the ground it opens the
/// build dialog. The hint warns which one the current aim will do.
pub fn plan_hint_text(aiming_at_wall: bool) -> &'static str {
    if aiming_at_wall {
        "📜 Careful — right-click on a WALL hangs this plan as a picture. Aim at the GROUND to build it."
    } else {
        "📜 Right-click the ground to open this plan and build it"
    }
}

/// TB-10 — persistent bottom-centre hint while a Plan is the held item.
/// (The old guidance was a single 4-second toast at gift time — gone by the
/// time a kid experimented; this stays up while the plan is in hand.)
pub fn draw_plan_hint(ctx: &egui::Context, viewport: &ViewportRect, player_index: usize, aiming_at_wall: bool) {
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 210.0,
        viewport.y as f32 + viewport.height as f32 - 96.0,
    );
    egui::Area::new(egui::Id::new(("plan_hint", player_index)))
        .fixed_pos(origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(12, 12, 16, 210))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(plan_hint_text(aiming_at_wall))
                            .size(14.0)
                            .color(egui::Color32::from_rgb(255, 220, 140)),
                    );
                });
        });
}

/// TB-10 — persistent ghost-placement controls (replaces the 4-second toast).
pub fn draw_ghost_controls(ctx: &egui::Context, viewport: &ViewportRect, player_index: usize) {
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 180.0,
        viewport.y as f32 + viewport.height as f32 - 96.0,
    );
    egui::Area::new(egui::Id::new(("ghost_controls", player_index)))
        .fixed_pos(origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(12, 12, 16, 210))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Left-click: build here  ·  Q/E: rotate  ·  Right-click: cancel")
                            .size(14.0)
                            .color(egui::Color32::LIGHT_GRAY),
                    );
                });
        });
}

pub fn draw_rain_overlay(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    tick: u64,
    legacy_streaks: bool,
) {
    let ppp = ctx.pixels_per_point();
    let vp = ViewportPts::from_physical(viewport, ppp);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("rain_overlay"),
    ));
    // Overcast tint across the viewport.
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(vp.x, vp.y), egui::vec2(vp.width, vp.height)),
        0.0,
        egui::Color32::from_rgba_unmultiplied(40, 55, 80, 26),
    );
    // Falling streaks — deterministic per-streak x + speed, animated by `tick`.
    // Since 2026-07-05 real 3D rain particles replace these; the 2D streaks
    // only draw when particles are Off (`legacy_streaks`). The overcast tint
    // above stays in both modes.
    if !legacy_streaks {
        return;
    }
    let stroke = egui::Stroke::new(1.0_f32, egui::Color32::from_rgba_unmultiplied(180, 200, 230, 110));
    let span = vp.height + 24.0;
    let w = vp.width.max(1.0) as u32;
    for i in 0u32..70 {
        let hsh = i.wrapping_mul(2_654_435_761);
        let x = vp.x + (hsh % w) as f32;
        let speed = 6.0 + ((hsh >> 5) % 40) as f32 * 0.15;
        let seed = ((hsh >> 11) % 1000) as f32;
        let y = vp.y + ((tick as f32 * speed + seed) % span) - 12.0;
        painter.line_segment([egui::pos2(x, y), egui::pos2(x - 2.5, y + 13.0)], stroke);
    }
    // Indicator (plain text — the egui font may lack weather glyphs).
    painter.text(
        egui::pos2(vp.x + vp.width * 0.5, vp.y + 8.0),
        egui::Align2::CENTER_TOP,
        "Rain",
        egui::FontId::proportional(13.0),
        egui::Color32::from_rgba_unmultiplied(200, 215, 235, 170),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_minimap(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    pidx: usize,
    texture: Option<&egui::TextureHandle>,
    yaw: f32,
    player_pos: glam::Vec3,
    blocks_per_pixel: f32,
    waypoints: &[crate::waypoint::Waypoint],
) {
    let Some(tex) = texture else {
        return;
    };
    let ppp = ctx.pixels_per_point();
    let vp = ViewportPts::from_physical(viewport, ppp);
    let size = crate::minimap::MINIMAP_DISPLAY_PTS;
    let margin = 8.0;
    let x = vp.x + vp.width - size - margin;
    let y = vp.y + margin;
    egui::Area::new(egui::Id::new(("minimap", pidx)))
        .order(egui::Order::Foreground)
        // Task 16 — display-only: keep out of egui hit-testing so hovering it
        // can't trip the `wants_pointer_input()` gameplay gate.
        .interactable(false)
        .fixed_pos(egui::pos2(x, y))
        .show(ctx, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
            let painter = ui.painter();
            // Backdrop covers transparent (unexplored) map pixels, then the map.
            painter.rect_filled(
                rect,
                4.0,
                egui::Color32::from_rgba_unmultiplied(20, 22, 28, 200),
            );
            painter.image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_gray(30)),
                egui::StrokeKind::Inside,
            );
            // Waypoint dots. Project each onto the map (north-up) relative to the
            // player; only those inside the frame draw (off-map pins show on the
            // full-screen map). One texture-pixel = `pts_per_px` on screen.
            let centre = rect.center();
            let pts_per_px = size / crate::minimap::MINIMAP_TEX_PX as f32;
            for w in waypoints {
                let (dpx, dpz) = crate::minimap::project_to_map(
                    w.pos[0] as f32,
                    w.pos[2] as f32,
                    player_pos.x,
                    player_pos.z,
                    blocks_per_pixel,
                );
                let p = centre + egui::vec2(dpx * pts_per_px, dpz * pts_per_px);
                if !rect.contains(p) {
                    continue;
                }
                let col = egui::Color32::from_rgb(w.colour[0], w.colour[1], w.colour[2]);
                painter.circle_filled(p, 3.5, col);
                painter.circle_stroke(p, 3.5, egui::Stroke::new(1.0_f32, egui::Color32::BLACK));
            }
            // Heading arrow at the centre. Forward on a north-up map is
            // (−sin yaw, −cos yaw); its right-perpendicular is (cos yaw, −sin yaw).
            let c = rect.center();
            let (s, co) = yaw.sin_cos();
            let fwd = egui::vec2(-s, -co);
            let right = egui::vec2(co, -s);
            let tip = c + fwd * 8.0;
            let bl = c - fwd * 5.0 + right * 5.0;
            let br = c - fwd * 5.0 - right * 5.0;
            painter.add(egui::Shape::convex_polygon(
                vec![tip, bl, br],
                egui::Color32::WHITE,
                egui::Stroke::new(1.0_f32, egui::Color32::BLACK),
            ));
        });
}

/// Guided build-along — the "how do you want this built?" choice shown after
/// confirming a plan's placement with the materials in hand (or in Creative).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildChoice {
    None,
    /// Build it automatically (the animated builder).
    Auto,
    /// Build it myself, guided one block at a time.
    GuideBlocks,
    /// Build it myself, guided one layer at a time (the Lego-instructions read).
    GuideLayers,
    /// Back out — keep the plan, place nothing.
    Cancel,
}

/// Render the build-choice panel. Returns the player's pick this frame. Pure UI.
pub fn draw_build_choice(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    plan_name: &str,
) -> BuildChoice {
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("build_choice_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 170));

    let panel_w = 460.0;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        // Four buttons + the strapline — taller than the original two-button
        // panel, so lift the origin to keep it centred.
        viewport.y as f32 + viewport.height as f32 / 2.0 - 150.0,
    );
    let mut choice = BuildChoice::None;
    egui::Area::new(egui::Id::new(("build_choice", player_index)))
        .fixed_pos(origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(format!("How do you want «{plan_name}» built?"))
                        .size(18.0)
                        .strong()
                        .color(egui::Color32::from_rgb(230, 220, 150)),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Have it built for you, or build it yourself with the guide showing \
                         you every step.",
                    )
                    .size(12.0)
                    .color(egui::Color32::from_rgb(190, 190, 200)),
                );
                ui.add_space(12.0);
                let button = |ui: &mut egui::Ui, text: &str, fill: egui::Color32| -> bool {
                    ui.add_sized(
                        [300.0, 34.0],
                        egui::Button::new(
                            egui::RichText::new(text).size(15.0).color(egui::Color32::WHITE),
                        )
                        .fill(fill),
                    )
                    .clicked()
                };
                if button(ui, "Build it automatically", egui::Color32::from_rgb(60, 140, 70)) {
                    choice = BuildChoice::Auto;
                }
                ui.add_space(8.0);
                if button(
                    ui,
                    "Guide me — block by block",
                    egui::Color32::from_rgb(70, 120, 160),
                ) {
                    choice = BuildChoice::GuideBlocks;
                }
                ui.add_space(8.0);
                if button(
                    ui,
                    "Guide me — layer by layer",
                    egui::Color32::from_rgb(70, 100, 170),
                ) {
                    choice = BuildChoice::GuideLayers;
                }
                ui.add_space(8.0);
                if button(ui, "Cancel", egui::Color32::from_rgb(80, 80, 100)) {
                    choice = BuildChoice::Cancel;
                }
                ui.add_space(14.0);
            });
        });
    choice
}

/// Trials (Race) — the live race clock: current time big, your best below.
/// Top-centre so it reads like a race timer.
#[allow(clippy::too_many_arguments)]
pub fn draw_trial_clock(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    name: &str,
    elapsed_ticks: u32,
    best_ticks: Option<u32>,
    blocks_done: u32,
    blocks_total: u32,
) {
    let ppp = ctx.pixels_per_point();
    let vp = ViewportPts::from_physical(viewport, ppp);
    egui::Area::new(egui::Id::new("trial_clock"))
        .order(egui::Order::Foreground)
        // Task 16 — display-only: don't hit-test (see sign_readout).
        .interactable(false)
        .fixed_pos(egui::pos2(vp.x + vp.width / 2.0 - 90.0, vp.y + 10.0))
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(15, 25, 40, 210))
                .show(ui, |ui| {
                    ui.set_width(170.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!("⚡ {name}"))
                                .size(12.0)
                                .color(egui::Color32::from_rgb(150, 205, 255)),
                        );
                        ui.label(
                            egui::RichText::new(crate::trials::format_time(elapsed_ticks))
                                .size(26.0)
                                .strong()
                                .color(egui::Color32::WHITE),
                        );
                        // Distance progress — "how far have I run?" the clock alone
                        // never answered. A bar + "X / N blocks" readout.
                        if blocks_total > 0 {
                            let frac = (blocks_done as f32 / blocks_total as f32).clamp(0.0, 1.0);
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .desired_width(150.0)
                                    .fill(egui::Color32::from_rgb(120, 200, 255))
                                    .text(
                                        egui::RichText::new(format!(
                                            "{blocks_done} / {blocks_total} blocks"
                                        ))
                                        .size(11.0)
                                        .strong(),
                                    ),
                            );
                        }
                        let best = match best_ticks {
                            Some(t) => format!("best {}", crate::trials::format_time(t)),
                            None => "first run!".to_string(),
                        };
                        ui.label(
                            egui::RichText::new(best)
                                .size(12.0)
                                .color(egui::Color32::from_rgb(255, 220, 130)),
                        );
                    });
                });
        });
}

/// Trials — the "Well done!" completion panel for a finished Race. Celebrate the
/// result, then head back to the Trials lobby (the trial's done) or re-run it.
/// No "live here" option — a trial arena is throwaway, not one of your worlds.
pub fn draw_trial_complete(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    name: &str,
    headline: &str,
    is_best: bool,
) -> crate::trials::TrialDoneAction {
    use crate::trials::TrialDoneAction;
    let overlay = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("trial_done_overlay"),
    ))
    .rect_filled(overlay, 0.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 185));

    let panel_w = 380.0;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - 120.0,
    );
    let mut action = TrialDoneAction::None;
    egui::Area::new(egui::Id::new("trial_done"))
        .fixed_pos(origin)
        .order(egui::Order::Middle)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("Well done!")
                        .size(26.0)
                        .strong()
                        .color(egui::Color32::from_rgb(255, 220, 130)),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(name)
                        .size(15.0)
                        .color(egui::Color32::from_rgb(180, 200, 255)),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(headline)
                        .size(22.0)
                        .strong()
                        .color(if is_best {
                            egui::Color32::from_rgb(150, 255, 160)
                        } else {
                            egui::Color32::WHITE
                        }),
                );
                ui.add_space(18.0);
                if ui
                    .add_sized(
                        [280.0, 36.0],
                        egui::Button::new(
                            egui::RichText::new("← Back to Trials").size(16.0).color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(50, 110, 70)),
                    )
                    .clicked()
                {
                    action = TrialDoneAction::BackToTrials;
                }
                ui.add_space(8.0);
                if ui
                    .add_sized(
                        [280.0, 34.0],
                        egui::Button::new(
                            egui::RichText::new("↻ Try again").size(15.0).color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::from_rgb(60, 80, 140)),
                    )
                    .clicked()
                {
                    action = TrialDoneAction::TryAgain;
                }
                ui.add_space(14.0);
            });
        });
    action
}

/// Test Lab — a small top-centre banner keeping the active mission's "what to
/// test" visible after the player closes Satoshi's dialogue. `title` is the
/// current mission, or `None` once they're all done / before the first talk.
pub fn draw_mission_banner(ctx: &egui::Context, viewport: &ViewportRect, title: Option<&str>) {
    let ppp = ctx.pixels_per_point();
    let vp = ViewportPts::from_physical(viewport, ppp);
    let text = match title {
        Some(t) => format!("🧪 Test Lab — try: {t}   (talk to Satoshi to report)"),
        None => "🧪 Test Lab — find Satoshi for a mission".to_string(),
    };
    egui::Area::new(egui::Id::new("mission_banner"))
        .order(egui::Order::Foreground)
        // Task 16 — display-only: don't hit-test (see sign_readout).
        .interactable(false)
        .fixed_pos(egui::pos2(vp.x + vp.width / 2.0 - 230.0, vp.y + 8.0))
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(15, 30, 45, 220))
                .show(ui, |ui| {
                    ui.set_max_width(460.0);
                    ui.label(
                        egui::RichText::new(text)
                            .size(13.0)
                            .color(egui::Color32::from_rgb(150, 205, 255)),
                    );
                });
        });
}

/// The build-guide panel. `inventory` is `Some` in Survival only — it drives
/// the "you're still short of…" call-out; in Creative materials are free, so
/// the caller passes `None` and the shortfall block never renders.
pub fn draw_build_guide_panel(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    guide: &crate::build_guide::BuildGuide,
    world: &crate::world::World,
    registry: &crate::block::BlockRegistry,
    inventory: Option<&crate::inventory::Inventory>,
) {
    let name = guide.name.as_str();
    let cells = guide.cells.as_slice();
    let origin = guide.origin;
    let statuses = crate::build_guide::verify(cells, origin, |x, y, z| world.get_block(x, y, z));
    let (correct, missing, wrong) = crate::build_guide::summarize(&statuses);
    let total = cells.len() as u32;
    let remaining =
        crate::build_guide::remaining_materials(cells, origin, |x, y, z| world.get_block(x, y, z));
    // What this step alone still wants (stepped modes only) + what the player
    // is short of for it. Both empty in `Whole` mode / Creative.
    let step_needs: Vec<(crate::block::BlockId, u32)> = guide
        .steps
        .get(guide.current_step)
        .filter(|_| guide.mode.is_stepped())
        .map(|step| crate::build_steps::step_materials(step, cells, &statuses))
        .unwrap_or_default();
    let short = match inventory {
        Some(inv) => crate::build_steps::shortfall(
            if step_needs.is_empty() { &remaining } else { &step_needs },
            |b| crate::plan::inventory_block_count(inv, b),
        ),
        None => Vec::new(),
    };

    let ppp = ctx.pixels_per_point();
    let vp = ViewportPts::from_physical(viewport, ppp);
    egui::Area::new(egui::Id::new("build_guide_panel"))
        .order(egui::Order::Foreground)
        // Task 16 — display-only: don't hit-test (see sign_readout).
        .interactable(false)
        .fixed_pos(egui::pos2(vp.x + 8.0, vp.y + 84.0))
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style()).show(ui, |ui| {
                ui.set_max_width(220.0);
                ui.label(egui::RichText::new(format!("📐 {name}")).strong());
                ui.label(format!(
                    "Done {correct}/{total}   missing {missing}   wrong {wrong}"
                ));
                // Build-along step indicator. In a stepped mode it counts real
                // steps ("Step 3 of 12"); in the whole-plan mode it falls back
                // to the derived lowest-unfinished-layer walk.
                if guide.mode.is_stepped() {
                    let total_steps = guide.steps.len();
                    if guide.current_step < total_steps {
                        let n = guide.current_step + 1;
                        let what = match guide.mode {
                            crate::build_steps::StepMode::Layers => {
                                "place the glowing layer"
                            }
                            _ => "place the glowing block",
                        };
                        ui.label(
                            egui::RichText::new(format!("Step {n} of {total_steps} — {what}"))
                                .color(egui::Color32::from_rgb(170, 200, 255))
                                .strong(),
                        );
                    }
                    if !step_needs.is_empty() {
                        let line = step_needs
                            .iter()
                            .take(4)
                            .map(|(b, n)| format!("{n}× {}", registry.get(*b).name))
                            .collect::<Vec<_>>()
                            .join(", ");
                        ui.label(
                            egui::RichText::new(format!("This step: {line}"))
                                .color(egui::Color32::from_rgb(200, 215, 240)),
                        );
                    }
                } else if let Some((idx, total_layers)) =
                    crate::build_guide::layer_progress(&statuses)
                {
                    ui.label(
                        egui::RichText::new(format!(
                            "Layer {idx} of {total_layers} — place the glowing blocks"
                        ))
                        .color(egui::Color32::from_rgb(170, 200, 255)),
                    );
                }
                // Survival: what you haven't got yet — the "go and gather this"
                // list. Emphasised above the full material list because it's the
                // bit that sends you back out to mine.
                if !short.is_empty() {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Go and gather:")
                            .color(egui::Color32::from_rgb(255, 190, 110))
                            .strong(),
                    );
                    for (b, n) in short.iter().take(6) {
                        ui.label(
                            egui::RichText::new(format!("  {n}× {}", registry.get(*b).name))
                                .color(egui::Color32::from_rgb(255, 210, 150)),
                        );
                    }
                    if short.len() > 6 {
                        ui.label(
                            egui::RichText::new(format!("  … +{} more", short.len() - 6)).weak(),
                        );
                    }
                }
                if remaining.is_empty() {
                    ui.label(egui::RichText::new("Complete! ✓").color(egui::Color32::LIGHT_GREEN));
                } else {
                    ui.separator();
                    ui.label(egui::RichText::new("Still need:").weak());
                    for (b, n) in remaining.iter().take(10) {
                        ui.label(format!("  {n}× {}", registry.get(*b).name));
                    }
                    if remaining.len() > 10 {
                        ui.label(egui::RichText::new(format!("  … +{} more", remaining.len() - 10)).weak());
                    }
                }
                ui.label(
                    egui::RichText::new(format!(
                        "Guiding {} · /buildguide mode block|layer|whole · /buildguide off to clear",
                        guide.mode.label()
                    ))
                    .small()
                    .weak(),
                );
            });
        });
}

/// Width threshold below which HUD elements (hotbar, hearts, hunger) shrink
/// to keep an ergonomic margin inside the viewport. 720 is the picked floor
/// per spec 14 phase 5: 1080p TV quad (960×540) stays at full size; sub-
/// 1080p windows (e.g. 1280×720 → 640×360 quads) get the scaled HUD.
const HUD_SHRINK_THRESHOLD: u32 = 720;

/// A viewport expressed in egui *points* (physical px ÷ pixels_per_point).
/// `ViewportRect` is in physical pixels because it doubles as the wgpu
/// `set_viewport`/scissor rect, but egui `Area` positions and painter coords
/// are in points. Converting once per HUD element keeps the HUD on-screen at
/// `devicePixelRatio` > 1 (mobile) and on HiDPI native displays; on a standard
/// display (ppp == 1) it is a no-op. See `docs/spec/03-rendering.md` §1.6.
#[derive(Clone, Copy)]
struct ViewportPts {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl ViewportPts {
    fn from_physical(v: &ViewportRect, pixels_per_point: f32) -> Self {
        let ppp = pixels_per_point.max(1.0);
        Self {
            x: v.x as f32 / ppp,
            y: v.y as f32 / ppp,
            width: v.width as f32 / ppp,
            height: v.height as f32 / ppp,
        }
    }
}

/// HUD-element scale factor for a given viewport width *in points*. 1.0 above
/// the shrink threshold, 0.75 below. Width is points (not physical px) so the
/// HUD shrinks on the perceived size, consistently across DPI.
pub(crate) fn hud_scale(viewport_width_pts: f32) -> f32 {
    if viewport_width_pts < HUD_SHRINK_THRESHOLD as f32 { 0.75 } else { 1.0 }
}

/// Hotbar slot geometry in egui *points*, for a viewport of `vp_w`×`vp_h` points
/// anchored at (`vp_x`,`vp_y`). The single source of truth shared by
/// [`draw_hotbar`] (drawing) and `touch_input::classify_zone` (hit-testing), so
/// a tapped slot is always the slot you see — the same guarantee
/// `touch_input::layout_buttons` gives the action buttons.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HotbarGeom {
    /// Left edge of slot 0.
    pub x: f32,
    /// Top edge of the slot row.
    pub y: f32,
    /// Slot side length.
    pub slot: f32,
    /// Gap between slots.
    pub gap: f32,
    /// Total width across all 9 slots + 8 gaps.
    pub total_w: f32,
}

pub(crate) fn hotbar_geom(vp_x: f32, vp_y: f32, vp_w: f32, vp_h: f32) -> HotbarGeom {
    let scale = hud_scale(vp_w);
    let slot = 48.0 * scale;
    let gap = 4.0 * scale;
    let total_w = 9.0 * slot + 8.0 * gap;
    HotbarGeom {
        x: vp_x + vp_w / 2.0 - total_w / 2.0,
        y: vp_y + vp_h - 16.0 - slot,
        slot,
        gap,
        total_w,
    }
}

/// Icon size for the hearts + hunger rows. Bumped 14 → 20 (#12) so the
/// status bars read clearly; scaled by `hud_scale` on narrow viewports.
const HUD_ICON: f32 = 20.0;

/// Vertical anchors for the bottom-centre HUD stack. Before #12 the hearts,
/// hunger, block-name, and armour rows used independent magic offsets from the
/// viewport bottom and overlapped each other AND the hotbar. They now stack
/// strictly upward from the hotbar's top edge with explicit gaps, so adding
/// rows (dynamic 4-12 health/hunger capacity) can't push anything into the
/// hotbar. Mirrors `draw_hotbar`'s `height - 16 - 48*scale` top edge.
struct HudStack {
    hearts_y: f32,
    hunger_y: f32,
    /// W2 — breath bubbles, one row above hunger (drawn only underwater /
    /// refilling, but the row is always reserved so nothing jumps).
    bubbles_y: f32,
    name_y: f32,
}

fn hud_stack(vp: ViewportPts, scale: f32) -> HudStack {
    let bottom = vp.y + vp.height;
    let hotbar_top = bottom - 16.0 - 48.0 * scale;
    let icon = HUD_ICON * scale;
    let gap = 6.0 * scale;
    let hearts_y = hotbar_top - icon - gap;
    let hunger_y = hearts_y - icon - gap;
    let bubbles_y = hunger_y - BUBBLE_ICON * scale - gap;
    let name_y = bubbles_y - 18.0 * scale - gap;
    HudStack { hearts_y, hunger_y, bubbles_y, name_y }
}

/// W2 — breath bubble size (a touch smaller than hearts/hunger).
const BUBBLE_ICON: f32 = 16.0;
const BUBBLE_FULL: egui::Color32 = egui::Color32::from_rgb(110, 190, 255);
const BUBBLE_EMPTY: egui::Color32 = egui::Color32::from_rgba_premultiplied(25, 40, 60, 120);

/// W2 — the 10-bubble breath bar (Spec 05 §1.5.1), one row above hunger.
/// The caller draws it only while underwater or refilling
/// (`survival::show_breath_bar`); `bubbles` is `survival::breath_bubbles`.
pub fn draw_breath(ctx: &egui::Context, viewport: &ViewportRect, player_index: usize, bubbles: u8) {
    let n = crate::survival::BREATH_BUBBLES as usize;
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let scale = hud_scale(vp.width);
    let size = BUBBLE_ICON * scale;
    let gap = 3.0 * scale;
    let total_w = n as f32 * (size + gap) - gap;
    let x = vp.x + vp.width / 2.0 - total_w / 2.0;
    let y = hud_stack(vp, scale).bubbles_y;

    egui::Area::new(egui::Id::new(("breath", player_index)))
        .fixed_pos(egui::pos2(x, y))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);
                for i in 0..n {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
                    let fill = if i < bubbles as usize { BUBBLE_FULL } else { BUBBLE_EMPTY };
                    ui.painter().rect_filled(rect, size / 2.0, fill);
                }
            });
        });
}

/// W2 — the hurt flash: a subtle red vignette at the viewport EDGES (never a
/// full-screen flash). `alpha` is `PlayerCombat::hurt_vignette_alpha` (≤ 0.35,
/// fading over ~0.4 s); zero draws nothing. A band of `EDGE` × the shorter
/// side runs from `alpha` at the edge to transparent at its inner rim.
pub fn draw_hurt_vignette(ctx: &egui::Context, viewport: &ViewportRect, player_index: usize, alpha: f32) {
    const EDGE: f32 = 0.18;
    let alpha = alpha.clamp(0.0, crate::survival::HURT_VIGNETTE_MAX_ALPHA);
    if alpha <= 0.0 {
        return;
    }
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let band = vp.width.min(vp.height) * EDGE;
    if band <= 0.0 {
        return;
    }
    let edge = egui::Color32::from_rgba_unmultiplied(200, 20, 20, (alpha * 255.0).round() as u8);
    let clear = egui::Color32::TRANSPARENT;
    let (x0, y0) = (vp.x, vp.y);
    let (x1, y1) = (vp.x + vp.width, vp.y + vp.height);
    let outer = [
        egui::pos2(x0, y0),
        egui::pos2(x1, y0),
        egui::pos2(x1, y1),
        egui::pos2(x0, y1),
    ];
    let inner = [
        egui::pos2(x0 + band, y0 + band),
        egui::pos2(x1 - band, y0 + band),
        egui::pos2(x1 - band, y1 - band),
        egui::pos2(x0 + band, y1 - band),
    ];
    let mut mesh = egui::epaint::Mesh::default();
    for p in outer {
        mesh.colored_vertex(p, edge);
    }
    for p in inner {
        mesh.colored_vertex(p, clear);
    }
    // Four trapezoids (two triangles each) between the outer and inner rims.
    for k in 0..4u32 {
        let n = (k + 1) % 4;
        mesh.add_triangle(k, n, 4 + k);
        mesh.add_triangle(n, 4 + n, 4 + k);
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("hurt_vignette", player_index)),
    ));
    painter.add(egui::Shape::mesh(mesh));
}

/// Rolling-window performance samples surfaced in the F3 debug overlay.
/// Only populated on WASM where the engine maintains `frame_samples` and
/// `tick_samples` rings — perf-budget checks on PWA quad split-screen
/// (spec 15 phase 6) read from here. `None` on platforms that don't
/// track the samples.
#[derive(Default, Clone, Copy)]
pub struct PerfSamples {
    /// Mean frame time (ms) over the rolling window. `None` = no samples
    /// or platform doesn't expose them.
    pub frame_ms_mean: Option<f32>,
    /// Mean tick time (ms) over the rolling window.
    pub tick_ms_mean: Option<f32>,
    /// Worst-case frame time (ms) in the window — useful for spike
    /// hunting under chunk streaming.
    pub frame_ms_worst: Option<f32>,
    /// Chunk/water/plant draw calls submitted last frame (Spec 39 A1). `None`
    /// when culling stats aren't available. Drops sharply when facing a wall.
    pub draw_calls: Option<u32>,
    /// Meshes skipped by frustum culling last frame (Spec 39 A1).
    pub culled: Option<u32>,
}

/// Extra read-outs the #44 debug overlay surfaces, bundled so `draw_hud`'s
/// signature gains one parameter rather than four. All cheap to compute at the
/// call site (yaw is a camera field; biome + light are single lookups).
#[derive(Clone, Copy)]
pub struct DebugReadout {
    /// Camera yaw in radians (for the compass facing line).
    pub yaw: f32,
    /// Biome label at the player's column (`Biome::name()`).
    pub biome: &'static str,
    /// Effective light level (0–15) at the targeted block, if any.
    pub light: Option<u8>,
    /// Wind, Copper & Electricity wave §2.3 — the breeze at the player's own
    /// height (word + speed + compass point). What tells a builder whether the
    /// spot they are standing on is worth a Windmill.
    pub wind: crate::wind::WindSample,
}

/// Draw the full in-game HUD.
#[allow(clippy::too_many_arguments)]
pub fn draw_hud(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    inventory: &Inventory,
    selected_slot: usize,
    registry: &BlockRegistry,
    egui_integration: &EguiIntegration,
    current_hp: f32,
    max_hp: f32,
    hunger: u8,
    max_hunger: u8,
    armour_points: u8,
    is_creative: bool,
    show_debug: bool,
    player_pos: glam::Vec3,
    world_time: u32,
    flying: bool,
    target_block: Option<[i32; 3]>,
    perf: PerfSamples,
    debug: DebugReadout,
) {
    draw_hotbar(ctx, viewport, player_index, inventory, selected_slot, registry, egui_integration);
    // Survival-only vitals. Creative takes no damage and never gets hungry,
    // so the hearts + hunger bars were just clutter there (2026-05-30
    // playtest: "in creative there is still hearts and hunger").
    if !is_creative {
        draw_hearts(ctx, viewport, player_index, current_hp, max_hp);
        draw_hunger(ctx, viewport, player_index, hunger, max_hunger);
        // #44 P2 — free-slot + arrow counts. Survival-only so Creative stays
        // uncluttered (mirrors the hearts/hunger suppression above). Saturation
        // outline is deferred — the food model tracks no saturation value yet.
        draw_vital_counts(
            ctx,
            viewport,
            player_index,
            inventory.free_slot_count(),
            inventory.arrow_count(),
        );
    }
    draw_armour_readout(ctx, viewport, player_index, armour_points);
    draw_block_name(ctx, viewport, player_index, inventory, selected_slot, registry);

    if show_debug {
        draw_debug_overlay(ctx, viewport, player_index, player_pos, world_time, flying, target_block, perf, debug);
    }
}

/// Spec 28e — small numeric badge near the hearts showing total
/// equipped armour points. Suppressed entirely when the player wears
/// nothing, so the HUD stays clean for unarmoured play.
fn draw_armour_readout(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    points: u8,
) {
    if points == 0 {
        return;
    }
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let scale = hud_scale(vp.width);
    // Anchor on the hearts row (shared stack, #12), to the right of the hearts
    // so it doesn't overlap them.
    let y = hud_stack(vp, scale).hearts_y;
    let x = vp.x + vp.width / 2.0 + 120.0 * scale;
    let origin = egui::pos2(x, y);
    egui::Area::new(egui::Id::new(("armour_readout", player_index)))
        .fixed_pos(origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(format!("🛡 {points}"))
                    .size(14.0 * scale)
                    .color(egui::Color32::from_rgb(180, 180, 200))
                    .strong(),
            );
        });
}

/// Spec 28d.nostrich v2 — small purple-feather badge in the top-
/// left corner showing the player's active Nostrich's Vow countdown.
/// Only renders when `vow.is_some()`; pure read, no side effects.
pub fn draw_nostrich_vow_badge(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    vow: Option<&crate::nostrich_vow::NostrichVow>,
) {
    let Some(v) = vow else { return };
    let origin = egui::pos2(
        viewport.x as f32 + 16.0,
        viewport.y as f32 + 16.0,
    );
    let mins = v.minutes_remaining();
    let secs_total = v.ticks_remaining / 20;
    let secs = secs_total % 60;
    let label = if mins > 0 {
        format!("🪶  Vow: {mins}m {secs}s")
    } else {
        format!("🪶  Vow: {secs}s")
    };
    egui::Area::new(egui::Id::new(("nostrich_vow_badge", player_index)))
        .fixed_pos(origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(50, 25, 70, 220))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(label)
                            .size(14.0)
                            .color(egui::Color32::from_rgb(220, 170, 255))
                            .strong(),
                    );
                });
        });
}

/// Draw the 9-slot hotbar at the bottom centre of the viewport.
fn draw_hotbar(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    inventory: &Inventory,
    selected: usize,
    registry: &BlockRegistry,
    egui_integration: &EguiIntegration,
) {
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    // Shared with `touch_input::classify_zone` so the tappable slot rects line
    // up exactly with what's drawn here.
    let g = hotbar_geom(vp.x, vp.y, vp.width, vp.height);
    let slot_size = g.slot;
    let gap = g.gap;
    let hotbar_x = g.x;
    let hotbar_y = g.y;

    egui::Area::new(egui::Id::new(("hotbar", player_index)))
        .fixed_pos(egui::pos2(hotbar_x, hotbar_y))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);
                for i in 0..9 {
                    let is_selected = i == selected;
                    let (bg, border) = if is_selected {
                        (SLOT_SELECTED_BG, SLOT_SELECTED_BORDER)
                    } else {
                        (SLOT_BG, SLOT_BORDER)
                    };

                    let (rect, _response) = ui.allocate_exact_size(
                        egui::vec2(slot_size, slot_size),
                        egui::Sense::hover(),
                    );

                    // Slot background
                    ui.painter().rect_filled(rect, 3.0, bg);
                    ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(if is_selected { 2.0_f32 } else { 1.0_f32 }, border), egui::StrokeKind::Inside);

                    // Item icon
                    if let Some(stack) = inventory.hotbar_slot(i) {
                        let icon_rect = rect.shrink(4.0);

                        match &stack.item {
                            Item::Block(id) => {
                                let tex_layer = registry.tex_top(*id);
                                if let Some(tex_id) = egui_integration.block_texture(tex_layer) {
                                    ui.painter().image(
                                        tex_id,
                                        icon_rect,
                                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                        egui::Color32::WHITE,
                                    );
                                }
                            }
                            Item::Tool(tool) => {
                                // Coloured rectangle for tools
                                let color = stack.item.color(registry);
                                let tool_color = egui::Color32::from_rgb(
                                    (color[0] * 255.0) as u8,
                                    (color[1] * 255.0) as u8,
                                    (color[2] * 255.0) as u8,
                                );
                                ui.painter().rect_filled(icon_rect, 2.0, tool_color);

                                // Tool type indicator
                                let indicator = match tool.tool_type {
                                    crate::crafting::ToolType::Sword => "S",
                                    crate::crafting::ToolType::Pickaxe => "P",
                                    crate::crafting::ToolType::Axe => "A",
                                    crate::crafting::ToolType::Shovel => "V",
                                    crate::crafting::ToolType::Bow => "B",
                                    crate::crafting::ToolType::Hoe => "H",
                                    crate::crafting::ToolType::FlintAndSteel => "F",
                                    crate::crafting::ToolType::Shears => "Sh",
                                    crate::crafting::ToolType::FishingRod => "Fr",
                                    crate::crafting::ToolType::Slingshot => "Sl",
                                    crate::crafting::ToolType::Eraser => "Er",
                                    crate::crafting::ToolType::DraftingStamp => "Ds",
                                };
                                ui.painter().text(
                                    icon_rect.left_top() + egui::vec2(2.0, 2.0),
                                    egui::Align2::LEFT_TOP,
                                    indicator,
                                    egui::FontId::proportional(12.0),
                                    egui::Color32::WHITE,
                                );
                            }
                            Item::Material(_) => {
                                let color = stack.item.color(registry);
                                let mat_color = egui::Color32::from_rgb(
                                    (color[0] * 255.0) as u8,
                                    (color[1] * 255.0) as u8,
                                    (color[2] * 255.0) as u8,
                                );
                                ui.painter().rect_filled(icon_rect, 2.0, mat_color);
                            }
                            Item::Plan(_) => {
                                // Spec 24 — Plan item. Dark-gold parchment
                                // swatch + "P" label so the hotbar entry
                                // reads as "I have a plan ready to place".
                                let color = stack.item.color(registry);
                                let plan_color = egui::Color32::from_rgb(
                                    (color[0] * 255.0) as u8,
                                    (color[1] * 255.0) as u8,
                                    (color[2] * 255.0) as u8,
                                );
                                ui.painter().rect_filled(icon_rect, 2.0, plan_color);
                                ui.painter().text(
                                    icon_rect.left_top() + egui::vec2(2.0, 2.0),
                                    egui::Align2::LEFT_TOP,
                                    "P",
                                    egui::FontId::proportional(12.0),
                                    egui::Color32::WHITE,
                                );
                            }
                            Item::Armour(_) => {
                                // Spec 28e — armour piece. Tier-tinted swatch
                                // with a slot-shorthand label until per-slot
                                // icons land.
                                let color = stack.item.color(registry);
                                let arm_color = egui::Color32::from_rgb(
                                    (color[0] * 255.0) as u8,
                                    (color[1] * 255.0) as u8,
                                    (color[2] * 255.0) as u8,
                                );
                                ui.painter().rect_filled(icon_rect, 2.0, arm_color);
                                ui.painter().text(
                                    icon_rect.left_top() + egui::vec2(2.0, 2.0),
                                    egui::Align2::LEFT_TOP,
                                    "A",
                                    egui::FontId::proportional(12.0),
                                    egui::Color32::WHITE,
                                );
                            }
                        }

                        // Stack count
                        if stack.count > 1 {
                            let text = format!("{}", stack.count);
                            // Shadow
                            ui.painter().text(
                                rect.right_bottom() + egui::vec2(-3.0, -3.0),
                                egui::Align2::RIGHT_BOTTOM,
                                &text,
                                egui::FontId::proportional(14.0),
                                egui::Color32::from_rgb(20, 20, 20),
                            );
                            // Text
                            ui.painter().text(
                                rect.right_bottom() + egui::vec2(-4.0, -4.0),
                                egui::Align2::RIGHT_BOTTOM,
                                &text,
                                egui::FontId::proportional(14.0),
                                egui::Color32::WHITE,
                            );
                        }

                        // Durability bar for tools
                        if let Item::Tool(tool) = &stack.item {
                            let max_dur = crate::crafting::Tool::new(tool.tool_type, tool.material).durability;
                            if tool.durability < max_dur {
                                let pct = tool.durability as f32 / max_dur as f32;
                                let bar_h = 3.0;
                                let bar_rect = egui::Rect::from_min_size(
                                    egui::pos2(rect.left() + 2.0, rect.bottom() - bar_h - 2.0),
                                    egui::vec2(rect.width() - 4.0, bar_h),
                                );
                                ui.painter().rect_filled(bar_rect, 1.0, egui::Color32::from_rgb(200, 50, 50));
                                let fill_rect = egui::Rect::from_min_size(
                                    bar_rect.left_top(),
                                    egui::vec2(bar_rect.width() * pct, bar_h),
                                );
                                ui.painter().rect_filled(fill_rect, 1.0, egui::Color32::from_rgb(50, 200, 50));
                            }
                        }
                    }
                }
            });
        });
}

/// Draw health hearts above the hotbar.
fn draw_hearts(ctx: &egui::Context, viewport: &ViewportRect, player_index: usize, current_hp: f32, max_hp: f32) {
    let num_hearts = (max_hp / 2.0).ceil() as usize;
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let scale = hud_scale(vp.width);
    let heart_size = HUD_ICON * scale;
    let gap = 3.0 * scale;
    let total_w = num_hearts as f32 * (heart_size + gap) - gap;

    let hearts_x = vp.x + vp.width / 2.0 - total_w / 2.0;
    let hearts_y = hud_stack(vp, scale).hearts_y;

    egui::Area::new(egui::Id::new(("hearts", player_index)))
        .fixed_pos(egui::pos2(hearts_x, hearts_y))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);
                for i in 0..num_hearts {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(heart_size, heart_size),
                        egui::Sense::hover(),
                    );

                    let hp_for_heart = (current_hp - i as f32 * 2.0).clamp(0.0, 2.0);

                    // Background (empty heart)
                    ui.painter().rect_filled(rect, 2.0, HEART_EMPTY);

                    if hp_for_heart >= 2.0 {
                        ui.painter().rect_filled(rect, 2.0, HEART_FULL);
                    } else if hp_for_heart >= 1.0 {
                        // Half heart
                        let half_rect = egui::Rect::from_min_size(
                            rect.left_top(),
                            egui::vec2(rect.width() * 0.5, rect.height()),
                        );
                        ui.painter().rect_filled(half_rect, 2.0, HEART_HALF);
                    }
                }
            });
        });
}

/// Draw hunger drumsticks just above the hearts (Wave 24).
fn draw_hunger(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    hunger: u8,
    max_hunger: u8,
) {
    let num_icons = (max_hunger as f32 / 2.0).ceil() as usize;
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let scale = hud_scale(vp.width);
    let icon_size = HUD_ICON * scale;
    let gap = 3.0 * scale;
    let total_w = num_icons as f32 * (icon_size + gap) - gap;

    let icons_x = vp.x + vp.width / 2.0 - total_w / 2.0;
    // One row above the hearts (shared stack so it never overlaps, #12).
    let icons_y = hud_stack(vp, scale).hunger_y;

    egui::Area::new(egui::Id::new(("hunger", player_index)))
        .fixed_pos(egui::pos2(icons_x, icons_y))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);
                for i in 0..num_icons {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(icon_size, icon_size),
                        egui::Sense::hover(),
                    );

                    let h_for_icon = (hunger as i32 - i as i32 * 2).clamp(0, 2);

                    // Background (empty drumstick).
                    ui.painter().rect_filled(rect, 2.0, DRUMSTICK_EMPTY);

                    if h_for_icon >= 2 {
                        ui.painter().rect_filled(rect, 2.0, DRUMSTICK_FULL);
                    } else if h_for_icon >= 1 {
                        // Half drumstick — fill the left half.
                        let half_rect = egui::Rect::from_min_size(
                            rect.left_top(),
                            egui::vec2(rect.width() * 0.5, rect.height()),
                        );
                        ui.painter().rect_filled(half_rect, 2.0, DRUMSTICK_HALF);
                    }
                }
            });
        });
}

/// #44 P2 — compact free-slot + arrow counts, anchored bottom-right just above
/// the hotbar line. Free slots are always shown (handy while mining); the arrow
/// line only appears when the player actually carries arrows, so it stays out of
/// the way otherwise.
fn draw_vital_counts(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    free_slots: usize,
    arrows: u32,
) {
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let scale = hud_scale(vp.width);
    let font = egui::FontId::proportional(13.0 * scale);
    // Sit just above the hotbar's top edge, hugging the right margin.
    let bottom = vp.y + vp.height;
    let anchor = egui::pos2(vp.x + vp.width - 12.0, bottom - 16.0 - 48.0 * scale - 6.0);

    egui::Area::new(egui::Id::new(("vital_counts", player_index)))
        .fixed_pos(anchor)
        .interactable(false)
        .show(ctx, |ui| {
            // Right-aligned stack: arrows above free-slots when present.
            if arrows > 0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    ui.label(
                        egui::RichText::new(format!("\u{27A4} {arrows}"))
                            .font(font.clone())
                            .color(egui::Color32::from_rgb(230, 230, 235)),
                    );
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                // Warm the colour as the inventory fills, so "nearly full" reads
                // at a glance (green → amber → red).
                let colour = if free_slots == 0 {
                    egui::Color32::from_rgb(220, 70, 70)
                } else if free_slots <= 4 {
                    egui::Color32::from_rgb(230, 180, 70)
                } else {
                    egui::Color32::from_rgb(200, 200, 205)
                };
                ui.label(
                    egui::RichText::new(format!("\u{2756} {free_slots} free"))
                        .font(font.clone())
                        .color(colour),
                );
            });
        });
}

/// Draw the name of the held item above the hotbar.
fn draw_block_name(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    inventory: &Inventory,
    selected_slot: usize,
    registry: &BlockRegistry,
) {
    let name = inventory.hotbar_item_name(selected_slot, registry);
    if name.is_empty() {
        return;
    }

    // Approximate text width; centre horizontally within the viewport.
    let approx_text_w = name.len() as f32 * 8.0;
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let name_x = vp.x + vp.width / 2.0 - approx_text_w / 2.0;
    // Above the hunger row in the shared stack (#12).
    let name_y = hud_stack(vp, hud_scale(vp.width)).name_y;

    egui::Area::new(egui::Id::new(("block_name", player_index)))
        .fixed_pos(egui::pos2(name_x, name_y))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(name)
                        .color(egui::Color32::from_rgba_premultiplied(220, 220, 220, 200))
                        .size(14.0),
                );
            });
        });
}

/// Workshop edit controls hint — shown centred above the block-name row while at
/// least one locked ×4 balloon exists in the room. Discoverability for an
/// young player: one persistent line listing the three most-used edit keys.
pub fn draw_workshop_edit_hint(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    mode: crate::workshop::EditMode,
) {
    // §6 (2026-06-18) — lead with the current mode so the kid always knows
    // whether a click paints or carves, and how to switch.
    let hint = format!(
        "{} mode   ·   V switch   ·   P Pin   ·   M Mirror   ·   G Pick colour",
        mode.label()
    );
    let approx_text_w = hint.len() as f32 * 7.5;
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let name_y = hud_stack(vp, hud_scale(vp.width)).name_y;
    // Place the hint ~22pts above the block-name row so it doesn't overlap.
    let hint_y = name_y - 22.0;
    let hint_x = vp.x + vp.width / 2.0 - approx_text_w / 2.0;

    egui::Area::new(egui::Id::new(("workshop_edit_hint", player_index)))
        .fixed_pos(egui::pos2(hint_x, hint_y))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&hint)
                        .color(egui::Color32::from_rgba_premultiplied(200, 200, 210, 180))
                        .size(13.0),
                );
            });
        });
}

/// Skin-paint edit hint — shown instead of the block hint while the locked
/// Workshop target is the player's avatar (a `skin_paint` session is open). The
/// avatar painter uses a different verb set than the block painter (no Sculpt;
/// dye paints, empty hand erases, V swaps Base/Outer, Z undoes), so the kid needs
/// the right reminder. Same placement / styling as `draw_workshop_edit_hint`.
pub fn draw_workshop_avatar_edit_hint(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
) {
    // One short line, not nine keys: everything else lives in the Tab panel,
    // where each control sits beside its own letter shortcut.
    let hint =
        "Left-click paints   ·   B tool   ·   F fill   ·   Tab = colours & options   ·   P pin";
    let approx_text_w = hint.len() as f32 * 7.5;
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let name_y = hud_stack(vp, hud_scale(vp.width)).name_y;
    let hint_y = name_y - 22.0;
    let hint_x = vp.x + vp.width / 2.0 - approx_text_w / 2.0;

    egui::Area::new(egui::Id::new(("workshop_avatar_edit_hint", player_index)))
        .fixed_pos(egui::pos2(hint_x, hint_y))
        .interactable(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(hint)
                        .color(egui::Color32::from_rgba_premultiplied(200, 200, 210, 180))
                        .size(13.0),
                );
            });
        });
}

/// What the paint panel wants the caller to do this frame.
#[derive(Default, Clone, Copy)]
pub struct SkinPanelResult {
    pub close: bool,
    pub pin: bool,
    pub undo: bool,
    pub redo: bool,
}

/// The skin paint panel (Tab). Carries the full colour wheel, a `#rrggbb` box,
/// all 16 dyes, the recently-used colours, the six tools and every option — so
/// nothing has to be memorised and no dye needs to be in hand (the Workshop
/// only gives you a Bellows). Every control shows its key. Mutates the session
/// directly; returns the actions the caller owns.
pub fn draw_skin_paint_panel(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    session: &mut crate::SkinPaintSession,
    registry: &crate::block::BlockRegistry,
) -> SkinPanelResult {
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let mut out = SkinPanelResult::default();
    let dim = egui::Color32::from_gray(150);
    egui::Area::new(egui::Id::new("skin_paint_panel"))
        .fixed_pos(egui::pos2(vp.x + vp.width - 282.0, vp.y + vp.height * 0.12))
        .order(egui::Order::Middle)
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(252.0);
                ui.label(egui::RichText::new("Paint").size(16.0).strong());
                ui.add_space(4.0);

                // ── Colour wheel + hex box ──────────────────────────────────
                ui.horizontal(|ui| {
                    let mut c = session
                        .picked_color
                        .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]))
                        .unwrap_or(egui::Color32::WHITE);
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut c,
                        egui::color_picker::Alpha::Opaque,
                    )
                    .changed()
                    {
                        session.set_color([c.r(), c.g(), c.b(), 255]);
                    }
                    // Type a colour straight in — the notation every skin
                    // tutorial on the internet uses. Committed only when it
                    // parses, so a half-typed "#ff88" is left to be finished
                    // rather than argued with.
                    let hex = ui.add(
                        egui::TextEdit::singleline(&mut session.hex_input)
                            .hint_text("#rrggbb")
                            .desired_width(78.0)
                            .char_limit(7),
                    );
                    if hex.changed()
                        && let Some(rgb) = crate::skin_paint::parse_hex_rgb(&session.hex_input)
                    {
                        session.set_color_keep_hex([rgb[0], rgb[1], rgb[2], 255]);
                    }
                });
                ui.add_space(4.0);

                // ── The 16 dye swatches, straight from the dye registry ─────
                let sw = egui::vec2(24.0, 24.0);
                for row in 0..2 {
                    ui.horizontal(|ui| {
                        for col in 0..8 {
                            let dye = crate::item::DYES[row * 8 + col];
                            let Some(c) = crate::item::dye_skin_color(dye, registry) else {
                                continue;
                            };
                            let col32 = egui::Color32::from_rgb(c[0], c[1], c[2]);
                            let (rect, resp) = ui.allocate_exact_size(sw, egui::Sense::click());
                            ui.painter().rect_filled(rect, 3.0, col32);
                            let selected = session.tool
                                != crate::skin_paint::PaintTool::Eraser
                                && session.picked_color == Some(c);
                            ui.painter().rect_stroke(
                                rect,
                                3.0,
                                egui::Stroke::new(
                                    if selected { 2.5_f32 } else { 1.0_f32 },
                                    if selected {
                                        egui::Color32::WHITE
                                    } else {
                                        egui::Color32::from_gray(70)
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                            if resp.clicked() {
                                session.set_color(c);
                            }
                        }
                    });
                }
                ui.add_space(4.0);

                // ── Recent colours ──────────────────────────────────────────
                // Everything just used, wheel/hex/dye/eyedropper alike, so a
                // shade mixed on the wheel can be got back without re-mixing it.
                if !session.recent.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Recent").size(12.0));
                        let mut pick: Option<[u8; 4]> = None;
                        for c in session.recent.clone() {
                            let (rect, resp) =
                                ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                            ui.painter().rect_filled(
                                rect,
                                3.0,
                                egui::Color32::from_rgb(c[0], c[1], c[2]),
                            );
                            ui.painter().rect_stroke(
                                rect,
                                3.0,
                                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(70)),
                                egui::StrokeKind::Inside,
                            );
                            if resp.clicked() {
                                pick = Some(c);
                            }
                        }
                        if let Some(c) = pick {
                            session.set_color(c);
                        }
                    });
                    ui.add_space(4.0);
                }

                // ── Tools ───────────────────────────────────────────────────
                // One row of "what the click does", so no two can be on at
                // once and the eraser stops being a mode hiding beside a brush.
                {
                    use crate::skin_paint::PaintTool;
                    let mut tool_button = |ui: &mut egui::Ui, t: PaintTool, label: &str| {
                        if ui.selectable_label(session.tool == t, label).clicked() {
                            session.tool = t;
                        }
                    };
                    ui.horizontal(|ui| {
                        ui.label("Tool");
                        tool_button(ui, PaintTool::Brush, "Brush  B");
                        tool_button(ui, PaintTool::Fill, "Fill  F");
                        tool_button(ui, PaintTool::Eraser, "Eraser");
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(30.0);
                        tool_button(ui, PaintTool::Lighten, "Lighten");
                        tool_button(ui, PaintTool::Darken, "Darken");
                        tool_button(ui, PaintTool::Noise, "Noise");
                    });
                }
                ui.label(
                    egui::RichText::new(match session.tool {
                        crate::skin_paint::PaintTool::Fill => {
                            "Floods the whole side you aim at."
                        }
                        crate::skin_paint::PaintTool::Eraser => "Rubs it out.",
                        crate::skin_paint::PaintTool::Lighten => "Makes it paler, a step a click.",
                        crate::skin_paint::PaintTool::Darken => "Deepens the shade, a step a click.",
                        crate::skin_paint::PaintTool::Noise => "Speckles the shade for texture.",
                        crate::skin_paint::PaintTool::Brush => "B cycles the shading brushes.",
                    })
                    .size(11.0)
                    .color(dim),
                );
                if session.tool.is_shading() {
                    // The once-per-texel-per-stroke rule, said out loud: without
                    // it a kid holding the button on one spot expects it to keep
                    // going and reads "stopped working" instead of "one step".
                    ui.label(
                        egui::RichText::new("One step per click — sweeping won't pile it up.")
                            .size(11.0)
                            .color(dim),
                    );
                }
                ui.separator();

                // ── Options, each showing its shortcut ──────────────────────
                ui.horizontal(|ui| {
                    ui.label("Clothes");
                    if ui.selectable_label(session.clothes_on, "on").clicked() {
                        session.clothes_on = true;
                    }
                    if ui.selectable_label(!session.clothes_on, "off").clicked() {
                        session.clothes_on = false;
                    }
                    ui.label(egui::RichText::new("V").weak());
                });
                ui.label(
                    egui::RichText::new(if session.clothes_on {
                        "The second layer, like Minecraft's overlay."
                    } else {
                        "Painting the body."
                    })
                    .size(11.0)
                    .color(dim),
                );
                ui.horizontal(|ui| {
                    ui.label("Mirror");
                    if ui.selectable_label(!session.mirror, "off").clicked() {
                        session.mirror = false;
                    }
                    if ui.selectable_label(session.mirror, "on").clicked() {
                        session.mirror = true;
                    }
                    ui.label(egui::RichText::new("M").weak());
                });
                ui.horizontal(|ui| {
                    ui.label("Size");
                    for size in 1u32..=3 {
                        if ui
                            .selectable_label(session.brush == size, format!("{size}"))
                            .clicked()
                        {
                            session.brush = size;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Limbs");
                    if ui.selectable_label(!session.limbs_apart, "together").clicked() {
                        session.limbs_apart = false;
                    }
                    if ui.selectable_label(session.limbs_apart, "apart").clicked() {
                        session.limbs_apart = true;
                    }
                    ui.label(egui::RichText::new("R").weak());
                });
                if !session.limbs_apart {
                    ui.label(
                        egui::RichText::new("Move them apart to paint the insides.")
                            .size(11.0)
                            .color(dim),
                    );
                }
                // Arms — Minecraft's two player models. Changing it reshapes
                // the mannequin (and therefore the hit-test, the grid and the
                // hover box) on the spot; Pin writes it to the wardrobe entry.
                ui.horizontal(|ui| {
                    ui.label("Arms");
                    let classic = crate::skin_uv::ArmModel::Classic;
                    let slim = crate::skin_uv::ArmModel::Slim;
                    if ui.selectable_label(session.arm_model == classic, "classic").clicked()
                        && session.arm_model != classic
                    {
                        session.arm_model = classic;
                        // The grid memo is keyed on the arm model, but drop it
                        // anyway so the very next frame is already correct.
                        session.grid.invalidate();
                    }
                    if ui.selectable_label(session.arm_model == slim, "slim").clicked()
                        && session.arm_model != slim
                    {
                        session.arm_model = slim;
                        session.grid.invalidate();
                    }
                });
                ui.label(
                    egui::RichText::new(if session.arm_model.is_slim() {
                        "Slim arm style: 3-pixel arms."
                    } else {
                        "Classic arm style: 4-pixel arms."
                    })
                    .size(11.0)
                    .color(dim),
                );
                // Precision aids (2026-09-06) — the texel grid + the yellow
                // "where your click lands" box travel together on one toggle.
                ui.horizontal(|ui| {
                    ui.label("Grid");
                    if ui.selectable_label(session.grid.on, "on").clicked() {
                        session.grid.on = true;
                    }
                    if ui.selectable_label(!session.grid.on, "off").clicked() {
                        session.grid.on = false;
                        session.grid.invalidate();
                    }
                    ui.label(egui::RichText::new("L").weak());
                });
                ui.separator();
                ui.label(
                    egui::RichText::new("Shift+click = straight line   ·   C = zoom")
                        .size(11.0)
                        .color(dim),
                );
                ui.horizontal(|ui| {
                    if ui.button("Undo  Z").clicked() {
                        out.undo = true;
                    }
                    if ui.button("Redo  X").clicked() {
                        out.redo = true;
                    }
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(egui::RichText::new("Pin — wear it   P").strong())
                        .clicked()
                    {
                        out.pin = true;
                    }
                    if ui.button("Close  Tab").clicked() {
                        out.close = true;
                    }
                });
            });
        });
    out
}

/// Draw a small reputation panel when the player is standing within range of
/// a village anchor. Shows the village id + tier name + score. Bottom-left
/// of the viewport, above the hotbar.
///
/// Per `docs/vision/sat-flow-and-economy-loops.md` §3.2: reputation is
/// load-bearing across quest + vendor + raid. Making it visible at all
/// times when the kid is in a village removes the "wait, what was my
/// rep here?" friction.
pub fn draw_village_reputation(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    player_pos: glam::Vec3,
    village_anchors: &ahash::AHashMap<(i32, i32), [i32; 3]>,
    reputation: &crate::reputation::Reputation,
) {
    /// Show the panel when within this many blocks of a village anchor.
    const RANGE: f32 = 40.0;
    // Find the nearest anchor within RANGE.
    let mut best: Option<((i32, i32), f32)> = None;
    for (&vid, &anchor) in village_anchors {
        let av = glam::Vec3::new(anchor[0] as f32, anchor[1] as f32, anchor[2] as f32);
        let d = (av - player_pos).length();
        if d <= RANGE && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((vid, d));
        }
    }
    let Some((vid, _)) = best else { return; };
    let score = reputation.score(vid);
    let tier = reputation.tier(vid);
    let (label, tier_color) = match tier {
        crate::reputation::Tier::Hostile => ("Hostile", egui::Color32::from_rgb(200, 80, 80)),
        crate::reputation::Tier::Wary => ("Wary", egui::Color32::from_rgb(200, 160, 80)),
        crate::reputation::Tier::Neutral => ("Neutral", egui::Color32::from_rgb(180, 180, 200)),
        crate::reputation::Tier::Friendly => ("Friendly", egui::Color32::from_rgb(150, 200, 120)),
        crate::reputation::Tier::Beloved => ("Beloved", egui::Color32::from_rgb(220, 200, 120)),
    };
    let vp = ViewportPts::from_physical(viewport, ctx.pixels_per_point());
    let scale = hud_scale(vp.width);
    let panel_x = vp.x + 12.0 * scale;
    let panel_y = vp.y + vp.height - 132.0 * scale;
    let panel_w = 180.0 * scale;
    let panel_h = 42.0 * scale;
    egui::Area::new(egui::Id::new(("village_rep", player_index)))
        .fixed_pos(egui::pos2(panel_x, panel_y))
        .interactable(false)
        .show(ctx, |ui| {
            let rect = egui::Rect::from_min_size(
                egui::pos2(panel_x, panel_y),
                egui::vec2(panel_w, panel_h),
            );
            ui.painter().rect_filled(
                rect,
                4.0,
                egui::Color32::from_rgba_premultiplied(20, 20, 28, 180),
            );
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0_f32, SLOT_BORDER),
                egui::StrokeKind::Inside,
            );
            // Two-line layout: "Village (gx, gz)" + "Tier name (score)"
            ui.painter().text(
                rect.left_top() + egui::vec2(8.0, 4.0),
                egui::Align2::LEFT_TOP,
                format!("Village ({}, {})", vid.0, vid.1),
                egui::FontId::proportional(11.0 * scale),
                egui::Color32::from_rgb(170, 170, 200),
            );
            let sign = if score > 0 { "+" } else { "" };
            ui.painter().text(
                rect.left_top() + egui::vec2(8.0, 20.0),
                egui::Align2::LEFT_TOP,
                format!("{label}  ({sign}{score})"),
                egui::FontId::proportional(14.0 * scale),
                tier_color,
            );
        });
}

/// Map a camera yaw (radians) to an 8-point compass label.
///
/// Convention (matches `camera.rs::forward()`): yaw `0` looks along `-Z` and
/// increasing yaw rotates counter-clockwise (yaw `π/2` → `-X`). Using the
/// Minecraft axis labelling `-Z = N`, `-X = W`, `+Z = S`, `+X = E`, the eight
/// 45°-wide sectors centred on each direction are, in increasing-yaw order:
/// N, NW, W, SW, S, SE, E, NE.
pub fn yaw_to_compass(yaw: f32) -> &'static str {
    const DIRS: [&str; 8] = ["N", "NW", "W", "SW", "S", "SE", "E", "NE"];
    let sector = (yaw / std::f32::consts::FRAC_PI_4).round() as i32;
    DIRS[sector.rem_euclid(8) as usize]
}

/// Standard compass bearing in degrees (N=0, E=90, S=180, W=270) from a camera
/// yaw. The camera's yaw increases counter-clockwise (N→W→S→E), so the bearing
/// is the clockwise complement of the yaw measured from North.
pub fn yaw_to_bearing(yaw: f32) -> u32 {
    let deg = yaw.to_degrees();
    ((360.0 - deg).rem_euclid(360.0)).round() as u32 % 360
}

/// Frames per second derived from a mean frame time in milliseconds. Returns
/// `0` for a non-positive frame time (no samples yet) so the readout never
/// divides by zero or shows nonsense.
pub fn fps_from_frame_ms(frame_ms: f32) -> u32 {
    if frame_ms <= 0.0 {
        return 0;
    }
    (1000.0 / frame_ms).round() as u32
}

/// Draw F3 debug overlay.
#[allow(clippy::too_many_arguments)]
fn draw_debug_overlay(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    player_pos: glam::Vec3,
    world_time: u32,
    flying: bool,
    target_block: Option<[i32; 3]>,
    perf: PerfSamples,
    debug: DebugReadout,
) {
    let mode = if flying { "FLY" } else { "SURV" };
    let hours = (world_time % 24000) * 24 / 24000;
    let mins = ((world_time % 24000) * 24 * 60 / 24000) % 60;

    egui::Area::new(egui::Id::new(("debug", player_index)))
        .fixed_pos(egui::pos2(viewport.x as f32 + 8.0, viewport.y as f32 + 8.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(0, 0, 0, 160))
                .inner_margin(egui::Margin::same(6))
                .corner_radius(egui::CornerRadius::same(3))
                .show(ui, |ui| {
                    let font = egui::FontId::monospace(12.0);
                    let color = egui::Color32::from_rgb(200, 200, 200);

                    ui.label(egui::RichText::new(format!("Mode: {mode}")).font(font.clone()).color(color));
                    ui.label(egui::RichText::new(format!(
                        "XYZ: {:.1} / {:.1} / {:.1}",
                        player_pos.x, player_pos.y, player_pos.z
                    )).font(font.clone()).color(color));
                    ui.label(egui::RichText::new(format!("Time: {hours:02}:{mins:02}")).font(font.clone()).color(color));

                    // #44 P1 — facing (compass + standard bearing) and biome.
                    ui.label(egui::RichText::new(format!(
                        "Facing: {} ({}\u{00b0})",
                        yaw_to_compass(debug.yaw),
                        yaw_to_bearing(debug.yaw),
                    )).font(font.clone()).color(color));
                    ui.label(egui::RichText::new(format!("Biome: {}", debug.biome)).font(font.clone()).color(color));
                    // Wind wave §2.3 — the Windmill's read-out: how hard it is
                    // blowing where you stand, and which way.
                    ui.label(egui::RichText::new(format!(
                        "Wind: {} {:.2} {}",
                        crate::wind::word(debug.wind.speed),
                        debug.wind.speed,
                        crate::wind::compass(debug.wind.direction),
                    )).font(font.clone()).color(color));

                    if let Some(t) = target_block {
                        ui.label(egui::RichText::new(format!(
                            "Target: {}, {}, {}", t[0], t[1], t[2]
                        )).font(font.clone()).color(color));
                        // #44 P1 — light level at the targeted block (0–15).
                        if let Some(l) = debug.light {
                            ui.label(egui::RichText::new(format!("Light: {l}")).font(font.clone()).color(color));
                        }
                    }

                    // #44 P1 — FPS derived from the mean frame time (now sampled
                    // on native + WASM). Shown whenever frame timing exists.
                    if let Some(frame_ms) = perf.frame_ms_mean {
                        ui.label(egui::RichText::new(format!(
                            "FPS: {}", fps_from_frame_ms(frame_ms)
                        )).font(font.clone()).color(color));
                    }

                    // Perf line — only when the host engine populates the
                    // sample windows. Format:
                    //   Frame: 14.2 ms (worst 31.5) | Tick: 3.1 ms
                    if perf.frame_ms_mean.is_some() || perf.tick_ms_mean.is_some() {
                        let frame_part = match (perf.frame_ms_mean, perf.frame_ms_worst) {
                            (Some(m), Some(w)) => format!("Frame: {m:.1} ms (worst {w:.1})"),
                            (Some(m), None) => format!("Frame: {m:.1} ms"),
                            _ => String::new(),
                        };
                        let tick_part = match perf.tick_ms_mean {
                            Some(m) => format!("Tick: {m:.1} ms"),
                            None => String::new(),
                        };
                        let line = match (frame_part.is_empty(), tick_part.is_empty()) {
                            (false, false) => format!("{frame_part} | {tick_part}"),
                            (false, true) => frame_part,
                            (true, false) => tick_part,
                            (true, true) => return,
                        };
                        ui.label(egui::RichText::new(line).font(font.clone()).color(color));
                    }

                    // Draw-call line (Spec 39 A1) — shows the frustum-culling
                    // win directly: "Draws" drops sharply when facing a wall.
                    if let Some(draws) = perf.draw_calls {
                        let culled = perf.culled.unwrap_or(0);
                        let total = draws + culled;
                        ui.label(
                            egui::RichText::new(format!(
                                "Draws: {draws} (culled {culled} / {total})"
                            ))
                            .font(font.clone())
                            .color(color),
                        );
                    }
                });
        });
}

/// Draw death screen overlay. W2: `cause` is the line from
/// `survival::death_message` (what killed you); `grave` is
/// `survival::grave_message` when this death left a grave. There is no
/// auto-respawn (W2 2026-10-06): returns true when the player chose Respawn —
/// the button, or Enter when `accept_enter` (the keyboard seat only, so one
/// key press can't respawn every split-screen seat). `hint` names the seat's
/// own respawn input.
pub fn draw_death_screen(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    cause: &str,
    grave: Option<&str>,
    accept_enter: bool,
    hint: &str,
) -> bool {
    let mut respawn = accept_enter && ctx.input(|i| i.key_pressed(egui::Key::Enter));

    // Approximate size of the death panel to centre it within the viewport.
    let panel_w = 320.0_f32;
    let panel_h = 220.0_f32;
    let death_x = viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0;
    let death_y = viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0;

    egui::Area::new(egui::Id::new(("death", player_index)))
        .fixed_pos(egui::pos2(death_x, death_y))
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(100, 0, 0, 180))
                .inner_margin(egui::Margin::same(30))
                .corner_radius(egui::CornerRadius::same(6))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("You Died!")
                                .size(36.0)
                                .color(egui::Color32::from_rgb(255, 80, 80))
                                .strong(),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(cause)
                                .size(18.0)
                                .color(egui::Color32::WHITE),
                        );
                        if let Some(grave) = grave {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(grave)
                                    .size(14.0)
                                    .color(egui::Color32::from_rgb(230, 210, 170)),
                            );
                        }
                        ui.add_space(16.0);
                        if ui.button(
                            egui::RichText::new("Respawn")
                                .size(18.0)
                                .color(egui::Color32::WHITE),
                        ).clicked() {
                            respawn = true;
                        }
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(hint)
                                .size(13.0)
                                .color(egui::Color32::from_rgb(220, 200, 200)),
                        );
                    });
                });
        });

    respawn
}

/// Trials — the in-game objective / help pop-up (H key). A centred, NON-blocking
/// card with the trial's title + how-to + live progress + a close hint. Drawn
/// for player 0 only; the game (and any race clock) keep running behind it, so
/// it's a glance-and-dismiss panel rather than a modal.
pub fn draw_objective_panel(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    title: &str,
    how_to: &str,
    progress: &str,
) {
    let w = 460.0_f32;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - w / 2.0,
        viewport.y as f32 + viewport.height as f32 * 0.16,
    );
    egui::Area::new(egui::Id::new("objective_panel"))
        .fixed_pos(origin)
        .order(egui::Order::Foreground)
        // Task 16 — display-only: don't hit-test (see sign_readout).
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(18, 22, 40, 242))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_max_width(w);
                    ui.label(
                        egui::RichText::new(title)
                            .size(20.0)
                            .strong()
                            .color(egui::Color32::from_rgb(255, 210, 130)),
                    );
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(how_to).size(15.0).color(egui::Color32::WHITE));
                    if !progress.is_empty() {
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(progress)
                                .size(15.0)
                                .strong()
                                .color(egui::Color32::from_rgb(150, 220, 150)),
                        );
                    }
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Press H to close")
                            .size(12.0)
                            .color(egui::Color32::from_gray(170)),
                    );
                });
        });
}

/// Trials — Satoshi's spoken intro card, shown for a few seconds at the top of
/// the screen when a trial starts (his `intro` from `scenario::trial_satoshi`).
/// Non-blocking and self-dismissing: a warm "Satoshi:" speech bubble that lets
/// the kid hear the guide before they glance at the steps (H). Player 0 only.
pub fn draw_satoshi_brief(ctx: &egui::Context, viewport: &ViewportRect, title: &str, intro: &str) {
    let w = 520.0_f32;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - w / 2.0,
        viewport.y as f32 + viewport.height as f32 * 0.05,
    );
    egui::Area::new(egui::Id::new("satoshi_brief"))
        .fixed_pos(origin)
        .order(egui::Order::Foreground)
        // Task 16 — display-only: don't hit-test (see sign_readout).
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(24, 20, 36, 244))
                .stroke(egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(255, 210, 130)))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_max_width(w);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("🧙").size(22.0));
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Satoshi — {title}"))
                                    .size(16.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(255, 210, 130)),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(intro)
                                    .size(15.0)
                                    .color(egui::Color32::from_rgb(235, 235, 245)),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new("Press H any time for the steps")
                                    .size(12.0)
                                    .italics()
                                    .color(egui::Color32::from_gray(165)),
                            );
                        });
                    });
                });
        });
}

/// Goal 1 — small scenario status badge (top-centre): the clock + running work
/// score while a scenario is active and not yet ended. Modelled on
/// `draw_nostrich_vow_badge`; pure read, non-interactable.
/// Nostrich ride speedometer (player 0, top-centre, just under the scenario
/// badge). A wind-up bar that fills as the bird builds speed, flashing
/// "💧 WATER-READY!" once you're fast enough to skim across water.
pub fn draw_nostrich_speed(ctx: &egui::Context, viewport: &ViewportRect, speed: f32) {
    let frac = (speed / crate::nostrich_ride::MAX_SPEED).clamp(0.0, 1.0);
    let water_ready = speed >= crate::nostrich_ride::WATER_RUN_SPEED;
    let bars = (frac * 12.0).round() as usize;
    let meter: String = (0..12).map(|i| if i < bars { '▰' } else { '▱' }).collect();
    let label = if water_ready {
        format!("🪶 {meter}  💧 SKIM!")
    } else {
        format!("🪶 {meter}")
    };
    let approx_w = 240.0_f32;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - approx_w / 2.0,
        viewport.y as f32 + 46.0,
    );
    let (fill, fg) = if water_ready {
        (
            egui::Color32::from_rgba_premultiplied(20, 60, 90, 225),
            egui::Color32::from_rgb(140, 220, 255),
        )
    } else {
        (
            egui::Color32::from_rgba_premultiplied(40, 30, 20, 220),
            egui::Color32::from_rgb(255, 200, 120),
        )
    };
    egui::Area::new(egui::Id::new("nostrich_speed"))
        .fixed_pos(origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style()).fill(fill).show(ui, |ui| {
                ui.label(egui::RichText::new(label).size(16.0).color(fg).strong());
            });
        });
}

pub fn draw_scenario_hud(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    scenario: &crate::scenario::ScenarioState,
) {
    // The badge content depends on objective + state. Timed runs count DOWN and
    // hand off to a blocking modal on end; the Satori speedrun counts UP and ends
    // with a NON-blocking "Genesis Block!" celebration (the player keeps playing).
    let label = match scenario.def().objective {
        crate::scenario::Objective::Timed { .. } => {
            if scenario.is_ended() {
                return; // the blocking end-card (drawn elsewhere) shows the result
            }
            let secs_total = scenario.remaining_ticks() / 20;
            let line = format!("⏱  {}:{:02}", secs_total / 60, secs_total % 60);
            match scenario.def().scoring {
                crate::scenario::Scoring::Work => format!("{line}   Work {}", scenario.score()),
                crate::scenario::Scoring::InventoryVariety => {
                    format!("{line}   🎒 {} different", scenario.score())
                }
                crate::scenario::Scoring::None => line,
            }
        }
        crate::scenario::Objective::FirstSatori => {
            if scenario.is_ended() {
                let t = scenario.result_tick().unwrap_or_else(|| scenario.elapsed_ticks());
                let secs_total = t / 20;
                let day = t / 24_000 + 1;
                format!(
                    "🏆  Genesis Block!  {}:{:02}  ·  Day {}",
                    secs_total / 60,
                    secs_total % 60,
                    day
                )
            } else {
                let secs_total = scenario.elapsed_ticks() / 20;
                format!("⛏  {}:{:02}", secs_total / 60, secs_total % 60)
            }
        }
        // Free-roam exploration shows no objective HUD.
        crate::scenario::Objective::FreeRoam => return,
        // Feature-coverage challenges: render "name — done / total" progress
        // (Action count, or completed steps/items), and a non-blocking
        // "complete!" celebration on the finishing frame.
        crate::scenario::Objective::Action { .. }
        | crate::scenario::Objective::Sequence { .. }
        | crate::scenario::Objective::Checklist { .. } => {
            if scenario.is_ended() {
                if scenario.shows_blocking_end_card() {
                    return; // the blocking completion card (drawn elsewhere) shows the result
                }
                format!("✅  {}  —  complete!", scenario.display_name())
            } else if let Some((done, total)) = scenario.objective_progress() {
                // For a stepped trial, name the step you're on so the badge says
                // WHAT to do now, not just "1 / 4".
                let stepped = matches!(
                    scenario.def().objective,
                    crate::scenario::Objective::Sequence { .. }
                        | crate::scenario::Objective::Checklist { .. }
                );
                let step_label = if stepped {
                    scenario.current_step().and_then(|i| {
                        crate::scenario::trial_tasks_for_display(scenario.display_name())
                            .and_then(|t| t.lines.get(i).cloned())
                    })
                } else {
                    None
                };
                match step_label {
                    Some(lbl) => {
                        format!("🎯  {}  ·  {}/{}  ·  {}", scenario.display_name(), done, total, lbl)
                    }
                    None => format!("🎯  {}  —  {} / {}", scenario.display_name(), done, total),
                }
            } else {
                return;
            }
        }
    };
    let approx_w = 200.0_f32;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - approx_w / 2.0,
        viewport.y as f32 + 12.0,
    );
    egui::Area::new(egui::Id::new(("scenario_hud", player_index)))
        .fixed_pos(origin)
        .interactable(false)
        .order(egui::Order::Tooltip)
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.global_style())
                .fill(egui::Color32::from_rgba_premultiplied(20, 30, 60, 220))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(label)
                            .size(18.0)
                            .color(egui::Color32::from_rgb(255, 210, 130))
                            .strong(),
                    );
                });
        });
}

/// The headline + detail lines for a finished scenario, by objective. Generic
/// so the official mods (Hash Dash timed score-attack, Satori Rush speedrun)
/// reuse the same end card unchanged.
fn scenario_result_text(scenario: &crate::scenario::ScenarioState) -> (String, String) {
    match scenario.def().objective {
        crate::scenario::Objective::Timed { .. } => {
            let detail = match scenario.def().scoring {
                crate::scenario::Scoring::InventoryVariety => {
                    let n = scenario.score();
                    let word = if n == 1 { "different thing" } else { "different things" };
                    format!("{n} {word} collected!")
                }
                _ => format!("{} work done", scenario.score()),
            };
            ("Time's up!".to_string(), detail)
        }
        crate::scenario::Objective::FirstSatori => {
            let ticks = scenario.result_tick().unwrap_or_else(|| scenario.elapsed_ticks());
            let secs_total = ticks / 20;
            let mins = secs_total / 60;
            let secs = secs_total % 60;
            // Day N from the active world-clock (24000 ticks per in-game day).
            let day = ticks / 24_000 + 1;
            // Moonshot Phase A — consistent founding-event framing with the
            // in-world Genesis toast (game_loop.rs). Copy only.
            (
                "Genesis Block — the founding!".to_string(),
                format!("{mins}:{secs:02}   ·   Day {day}"),
            )
        }
        // Free-roam never ends, so this end card is never shown for it.
        crate::scenario::Objective::FreeRoam => (String::new(), String::new()),
        // Coverage challenges end NON-blocking (no modal), so this text is not
        // normally rendered for them; provide a sensible generic result anyway.
        crate::scenario::Objective::Action { .. }
        | crate::scenario::Objective::Sequence { .. }
        | crate::scenario::Objective::Checklist { .. } => (
            "Challenge complete!".to_string(),
            scenario.display_name().to_string(),
        ),
    }
}

/// Goal 1 — scenario end-card overlay (centred modal). Shows the result + a
/// Back-to-Menu button; returns true if it was clicked. Modelled on
/// `draw_death_screen` (`interactable(true)` → captures clicks; the caller
/// gates gameplay input while a scenario is ended).
pub fn draw_scenario_end_screen(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    scenario: &crate::scenario::ScenarioState,
) -> ScenarioEndAction {
    let mut action = ScenarioEndAction::None;
    let (headline, detail) = scenario_result_text(scenario);
    // Coverage challenges are Trials (return to the Trials column + can retry);
    // the official games/experiences keep the plain "Back to Menu".
    let is_trial = scenario.def().kind == crate::scenario::ScenarioKind::Challenge;

    let panel_w = 360.0_f32;
    let panel_h = 200.0_f32;
    let x = viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0;
    let y = viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0;

    egui::Area::new(egui::Id::new(("scenario_end", player_index)))
        .fixed_pos(egui::pos2(x, y))
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(20, 25, 45, 220))
                .inner_margin(egui::Margin::same(30))
                .corner_radius(egui::CornerRadius::same(8))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(scenario.display_name())
                                .size(20.0)
                                .color(egui::Color32::from_rgb(180, 200, 255)),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(headline)
                                .size(30.0)
                                .color(egui::Color32::from_rgb(255, 210, 130))
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(detail)
                                .size(18.0)
                                .color(egui::Color32::WHITE),
                        );
                        ui.add_space(18.0);
                        if is_trial {
                            if ui
                                .button(
                                    egui::RichText::new("← Back to Trials")
                                        .size(18.0)
                                        .color(egui::Color32::WHITE),
                                )
                                .clicked()
                            {
                                action = ScenarioEndAction::BackToTrials;
                            }
                            ui.add_space(8.0);
                            if ui
                                .button(
                                    egui::RichText::new("↻ Try again")
                                        .size(16.0)
                                        .color(egui::Color32::WHITE),
                                )
                                .clicked()
                            {
                                action = ScenarioEndAction::TryAgain;
                            }
                        } else if ui
                            .button(
                                egui::RichText::new("Back to Menu")
                                    .size(18.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .clicked()
                        {
                            action = ScenarioEndAction::BackToMenu;
                        }
                    });
                });
        });

    action
}

/// What the player picked on the scenario end-card (Trials reuse this card for
/// Challenge trials; Experiences keep the plain "Back to Menu").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioEndAction {
    None,
    /// Experience end-card: return to the lobby.
    BackToMenu,
    /// Challenge trial: return to the Trials column (marked ✓).
    BackToTrials,
    /// Challenge trial: re-run the same challenge in place.
    TryAgain,
}

/// Draw the "press A to join" prompt in the bottom-right quadrant when only
/// 3 local players are seated. The 2x2 grid in `compute_screen_layout`
/// leaves that quadrant unrendered (no Screen entry), so its pixels would
/// otherwise carry whatever was last cleared into them — this function
/// paints a flat dark fill + a single line of join-prompt text so the empty
/// slot looks intentional and obviously invites P4.
pub fn draw_empty_quadrant_prompt(ctx: &egui::Context, viewport: &ViewportRect) {
    let fill = egui::Color32::from_rgba_premultiplied(20, 20, 25, 255);
    let text_colour = egui::Color32::from_rgba_premultiplied(220, 220, 230, 220);

    egui::Area::new(egui::Id::new("empty_quadrant_prompt"))
        .fixed_pos(egui::pos2(viewport.x as f32, viewport.y as f32))
        .interactable(false)
        .show(ctx, |ui| {
            let rect = egui::Rect::from_min_size(
                egui::pos2(viewport.x as f32, viewport.y as f32),
                egui::vec2(viewport.width as f32, viewport.height as f32),
            );
            ui.painter().rect_filled(rect, 0.0, fill);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for Player 4 — press A on any controller to join",
                egui::FontId::proportional(16.0),
                text_colour,
            );
        });
}

/// Colour swatch for a richness tier's gauge fill. Cool greys for low
/// tiers, warming through to a bright golden glow at Fat.
pub fn tier_colour(tier: RichnessTier) -> egui::Color32 {
    match tier {
        RichnessTier::Empty => egui::Color32::from_rgb(80, 80, 90),
        RichnessTier::Thin => egui::Color32::from_rgb(90, 130, 170),
        RichnessTier::Healthy => egui::Color32::from_rgb(180, 130, 60),
        RichnessTier::Fat => egui::Color32::from_rgb(240, 200, 90),
    }
}

/// Mining-rate text shown below the gauge. The rate is a kid-readable
/// proxy for the underlying §7.2 reward-multiplier — at Fat it's full,
/// at Empty it's zero. Linear interpolation in between.
///
/// BRIDGE: the precise per-1000-digs payout comes from Spec 6 §3.3
/// pool-sustainability math when D-003 reverses. For alpha visuals we
/// show a rounded sats/1000-digs figure derived directly from richness.
fn mining_rate_text(richness: f32) -> String {
    let base_per_1000_digs = 20.0; // sats — placeholder for Spec 6 §3.3
    let rate = (base_per_1000_digs * richness.clamp(0.0, 1.0)).round() as u32;
    format!("Mining rate: ~{} sats / 1000 digs", rate)
}

/// Draw the Deepslate Reserve gauge in the top-right of the viewport.
/// Spec 16 Phase 4. Cross-platform via egui. Gated by `show_debug` in
/// alpha (F3) until the full Bitcoin menu lands (Spec 6 §6.5);
/// post-alpha this moves into that menu unchanged.
///
/// Returns `true` if the player clicked the "Fund the Reserve" button —
/// caller should open the [`draw_fund_dialog`] flow.
pub fn draw_reserve_gauge(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    reserve: &ReserveState,
    cursor_captured: bool,
) -> bool {
    let tier = reserve.tier();
    let colour = tier_colour(tier);

    let panel_w = 240.0_f32;
    let panel_x = viewport.x as f32 + viewport.width as f32 - panel_w - 12.0;
    let panel_y = viewport.y as f32 + 12.0;

    let mut clicked = false;

    egui::Area::new(egui::Id::new(("reserve_gauge", player_index)))
        .fixed_pos(egui::pos2(panel_x, panel_y))
        // Task 16 — hit-test only while the cursor is free: the Fund button
        // needs a visible cursor to click, and a captured cursor resting here
        // must not trip the `wants_pointer_input()` gameplay gate.
        .interactable(!cursor_captured)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(0, 0, 0, 180))
                .inner_margin(egui::Margin::same(10))
                .corner_radius(egui::CornerRadius::same(4))
                .show(ui, |ui| {
                    ui.set_min_width(panel_w - 20.0);

                    // Title row
                    ui.label(
                        egui::RichText::new("The Reserve")
                            .color(egui::Color32::from_rgb(220, 200, 120))
                            .strong()
                            .size(13.0),
                    );

                    // Gauge bar
                    let bar_h = 12.0;
                    let bar_w = panel_w - 20.0;
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(bar_w, bar_h),
                        egui::Sense::hover(),
                    );
                    let bg = egui::Color32::from_rgba_premultiplied(30, 30, 40, 255);
                    ui.painter().rect_filled(rect, 2.0, bg);

                    let fill_w = bar_w * reserve.richness.clamp(0.0, 1.0);
                    let fill_rect = egui::Rect::from_min_size(
                        rect.left_top(),
                        egui::vec2(fill_w, bar_h),
                    );
                    ui.painter().rect_filled(fill_rect, 2.0, colour);

                    // Label + percent
                    let pct = (reserve.richness.clamp(0.0, 1.0) * 100.0).round() as u32;
                    ui.label(
                        egui::RichText::new(format!("{} — {}%", tier.label(), pct))
                            .color(colour)
                            .size(12.0),
                    );

                    // Sats counter
                    ui.label(
                        egui::RichText::new(format!(
                            "{} / {} sats",
                            reserve.current_sats, reserve.target_sats
                        ))
                        .color(egui::Color32::from_rgb(180, 180, 180))
                        .size(11.0),
                    );

                    // Mining-rate text — the operational consequence
                    ui.label(
                        egui::RichText::new(mining_rate_text(reserve.richness))
                            .color(egui::Color32::from_rgb(180, 180, 180))
                            .size(11.0),
                    );

                    ui.add_space(4.0);
                    if ui.button(
                        egui::RichText::new("Fund the Reserve")
                            .size(12.0)
                            .color(egui::Color32::from_rgb(220, 200, 120)),
                    ).clicked() {
                        clicked = true;
                    }
                });
        });

    clicked
}

/// Fund-the-Reserve dialog state. Persists across frames until the
/// player Cancels or Confirms.
#[derive(Clone, Debug)]
pub struct FundDialogState {
    /// Selected funding amount in sats.
    pub amount_sats: u32,
    /// Whether the player has ticked "Show my contribution anonymously".
    pub anonymous: bool,
}

impl Default for FundDialogState {
    fn default() -> Self {
        Self {
            amount_sats: 1000,
            anonymous: false,
        }
    }
}

/// Outcome of a single Fund dialog render. Caller acts on each
/// frame's return value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FundDialogAction {
    /// Player closed the dialog without paying.
    Cancel,
    /// Player confirmed payment. Caller mocks Lightning invoice flow
    /// (no LNbits in alpha — BRIDGE).
    Confirm { amount_sats: u32, anonymous: bool },
    /// Dialog still open, no action this frame.
    KeepOpen,
}

/// Compute split breakdown in sats for a given total + percentages.
/// Returned as (pool, creator, platform, reserve). Reserve sweeps any
/// rounding remainder so percentages sum to total even with integer
/// truncation.
pub(crate) fn compute_split_breakdown(
    total_sats: u32,
    pool_pct: u32,
    creator_pct: u32,
    platform_pct: u32,
) -> (u32, u32, u32, u32) {
    let pool = (total_sats * pool_pct) / 100;
    let creator = (total_sats * creator_pct) / 100;
    let platform = (total_sats * platform_pct) / 100;
    let reserve = total_sats - pool - creator - platform;
    (pool, creator, platform, reserve)
}

/// Draw the Fund-the-Reserve dialog. Spec 16 Phase 5. Mutates `state`
/// in place as the player tweaks inputs; returns a [`FundDialogAction`]
/// telling the caller whether to close the dialog or keep it open.
///
/// In alpha the "Confirm" action is mocked — there's no LNbits in the
/// engine yet (Sentinel D-003 deferred). The dialog renders correctly
/// and the audit log records the intent; real Lightning payment lands
/// when Spec 6 §4 + §6.2 ship in code.
pub fn draw_fund_dialog(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    state: &mut FundDialogState,
) -> FundDialogAction {
    // Default split percentages (Spec 6 §5.2 standard template).
    let pool_pct = 50;
    let creator_pct = 30;
    let platform_pct = 15;
    // reserve = remainder

    let mut action = FundDialogAction::KeepOpen;

    let panel_w = 460.0_f32;
    let panel_h = 380.0_f32;
    let panel_x = viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0;
    let panel_y = viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0;

    egui::Area::new(egui::Id::new(("fund_dialog", player_index)))
        .fixed_pos(egui::pos2(panel_x, panel_y))
        .interactable(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(10, 10, 16, 240))
                .inner_margin(egui::Margin::same(20))
                .corner_radius(egui::CornerRadius::same(6))
                .stroke(egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(120, 100, 60)))
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(panel_w - 40.0, panel_h - 40.0));

                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("Fund the Reserve")
                                .size(20.0)
                                .strong()
                                .color(egui::Color32::from_rgb(220, 200, 120)),
                        );
                    });
                    ui.add_space(8.0);

                    // Amount selection — preset chips
                    ui.label(
                        egui::RichText::new("How much to sponsor?").size(13.0),
                    );
                    ui.horizontal(|ui| {
                        for preset in [100u32, 500, 1000, 5000] {
                            let selected = state.amount_sats == preset;
                            let label = format!("{} sats", preset);
                            let mut btn = egui::Button::new(label);
                            if selected {
                                btn = btn.fill(egui::Color32::from_rgb(60, 80, 110));
                            }
                            if ui.add(btn).clicked() {
                                state.amount_sats = preset;
                            }
                        }
                    });
                    ui.add_space(8.0);

                    // Split breakdown
                    let (pool, creator, platform, reserve) = compute_split_breakdown(
                        state.amount_sats, pool_pct, creator_pct, platform_pct,
                    );
                    ui.label(
                        egui::RichText::new("Your sats will split:").size(12.0),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "  Pool (everyone mines): {} sats ({}%)",
                            pool, pool_pct
                        )).size(11.0)
                            .color(egui::Color32::from_rgb(240, 200, 90)),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "  Server operator: {} sats ({}%)",
                            creator, creator_pct
                        )).size(11.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "  Platform: {} sats ({}%)",
                            platform, platform_pct
                        )).size(11.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "  Reserve buffer: {} sats",
                            reserve
                        )).size(11.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)),
                    );
                    ui.add_space(8.0);

                    // Confirmation copy — sponsorship messaging
                    ui.label(
                        egui::RichText::new(
                            "You're sponsoring this world. Your contribution makes \
                             mining more rewarding for every player. You don't get \
                             sats back to your own wallet — you earn yours by \
                             playing, the same as everyone else.",
                        )
                        .size(11.0)
                        .color(egui::Color32::from_rgb(200, 200, 200)),
                    );
                    ui.add_space(6.0);

                    // Anonymous toggle
                    ui.checkbox(&mut state.anonymous, "Show my contribution anonymously");
                    ui.add_space(8.0);

                    // Action buttons
                    ui.horizontal(|ui| {
                        if ui.button(
                            egui::RichText::new("Cancel").size(13.0),
                        ).clicked() {
                            action = FundDialogAction::Cancel;
                        }
                        ui.add_space(20.0);
                        if ui.button(
                            egui::RichText::new(format!("Confirm — pay {} sats", state.amount_sats))
                                .size(13.0)
                                .color(egui::Color32::from_rgb(120, 200, 120)),
                        ).clicked() {
                            action = FundDialogAction::Confirm {
                                amount_sats: state.amount_sats,
                                anonymous: state.anonymous,
                            };
                        }
                    });
                });
        });

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_full_size_above_threshold() {
        // 1080p TV quad quadrants (960×540), 4K quads (1920×1080), and
        // any single-player full-screen viewport must stay at 1.0 scale.
        // Widths are in egui points (DPI-independent).
        assert_eq!(hud_scale(960.0), 1.0);
        assert_eq!(hud_scale(1920.0), 1.0);
        assert_eq!(hud_scale(720.0), 1.0);
    }

    #[test]
    fn hud_shrinks_below_threshold() {
        // Sub-1080p TV → quad quadrants get the shrunk HUD so the
        // 9-slot hotbar stops crowding the viewport edges.
        assert_eq!(hud_scale(640.0), 0.75);
        assert_eq!(hud_scale(480.0), 0.75);
        assert_eq!(hud_scale(360.0), 0.75);
    }

    #[test]
    fn hotbar_fits_inside_smallest_target_quad() {
        // 480×270 (sub-1080p quad) is the smallest documented quad
        // quadrant. The scaled hotbar (9 * 48 + 8 * 4 = 464px at full
        // size → 348px at 0.75) must leave at least 16px of slack on
        // each side so the slots aren't pressed into the bezel.
        let scale = hud_scale(480.0);
        let total_w = 9.0 * 48.0 * scale + 8.0 * 4.0 * scale;
        assert!(total_w < 480.0 - 32.0, "scaled hotbar {} must leave slack", total_w);
    }

    // ── Reserve gauge helpers ────────────────────────────────────────

    #[test]
    fn tier_colour_warms_with_richness() {
        // Sanity: warmer colour at higher tier. Compare R channel.
        // Empty/Thin are cool greys/blues (low R); Healthy/Fat warm
        // through to gold (high R).
        let r_empty = tier_colour(RichnessTier::Empty).r();
        let r_thin = tier_colour(RichnessTier::Thin).r();
        let r_healthy = tier_colour(RichnessTier::Healthy).r();
        let r_fat = tier_colour(RichnessTier::Fat).r();
        assert!(r_healthy > r_thin, "Healthy should be warmer than Thin");
        assert!(r_fat > r_healthy, "Fat should be warmer than Healthy");
        // Empty is cool-grey; Thin shifts towards cool-blue but
        // similar low-R. Just check they're not warmer than Healthy.
        assert!(r_empty < r_healthy);
    }

    #[test]
    fn mining_rate_scales_with_richness() {
        // 1.0 -> base rate (~20 sats/1000 digs). 0.0 -> 0. 0.5 -> half.
        let full = mining_rate_text(1.0);
        assert!(full.contains("20"), "full rate expected ~20: {}", full);
        let empty = mining_rate_text(0.0);
        assert!(empty.contains("0"), "empty rate expected 0: {}", empty);
        let half = mining_rate_text(0.5);
        assert!(half.contains("10"), "half rate expected ~10: {}", half);
    }

    #[test]
    fn mining_rate_clamps_above_one() {
        // A misbehaving server claiming richness > 1.0 must not produce
        // absurd mining-rate text — clamp at the upper bound.
        let over = mining_rate_text(2.5);
        assert!(over.contains("20"), "clamped to 20 sats: {}", over);
    }

    #[test]
    fn mining_rate_handles_negative() {
        // Negative richness clamps to 0.
        let neg = mining_rate_text(-1.0);
        assert!(neg.contains("0"), "negative clamps to 0: {}", neg);
    }

    // ── Fund-the-Reserve dialog ──────────────────────────────────────

    #[test]
    fn split_breakdown_sums_to_total() {
        // The dialog displays four numbers; if they don't sum to the
        // total the player will spot the discrepancy. Test every preset.
        for total in [100u32, 500, 1000, 5000] {
            let (pool, creator, platform, reserve) =
                compute_split_breakdown(total, 50, 30, 15);
            assert_eq!(
                pool + creator + platform + reserve, total,
                "split for {} didn't sum: pool {} + creator {} + platform {} + reserve {}",
                total, pool, creator, platform, reserve,
            );
        }
    }

    #[test]
    fn split_breakdown_matches_spec_6_default_percentages() {
        // Spec 6 §5.2 standard template: pool 50%, creator 30%, platform
        // 15%, reserve 5%. A 1000-sat payment should split 500/300/150/50.
        let (pool, creator, platform, reserve) =
            compute_split_breakdown(1000, 50, 30, 15);
        assert_eq!(pool, 500);
        assert_eq!(creator, 300);
        assert_eq!(platform, 150);
        assert_eq!(reserve, 50);
    }

    #[test]
    fn split_breakdown_rounding_lands_in_reserve() {
        // Integer math: a 7-sat split at 50/30/15/5 truncates the named
        // shares and the remainder rolls into reserve.
        let (pool, creator, platform, reserve) =
            compute_split_breakdown(7, 50, 30, 15);
        // pool = 7*50/100 = 3 ; creator = 7*30/100 = 2 ; platform = 7*15/100 = 1
        // reserve = 7 - 3 - 2 - 1 = 1
        assert_eq!(pool, 3);
        assert_eq!(creator, 2);
        assert_eq!(platform, 1);
        assert_eq!(reserve, 1);
        assert_eq!(pool + creator + platform + reserve, 7);
    }

    #[test]
    fn fund_dialog_state_default_is_reasonable() {
        // The dialog defaults must be sensible — non-zero amount,
        // public sponsorship (anonymous = false by default per Spec
        // 16 §5 + Spec 6 §6.2 amended: sponsorship is public by
        // default, opt-out available).
        let state = FundDialogState::default();
        assert!(state.amount_sats >= 100, "default amount must be at least the minimum");
        assert!(!state.anonymous, "default sponsorship is public");
    }

    #[test]
    fn fund_action_equality_for_caller_branches() {
        // The caller pattern-matches FundDialogAction; quick sanity
        // that Confirm with the same fields is equal so tests can use
        // direct comparison.
        let a = FundDialogAction::Confirm { amount_sats: 500, anonymous: true };
        let b = FundDialogAction::Confirm { amount_sats: 500, anonymous: true };
        let c = FundDialogAction::Confirm { amount_sats: 500, anonymous: false };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    // ── #44 Phase 1 — debug HUD helpers (facing + FPS) ───────────────

    #[test]
    fn yaw_to_compass_cardinals() {
        use std::f32::consts::PI;
        // Camera convention (camera.rs:483 forward()): yaw 0 → -Z, and
        // increasing yaw rotates CCW (yaw π/2 → -X). Minecraft axis
        // labelling: -Z=N, -X=W, +Z=S, +X=E.
        assert_eq!(yaw_to_compass(0.0), "N");
        assert_eq!(yaw_to_compass(PI / 2.0), "W");
        assert_eq!(yaw_to_compass(PI), "S");
        assert_eq!(yaw_to_compass(3.0 * PI / 2.0), "E");
        // Wrap-around: 2π is North again.
        assert_eq!(yaw_to_compass(2.0 * PI), "N");
        // Negative yaw normalises (-π/2 == 3π/2 → E).
        assert_eq!(yaw_to_compass(-PI / 2.0), "E");
    }

    #[test]
    fn yaw_to_compass_intercardinals() {
        use std::f32::consts::PI;
        // π/4 sits exactly between N (-Z) and W (-X) → NW.
        assert_eq!(yaw_to_compass(PI / 4.0), "NW");
        assert_eq!(yaw_to_compass(3.0 * PI / 4.0), "SW");
        assert_eq!(yaw_to_compass(5.0 * PI / 4.0), "SE");
        assert_eq!(yaw_to_compass(7.0 * PI / 4.0), "NE");
    }

    #[test]
    fn yaw_to_bearing_standard_compass() {
        use std::f32::consts::PI;
        // Standard compass bearing: N=0, E=90, S=180, W=270.
        assert_eq!(yaw_to_bearing(0.0), 0); // N
        assert_eq!(yaw_to_bearing(PI), 180); // S
        // yaw π/2 faces W → bearing 270; yaw 3π/2 faces E → bearing 90.
        assert_eq!(yaw_to_bearing(PI / 2.0), 270);
        assert_eq!(yaw_to_bearing(3.0 * PI / 2.0), 90);
        // Wrap stays in [0,360).
        assert!(yaw_to_bearing(2.0 * PI) < 360);
    }

    #[test]
    fn fps_from_frame_ms_typical() {
        // 16.667 ms ≈ 60 fps; 33.333 ms ≈ 30 fps.
        assert_eq!(fps_from_frame_ms(1000.0 / 60.0), 60);
        assert_eq!(fps_from_frame_ms(1000.0 / 30.0), 30);
        assert_eq!(fps_from_frame_ms(10.0), 100);
    }

    #[test]
    fn fps_from_frame_ms_guards_nonpositive() {
        // No samples yet (0 ms) or a bogus negative → 0, never a divide blow-up.
        assert_eq!(fps_from_frame_ms(0.0), 0);
        assert_eq!(fps_from_frame_ms(-5.0), 0);
    }
}


#[cfg(test)]
mod plan_hint_tests {
    use super::*;

    #[test]
    fn plan_hint_warns_about_the_wall_overload() {
        assert!(plan_hint_text(true).contains("WALL"), "wall aim warns about hanging");
        assert!(plan_hint_text(false).contains("Right-click the ground"));
    }
}
