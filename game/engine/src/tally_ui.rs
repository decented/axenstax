//! Living world S5 — the grown-up panel: the Tally and world bloom, drawn in the
//! F3 overlay's top-right slot (where the Deepslate Reserve gauge draws) when
//! Settings -> "Grown-up panel (F3)" is on (decision D16, spec section 5).
//!
//! The figures are about the world, never a person. Numbers are fine here: this
//! is the adult panel, and a child's view of bloom stays words and colour. Every
//! string is covered by the `*_ui.rs` copy lint, so it says "trades", never a
//! money word.

use crate::bloom::{WorldSummary, WorldTerms};
use crate::screen::ViewportRect;
use crate::tally::Tally;

/// The plain sentence at the foot of the panel.
pub const PANEL_SENTENCE: &str = "The Tally counts what was taken and made. Bloom is how alive the places are.";

/// How far down the panel starts when the Reserve gauge is showing in the same
/// slot: the gauge's height and a gap.
pub const BELOW_RESERVE_GAUGE: f32 = 170.0;

/// One row of the panel: what it counts, and the figure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

fn row(label: &'static str, value: impl Into<String>) -> Row {
    Row { label, value: value.into() }
}

fn percent(x: f32) -> String {
    format!("{}%", (x.clamp(0.0, 1.0) * 100.0).round() as u32)
}

/// The Tally rows: each counter with a plain label, then the stock.
pub fn tally_rows(tally: &Tally, stock: u64) -> Vec<Row> {
    vec![
        row("Blocks mined", tally.mined.to_string()),
        row("Blocks placed", tally.placed.to_string()),
        row("Items crafted", tally.crafted.to_string()),
        row("Items smelted", tally.smelted.to_string()),
        row("Vendor trades", tally.trades.to_string()),
        row("Stock in containers", stock.to_string()),
    ]
}

/// What shows where a figure is absent or this client cannot know it.
const ABSENT: &str = "\u{2014}";

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The Ideas value: designs built at least once and the builds in all.
pub fn ideas_text(designs: u32, builds: u32) -> String {
    format!("{}, {}", plural(designs, "design", "designs"), plural(builds, "build", "builds"))
}

/// The world-bloom rows: the mean level and the four named terms (life, ideas,
/// people together, time), then the places measured. World totals only.
///
/// A joined client keeps no Plan-build table and no use record (the host does),
/// so it shows "\u{2014}" for ideas and time; its life and people are what it can
/// compute from its own copy of the world. Time also shows "\u{2014}" where no
/// place has a use record yet.
pub fn bloom_rows(bloom: &WorldSummary, terms: &WorldTerms, joined: bool) -> Vec<Row> {
    let ideas = if joined { ABSENT.to_string() } else { ideas_text(terms.ideas.0, terms.ideas.1) };
    let ideas_row = row("Ideas", ideas);
    if bloom.regions == 0 {
        return vec![row("Places measured", "none yet"), ideas_row];
    }
    let time = match terms.time {
        Some(u) if !joined => percent(u),
        _ => ABSENT.to_string(),
    };
    vec![
        row("Bloom, mean level", percent(bloom.mean_level)),
        row("Life", percent(terms.life)),
        ideas_row,
        row("People together", percent(terms.people)),
        row("Time", time),
        row("Places measured", bloom.regions.to_string()),
    ]
}

/// Draw the panel with its top edge `y_offset` below the viewport's top.
/// `y_offset` is 0 when it has the slot to itself, and clears the Reserve
/// gauge when that is showing too.
pub fn draw_grownup_panel(
    ctx: &egui::Context,
    viewport: &ViewportRect,
    player_index: usize,
    tally: &Tally,
    stock: u64,
    bloom: &WorldSummary,
    terms: &WorldTerms,
    joined: bool,
    y_offset: f32,
) {
    let panel_w = 260.0_f32;
    let panel_x = viewport.x as f32 + viewport.width as f32 - panel_w - 12.0;
    let panel_y = viewport.y as f32 + 12.0 + y_offset;
    let heading = egui::Color32::from_rgb(220, 200, 120);
    let body = egui::Color32::from_rgb(225, 225, 230);
    let faint = egui::Color32::from_rgb(160, 160, 170);

    egui::Area::new(egui::Id::new(("grownup_panel", player_index)))
        .fixed_pos(egui::pos2(panel_x, panel_y))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(0, 0, 0, 180))
                .inner_margin(egui::Margin::same(10))
                .corner_radius(egui::CornerRadius::same(4))
                .show(ui, |ui| {
                    ui.set_min_width(panel_w - 20.0);
                    ui.set_max_width(panel_w - 20.0);
                    for (title, rows) in [("The Tally", tally_rows(tally, stock)), ("World bloom", bloom_rows(bloom, terms, joined))] {
                        ui.label(egui::RichText::new(title).color(heading).strong().size(13.0));
                        for r in rows {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(r.label).color(faint).size(12.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(egui::RichText::new(r.value).color(body).size(12.0));
                                });
                            });
                        }
                        ui.add_space(6.0);
                    }
                    ui.label(egui::RichText::new(PANEL_SENTENCE).color(faint).size(11.0));
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> WorldSummary {
        WorldSummary { mean_level: 0.62, mean_eco: 0.7, mean_people: 0.2, regions: 12 }
    }

    fn terms() -> WorldTerms {
        WorldTerms { life: 0.7, ideas: (4, 11), people: 0.2, time: Some(0.35) }
    }

    fn get(rows: &[Row], label: &str) -> Option<String> {
        rows.iter().find(|r| r.label == label).map(|r| r.value.clone())
    }

    #[test]
    fn the_tally_rows_name_each_counter_and_the_stock() {
        let t = Tally { mined: 5, placed: 4, crafted: 3, smelted: 2, trades: 1 };
        let rows = tally_rows(&t, 77);
        let get = |label: &str| get(&rows, label);
        assert_eq!(get("Blocks mined").as_deref(), Some("5"));
        assert_eq!(get("Blocks placed").as_deref(), Some("4"));
        assert_eq!(get("Items crafted").as_deref(), Some("3"));
        assert_eq!(get("Items smelted").as_deref(), Some("2"));
        assert_eq!(get("Vendor trades").as_deref(), Some("1"));
        assert_eq!(get("Stock in containers").as_deref(), Some("77"));
    }

    #[test]
    fn the_bloom_rows_show_the_mean_and_the_four_named_terms() {
        let rows = bloom_rows(&summary(), &terms(), false);
        assert_eq!(get(&rows, "Bloom, mean level").as_deref(), Some("62%"));
        assert_eq!(get(&rows, "Life").as_deref(), Some("70%"));
        assert_eq!(get(&rows, "Ideas").as_deref(), Some("4 designs, 11 builds"));
        assert_eq!(get(&rows, "People together").as_deref(), Some("20%"));
        assert_eq!(get(&rows, "Time").as_deref(), Some("35%"));
        assert_eq!(get(&rows, "Places measured").as_deref(), Some("12"));
        // The four sit in the order the grown-up reads them, under the level.
        let labels: Vec<&str> = rows.iter().map(|r| r.label).collect();
        assert_eq!(
            labels,
            ["Bloom, mean level", "Life", "Ideas", "People together", "Time", "Places measured"]
        );
    }

    #[test]
    fn a_term_with_nothing_behind_it_shows_a_dash() {
        let none = WorldTerms { time: None, ..terms() };
        assert_eq!(get(&bloom_rows(&summary(), &none, false), "Time").as_deref(), Some("\u{2014}"));
        // A single design built once is singular.
        assert_eq!(ideas_text(1, 1), "1 design, 1 build");
        assert_eq!(ideas_text(0, 0), "0 designs, 0 builds");
    }

    #[test]
    fn a_joined_client_shows_a_dash_for_ideas_and_time_but_keeps_life_and_people() {
        // Even if the client somehow held figures, it must not present them.
        let rows = bloom_rows(&summary(), &terms(), true);
        assert_eq!(get(&rows, "Ideas").as_deref(), Some("\u{2014}"));
        assert_eq!(get(&rows, "Time").as_deref(), Some("\u{2014}"));
        assert_eq!(get(&rows, "Life").as_deref(), Some("70%"));
        assert_eq!(get(&rows, "People together").as_deref(), Some("20%"));
        let none = bloom_rows(&WorldSummary::default(), &WorldTerms::default(), true);
        assert_eq!(get(&none, "Ideas").as_deref(), Some("\u{2014}"));
    }

    #[test]
    fn with_no_region_scanned_the_panel_says_so_instead_of_showing_zeros() {
        let rows = bloom_rows(&WorldSummary::default(), &WorldTerms::default(), false);
        assert_eq!(get(&rows, "Places measured").as_deref(), Some("none yet"));
        assert_eq!(get(&rows, "Life"), None);
        assert_eq!(get(&rows, "Ideas").as_deref(), Some("0 designs, 0 builds"));
    }

    #[test]
    fn the_panel_text_is_plain_and_money_free() {
        let mut all = String::new();
        for r in bloom_rows(&summary(), &terms(), false).into_iter().chain(bloom_rows(&summary(), &terms(), true)) {
            all.push_str(r.label);
            all.push(' ');
            all.push_str(&r.value);
            all.push(' ');
        }
        assert!(crate::copy_lint::banned_in(&all, crate::copy_lint::BANNED_MONEY_WORDS).is_empty());
    }

    #[test]
    fn the_percentages_clamp() {
        assert_eq!(percent(1.7), "100%");
        assert_eq!(percent(-0.2), "0%");
    }
}
