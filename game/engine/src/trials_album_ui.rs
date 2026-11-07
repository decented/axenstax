//! The Trials album: every kept postcard, full screen, grouped by shelf in the
//! gallery's order (Make, Tend, Tech, Brave, Against the clock), newest first.
//! Click one to see it large; Back or Escape steps out. Like the gallery it
//! shows no counts, totals or "x of y": it fills like a scrapbook.
//! Design: `docs/vision/trials-two-acts.md` §C.1 principle 2 and §C.2 "Album".

use crate::brand;
use crate::trials_gallery_ui::{cover_uv, GalleryState};

/// The album's own state, held on `GalleryState` while it is open.
#[derive(Default)]
pub struct AlbumView {
    /// The postcard shown large (an index into the gallery's postcards).
    pub(crate) enlarged: Option<usize>,
}

const THUMB_W: f32 = 240.0;
const THUMB_H: f32 = 135.0;
const EMPTY_LINE: &str = "Finish a trial and its postcard lands here.";

fn rgb(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

/// A postcard in its white border, with a soft shadow. `lift` 0..1 grows it.
fn paint_postcard(painter: &egui::Painter, rect: egui::Rect, tex: Option<egui::TextureId>, lift: f32) {
    let r = egui::Rect::from_center_size(rect.center(), rect.size() * (1.0 + 0.04 * lift));
    let border = (r.width() * 0.025).clamp(4.0, 14.0);
    let cr = egui::CornerRadius::same(4);
    painter.add(
        egui::Shadow {
            offset: [0, (3.0 + 5.0 * lift) as i8],
            blur: (8.0 + 10.0 * lift) as u8,
            spread: 0,
            color: egui::Color32::from_black_alpha((80.0 + 60.0 * lift) as u8),
        }
        .as_shape(r, cr),
    );
    painter.rect_filled(r, cr, egui::Color32::from_rgb(245, 240, 228));
    let inner = r.shrink(border);
    match tex {
        Some(t) => {
            painter.add(egui::Shape::Rect(
                egui::epaint::RectShape::filled(inner, egui::CornerRadius::same(2), egui::Color32::WHITE)
                    .with_texture(t, cover_uv(inner)),
            ));
        }
        None => {
            painter.rect_filled(inner, egui::CornerRadius::same(2), egui::Color32::from_rgb(58, 66, 74));
        }
    }
}

/// Draw the album full-screen for one frame (the gallery hands over to it
/// while it is open). `modal_open` mutes the keyboard as the gallery does.
pub fn draw(ctx: &egui::Context, st: &mut GalleryState, modal_open: bool) {
    let Some(view) = st.album.as_ref() else { return };
    let enlarged = view.enlarged.filter(|&i| i < st.postcards.len());

    if !modal_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        step_back(st, enlarged.is_some());
        return;
    }

    // Textures and captions for what this frame shows.
    let groups = crate::postcard::album_groups(&st.postcards);
    let shown: Vec<usize> = match enlarged {
        Some(i) => vec![i],
        None => groups.iter().flat_map(|(_, idx)| idx.iter().copied()).collect(),
    };
    let mut tex = std::collections::HashMap::new();
    for i in shown {
        tex.insert(i, st.postcard_texture(ctx, i));
    }
    let caption = |st: &GalleryState, i: usize| -> String {
        st.postcards.get(i).and_then(|p| st.title_of(&p.token)).unwrap_or_default()
    };

    let mut back = false;
    let mut opened: Option<usize> = None;
    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(brand::DEEP_FRONTIER).inner_margin(egui::Margin::symmetric(20, 14)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("\u{2039}  Back").size(15.0).color(brand::STONE))
                            .min_size(egui::vec2(0.0, 34.0))
                            .corner_radius(egui::CornerRadius::same(6)),
                    )
                    .clicked()
                {
                    back = true;
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Album").size(26.0).strong().color(brand::COPPER));
            });

            if let Some(i) = enlarged {
                // One postcard, as large as fits.
                ui.add_space(12.0);
                let avail = ui.available_size();
                let w = avail.x.min((avail.y - 60.0).max(120.0) * 16.0 / 9.0).min(960.0);
                let h = w * 9.0 / 16.0;
                ui.vertical_centered(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
                    paint_postcard(ui.painter(), rect, tex.get(&i).copied().flatten(), 0.0);
                    if resp.clicked() {
                        back = true;
                    }
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(caption(st, i)).size(20.0).color(brand::STONE));
                });
                return;
            }

            if groups.is_empty() {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(EMPTY_LINE).size(18.0).color(brand::MUTED_TEXT));
                });
                return;
            }

            egui::ScrollArea::vertical().id_salt("trials_album").auto_shrink([false, false]).show(ui, |ui| {
                for (kind, idx) in &groups {
                    let colour = rgb(kind.color_rgb());
                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new(format!("{}  {}", kind.glyph(), kind.title()))
                            .size(20.0)
                            .strong()
                            .color(colour),
                    );
                    ui.add_space(8.0);
                    let gap = 18.0;
                    let cols = (((ui.available_width() + gap) / (THUMB_W + gap)).floor() as usize).max(1);
                    for line in idx.chunks(cols) {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            for &i in line {
                                ui.vertical(|ui| {
                                    ui.set_width(THUMB_W);
                                    let (rect, resp) =
                                        ui.allocate_exact_size(egui::vec2(THUMB_W, THUMB_H), egui::Sense::click());
                                    let lift = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                                    paint_postcard(ui.painter(), rect, tex.get(&i).copied().flatten(), lift);
                                    if resp.clicked() {
                                        opened = Some(i);
                                    }
                                    resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                                    ui.add_space(4.0);
                                    ui.label(egui::RichText::new(caption(st, i)).size(13.0).color(brand::STONE));
                                });
                            }
                        });
                        ui.add_space(10.0);
                    }
                }
                ui.add_space(20.0);
            });
        });

    if back {
        step_back(st, enlarged.is_some());
    } else if let (Some(i), Some(view)) = (opened, st.album.as_mut()) {
        view.enlarged = Some(i);
    }
}

/// Back: from a large postcard to the album, from the album to the gallery.
fn step_back(st: &mut GalleryState, enlarged: bool) {
    if enlarged {
        if let Some(view) = st.album.as_mut() {
            view.enlarged = None;
        }
    } else {
        st.album = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_album_line_is_friendly_and_shows_no_count() {
        assert!(EMPTY_LINE.contains("postcard"));
        assert!(!EMPTY_LINE.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn back_steps_out_of_a_large_postcard_then_the_album() {
        let mut st = GalleryState::default();
        st.open_with(crate::trials::TrialBests::default(), crate::postcard::fixtures());
        st.open_album();
        st.album.as_mut().unwrap().enlarged = Some(0);
        step_back(&mut st, true);
        assert!(st.album.as_ref().is_some_and(|a| a.enlarged.is_none()));
        step_back(&mut st, false);
        assert!(st.album.is_none());
        assert!(st.is_open(), "the gallery stays open beneath");
    }
}
