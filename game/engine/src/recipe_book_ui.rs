//! Recipe book panel (2026-06-12) — browse the recipe catalogue and
//! click/select a card to auto-fill the crafting grid.
//!
//! The book never crafts: selecting a card returns [`RecipeBookAction::Fill`]
//! with the catalogue index; `game_loop` calls
//! `CraftingUi::autofill_from_example`, which lays the card's `example_grid`
//! into the grid so the real `crafting::match_recipe` produces the result.
//! So nothing here can craft something the engine wouldn't.
//!
//! Three input modes, one surface: mouse/touch click the rows; a controller
//! drives `CraftingUi::book_focus` (set in `game_loop`'s per-frame pad pass)
//! and the gold ring shows the focused row.

use crate::block::BlockRegistry;
use crate::craft_ui::CraftingUi;
use crate::crafting_catalogue::{self, RecipeCategory};
use crate::inventory::Inventory;
use crate::screen::ViewportRect;

const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 220);
const TITLE_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 200, 120);
const TAB_ACTIVE: egui::Color32 = egui::Color32::from_rgb(70, 62, 38);
const TAB_IDLE: egui::Color32 = egui::Color32::from_rgb(30, 32, 40);
const AFFORD: egui::Color32 = egui::Color32::from_rgb(184, 232, 184);
const UNAFFORD: egui::Color32 = egui::Color32::from_rgb(120, 120, 130);
const PAD_FOCUS_BORDER: egui::Color32 = egui::Color32::from_rgb(255, 210, 80);
const USAGE_COLOR: egui::Color32 = egui::Color32::from_rgb(150, 190, 210);
/// Row height for a card with a `usage` line (name + hint sit above it, the
/// usage line sits below — the ingredient grid itself lives in the separate
/// "How to place" guide in `craft_ui.rs`, not this list row).
const ROW_H_WITH_USAGE: f32 = 44.0;
const ROW_H: f32 = 30.0;

/// What the book wants the game loop to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipeBookAction {
    None,
    /// Close the book, back to the grid.
    Close,
    /// Switch the active category tab (index into `RecipeCategory::ALL`).
    SetCategory(usize),
    /// Auto-fill the grid from the catalogue card at this global index.
    Fill(usize),
}

/// Global catalogue indices of the cards visible right now, in display order.
///
/// Precedence: a `uses_filter` ("what can I make with X", #46) constrains the
/// set first; within it (or, without it, across the whole catalogue) a non-empty
/// `search` name-filters, else the active category tab applies. `game_loop`'s
/// pad handler calls this with the same args so `book_focus` lines up with what's
/// drawn.
pub fn visible_indices(category_idx: usize, search: &str, uses_filter: Option<&[usize]>) -> Vec<usize> {
    let q = search.trim().to_lowercase();
    let name_matches = |i: usize| {
        crafting_catalogue::all_cards()[i]
            .name
            .to_lowercase()
            .contains(&q)
    };

    // #46 — uses filter wins: show exactly those cards (name-narrowed if searching).
    if let Some(uses) = uses_filter {
        return uses
            .iter()
            .copied()
            .filter(|&i| q.is_empty() || name_matches(i))
            .collect();
    }

    if !q.is_empty() {
        return crafting_catalogue::all_cards()
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
    }
    let cat = RecipeCategory::ALL[category_idx.min(RecipeCategory::ALL.len() - 1)];
    crafting_catalogue::all_cards()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.category == cat)
        .map(|(i, _)| i)
        .collect()
}

/// Draw the recipe book for one viewport. Returns the first action this
/// frame (mouse clicks win; pad actions come via `game_loop`).
pub fn draw_recipe_book(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    pidx: usize,
    ui_state: &mut CraftingUi,
    inventory: &Inventory,
    registry: &BlockRegistry,
) -> RecipeBookAction {
    let mut action = RecipeBookAction::None;

    // Dim the world behind, per viewport.
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("book_overlay", pidx)),
    ))
    .rect_filled(overlay_rect, 0.0, OVERLAY_BG);

    let panel_w = 560.0_f32.min(viewport.width as f32 - 40.0);
    let panel_origin = egui::pos2(
        viewport.x as f32 + (viewport.width as f32 - panel_w) / 2.0,
        viewport.y as f32 + 24.0,
    );

    let cat_idx = ui_state.book_category.min(RecipeCategory::ALL.len() - 1);

    egui::Area::new(egui::Id::new(("recipe_book", pidx)))
        .fixed_pos(panel_origin)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            ui.vertical(|ui| {
                // Header row: title + close.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("📖 Recipe Book").size(22.0).color(TITLE_COLOR).strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("← Back").size(14.0))
                                    .fill(egui::Color32::from_rgb(40, 30, 30))
                                    .corner_radius(egui::CornerRadius::same(6)),
                            )
                            .clicked()
                        {
                            action = RecipeBookAction::Close;
                        }
                    });
                });
                ui.add_space(6.0);

                // Search row — type to filter recipe names across all
                // categories. egui's TextEdit grabs keyboard focus, so the
                // main.rs `egui_consumed` guard stops keystrokes leaking to
                // gameplay (hotbar number keys etc.).
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🔍").size(15.0));
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut ui_state.book_search)
                            .desired_width(panel_w - 150.0)
                            .hint_text("Search recipes…"),
                    );
                    if ui_state.focus_book_search {
                        resp.request_focus();
                        ui_state.focus_book_search = false;
                    }
                    if !ui_state.book_search.is_empty()
                        && ui.small_button("✕").on_hover_text("Clear search").clicked()
                    {
                        ui_state.book_search.clear();
                    }
                });
                ui.add_space(6.0);

                let searching = !ui_state.book_search.trim().is_empty();

                // Category tabs (wrap on narrow viewports). Dimmed while a
                // search is active; clicking one clears the search.
                ui.horizontal_wrapped(|ui| {
                    for (i, cat) in RecipeCategory::ALL.iter().enumerate() {
                        let active = !searching && i == cat_idx;
                        let btn = egui::Button::new(
                            egui::RichText::new(cat.label())
                                .size(13.0)
                                .color(if active { TITLE_COLOR } else { egui::Color32::from_rgb(170, 170, 180) }),
                        )
                        .fill(if active { TAB_ACTIVE } else { TAB_IDLE })
                        .corner_radius(egui::CornerRadius::same(5));
                        if ui.add(btn).clicked() {
                            ui_state.book_search.clear();
                            action = RecipeBookAction::SetCategory(i);
                        }
                    }
                });
                ui.add_space(8.0);
                ui.separator();

                // Resolve the visible list AFTER the search box so typed text
                // takes effect the same frame.
                let visible = visible_indices(
                    cat_idx,
                    &ui_state.book_search,
                    ui_state.book_uses_filter.as_deref(),
                );
                let focus = ui_state.book_focus.min(visible.len().saturating_sub(1));

                // Card list.
                egui::ScrollArea::vertical()
                    .max_height(viewport.height as f32 - 220.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if visible.is_empty() {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(if searching {
                                    "No recipes match your search."
                                } else {
                                    "No recipes in this category yet."
                                })
                                .size(13.0)
                                .color(UNAFFORD),
                            );
                        }
                        for (row, &gidx) in visible.iter().enumerate() {
                            let card = &crafting_catalogue::all_cards()[gidx];
                            let afford = crafting_catalogue::can_craft(inventory, card);
                            let pad_focused = row == focus && ui_state.book_focus < visible.len();

                            let row_h = if card.usage.is_some() { ROW_H_WITH_USAGE } else { ROW_H };
                            let (rect, resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), row_h),
                                egui::Sense::click(),
                            );
                            let bg = if resp.hovered() || pad_focused {
                                egui::Color32::from_rgb(44, 48, 60)
                            } else {
                                egui::Color32::from_rgba_premultiplied(26, 28, 36, 230)
                            };
                            ui.painter().rect_filled(rect, 4.0, bg);
                            if pad_focused {
                                ui.painter().rect_stroke(
                                    rect,
                                    4.0,
                                    egui::Stroke::new(2.0_f32, PAD_FOCUS_BORDER),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            // Name (afford-coloured) + ingredient hint. A card
                            // with a `usage` line gets the taller row: name +
                            // hint sit on the top line, usage sits under them.
                            let (name_anchor, name_align, hint_anchor, hint_align) =
                                if card.usage.is_some() {
                                    (
                                        rect.left_top() + egui::vec2(10.0, 8.0),
                                        egui::Align2::LEFT_TOP,
                                        rect.right_top() + egui::vec2(-10.0, 8.0),
                                        egui::Align2::RIGHT_TOP,
                                    )
                                } else {
                                    (
                                        rect.left_center() + egui::vec2(10.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        rect.right_center() - egui::vec2(10.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                    )
                                };
                            ui.painter().text(
                                name_anchor,
                                name_align,
                                &card.name,
                                egui::FontId::proportional(14.0),
                                if afford { AFFORD } else { UNAFFORD },
                            );
                            let hint = ingredient_hint(card, registry);
                            ui.painter().text(
                                hint_anchor,
                                hint_align,
                                hint,
                                egui::FontId::proportional(11.0),
                                egui::Color32::from_rgb(120, 122, 132),
                            );
                            if let Some(usage) = card.usage {
                                ui.painter().text(
                                    rect.left_bottom() + egui::vec2(10.0, -8.0),
                                    egui::Align2::LEFT_BOTTOM,
                                    usage,
                                    egui::FontId::proportional(10.5),
                                    USAGE_COLOR,
                                );
                            }
                            if resp.clicked() {
                                action = RecipeBookAction::Fill(gidx);
                            }
                        }
                    });

                ui.add_space(6.0);
                ui.separator();
                ui.label(
                    egui::RichText::new(
                        "D-pad Move · A Show how to place · LB/RB Tabs · B/Y Back   (green = you have the materials)",
                    )
                    .size(11.0)
                    .color(egui::Color32::from_rgb(110, 110, 120)),
                );
            });
        });

    action
}

/// A compact "3× Iron Ingot · 2× Stick" style hint for the card's bill.
fn ingredient_hint(card: &crafting_catalogue::RecipeCard, registry: &BlockRegistry) -> String {
    card.ingredients
        .iter()
        .map(|ing| {
            let label = match &ing.fuzzy {
                Some(crafting_catalogue::FuzzyKind::Log) => "Any Log".to_string(),
                Some(crafting_catalogue::FuzzyKind::Plank) => "Any Plank".to_string(),
                Some(crafting_catalogue::FuzzyKind::Paper) => "Any Paper".to_string(),
                None => ing.item.name(registry),
            };
            format!("{}× {}", ing.count, label)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
