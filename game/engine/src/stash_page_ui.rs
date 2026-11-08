//! More to play (home page overhaul spec §7; was "The Stash"): the official
//! games as large gallery-style cards, each with Play, and, on the web only,
//! "Look up a creator" with the experiences and skins they share.
//!
//! Everything here still works offline: the official games are embedded. The
//! look-up is a PWA path (on native Browse only ever printed "Live browse is a
//! PWA feature", so it is hidden there, not shown and explained).

use crate::brand;
use crate::lobby_pages_ui::{page_header, Keys};
#[cfg(target_arch = "wasm32")]
use crate::lobby_pages_ui::{button, Kind};
use crate::menu::{MenuAction, MenuState};
#[cfg(target_arch = "wasm32")]
use crate::menu::StashItem;
use crate::scenario::{ScenarioDef, ScenarioKind};
use crate::trials_diorama::Cube;
use crate::ui_safe_area::Metrics;
use crate::worlds_ui::words;

const GAP: f32 = 18.0;
/// From this width (before scaling) the official cards sit two across.
const TWO_ACROSS_FROM: f32 = 760.0;

/// Draw More to play for one frame.
pub(crate) fn draw(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, keys: Keys) -> MenuAction {
    let mut action = MenuAction::None;
    if page_header(ui, m, words::MORE_TO_PLAY, |_| {}) {
        state.step_back();
        return action;
    }
    ui.add_space(m.px(12.0));
    #[cfg(target_arch = "wasm32")]
    drain(state, &mut action);
    let icons = crate::egui_integration::block_texture_handles(ui.ctx());
    egui::ScrollArea::vertical().id_salt("more_to_play").auto_shrink([false, false]).show(ui, |ui| {
        if let Some(def) = official(ui, m, &icons, keys) {
            action = MenuAction::PlayScenario { def: Box::new(def) };
        }
        #[cfg(target_arch = "wasm32")]
        look_up(ui, state, m, &mut action);
        ui.add_space(m.px(GAP));
    });
    action
}

// ---------------------------------------------------------------------------
// AxeNStax Official
// ---------------------------------------------------------------------------

/// One official game as the card draws it.
struct Game {
    def: ScenarioDef,
    line: &'static str,
    colour: egui::Color32,
    picture: &'static [(&'static str, [i32; 3])],
}

/// The blocks in a card's picture (a handful of stone and what's in it).
const HASH_DASH_PICTURE: &[(&str, [i32; 3])] = &[
    ("stone", [-1, 0, 0]),
    ("stone", [0, 0, 0]),
    ("stone", [1, 0, 0]),
    ("stone", [0, 0, 1]),
    ("iron_ore", [0, 1, 0]),
    ("coal_ore", [-1, 1, 0]),
    ("iron_ore", [1, 1, 0]),
];
const SATORI_RUSH_PICTURE: &[(&str, [i32; 3])] = &[
    ("stone", [-1, 0, 0]),
    ("stone", [0, 0, 0]),
    ("stone", [1, 0, 0]),
    ("stone", [0, 0, 1]),
    ("diamond_ore", [0, 1, 0]),
    ("diamond_block", [0, 2, 0]),
    ("diamond_ore", [1, 1, 0]),
];

fn games() -> Vec<Game> {
    crate::menu::official_stash_items()
        .into_iter()
        .filter_map(|item| item.embedded_def)
        .map(|def| {
            let (line, colour, picture) = match def.kind {
                ScenarioKind::HashDash => {
                    (words::HASH_DASH_LINE, egui::Color32::from_rgb(190, 120, 40), HASH_DASH_PICTURE)
                }
                _ => (words::SATORI_RUSH_LINE, egui::Color32::from_rgb(170, 96, 160), SATORI_RUSH_PICTURE),
            };
            Game { def, line, colour, picture }
        })
        .collect()
}

/// A picture's blocks, resolved once, back to front.
fn cubes(picture: &'static [(&'static str, [i32; 3])]) -> Vec<Cube> {
    let blocks: Vec<crate::trials_diorama::DioramaBlock> = picture
        .iter()
        .map(|(b, at)| crate::trials_diorama::DioramaBlock { block: (*b).to_string(), at: *at })
        .collect();
    crate::trials_gallery::resolve_diorama(&blocks)
}

/// The AxeNStax Official section. Returns the game to play when a card was pressed.
fn official(ui: &mut egui::Ui, m: &Metrics, icons: &[egui::TextureId], keys: Keys) -> Option<ScenarioDef> {
    let mut play = None;
    ui.label(egui::RichText::new(words::OFFICIAL).size(m.px(22.0)).strong().color(brand::COPPER));
    ui.label(egui::RichText::new(words::OFFICIAL_LEAD).size(m.body()).color(brand::MUTED_TEXT));
    ui.add_space(m.px(10.0));
    let gap = m.px(GAP);
    let w = ui.available_width();
    let per_row = if w >= m.px(TWO_ACROSS_FROM) { 2 } else { 1 };
    let card_w = ((w - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
    let pic_h = (card_w * 9.0 / 16.0).clamp(m.px(120.0), m.px(220.0));
    let text_h = m.px(if card_w < m.px(440.0) { 172.0 } else { 160.0 });
    let all = games();
    for (row_i, row) in all.chunks(per_row).enumerate() {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (i, game) in row.iter().enumerate() {
                let seed = keys.seed_focus && row_i == 0 && i == 0;
                if game_card(ui, m, icons, egui::vec2(card_w, pic_h + text_h), pic_h, game, seed) {
                    play = Some(game.def.clone());
                }
            }
        });
        ui.add_space(gap);
    }
    play
}

/// One official game: its picture, name, a plain line and Play. The whole card
/// is one target, so the keyboard ring goes round it and Enter plays.
fn game_card(
    ui: &mut egui::Ui,
    m: &Metrics,
    icons: &[egui::TextureId],
    size: egui::Vec2,
    pic_h: f32,
    game: &Game,
    seed_focus: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if seed_focus {
        resp.request_focus();
    }
    let p = ui.painter().clone();
    let cr = egui::CornerRadius::same(16);
    let hot = resp.hovered() || resp.has_focus();
    p.rect_filled(rect, cr, if hot { brand::SURFACE_HOVER } else { brand::SURFACE });
    let pic = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), pic_h));
    let lerp = crate::trials_diorama_ui::lerp_color;
    crate::trials_diorama_ui::paint_backdrop(
        &p,
        pic,
        [16.0, 16.0, 0.0, 0.0],
        lerp(brand::SURFACE, game.colour, 0.5),
        lerp(brand::SURFACE, game.colour, 0.16),
    );
    p.circle_filled(pic.center(), pic.height() * 0.42, egui::Color32::from_white_alpha(10));
    crate::trials_diorama_ui::paint_diorama(&p, pic.shrink(pic.height() * 0.08), &cubes(game.picture), icons, 0.2);
    p.rect_filled(
        egui::Rect::from_min_max(egui::pos2(pic.min.x, pic.max.y - 5.0), pic.max),
        egui::CornerRadius::ZERO,
        game.colour,
    );

    let pad = m.px(16.0);
    let left = rect.min.x + pad;
    let inner_w = rect.width() - pad * 2.0;
    // The kind ("Race", "Speedrun") sits at the right end of the name's line.
    let tag = game.def.kind.label();
    let tag_g = p.layout_no_wrap(
        tag.to_string(),
        egui::FontId::proportional(m.body()),
        egui::Color32::from_rgb(120, 200, 160),
    );
    let name_y = pic.max.y + pad * 0.8;
    crate::home_ui::text(
        &p,
        egui::pos2(left, name_y),
        &game.def.display_name,
        m.tile_title(),
        brand::STONE,
        inner_w - tag_g.size().x - pad,
    );
    p.galley(
        egui::pos2(rect.max.x - pad - tag_g.size().x, name_y + m.px(8.0)),
        tag_g,
        egui::Color32::from_rgb(120, 200, 160),
    );
    let mut job = egui::text::LayoutJob::single_section(
        game.line.to_string(),
        egui::TextFormat::simple(egui::FontId::proportional(m.body()), brand::MUTED_TEXT),
    );
    job.wrap = egui::text::TextWrapping { max_width: inner_w, max_rows: 3, break_anywhere: false, overflow_character: Some('…') };
    let g = p.layout_job(job);
    p.galley(egui::pos2(left, name_y + m.px(38.0)), g, brand::MUTED_TEXT);

    // The Play pill along the bottom of the card.
    let play = egui::Rect::from_min_max(
        egui::pos2(left, rect.max.y - pad - m.hit()),
        egui::pos2(rect.max.x - pad, rect.max.y - pad),
    );
    p.rect_filled(play, egui::CornerRadius::same(8), egui::Color32::from_rgb(45, 107, 45));
    p.rect_stroke(
        play,
        egui::CornerRadius::same(8),
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 140, 70)),
        egui::StrokeKind::Inside,
    );
    p.text(
        play.center(),
        egui::Align2::CENTER_CENTER,
        format!("\u{25B6}  {}", words::PLAY),
        egui::FontId::proportional(m.body()),
        egui::Color32::from_rgb(214, 244, 214),
    );
    p.rect_stroke(rect, cr, egui::Stroke::new(1.0_f32, brand::BORDER), egui::StrokeKind::Inside);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

// ---------------------------------------------------------------------------
// Look up a creator (web only)
// ---------------------------------------------------------------------------

/// Drain the async results (PWA). A resolved download becomes the action.
#[cfg(target_arch = "wasm32")]
fn drain(state: &mut MenuState, action: &mut MenuAction) {
    if let Some(slot) = state.stash_browse.clone() {
        if let Some(result) = slot.borrow_mut().take() {
            state.stash_browse = None;
            match result {
                Ok(items) => {
                    state.stash_status = Some(if items.is_empty() {
                        "They haven't shared any experiences or skins yet.".to_string()
                    } else {
                        format!("{} item(s) found.", items.len())
                    });
                    state.stash_items = items;
                }
                Err(e) => state.stash_status = Some(format!("Couldn't look them up: {e}")),
            }
        }
    }
    if let Some(slot) = state.stash_action_dl.clone() {
        if let Some(result) = slot.borrow_mut().take() {
            state.stash_action_dl = None;
            match result {
                Ok((crate::open_stash::StashItemKind::Scenario, bytes)) => {
                    match crate::scenario::scenario_from_blob_bytes(&bytes) {
                        Ok(def) => *action = MenuAction::PlayScenario { def: Box::new(def) },
                        Err(e) => state.stash_status = Some(format!("Bad experience: {e}")),
                    }
                }
                Ok((crate::open_stash::StashItemKind::Skin, bytes)) => {
                    *action = MenuAction::AdoptSkin { bytes };
                }
                Err(e) => state.stash_status = Some(format!("Download failed: {e}")),
            }
        }
    }
}

/// "Look up a creator": the address field, its status line, and what they share.
#[cfg(target_arch = "wasm32")]
fn look_up(ui: &mut egui::Ui, state: &mut MenuState, m: &Metrics, action: &mut MenuAction) {
    ui.label(egui::RichText::new(words::LOOK_UP_CREATOR).size(m.px(22.0)).strong().color(brand::COPPER));
    ui.label(egui::RichText::new(words::LOOK_UP_LEAD).size(m.body()).color(brand::MUTED_TEXT));
    ui.add_space(m.px(8.0));
    let mut do_browse = false;
    ui.horizontal(|ui| {
        let btn_w = m.px(110.0);
        let field_w = (ui.available_width() - btn_w - ui.spacing().item_spacing.x).max(m.px(120.0));
        let resp = crate::menu::menu_text_field_tall(
            ui,
            &mut state.stash_input,
            words::LOOK_UP_HINT,
            "npub1…",
            field_w,
            m.body(),
            m.hit(),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            do_browse = true;
        }
        if ui.add_sized(egui::vec2(btn_w, m.hit()), button(m, "Look up", Kind::Primary)).clicked() {
            do_browse = true;
        }
    });
    if do_browse {
        trigger_stash_browse(state);
    }
    if let Some(status) = state.stash_status.clone() {
        ui.add_space(m.px(4.0));
        ui.label(egui::RichText::new(status).size(m.body()).color(egui::Color32::from_rgb(170, 180, 150)));
    }
    ui.add_space(m.px(14.0));

    // Browsed items. A live item that duplicates an embedded official game (by
    // name) is the SAME content as the card above, so it is skipped: the
    // offline seed represents it once; only NEW items list here.
    let official: Vec<String> = crate::menu::official_stash_items().into_iter().map(|i| i.name).collect();
    let items: Vec<StashItem> = state
        .stash_items
        .iter()
        .filter(|it| {
            !(it.kind == crate::open_stash::StashItemKind::Scenario && official.contains(&it.name))
        })
        .cloned()
        .collect();
    if items.is_empty() {
        return;
    }
    ui.label(egui::RichText::new("From this creator").size(m.px(22.0)).strong().color(brand::COPPER));
    ui.add_space(m.px(6.0));
    for item in &items {
        match item_card(ui, m, item) {
            ItemClick::Play => {
                if let Some(def) = item.embedded_def.clone() {
                    *action = MenuAction::PlayScenario { def: Box::new(def) };
                } else if item.blob_hash.is_empty() {
                    state.stash_status = Some("That item has no download — skip.".to_string());
                } else {
                    trigger_stash_download(state, item.blob_hash.clone(), item.kind);
                }
            }
            ItemClick::Adopt => {
                if item.blob_hash.is_empty() {
                    state.stash_status = Some("That item has no download — skip.".to_string());
                } else {
                    trigger_stash_download(state, item.blob_hash.clone(), item.kind);
                }
            }
            ItemClick::None => {}
        }
    }
}

/// What a browsed item's card emitted this frame.
#[cfg(target_arch = "wasm32")]
enum ItemClick {
    None,
    /// Play this scenario: embedded def in hand, or download by hash.
    Play,
    /// Adopt this skin: download by hash, apply on next world entry.
    Adopt,
}

/// One browsed item: name, author and one action (Play an experience, Adopt a skin).
#[cfg(target_arch = "wasm32")]
fn item_card(ui: &mut egui::Ui, m: &Metrics, item: &StashItem) -> ItemClick {
    let mut click = ItemClick::None;
    egui::Frame::new()
        .fill(brand::SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(m.px(14.0) as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&item.name).size(m.px(20.0)).color(brand::STONE).strong());
                if item.kind == crate::open_stash::StashItemKind::Scenario {
                    let (tag, col) = match &item.embedded_def {
                        Some(def) => (def.kind.label(), egui::Color32::from_rgb(120, 200, 160)),
                        None => ("Experience", egui::Color32::from_rgb(120, 160, 210)),
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(tag).size(m.body()).color(col).strong());
                    });
                }
            });
            ui.label(
                egui::RichText::new(crate::plan::short_npub(&item.author_npub, 22))
                    .size(m.body())
                    .color(brand::MUTED_TEXT),
            );
            ui.add_space(m.px(6.0));
            let (label, kind, which) = match item.kind {
                crate::open_stash::StashItemKind::Scenario => ("Play", Kind::Primary, ItemClick::Play),
                crate::open_stash::StashItemKind::Skin => ("\u{2726} Adopt skin", Kind::Secondary, ItemClick::Adopt),
            };
            let w = ui.available_width();
            if ui.add_sized(egui::vec2(w, m.hit()), button(m, label, kind)).clicked() {
                click = which;
            }
        });
    ui.add_space(m.px(8.0));
    click
}

// ---------------------------------------------------------------------------
// WASM async plumbing (moved with the page from `menu.rs`)
// ---------------------------------------------------------------------------

/// WASM: kick off a Beacon browse of `pubkey_hex` for the Stash Column. Lists
/// the npub's published items, keeps only the types this build can render
/// (skin / scenario), and maps them to typed [`StashItem`]s. Result lands in
/// the returned slot, drained by `drain` each frame. Mirrors
/// `kick_off_local_fetch` / `game_loop`'s `beacon_browse_slot` pattern.
#[cfg(target_arch = "wasm32")]
fn kick_off_stash_browse(
    pubkey_hex: String,
) -> std::rc::Rc<std::cell::RefCell<Option<Result<Vec<StashItem>, String>>>> {
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = match crate::open_stash::beacon_list(&pubkey_hex).await {
            Ok(items) => {
                let npub = crate::npub::hex_to_npub(&pubkey_hex);
                let mapped = items
                    .into_iter()
                    .filter_map(|it| {
                        crate::open_stash::classify_content_type(&it.content_type).map(|kind| {
                            StashItem {
                                name: it.name,
                                author_npub: npub.clone(),
                                kind,
                                blob_hash: it.blob_hash,
                                embedded_def: None,
                            }
                        })
                    })
                    .collect();
                Ok(mapped)
            }
            Err(e) => Err(e),
        };
        *slot_clone.borrow_mut() = Some(result);
    });
    slot
}

/// WASM: kick off a Beacon blob download for a Play/Adopt action, tagged with
/// the item kind so the resolved bytes route to the right `MenuAction`.
#[cfg(target_arch = "wasm32")]
fn kick_off_stash_download(
    blob_hash: String,
    kind: crate::open_stash::StashItemKind,
) -> std::rc::Rc<std::cell::RefCell<Option<Result<(crate::open_stash::StashItemKind, Vec<u8>), String>>>> {
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot_clone = slot.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::open_stash::beacon_download(&blob_hash)
            .await
            .map(|bytes| (kind, bytes));
        *slot_clone.borrow_mut() = Some(result);
    });
    slot
}

/// Normalise the typed npub/hex and kick a browse (PWA only: the look-up is
/// hidden on native). Centralised so the input handler stays readable.
#[cfg(target_arch = "wasm32")]
fn trigger_stash_browse(state: &mut MenuState) {
    let trimmed = state.stash_input.trim().to_string();
    // Empty Browse → show the official AxeNStax stash LIVE (the online
    // counterpart of the embedded floor). Otherwise normalise the typed value:
    // hex passes straight through; an npub bech32-decodes at the boundary (PWA).
    let hex = if trimmed.is_empty() {
        Some(crate::open_stash::official_axenstax_pubkey())
    } else {
        crate::open_stash::normalize_pubkey(&trimmed).or_else(|| crate::npub::npub_to_hex(&trimmed))
    };
    match hex {
        Some(h) => {
            state.stash_status = Some("Looking…".to_string());
            state.stash_items.clear();
            state.stash_browse = Some(kick_off_stash_browse(h));
        }
        None => {
            state.stash_status = Some("That isn't an npub1… address.".to_string());
        }
    }
}

/// Kick a download for a Play/Adopt action on a browsed item (PWA only: browsed
/// items exist only on the web).
#[cfg(target_arch = "wasm32")]
fn trigger_stash_download(state: &mut MenuState, blob_hash: String, kind: crate::open_stash::StashItemKind) {
    state.stash_status = Some("Fetching…".to_string());
    state.stash_action_dl = Some(kick_off_stash_download(blob_hash, kind));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_official_games_are_cards_with_a_line_and_a_picture() {
        let all = games();
        assert_eq!(all.len(), 2, "Hash Dash and Satori Rush");
        let names: Vec<&str> = all.iter().map(|g| g.def.display_name.as_str()).collect();
        assert_eq!(names, ["Hash Dash", "Satori Rush"]);
        for g in &all {
            assert!(!g.line.is_empty());
            assert_eq!(cubes(g.picture).len(), g.picture.len(), "{}: a picture block does not resolve", g.def.display_name);
        }
    }

    #[test]
    fn the_cards_say_nothing_about_money_scores_or_counts() {
        for line in [words::HASH_DASH_LINE, words::SATORI_RUSH_LINE, words::OFFICIAL_LEAD, words::LOOK_UP_LEAD] {
            let bad = crate::copy_lint::banned_in(line, crate::copy_lint::BANNED_MONEY_WORDS);
            assert!(bad.is_empty(), "{line:?} contains {bad:?}");
            for w in ["score", "rank", "leaderboard", "players", "followers"] {
                assert!(!line.to_lowercase().contains(w), "{line:?} contains {w:?}");
            }
        }
    }

    #[test]
    fn hash_dash_line_matches_its_timer() {
        // The three minutes the line promises are the objective's 3600 ticks at 20 per second.
        let def = crate::scenario::hash_dash_def();
        assert!(matches!(def.objective, crate::scenario::Objective::Timed { ticks: 3600 }));
    }
}
