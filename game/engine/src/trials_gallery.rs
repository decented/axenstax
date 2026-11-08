//! The Trials gallery's data side: which trial sits in which row, how a
//! `card_art` name becomes a picture, and the cards built from them. Pure (no
//! drawing), so it is unit-tested; `trials_gallery_ui` draws it.
//!
//! Design: `docs/vision/trials-two-acts.md` Part C, §C.2 (internal repo). Nothing
//! here changes how a trial plays: a card carries the very scenario the old
//! Trials column launched, and trial order, objectives and persistence are
//! untouched.

use crate::trials::{front_shelf, TrialBests, TrialRef, TrialType, TRIAL_ORDER};
use crate::trials_unlock::{is_new, is_unlocked, next_hints, route_names};
use crate::trials_diorama::{sort_back_to_front, CardArtDef, Cube, DioramaBlock, Shape};

// ---------------------------------------------------------------------------
// Pure: rows
// ---------------------------------------------------------------------------

/// The five shelves, in the order they are shown (Bloom, act two, last).
pub const SHELF_ORDER: [TrialType; 5] =
    [TrialType::Make, TrialType::Tend, TrialType::Tech, TrialType::Brave, TrialType::Bloom];

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

/// Which row a trial belongs in: anything typed `Race` (the ghost races) plus
/// Scavenger go against the clock; the rest sit on their own shelf.
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
#[derive(Clone, Debug, PartialEq)]
pub enum CardArt {
    /// One or more blocks drawn as an isometric diorama from the game's atlas
    /// (a lone block is a one-cube diorama).
    Diorama(Vec<Cube>),
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

/// Block names whose picture is a flat tile lying on the floor, not a cube.
const FLAT_BLOCKS: [&str; 3] = ["track", "pressure_plate", "cable"];

/// How a block is drawn: liquids bright, the flat tiles flat, anything
/// non-solid (crops, torches, doors) as an upright sprite, the rest as cubes.
fn shape_of(id: crate::block::BlockId) -> Shape {
    use crate::block;
    let reg = registry();
    if id == block::WATER || id == block::LAVA {
        Shape::Fluid
    } else if FLAT_BLOCKS.iter().any(|n| block_name_matches(id, n)) {
        Shape::Flat
    } else if !reg.is_solid(id) {
        Shape::Sprite
    } else {
        Shape::Cube
    }
}

fn block_name_matches(id: crate::block::BlockId, name: &str) -> bool {
    crate::commands::builtins::give::resolve_item(name, 1)
        .ok()
        .is_some_and(|s| matches!(s.item, crate::item::Item::Block(b) if b == id))
}

/// A block name (`/give` vocabulary) at a grid cell, resolved to the faces to
/// draw. `None` when the name is not a block.
pub fn resolve_cube(name: &str, pos: [i32; 3]) -> Option<Cube> {
    let stack = crate::commands::builtins::give::resolve_item(name, 1).ok()?;
    match stack.item {
        crate::item::Item::Block(id) => {
            let reg = registry();
            Some(Cube { pos, top: reg.tex_top(id), side: reg.tex_side(id), shape: shape_of(id) })
        }
        _ => None,
    }
}

/// Resolve a diorama's blocks, sorted back to front. Blocks that do not
/// resolve, or sit off the grid, are skipped (`trials_lint` makes one in a bundled trial a failure).
pub fn resolve_diorama(blocks: &[DioramaBlock]) -> Vec<Cube> {
    let mut cubes: Vec<Cube> = blocks
        .iter()
        .take(crate::trials_diorama::MAX_DIORAMA_BLOCKS)
        .filter(|b| b.at.iter().all(|c| c.abs() <= crate::trials_diorama::MAX_COORD))
        .filter_map(|b| resolve_cube(&b.block, b.at))
        .collect();
    sort_back_to_front(&mut cubes);
    cubes
}

/// Resolve a `card_art` to a picture. `None` or a name the game does not know
/// falls back to the shelf glyph, so a card is never blank (`trials_lint`
/// makes a bad bundled value a test failure).
pub fn resolve_card_art(art: Option<&CardArtDef>) -> CardArt {
    let Some(art) = art else { return CardArt::Shelf };
    match art {
        CardArtDef::Diorama(blocks) => {
            let cubes = resolve_diorama(blocks);
            if cubes.is_empty() {
                CardArt::Shelf
            } else {
                CardArt::Diorama(cubes)
            }
        }
        CardArtDef::Name(name) => {
            if let Some(cube) = resolve_cube(name, [0, 0, 0]) {
                return CardArt::Diorama(vec![cube]);
            }
            let Ok(stack) = crate::commands::builtins::give::resolve_item(name, 1) else {
                return CardArt::Shelf;
            };
            let c = stack.item.color(registry());
            let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            CardArt::Swatch { rgb: [to_u8(c[0]), to_u8(c[1]), to_u8(c[2])] }
        }
    }
}

/// The fixed diorama for each bundled race (races carry no `card_art` data):
/// a short course with its finish marker.
const RACE_ART: [(&str, &[(&str, [i32; 3])]); 3] = [
    (
        "sprint",
        &[
            ("sand", [0, 0, 0]),
            ("sand", [1, 0, 0]),
            ("sand", [2, 0, 0]),
            ("sand", [3, 0, 0]),
            ("lamp", [3, 1, 0]),
            ("pressure_plate", [0, 1, 0]),
        ],
    ),
    (
        "cross-country",
        &[
            ("grass", [0, 0, 0]),
            ("grass", [1, 0, 0]),
            ("dirt", [2, 0, 0]),
            ("stone", [2, 1, 0]),
            ("grass", [3, 0, 0]),
            ("oak_leaves", [1, 1, 1]),
            ("oak_log", [1, 0, 1]),
            ("lamp", [3, 1, 0]),
        ],
    ),
    (
        "marathon",
        &[
            ("cobblestone", [0, 0, 0]),
            ("cobblestone", [1, 0, 0]),
            ("cobblestone", [2, 0, 0]),
            ("cobblestone", [3, 0, 0]),
            ("cobblestone", [4, 0, 0]),
            ("fence_post", [2, 1, 1]),
            ("fence_post", [4, 1, 1]),
            ("lamp", [4, 1, 0]),
        ],
    ),
];

/// A bundled race's diorama cells, `(block name, [x, y, z])` (empty for an
/// unknown race). The Trials island builds them in real blocks.
pub fn race_art_cells(race_id: &str) -> &'static [(&'static str, [i32; 3])] {
    RACE_ART.iter().find(|(id, _)| *id == race_id).map_or(&[], |(_, cells)| cells)
}

/// The picture for a bundled race.
pub fn race_art(race_id: &str) -> CardArt {
    let Some((_, cells)) = RACE_ART.iter().find(|(id, _)| *id == race_id) else {
        return CardArt::Shelf;
    };
    let blocks: Vec<DioramaBlock> = cells
        .iter()
        .map(|(b, at)| DioramaBlock { block: (*b).to_string(), at: *at })
        .collect();
    let cubes = resolve_diorama(&blocks);
    if cubes.is_empty() { CardArt::Shelf } else { CardArt::Diorama(cubes) }
}

// ---------------------------------------------------------------------------
// Built cards (everything a frame needs, resolved once when the gallery opens)
// ---------------------------------------------------------------------------

/// How many frosted hint cards one row shows after its unlocked cards: the
/// player sees the edge of the map, never its size.
pub(crate) const HINTS_PER_ROW: usize = 2;

/// One way (`AnyOf` route) of a trial, for the hero view's "Pick a way:" list.
pub(crate) struct RouteView {
    pub(crate) name: String,
    /// Whether this trial has been done by this way (a remembered mark, never a total).
    pub(crate) done: bool,
    /// The way's own steps (task labels), numbered when `ordered`.
    pub(crate) ordered: bool,
    pub(crate) steps: Vec<String>,
}

/// A trial the player has not unlocked yet, shown as a frosted card: the
/// picture dimmed, no title, and one line of nudge. Not playable.
#[derive(Clone)]
pub(crate) struct HintCard {
    pub(crate) token: &'static str,
    pub(crate) art: CardArt,
    pub(crate) hint: String,
}

/// One trial, ready to draw.
pub(crate) struct Card {
    pub(crate) key: String,
    /// The trial's token (challenge token or race id): its postcards' key.
    pub(crate) token: &'static str,
    pub(crate) row: RowKind,
    pub(crate) title: String,
    pub(crate) premise: String,
    pub(crate) steps: Vec<String>,
    pub(crate) ordered: bool,
    pub(crate) satoshi: String,
    pub(crate) art: CardArt,
    pub(crate) done: bool,
    /// Appeared by doing and its card has not been opened yet: wears the quiet "New" mark.
    pub(crate) is_new: bool,
    /// An `AnyOf` trial's ways, in authored order (empty for a one-way trial).
    pub(crate) routes: Vec<RouteView>,
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
    /// The unlocked trials, in `TRIAL_ORDER` order.
    pub(crate) cards: Vec<Card>,
    /// Up to [`HINTS_PER_ROW`] frosted nudges toward trials still hidden.
    pub(crate) hints: Vec<HintCard>,
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
                token: d.id,
                row,
                title: d.name.to_string(),
                premise: d.blurb.to_string(),
                steps: vec![
                    format!("Run ~{dist} blocks to the bright finish beacon."),
                    "Beat your best time: you race the ghost of your last run.".to_string(),
                ],
                ordered: false,
                satoshi: crate::scenario::trial_satoshi(d.id).0.to_string(),
                art: race_art(d.id),
                done: best.is_some(),
                is_new: false,
                routes: Vec::new(),
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
            let ways_done = bests.ways_done(display);
            let groups = tasks.groups();
            let routes: Vec<RouteView> = route_names(token)
                .iter()
                .map(|name| {
                    let group = groups.iter().find(|g| g.name == Some(name.as_str()));
                    RouteView {
                        name: name.clone(),
                        done: ways_done.contains(&name.as_str()),
                        ordered: group.is_some_and(|g| g.ordered),
                        steps: group
                            .map(|g| g.lines.iter().map(|(_, l)| (*l).to_string()).collect())
                            .unwrap_or_default(),
                    }
                })
                .collect();
            Some(Card {
                key: trial_key(tref),
                token,
                row,
                title: head.to_string(),
                // A trial with ways (a Place trial's "life" / "people") lists
                // the ones the player has done, once there is one.
                premise: if routes.is_empty() && !ways_done.is_empty() {
                    format!("{premise} Ways you have done it: {}.", ways_done.join(", "))
                } else {
                    premise.to_string()
                },
                steps: tasks.lines,
                ordered: tasks.ordered,
                satoshi: crate::scenario::trial_satoshi(token).0.to_string(),
                art: resolve_card_art(def.card_art.as_ref()),
                done: bests.is_challenge_done(display),
                is_new: is_new(token, bests),
                routes,
                best_ticks: None,
                rival: None,
                race_id: None,
                def,
            })
        }
    }
}

/// A frosted hint card for the hidden trial `token`, with its row.
fn build_hint(token: &'static str, hint: String) -> Option<(RowKind, HintCard)> {
    let (ttype, tref) = TRIAL_ORDER
        .iter()
        .find(|(_, r)| matches!(r, TrialRef::Challenge(t) if *t == token))?;
    let def = crate::scenario::named_builtin_def(token)?;
    Some((
        row_kind_of(*ttype, *tref),
        HintCard { token, art: resolve_card_art(def.card_art.as_ref()), hint },
    ))
}

/// The gallery's rows for these bests: each shelf shows only the trials that
/// are unlocked (the front ten always are), in `TRIAL_ORDER`, then up to two
/// frosted hint cards for what comes next. A row with neither is left out.
pub(crate) fn build_rows(bests: &TrialBests) -> Vec<BuiltRow> {
    let listing: std::collections::HashMap<&'static str, String> =
        crate::scenario::challenge_listing().into_iter().collect();
    let hints: Vec<(RowKind, HintCard)> = next_hints(bests)
        .into_iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .filter_map(|(token, text)| build_hint(token, text))
        .collect();
    row_specs()
        .into_iter()
        .map(|spec| BuiltRow {
            kind: spec.kind,
            cards: spec
                .entries
                .iter()
                .filter(|(ttype, tref)| {
                    // The front ten are always there; the rest appear when unlocked.
                    let token = match tref {
                        TrialRef::Race(id) | TrialRef::Challenge(id) => *id,
                    };
                    front_shelf().contains(&(*ttype, *tref)) || is_unlocked(token, bests)
                })
                .filter_map(|(_, tref)| build_card(spec.kind, *tref, bests, &listing))
                .collect(),
            hints: Vec::new(),
        })
        .map(|mut row| {
            row.hints = hints
                .iter()
                .filter(|(kind, _)| *kind == row.kind)
                .take(HINTS_PER_ROW)
                .map(|(_, h)| h.clone())
                .collect();
            row
        })
        .filter(|row| !row.cards.is_empty() || !row.hints.is_empty())
        .collect()
}

/// The gallery's card for one unlocked trial (`token` is a challenge token or
/// a race id), as its hero view shows it. `None` when it is not unlocked.
pub(crate) fn card_for(token: &str, bests: &TrialBests) -> Option<Card> {
    build_rows(bests).into_iter().flat_map(|r| r.cards).find(|c| c.token == token)
}

/// Keep only the fixture postcards that fit `bests`: a postcard stays when its
/// trial is done there (or when no unlock fixture is in play, `bests` empty of
/// completions, so a postcards-only shot keeps them all).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn postcards_matching(bests: &TrialBests, cards: Vec<crate::postcard::Postcard>) -> Vec<crate::postcard::Postcard> {
    if bests.completed_challenges.is_empty() {
        return cards;
    }
    let listing: std::collections::HashMap<&'static str, String> =
        crate::scenario::challenge_listing().into_iter().collect();
    cards
        .into_iter()
        .filter(|c| listing.get(c.token.as_str()).is_some_and(|d| bests.is_challenge_done(d)))
        .collect()
}

/// Made-up unlock states for the `--shot-lobby --fixture-unlocks <name>`
/// screenshots and tests (never written to any store): `fresh` (nothing done),
/// `started` (a few front-shelf trials done) and `several` (most of the front
/// shelf done, two ways of Camp Cook, and a catch and a bucket seen so those
/// trials appeared).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn fixture_bests(name: &str) -> TrialBests {
    use crate::scenario::ChallengeEvent;
    let listing: std::collections::HashMap<&'static str, String> =
        crate::scenario::challenge_listing().into_iter().collect();
    let mut bests = TrialBests::default();
    let done = |bests: &mut TrialBests, token: &str| {
        if let Some(display) = listing.get(token) {
            bests.mark_challenge_done(display);
        }
    };
    match name {
        "started" => {
            for t in ["onboarding", "craft", "smelt"] {
                done(&mut bests, t);
            }
        }
        "several" => {
            for t in ["onboarding", "mine", "craft", "build", "smelt", "harvest", "kill", "cook-three"] {
                done(&mut bests, t);
            }
            if let Some(display) = listing.get("cook-three") {
                bests.mark_way_done(display, "Roast three");
            }
            bests.record_event(&ChallengeEvent::CatchFish);
            bests.record_event(&ChallengeEvent::UseBucket);
        }
        _ => {}
    }
    bests.refresh_unlocks();
    bests
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
    fn rows_run_make_tend_tech_brave_bloom_then_the_clock() {
        let kinds: Vec<RowKind> = row_specs().iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RowKind::Shelf(TrialType::Make),
                RowKind::Shelf(TrialType::Tend),
                RowKind::Shelf(TrialType::Tech),
                RowKind::Shelf(TrialType::Brave),
                RowKind::Shelf(TrialType::Bloom),
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
    fn the_clock_row_holds_the_races_and_scavenger() {
        let rows = row_specs();
        let clock = rows.iter().find(|r| r.kind == RowKind::Clock).expect("a clock row");
        let t = tokens(clock);
        for want in ["sprint", "cross-country", "marathon", "scavenger"] {
            assert!(t.contains(&want), "{want} should be against the clock: {t:?}");
        }
        // Scavenger is typed Make in TRIAL_ORDER but must not stay on that shelf.
        let make = rows.iter().find(|r| r.kind == RowKind::Shelf(TrialType::Make)).unwrap();
        assert!(!tokens(make).contains(&"scavenger"));
        // Nothing else leaks into the clock row.
        assert_eq!(t.len(), 4, "{t:?}");
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

    fn name(n: &str) -> CardArtDef {
        CardArtDef::Name(n.to_string())
    }

    #[test]
    fn a_single_block_name_is_a_one_cube_diorama_and_other_items_a_swatch() {
        let CardArt::Diorama(cubes) = resolve_card_art(Some(&name("furnace"))) else {
            panic!("a block name should resolve to a diorama");
        };
        assert_eq!(cubes.len(), 1);
        assert_eq!(cubes[0].shape, Shape::Cube);
        assert!(matches!(resolve_card_art(Some(&name("bone"))), CardArt::Swatch { .. }));
        assert!(matches!(resolve_card_art(Some(&name("stone_pickaxe"))), CardArt::Swatch { .. }));
    }

    #[test]
    fn card_art_falls_back_to_the_shelf_glyph() {
        assert_eq!(resolve_card_art(None), CardArt::Shelf);
        assert_eq!(resolve_card_art(Some(&name("not_a_thing"))), CardArt::Shelf);
        assert_eq!(resolve_card_art(Some(&CardArtDef::Diorama(vec![]))), CardArt::Shelf);
    }

    #[test]
    fn a_diorama_resolves_per_face_textures_in_painter_order() {
        let blocks = vec![
            DioramaBlock { block: "grass".into(), at: [1, 0, 1] },
            DioramaBlock { block: "furnace".into(), at: [0, 0, 0] },
            DioramaBlock { block: "not_a_block".into(), at: [5, 0, 0] },
            DioramaBlock { block: "bone".into(), at: [2, 0, 0] },
        ];
        let CardArt::Diorama(cubes) = resolve_card_art(Some(&CardArtDef::Diorama(blocks))) else {
            panic!("expected a diorama");
        };
        // The unknown name and the non-block item are skipped; the rest is sorted.
        assert_eq!(cubes.len(), 2);
        assert_eq!(cubes[0].pos, [0, 0, 0]);
        assert_eq!(cubes[1].pos, [1, 0, 1]);
        let reg = registry();
        assert_eq!(cubes[1].top, reg.tex_top(crate::block::GRASS));
        assert_eq!(cubes[1].side, reg.tex_side(crate::block::GRASS));
        assert_ne!(cubes[1].top, cubes[1].side, "grass has a different top and side");
    }

    #[test]
    fn a_diorama_is_capped_at_its_block_limit() {
        let blocks: Vec<DioramaBlock> = (0..20)
            .map(|i| DioramaBlock { block: "stone".into(), at: [i % 5, 0, i / 5] })
            .collect();
        let CardArt::Diorama(cubes) = resolve_card_art(Some(&CardArtDef::Diorama(blocks))) else {
            panic!("expected a diorama");
        };
        assert_eq!(cubes.len(), crate::trials_diorama::MAX_DIORAMA_BLOCKS);
    }

    #[test]
    fn blocks_pick_their_drawing_shape() {
        let shape = |n: &str| resolve_cube(n, [0, 0, 0]).unwrap().shape;
        assert_eq!(shape("stone"), Shape::Cube);
        assert_eq!(shape("water"), Shape::Fluid);
        assert_eq!(shape("lava"), Shape::Fluid);
        assert_eq!(shape("track"), Shape::Flat);
        assert_eq!(shape("wheat_mature"), Shape::Sprite);
    }

    #[test]
    fn every_race_has_a_diorama() {
        for d in crate::trials::CATALOG {
            assert!(matches!(race_art(d.id), CardArt::Diorama(_)), "{}", d.id);
        }
    }

    fn tokens_in(rows: &[BuiltRow]) -> Vec<&'static str> {
        rows.iter().flat_map(|r| r.cards.iter().map(|c| c.token)).collect()
    }

    fn front_tokens() -> Vec<&'static str> {
        front_shelf()
            .iter()
            .map(|(_, r)| match r {
                TrialRef::Race(id) | TrialRef::Challenge(id) => *id,
            })
            .collect()
    }

    #[test]
    fn a_fresh_profile_shows_the_front_ten_and_no_locked_trial() {
        let rows = build_rows(&fixture_bests("fresh"));
        let shown = tokens_in(&rows);
        for t in front_tokens() {
            assert!(shown.contains(&t), "the front shelf always shows {t}");
        }
        // Nothing hidden by an unlock rule leaks in; races and Scavenger have none.
        for t in &shown {
            assert!(
                front_tokens().contains(t) || !crate::trials_unlock::catalogue().iter().any(|e| e.token == *t && e.unlock.is_some()),
                "{t} is locked on a fresh profile"
            );
        }
        // Nothing started, nothing nudged.
        assert!(rows.iter().all(|r| r.hints.is_empty()));
        assert!(rows.iter().flat_map(|r| &r.cards).all(|c| !c.is_new));
    }

    #[test]
    fn shelves_keep_trial_order_and_hide_what_is_locked() {
        let rows = build_rows(&fixture_bests("several"));
        let pos = |tok: &str| {
            TRIAL_ORDER
                .iter()
                .position(|(_, r)| matches!(r, TrialRef::Race(i) | TrialRef::Challenge(i) if *i == tok))
                .unwrap()
        };
        for row in &rows {
            let idx: Vec<usize> = row.cards.iter().map(|c| pos(c.token)).collect();
            assert!(idx.windows(2).all(|w| w[0] < w[1]), "{:?} is out of TRIAL_ORDER order", row.kind);
            for c in &row.cards {
                assert!(is_unlocked(c.token, &fixture_bests("several")), "{} shown while locked", c.token);
            }
        }
    }

    #[test]
    fn hints_come_after_the_cards_at_most_two_a_row_and_never_for_a_shown_trial() {
        for name in ["started", "several"] {
            let bests = fixture_bests(name);
            let rows = build_rows(&bests);
            let shown = tokens_in(&rows);
            let mut any = false;
            for row in &rows {
                assert!(row.hints.len() <= HINTS_PER_ROW, "{name}: {:?} has too many hints", row.kind);
                assert!(!row.cards.is_empty() || !row.hints.is_empty());
                for h in &row.hints {
                    any = true;
                    assert!(!h.hint.trim().is_empty());
                    assert!(!shown.contains(&h.token), "{name}: {} is both shown and hinted", h.token);
                    assert!(!is_unlocked(h.token, &bests));
                }
            }
            assert!(any, "{name}: a started profile gets at least one hint");
        }
    }

    #[test]
    fn a_started_profile_unlocks_what_it_did_and_hints_at_what_is_one_step_off() {
        let rows = build_rows(&fixture_bests("started"));
        let shown = tokens_in(&rows);
        let hinted: Vec<&str> = rows.iter().flat_map(|r| r.hints.iter().map(|h| h.token)).collect();
        // Smelt done, so The Light Lab (after: smelt) has appeared; craft done, so Colour Lab.
        assert!(shown.contains(&"power") && shown.contains(&"dye"), "{shown:?}");
        // Suit Up wants smelt AND kill: one is done, so it is nudged, not shown.
        assert!(hinted.contains(&"armour-up") && !shown.contains(&"armour-up"), "{hinted:?}");
    }

    #[test]
    fn unlocked_extras_wear_new_until_opened() {
        let mut bests = fixture_bests("several");
        let rows = build_rows(&bests);
        let card = |rows: &[BuiltRow], t: &str| {
            rows.iter().flat_map(|r| &r.cards).find(|c| c.token == t).map(|c| c.is_new)
        };
        assert_eq!(card(&rows, "fish"), Some(true), "a catch seen unlocks the angler, new");
        assert_eq!(card(&rows, "bucket"), Some(true));
        assert_eq!(card(&rows, "smelt"), Some(false), "a front-shelf trial is never new");
        assert!(bests.mark_card_opened("fish"));
        let rows = build_rows(&bests);
        assert_eq!(card(&rows, "fish"), Some(false), "opening the card clears the mark");
        assert_eq!(card(&rows, "bucket"), Some(true));
    }

    #[test]
    fn a_multi_route_card_marks_only_the_ways_done() {
        let rows = build_rows(&fixture_bests("several"));
        let cook = rows.iter().flat_map(|r| &r.cards).find(|c| c.token == "cook-three").unwrap();
        let names: Vec<&str> = cook.routes.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Catch, cook and eat", "Cook and eat", "Roast three"]);
        let done: Vec<bool> = cook.routes.iter().map(|r| r.done).collect();
        assert_eq!(done, vec![false, false, true]);
        assert!(cook.routes.iter().all(|r| !r.steps.is_empty()), "every way lists its own steps");
        // A one-way trial has no routes.
        let smelt = rows.iter().flat_map(|r| &r.cards).find(|c| c.token == "smelt").unwrap();
        assert!(smelt.routes.is_empty());
    }

    #[test]
    fn every_card_gets_real_art() {
        // Unlock everything so every bundled trial is built.
        let mut bests = TrialBests::default();
        for (_, r) in TRIAL_ORDER {
            if let TrialRef::Challenge(t) = r {
                bests.unlocked.insert((*t).to_string());
            }
        }
        let rows = build_rows(&bests);
        let mut keys = std::collections::HashSet::new();
        let mut total = 0;
        for row in &rows {
            for card in &row.cards {
                total += 1;
                assert!(keys.insert(card.key.clone()), "{} appears twice", card.key);
                assert_ne!(card.art, CardArt::Shelf, "{} has no usable card_art", card.key);
                assert!(!card.done, "a fresh profile has nothing done");
            }
        }
        assert_eq!(total, TRIAL_ORDER.len());
    }

    #[test]
    fn fixture_postcards_follow_what_is_done() {
        let cards = crate::postcard::fixtures();
        let kept = postcards_matching(&fixture_bests("several"), cards.clone());
        assert!(kept.iter().any(|c| c.token == "craft"));
        assert!(kept.iter().all(|c| c.token != "fish" && c.token != "power"), "undone trials keep no postcard");
        assert_eq!(postcards_matching(&fixture_bests("fresh"), cards.clone()).len(), cards.len());
    }

    #[test]
    fn every_hidden_trial_has_real_art_for_its_hint_card() {
        for (_, r) in TRIAL_ORDER {
            let TrialRef::Challenge(t) = r else { continue };
            let (_, hint) = build_hint(t, "Something\u{2026}".into()).expect("a hint card builds");
            assert_ne!(hint.art, CardArt::Shelf, "{t} has no usable card_art");
        }
    }
}
