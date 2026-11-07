//! The Trials gallery's data side: which trial sits in which row, how a
//! `card_art` name becomes a picture, and the cards built from them. Pure (no
//! drawing), so it is unit-tested; `trials_gallery_ui` draws it.
//!
//! Design: `docs/vision/trials-two-acts.md` Part C, §C.2 (internal repo). Nothing
//! here changes how a trial plays: a card carries the very scenario the old
//! Trials column launched, and trial order, objectives and persistence are
//! untouched.

use crate::trials::{TrialBests, TrialRef, TrialType, TRIAL_ORDER};

// ---------------------------------------------------------------------------
// Pure: rows
// ---------------------------------------------------------------------------

/// The four shelves, in the order they are shown.
pub const SHELF_ORDER: [TrialType; 4] =
    [TrialType::Make, TrialType::Tend, TrialType::Tech, TrialType::Brave];

/// Title of the row that holds the races and the timed scavenge.
pub const CLOCK_TITLE: &str = "Against the clock";

/// The one challenge (typed `Make` in `TRIAL_ORDER`) that belongs in the clock
/// row: it is a three-minute dash, not a build.
const CLOCK_CHALLENGE: &str = "scavenger";

/// Which gallery row a trial lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Shelf(TrialType),
    Clock,
}

impl RowKind {
    /// The row header's words.
    pub fn title(self) -> &'static str {
        match self {
            RowKind::Shelf(t) => t.label(),
            RowKind::Clock => CLOCK_TITLE,
        }
    }

    /// The row's colour (the clock row takes the Race colour).
    pub fn color_rgb(self) -> [u8; 3] {
        match self {
            RowKind::Shelf(t) => t.color_rgb(),
            RowKind::Clock => TrialType::Race.color_rgb(),
        }
    }

    /// The row's glyph (the clock row takes the Race flag).
    pub fn glyph(self) -> &'static str {
        match self {
            RowKind::Shelf(t) => t.icon(),
            RowKind::Clock => TrialType::Race.icon(),
        }
    }
}

/// Which row a trial belongs in: anything typed `Race` (both the ghost races
/// and the Roadrunner challenge) plus Scavenger go against the clock; the rest
/// sit on their own shelf.
pub fn row_kind_of(ttype: TrialType, tref: TrialRef) -> RowKind {
    let scavenge = matches!(tref, TrialRef::Challenge(t) if t == CLOCK_CHALLENGE);
    if ttype == TrialType::Race || scavenge {
        RowKind::Clock
    } else {
        RowKind::Shelf(ttype)
    }
}

/// One row's worth of trials, in `TRIAL_ORDER` order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowSpec {
    pub kind: RowKind,
    pub entries: Vec<(TrialType, TrialRef)>,
}

/// The gallery's rows: Make, Tend, Tech, Brave, then the clock row. A row with
/// nothing in it is left out. Within a row the trials keep their `TRIAL_ORDER`
/// order (easiest first).
pub fn row_specs() -> Vec<RowSpec> {
    let kinds = SHELF_ORDER.iter().map(|t| RowKind::Shelf(*t)).chain([RowKind::Clock]);
    kinds
        .map(|kind| RowSpec {
            kind,
            entries: TRIAL_ORDER
                .iter()
                .filter(|(t, r)| row_kind_of(*t, *r) == kind)
                .copied()
                .collect(),
        })
        .filter(|row| !row.entries.is_empty())
        .collect()
}

/// The key form the "How it works" modal and `type_for_key` speak.
pub fn trial_key(tref: TrialRef) -> String {
    match tref {
        TrialRef::Race(id) => format!("race:{id}"),
        TrialRef::Challenge(token) => format!("ch:{token}"),
    }
}

// ---------------------------------------------------------------------------
// Pure: card art
// ---------------------------------------------------------------------------

/// What a card's picture is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CardArt {
    /// A block: drawn with its own texture layer from the game's atlas.
    Block { layer: u32 },
    /// Any other item: a flat colour swatch (how the game's own slots draw
    /// tools and materials).
    Swatch { rgb: [u8; 3] },
    /// No usable name: the shelf's glyph, drawn large.
    Shelf,
}

fn registry() -> &'static crate::block::BlockRegistry {
    static REG: std::sync::LazyLock<crate::block::BlockRegistry> =
        std::sync::LazyLock::new(crate::block::BlockRegistry::new);
    &REG
}

/// Resolve a `card_art` name (`/give` vocabulary) to a picture. `None` or a
/// name the game does not know falls back to the shelf glyph, so a card is
/// never blank (`trials_lint` makes a bad bundled name a test failure).
pub fn resolve_card_art(name: Option<&str>) -> CardArt {
    let Some(name) = name else { return CardArt::Shelf };
    let Ok(stack) = crate::commands::builtins::give::resolve_item(name, 1) else {
        return CardArt::Shelf;
    };
    match &stack.item {
        crate::item::Item::Block(id) => CardArt::Block { layer: registry().tex_top(*id) },
        other => {
            let c = other.color(registry());
            let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            CardArt::Swatch { rgb: [to_u8(c[0]), to_u8(c[1]), to_u8(c[2])] }
        }
    }
}

// ---------------------------------------------------------------------------
// Built cards (everything a frame needs, resolved once when the gallery opens)
// ---------------------------------------------------------------------------

/// One trial, ready to draw.
pub(crate) struct Card {
    pub(crate) key: String,
    pub(crate) row: RowKind,
    pub(crate) title: String,
    pub(crate) premise: String,
    pub(crate) steps: Vec<String>,
    pub(crate) ordered: bool,
    pub(crate) satoshi: String,
    pub(crate) art: CardArt,
    pub(crate) done: bool,
    /// A race's personal best, in ticks.
    pub(crate) best_ticks: Option<u32>,
    /// A race's imported rival: (label, ticks).
    pub(crate) rival: Option<(String, u32)>,
    /// Set for a race: the id the ghost export / import buttons use.
    pub(crate) race_id: Option<&'static str>,
    /// What Play launches: the very scenario the old column launched.
    pub(crate) def: crate::scenario::ScenarioDef,
}

pub(crate) struct BuiltRow {
    pub(crate) kind: RowKind,
    pub(crate) cards: Vec<Card>,
}

fn build_card(
    row: RowKind,
    tref: TrialRef,
    bests: &TrialBests,
    listing: &std::collections::HashMap<&'static str, String>,
) -> Option<Card> {
    match tref {
        TrialRef::Race(id) => {
            let d = crate::trials::find_def(id)?;
            let dist = {
                let dx = (d.finish_xz[0] - d.start_xz[0]) as f32;
                let dz = (d.finish_xz[1] - d.start_xz[1]) as f32;
                (dx * dx + dz * dz).sqrt().round() as i32
            };
            let best = bests.best(d.id);
            Some(Card {
                key: trial_key(tref),
                row,
                title: d.name.to_string(),
                premise: d.blurb.to_string(),
                steps: vec![
                    format!("Run ~{dist} blocks to the bright finish beacon."),
                    "Beat your best time: you race the ghost of your last run.".to_string(),
                ],
                ordered: false,
                satoshi: crate::scenario::trial_satoshi(d.id).0.to_string(),
                art: CardArt::Shelf,
                done: best.is_some(),
                best_ticks: best.map(|b| b.ticks),
                rival: bests.rivals.get(d.id).map(|r| (r.label.clone(), r.ticks)),
                race_id: Some(d.id),
                def: crate::trials::race_scenario_def(d),
            })
        }
        TrialRef::Challenge(token) => {
            let display = listing.get(token)?;
            let def = crate::scenario::named_builtin_def(token)?;
            let (head, display_premise) =
                display.split_once(" \u{2014} ").unwrap_or((display.as_str(), ""));
            let (tagline, _) = crate::scenario::challenge_help(token);
            let premise = if tagline.is_empty() { display_premise } else { tagline };
            let tasks =
                crate::scenario::objective_tasks(&def, crate::scenario::trial_task_labels(token));
            Some(Card {
                key: trial_key(tref),
                row,
                title: head.to_string(),
                premise: premise.to_string(),
                steps: tasks.lines,
                ordered: tasks.ordered,
                satoshi: crate::scenario::trial_satoshi(token).0.to_string(),
                art: resolve_card_art(def.card_art.as_deref()),
                done: bests.is_challenge_done(display),
                best_ticks: None,
                rival: None,
                race_id: None,
                def,
            })
        }
    }
}

pub(crate) fn build_rows(bests: &TrialBests) -> Vec<BuiltRow> {
    let listing: std::collections::HashMap<&'static str, String> =
        crate::scenario::challenge_listing().into_iter().collect();
    row_specs()
        .into_iter()
        .map(|spec| BuiltRow {
            kind: spec.kind,
            cards: spec
                .entries
                .iter()
                .filter_map(|(_, tref)| build_card(spec.kind, *tref, bests, &listing))
                .collect(),
        })
        .filter(|row| !row.cards.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(row: &RowSpec) -> Vec<&'static str> {
        row.entries
            .iter()
            .map(|(_, r)| match r {
                TrialRef::Race(id) | TrialRef::Challenge(id) => *id,
            })
            .collect()
    }

    #[test]
    fn rows_run_make_tend_tech_brave_then_the_clock() {
        let kinds: Vec<RowKind> = row_specs().iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RowKind::Shelf(TrialType::Make),
                RowKind::Shelf(TrialType::Tend),
                RowKind::Shelf(TrialType::Tech),
                RowKind::Shelf(TrialType::Brave),
                RowKind::Clock,
            ]
        );
        assert_eq!(RowKind::Clock.title(), "Against the clock");
    }

    #[test]
    fn every_trial_lands_in_exactly_one_row() {
        let rows = row_specs();
        let placed: usize = rows.iter().map(|r| r.entries.len()).sum();
        assert_eq!(placed, TRIAL_ORDER.len(), "a trial is missing from, or doubled in, the gallery");
        let mut seen: Vec<TrialRef> = Vec::new();
        for row in &rows {
            for (_, r) in &row.entries {
                assert!(!seen.contains(r), "{r:?} sits in two rows");
                seen.push(*r);
            }
        }
    }

    #[test]
    fn the_clock_row_holds_the_races_roadrunner_and_scavenger() {
        let rows = row_specs();
        let clock = rows.iter().find(|r| r.kind == RowKind::Clock).expect("a clock row");
        let t = tokens(clock);
        for want in ["sprint", "cross-country", "marathon", "roadrunner", "scavenger"] {
            assert!(t.contains(&want), "{want} should be against the clock: {t:?}");
        }
        // Scavenger is typed Make in TRIAL_ORDER but must not stay on that shelf.
        let make = rows.iter().find(|r| r.kind == RowKind::Shelf(TrialType::Make)).unwrap();
        assert!(!tokens(make).contains(&"scavenger"));
        // Nothing else leaks into the clock row.
        assert_eq!(t.len(), 5, "{t:?}");
    }

    #[test]
    fn rows_keep_trial_order_within_a_row() {
        let pos = |tok: &str| {
            TRIAL_ORDER
                .iter()
                .position(|(_, r)| matches!(r, TrialRef::Race(i) | TrialRef::Challenge(i) if *i == tok))
                .unwrap()
        };
        for row in row_specs() {
            let idx: Vec<usize> = tokens(&row).iter().map(|t| pos(t)).collect();
            assert!(idx.windows(2).all(|w| w[0] < w[1]), "{:?} is out of TRIAL_ORDER order", row.kind);
        }
    }

    #[test]
    fn card_art_resolves_blocks_to_a_texture_and_other_items_to_a_swatch() {
        assert!(matches!(resolve_card_art(Some("furnace")), CardArt::Block { .. }));
        assert!(matches!(resolve_card_art(Some("bone")), CardArt::Swatch { .. }));
        assert!(matches!(resolve_card_art(Some("stone_pickaxe")), CardArt::Swatch { .. }));
    }

    #[test]
    fn card_art_falls_back_to_the_shelf_glyph() {
        assert_eq!(resolve_card_art(None), CardArt::Shelf);
        assert_eq!(resolve_card_art(Some("not_a_thing")), CardArt::Shelf);
    }

    #[test]
    fn every_challenge_card_gets_real_art_and_every_race_the_fallback() {
        let rows = build_rows(&TrialBests::default());
        let mut keys = std::collections::HashSet::new();
        let mut total = 0;
        for row in &rows {
            for card in &row.cards {
                total += 1;
                assert!(keys.insert(card.key.clone()), "{} appears twice", card.key);
                if card.race_id.is_some() {
                    assert_eq!(card.art, CardArt::Shelf, "{}", card.key);
                } else {
                    assert_ne!(card.art, CardArt::Shelf, "{} has no usable card_art", card.key);
                }
                assert!(!card.done, "a fresh profile has nothing done");
            }
        }
        assert_eq!(total, TRIAL_ORDER.len());
    }
}
