//! Shared loading-screen content + pure helpers.
//!
//! The card list is the SAME file the web bundle-loader fetches
//! (`game/engine/assets/loading_tips.json`), baked in here via `include_str!`
//! so the engine needs no network at world-load time. The web loader
//! (`tools/sites/game/static/loading-screen.js`) reads a trunk-copied copy of
//! the same file — one source of truth, no drift.
//!
//! Design: `docs/superpowers/specs/2026-06-17-loading-screen-overhaul-design.md`.

use serde::Deserialize;
use web_time::Instant;

/// Canonical loading-card content, baked at compile time.
pub const CARDS_JSON: &str = include_str!("../assets/loading_tips.json");

/// Per-card display before rotating to the next.
pub const CARD_HOLD_SECS: f32 = 6.0;
/// Floor on total loading-screen time so even a fast load is readable.
pub const MIN_DISPLAY_SECS: f32 = 5.0;
/// Columns generated + meshed per frame in the incremental loader.
pub const LOAD_BUDGET_PER_FRAME: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardKind {
    /// How-to-play guidance.
    Tip,
    /// "What's new — needs testing" note.
    New,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoadingCard {
    pub kind: CardKind,
    pub title: String,
    pub body: String,
}

/// Parse the bundled card JSON.
pub fn parse_cards(json: &str) -> Result<Vec<LoadingCard>, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

/// Deterministic Fisher–Yates permutation of `0..len` using a tiny SplitMix64,
/// re-rolled while the first element equals `prev_last` so no card repeats
/// across a reshuffle boundary. `len <= 1` short-circuits.
pub fn shuffle_no_repeat(len: usize, seed: u64, prev_last: Option<usize>) -> Vec<usize> {
    if len <= 1 {
        return (0..len).collect();
    }
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for attempt in 0..8 {
        let mut v: Vec<usize> = (0..len).collect();
        for i in (1..len).rev() {
            let j = (next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
        if prev_last != Some(v[0]) || attempt == 7 {
            return v;
        }
    }
    (0..len).collect()
}

/// Fraction of the load done, in `[0, 1]`. `total == 0` → `1.0` (nothing to do).
pub fn load_progress(total: usize, remaining: usize) -> f32 {
    if total == 0 {
        return 1.0;
    }
    let done = total.saturating_sub(remaining) as f32;
    (done / total as f32).clamp(0.0, 1.0)
}

/// Transition to Playing only once the work queue is empty AND the minimum
/// readable time has elapsed.
pub fn should_finish(queue_empty: bool, elapsed_secs: f32) -> bool {
    queue_empty && elapsed_secs >= MIN_DISPLAY_SECS
}

/// Animated loading-screen state for the in-engine world-entry screen.
///
/// Holds a shuffled card order + the timing needed to rotate cards and gate the
/// minimum display time. `setup_done` / `total` are filled by the game loop once
/// the one-shot `begin_load` has built the column queue.
pub struct LoadingState {
    pub started_at: Instant,
    /// False until the screen has been painted at least once. Lets the game loop
    /// show the screen for one frame BEFORE running the (briefly blocking)
    /// one-shot `begin_load`, so the player sees it instantly.
    pub painted: bool,
    /// False until the one-shot `begin_load` has run.
    pub setup_done: bool,
    /// Columns to process (set after `begin_load`); drives the progress bar.
    pub total: usize,
    cards: Vec<LoadingCard>,
    order: Vec<usize>,
    pos: usize,
    last_swap: Instant,
}

impl LoadingState {
    pub fn new(world_name: String) -> Self {
        let cards = parse_cards(CARDS_JSON).unwrap_or_default();
        // Seed the first shuffle off the world name so different worlds open on
        // different tips, but a given world is stable within a session.
        let seed = world_name
            .bytes()
            .fold(0xABCD_1234_u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
        let order = shuffle_no_repeat(cards.len(), seed, None);
        let now = Instant::now();
        Self {
            started_at: now,
            painted: false,
            setup_done: false,
            total: 0,
            cards,
            order,
            pos: 0,
            last_swap: now,
        }
    }

    pub fn elapsed_secs(&self) -> f32 {
        self.started_at.elapsed().as_secs_f32()
    }

    /// Rotate to the next card once `CARD_HOLD_SECS` has passed; reshuffle (with
    /// no immediate repeat) when the order is exhausted.
    pub fn maybe_rotate(&mut self) {
        if self.cards.len() <= 1 {
            return;
        }
        if self.last_swap.elapsed().as_secs_f32() >= CARD_HOLD_SECS {
            let last = self.order.get(self.pos).copied();
            self.pos += 1;
            if self.pos >= self.order.len() {
                let seed = self.started_at.elapsed().as_nanos() as u64 ^ 0x5151;
                self.order = shuffle_no_repeat(self.cards.len(), seed, last);
                self.pos = 0;
            }
            self.last_swap = Instant::now();
        }
    }

    pub fn current_card(&self) -> Option<&LoadingCard> {
        self.order.get(self.pos).and_then(|&i| self.cards.get(i))
    }
}

// ---------------------------------------------------------------------------
// egui rendering — the in-engine world-load screen (approved "Voxel World hero
// + rotating card beneath" direction). Painter-driven; structure mirrors
// `splash_ui::draw_splash` (bg Area behind a transparent CentralPanel).
// ---------------------------------------------------------------------------

const BG: egui::Color32 = egui::Color32::from_rgb(13, 18, 32);

fn lighten(c: egui::Color32, f: f32) -> egui::Color32 {
    let m = |v: u8| (v as f32 + (255.0 - v as f32) * f) as u8;
    egui::Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}
fn darken(c: egui::Color32, f: f32) -> egui::Color32 {
    let m = |v: u8| (v as f32 * (1.0 - f)) as u8;
    egui::Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// One shaded voxel cube at `top_left`.
fn cube(painter: &egui::Painter, top_left: egui::Pos2, size: f32, base: egui::Color32) {
    let r = egui::Rect::from_min_size(top_left, egui::vec2(size, size));
    painter.rect_filled(r, 5.0, base);
    // Top highlight + bottom shadow strips for a faux-3D voxel look.
    painter.rect_filled(
        egui::Rect::from_min_size(top_left, egui::vec2(size, size * 0.26)),
        0.0,
        lighten(base, 0.22),
    );
    painter.rect_filled(
        egui::Rect::from_min_size(top_left + egui::vec2(0.0, size * 0.74), egui::vec2(size, size * 0.26)),
        0.0,
        darken(base, 0.30),
    );
}

/// The floating voxel diorama — 2 cubes over 3, centred on `center`.
fn paint_voxel_hero(painter: &egui::Painter, center: egui::Pos2) {
    const S: f32 = 30.0;
    const G: f32 = 5.0;
    let green = egui::Color32::from_rgb(111, 174, 90);
    let orange = egui::Color32::from_rgb(232, 134, 46);
    let brown = egui::Color32::from_rgb(138, 113, 88);
    let gold = egui::Color32::from_rgb(240, 198, 116);
    // Top row (2 cubes).
    let top_w = 2.0 * S + G;
    let top_x = center.x - top_w / 2.0;
    let top_y = center.y - S - G / 2.0;
    cube(painter, egui::pos2(top_x, top_y), S, green);
    cube(painter, egui::pos2(top_x + S + G, top_y), S, orange);
    // Bottom row (3 cubes).
    let bot_w = 3.0 * S + 2.0 * G;
    let bot_x = center.x - bot_w / 2.0;
    let bot_y = center.y + G / 2.0;
    cube(painter, egui::pos2(bot_x, bot_y), S, brown);
    cube(painter, egui::pos2(bot_x + S + G, bot_y), S, brown);
    cube(painter, egui::pos2(bot_x + 2.0 * (S + G), bot_y), S, gold);
}

/// Draw the full-screen world-load screen. `progress` is `[0,1]`.
pub fn draw_loading_screen(ctx: &egui::Context, state: &LoadingState, progress: f32) {
    ctx.request_repaint(); // animate continuously
    let t = ctx.input(|i| i.time) as f32;
    let bob = (t * 1.8).sin() * 6.0;

    // Dark backdrop, behind the content panel (default order, non-interactable).
    egui::Area::new(egui::Id::new("loading_screen_bg"))
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0.0, BG);
        });

    // `.show(ctx, ..)` is deprecated in favour of `.show_inside(ui, ..)`, but this
    // is a genuine top-level panel (no enclosing Ui) — egui 0.34 has no
    // non-deprecated top-level entry point for CentralPanel.
    #[allow(deprecated)]
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::TRANSPARENT))
        .show(ctx, |ui| {
            let available = ui.available_size();
            ui.vertical_centered(|ui| {
                ui.add_space((available.y / 2.0 - 165.0).max(20.0));

                // Voxel hero (floats with `bob`).
                let hero_center =
                    ui.cursor().left_top() + egui::vec2(available.x / 2.0, 22.0 + bob);
                paint_voxel_hero(ui.painter(), hero_center);
                ui.add_space(70.0);

                // Wordmark + tagline.
                ui.label(
                    egui::RichText::new("AXE'N'STAX")
                        .size(40.0)
                        .color(egui::Color32::from_rgb(243, 230, 207))
                        .strong()
                        .extra_letter_spacing(3.0),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("PROOF  OF  PLAY")
                        .size(12.0)
                        .color(egui::Color32::from_rgb(212, 160, 68))
                        .extra_letter_spacing(7.0),
                );
                ui.add_space(22.0);

                // Progress bar (real progress).
                let bar_w = 300.0;
                let bar_h = 7.0;
                let bar_rect = egui::Rect::from_center_size(
                    ui.cursor().left_top() + egui::vec2(available.x / 2.0, bar_h / 2.0),
                    egui::vec2(bar_w, bar_h),
                );
                ui.painter().rect_filled(
                    bar_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20),
                );
                let fill_rect = egui::Rect::from_min_size(
                    bar_rect.left_top(),
                    egui::vec2(bar_w * progress.clamp(0.0, 1.0), bar_h),
                );
                ui.painter()
                    .rect_filled(fill_rect, 4.0, egui::Color32::from_rgb(240, 198, 116));
                ui.add_space(28.0);

                // Rotating content card.
                if let Some(card) = state.current_card() {
                    let (label, col) = match card.kind {
                        CardKind::Tip => ("\u{1F4A1} TIP", egui::Color32::from_rgb(127, 179, 255)),
                        CardKind::New => (
                            "\u{2728} NEW \u{2014} NEEDS TESTING",
                            egui::Color32::from_rgb(111, 174, 90),
                        ),
                    };
                    let card_w = 520.0_f32.min(available.x - 40.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(card_w, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::Frame::new()
                                .fill(egui::Color32::from_rgb(20, 27, 46))
                                .stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(40, 51, 86)))
                                .corner_radius(egui::CornerRadius::same(12))
                                .inner_margin(egui::Margin::symmetric(20, 16))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        egui::RichText::new(label)
                                            .size(11.0)
                                            .color(col)
                                            .strong()
                                            .extra_letter_spacing(2.0),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new(&card.title)
                                            .size(16.0)
                                            .color(egui::Color32::from_rgb(238, 242, 251))
                                            .strong(),
                                    );
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new(&card.body)
                                            .size(13.5)
                                            .color(egui::Color32::from_rgb(176, 186, 205)),
                                    );
                                });
                        },
                    );
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_parse_and_are_valid() {
        let cards = parse_cards(CARDS_JSON).expect("bundled cards parse");
        assert!(cards.len() >= 10, "expected a decent starter set");
        for c in &cards {
            assert!(!c.title.trim().is_empty());
            assert!(!c.body.trim().is_empty());
            assert!(matches!(c.kind, CardKind::Tip | CardKind::New));
        }
    }

    #[test]
    fn diamond_age_lore_tips_present() {
        // Moonshot Phase A (theme capture) — the Diamond-Age lore cards seed
        // the founding-myth setting diegetically. Titles are the stable key;
        // body copy is owner-tunable.
        let cards = parse_cards(CARDS_JSON).expect("bundled cards parse");
        for title in ["The Age of Diamonds", "The first Satori", "A world to found"] {
            assert!(
                cards.iter().any(|c| c.title == title),
                "missing Diamond-Age lore tip '{title}'"
            );
        }
    }

    #[test]
    fn loading_cards_never_carry_money_or_bitcoin_words() {
        // Moonshot guardrail (north-star §1/§9): the play surface never says
        // "Bitcoin"/"crypto"/"sats"/"earn" — money in-world is just money.
        // Whole-word match so e.g. "learn" doesn't trip "earn".
        let cards = parse_cards(CARDS_JSON).expect("bundled cards parse");
        for c in &cards {
            let text = format!("{} {}", c.title, c.body).to_lowercase();
            for banned in ["bitcoin", "crypto", "sats", "earn", "earning", "earned"] {
                let hit = text
                    .split(|ch: char| !ch.is_alphanumeric())
                    .any(|word| word == banned);
                assert!(!hit, "loading card '{}' contains banned word '{banned}'", c.title);
            }
        }
    }

    #[test]
    fn shuffle_no_immediate_repeat() {
        for seed in 0u64..50 {
            let order = shuffle_no_repeat(8, seed, Some(3));
            assert_eq!(order.len(), 8);
            assert_ne!(order[0], 3, "seed {seed} repeated the last card");
            let mut sorted = order.clone();
            sorted.sort();
            assert_eq!(sorted, (0..8).collect::<Vec<_>>(), "not a permutation");
        }
    }

    #[test]
    fn shuffle_handles_degenerate_lengths() {
        assert_eq!(shuffle_no_repeat(0, 1, None), Vec::<usize>::new());
        assert_eq!(shuffle_no_repeat(1, 1, Some(0)), vec![0]);
    }

    #[test]
    fn progress_is_clamped_and_correct() {
        assert_eq!(load_progress(0, 0), 1.0);
        assert_eq!(load_progress(10, 10), 0.0);
        assert_eq!(load_progress(10, 0), 1.0);
        assert!((load_progress(4, 1) - 0.75).abs() < 1e-6);
        // remaining > total can't happen, but stay clamped if it ever did.
        assert_eq!(load_progress(4, 9), 0.0);
    }

    #[test]
    fn finish_gates_on_both_queue_and_time() {
        assert!(!should_finish(true, 1.0));
        assert!(!should_finish(false, 9.0));
        assert!(should_finish(true, MIN_DISPLAY_SECS + 0.1));
    }
}
