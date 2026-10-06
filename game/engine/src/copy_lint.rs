//! The ONE source of truth for the regulatory money-word lint.
//!
//! Why this exists: CLAUDE.md "Regulatory Red Lines" #4 — AxeNStax never frames
//! play as money or earning (UK FCA financial-promotion perimeter; Proof of Play
//! is an educational proof-of-work primitive, never "earn Bitcoin"). The ban was
//! enforced by a dozen hand-copied word lists (trials, packaging, Satoshi,
//! quests, loading cards, gossip, lobby copy…) that had already drifted apart:
//! only some carried `crypto`, only some `lightning`/`prize`/`cash`. Every lint
//! now imports [`BANNED_MONEY_WORDS`] from here, so a word added once is banned
//! everywhere, and no lint can quietly keep a shorter list.
//!
//! Test-only (`#[cfg(test)]` in `lib.rs`) — nothing here ships.
//!
//! ## The three tiers
//!
//! [`BANNED_MONEY_WORDS`] is the union. It is partitioned (a test pins the
//! partition, so the tiers cannot drift from the union) into:
//!
//! * [`EARNING_FRAME`] — words that frame play as earning (`earn*`, `payout*`,
//!   `prize*`, `wages`…). Banned on EVERY linted surface.
//! * [`HARD_MONEY`] — real-world money words (`money`, `wallet`, `btc`,
//!   `lightning`, `crypto`, `cash`). Banned everywhere except an explicit,
//!   reasoned allow (a negation such as "no money involved").
//! * [`CURRENCY_VOCAB`] — the in-game sat SCORE and trade verbs (`sats`,
//!   `bitcoin`, `buy`, `sell`). Banned on neutral surfaces (trials, packaging,
//!   README, lobby copy, loading cards…). Permitted on the sats-gated economy
//!   panels and in the player guide, which exist to explain that score
//!   honestly (a flat ban would forbid writing "no sats show up, no real
//!   money changes hands").
//!
//! The player guide is therefore linted at [`EARNING_FRAME`] only. That is a
//! judgement call, recorded here so it can be tightened.
//!
//! ## Exceptions
//!
//! Every exception is one row in [`ALLOWS`], with a reason. A companion check
//! ([`scan`]) fails on a STALE row (one that no longer excuses anything), so
//! the list cannot rot into a blanket pass.
//!
//! ## Surfaces covered here
//!
//! * `README.md` — full union.
//! * `docs/player-guide/**/*.md` — [`EARNING_FRAME`].
//! * The string literals of every `src/*_ui.rs` and `src/menu.rs` (the egui
//!   text surfaces: HUD, panels, dialogs, menus) — full union, except the
//!   sats-gated economy panels ([`ECONOMY_UI_FILES`], earning + hard money).
//!
//! Not covered: strings in `game_loop.rs` (toasts) and other non-UI modules —
//! they carry log lines and internal identifiers a word ban cannot tell apart
//! from copy; the in-game copy tables that matter (trials, Satoshi, quests,
//! gossip, loading cards, lobby, controls) have their own lints, which import
//! this list.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Words that frame play as earning. Banned on every linted surface.
pub(crate) const EARNING_FRAME: &[&str] = &[
    "earn", "earns", "earned", "earning", "earnings", "payout", "payouts", "cashback", "wages",
    "salary", "prize", "prizes",
];

/// Real-world money words.
pub(crate) const HARD_MONEY: &[&str] =
    &["btc", "crypto", "lightning", "wallet", "wallets", "money", "cash"];

/// The in-game sat score and trade verbs — legitimate only where the game
/// openly discusses its economy.
pub(crate) const CURRENCY_VOCAB: &[&str] = &["sat", "sats", "bitcoin", "bitcoins", "buy", "sell"];

/// THE list. The union of every word any money-word lint in the engine ever
/// banned. Add a word here and it is banned on every surface that imports it;
/// never remove one without an owner decision.
pub(crate) const BANNED_MONEY_WORDS: &[&str] = &[
    // EARNING_FRAME
    "earn", "earns", "earned", "earning", "earnings", "payout", "payouts", "cashback", "wages",
    "salary", "prize", "prizes",
    // HARD_MONEY
    "btc", "crypto", "lightning", "wallet", "wallets", "money", "cash",
    // CURRENCY_VOCAB
    "sat", "sats", "bitcoin", "bitcoins", "buy", "sell",
];

/// Flatten several tiers (or any word slices) into one word list.
pub(crate) fn words_of(tiers: &[&[&'static str]]) -> Vec<&'static str> {
    tiers.iter().flat_map(|t| t.iter().copied()).collect()
}

/// Lowercased whole-word tokens: runs of letters, plus an apostrophe BETWEEN
/// letters (`don't`). A possessive also yields its stem (`bitcoin's` →
/// `bitcoin`). Digits, `_`, `/`, quotes and markdown punctuation all split, so
/// `cash_stack`, `5sats`, `'earn'` and `**earned**` tokenise as expected, and
/// `learn`, `Satoshi`, `cryptography` never match `earn`, `sat`, `crypto`.
pub(crate) fn tokenize_words(text: &str) -> HashSet<String> {
    fn flush(cur: &mut String, out: &mut HashSet<String>) {
        if cur.is_empty() {
            return;
        }
        if let Some(stem) = cur.strip_suffix("'s") {
            out.insert(stem.to_string());
        }
        out.insert(std::mem::take(cur));
    }
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut out = HashSet::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphabetic() {
            cur.push(c);
        } else if (c == '\'' || c == '\u{2019}')
            && !cur.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_alphabetic())
        {
            cur.push('\'');
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Which of `words` occur in `text` as whole words (in `words` order).
pub(crate) fn banned_in<'a>(text: &str, words: &[&'a str]) -> Vec<&'a str> {
    let toks = tokenize_words(text);
    words.iter().copied().filter(|w| toks.contains(*w)).collect()
}

/// One line / literal of linted text and where it came from.
pub(crate) struct Item {
    pub(crate) at: String,
    pub(crate) text: String,
}

/// A reasoned exception. It excuses ONE banned `word` on `surface`, only in
/// items whose location contains `at` and whose text contains `contains`
/// (case-insensitive). [`scan`] fails on a row that excuses nothing.
pub(crate) struct Allow {
    pub(crate) surface: &'static str,
    pub(crate) at: &'static str,
    pub(crate) word: &'static str,
    pub(crate) contains: &'static str,
    pub(crate) reason: &'static str,
}

/// Every exception to the money-word lint, in one reviewable place.
pub(crate) const ALLOWS: &[Allow] = &[
    // ── README: negations of money, not framing play as money ──────────────
    Allow {
        surface: "README.md",
        at: "",
        word: "money",
        contains: "no money involved",
        reason: "negation: 'fully fun, with no money involved at all' — says the opposite of earning",
    },
    Allow {
        surface: "README.md",
        at: "",
        word: "money",
        contains: "requires money",
        reason: "negation: 'Nothing about the game requires money' — says the opposite of earning",
    },
    // ── Player guide: negated 'payout' ─────────────────────────────────────
    Allow {
        surface: "docs/player-guide",
        at: "bitcoin-and-sats.md",
        word: "payout",
        contains: "not a payout",
        reason: "negation: the heading 'Proof of Play — real maths, not a payout' is the \
                 educational-proof-of-work framing the red lines require",
    },
    Allow {
        surface: "docs/player-guide",
        at: "trade-value.md",
        word: "payout",
        contains: "not a payout",
        reason: "negation: the Genesis celebration is 'a one-time celebration, not a payout'",
    },
    // ── Moonshot Phase A founding-myth lore (in-fiction, not real money) ───
    // docs/superpowers/specs/2026-06-22-moonshot-phase-a-theme-capture-build-spec.md:
    // diamonds are the world's "old money" in the fiction (Diamond Age ->
    // Genesis / Satori). Neither line mentions Bitcoin, sats or earning, and the
    // spec mandates the gossip line verbatim; villager::tests pins both strings.
    Allow {
        surface: "loading cards",
        at: "The Age of Diamonds",
        word: "money",
        contains: "the old money",
        reason: "Moonshot Phase A lore card: diamonds are the fiction's 'old money'; no real-money, \
                 Bitcoin or earning claim",
    },
    Allow {
        surface: "villager gossip",
        at: "Diamonds buy less bread",
        word: "buy",
        contains: "Diamonds buy less bread",
        reason: "Moonshot Phase A gossip line mandated verbatim by the spec: in-fiction diamond \
                 debasement, hints at drift without changing any mechanic",
    },
    // ── HUD: the sats-gated Reserve gauge + Fund dialog ────────────────────
    // These three functions are drawn ONLY when `economy::sats_ui_visible`
    // (a Bitcoin-enabled server AND the guardian flag) — see the call sites in
    // game_loop.rs ("Audit 2026-09-27", "Review W4 S4"). Never on web, never by
    // default. Their copy may name the in-game sat score; the lint still bans
    // every other money word in them.
    Allow {
        surface: "egui UI literals",
        at: "fn mining_rate_text",
        word: "sats",
        contains: "sats",
        reason: "Reserve gauge rate line — sats-gated (economy::sats_ui_visible)",
    },
    Allow {
        surface: "egui UI literals",
        at: "fn draw_reserve_gauge",
        word: "sats",
        contains: "sats",
        reason: "Reserve gauge sat counter — sats-gated (economy::sats_ui_visible)",
    },
    Allow {
        surface: "egui UI literals",
        at: "fn draw_fund_dialog",
        word: "sats",
        contains: "sats",
        reason: "Fund-the-Reserve dialog amounts and split — sats-gated (economy::sats_ui_visible)",
    },
];

/// `Err` listing every un-excused hit of `words` in `items`, and every STALE
/// allow row for `surface`; `Ok` when the surface is clean.
pub(crate) fn scan(surface: &str, items: &[Item], words: &[&str]) -> Result<(), String> {
    let allows: Vec<&Allow> = ALLOWS.iter().filter(|a| a.surface == surface).collect();
    let mut used = vec![false; allows.len()];
    let mut problems: Vec<String> = Vec::new();
    for it in items {
        let lower = it.text.to_lowercase();
        for word in banned_in(&it.text, words) {
            let mut excused = false;
            for (k, a) in allows.iter().enumerate() {
                if a.word == word
                    && it.at.contains(a.at)
                    && lower.contains(&a.contains.to_lowercase())
                {
                    used[k] = true;
                    excused = true;
                }
            }
            if !excused {
                let shown: String = it.text.chars().take(120).collect();
                problems.push(format!("  {} — banned word {word:?}: {shown}", it.at));
            }
        }
    }
    for (k, a) in allows.iter().enumerate() {
        if !used[k] {
            problems.push(format!(
                "  STALE ALLOW (surface {surface:?}, at {:?}, word {:?}, contains {:?}) excuses \
                 nothing — delete the row from copy_lint::ALLOWS",
                a.at, a.word, a.contains
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "money-word lint failed on {surface:?} (CLAUDE.md \"Regulatory Red Lines\" #4 — play is \
             never framed as money or earning; Proof of Play is educational proof-of-work). Reword \
             the copy sovereignty-first; if a hit is a genuine negation, add a reasoned row to \
             copy_lint::ALLOWS.\n{}",
            problems.join("\n")
        ))
    }
}

// ─── Repo access ─────────────────────────────────────────────────────────

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Read a repo-relative file; a moved or renamed file must fail LOUDLY, never
/// silently pass an empty scan.
pub(crate) fn read_repo_file(rel: &str) -> String {
    let path = manifest_dir().join("../..").join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "copy_lint: cannot read {} ({e}). If the surface moved, update the lint's path — \
             do not delete the lint.",
            path.display()
        )
    })
}

/// Every `*.md` under a repo-relative directory, recursively, sorted.
fn markdown_files_under(rel_dir: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let rd = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("copy_lint: cannot list {} ({e})", dir.display()));
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&manifest_dir().join("../..").join(rel_dir), &mut out);
    out.sort();
    out
}

/// One [`Item`] per non-empty line of a markdown text.
fn markdown_items(label: &str, raw: &str) -> Vec<Item> {
    raw.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, l)| Item { at: format!("{label}:{}", i + 1), text: l.to_string() })
        .collect()
}

// ─── Rust string-literal extraction (the egui text surfaces) ─────────────

/// Cut `src` at its test module (`#[cfg(test)]` directly followed by `mod`),
/// so test fixtures that name banned words on purpose are never linted.
fn without_test_module(src: &str) -> &str {
    let mut offset = 0;
    let lines: Vec<&str> = src.split_inclusive('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "#[cfg(test)]" {
            let next = lines[i + 1..].iter().map(|l| l.trim()).find(|l| !l.is_empty());
            if next.is_some_and(|n| {
                n.starts_with("mod ") || n.starts_with("pub mod ") || n.starts_with("pub(crate) mod ")
            }) {
                return &src[..offset];
            }
        }
        offset += line.len();
    }
    src
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every string literal in Rust source `src` (non-test code only), with its
/// line and enclosing `fn`. Comments are skipped; `\`-newline continuations
/// are joined the way rustc joins them; raw strings are handled; char
/// literals and lifetimes are told apart. Good enough to lint copy, not a
/// parser.
pub(crate) fn rust_string_literals(file: &str, src: &str) -> Vec<Item> {
    let src = without_test_module(src);
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1usize;
    let mut func = String::from("-");
    while i < c.len() {
        match c[i] {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if c.get(i + 1) == Some(&'/') => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
            }
            '/' if c.get(i + 1) == Some(&'*') => {
                let mut depth = 1;
                i += 2;
                while i < c.len() && depth > 0 {
                    if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        if c[i] == '\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            'f' if c.get(i + 1) == Some(&'n')
                && c.get(i + 2) == Some(&' ')
                && (i == 0 || !is_ident(c[i - 1])) =>
            {
                let mut j = i + 3;
                let mut name = String::new();
                while j < c.len() && is_ident(c[j]) {
                    name.push(c[j]);
                    j += 1;
                }
                if !name.is_empty() {
                    func = name;
                }
                i = j;
            }
            'r' if (i == 0 || !is_ident(c[i - 1]) || (c[i - 1] == 'b' && (i < 2 || !is_ident(c[i - 2]))))
                && {
                    let mut j = i + 1;
                    while c.get(j) == Some(&'#') {
                        j += 1;
                    }
                    c.get(j) == Some(&'"')
                } =>
            {
                let mut hashes = 0;
                let mut j = i + 1;
                while c.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                j += 1; // opening quote
                let start_line = line;
                let mut text = String::new();
                while j < c.len() {
                    if c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#')) {
                        j += 1 + hashes;
                        break;
                    }
                    if c[j] == '\n' {
                        line += 1;
                    }
                    text.push(c[j]);
                    j += 1;
                }
                out.push(Item { at: format!("{file}:{start_line} (fn {func})"), text });
                i = j;
            }
            '"' => {
                let start_line = line;
                let mut text = String::new();
                i += 1;
                while i < c.len() && c[i] != '"' {
                    if c[i] == '\\' {
                        match c.get(i + 1) {
                            Some('\n') => {
                                // line continuation: drop the newline and the
                                // indentation that follows it
                                line += 1;
                                i += 2;
                                while i < c.len() && c[i].is_whitespace() {
                                    if c[i] == '\n' {
                                        line += 1;
                                    }
                                    i += 1;
                                }
                            }
                            Some('u') => {
                                i += 2;
                                while i < c.len() && c[i] != '}' {
                                    i += 1;
                                }
                                i += 1;
                                text.push('?');
                            }
                            Some('x') => {
                                i += 4;
                                text.push('?');
                            }
                            Some('n') | Some('t') | Some('r') | Some('0') => {
                                text.push(' ');
                                i += 2;
                            }
                            Some(&other) => {
                                text.push(other);
                                i += 2;
                            }
                            None => i += 1,
                        }
                    } else {
                        if c[i] == '\n' {
                            line += 1;
                        }
                        text.push(c[i]);
                        i += 1;
                    }
                }
                i += 1; // closing quote
                out.push(Item { at: format!("{file}:{start_line} (fn {func})"), text });
            }
            '\'' => {
                if c.get(i + 1) == Some(&'\\') {
                    // escaped char literal: '\n', '\'', '\u{..}'
                    i += 3;
                    while i < c.len() && c[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if c.get(i + 2) == Some(&'\'') {
                    i += 3; // 'x' — including '"'
                } else {
                    i += 1; // a lifetime
                }
            }
            _ => i += 1,
        }
    }
    out
}

// ─── Surfaces linted here ────────────────────────────────────────────────

/// Sats-gated economy panels: the shop / trade / tip UIs, drawn only when
/// `economy::sats_ui_visible`. They name the in-game sat score and the trade
/// verbs by design, so they are linted at earning + hard money. Every OTHER
/// `*_ui.rs` (and `menu.rs`) is linted at the full union.
const ECONOMY_UI_FILES: &[&str] = &[
    "auction_ui.rs",
    "bazaar_ui.rs",
    "bounty_ui.rs",
    "commission_ui.rs",
    "market_hub_ui.rs",
    "plan_ui.rs",
    "repair_ui.rs",
    "tip_jar_ui.rs",
    "vendor_ui.rs",
    "village_bell_ui.rs",
];

/// Every egui text-surface source file: `src/*_ui.rs` and `src/menu.rs`.
fn ui_source_files() -> Vec<(String, String)> {
    let dir = manifest_dir().join("src");
    let rd = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("copy_lint: cannot list {} ({e})", dir.display()));
    let mut files: Vec<(String, String)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.ends_with("_ui.rs") || name == "menu.rs").then(|| {
                let text = std::fs::read_to_string(e.path())
                    .unwrap_or_else(|err| panic!("copy_lint: cannot read {name} ({err})"));
                (name, text)
            })
        })
        .collect();
    files.sort();
    files
}

#[test]
fn readme_has_no_money_or_earning_words() {
    let items = markdown_items("README.md", &read_repo_file("README.md"));
    assert!(items.len() > 20, "README.md scan covered almost nothing — wrong path?");
    scan("README.md", &items, BANNED_MONEY_WORDS).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn player_guide_never_frames_play_as_earning() {
    let files = markdown_files_under("docs/player-guide");
    assert!(files.len() > 30, "player-guide scan found {} files — wrong path?", files.len());
    let mut items = Vec::new();
    for f in &files {
        let raw = std::fs::read_to_string(f)
            .unwrap_or_else(|e| panic!("copy_lint: cannot read {} ({e})", f.display()));
        let label = f.strip_prefix(manifest_dir().join("../..")).unwrap_or(f).display().to_string();
        items.extend(markdown_items(&label, &raw));
    }
    scan("docs/player-guide", &items, EARNING_FRAME).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn egui_ui_literals_have_no_money_or_earning_words() {
    let files = ui_source_files();
    assert!(files.len() > 25, "UI source scan found {} files — wrong path?", files.len());
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"hud_ui.rs") && names.contains(&"menu.rs"));
    for econ in ECONOMY_UI_FILES {
        assert!(names.contains(econ), "ECONOMY_UI_FILES names {econ}, which no longer exists");
    }
    let mut strict = Vec::new();
    let mut economy = Vec::new();
    for (name, text) in &files {
        let lits = rust_string_literals(name, text);
        if ECONOMY_UI_FILES.contains(&name.as_str()) {
            economy.extend(lits);
        } else {
            strict.extend(lits);
        }
    }
    assert!(strict.len() > 500, "only {} UI literals scanned — extractor broken?", strict.len());
    // The UI literals share one surface name so its ALLOWS rows are checked
    // for staleness across both passes.
    let strict_result = scan("egui UI literals", &strict, BANNED_MONEY_WORDS);
    let economy_result =
        scan("egui economy panels", &economy, &words_of(&[EARNING_FRAME, HARD_MONEY]));
    let errs: Vec<String> = [strict_result, economy_result].into_iter().filter_map(Result::err).collect();
    assert!(errs.is_empty(), "{}", errs.join("\n\n"));
}

// ─── The lint's own tests: it must bite ─────────────────────────────────

fn item(at: &str, text: &str) -> Item {
    Item { at: at.to_string(), text: text.to_string() }
}

#[test]
fn the_tiers_partition_the_union_exactly() {
    let mut tiers: Vec<&str> = Vec::new();
    for t in [EARNING_FRAME, HARD_MONEY, CURRENCY_VOCAB] {
        tiers.extend_from_slice(t);
    }
    let tier_set: HashSet<&str> = tiers.iter().copied().collect();
    assert_eq!(tier_set.len(), tiers.len(), "a word sits in two tiers");
    let union: HashSet<&str> = BANNED_MONEY_WORDS.iter().copied().collect();
    assert_eq!(union.len(), BANNED_MONEY_WORDS.len(), "BANNED_MONEY_WORDS repeats a word");
    assert_eq!(tier_set, union, "BANNED_MONEY_WORDS and the tiers must hold exactly the same words");
    // Never drop a word: the union of every list the engine had before this
    // module existed (trials/packaging/scenario, Satoshi, quest, loading,
    // gossip, lobby, controls, contacts).
    for legacy in [
        "sats", "bitcoin", "btc", "earn", "earning", "earned", "payout", "payouts", "wallet",
        "money", "cash", "cashback", "prize", "prizes", "sell", "buy", "lightning", "wages",
        "salary", "sat", "bitcoins", "earns", "earnings", "wallets", "crypto",
    ] {
        assert!(union.contains(legacy), "the shared list lost {legacy:?}");
    }
    assert_eq!(BANNED_MONEY_WORDS.len(), 25);
}

#[test]
fn tokenizer_matches_whole_words_only() {
    let t = tokenize_words("Learn about Satoshi's cryptography; 'earn' + **earned** 5sats cash_stack");
    for yes in ["earn", "earned", "sats", "cash", "stack", "satoshi", "satoshi's"] {
        assert!(t.contains(yes), "expected token {yes:?}");
    }
    for no in ["sat", "crypto", "satoshi'", "earn'"] {
        assert!(!t.contains(no), "unexpected token {no:?}");
    }
    assert!(banned_in("Bitcoin's price", BANNED_MONEY_WORDS).contains(&"bitcoin"));
    assert!(banned_in("She sat down to learn.", &["earn"]).is_empty());
}

#[test]
fn scan_bites_on_a_banned_word_and_honours_a_reasoned_allow() {
    // The README allow rows are real: the two negation lines pass, but a
    // different 'money' line on the same surface is still a violation.
    let bad = [
        item("README.md:1", "fully fun, with no money involved at all"),
        item("README.md:2", "Nothing about the game requires money"),
        item("README.md:3", "Mine to earn money"),
    ];
    let err = scan("README.md", &bad, BANNED_MONEY_WORDS).expect_err("must reject 'earn money'");
    assert!(err.contains("README.md:3") && err.contains("\"earn\"") && err.contains("\"money\""));
    assert!(!err.contains("README.md:1"), "an allowed line must not be reported");
}

#[test]
fn scan_flags_a_stale_allow() {
    let clean = [item("README.md:1", "nothing to see here")];
    let err = scan("README.md", &clean, BANNED_MONEY_WORDS).expect_err("unused allows are stale");
    assert!(err.contains("STALE ALLOW"));
}

#[test]
fn every_allow_row_has_a_reason_and_a_real_word() {
    for a in ALLOWS {
        assert!(a.reason.trim().len() > 20, "allow row {:?}/{:?} needs a real reason", a.surface, a.contains);
        assert!(BANNED_MONEY_WORDS.contains(&a.word), "allow row names {:?}, not a banned word", a.word);
        assert!(!a.contains.is_empty(), "allow row for {:?} must pin the text it excuses", a.word);
    }
}

#[test]
fn literal_extractor_reads_what_the_player_reads() {
    let src = r##"
// a comment about earn and "bitcoin"
/* block "sats" */
fn shown() {
    let a = "plain 'earn' text";
    let b = "joined \
             across lines";
    let q = '"'; // a char literal must not open a string
    let r = r#"raw "quoted" money"#;
    let url = "https://example.test/wallet";
    fn inner<'a>(x: &'a str) -> &'a str { x }
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() { let _ = "test fixture mentions sats"; }
}
"##;
    let lits = rust_string_literals("x.rs", src);
    let texts: Vec<&str> = lits.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        ["plain 'earn' text", "joined across lines", "raw \"quoted\" money", "https://example.test/wallet"]
    );
    assert!(lits.iter().all(|l| l.at.contains("fn shown")), "{:?}", lits.iter().map(|l| &l.at).collect::<Vec<_>>());
    assert_eq!(lits[0].at, "x.rs:5 (fn shown)");
}

#[test]
fn ui_scan_bites_on_a_banned_literal() {
    let src = "fn draw() { ui.label(\"You earn yours by playing\"); }\n";
    let items = rust_string_literals("fake_ui.rs", src);
    let err = scan("fake surface", &items, BANNED_MONEY_WORDS).expect_err("'earn' in a label");
    assert!(err.contains("fake_ui.rs:1") && err.contains("\"earn\""));
}
