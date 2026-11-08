//! One world's page (home page overhaul spec §4.2): a big picture, the name,
//! description and mode, the facts, a large Play, "Start at", the two ways to
//! host (native), and a More ⋯ menu with everything rare or destructive.
//!
//! Every button routes through `menu::handle_card_action`, which refuses
//! (with the reason) a world saved by a newer build or retired by a handover.
//!
//! Hidden where they don't work (rule 6): hosting and Make a copy on the web;
//! the two "Save a copy" entries on Android.

use crate::brand;
use crate::lobby_pages_ui::{button, menu_button, menu_entry, HostOnline, Keys, Kind};
use crate::menu::{CardAction, MenuAction, MenuState};
use crate::ui_safe_area::Metrics;
#[cfg(not(target_arch = "wasm32"))]
use crate::worlds_ui::words;

/// Draw world `idx`'s page into `ui` (the right-hand panel, or the whole page).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    ui: &mut egui::Ui,
    state: &mut MenuState,
    idx: usize,
    m: &Metrics,
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))] host: HostOnline,
    keys: Keys,
    icons: &[egui::TextureId],
) -> MenuAction {
    let mut card_action = CardAction::None;
    let Some(entry) = state.worlds.get(idx) else { return MenuAction::None };
    let meta = entry.meta.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let last_played = crate::save::format_relative_time(entry.last_played);
    let size = crate::save::format_size(entry.size_bytes);
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    let cloud_only = entry.cloud_only;
    let folder = entry.folder_name.clone();
    #[cfg(target_arch = "wasm32")]
    let in_cloud = state.cloud_world_names.contains(&meta.display_name);
    #[cfg(not(target_arch = "wasm32"))]
    let in_cloud = false;
    let _ = in_cloud;

    let frame = egui::Frame::new()
        .fill(brand::SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(14));
    ui.push_id(("world_detail", &folder), |ui| {
        frame.show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("world_detail_scroll").auto_shrink([false, false]).show(ui, |ui| {
                let w = ui.available_width() - m.px(14.0);
                ui.set_width(w);
                let pic_h = (w * 9.0 / 16.0).min(m.px(320.0)).min(ui.ctx().content_rect().height() * 0.3);
                let (pic, _) = ui.allocate_exact_size(egui::vec2(w, pic_h), egui::Sense::hover());
                crate::world_picture::paint(ui.painter(), pic, [12.0; 4], &meta, icons);
                ui.add_space(m.px(10.0));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&meta.display_name).size(m.px(28.0)).strong().color(brand::STONE),
                    )
                    .wrap(),
                );
                ui.horizontal(|ui| {
                    let mode = crate::world_picture::mode_label(&meta.game_mode);
                    ui.label(
                        egui::RichText::new(mode).size(m.body()).strong().color(crate::world_picture::mode_colour(&meta.game_mode)),
                    );
                    if let Some((label, _, fg)) = crate::worlds_ui::publish_badge(meta.publish_badge()) {
                        ui.label(egui::RichText::new(label).size(m.px(13.0)).color(fg));
                    }
                });
                ui.add_space(m.px(4.0));
                let desc = if meta.description.is_empty() {
                    egui::RichText::new("No description").italics()
                } else {
                    egui::RichText::new(&meta.description)
                };
                ui.label(desc.size(m.body()).color(brand::MUTED_TEXT));
                ui.add_space(m.px(6.0));
                let mut facts = vec![format!("Size {size}")];
                #[cfg(not(target_arch = "wasm32"))]
                facts.push(format!("Last played {}", last_played.to_lowercase()));
                if meta.version > 0 {
                    facts.push(format!("Version {}", meta.version));
                }
                ui.label(egui::RichText::new(facts.join("  \u{00B7}  ")).size(m.body()).color(brand::MUTED_TEXT));
                ui.add_space(m.px(14.0));

                // ▶ Play: the one big thing.
                let play_label = if entry_cloud_only(state, idx) { "\u{25B6}  Play (from Stash)" } else { "\u{25B6}  Play" };
                let play = ui.add(
                    button(m, play_label, Kind::Primary).min_size(egui::vec2(w, m.px(56.0))),
                );
                if keys.seed_focus {
                    play.request_focus();
                }
                if play.clicked() {
                    card_action = CardAction::Play;
                }
                ui.add_space(m.px(10.0));

                // Start at: the one remembered spawn choice; it applies only to Play.
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Start at").size(m.body()).color(brand::STONE));
                    let cw = (ui.available_width() - m.px(8.0)).clamp(m.px(120.0), m.px(260.0));
                    ui.spacing_mut().interact_size.y = m.hit();
                    egui::ComboBox::from_id_salt("spawn_pref_combo")
                        .width(cw)
                        .height(m.px(400.0))
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .selected_text(egui::RichText::new(state.spawn_pref.label()).size(m.body()))
                        .show_ui(ui, |ui| {
                            for opt in crate::spawn_pref::SpawnPref::all() {
                                ui.selectable_value(
                                    &mut state.spawn_pref,
                                    *opt,
                                    egui::RichText::new(opt.label()).size(m.body()),
                                );
                            }
                        });
                });
                ui.add_space(m.px(12.0));

                // Hosting (native): you host a particular world, so it's here.
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let full = egui::vec2(w, m.hit());
                    if ui.add(button(m, words::HOST_LAN, Kind::Secondary).min_size(full)).clicked() {
                        card_action = CardAction::Host;
                    }
                    let resp = ui.add_enabled(host.enabled, button(m, words::HOST_ONLINE, Kind::Secondary).min_size(full));
                    if resp.clicked() {
                        card_action = CardAction::HostOnline;
                    }
                    if let Some(note) = host.note {
                        ui.label(egui::RichText::new(note).size(m.body()).color(brand::MUTED_TEXT));
                    }
                    ui.add_space(m.px(10.0));
                }

                let a = more_menu(ui, m, &meta, cloud_only, in_cloud, state.dialog_pending());
                if !matches!(a, CardAction::None) {
                    card_action = a;
                }
            });
        });
    });
    crate::menu::handle_card_action(state, idx, card_action)
}

fn entry_cloud_only(state: &MenuState, idx: usize) -> bool {
    state.worlds.get(idx).is_some_and(|e| e.cloud_only)
}

/// More ⋯: rename, copy, save a copy, the Stash toggle, then (after a divider) Delete.
#[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
fn more_menu(
    ui: &mut egui::Ui,
    m: &Metrics,
    meta: &crate::save::WorldMeta,
    cloud_only: bool,
    in_cloud: bool,
    dialog_pending: bool,
) -> CardAction {
    let mut out = CardAction::None;
    menu_button(ui, m, "More \u{2026}", |ui| {
        let mut pick = |ui: &mut egui::Ui, a: CardAction| {
            out = a;
            ui.close();
        };
        if ui.add(menu_entry(m, "Rename and describe…", brand::STONE)).clicked() {
            pick(ui, CardAction::Edit);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if ui.add(menu_entry(m, "Make a copy", brand::STONE)).clicked() {
            pick(ui, CardAction::Fork);
        }
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
        {
            if ui.add_enabled(!dialog_pending, menu_entry(m, "Save a copy as a file…", brand::STONE)).clicked() {
                pick(ui, CardAction::Export);
            }
            if ui
                .add_enabled(!dialog_pending, menu_entry(m, "Save a copy to the transfer folder", brand::STONE))
                .clicked()
            {
                pick(ui, CardAction::ExportToFolder);
            }
        }
        #[cfg(target_arch = "wasm32")]
        if ui.add(menu_entry(m, "Download a backup", brand::STONE)).clicked() {
            pick(ui, CardAction::Backup);
        }
        // The per-world Stash toggle (native; a cloud-only world is already stashed).
        #[cfg(not(target_arch = "wasm32"))]
        if !cloud_only {
            let (dot, label, colour, hint) = keep_style(meta.cloud_save, in_cloud);
            if ui
                .add(menu_entry(m, format!("{dot}  {label}"), colour))
                .on_hover_text(hint)
                .clicked()
            {
                pick(ui, CardAction::ToggleStash);
            }
        }
        ui.separator();
        if ui
            .add(menu_entry(m, "Delete this world…", egui::Color32::from_rgb(236, 104, 104)))
            .clicked()
        {
            pick(ui, CardAction::Delete);
        }
    });
    out
}

/// The Stash entry's dot, words, colour and hint: off (red), on but not yet
/// confirmed (amber), on and confirmed (green).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn keep_style(cloud_save: bool, in_cloud: bool) -> (&'static str, &'static str, egui::Color32, &'static str) {
    if !cloud_save {
        ("\u{1F534}", words::KEEP_OFF, egui::Color32::from_rgb(210, 150, 150), words::KEEP_OFF_HINT)
    } else if in_cloud {
        ("\u{1F7E2}", words::KEEP_ON, egui::Color32::from_rgb(160, 220, 160), words::KEEP_ON_HINT)
    } else {
        ("\u{1F7E0}", words::KEEP_ON_UNCONFIRMED, egui::Color32::from_rgb(230, 190, 120), words::KEEP_ON_UNCONFIRMED_HINT)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn the_stash_entry_keeps_its_three_states() {
        assert_eq!(keep_style(false, false).1, words::KEEP_OFF);
        assert_eq!(keep_style(true, false).1, words::KEEP_ON_UNCONFIRMED);
        assert_eq!(keep_style(true, true).1, words::KEEP_ON);
        assert_ne!(keep_style(false, false).2, keep_style(true, true).2);
    }
}
