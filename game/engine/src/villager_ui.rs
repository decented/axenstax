//! Spec 19 phase 5 — villager dialogue overlay.
//!
//! When a player right-clicks a Villager (or Peddler) within
//! `INTERACT_RANGE`, the engine opens a modal egui dialogue showing the
//! villager's profession and the current quest on offer. Three actions:
//! **Accept** (Phase 6 binds the quest to the player), **Decline** (puts the
//! quest on a 5-minute cooldown), **Close** (dismisses without committing).
//!
//! This module owns just the UI; the quest data model lives in `quest.rs`
//! (Phase 6). The dialogue itself only needs a description string + a
//! per-action callback flag.


use crate::villager::{Profession, VillagerComponent};

/// Maximum distance (blocks) at which a right-click can open the dialogue.
pub const INTERACT_RANGE: f32 = 4.0;

/// 5-minute decline cooldown @ 20 TPS.
pub const DECLINE_COOLDOWN_TICKS: u64 = 6000;

/// Action returned by the dialogue render call. Phase 6 wires `Accept` /
/// `Decline`; Phase 7 wires `TurnIn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogueAction {
    None,
    Accept,
    Decline,
    TurnIn,
    Close,
}

/// Which control set the dialogue is in. The game loop computes this before
/// rendering based on the (player, villager) quest state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogueMode {
    /// No quest taken — show Accept / Decline.
    Offer,
    /// Active quest in progress — show Close only.
    InProgress,
    /// Active quest completed — show Turn-in.
    ReadyToTurnIn,
}

/// Render the villager dialogue overlay for the given player. Returns the
/// action the player picked this frame (if any). The caller is responsible
/// for translating `Accept` / `Decline` into state changes; the UI is pure.
///
/// `quest_summary` is the human-readable one-liner of the current quest
/// offer (e.g. "Bring 5 Wheat — pays 1 Bread + 12 sats"). Phase 6 generates
/// these from `QuestFlavour`; Phase 5 passes a placeholder string.
///
/// `defender_leaderboard` is the Spec 22 Phase 18 hook — caller passes a
/// pre-formatted single line listing top defenders ("Heroes here:
/// Player 0 (14 kills), Player 1 (9 kills)") when this villager's village
/// has non-zero raid-kill totals. None when the village has never been
/// defended.
pub fn draw_villager_dialogue(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    villager_label: &str,
    profession: Profession,
    quest_summary: Option<&str>,
    gossip_line: Option<&str>,
    mode: DialogueMode,
    raid_banner: Option<&str>,
    defender_leaderboard: Option<&str>,
) -> DialogueAction {
    // Dim the world behind the dialogue with a Background-order layer
    // (same pattern as the crafting UI overlay after the Spec 19 fix).
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("villager_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    // Centred dialogue panel: 480×220.
    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - 240.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - 110.0,
    );
    let mut action = DialogueAction::None;
    egui::Area::new(egui::Id::new(("villager_dialogue", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            // Pin both min + max width (Spec 5 §3.6 rendering pitfall — pin
            // width before children layout, otherwise vertical_centered's
            // children collapse).
            ui.set_min_width(480.0);
            ui.set_max_width(480.0);

            // Panel background.
            let bg_rect = egui::Rect::from_min_size(
                panel_origin,
                egui::vec2(480.0, 220.0),
            );
            ui.painter().rect_filled(
                bg_rect,
                6.0,
                egui::Color32::from_rgb(28, 30, 40),
            );
            ui.painter().rect_stroke(
                bg_rect,
                6.0,
                egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(120, 170, 255)),
                egui::StrokeKind::Inside,
            );

            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                // Villager name (Phase 13 may swap in nicer names from
                // Axolittle's playtest).
                ui.label(
                    egui::RichText::new(villager_label)
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(220, 200, 120)),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(profession.name())
                        .size(15.0)
                        .italics()
                        .color(egui::Color32::from_rgb(170, 170, 200)),
                );
                // Spec 22 Phase 9 — raid bounty banner sits at the
                // top of the dialogue so the kid sees the offer
                // before the routine quest summary. Red tint so it
                // reads as urgent.
                if let Some(banner) = raid_banner {
                    ui.add_space(10.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(banner)
                                .size(14.0)
                                .strong()
                                .color(egui::Color32::from_rgb(220, 110, 80)),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(18.0);

                // Quest preview line. Wrapping protects against unusually long
                // summaries (Phase 6 keeps them under ~340 px today but T1.5
                // adds longer item names; the panel is 480 wide so wrap fits
                // two lines comfortably).
                let preview = quest_summary.unwrap_or(
                    "No quest right now. Come back when you've helped others \
                     in the village.",
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(preview)
                            .size(15.0)
                            .color(egui::Color32::from_rgb(220, 220, 220)),
                    )
                    .wrap(),
                );

                // Spec 19 gossip-extension line. Italicised + dimmer than the
                // quest text so the player reads it as ambient flavour, not as
                // a quest objective. Only render when set — a freshly-spawned
                // villager has gossip_line=None for ~5 min of game time before
                // the daily-tick refresh fires.
                if let Some(line) = gossip_line {
                    ui.add_space(10.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("\u{201C}{line}\u{201D}"))
                                .size(13.0)
                                .italics()
                                .color(egui::Color32::from_rgb(160, 160, 180)),
                        )
                        .wrap(),
                    );
                }

                // Spec 22 Phase 18 — defender leaderboard line. Pure
                // social signal; renders below the gossip line in a
                // distinct warm-gold tint so the player reads it as
                // an honour roll, not as ambient flavour. Only when
                // the village has actually been defended at least
                // once.
                if let Some(roll) = defender_leaderboard {
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(roll)
                                .size(13.0)
                                .color(egui::Color32::from_rgb(220, 200, 120)),
                        )
                        .wrap(),
                    );
                }

                ui.add_space(20.0);

                ui.horizontal(|ui| {
                    // Centre the buttons inside the 480-wide panel. Button
                    // set depends on the dialogue mode.
                    let button_w = 100.0;
                    let gap = 10.0;
                    let n_buttons: u32 = match mode {
                        DialogueMode::Offer => 3,
                        DialogueMode::InProgress => 1,
                        DialogueMode::ReadyToTurnIn => 2,
                    };
                    let total = button_w * n_buttons as f32
                        + gap * (n_buttons.saturating_sub(1) as f32);
                    ui.add_space((ui.available_width() - total).max(0.0) / 2.0);

                    match mode {
                        DialogueMode::Offer => {
                            let accept = egui::Button::new(
                                egui::RichText::new("Accept")
                                    .size(15.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(button_w, 32.0))
                            .fill(egui::Color32::from_rgb(60, 140, 70));
                            if ui.add_enabled(quest_summary.is_some(), accept).clicked() {
                                action = DialogueAction::Accept;
                            }
                            ui.add_space(gap);

                            let decline = egui::Button::new(
                                egui::RichText::new("Decline")
                                    .size(15.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(button_w, 32.0))
                            .fill(egui::Color32::from_rgb(140, 80, 60));
                            if ui.add_enabled(quest_summary.is_some(), decline).clicked() {
                                action = DialogueAction::Decline;
                            }
                            ui.add_space(gap);

                            let close = egui::Button::new(
                                egui::RichText::new("Close")
                                    .size(15.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(button_w, 32.0))
                            .fill(egui::Color32::from_rgb(80, 80, 100));
                            if ui.add(close).clicked() {
                                action = DialogueAction::Close;
                            }
                        }
                        DialogueMode::InProgress => {
                            let close = egui::Button::new(
                                egui::RichText::new("Close")
                                    .size(15.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(button_w, 32.0))
                            .fill(egui::Color32::from_rgb(80, 80, 100));
                            if ui.add(close).clicked() {
                                action = DialogueAction::Close;
                            }
                        }
                        DialogueMode::ReadyToTurnIn => {
                            let turn_in = egui::Button::new(
                                egui::RichText::new("Turn in")
                                    .size(15.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(button_w, 32.0))
                            .fill(egui::Color32::from_rgb(70, 150, 90));
                            if ui.add(turn_in).clicked() {
                                action = DialogueAction::TurnIn;
                            }
                            ui.add_space(gap);
                            let close = egui::Button::new(
                                egui::RichText::new("Close")
                                    .size(15.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(button_w, 32.0))
                            .fill(egui::Color32::from_rgb(80, 80, 100));
                            if ui.add(close).clicked() {
                                action = DialogueAction::Close;
                            }
                        }
                    }
                });
            });
        });

    action
}

/// Render Satoshi's warm, fully-scripted onboarding dialogue. Returns the
/// action the player picked this frame. Pure UI — the caller applies state
/// changes. `nudge_labels` are the play-nudge options (used only in the `Nudge`
/// view; pass an empty slice otherwise). Never mentions sats/earning/money;
/// always offers a single-input dismissal and never seizes the player's input.
pub fn draw_satoshi_dialogue(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    view: crate::satoshi::SatoshiView,
    is_creative: bool,
) -> crate::satoshi::SatoshiAction {
    use crate::satoshi::{self, SatoshiAction, SatoshiView};

    // Dim the world behind the dialogue (same pattern as the villager overlay).
    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("satoshi_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    // Primary line + an optional italic follow-up.
    let (body, secondary): (&str, Option<&str>) = match view {
        SatoshiView::Greeting => (satoshi::SATOSHI_GREETING, Some(satoshi::SATOSHI_GIFT_LINE)),
        SatoshiView::SchematicOffer => {
            if is_creative {
                (satoshi::SATOSHI_CREATIVE_GREETING, Some(satoshi::SATOSHI_SCHEMATIC_LINE))
            } else {
                (satoshi::SATOSHI_SCHEMATIC_LINE, None)
            }
        }
        SatoshiView::Idle => (satoshi::SATOSHI_RETURN_LINE, None),
    };

    let panel_w = 480.0;
    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - 130.0,
    );
    let mut action = SatoshiAction::None;
    egui::Area::new(egui::Id::new(("satoshi_dialogue", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(satoshi::SATOSHI_NAME)
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(240, 210, 120)),
                );
                ui.add_space(12.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(body)
                            .size(15.0)
                            .color(egui::Color32::from_rgb(225, 225, 225)),
                    )
                    .wrap(),
                );
                if let Some(line) = secondary {
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(line)
                                .size(14.0)
                                .italics()
                                .color(egui::Color32::from_rgb(180, 200, 170)),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(16.0);

                // One vertical button per choice — always a dismissal last.
                let button = |ui: &mut egui::Ui, text: &str, fill: egui::Color32| -> bool {
                    ui.add_sized(
                        [260.0, 34.0],
                        egui::Button::new(
                            egui::RichText::new(text).size(15.0).color(egui::Color32::WHITE),
                        )
                        .fill(fill),
                    )
                    .clicked()
                };

                match view {
                    SatoshiView::Greeting => {
                        if button(ui, satoshi::SATOSHI_GIFT_LABEL, egui::Color32::from_rgb(60, 140, 70)) {
                            action = SatoshiAction::TakeGift;
                        }
                        ui.add_space(8.0);
                    }
                    SatoshiView::SchematicOffer => {
                        if button(ui, satoshi::SATOSHI_SCHEMATIC_LABEL, egui::Color32::from_rgb(70, 120, 160)) {
                            action = SatoshiAction::TakeSchematic;
                        }
                        ui.add_space(8.0);
                    }
                    SatoshiView::Idle => {}
                }
                if button(ui, satoshi::SATOSHI_DISMISS_LABEL, egui::Color32::from_rgb(80, 80, 100)) {
                    action = SatoshiAction::Dismiss;
                }
                ui.add_space(14.0);
            });
        });
    action
}

/// Test Lab — Satoshi hands the player one playtest mission at a time (from the
/// ranked `test_board` registry) and takes a one-tap verdict back. `item` is the
/// current mission, or `None` when the list is exhausted. `note` is the optional
/// free-text bound to a "what happened?" field. Returns the player's pick.
pub fn draw_satoshi_mission(
    ctx: &egui::Context,
    viewport: &crate::screen::ViewportRect,
    player_index: usize,
    item: Option<&crate::test_board::TestItem>,
    idx: usize,
    total: usize,
    note: &mut String,
) -> crate::satoshi::MissionAction {
    use crate::satoshi::{self, MissionAction};

    let overlay_rect = egui::Rect::from_min_size(
        egui::pos2(viewport.x as f32, viewport.y as f32),
        egui::vec2(viewport.width as f32, viewport.height as f32),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(("satoshi_mission_overlay", player_index)),
    ))
    .rect_filled(
        overlay_rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 180),
    );

    let panel_w = 520.0;
    let panel_origin = egui::pos2(
        viewport.x as f32 + viewport.width as f32 / 2.0 - panel_w / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0 - 160.0,
    );
    let mut action = MissionAction::None;
    egui::Area::new(egui::Id::new(("satoshi_mission", player_index)))
        .fixed_pos(panel_origin)
        .interactable(true)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_width(panel_w);
            ui.set_max_width(panel_w);
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(satoshi::SATOSHI_NAME)
                        .size(22.0)
                        .strong()
                        .color(egui::Color32::from_rgb(240, 210, 120)),
                );
                ui.add_space(10.0);

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

                match item {
                    Some(it) => {
                        ui.label(
                            egui::RichText::new(format!("Could you try this for me?  ({} of {total})", idx + 1))
                                .size(13.0)
                                .color(egui::Color32::from_rgb(150, 200, 255)),
                        );
                        ui.add_space(8.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&it.title)
                                    .size(17.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(235, 235, 235)),
                            )
                            .wrap(),
                        );
                        if !it.summary.is_empty() {
                            ui.add_space(6.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&it.summary)
                                        .size(14.0)
                                        .color(egui::Color32::from_rgb(200, 200, 200)),
                                )
                                .wrap(),
                            );
                        }
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new("Anything to add? (optional)")
                                .size(12.0)
                                .color(egui::Color32::from_rgb(150, 150, 160)),
                        );
                        ui.add_space(4.0);
                        ui.add_sized([360.0, 26.0], egui::TextEdit::singleline(note));
                        ui.add_space(14.0);

                        if button(ui, "✓  It worked", egui::Color32::from_rgb(45, 130, 65)) {
                            action = MissionAction::Worked;
                        }
                        ui.add_space(8.0);
                        if button(ui, "✗  It's broken", egui::Color32::from_rgb(150, 55, 45)) {
                            action = MissionAction::Broken;
                        }
                        ui.add_space(8.0);
                        if button(ui, "Skip this one", egui::Color32::from_rgb(80, 80, 100)) {
                            action = MissionAction::Skip;
                        }
                        ui.add_space(8.0);
                        if button(ui, "I'll come back", egui::Color32::from_rgb(60, 60, 75)) {
                            action = MissionAction::Close;
                        }
                    }
                    None => {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(satoshi::SATOSHI_MISSIONS_DONE)
                                    .size(15.0)
                                    .color(egui::Color32::from_rgb(225, 225, 225)),
                            )
                            .wrap(),
                        );
                        ui.add_space(16.0);
                        if button(ui, "Thanks, Satoshi", egui::Color32::from_rgb(60, 140, 70)) {
                            action = MissionAction::Close;
                        }
                    }
                }
                ui.add_space(14.0);
            });
        });
    action
}

/// Format a villager's display label. For Phase 5 we use the entity's stable
/// numeric handle as the ID; Phase 13 may swap in nicer names.
pub fn villager_label(entity: hecs::Entity, vc: &VillagerComponent) -> String {
    // hecs Entity Debug = "Entity(id, gen)" — strip to just the id.
    let id_str = format!("{:?}", entity);
    let short = id_str
        .split(['(', ',', ' '])
        .nth(1)
        .unwrap_or(&id_str)
        .to_string();
    match vc.profession {
        Profession::None => format!("Villager #{short}"),
        prof => format!("Villager #{short}, {}", prof.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogue_action_default_is_none() {
        // Sanity — `None` is the variant used as "no click yet"; both other
        // variants must be distinct.
        assert_ne!(DialogueAction::None, DialogueAction::Accept);
        assert_ne!(DialogueAction::None, DialogueAction::Decline);
        assert_ne!(DialogueAction::None, DialogueAction::Close);
    }

    #[test]
    fn decline_cooldown_is_five_minutes() {
        // 5 min × 60 s × 20 TPS = 6000 ticks.
        assert_eq!(DECLINE_COOLDOWN_TICKS, 6000);
    }
}
