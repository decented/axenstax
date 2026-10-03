//! Showcase / kiosk-containment mode — the visitor loop on top of the Phase 1
//! Exhibit primitive. PURE core only: config-flag parsing, the exit-gating
//! predicate ("should exit be a one-step dead-end, no lobby back-door?"), the
//! per-visitor basket (collect-on-click, ephemeral per guest session — never
//! saved), the exit-action slot (CTA/board now; checkout in Phase 4), the
//! auto-loop disposition for unattended booths, and the ray-vs-exhibit click
//! selector.
//!
//! Showcase is deliberately NOT a `Scenario` (which models an in-world objective
//! whose end-card returns you to the lobby — the exact back-door we remove) and
//! NOT a `GameMode` variant. It is an app-lifecycle flag held on `GameState`.
//!
//! Spec: docs/superpowers/specs/2026-06-19-creator-gallery-showcase-design.md
//! (§4 skeleton + exit-action slot; §8 Phase 2; §10 Phase 2 paragraph; §11 seams).

use crate::exhibit::Exhibit;

/// What the basket leads to at exit — the "exit action is a slot" knob (§4/§11).
/// One variant for now; `Cta`/`Checkout` join it in later phases without
/// reshaping the call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitAction {
    /// Render a board / call-to-action screen from the collected basket (the
    /// cheap first exit — "here's what you saved; get the game / sign up").
    Board,
}

impl ExitAction {
    /// Parse an operator flag value; unknown / empty → the default `Board`.
    pub fn parse(s: &str) -> ExitAction {
        match s.trim().to_ascii_lowercase().as_str() {
            "board" | "cta" | "" => ExitAction::Board,
            _ => ExitAction::Board,
        }
    }
}

/// Resolved showcase configuration (from the server's flags). Inert when
/// `enabled` is false — the engine behaves exactly as today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowcaseConfig {
    pub enabled: bool,
    pub exit_action: ExitAction,
    /// `Some(secs)` ⇒ unattended booth: after the exit screen has shown for
    /// `secs`, loop back to a fresh session. `None` ⇒ stay on the exit screen
    /// (a dead-end the operator resets by hand).
    pub auto_loop_secs: Option<u32>,
}

impl Default for ShowcaseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            exit_action: ExitAction::Board,
            auto_loop_secs: None,
        }
    }
}

impl ShowcaseConfig {
    /// Build from raw flag strings (mirrors `server_main::resolve` outputs).
    /// `showcase`: `"1"`/`"true"` enables. `exit_action`: see `ExitAction::parse`.
    /// `auto_loop`: seconds as a positive integer; `"0"`/empty/garbage ⇒ no loop.
    pub fn from_flags(showcase: &str, exit_action: &str, auto_loop: &str) -> ShowcaseConfig {
        let enabled = matches!(
            showcase.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
        let auto_loop_secs = auto_loop.trim().parse::<u32>().ok().filter(|&n| n > 0);
        ShowcaseConfig {
            enabled,
            exit_action: ExitAction::parse(exit_action),
            auto_loop_secs,
        }
    }
}

/// Whether exit must be a one-step dead-end (out of the game AND the lobby, to
/// the terminal exit screen). True only in an enabled showcase — otherwise the
/// normal lobby/quit path stands.
pub fn should_dead_end_exit(cfg: &ShowcaseConfig) -> bool {
    cfg.enabled
}

/// What the kiosk does once the exit screen is up. The "thin wiring layer"
/// (native relaunch / web `location.reload`) that would consume this hasn't
/// been built yet — tested here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum ExitDisposition {
    /// Stay on the exit screen until the operator resets (attended demo).
    DeadEnd,
    /// Loop back to a fresh session after `after_secs` (unattended booth).
    AutoLoop { after_secs: u32 },
}

/// Decide the booth behaviour from config. Pure — the actual session reset
/// (native relaunch / web `location.reload`) is the thin wiring layer.
#[cfg_attr(not(test), allow(dead_code))]
pub fn exit_disposition(cfg: &ShowcaseConfig) -> ExitDisposition {
    match cfg.auto_loop_secs {
        Some(secs) => ExitDisposition::AutoLoop { after_secs: secs },
        None => ExitDisposition::DeadEnd,
    }
}

/// One collected piece in a visitor's basket. A flattened snapshot of the
/// exhibit's payload (§5/§11) so the basket is self-contained at exit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasketItem {
    pub image_ref: String,
    pub label: String,
    pub link: Option<String>,
    pub sku: Option<String>,
    pub price: Option<u64>,
}

impl BasketItem {
    /// Snapshot the collectable payload off a placed exhibit.
    pub fn from_exhibit(e: &Exhibit) -> BasketItem {
        BasketItem {
            image_ref: e.image_ref.clone(),
            label: e.label.clone(),
            link: e.link.clone(),
            sku: e.sku.clone(),
            price: e.price,
        }
    }
}

/// A per-visitor basket — EPHEMERAL (per guest session, never persisted). The
/// world is shared; the basket is personal (Spec §10).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Basket {
    pub items: Vec<BasketItem>,
}

impl Basket {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a piece, de-duplicating by `image_ref` (clicking the same exhibit
    /// twice collects it once). Returns `true` if it was newly added.
    pub fn add(&mut self, item: BasketItem) -> bool {
        if self.items.iter().any(|i| i.image_ref == item.image_ref) {
            return false;
        }
        self.items.push(item);
        true
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Sum of priced items (Phase 4 checkout reads this; harmless now). Items
    /// without a price contribute nothing.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn total_price(&self) -> u64 {
        self.items
            .iter()
            .filter_map(|i| i.price)
            .fold(0u64, |a, b| a.saturating_add(b))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

/// Ray-vs-exhibit-quad hit distance. The exhibit is a `width`×`height` quad
/// centred on its `(x, y, z)` anchor, facing along `yaw` (rotation about the
/// vertical Y axis; the quad spans the horizontal "right" axis and world-up).
/// Returns `Some(distance)` along `dir` if the ray strikes the quad in front of
/// `eye`, else `None`. Deterministic (uses the exhibit's STORED yaw, not a
/// per-frame camera billboard) so the click test is frame-independent. The
/// basis matches `exhibit::yaw_to_normal` (yaw 0 → +Z) so a click lands where
/// 1b draws the art.
fn quad_hit(eye: [f32; 3], dir: [f32; 3], e: &Exhibit) -> Option<f32> {
    // Quad basis from yaw: normal = [sin, 0, cos] (matches yaw_to_normal); right
    // is the in-plane horizontal axis; up is world-up.
    let (sy, cy) = e.yaw.sin_cos();
    let normal = [sy, 0.0, cy];
    let right = [cy, 0.0, -sy];
    let up = [0.0, 1.0, 0.0];
    let centre = [e.x as f32 + 0.5, e.y as f32 + 0.5, e.z as f32 + 0.5];

    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let denom = dot(dir, normal);
    if denom.abs() < 1e-6 {
        return None; // ray parallel to the quad plane
    }
    let to_centre = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
    let t = dot(to_centre, normal) / denom;
    if t <= 0.0 {
        return None; // behind the eye
    }
    let hit = [eye[0] + dir[0] * t, eye[1] + dir[1] * t, eye[2] + dir[2] * t];
    let local = [hit[0] - centre[0], hit[1] - centre[1], hit[2] - centre[2]];
    let u = dot(local, right);
    let v = dot(local, up);
    if u.abs() <= e.width / 2.0 && v.abs() <= e.height / 2.0 {
        Some(t)
    } else {
        None
    }
}

/// Pick the nearest exhibit the look ray strikes within `max_dist` (the click
/// reach). `dir` need not be normalised — it is here. Returns the exhibit's
/// index in `exhibits`, or `None` when nothing is in reach.
pub fn pick_exhibit(eye: [f32; 3], dir: [f32; 3], exhibits: &[Exhibit], max_dist: f32) -> Option<usize> {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-8 {
        return None;
    }
    let dn = [dir[0] / len, dir[1] / len, dir[2] / len];
    let mut best: Option<(usize, f32)> = None;
    for (i, e) in exhibits.iter().enumerate() {
        if !e.is_valid() {
            continue;
        }
        if let Some(t) = quad_hit(eye, dn, e)
            && t <= max_dist && best.map(|(_, bt)| t < bt).unwrap_or(true) {
                best = Some((i, t));
            }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exhibit::{Exhibit, Presentation};

    fn exhibit(image: &str, price: Option<u64>) -> Exhibit {
        Exhibit {
            x: 0,
            y: 64,
            z: 0,
            presentation: Presentation::Wall,
            image_ref: image.to_string(),
            width: 2.0,
            height: 1.0,
            yaw: 0.0,
            label: format!("Piece {image}"),
            link: Some("https://example.test".to_string()),
            sku: None,
            price,
        }
    }

    #[test]
    fn flag_parsing_enables_only_on_truthy() {
        assert!(ShowcaseConfig::from_flags("1", "", "").enabled);
        assert!(ShowcaseConfig::from_flags("true", "", "").enabled);
        assert!(ShowcaseConfig::from_flags("ON", "", "").enabled);
        assert!(!ShowcaseConfig::from_flags("0", "", "").enabled);
        assert!(!ShowcaseConfig::from_flags("", "", "").enabled);
        assert!(!ShowcaseConfig::from_flags("nope", "", "").enabled);
        assert!(!ShowcaseConfig::default().enabled);
    }

    #[test]
    fn exit_action_defaults_to_board() {
        assert_eq!(ShowcaseConfig::from_flags("1", "", "").exit_action, ExitAction::Board);
        assert_eq!(ShowcaseConfig::from_flags("1", "board", "").exit_action, ExitAction::Board);
        assert_eq!(ShowcaseConfig::from_flags("1", "garbage", "").exit_action, ExitAction::Board);
    }

    #[test]
    fn auto_loop_parses_positive_seconds_only() {
        assert_eq!(ShowcaseConfig::from_flags("1", "", "45").auto_loop_secs, Some(45));
        assert_eq!(ShowcaseConfig::from_flags("1", "", "0").auto_loop_secs, None);
        assert_eq!(ShowcaseConfig::from_flags("1", "", "").auto_loop_secs, None);
        assert_eq!(ShowcaseConfig::from_flags("1", "", "x").auto_loop_secs, None);
    }

    #[test]
    fn dead_end_only_when_enabled() {
        let off = ShowcaseConfig::default();
        assert!(!should_dead_end_exit(&off), "normal mode keeps the lobby");
        let on = ShowcaseConfig::from_flags("1", "", "");
        assert!(should_dead_end_exit(&on), "showcase intercepts exit");
    }

    #[test]
    fn disposition_follows_auto_loop_flag() {
        let dead = ShowcaseConfig::from_flags("1", "", "");
        assert_eq!(exit_disposition(&dead), ExitDisposition::DeadEnd);
        let loopy = ShowcaseConfig::from_flags("1", "", "30");
        assert_eq!(exit_disposition(&loopy), ExitDisposition::AutoLoop { after_secs: 30 });
    }

    #[test]
    fn basket_collects_and_dedupes_by_image_ref() {
        let mut b = Basket::new();
        assert!(b.is_empty());
        assert!(b.add(BasketItem::from_exhibit(&exhibit("a.png", Some(100)))));
        assert!(b.add(BasketItem::from_exhibit(&exhibit("b.png", Some(50)))));
        // Same image again → not added a second time.
        assert!(!b.add(BasketItem::from_exhibit(&exhibit("a.png", Some(100)))));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn basket_total_sums_priced_items_only() {
        let mut b = Basket::new();
        b.add(BasketItem::from_exhibit(&exhibit("a.png", Some(100))));
        b.add(BasketItem::from_exhibit(&exhibit("b.png", None))); // no price
        b.add(BasketItem::from_exhibit(&exhibit("c.png", Some(50))));
        assert_eq!(b.total_price(), 150);
    }

    #[test]
    fn basket_item_snapshots_payload() {
        let item = BasketItem::from_exhibit(&exhibit("art.png", Some(7)));
        assert_eq!(item.image_ref, "art.png");
        assert_eq!(item.label, "Piece art.png");
        assert_eq!(item.link.as_deref(), Some("https://example.test"));
        assert_eq!(item.price, Some(7));
    }

    #[test]
    fn basket_clear_empties() {
        let mut b = Basket::new();
        b.add(BasketItem::from_exhibit(&exhibit("a.png", None)));
        b.clear();
        assert!(b.is_empty());
    }

    // --- pick_exhibit (ray-vs-quad) ---

    fn wall_at(x: i32, y: i32, z: i32, yaw: f32) -> Exhibit {
        Exhibit {
            x,
            y,
            z,
            presentation: Presentation::Wall,
            image_ref: "p.png".to_string(),
            width: 2.0,
            height: 2.0,
            yaw,
            label: "P".to_string(),
            link: None,
            sku: None,
            price: None,
        }
    }

    #[test]
    fn pick_hits_exhibit_dead_ahead() {
        // Exhibit at z=5 facing -Z (normal toward the eye at the origin looking +Z).
        // yaw=PI → normal = [sin PI, 0, cos PI] = [0,0,-1] (faces -Z, toward eye).
        let e = wall_at(0, 0, 5, std::f32::consts::PI);
        let hit = pick_exhibit([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], &[e], 10.0);
        assert_eq!(hit, Some(0));
    }

    #[test]
    fn pick_misses_when_aimed_away() {
        let e = wall_at(0, 0, 5, std::f32::consts::PI);
        // Looking the opposite way (-Z) → no hit.
        let hit = pick_exhibit([0.5, 0.5, 0.0], [0.0, 0.0, -1.0], &[e], 10.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn pick_respects_max_dist() {
        let e = wall_at(0, 0, 5, std::f32::consts::PI);
        // In line but the reach is shorter than the ~4.5-block distance.
        let hit = pick_exhibit([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], &[e], 2.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn pick_chooses_nearest_of_several() {
        let near = wall_at(0, 0, 3, std::f32::consts::PI);
        let far = wall_at(0, 0, 8, std::f32::consts::PI);
        let hit = pick_exhibit([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], &[far, near], 20.0);
        assert_eq!(hit, Some(1), "the nearer exhibit (index 1) wins");
    }

    #[test]
    fn pick_skips_invalid_exhibits() {
        let mut bad = wall_at(0, 0, 3, std::f32::consts::PI);
        bad.image_ref = "  ".to_string(); // is_valid() == false
        let hit = pick_exhibit([0.5, 0.5, 0.0], [0.0, 0.0, 1.0], &[bad], 20.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn pick_misses_off_to_the_side() {
        // Aimed parallel-ish but offset far beyond the 1-block half-width.
        let e = wall_at(0, 0, 5, std::f32::consts::PI);
        let hit = pick_exhibit([20.0, 0.5, 0.0], [0.0, 0.0, 1.0], &[e], 50.0);
        assert_eq!(hit, None);
    }
}
