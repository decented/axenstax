//! Compliance banlist over PACKAGING metadata — the copy that ships INSIDE
//! installers (AppImage/.deb/.msi/.dmg metadata, shown in software centres).
//!
//! Why this exists: `tools/packaging/packager.toml`'s description said
//! "a voxel sandbox where mining earns real Bitcoin" from first packaging
//! until 2026-07-24 — a straight breach of the no-money-words rule (UK FCA
//! promotion perimeter; CLAUDE.md "Regulatory Red Lines" #4) that the in-game
//! guards (`trials_lint`, `scenario.rs`) never covered because they only scan
//! game text surfaces. This lint closes that gap: any banned word in a
//! packager.toml string value fails `cargo test` (and therefore `check.sh`).
//!
//! Scope is deliberately the PACKAGING metadata only. Site/marketing copy has
//! a nuanced policy (Bitcoin may appear as a byproduct mention, never the
//! headline — see `project_sovereignty_first_marketing`) that a word ban
//! cannot encode; it is audited editorially, not linted.

#![cfg(test)]

use std::collections::HashSet;

/// Same list as `trials_lint::BANNED_MONEY_WORDS` / `scenario.rs` (source of
/// truth). Kept in sync by hand, matching the existing duplication note there.
const BANNED_MONEY_WORDS: &[&str] = &[
    "sats", "bitcoin", "btc", "earn", "earning", "earned", "payout", "payouts",
    "wallet", "money", "cash", "cashback", "prize", "prizes", "sell", "buy",
    "lightning", "wages", "salary",
];

/// Quoted string VALUES from a TOML-ish file, with full-line comments
/// stripped first (so dev commentary about the rule itself can't trip it).
/// This lints exactly what ships — string values — not keys or comments.
fn quoted_values_sans_comments(raw: &str) -> String {
    let mut out = String::new();
    for line in raw.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else { break };
            out.push_str(&after[..close]);
            out.push(' ');
            rest = &after[close + 1..];
        }
    }
    out
}

fn tokenize_words(corpus: &str) -> HashSet<String> {
    corpus
        .to_lowercase()
        .split(|c: char| !(c.is_ascii_alphabetic() || c == '\''))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn packager_metadata_has_no_money_or_earning_words() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/packaging/packager.toml");
    // A moved/renamed file must fail LOUDLY, not silently pass an empty scan.
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "packaging_copy_lint: cannot read {} ({e}). If the packaging config \
             moved, update this lint's path — do not delete the lint.",
            path.display()
        )
    });
    let words = tokenize_words(&quoted_values_sans_comments(&raw));
    for banned in BANNED_MONEY_WORDS {
        assert!(
            !words.contains(*banned),
            "tools/packaging/packager.toml contains banned money/earning word {banned:?} \
             in a string value. This copy ships inside every installer (software-centre \
             metadata) — it must never frame play as money/earning (compliance red line, \
             see CLAUDE.md \"Regulatory Red Lines\"). Reword it sovereignty-first."
        );
    }
    // And the guard itself must be scanning something real — the description
    // key is the surface that regressed historically.
    assert!(
        raw.contains("description ="),
        "packager.toml no longer has a description key — update this lint to cover \
         whatever replaced the shipping copy surface"
    );
}

#[test]
fn quoted_value_extraction_skips_comments_and_keys() {
    let sample = r#"
# a comment daring to mention earn and "bitcoin" in quotes
name = "clean"
description = "totally fine copy"   # trailing note
"#;
    let words = tokenize_words(&quoted_values_sans_comments(sample));
    assert!(words.contains("clean") && words.contains("copy"));
    assert!(
        !words.contains("bitcoin") && !words.contains("earn"),
        "comment content must never reach the scanned corpus"
    );
    assert!(!words.contains("description"), "keys are not scanned, only values");
}
