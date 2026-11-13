//! The "More games" section of the lobby's More to play page (native only):
//! one card per other game from the same makers, each opening the website's
//! More games page in the browser. See `more_games.rs` for the list.
//!
//! The card is one keyboard and gamepad target, like the official cards. It
//! opens its link with `Context::open_url`, the call `ui.hyperlink_to` makes
//! (as the version line in `menu.rs` does), so the host's existing link
//! handling (`egui-winit` "links", then `webbrowser`) opens it on desktop and
//! on Android alike.

use crate::brand;
use crate::more_games::{MoreGame, MORE_GAMES};
use crate::ui_safe_area::Metrics;
use crate::worlds_ui::words;

const GAP: f32 = 18.0;
/// From this width (before scaling) the cards sit two across.
const TWO_ACROSS_FROM: f32 = 760.0;
const ICON: f32 = 72.0;

/// The section: title, lead and the cards.
pub(crate) fn section(ui: &mut egui::Ui, m: &Metrics) {
    ui.label(egui::RichText::new(words::MORE_GAMES).size(m.px(22.0)).strong().color(brand::COPPER));
    ui.label(egui::RichText::new(words::MORE_GAMES_LEAD).size(m.body()).color(brand::MUTED_TEXT));
    ui.add_space(m.px(10.0));
    let gap = m.px(GAP);
    let w = ui.available_width();
    let per_row = if w >= m.px(TWO_ACROSS_FROM) { 2 } else { 1 };
    let card_w = ((w - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
    let card_h = m.px(20.0) + m.px(34.0) + m.px(30.0) + m.body() * 1.4 * 3.0 + m.px(10.0) + m.hit();
    for row in MORE_GAMES.chunks(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for game in row {
                if card(ui, m, egui::vec2(card_w, card_h), game) {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(game.url()));
                }
            }
        });
        ui.add_space(gap);
    }
}

/// The game's icon as a texture, decoded and uploaded once per context.
fn icon(ctx: &egui::Context, game: &MoreGame) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("more_games_icon", game.anchor));
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(tex);
    }
    let img = image::load_from_memory(game.icon_png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let tex = ctx.load_texture(
        format!("more_games_{}", game.anchor),
        egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    Some(tex)
}

/// One game: icon, name, an "Android" tag, its blurb and a "See it" pill.
/// Returns true when pressed (click, or Enter on the focus ring).
fn card(ui: &mut egui::Ui, m: &Metrics, size: egui::Vec2, game: &MoreGame) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{}: {}", words::SEE_IT, game.name))
    });
    let p = ui.painter().clone();
    let cr = egui::CornerRadius::same(16);
    let hot = resp.hovered() || resp.has_focus();
    p.rect_filled(rect, cr, if hot { brand::SURFACE_HOVER } else { brand::SURFACE });

    let pad = m.px(16.0);
    let icon_side = m.px(ICON);
    let icon_rect = egui::Rect::from_min_size(rect.min + egui::vec2(pad, pad), egui::vec2(icon_side, icon_side));
    if let Some(tex) = icon(ui.ctx(), game) {
        p.image(
            tex.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    let left = icon_rect.max.x + pad;
    let inner_w = rect.max.x - pad - left;
    crate::home_ui::text(&p, egui::pos2(left, rect.min.y + pad * 0.8), game.name, m.tile_title(), brand::STONE, inner_w);

    // The platform tag, an outlined pill under the name.
    let green = egui::Color32::from_rgb(120, 200, 160);
    let tag = p.layout_no_wrap(words::ANDROID.to_string(), egui::FontId::proportional(m.body()), green);
    let tag_rect = egui::Rect::from_min_size(
        egui::pos2(left, rect.min.y + pad * 0.8 + m.px(34.0)),
        tag.size() + egui::vec2(m.px(16.0), m.px(4.0)),
    );
    p.rect_stroke(tag_rect, egui::CornerRadius::same(10), egui::Stroke::new(1.0_f32, green), egui::StrokeKind::Inside);
    p.galley(tag_rect.min + egui::vec2(m.px(8.0), m.px(2.0)), tag, green);

    // The blurb runs full width under the icon and the name.
    let blurb_top = tag_rect.max.y + m.px(8.0);
    let blurb_left = rect.min.x + pad;
    let mut job = egui::text::LayoutJob::single_section(
        game.blurb.to_string(),
        egui::TextFormat::simple(egui::FontId::proportional(m.body()), brand::MUTED_TEXT),
    );
    job.wrap = egui::text::TextWrapping {
        max_width: rect.width() - pad * 2.0,
        max_rows: 3,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let g = p.layout_job(job);
    p.galley(egui::pos2(blurb_left, blurb_top.max(icon_rect.max.y + m.px(8.0))), g, brand::MUTED_TEXT);

    // The See it pill along the bottom of the card.
    let pill = egui::Rect::from_min_max(
        egui::pos2(blurb_left, rect.max.y - pad - m.hit()),
        egui::pos2(rect.max.x - pad, rect.max.y - pad),
    );
    p.rect_filled(pill, egui::CornerRadius::same(8), egui::Color32::from_rgb(45, 107, 45));
    p.rect_stroke(
        pill,
        egui::CornerRadius::same(8),
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 140, 70)),
        egui::StrokeKind::Inside,
    );
    p.text(
        pill.center(),
        egui::Align2::CENTER_CENTER,
        words::SEE_IT,
        egui::FontId::proportional(m.body()),
        egui::Color32::from_rgb(214, 244, 214),
    );
    p.rect_stroke(rect, cr, egui::Stroke::new(1.0_f32, brand::BORDER), egui::StrokeKind::Inside);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}
