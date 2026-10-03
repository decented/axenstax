//! Community **Test Board** — the lobby column listing features that need
//! playtesting, each with a traffic-light status driven by tester verdicts.
//!
//! Pure core lives here: the importance ordering and the embedded registry.
//! The verdict transport (the browser lobby mailbox) was removed 2026-10-01 —
//! see `queue_verdict`. Spec:
//! `docs/foundations/2026-06-17-community-test-board.md`.
//!
use serde::{Deserialize, Serialize};

/// One feature on the board (authored registry data).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestItem {
    /// Stable, searchable token, e.g. `"TB-07-worldedit"`. It also appears in the
    /// linked test sheet, so searching the wiki/docs for the ref finds the sheet.
    #[serde(rename = "ref")]
    pub ref_id: String,
    pub title: String,
    /// One-line "what to test".
    #[serde(default)]
    pub summary: String,
    /// Link to the test sheet (docs path + anchor, or wiki URL).
    #[serde(default)]
    pub sheet: String,
    /// Curated importance; higher sorts first within the same gating tier.
    #[serde(default)]
    pub priority: i32,
    /// Downstream work this feature gates. More entries → higher rank, because
    /// pipelined work depends on this being tested first.
    #[serde(default)]
    pub blocks: Vec<String>,
}

/// Importance ordering: features gating the most downstream work first, then by
/// curated `priority`, then `ref` for stability. Mutates in place.
pub fn rank_items(items: &mut [TestItem]) {
    items.sort_by(|a, b| {
        b.blocks
            .len()
            .cmp(&a.blocks.len()) // most downstream work gated first
            .then(b.priority.cmp(&a.priority)) // then curated priority
            .then(a.ref_id.cmp(&b.ref_id)) // stable tiebreak
    });
}

/// A tester's verdict on a mission. There is NO transport for it: the only one
/// was the browser build's lobby mailbox, removed 2026-10-01 (the browser build
/// has no feedback channel at all), and the native mailbox is for `/bug` and
/// `/idea`, not verdicts. The mission dialogue still lets a tester step through
/// the list; the verdict goes nowhere and this returns the line to show the
/// player, which says so.
pub fn queue_verdict(_ref_id: &str, _good: bool, _note: &str) -> String {
    "Test verdicts aren't collected in this build.".to_string()
}

/// Embedded baseline registry (compile-time) so the column renders offline and
/// on native `--shot-lobby`. A live-served registry may overlay this later.
pub const REGISTRY_JSON: &str = include_str!("../assets/test_board.json");

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct TestBoardRegistry {
    pub items: Vec<TestItem>,
}

/// Parse + rank the embedded registry. Returns ranked items (empty on parse
/// failure — the column simply shows nothing rather than crashing the lobby).
pub fn load_registry() -> Vec<TestItem> {
    let mut reg: TestBoardRegistry = serde_json::from_str(REGISTRY_JSON).unwrap_or_default();
    rank_items(&mut reg.items);
    reg.items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(ref_id: &str, priority: i32, blocks: &[&str]) -> TestItem {
        TestItem {
            ref_id: ref_id.to_string(),
            title: String::new(),
            summary: String::new(),
            sheet: String::new(),
            priority,
            blocks: blocks.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn rank_puts_most_gating_first_then_priority() {
        let mut items = vec![
            item("leaf", 99, &[]),         // high priority, gates nothing
            item("gate1", 10, &["a"]),     // gates 1
            item("gate2", 10, &["a", "b"]), // gates 2
        ];
        rank_items(&mut items);
        assert_eq!(items[0].ref_id, "gate2"); // most downstream work first
        assert_eq!(items[1].ref_id, "gate1");
        assert_eq!(items[2].ref_id, "leaf"); // leaf last despite high priority
    }

    #[test]
    fn rank_breaks_ties_by_priority_then_ref() {
        let mut items = vec![
            item("b-mid", 50, &["x"]),
            item("a-hi", 90, &["x"]),
            item("c-hi", 90, &["x"]),
        ];
        rank_items(&mut items);
        assert_eq!(items[0].ref_id, "a-hi"); // same gating, higher priority, ref tiebreak
        assert_eq!(items[1].ref_id, "c-hi");
        assert_eq!(items[2].ref_id, "b-mid");
    }

    #[test]
    fn embedded_registry_parses_ranked_and_complete() {
        let items = load_registry();
        assert!(!items.is_empty(), "embedded test_board.json should list features");
        for it in &items {
            assert!(!it.ref_id.is_empty());
            assert!(!it.sheet.is_empty(), "{} missing sheet link", it.ref_id);
            assert!(
                it.sheet.contains(&it.ref_id.to_lowercase()),
                "{} sheet anchor should embed its ref so the ref is searchable",
                it.ref_id
            );
        }
        // ranked: first gates at least as much as last.
        assert!(items.first().unwrap().blocks.len() >= items.last().unwrap().blocks.len());
    }

    #[test]
    fn current_build_missions_present_for_satoshi_test_lab() {
        // Satoshi's Test Lab serves these exact ref_ids as in-world missions.
        let items = load_registry();
        for r in ["TB-51-worldgen-floor", "TB-52-satoshi-welcome", "TB-53-build-along"] {
            assert!(items.iter().any(|i| i.ref_id == r), "{r} must be a live mission");
        }
        // The verdict hook is callable on every target and never panics.
        assert!(!queue_verdict("TB-51-worldgen-floor", true, "ok").is_empty());
        assert!(!queue_verdict("TB-53-build-along", false, "layer glow stuck").is_empty());
    }
}
