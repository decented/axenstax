//! Spec 26 Phase 5 — Commission dialog for the NPC Builder.
//!
//! Renders an egui overlay (layered painter, Spec 19 villager dialogue
//! pattern) with:
//! 1. Plan slot picker (hotbar scan; `Item::Plan` only).
//! 2. Materials-required readout (green tick / red cross per id).
//! 3. "Pick build site" button → flips the dialog into pick mode.
//! 4. Fee preview (base + per-block + premium + reputation discount).
//! 5. Confirm button (greyed until plan + site + materials + sats are
//!    sufficient and the Charter/server policy allows sats movement).
//!
//! Wiring: the game loop opens the dialog when a player right-clicks a
//! Builder villager OR right-clicks a DRAFTING_TABLE block. On Confirm
//! it returns `CommissionDialogOutcome::Confirm`; on Cancel,
//! `Closed`; on the "Pick build site" button, `PickSite`. The actual
//! commission spawning (material lock + fee debit + entity-component
//! insertion) is in `game_loop.rs` so this module stays pure-UI.


use crate::block::{self, BlockId, BlockRegistry};
use crate::builder::{self, CommissionDraft};
use crate::economy::ServerSatsPolicy;
use crate::plan::PlanData;
use crate::reputation::Tier as ReputationTier;

/// What the dialog produced this frame. `None` = no UI change yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommissionDialogOutcome {
    None,
    /// Player clicked "Pick build site". The game loop should close
    /// the dialog (keep `open_commission_villager`), enter pick mode,
    /// then re-open the dialog when the anchor is committed.
    PickSite,
    /// Player picked a different plan slot. Carries the new hotbar idx.
    PlanSlotChanged(usize),
    /// Player clicked Confirm. The game loop locks materials, debits
    /// the fee, and attaches the commission to the villager.
    Confirm,
    /// Player clicked Close / Cancel. Drops the draft.
    Closed,
}

/// One row of the materials readout: required vs owned + sufficiency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialRow {
    pub block_id: BlockId,
    pub needed: u32,
    pub owned: u32,
}

impl MaterialRow {
    pub fn sufficient(self) -> bool {
        self.owned >= self.needed
    }
}

/// Build the materials-required table for the dialog. Pure — used by
/// both the renderer + the Confirm-button enable check.
pub fn build_material_rows(
    plan: &PlanData,
    inventory: &crate::inventory::Inventory,
) -> Vec<MaterialRow> {
    let counts = builder::cell_block_counts(plan);
    counts
        .into_iter()
        .map(|(block_id, needed)| MaterialRow {
            block_id,
            needed,
            owned: builder::inventory_block_count(inventory, block_id),
        })
        .collect()
}

/// True iff every row in `rows` has enough materials. In creative
/// mode, materials aren't required so the gate is always open.
pub fn rows_sufficient(rows: &[MaterialRow], is_creative: bool) -> bool {
    is_creative || rows.iter().all(|r| r.sufficient())
}

/// Returned by `confirm_gate` — bundles the "is Confirm enabled?" check
/// with the human-readable reason if it isn't, so the dialog can show
/// a hover-text explaining why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmGate {
    Ready,
    NoPlan,
    NoSite,
    InsufficientMaterials,
    InsufficientSats,
    CharterDisabled,
    ServerDisabled,
}

impl ConfirmGate {
    pub fn enabled(&self) -> bool { *self == ConfirmGate::Ready }

    pub fn reason(&self) -> &'static str {
        match self {
            ConfirmGate::Ready => "",
            ConfirmGate::NoPlan => "Slot a Plan to commission.",
            ConfirmGate::NoSite => "Pick a build site first.",
            ConfirmGate::InsufficientMaterials => "You don't have enough materials.",
            ConfirmGate::InsufficientSats => "You don't have enough sats for the fee.",
            ConfirmGate::CharterDisabled => "Bitcoin disabled by your Charter.",
            ConfirmGate::ServerDisabled => "Bitcoin disabled on this server.",
        }
    }
}

/// Decide whether the Confirm button is enabled. Pure.
pub fn confirm_gate(
    draft: &CommissionDraft,
    plan: Option<&PlanData>,
    rows: &[MaterialRow],
    fee_sats: u32,
    player_sats_balance: u64,
    charter_allows_sats: bool,
    policy: &ServerSatsPolicy,
    is_creative: bool,
) -> ConfirmGate {
    if !charter_allows_sats {
        return ConfirmGate::CharterDisabled;
    }
    if !policy.bitcoin_enabled {
        return ConfirmGate::ServerDisabled;
    }
    if plan.is_none() || draft.plan_hotbar_slot.is_none() {
        return ConfirmGate::NoPlan;
    }
    if draft.site_anchor.is_none() {
        return ConfirmGate::NoSite;
    }
    if !rows_sufficient(rows, is_creative) {
        return ConfirmGate::InsufficientMaterials;
    }
    if (fee_sats as u64) > player_sats_balance {
        return ConfirmGate::InsufficientSats;
    }
    ConfirmGate::Ready
}

/// Reputation-discount label for the fee preview. Returns `Some(label)`
/// when there's an actual discount; `None` for Neutral (no badge).
pub fn discount_label(tier: ReputationTier) -> Option<&'static str> {
    match tier {
        ReputationTier::Friendly => Some("Friendly: 10% off"),
        ReputationTier::Beloved => Some("Beloved: 25% off"),
        _ => None,
    }
}

/// Render the commission dialog. Returns the outcome for this frame.
pub fn show_commission_dialog(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    villager_label: &str,
    draft: &mut CommissionDraft,
    plan: Option<&PlanData>,
    held_plan_slots: &[usize],
    registry: &BlockRegistry,
    inventory: &crate::inventory::Inventory,
    reputation_tier: ReputationTier,
    fee_sats: u32,
    player_sats_balance: u64,
    charter_allows_sats: bool,
    policy: &ServerSatsPolicy,
    is_creative: bool,
) -> CommissionDialogOutcome {
    // Backdrop dim.
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("commission_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    let panel_w = 540.0;
    let panel_h = 420.0;
    let origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - panel_h / 2.0,
    );

    let mut outcome = CommissionDialogOutcome::None;
    let rows = plan.map(|p| build_material_rows(p, inventory)).unwrap_or_default();
    let gate = confirm_gate(
        draft,
        plan,
        &rows,
        fee_sats,
        player_sats_balance,
        charter_allows_sats,
        policy,
        is_creative,
    );

    egui::Area::new(egui::Id::new(("commission_dialog", player_index)))
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
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("\u{1F3D7} Commission a build")
                        .size(20.0)
                        .strong()
                        .color(egui::Color32::from_rgb(230, 200, 120)),
                );
                ui.label(
                    egui::RichText::new(villager_label)
                        .size(13.0)
                        .italics()
                        .color(egui::Color32::from_rgb(170, 170, 200)),
                );
                ui.add_space(10.0);
            });

            // ── Plan slot picker ─────────────────────────────────
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Plan:").size(13.0).strong()
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                );
                if held_plan_slots.is_empty() {
                    ui.label(
                        egui::RichText::new("(no Plans in your inventory)")
                            .size(12.0)
                            .italics()
                            .color(egui::Color32::from_rgb(180, 110, 110)),
                    );
                } else {
                    for &slot in held_plan_slots {
                        let selected = draft.plan_hotbar_slot == Some(slot);
                        let label = format!("Slot {}", slot + 1);
                        let btn = egui::Button::new(
                            egui::RichText::new(label).size(12.0),
                        )
                        .selected(selected);
                        if ui.add(btn).clicked() {
                            outcome = CommissionDialogOutcome::PlanSlotChanged(slot);
                        }
                    }
                }
            });
            if let Some(p) = plan {
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "\u{201C}{}\u{201D} \u{2022} {}x{}x{} \u{2022} {} cells",
                            p.name, p.width, p.depth, p.height, p.cells.len()
                        ))
                        .size(12.0)
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                    );
                });
            }
            ui.add_space(10.0);

            // ── Materials required readout ───────────────────────
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Materials required:").size(13.0).strong()
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                );
            });
            if rows.is_empty() && plan.is_some() {
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    ui.label(
                        egui::RichText::new("(none)")
                            .size(12.0)
                            .italics()
                            .color(egui::Color32::from_rgb(170, 170, 180)),
                    );
                });
            }
            for row in &rows {
                ui.horizontal(|ui| {
                    ui.add_space(36.0);
                    let mark = if row.sufficient() { "\u{2713}" } else { "\u{2717}" };
                    let colour = if row.sufficient() {
                        egui::Color32::from_rgb(120, 200, 120)
                    } else {
                        egui::Color32::from_rgb(220, 110, 110)
                    };
                    let name = crate::plan_ui::display_block_name(registry, row.block_id);
                    ui.label(
                        egui::RichText::new(format!(
                            "{mark}  {} \u{00D7} {} (have {})",
                            name, row.needed, row.owned
                        ))
                        .size(12.0)
                        .color(colour),
                    );
                });
            }
            ui.add_space(10.0);

            // ── Build site picker ───────────────────────────────
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Build site:").size(13.0).strong()
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                );
                if let Some([x, y, z]) = draft.site_anchor {
                    let facing = match draft.site_rotations % 4 {
                        0 => "North",
                        1 => "East",
                        2 => "South",
                        _ => "West",
                    };
                    ui.label(
                        egui::RichText::new(format!("({x}, {y}, {z}) facing {facing}"))
                            .size(12.0)
                            .color(egui::Color32::from_rgb(200, 200, 220)),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("(not picked)")
                            .size(12.0)
                            .italics()
                            .color(egui::Color32::from_rgb(170, 170, 180)),
                    );
                }
                let pick_btn = egui::Button::new(
                    egui::RichText::new("Pick build site")
                        .size(12.0)
                        .color(egui::Color32::WHITE),
                )
                .fill(egui::Color32::from_rgb(80, 100, 160));
                if ui.add_enabled(plan.is_some(), pick_btn).clicked() {
                    outcome = CommissionDialogOutcome::PickSite;
                }
            });
            ui.add_space(12.0);

            // ── Fee preview ─────────────────────────────────────
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Fee:").size(13.0).strong()
                        .color(egui::Color32::from_rgb(200, 200, 220)),
                );
                if plan.is_some() {
                    ui.label(
                        egui::RichText::new(format!("{fee_sats} sats"))
                            .size(14.0)
                            .strong()
                            .color(egui::Color32::from_rgb(230, 200, 120)),
                    );
                    if let Some(label) = discount_label(reputation_tier) {
                        ui.label(
                            egui::RichText::new(label)
                                .size(11.0)
                                .italics()
                                .color(egui::Color32::from_rgb(120, 200, 120)),
                        );
                    }
                } else {
                    ui.label(
                        egui::RichText::new("(slot a Plan)")
                            .size(12.0)
                            .italics()
                            .color(egui::Color32::from_rgb(170, 170, 180)),
                    );
                }
            });
            ui.add_space(16.0);

            // ── Confirm / Close buttons ─────────────────────────
            ui.horizontal(|ui| {
                let button_w = 120.0;
                let gap = 12.0;
                let total = button_w * 2.0 + gap;
                ui.add_space((ui.available_width() - total).max(0.0) / 2.0);

                let confirm_btn = egui::Button::new(
                    egui::RichText::new("Confirm")
                        .size(14.0)
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(button_w, 34.0))
                .fill(egui::Color32::from_rgb(70, 150, 90));
                let resp = ui.add_enabled(gate.enabled(), confirm_btn);
                if resp.clicked() {
                    outcome = CommissionDialogOutcome::Confirm;
                }
                if !gate.enabled() {
                    resp.on_hover_text(gate.reason());
                }
                ui.add_space(gap);
                let close_btn = egui::Button::new(
                    egui::RichText::new("Close")
                        .size(14.0)
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(button_w, 34.0))
                .fill(egui::Color32::from_rgb(80, 80, 100));
                if ui.add(close_btn).clicked() {
                    outcome = CommissionDialogOutcome::Closed;
                }
            });
        });
    let _ = block::AIR; // silence unused-import for the registry shape.
    outcome
}

/// Scan the player's 36 inventory slots for hotbar/main-slot indices
/// holding a Plan. Used to populate the dialog's plan-slot picker.
pub fn held_plan_slots(inventory: &crate::inventory::Inventory) -> Vec<usize> {
    (0..36)
        .filter_map(|i| {
            inventory
                .slot(i)
                .and_then(|s| match &s.item {
                    crate::item::Item::Plan(_) => Some(i),
                    _ => None,
                })
        })
        .collect()
}

/// Lookup helper — given a hotbar slot index, return the held Plan's data.
pub fn plan_in_slot(
    inventory: &crate::inventory::Inventory,
    slot: usize,
) -> Option<&PlanData> {
    inventory.slot(slot).and_then(|s| match &s.item {
        crate::item::Item::Plan(p) => Some(p),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;
    use crate::item::{Item, ItemStack};
    use crate::plan::{CapturedCell, PlanData, PlanLicense};

    fn small_plan(cells: Vec<CapturedCell>) -> PlanData {
        let (max_x, max_y, max_z) = cells.iter().fold((0u8, 0u8, 0u8), |acc, c| {
            (acc.0.max(c.rx), acc.1.max(c.ry), acc.2.max(c.rz))
        });
        PlanData {
            version: 1,
            name: "small".into(),
            author_npub: String::new(),
            license: PlanLicense::Ccbysa,
            derivation_chain: Vec::new(),
            is_master: true,
            width: max_x + 1,
            depth: max_z + 1,
            height: max_y + 1,
            cells,
            authored_in: "survival".into(),
            develop_state: crate::plan::DevelopState::Developed,
            kind: crate::plan::PlanKind::Building,
        }
    }

    fn cell(x: u8, y: u8, z: u8, b: BlockId) -> CapturedCell {
        CapturedCell { rx: x, ry: y, rz: z, block_id: b }
    }

    #[test]
    fn material_rows_mark_short_inventory() {
        let plan = small_plan(vec![
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(2, 0, 0, block::DIRT),
        ]);
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 1)));
        let rows = build_material_rows(&plan, &inv);
        let stone = rows.iter().find(|r| r.block_id == block::STONE).unwrap();
        assert_eq!(stone.needed, 2);
        assert_eq!(stone.owned, 1);
        assert!(!stone.sufficient());
        let dirt = rows.iter().find(|r| r.block_id == block::DIRT).unwrap();
        assert!(!dirt.sufficient(), "no dirt should fail sufficiency");
    }

    #[test]
    fn rows_sufficient_short_circuits_creative() {
        let rows = vec![MaterialRow {
            block_id: block::STONE,
            needed: 10,
            owned: 0,
        }];
        assert!(rows_sufficient(&rows, true));
        assert!(!rows_sufficient(&rows, false));
    }

    #[test]
    fn confirm_gate_requires_plan_then_site_then_materials_then_sats() {
        let mut draft = CommissionDraft::new([0, 0, 0]);
        let plan = small_plan(vec![cell(0, 0, 0, block::STONE)]);
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(ItemStack { item: Item::Plan(plan.clone()), count: 1 }));
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();

        // No plan slotted → NoPlan.
        let rows = build_material_rows(&plan, &inv);
        assert_eq!(
            confirm_gate(&draft, Some(&plan), &rows, 100, 1000, true, &policy, false),
            ConfirmGate::NoPlan,
        );

        // Plan slotted, no site → NoSite.
        draft.plan_hotbar_slot = Some(0);
        assert_eq!(
            confirm_gate(&draft, Some(&plan), &rows, 100, 1000, true, &policy, false),
            ConfirmGate::NoSite,
        );

        // Site picked but no stone → InsufficientMaterials.
        draft.site_anchor = Some([10, 64, 10]);
        assert_eq!(
            confirm_gate(&draft, Some(&plan), &rows, 100, 1000, true, &policy, false),
            ConfirmGate::InsufficientMaterials,
        );

        // Add stone, but not enough sats → InsufficientSats.
        inv.set_slot(1, Some(ItemStack::new_block(block::STONE, 5)));
        let rows = build_material_rows(&plan, &inv);
        assert_eq!(
            confirm_gate(&draft, Some(&plan), &rows, 100, 10, true, &policy, false),
            ConfirmGate::InsufficientSats,
        );

        // Enough sats → Ready.
        assert_eq!(
            confirm_gate(&draft, Some(&plan), &rows, 100, 1000, true, &policy, false),
            ConfirmGate::Ready,
        );

        // Charter off → CharterDisabled (overrides everything).
        assert_eq!(
            confirm_gate(&draft, Some(&plan), &rows, 100, 1000, false, &policy, false),
            ConfirmGate::CharterDisabled,
        );

        // Server bitcoin off → ServerDisabled.
        let mut server_off = ServerSatsPolicy::bitcoin_enabled_policy();
        server_off.bitcoin_enabled = false;
        assert_eq!(
            confirm_gate(&draft, Some(&plan), &rows, 100, 1000, true, &server_off, false),
            ConfirmGate::ServerDisabled,
        );
    }

    #[test]
    fn discount_label_returns_some_only_for_friendly_and_beloved() {
        assert!(discount_label(ReputationTier::Neutral).is_none());
        assert!(discount_label(ReputationTier::Hostile).is_none());
        assert!(discount_label(ReputationTier::Wary).is_none());
        assert_eq!(discount_label(ReputationTier::Friendly), Some("Friendly: 10% off"));
        assert_eq!(discount_label(ReputationTier::Beloved), Some("Beloved: 25% off"));
    }

    #[test]
    fn held_plan_slots_lists_only_plan_slots() {
        let plan = small_plan(vec![cell(0, 0, 0, block::STONE)]);
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(block::STONE, 1)));
        inv.set_slot(2, Some(ItemStack { item: Item::Plan(plan.clone()), count: 1 }));
        inv.set_slot(5, Some(ItemStack { item: Item::Plan(plan.clone()), count: 1 }));
        let slots = held_plan_slots(&inv);
        assert_eq!(slots, vec![2, 5]);
    }
}
