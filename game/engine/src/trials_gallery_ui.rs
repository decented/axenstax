//! The lobby's Trials gallery, drawn: a full-screen shelf of big picture cards
//! and a hero view for one opened card. It replaces the old 272-px text column
//! (design: `docs/vision/trials-two-acts.md` Part C, §C.2 in the internal repo).
//!
//! Principles it keeps: a trial is a place you look into, not a row you read;
//! no counts, totals, percentages or "x of y" anywhere; the shelves are shown
//! in a fixed order and nothing is ranked. The data side (rows, card art,
//! built cards) is `trials_gallery`; Play returns the same `MenuAction` the old
//! column did.

use crate::brand;
use crate::menu::MenuAction;
use crate::trials::{TrialBests, TrialType};
use crate::trials_diorama_ui::{paint_backdrop, paint_diorama};
use crate::trials_gallery::{build_rows, BuiltRow, Card, CardArt};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The gallery's state, held on `MenuState`. Closed by default.
#[derive(Default)]
pub struct GalleryState {
    built: Option<Vec<BuiltRow>>,
    /// The opened card, as (row, column), when the hero view is showing.
    hero: Option<(usize, usize)>,
    /// The focused card, as (row, column).
    focus: (usize, usize),
    /// Set when the keyboard moved focus: scroll it into view once.
    scroll_to_focus: bool,
    /// Set when the hero just opened: give the Play button keyboard focus.
    hero_fresh: bool,
}

impl GalleryState {
    pub fn is_open(&self) -> bool {
        self.built.is_some()
    }

    /// Open the gallery on its first card. Reads the player's bests afresh.
    pub fn open(&mut self) {
        self.built = Some(build_rows(&TrialBests::load()));
        self.hero = None;
        self.focus = (0, 0);
        self.scroll_to_focus = false;
    }

    /// Back to the world list.
    pub fn close(&mut self) {
        self.built = None;
        self.hero = None;
    }

    /// Open the gallery straight onto one trial's hero view (the screenshot
    /// path). `key` is `"race:<id>"` or `"ch:<token>"` (a bare token means the
    /// challenge). Unknown keys leave the gallery open on its shelves.
    pub fn open_hero(&mut self, key: &str) {
        self.open();
        let key = &if key.contains(':') { key.to_string() } else { format!("ch:{key}") };
        let found = self.built.as_ref().and_then(|rows| {
            rows.iter().enumerate().find_map(|(r, row)| {
                row.cards.iter().position(|c| c.key == *key).map(|c| (r, c))
            })
        });
        if let Some(pos) = found {
            self.focus = pos;
            self.hero = Some(pos);
            self.hero_fresh = true;
        }
    }
}

/// What a gallery frame asks of the lobby.
pub struct GalleryOutput {
    /// Play (or ghost export / import) pressed: hand to the caller as usual.
    pub action: MenuAction,
    /// "How it works" pressed: open the info modal for this trial key.
    pub how_it_works: Option<String>,
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

const CARD_W: f32 = 200.0;
const CARD_H: f32 = 150.0;
/// Height of the picture part of a card; the rest is the title.
const ART_H: f32 = 104.0;
const CARD_GAP: f32 = 16.0;
/// How much a lifted card grows.
const LIFT_SCALE: f32 = 0.06;

const TEXT: egui::Color32 = brand::STONE;
const MUTED: egui::Color32 = brand::MUTED_TEXT;
const RIVAL_ORANGE: egui::Color32 = egui::Color32::from_rgb(255, 165, 60);
const DONE_GREEN: egui::Color32 = egui::Color32::from_rgb(110, 200, 120);

fn rgb(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn with_alpha(c: egui::Color32, a: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// The art backdrop's two colours: light at the top, deeper at the bottom, both
/// the shelf's colour over the surface.
fn backdrop_colours(shelf: egui::Color32) -> (egui::Color32, egui::Color32) {
    (
        mix(mix(brand::SURFACE, shelf, 0.48), egui::Color32::WHITE, 0.07),
        mix(brand::SURFACE, shelf, 0.17),
    )
}

/// Paint a card's picture into `rect`: a shelf-coloured gradient (rounded by
/// `radii`, nw/ne/se/sw), then the picture: an isometric block diorama, a flat
/// swatch, or the shelf glyph. This is the one place the picture is made, so
/// real arena art can replace it here later without touching the layout.
fn paint_art(
    painter: &egui::Painter,
    rect: egui::Rect,
    radii: [f32; 4],
    art: &CardArt,
    shelf: egui::Color32,
    glyph: &str,
    icons: &[egui::TextureId],
) {
    let (top, bottom) = backdrop_colours(shelf);
    paint_backdrop(painter, rect, radii, top, bottom);
    let centre = rect.center();
    let icon_px = rect.height() * 0.62;
    // A soft glow behind the picture lifts dark textures off the backdrop.
    painter.circle_filled(centre, rect.height() * 0.46, with_alpha(egui::Color32::WHITE, 9));
    painter.circle_filled(centre, rect.height() * 0.32, with_alpha(egui::Color32::WHITE, 10));
    match art {
        CardArt::Diorama(cubes) => {
            paint_diorama(painter, rect, cubes, icons, DIORAMA_MAX_SCALE);
        }
        CardArt::Swatch { rgb: c } => {
            let base = rgb(*c);
            let r = egui::Rect::from_center_size(centre, egui::Vec2::splat(icon_px * 0.82));
            let cr = egui::CornerRadius::same(10);
            painter.rect_filled(
                r.translate(egui::vec2(0.0, 4.0)),
                cr,
                egui::Color32::from_black_alpha(70),
            );
            painter.rect_filled(r, cr, base);
            let inner = r.shrink(r.width() * 0.16);
            painter.rect_filled(inner, egui::CornerRadius::same(6), mix(base, egui::Color32::WHITE, 0.22));
            painter.rect_stroke(
                r,
                cr,
                egui::Stroke::new(2.0_f32, mix(base, egui::Color32::BLACK, 0.45)),
                egui::StrokeKind::Inside,
            );
        }
        CardArt::Shelf => {
            painter.text(
                centre,
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::proportional(icon_px * 0.85),
                mix(shelf, egui::Color32::WHITE, 0.25),
            );
        }
    }
}

/// A lone block is drawn no taller than this fraction of the art's height.
const DIORAMA_MAX_SCALE: f32 = 0.36;

/// A small "done" mark in a card's corner (postcards come later).
fn paint_done_mark(painter: &egui::Painter, centre: egui::Pos2) {
    painter.circle_filled(centre, 11.0, DONE_GREEN);
    painter.circle_stroke(centre, 11.0, egui::Stroke::new(1.5_f32, egui::Color32::from_black_alpha(90)));
    painter.add(egui::Shape::line(
        vec![
            centre + egui::vec2(-5.0, 0.0),
            centre + egui::vec2(-1.5, 4.0),
            centre + egui::vec2(5.0, -4.0),
        ],
        egui::Stroke::new(2.5_f32, egui::Color32::WHITE),
    ));
}

/// Paint one card. `lift` runs 0..1 (eased): the card grows and its shadow deepens.
fn paint_card(
    painter: &egui::Painter,
    rect: egui::Rect,
    card: &Card,
    lift: f32,
    icons: &[egui::TextureId],
) {
    let shelf = rgb(card.row.color_rgb());
    let r = egui::Rect::from_center_size(rect.center(), rect.size() * (1.0 + LIFT_SCALE * lift));
    let cr = egui::CornerRadius::same(12);
    painter.add(
        egui::Shadow {
            offset: [0, (3.0 + 7.0 * lift) as i8],
            blur: (6.0 + 14.0 * lift) as u8,
            spread: 0,
            color: egui::Color32::from_black_alpha((70.0 + 80.0 * lift) as u8),
        }
        .as_shape(r, cr),
    );
    painter.rect_filled(r, cr, brand::SURFACE);
    let art_rect = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), r.height() * ART_H / CARD_H));
    paint_art(painter, art_rect, [12.0, 12.0, 0.0, 0.0], &card.art, shelf, card.row.glyph(), icons);
    // The shelf-colour band between picture and title.
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(r.min.x, art_rect.max.y), egui::vec2(r.width(), 3.0)),
        0.0,
        shelf,
    );
    // Title under the art: up to two lines.
    let mut job = egui::text::LayoutJob::simple(
        card.title.clone(),
        egui::FontId::proportional(13.5),
        TEXT,
        r.width() - 20.0,
    );
    job.wrap.max_rows = 2;
    let galley = painter.layout_job(job);
    painter.galley(egui::pos2(r.min.x + 10.0, art_rect.max.y + 9.0), galley, TEXT);
    // Frame in the shelf colour: soft at rest, full when lifted.
    painter.rect_stroke(
        r,
        cr,
        egui::Stroke::new(2.0_f32, mix(mix(brand::BORDER, shelf, 0.55), shelf, lift)),
        egui::StrokeKind::Inside,
    );
    if card.done {
        paint_done_mark(painter, egui::pos2(r.max.x - 18.0, r.min.y + 18.0));
    }
}

/// What the shelves view found this frame.
#[derive(Default)]
struct ShelfResult {
    opened: Option<(usize, usize)>,
    hovered: Option<(usize, usize)>,
}

fn draw_shelves(
    ui: &mut egui::Ui,
    rows: &[BuiltRow],
    focus: (usize, usize),
    scroll_to_focus: bool,
    icons: &[egui::TextureId],
) -> ShelfResult {
    let mut out = ShelfResult::default();
    let pointer_moved = ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    egui::ScrollArea::vertical().id_salt("trials_gallery_rows").auto_shrink([false, false]).show(ui, |ui| {
        for (ri, row) in rows.iter().enumerate() {
            let colour = rgb(row.kind.color_rgb());
            ui.add_space(14.0);
            ui.label(
                egui::RichText::new(format!("{}  {}", row.kind.glyph(), row.kind.title()))
                    .size(20.0)
                    .strong()
                    .color(colour),
            );
            ui.add_space(4.0);
            egui::ScrollArea::horizontal()
                .id_salt(("trials_gallery_row", ri))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = CARD_GAP;
                        ui.add_space(6.0);
                        for (ci, card) in row.cards.iter().enumerate() {
                            let (rect, resp) =
                                ui.allocate_exact_size(egui::vec2(CARD_W, CARD_H), egui::Sense::click());
                            let is_focus = focus == (ri, ci);
                            let lift = ui.ctx().animate_bool_with_time_and_easing(
                                resp.id,
                                is_focus,
                                0.14,
                                egui::emath::easing::cubic_out,
                            );
                            if resp.clicked() {
                                out.opened = Some((ri, ci));
                            }
                            if resp.hovered() && pointer_moved {
                                out.hovered = Some((ri, ci));
                            }
                            if is_focus && scroll_to_focus {
                                ui.scroll_to_rect(rect.expand(28.0), None);
                            }
                            paint_card(ui.painter(), rect, card, lift, icons);
                            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        }
                        ui.add_space(6.0);
                    });
                    ui.add_space(10.0);
                });
        }
        ui.add_space(16.0);
    });
    out
}

/// What the hero view found this frame.
#[derive(Default)]
struct HeroResult {
    action: Option<MenuAction>,
    how_it_works: bool,
}

fn draw_hero(ui: &mut egui::Ui, card: &Card, icons: &[egui::TextureId], fresh: bool) -> HeroResult {
    let mut out = HeroResult::default();
    let shelf = rgb(card.row.color_rgb());
    egui::ScrollArea::vertical().id_salt("trials_gallery_hero").auto_shrink([false, false]).show(ui, |ui| {
        let w = ui.available_width().min(640.0);
        let pad = ((ui.available_width() - w) / 2.0).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(w);
                // The art at full width.
                let h = (w * 0.5).clamp(160.0, 320.0);
                let (art_rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
                let p = ui.painter();
                let cr = egui::CornerRadius::same(16);
                paint_art(p, art_rect, [16.0; 4], &card.art, shelf, card.row.glyph(), icons);
                p.rect_stroke(art_rect, cr, egui::Stroke::new(3.0_f32, shelf), egui::StrokeKind::Inside);
                if card.done {
                    paint_done_mark(p, egui::pos2(art_rect.max.x - 24.0, art_rect.min.y + 24.0));
                }

                ui.add_space(12.0);
                ui.label(egui::RichText::new(&card.title).size(28.0).strong().color(brand::COPPER));
                ui.label(
                    egui::RichText::new(format!("{} {}", card.row.glyph(), card.row.title()))
                        .size(13.0)
                        .color(shelf),
                );
                if !card.premise.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&card.premise).size(16.0).color(TEXT));
                }

                // Satoshi's line, in a speech bubble.
                if !card.satoshi.is_empty() {
                    ui.add_space(14.0);
                    ui.horizontal_top(|ui| {
                        let (face, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), egui::Sense::hover());
                        ui.painter().circle_filled(face.center(), 19.0, brand::LANTERN);
                        ui.painter().text(
                            face.center(),
                            egui::Align2::CENTER_CENTER,
                            "S",
                            egui::FontId::proportional(20.0),
                            brand::DEEP_FRONTIER,
                        );
                        ui.add_space(10.0);
                        let fill = egui::Color32::from_rgb(36, 50, 56);
                        let bubble = egui::Frame::new()
                            .fill(fill)
                            .stroke(egui::Stroke::new(1.0_f32, brand::BORDER))
                            .corner_radius(egui::CornerRadius::same(14))
                            .inner_margin(egui::Margin::same(14))
                            .show(ui, |ui| {
                                ui.set_width((w - 38.0 - 10.0 - 30.0).max(120.0));
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new("Satoshi").size(12.0).strong().color(brand::LANTERN));
                                    ui.add(egui::Label::new(egui::RichText::new(&card.satoshi).size(15.0).color(TEXT)).wrap());
                                });
                            });
                        // The bubble's tail, pointing back at Satoshi.
                        let b = bubble.response.rect;
                        let y = b.min.y + 19.0;
                        ui.painter().add(egui::Shape::convex_polygon(
                            vec![
                                egui::pos2(b.min.x + 1.0, y - 8.0),
                                egui::pos2(b.min.x + 1.0, y + 8.0),
                                egui::pos2(b.min.x - 10.0, y),
                            ],
                            fill,
                            egui::Stroke::NONE,
                        ));
                    });
                }

                // The steps.
                if !card.steps.is_empty() {
                    ui.add_space(14.0);
                    for (i, line) in card.steps.iter().take(8).enumerate() {
                        ui.horizontal_top(|ui| {
                            let mark = if card.ordered { format!("{}", i + 1) } else { card.row.glyph().to_string() };
                            ui.add_sized(
                                [24.0, 20.0],
                                egui::Label::new(egui::RichText::new(mark).size(15.0).strong().color(shelf)),
                            );
                            ui.add(egui::Label::new(egui::RichText::new(line).size(15.0).color(TEXT)).wrap());
                        });
                        ui.add_space(2.0);
                    }
                }

                // A race's personal best, and an imported rival.
                if card.race_id.is_some() {
                    ui.add_space(8.0);
                    if let Some(t) = card.best_ticks {
                        ui.label(
                            egui::RichText::new(format!("Your best: {}", crate::trials::format_time(t)))
                                .size(15.0)
                                .color(DONE_GREEN),
                        );
                    }
                    if let Some((label, t)) = &card.rival {
                        ui.label(
                            egui::RichText::new(format!("Rival: {label} \u{2014} {}", crate::trials::format_time(*t)))
                                .size(14.0)
                                .color(RIVAL_ORANGE),
                        );
                    }
                }

                // Play, big.
                ui.add_space(18.0);
                let (play_rect, play) = ui.allocate_exact_size(
                    egui::vec2(w.min(280.0), 58.0),
                    egui::Sense::click(),
                );
                let green = if play.hovered() || play.has_focus() {
                    egui::Color32::from_rgb(58, 128, 58)
                } else {
                    egui::Color32::from_rgb(45, 107, 45)
                };
                ui.painter().rect_filled(play_rect, egui::CornerRadius::same(10), green);
                ui.painter().text(
                    play_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Play",
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
                if fresh {
                    play.request_focus();
                }
                if play.has_focus() {
                    ui.painter().rect_stroke(
                        play_rect.expand(2.0),
                        10.0,
                        egui::Stroke::new(2.0_f32, brand::LANTERN),
                        egui::StrokeKind::Outside,
                    );
                }
                if play.clicked() {
                    out.action = Some(MenuAction::PlayScenario { def: Box::new(card.def.clone()) });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(egui::RichText::new("How it works").size(13.0).underline().color(MUTED)).frame(false))
                        .clicked()
                    {
                        out.how_it_works = true;
                    }
                    if let Some(id) = card.race_id {
                        ui.add_space(12.0);
                        if card.best_ticks.is_some()
                            && ui
                                .add(egui::Button::new(egui::RichText::new("Export ghost").size(13.0).color(MUTED)).frame(false))
                                .on_hover_text("Save your best run as a file a friend can race against.")
                                .clicked()
                        {
                            out.action = Some(MenuAction::ExportGhost { trial_id: id.to_string() });
                        }
                        ui.add_space(8.0);
                        if ui
                            .add(egui::Button::new(egui::RichText::new("Import ghost").size(13.0).color(MUTED)).frame(false))
                            .on_hover_text("Open a friend's ghost file: it becomes the rival you race here.")
                            .clicked()
                        {
                            out.action = Some(MenuAction::ImportGhost);
                        }
                    }
                });
                ui.add_space(20.0);
            });
        });
    });
    out
}

/// Draw the gallery full-screen for one frame. `modal_open` is true while the
/// "How it works" modal is up: the gallery then ignores the keyboard so Escape
/// closes only the modal.
pub fn draw(
    ctx: &egui::Context,
    st: &mut GalleryState,
    icons: &[egui::TextureId],
    modal_open: bool,
) -> GalleryOutput {
    let mut out = GalleryOutput { action: MenuAction::None, how_it_works: None };
    let Some(row_lens) = st.built.as_ref().map(|rows| rows.iter().map(|r| r.cards.len()).collect::<Vec<_>>()) else {
        return out;
    };

    // Keyboard. Arrows move between and across rows, Enter opens, Escape backs out.
    if !modal_open {
        let none = egui::Modifiers::NONE;
        if st.hero.is_some() {
            if ctx.input_mut(|i| i.consume_key(none, egui::Key::Escape)) {
                st.hero = None;
            }
        } else {
            let (esc, enter, left, right, up, down) = ctx.input_mut(|i| {
                (
                    i.consume_key(none, egui::Key::Escape),
                    i.consume_key(none, egui::Key::Enter),
                    i.consume_key(none, egui::Key::ArrowLeft),
                    i.consume_key(none, egui::Key::ArrowRight),
                    i.consume_key(none, egui::Key::ArrowUp),
                    i.consume_key(none, egui::Key::ArrowDown),
                )
            });
            if esc {
                st.close();
                return out;
            }
            let (mut r, mut c) = st.focus;
            let last_row = row_lens.len().saturating_sub(1);
            if left {
                c = c.saturating_sub(1);
            }
            if right {
                c = (c + 1).min(row_lens[r].saturating_sub(1));
            }
            if up {
                r = r.saturating_sub(1);
            }
            if down {
                r = (r + 1).min(last_row);
            }
            c = c.min(row_lens[r].saturating_sub(1));
            if (r, c) != st.focus {
                st.focus = (r, c);
                st.scroll_to_focus = true;
            }
            if enter {
                st.hero = Some(st.focus);
                st.hero_fresh = true;
            }
        }
    }

    let mut back = false;
    let mut shelf_result = ShelfResult::default();
    let mut hero_result = HeroResult::default();
    let hero_pos = st.hero;
    let fresh = std::mem::take(&mut st.hero_fresh);
    let scroll_to_focus = std::mem::take(&mut st.scroll_to_focus);
    let focus = st.focus;

    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(brand::DEEP_FRONTIER).inner_margin(egui::Margin::symmetric(20, 14)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("\u{2039}  Back").size(15.0).color(TEXT))
                            .min_size(egui::vec2(0.0, 34.0))
                            .corner_radius(egui::CornerRadius::same(6)),
                    )
                    .clicked()
                {
                    back = true;
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Trials").size(26.0).strong().color(brand::COPPER));
            });
            let Some(rows) = st.built.as_ref() else { return };
            match hero_pos.and_then(|(r, c)| rows.get(r).and_then(|row| row.cards.get(c))) {
                Some(card) => hero_result = draw_hero(ui, card, icons, fresh),
                None => {
                    ui.label(egui::RichText::new("Pick one to look into it, then play.").size(13.0).color(MUTED));
                    shelf_result = draw_shelves(ui, rows, focus, scroll_to_focus, icons);
                }
            }
        });

    if back {
        if st.hero.is_some() {
            st.hero = None;
        } else {
            st.close();
            return out;
        }
    }
    if let Some(pos) = shelf_result.hovered {
        st.focus = pos;
    }
    if let Some(pos) = shelf_result.opened {
        st.focus = pos;
        st.hero = Some(pos);
        st.hero_fresh = true;
    }
    if let Some(a) = hero_result.action {
        out.action = a;
    }
    if hero_result.how_it_works
        && let Some(card) = hero_pos.and_then(|(r, c)| st.built.as_ref()?.get(r)?.cards.get(c))
    {
        out.how_it_works = Some(card.key.clone());
    }
    out
}

// ---------------------------------------------------------------------------
// The lobby tile that opens the gallery
// ---------------------------------------------------------------------------

/// The big "Trials" tile in the world-list area (a full-width button on narrow
/// screens). Returns true when pressed.
pub fn draw_trials_tile(ui: &mut egui::Ui, narrow: bool) -> bool {
    let width = if narrow { ui.available_width() } else { ui.available_width().min(420.0) };
    let height = if narrow { 64.0 } else { 78.0 };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let painter = ui.painter();
    let cr = egui::CornerRadius::same(12);
    let hot = resp.hovered() || resp.has_focus();
    painter.rect_filled(rect, cr, if hot { brand::SURFACE_HOVER } else { brand::SURFACE });
    // The four shelf colours as a band down the left edge.
    let shelves = [TrialType::Make, TrialType::Tend, TrialType::Tech, TrialType::Brave];
    let seg = (height - 20.0) / shelves.len() as f32;
    for (i, t) in shelves.iter().enumerate() {
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.min.x + 12.0, rect.min.y + 10.0 + seg * i as f32),
                egui::vec2(6.0, seg - 2.0),
            ),
            egui::CornerRadius::same(3),
            rgb(t.color_rgb()),
        );
    }
    painter.text(
        egui::pos2(rect.min.x + 32.0, rect.min.y + height * 0.34),
        egui::Align2::LEFT_CENTER,
        "Trials",
        egui::FontId::proportional(if narrow { 22.0 } else { 26.0 }),
        brand::COPPER,
    );
    painter.text(
        egui::pos2(rect.min.x + 32.0, rect.min.y + height * 0.72),
        egui::Align2::LEFT_CENTER,
        "Little challenges. Pick one and play.",
        egui::FontId::proportional(12.5),
        MUTED,
    );
    painter.text(
        egui::pos2(rect.max.x - 18.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "\u{203A}",
        egui::FontId::proportional(30.0),
        brand::COPPER,
    );
    painter.rect_stroke(
        rect,
        cr,
        egui::Stroke::new(if resp.has_focus() { 2.0_f32 } else { 1.0_f32 }, if resp.has_focus() { brand::LANTERN } else { brand::BORDER }),
        egui::StrokeKind::Inside,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_counts_or_money_words_in_the_gallery_copy() {
        // The gallery never shows how many there are, nor frames play as money.
        for src in [include_str!("trials_gallery_ui.rs"), include_str!("trials_gallery.rs")] {
            let body = src.split("#[cfg(test)]").next().unwrap();
            let hits = crate::copy_lint::banned_in(body, crate::copy_lint::BANNED_MONEY_WORDS);
            assert!(hits.is_empty(), "money words in gallery copy: {hits:?}");
            for bad in [" of {", "{}%", "{}/{}", "{} of"] {
                assert!(!body.contains(bad), "gallery copy must not show counts ({bad})");
            }
        }
    }
}
