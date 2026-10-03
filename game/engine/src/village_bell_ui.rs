//! Spec 27 Phase 9 — Village Bell right-click dialog with "Houses"
//! tab. Lists each house in the village with its plan name, the
//! architect credit (first link in the derivation chain), a per-
//! architect tip button, and a "Tip all" pool button that fans out
//! one sat per architect through `economy::apply_sats_payout`.
//!
//! Pure render — the caller drives state mutation (tipping, dialog
//! dismissal) from the outcome enum.


use crate::plan::ArchitectPlaqueData;

/// One row in the "Houses" tab — a house found within the village's
/// reach radius around the bell.
#[derive(Clone, Debug)]
pub struct HouseEntry {
    /// World position of the Plaque block this row represents. The
    /// caller uses this to scope the per-architect tip + to render
    /// debug info if requested.
    pub plaque_pos: (i32, i32, i32),
    /// The plan name from the most recent link in the derivation
    /// chain (the "current" plan); empty chain falls back to
    /// "(unknown plan)".
    pub plan_name: String,
    /// First link's author npub — the original architect. Empty
    /// string when the plan is unsigned (every engine-bundled v1
    /// plan, since Spec 1 Phase 4 auth isn't live).
    pub architect_npub: String,
}

/// Returned by `show_bell_dialog`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BellDialogOutcome {
    InProgress,
    Closed,
    /// Player clicked the per-house Tip button. The usize is the
    /// index into the `houses` slice passed to `show_bell_dialog`.
    TipHouse(usize),
    /// Player clicked the "Tip all" pool button. Caller fans out
    /// one sat per non-empty architect across every house.
    TipAll,
}

/// Render the Village Bell info dialog. Returns the player's action
/// this frame (default `InProgress`).
///
/// `tip_eligible` mirrors the plaque dialog's check — Charter sats
/// enabled AND server-policy Bitcoin enabled. `tips_visible`
/// ([`crate::economy::sats_ui_visible`]) hides every tip button when sats
/// are off (review W4 S4).
pub fn show_bell_dialog(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    houses: &[HouseEntry],
    tip_eligible: bool,
    tips_visible: bool,
) -> BellDialogOutcome {
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("bell_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    let panel_w = 540.0;
    let panel_h = (200.0 + (houses.len() as f32 * 36.0)).min(600.0);
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0,
    );

    let mut outcome = BellDialogOutcome::InProgress;

    egui::Area::new(egui::Id::new(("bell_dialog", player_index)))
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
                    egui::RichText::new("\u{1F514} Village Bell")
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(230, 200, 120)),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new("Houses")
                        .size(15.0)
                        .strong()
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                );
                ui.add_space(8.0);
            });
            if houses.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("No attributable houses near this bell yet.")
                            .size(13.0)
                            .italics()
                            .color(egui::Color32::from_rgb(160, 160, 180)),
                    );
                    ui.add_space(10.0);
                });
            } else {
                egui::ScrollArea::vertical()
                    .max_height(panel_h - 130.0)
                    .show(ui, |ui| {
                        for (i, house) in houses.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.add_space(16.0);
                                ui.label(
                                    egui::RichText::new(&house.plan_name)
                                        .size(14.0)
                                        .strong()
                                        .color(egui::Color32::from_rgb(220, 220, 220)),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.add_space(16.0);
                                        let architect_known = !house.architect_npub.is_empty();
                                        let can_tip = tip_eligible && architect_known;
                                        if tips_visible {
                                        let btn = egui::Button::new(
                                            egui::RichText::new("Tip 1 sat").size(11.0),
                                        );
                                        let resp = ui.add_enabled(can_tip, btn);
                                        if resp.clicked() {
                                            outcome = BellDialogOutcome::TipHouse(i);
                                        }
                                        if !can_tip {
                                            let reason = if !architect_known {
                                                "Architect is unsigned — no tip target."
                                            } else {
                                                "Bitcoin disabled for this account / server."
                                            };
                                            resp.on_hover_text(reason);
                                        }
                                        }
                                        let credit = if architect_known {
                                            format!(
                                                "by {}…",
                                                &house.architect_npub
                                                    [..house.architect_npub.len().min(12)],
                                            )
                                        } else {
                                            "(unsigned)".to_string()
                                        };
                                        ui.label(
                                            egui::RichText::new(credit)
                                                .size(12.0)
                                                .italics()
                                                .color(egui::Color32::from_rgb(160, 160, 180)),
                                        );
                                    },
                                );
                            });
                            ui.add_space(4.0);
                        }
                    });
            }
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    ui.add_space((panel_w - 280.0) / 2.0);
                    // Tip-all is enabled only when at least one house has
                    // a signed architect AND the player can send sats.
                    let any_signed = houses.iter().any(|h| !h.architect_npub.is_empty());
                    let can_tip_all = tip_eligible && any_signed;
                    if tips_visible {
                    let tip_all_btn = egui::Button::new(
                        egui::RichText::new("Tip all architects").size(13.0).strong(),
                    );
                    let resp = ui.add_sized([160.0, 30.0], tip_all_btn);
                    let resp = if !can_tip_all {
                        if !any_signed {
                            resp.on_hover_text("No signed architects to tip.")
                        } else {
                            resp.on_hover_text("Bitcoin disabled for this account / server.")
                        }
                    } else {
                        resp
                    };
                    if can_tip_all && resp.clicked() {
                        outcome = BellDialogOutcome::TipAll;
                    }
                    }
                    if ui
                        .add_sized(
                            [110.0, 30.0],
                            egui::Button::new(egui::RichText::new("Close").size(13.0)),
                        )
                        .clicked()
                    {
                        outcome = BellDialogOutcome::Closed;
                    }
                });
            });
        });
    outcome
}

/// Spec 27 Phase 9 — pure helper that builds the "Houses" tab rows
/// from the world's plaque map for every plaque within `radius` of
/// the bell at `bell_pos`. Procgen + player-built plaques both
/// surface here; the dialog labels them identically because both
/// are attributable.
pub fn collect_houses_near_bell(
    bell_pos: (i32, i32, i32),
    plaques: &ahash::AHashMap<(i32, i32, i32), ArchitectPlaqueData>,
    radius: i32,
) -> Vec<HouseEntry> {
    let (bx, _, bz) = bell_pos;
    let mut rows: Vec<HouseEntry> = plaques
        .iter()
        .filter(|((x, _, z), _)| {
            let dx = x - bx;
            let dz = z - bz;
            dx * dx + dz * dz <= radius * radius
        })
        .map(|(pos, data)| {
            let (plan_name, architect_npub) = match data.chain.first() {
                Some(link) => (link.plan_name.clone(), link.author_npub.clone()),
                None => ("(unknown plan)".to_string(), String::new()),
            };
            HouseEntry {
                plaque_pos: *pos,
                plan_name,
                architect_npub,
            }
        })
        .collect();
    // Deterministic order so the dialog rows don't shuffle between
    // frames or test runs.
    rows.sort_by_key(|r| (r.plaque_pos.0, r.plaque_pos.1, r.plaque_pos.2));
    rows
}

/// Spec 27 Phase 9 — fan-out helper for "Tip all". Routes one sat per
/// signed architect through the unified payout pipeline so server-tax
/// + Charter + Reserve gates apply per leg. Returns the total
///   credited (sum across legs) + the number of legs that were
///   suppressed (Charter or Bitcoin disabled). Pure — no engine state
///   touched; caller handles toast + audit log.
pub fn tip_all_pool(
    houses: &[HouseEntry],
    policy: &crate::economy::ServerSatsPolicy,
    charter_allows_sats: bool,
) -> TipAllResult {
    let mut total_credited = 0u64;
    let mut legs_paid = 0u32;
    let mut legs_suppressed = 0u32;
    let mut legs_skipped_unsigned = 0u32;
    for h in houses {
        if h.architect_npub.is_empty() {
            legs_skipped_unsigned += 1;
            continue;
        }
        let r = crate::economy::apply_sats_payout(
            1,
            crate::economy::PayoutKind::PlaqueTip,
            policy,
            charter_allows_sats,
        );
        if r.suppressed {
            legs_suppressed += 1;
        } else {
            total_credited += r.credited;
            legs_paid += 1;
        }
    }
    TipAllResult {
        total_credited,
        legs_paid,
        legs_suppressed,
        legs_skipped_unsigned,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TipAllResult {
    pub total_credited: u64,
    pub legs_paid: u32,
    pub legs_suppressed: u32,
    pub legs_skipped_unsigned: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economy::ServerSatsPolicy;
    use crate::plan::{ArchitectPlaqueData, DerivationLink, PlanLicense};

    fn link(name: &str, npub: &str) -> DerivationLink {
        DerivationLink {
            author_npub: npub.to_string(),
            plan_name: name.to_string(),
            license: PlanLicense::Ccbysa,
            captured_at: 0,
            plan_hash: [0u8; 32],
        }
    }

    fn plaque(name: &str, npub: &str) -> ArchitectPlaqueData {
        ArchitectPlaqueData {
            chain: vec![link(name, npub)],
            authored_in: "creative".to_string(),
            builder_credit: None,
        }
    }

    #[test]
    fn collect_houses_filters_by_radius_and_extracts_credit() {
        let mut map = ahash::AHashMap::new();
        map.insert((0, 70, 0), plaque("Near", "npub1aaaaaaaaaaa"));
        map.insert((100, 70, 100), plaque("Far", "npub1bbbbbbbbbbb"));
        let rows = collect_houses_near_bell((0, 70, 0), &map, 32);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].plan_name, "Near");
        assert_eq!(rows[0].architect_npub, "npub1aaaaaaaaaaa");
    }

    #[test]
    fn collect_houses_handles_empty_chain_safely() {
        let mut map = ahash::AHashMap::new();
        map.insert(
            (0, 70, 0),
            ArchitectPlaqueData { chain: vec![], authored_in: "creative".to_string(), builder_credit: None },
        );
        let rows = collect_houses_near_bell((0, 70, 0), &map, 8);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].plan_name, "(unknown plan)");
        assert_eq!(rows[0].architect_npub, "");
    }

    #[test]
    fn tip_all_pool_credits_one_sat_per_signed_architect() {
        let houses = vec![
            HouseEntry { plaque_pos: (0, 70, 0), plan_name: "A".into(), architect_npub: "npub-a".into() },
            HouseEntry { plaque_pos: (5, 70, 0), plan_name: "B".into(), architect_npub: "npub-b".into() },
            HouseEntry { plaque_pos: (10, 70, 0), plan_name: "C".into(), architect_npub: String::new() },
        ];
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let r = tip_all_pool(&houses, &policy, true);
        assert_eq!(r.legs_paid, 2);
        assert_eq!(r.legs_skipped_unsigned, 1);
        assert_eq!(r.legs_suppressed, 0);
        assert_eq!(r.total_credited, 2);
    }

    #[test]
    fn tip_all_pool_records_suppressed_when_charter_disabled() {
        let houses = vec![
            HouseEntry { plaque_pos: (0, 70, 0), plan_name: "A".into(), architect_npub: "npub-a".into() },
            HouseEntry { plaque_pos: (5, 70, 0), plan_name: "B".into(), architect_npub: "npub-b".into() },
        ];
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let r = tip_all_pool(&houses, &policy, false);
        assert_eq!(r.legs_paid, 0);
        assert_eq!(r.legs_suppressed, 2);
        assert_eq!(r.total_credited, 0);
    }

    #[test]
    fn tip_all_pool_applies_per_leg_tax() {
        let houses = vec![
            HouseEntry { plaque_pos: (0, 70, 0), plan_name: "A".into(), architect_npub: "npub-a".into() },
            HouseEntry { plaque_pos: (5, 70, 0), plan_name: "B".into(), architect_npub: "npub-b".into() },
        ];
        // 100% tax means each leg credits 0 to the architect — still
        // not suppressed (the helper records taxed-to-zero as a
        // successful leg per Spec 19 follow-on `economy::apply_sats_payout`
        // contract).
        let policy = ServerSatsPolicy { server_tax_bps: 10_000, ..ServerSatsPolicy::bitcoin_enabled_policy() };
        let r = tip_all_pool(&houses, &policy, true);
        assert_eq!(r.legs_paid, 2);
        assert_eq!(r.total_credited, 0);
    }

    #[test]
    fn collect_houses_orders_deterministically_by_position() {
        let mut map = ahash::AHashMap::new();
        map.insert((10, 70, 0), plaque("B", "n1"));
        map.insert((0, 70, 0), plaque("A", "n2"));
        map.insert((5, 70, 5), plaque("C", "n3"));
        let rows = collect_houses_near_bell((0, 70, 0), &map, 64);
        let names: Vec<&str> = rows.iter().map(|r| r.plan_name.as_str()).collect();
        assert_eq!(names, vec!["A", "C", "B"]);
    }
}
