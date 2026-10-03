//! Spec 21 Phases 4-8 — Vendor Block egui overlay.
//!
//! Right-click on a Vendor Block routes to one of two views based on
//! whether the player is the owner:
//! - **Owner view**: mode picker (Sell / Buy / Barter) + slot item +
//!   price +/- + stock readout + Withdraw button.
//! - **Buyer view**: read-only offer + Buy / Barter button.
//!
//! Charter gating (Phase 8): the mode picker greys out Sell + Buy
//! when the player's `charter_allows_sats` is false; Barter is always
//! available regardless of Charter or per-server policy.

use crate::block::BlockRegistry;
use crate::egui_integration::EguiIntegration;
use crate::item::Item;
use crate::vendor::{
    plan_listing_allowed, slot_accepts_for_mode, BuyRefusal, VendorData, VendorMode,
    UNLIMITED_STOCK,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VendorUiOutcome {
    InProgress,
    Closed,
    /// Owner clicked one of the mode buttons. Caller updates
    /// `VendorData.mode`. Charter-disallowed modes are filtered at
    /// the UI level so the click never fires for them.
    OwnerPickMode(VendorMode),
    /// Owner clicked +1 / -1 on price (Buy/Sell only).
    OwnerPriceAdjust(i32),
    /// Owner clicked the Slot button while holding an item — caller
    /// moves one of the held item into the slot.
    OwnerSlotDeposit,
    /// Owner clicked the Slot button while NOT holding an item —
    /// caller removes the slot back into inventory.
    OwnerSlotWithdraw,
    /// Owner clicked Withdraw — caller takes the escrow_sats into
    /// the player's wallet (mocked on alpha).
    OwnerWithdrawEscrow,
    /// Buyer clicked the Buy button. Caller runs the transaction
    /// (via `vendor::try_buy`) and toasts the refusal if any.
    BuyerBuy,
    /// Spec 25 Phase 3 — owner clicked +/- on the Licence stock
    /// counter. Caller adjusts `VendorData.stock` by the i32 delta.
    /// Master listings ignore this outcome (stock fixed at 1).
    OwnerStockAdjust(i32),
    /// Spec 25 Phase 3 — owner clicked the "Unlimited" pill on a
    /// Licence listing. Caller sets `VendorData.stock = UNLIMITED_STOCK`.
    OwnerSetUnlimitedStock,
    /// Spec 25 Phase 3 — owner attempted to list a Licence-tier plan
    /// at Master tier. Caller toasts a refusal. Held to make the
    /// invalid-listing message a UI concern rather than a silent drop.
    OwnerPlanTierViolation,
    /// Spec 40 — owner clicked one of the Bulk lot-size buttons. Caller
    /// writes the value to `VendorData.lot_size` after validating it's
    /// in `BULK_LOT_SIZES`.
    OwnerSetLotSize(u32),
}

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 180);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 195, 100);
const DIM_COLOR: egui::Color32 = egui::Color32::LIGHT_GRAY;
/// Spec 25 Phase 4 — Master-tier plan listing header tint. Distinct
/// from the default `TITLE_COLOR` (which is warm) so the player can
/// see "this is a one-of-a-kind sale" at a glance.
const MASTER_GOLD: egui::Color32 = egui::Color32::from_rgb(255, 215, 0);
/// Spec 25 Phase 4 — Licence-tier plan listing header tint. Cool
/// silver, paired against the warm gold so Master vs Licence reads
/// instantly without needing to scan the text.
const LICENCE_SILVER: egui::Color32 = egui::Color32::from_rgb(200, 200, 220);

/// Render the vendor dialog. `is_owner` selects the owner vs buyer
/// view. `charter_allows_sats` greys Sell/Buy when false (Barter
/// always available). `raid_supplies_highlight` is the Spec 22
/// Phase 17 cross-economy hook — caller passes `true` for vendors
/// near a raid-warned village whose listing matches the raid-supplies
/// whitelist; the buyer view prepends a 🛡 badge to the listing.
/// Returns the outcome of the player's click; the caller drives state
/// mutation from there.
pub fn draw_vendor_ui(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    is_owner: bool,
    charter_allows_sats: bool,
    bitcoin_enabled: bool,
    data: &VendorData,
    held_item_label: Option<&str>,
    held_stack: Option<&crate::item::ItemStack>,
    registry: &BlockRegistry,
    _egui_integration: &EguiIntegration,
    raid_supplies_highlight: bool,
) -> VendorUiOutcome {
    let mut outcome = VendorUiOutcome::InProgress;

    // Dim background
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("vendor_overlay", player_index)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 200.0,
        viewport.y as f32 + 70.0,
    );
    egui::Area::new(egui::Id::new(("vendor_panel", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(400.0);
            ui.set_max_width(400.0);
            ui.vertical_centered(|ui| {
                let title = if is_owner { "Vendor — Owner View" } else { "Vendor" };
                ui.label(
                    egui::RichText::new(title)
                        .size(22.0)
                        .color(TITLE_COLOR)
                        .strong(),
                );
                ui.add_space(8.0);

                if is_owner {
                    draw_owner_view(ui, charter_allows_sats, bitcoin_enabled, data, held_item_label, held_stack, registry, &mut outcome);
                } else {
                    draw_buyer_view(ui, data, registry, charter_allows_sats, bitcoin_enabled, raid_supplies_highlight, &mut outcome);
                }

                ui.add_space(12.0);
                if ui.button("Close").clicked() {
                    outcome = VendorUiOutcome::Closed;
                }
            });
        });

    // Esc closes.
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = VendorUiOutcome::Closed;
    }

    outcome
}

/// The modes the owner's picker offers. Sats modes appear only when
/// [`crate::economy::sats_ui_visible`] (review W4 S4); otherwise Barter is
/// the only choice. NOTE: Buy mode (vendor buys FROM the player) is
/// intentionally omitted pending a correct implementation — the inline
/// version handed the buyer a free copy of the wanted item and consumed
/// nothing (item-dup), and a correct Buy needs the sats-settlement layer.
/// `try_buy` also refuses Buy as a safety net. (Pre-alpha bug hunt 2026-06-22.)
pub fn owner_mode_choices(sats_visible: bool) -> Vec<VendorMode> {
    [
        VendorMode::Sell,
        VendorMode::Barter,
        VendorMode::SellPlanMaster,
        VendorMode::SellPlanLicence,
        VendorMode::Bulk,
    ]
    .into_iter()
    .filter(|m| sats_visible || !m.requires_sats())
    .collect()
}

fn draw_owner_view(
    ui: &mut egui::Ui,
    charter_allows_sats: bool,
    bitcoin_enabled: bool,
    data: &VendorData,
    held_item_label: Option<&str>,
    held_stack: Option<&crate::item::ItemStack>,
    registry: &BlockRegistry,
    outcome: &mut VendorUiOutcome,
) {
    let sats_visible = crate::economy::sats_ui_visible(bitcoin_enabled, charter_allows_sats);
    ui.horizontal(|ui| {
        for mode in owner_mode_choices(sats_visible) {
            let selected = data.mode == Some(mode);
            let label = if selected {
                format!("[{}]", mode.label())
            } else {
                mode.label().to_string()
            };
            if ui.button(label).clicked() {
                *outcome = VendorUiOutcome::OwnerPickMode(mode);
            }
        }
    });
    ui.add_space(6.0);

    let mode = data.mode;
    let is_plan_mode = mode.is_some_and(|m| m.is_plan_mode());

    // Slot display
    let slot_label = format_slot(data.slot.as_ref(), registry);
    ui.label(format!("Slot: {slot_label}"));

    // Plan-mode slot picker filters to Item::Plan(_); other modes
    // accept anything that isn't a plan. The button only fires when
    // the (mode, held-stack) combination is legal — illegal combos
    // render a disabled button with a tooltip explaining why.
    let deposit_allowed = match (mode, held_stack) {
        (Some(m), Some(stack)) => slot_accepts_for_mode(m, stack),
        _ => held_stack.is_some(),
    };
    if let Some(label) = held_item_label {
        let mut deposit_button_label = format!("Deposit {label}");
        // Spec 25 — surface the tier check at button-render time so
        // the player sees the refusal BEFORE they click. Licence-tier
        // plans can't be listed at Master tier.
        let mut blocked_reason: Option<&'static str> = None;
        if mode == Some(VendorMode::SellPlanMaster)
            && let Some(stack) = held_stack
                && let Item::Plan(p) = &stack.item
                    && !plan_listing_allowed(VendorMode::SellPlanMaster, p) {
                        blocked_reason = Some("Only Master plans can be listed at Master tier");
                        deposit_button_label = "Deposit (Licence — not allowed)".to_string();
                    }
        if !deposit_allowed && blocked_reason.is_none() {
            blocked_reason = if is_plan_mode {
                Some("Plan-listing modes only accept Plan items")
            } else {
                Some("This mode doesn't accept plans")
            };
        }
        let btn = egui::Button::new(deposit_button_label);
        let resp = ui.add_enabled(blocked_reason.is_none(), btn);
        let clicked = resp.clicked();
        match (blocked_reason, clicked) {
            (None, true) => *outcome = VendorUiOutcome::OwnerSlotDeposit,
            (Some(reason), clicked) => {
                resp.on_hover_text(reason);
                if clicked {
                    // Defensive — disabled buttons don't click in
                    // egui today, but if that ever changes the
                    // caller toasts the tier violation rather than
                    // silently mutating state.
                    *outcome = VendorUiOutcome::OwnerPlanTierViolation;
                }
            }
            _ => {}
        }
    } else if ui.button("Withdraw from slot").clicked() {
        *outcome = VendorUiOutcome::OwnerSlotWithdraw;
    }
    ui.add_space(6.0);

    // Price +/- for any sats-priced mode (Sell, Buy, both plan modes,
    // plus Bulk — where the field stores the per-unit price).
    if sats_visible
        && matches!(
            mode,
            Some(VendorMode::Sell)
                | Some(VendorMode::Buy)
                | Some(VendorMode::SellPlanMaster)
                | Some(VendorMode::SellPlanLicence)
                | Some(VendorMode::Bulk)
        )
    {
        ui.horizontal(|ui| {
            let price_label = if mode == Some(VendorMode::Bulk) {
                format!("Per-unit price: {} sats", data.price_sats)
            } else {
                format!("Price: {} sats", data.price_sats)
            };
            ui.label(price_label);
            if ui.button("-1").clicked() && data.price_sats > 0 {
                *outcome = VendorUiOutcome::OwnerPriceAdjust(-1);
            }
            if ui.button("+1").clicked() {
                *outcome = VendorUiOutcome::OwnerPriceAdjust(1);
            }
            if ui.button("+10").clicked() {
                *outcome = VendorUiOutcome::OwnerPriceAdjust(10);
            }
            if ui.button("+100").clicked() {
                *outcome = VendorUiOutcome::OwnerPriceAdjust(100);
            }
        });
    }

    // Spec 40 — Bulk-only lot-size picker. Four fixed buttons matching
    // `BULK_LOT_SIZES`; the selected lot is bracketed. Total-price
    // hint shows `lot_size × price_sats` so the owner can sanity-check
    // the per-lot price as they tune the per-unit value.
    if mode == Some(VendorMode::Bulk) {
        ui.horizontal(|ui| {
            ui.label("Lot size:");
            for &lot in crate::vendor::BULK_LOT_SIZES.iter() {
                let selected = data.lot_size == lot;
                let label = if selected { format!("[{lot}]") } else { lot.to_string() };
                if ui.button(label).clicked() {
                    *outcome = VendorUiOutcome::OwnerSetLotSize(lot);
                }
            }
        });
        let lot = data.lot_size.max(crate::vendor::DEFAULT_LOT_SIZE);
        let total = (data.price_sats as u64).saturating_mul(lot as u64);
        if sats_visible {
            ui.label(format!("Per-lot total: {total} sats ({lot} × {} sats)", data.price_sats));
        }
    }

    // Stock counter (plan-Licence mode only — Master listings are
    // fixed at 1 by spec). For Licence, surface the per-copy +/-
    // controls plus the Unlimited pill.
    match mode {
        Some(VendorMode::SellPlanLicence) => {
            ui.horizontal(|ui| {
                let stock_label = if data.stock == UNLIMITED_STOCK {
                    "Stock: Unlimited".to_string()
                } else {
                    format!("Stock: {}", data.stock)
                };
                ui.label(stock_label);
                if ui.button("-1").clicked() && data.stock != UNLIMITED_STOCK && data.stock > 0 {
                    *outcome = VendorUiOutcome::OwnerStockAdjust(-1);
                }
                if ui.button("+1").clicked() && data.stock != UNLIMITED_STOCK {
                    *outcome = VendorUiOutcome::OwnerStockAdjust(1);
                }
                if ui.button("+5").clicked() && data.stock != UNLIMITED_STOCK {
                    *outcome = VendorUiOutcome::OwnerStockAdjust(5);
                }
                let pill_label = if data.stock == UNLIMITED_STOCK {
                    "[Unlimited]"
                } else {
                    "Unlimited"
                };
                if ui.button(pill_label).clicked() {
                    *outcome = VendorUiOutcome::OwnerSetUnlimitedStock;
                }
            });
        }
        Some(VendorMode::SellPlanMaster) => {
            ui.label("Stock: 1 (one-time sale)");
        }
        _ => {
            ui.label(format!("Stock: {}", data.stock));
        }
    }

    // Escrow
    if sats_visible && data.escrow_sats > 0 {
        ui.horizontal(|ui| {
            ui.label(format!("Escrow: {} sats", data.escrow_sats));
            if ui.button("Withdraw").clicked() {
                *outcome = VendorUiOutcome::OwnerWithdrawEscrow;
            }
        });
    }
}

fn draw_buyer_view(
    ui: &mut egui::Ui,
    data: &VendorData,
    registry: &BlockRegistry,
    charter_allows_sats: bool,
    bitcoin_enabled: bool,
    raid_supplies_highlight: bool,
    outcome: &mut VendorUiOutcome,
) {
    let mode = match data.mode {
        Some(m) => m,
        None => {
            ui.label(egui::RichText::new("(vendor not set up)").color(DIM_COLOR));
            return;
        }
    };
    // No sats UI unless sats are on for this server AND player (review W4
    // S4; never on web): a sats listing is shut.
    if mode.requires_sats()
        && !crate::economy::sats_ui_visible(bitcoin_enabled, charter_allows_sats)
    {
        ui.label(egui::RichText::new("(this stall isn't trading here)").color(DIM_COLOR));
        return;
    }
    // Spec 22 Phase 17 — raid-supplies badge sits above the listing
    // card so the buyer sees the "useful for the raid" cue first.
    // Red-orange tint matches the raid-bounty banner colour family.
    if raid_supplies_highlight {
        ui.label(
            egui::RichText::new("\u{1F6E1}  Raid Supplies")
                .size(15.0)
                .strong()
                .color(egui::Color32::from_rgb(220, 110, 80)),
        );
        ui.add_space(4.0);
    }
    let slot_label = format_slot(data.slot.as_ref(), registry);
    let req_label = format_slot(data.barter_request.as_ref(), registry);
    match mode {
        VendorMode::Sell => {
            ui.label(format!("For sale: {slot_label}"));
            ui.label(format!("Price: {} sats", data.price_sats));
            ui.label(format!("Stock: {}", data.stock));
            let refusal = preview_refusal(data, mode, charter_allows_sats, bitcoin_enabled);
            let enabled = refusal.is_none();
            let btn = egui::Button::new("Buy");
            let resp = ui.add_enabled(enabled, btn);
            if resp.clicked() {
                *outcome = VendorUiOutcome::BuyerBuy;
            }
            if let Some(r) = refusal {
                resp.on_hover_text(r.message());
            }
        }
        VendorMode::Buy => {
            ui.label(format!("Wants to buy: {slot_label}"));
            ui.label(format!("Pays: {} sats", data.price_sats));
            ui.label(format!("Slots remaining: {}", data.stock));
            let refusal = preview_refusal(data, mode, charter_allows_sats, bitcoin_enabled);
            let enabled = refusal.is_none();
            let btn = egui::Button::new("Sell to vendor");
            let resp = ui.add_enabled(enabled, btn);
            if resp.clicked() {
                *outcome = VendorUiOutcome::BuyerBuy;
            }
            if let Some(r) = refusal {
                resp.on_hover_text(r.message());
            }
        }
        VendorMode::Barter => {
            ui.label(format!("Offers: {slot_label}"));
            ui.label(format!("Wants: {req_label}"));
            ui.label(format!("Stock: {}", data.stock));
            let refusal = preview_refusal(data, mode, charter_allows_sats, bitcoin_enabled);
            let enabled = refusal.is_none();
            let btn = egui::Button::new("Barter");
            let resp = ui.add_enabled(enabled, btn);
            if resp.clicked() {
                *outcome = VendorUiOutcome::BuyerBuy;
            }
            if let Some(r) = refusal {
                resp.on_hover_text(r.message());
            }
        }
        VendorMode::SellPlanMaster | VendorMode::SellPlanLicence => {
            draw_plan_buyer_view(ui, data, mode, registry, charter_allows_sats, bitcoin_enabled, outcome);
        }
        VendorMode::Bulk => {
            // Spec 40 — Bulk buyer view. Displays "Buy {lot} × {item}
            // for {total} sats" so the player sees the full transaction
            // at a glance. Stock < lot_size → button greyed; the
            // refusal tooltip points at OutOfStock.
            let lot_size = data.lot_size.max(crate::vendor::DEFAULT_LOT_SIZE);
            let total_price = (data.price_sats as u64) * (lot_size as u64);
            ui.label(format!("For sale (bulk): {slot_label}"));
            ui.label(format!(
                "Lot: {lot_size} × {slot_label} — {total_price} sats ({} sats / unit)",
                data.price_sats
            ));
            ui.label(format!("Stock: {} units", data.stock));
            let refusal = preview_refusal(data, mode, charter_allows_sats, bitcoin_enabled);
            let enabled = refusal.is_none() && data.stock >= lot_size;
            let btn = egui::Button::new(format!("Buy {lot_size}"));
            let resp = ui.add_enabled(enabled, btn);
            if resp.clicked() {
                *outcome = VendorUiOutcome::BuyerBuy;
            }
            if let Some(r) = refusal {
                resp.on_hover_text(r.message());
            } else if data.stock < lot_size {
                resp.on_hover_text(crate::vendor::BuyRefusal::OutOfStock.message());
            }
        }
    }
}

/// Spec 25 Phase 4 — buyer-side render for a plan-listing Vendor
/// Block. Distinct from the generic Sell/Buy/Barter render because
/// plans carry rich metadata (footprint preview, material summary,
/// author credit, derivation chain) that the regular slot-label
/// format can't express. The Master/Licence tier is reflected in the
/// header tint so the player can tell at a glance which kind of sale
/// this is.
fn draw_plan_buyer_view(
    ui: &mut egui::Ui,
    data: &VendorData,
    mode: VendorMode,
    registry: &BlockRegistry,
    charter_allows_sats: bool,
    bitcoin_enabled: bool,
    outcome: &mut VendorUiOutcome,
) {
    let (header_text, header_tint) = match mode {
        VendorMode::SellPlanMaster => ("Master Plan", MASTER_GOLD),
        VendorMode::SellPlanLicence => ("Licence Plan", LICENCE_SILVER),
        _ => ("Plan", TITLE_COLOR),
    };
    ui.label(
        egui::RichText::new(header_text)
            .size(18.0)
            .color(header_tint)
            .strong(),
    );

    // Read the plan out of the slot; defensive on a malformed listing.
    let plan = match data.slot.as_ref().and_then(|s| match &s.item {
        Item::Plan(p) => Some(p),
        _ => None,
    }) {
        Some(p) => p,
        None => {
            ui.label(egui::RichText::new("(plan slot empty)").color(DIM_COLOR));
            return;
        }
    };

    // Name + author credit. The chain's [0] entry is the root author;
    // empty `author_npub` is the v1 default before Signet wiring lands.
    ui.label(egui::RichText::new(format!("Name: {}", plan.name)).strong());
    let author_label = plan
        .derivation_chain
        .first()
        .map(|link| link.author_npub.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Anonymous".to_string());
    ui.label(format!("Author: {author_label}"));
    ui.label(format!(
        "Footprint: {} × {}, height {}",
        plan.width, plan.depth, plan.height
    ));

    // ASCII top-down preview.
    let footprint = crate::plan_ui::ascii_footprint(plan);
    if !footprint.is_empty() {
        ui.add_space(4.0);
        ui.monospace(footprint);
        ui.add_space(4.0);
    }

    // Material summary — render the first 5 rows so a 20-block-type
    // plan doesn't blow the dialog out vertically. The Inspect dialog
    // is where the full list lives; this is the buyer's at-a-glance.
    let materials = crate::plan_ui::material_summary(plan);
    let to_show = materials.iter().take(5).collect::<Vec<_>>();
    if !to_show.is_empty() {
        ui.label(egui::RichText::new("Materials").strong());
        for (block_id, count) in to_show {
            ui.label(format!(
                "  {} × {}",
                count,
                crate::plan_ui::display_block_name(registry, *block_id),
            ));
        }
        if materials.len() > 5 {
            ui.label(format!("  …and {} more", materials.len() - 5));
        }
    }

    // Stock + price summary. Unlimited renders as the word.
    let stock_str = if data.stock == UNLIMITED_STOCK {
        "Unlimited".to_string()
    } else {
        data.stock.to_string()
    };
    ui.add_space(4.0);
    ui.label(format!("Price: {} sats", data.price_sats));
    ui.label(format!("Stock: {stock_str}"));

    // Buy button — same refusal-preview logic as the other modes,
    // plus a specific Charter-disabled tooltip to match the spec
    // wording ("Bitcoin disabled for this account").
    let refusal = preview_refusal(data, mode, charter_allows_sats, bitcoin_enabled);
    let enabled = refusal.is_none();
    let btn_label = match mode {
        VendorMode::SellPlanMaster => "Buy (Master)",
        VendorMode::SellPlanLicence => "Buy (Licence)",
        _ => "Buy",
    };
    let btn = egui::Button::new(btn_label);
    let resp = ui.add_enabled(enabled, btn);
    if resp.clicked() {
        *outcome = VendorUiOutcome::BuyerBuy;
    }
    if let Some(r) = refusal {
        // Honour the spec's exact wording for the Charter refusal so
        // the buyer-side dialog matches the Plaque-tip Charter tooltip.
        let tooltip = match r {
            BuyRefusal::CharterDeny => "Bitcoin disabled for this account",
            _ => r.message(),
        };
        resp.on_hover_text(tooltip);
    }
}

/// Pure helper: render a slot stack as a human-readable label.
fn format_slot(stack: Option<&crate::item::ItemStack>, registry: &BlockRegistry) -> String {
    match stack {
        None => "(empty)".to_string(),
        Some(s) => {
            let label = match &s.item {
                Item::Material(m) => format!("{:?}", m),
                Item::Block(b) => registry.get(*b).name.to_string(),
                Item::Tool(_) => "tool".to_string(),
                Item::Plan(_) => "plan".to_string(),
                Item::Armour(_) => "armour".to_string(),
            };
            if s.count > 1 { format!("{label} ×{}", s.count) } else { label }
        }
    }
}

/// Re-export of `crate::vendor::preview_refusal` so the rest of the UI
/// module + its tests keep their existing import path. The canonical
/// implementation moved into `vendor.rs` in Spec 40 Phase 2 so
/// `vendor::try_buy` can reuse the same gates without an `vendor_ui`
/// dep.
pub use crate::vendor::preview_refusal;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::VendorData;

    /// Review W4 S4 — the owner's picker offers no sats mode unless sats are
    /// on for the server AND the player (and never on web).
    #[test]
    fn owner_picker_hides_sats_modes_unless_sats_are_on() {
        let pick = |btc, charter| {
            owner_mode_choices(crate::economy::sats_ui_visible(btc, charter))
        };
        for (btc, charter) in [(false, false), (true, false), (false, true)] {
            assert_eq!(pick(btc, charter), vec![VendorMode::Barter], "btc={btc} charter={charter}");
        }
        let on = pick(true, true);
        if cfg!(target_arch = "wasm32") {
            assert_eq!(on, vec![VendorMode::Barter]);
        } else {
            assert!(on.contains(&VendorMode::Sell) && on.contains(&VendorMode::Bulk));
            assert!(!on.contains(&VendorMode::Buy), "Buy stays omitted");
        }
    }

    #[test]
    fn preview_refusal_passes_when_all_clear() {
        let mut d = VendorData::default();
        d.stock = 5;
        d.mode = Some(VendorMode::Sell);
        d.price_sats = 10;
        let r = preview_refusal(&d, VendorMode::Sell, true, true);
        assert_eq!(r, None);
    }

    #[test]
    fn preview_refusal_charter_blocks_sats_mode() {
        let mut d = VendorData::default();
        d.stock = 5;
        let r = preview_refusal(&d, VendorMode::Sell, false, true);
        assert_eq!(r, Some(BuyRefusal::CharterDeny));
        // Barter is unaffected by Charter — stock present, no refusal.
        let r2 = preview_refusal(&d, VendorMode::Barter, false, true);
        assert_eq!(r2, None);
    }

    #[test]
    fn preview_refusal_bitcoin_off_blocks_sats_mode() {
        let mut d = VendorData::default();
        d.stock = 5;
        let r = preview_refusal(&d, VendorMode::Sell, true, false);
        assert_eq!(r, Some(BuyRefusal::BitcoinDisabled));
    }

    #[test]
    fn preview_refusal_out_of_stock_fires() {
        let d = VendorData::default(); // stock = 0
        let r = preview_refusal(&d, VendorMode::Barter, true, true);
        assert_eq!(r, Some(BuyRefusal::OutOfStock));
    }
}
