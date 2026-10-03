//! Trial-authoring guardrails (harness Task 1).
//!
//! `EXPLORER_CHALLENGES` + the onboarding arc were hand-audited and confirmed
//! correct — this module exists for the NEXT trial, likely authored by a
//! weak/cheap model. Every check here turns a silent-failure authoring
//! mistake (an objective that can never complete, an arena that can't
//! furnish its own targets, a stray "earn"/"sats" word) into a loud,
//! specific `cargo test` failure instead of a trial that ships broken.
//!
//! Each check is a PURE function `fn check_x(&[(&str, ScenarioDef)]) ->
//! Result<(), String>` with a thin `#[test]` wrapper — so a companion
//! "bites" test can hand it a known-bad fixture and assert it returns `Err`.
//! A lint that has never been proven to fail on bad input is not a
//! guardrail, so those negative tests are permanent, not scaffolding.

use crate::block::{self, BlockId};
use crate::breeding::{offspring_kind, pair_allowed};
use crate::commands::builtins::give::resolve_item;
use crate::mob::MobType;
use crate::scenario::{
    ArenaSetup, ChallengeEvent, Objective, ScenarioDef, ScenarioKind, ScenarioTuning, Scoring,
    trial_task_labels,
};
use std::collections::HashSet;

// ─────────────────────────────────────────────────────────────────────────
// Shared fixtures
// ─────────────────────────────────────────────────────────────────────────

/// Every bundled challenge, `(token, def)`, in the exact form `named_builtin_def`
/// resolves them by — the onboarding arc first (its own token), then every
/// `EXPLORER_CHALLENGES` entry. This is the corpus every check below runs over.
fn all_bundled_defs() -> Vec<(&'static str, ScenarioDef)> {
    let mut out = vec![(
        "onboarding",
        crate::scenario::load_scenario_def(crate::scenario::ONBOARDING_JSON)
            .expect("bundled onboarding.json must parse"),
    )];
    for (name, bytes) in crate::scenario::EXPLORER_CHALLENGES {
        out.push((
            name,
            crate::scenario::load_scenario_def(bytes)
                .unwrap_or_else(|e| panic!("bundled explorer challenge {name} must parse: {e}")),
        ));
    }
    out
}

/// Minimal `ScenarioDef` fixture for negative tests — a bare `Test`-kind def
/// wrapping whatever `objective`/`arena`/`kit` the test wants to poke at.
fn fixture_def(objective: Objective) -> ScenarioDef {
    ScenarioDef {
        kind: ScenarioKind::Test,
        display_name: "Fixture".to_string(),
        lock_creative: false,
        arena_mode: crate::scenario::ArenaMode::Reuse,
        kit: vec![],
        objective,
        scoring: Scoring::None,
        arena_seed: None,
        tuning: ScenarioTuning::default(),
        world_type: None,
        game_mode: None,
        time_lock: None,
        weather_lock: None,
        mobs_enabled: None,
        trial_race: None,
        arena: None,
    }
}

/// Every `ChallengeEvent`'s bare variant name — exhaustive match so a future
/// variant added to the enum forces a compile error HERE, prompting whoever
/// adds it to also teach this module (and the fired-event scan) about it.
fn event_variant_name(e: &ChallengeEvent) -> &'static str {
    match e {
        ChallengeEvent::BreakBlock { .. } => "BreakBlock",
        ChallengeEvent::PlaceBlock { .. } => "PlaceBlock",
        ChallengeEvent::CraftItem => "CraftItem",
        ChallengeEvent::CookAtCampfire => "CookAtCampfire",
        ChallengeEvent::TameMob => "TameMob",
        ChallengeEvent::VendorSale => "VendorSale",
        ChallengeEvent::WorkshopPublish => "WorkshopPublish",
        ChallengeEvent::ClaimPlot => "ClaimPlot",
        ChallengeEvent::GainMaterial { .. } => "GainMaterial",
        ChallengeEvent::KillMob => "KillMob",
        ChallengeEvent::EatFood => "EatFood",
        ChallengeEvent::HarvestCrop => "HarvestCrop",
        ChallengeEvent::SmeltItem => "SmeltItem",
        ChallengeEvent::CatchFish => "CatchFish",
        ChallengeEvent::RideEntity => "RideEntity",
        ChallengeEvent::PowerDevice => "PowerDevice",
        ChallengeEvent::UsePiston => "UsePiston",
        ChallengeEvent::Detonate => "Detonate",
        ChallengeEvent::UseBucket => "UseBucket",
        ChallengeEvent::ShearOrMilk => "ShearOrMilk",
        ChallengeEvent::BreedAnimals { .. } => "BreedAnimals",
        ChallengeEvent::SourceTurned { .. } => "SourceTurned",
        ChallengeEvent::CompleteBuildGuide => "CompleteBuildGuide",
        ChallengeEvent::SaveSkin => "SaveSkin",
        ChallengeEvent::SpawnRig => "SpawnRig",
        ChallengeEvent::SendFeedback => "SendFeedback",
    }
}

/// Collect every `Action` leaf's `(event, count)` directly under `obj` — one
/// level deep, matching the v1 objective-shape invariant that Sequence/
/// Checklist steps ARE Action leaves (nesting deeper is check 2's concern:
/// [`check_sequence_and_checklist_leaves_are_actions`] flags anything that
/// isn't, so this only needs to look at the immediate children).
fn collect_leaf_events(obj: &Objective) -> Vec<(&ChallengeEvent, u32)> {
    match obj {
        Objective::Action { event, count } => vec![(event, *count)],
        Objective::Sequence { steps } | Objective::Checklist { items: steps } => steps
            .iter()
            .filter_map(|s| match s {
                Objective::Action { event, count } => Some((event, *count)),
                _ => None,
            })
            .collect(),
        Objective::Timed { .. } | Objective::FirstSatori | Objective::FreeRoam => vec![],
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Check 1 — every objective event is actually fired somewhere in gameplay
// ─────────────────────────────────────────────────────────────────────────

/// A future new fire-site living outside `game_loop.rs` needs its source
/// scanned here too — add another `include_str!` + `haystack.contains` pass.
fn check_events_fired_in_gameplay(defs: &[(&str, ScenarioDef)], game_loop_src: &str) -> Result<(), String> {
    for (token, def) in defs {
        for (event, _count) in collect_leaf_events(&def.objective) {
            let variant = event_variant_name(event);
            let needle = format!("ChallengeEvent::{variant}");
            if !game_loop_src.contains(&needle) {
                return Err(format!(
                    "{token}: objective references `{needle}`, but no fire/construct site for it \
                     was found in game_loop.rs — an objective listening for an event gameplay \
                     never fires can NEVER complete. Either wire up a `self.fire_challenge({needle} \
                     {{ .. }})` / `scenario.on_event({needle} {{ .. }})` call at the real gameplay \
                     action this represents, or fix the objective if the event kind was a typo. \
                     (If the fire site lives in a file other than game_loop.rs, scan that file too \
                     in `check_events_fired_in_gameplay`.)"
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn every_objective_event_is_fired_in_gameplay() {
    let game_loop_src = include_str!("../game_loop.rs");
    check_events_fired_in_gameplay(&all_bundled_defs(), game_loop_src)
        .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn check_events_fired_in_gameplay_bites_on_a_never_fired_event() {
    // Prove it: a def referencing a real event, scanned against a haystack
    // that (deliberately) never mentions it must fail loudly, naming both
    // the offending trial and the missing fire site.
    let bad = vec![("bad-trial", fixture_def(Objective::Action { event: ChallengeEvent::KillMob, count: 1 }))];
    let err = check_events_fired_in_gameplay(&bad, "no challenge events are fired in this haystack")
        .expect_err("a never-fired event must be rejected");
    assert!(err.contains("bad-trial"), "error must name the offending trial: {err}");
    assert!(err.contains("ChallengeEvent::KillMob"), "error must name the missing event: {err}");
}

// ─────────────────────────────────────────────────────────────────────────
// Check 2 — Sequence/Checklist leaves must be Action (never complete otherwise)
// ─────────────────────────────────────────────────────────────────────────

fn check_sequence_and_checklist_leaves_are_actions(defs: &[(&str, ScenarioDef)]) -> Result<(), String> {
    for (token, def) in defs {
        let (kind, steps) = match &def.objective {
            Objective::Sequence { steps } => ("Sequence", steps),
            Objective::Checklist { items } => ("Checklist", items),
            _ => continue,
        };
        for (i, step) in steps.iter().enumerate() {
            if !matches!(step, Objective::Action { .. }) {
                return Err(format!(
                    "{token}: {kind} step {i} is a nested {step:?}, not an `Action` leaf. \
                     `ScenarioState::objective_is_complete` only recognises Action leaves inside a \
                     {kind} — a nested Sequence/Checklist/Timed parses fine but can NEVER complete, \
                     silently stalling the trial forever. Flatten step {i} to a plain \
                     `Action {{ event: .., count: .. }}`."
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn sequence_and_checklist_leaves_are_actions() {
    check_sequence_and_checklist_leaves_are_actions(&all_bundled_defs())
        .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn check_leaves_are_actions_bites_on_a_nested_sequence() {
    let bad = vec![(
        "bad-trial",
        fixture_def(Objective::Sequence {
            steps: vec![Objective::Sequence {
                steps: vec![Objective::Action { event: ChallengeEvent::CraftItem, count: 1 }],
            }],
        }),
    )];
    let err = check_sequence_and_checklist_leaves_are_actions(&bad)
        .expect_err("a Sequence-of-Sequence must be rejected");
    assert!(err.contains("bad-trial"));
    assert!(err.contains("step 0"));
}

#[test]
fn check_leaves_are_actions_bites_on_a_timed_checklist_item() {
    let bad = vec![(
        "bad-trial-2",
        fixture_def(Objective::Checklist {
            items: vec![
                Objective::Action { event: ChallengeEvent::CraftItem, count: 1 },
                Objective::Timed { ticks: 100 },
            ],
        }),
    )];
    let err = check_sequence_and_checklist_leaves_are_actions(&bad)
        .expect_err("a Timed leaf inside a Checklist must be rejected");
    assert!(err.contains("step 1"));
}

// ─────────────────────────────────────────────────────────────────────────
// Check 3 — compliance banlist over EVERY trial text surface
// ─────────────────────────────────────────────────────────────────────────

/// Duplicated from `scenario.rs`'s `trial_text_has_no_money_or_earning_words`
/// (that test is the source of truth for the word list; keep the two in
/// sync by hand — factoring into one shared const wasn't worth the extra
/// cross-module plumbing for a 16-entry list). This check covers the text
/// surfaces that test does NOT: `display_name`, the authored task labels,
/// and kit item names.
const BANNED_MONEY_WORDS: &[&str] = &[
    "sats", "bitcoin", "btc", "earn", "earning", "earned", "payout", "payouts",
    "wallet", "money", "cash", "cashback", "prize", "prizes", "sell", "buy",
    "lightning", "wages", "salary",
];

fn tokenize_words(corpus: &str) -> HashSet<String> {
    corpus
        .to_lowercase()
        .split(|c: char| !(c.is_ascii_alphabetic() || c == '\''))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn check_banlist_covers_all_text_surfaces(defs: &[(&str, ScenarioDef)]) -> Result<(), String> {
    for (token, def) in defs {
        let mut corpus = def.display_name.clone();
        corpus.push(' ');
        for line in trial_task_labels(token) {
            corpus.push_str(line);
            corpus.push(' ');
        }
        for kit in &def.kit {
            corpus.push_str(&kit.name);
            corpus.push(' ');
        }
        let words = tokenize_words(&corpus);
        for banned in BANNED_MONEY_WORDS {
            if words.contains(*banned) {
                return Err(format!(
                    "{token} (\"{}\"): display_name/task-label/kit-name text contains banned \
                     money/earning word {banned:?}. Trials must never frame play as money/earning \
                     (compliance red line — see CLAUDE.md \"Regulatory Red Lines\"). Reword the \
                     offending text; \"trade/barter/swap\" is fine, \"buy/sell/earn/sats\" is not.",
                    def.display_name,
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn compliance_banlist_covers_all_text_surfaces() {
    check_banlist_covers_all_text_surfaces(&all_bundled_defs()).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn check_banlist_bites_on_a_banned_word_in_display_name() {
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::CraftItem, count: 1 });
    def.display_name = "Buy Your First Sword".to_string();
    let err = check_banlist_covers_all_text_surfaces(&[("bad-trial", def)])
        .expect_err("a money word in display_name must be rejected");
    assert!(err.contains("bad-trial"));
    assert!(err.contains("\"buy\""));
}

#[test]
fn check_banlist_bites_on_a_banned_word_in_kit_name() {
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::CraftItem, count: 1 });
    def.kit = vec![crate::scenario::KitItem { name: "cash_stack".to_string(), count: 1 }];
    let err = check_banlist_covers_all_text_surfaces(&[("bad-trial", def)])
        .expect_err("a money word in a kit item name must be rejected");
    assert!(err.contains("\"cash\""));
}

// ─────────────────────────────────────────────────────────────────────────
// Check 4 — the arena actually furnishes the objective's targets
// ─────────────────────────────────────────────────────────────────────────

/// Trials whose objective reachability can't be decided by the static rules
/// below — each entry hand-verified against the real def. Keep this SHORT:
/// if a trial passes `check_arena_provides_objective_targets` on its own
/// merits it must NOT be listed here (see
/// `reachability_allowlist_entries_are_actually_needed`, which asserts every
/// entry would actually fail without the bypass).
const REACHABILITY_MANUALLY_VERIFIED: &[(&str, &str)] = &[
    (
        "obsidian",
        "the target block is FORMED by a world reaction (kit lava bucket + water bucket → \
         obsidian), never authored directly in arena.blocks, then broken — no static block-count \
         rule can see that transformation.",
    ),
    (
        "rail-rider",
        "RideEntity is satisfied by riding the CART the kit provides (after laying the arena's \
         rail blocks), not an arena-authored mob — the arena has no `mobs` list at all.",
    ),
    (
        "mill-race",
        "the Water Wheel to place comes from the KIT, never from arena.blocks (the arena \
         authors the spring and the pillar it falls off) — and the static block count \
         deliberately ignores kits.",
    ),
    (
        "catch-the-wind",
        "the Windmill to place comes from the KIT, never from arena.blocks (the arena authors \
         the hilltop platform it stands on) — and the static block count deliberately ignores \
         kits.",
    ),
];

/// Every mob a `ScenarioDef`'s arena authors, expanded to one `MobType` per
/// unit of `count` (an unresolvable name — already caught by
/// `every_challenge_kit_and_arena_name_resolves` — simply contributes none).
fn arena_mob_species(arena: Option<&ArenaSetup>) -> Vec<MobType> {
    let mut out = Vec::new();
    if let Some(a) = arena {
        for m in &a.mobs {
            if let Some(kind) = MobType::from_name(&m.mob) {
                for _ in 0..m.count {
                    out.push(kind);
                }
            }
        }
    }
    out
}

fn arena_mob_total_count(arena: Option<&ArenaSetup>) -> u32 {
    arena.map(|a| a.mobs.iter().map(|m| m.count as u32).sum()).unwrap_or(0)
}

/// How many arena mobs satisfy `pred` (summed over each entry's `count`).
/// Unresolvable names contribute nothing (already flagged elsewhere).
fn arena_mob_count_where(arena: Option<&ArenaSetup>, pred: impl Fn(MobType) -> bool) -> u32 {
    arena
        .map(|a| {
            a.mobs
                .iter()
                .filter(|m| MobType::from_name(&m.mob).is_some_and(&pred))
                .map(|m| m.count as u32)
                .sum()
        })
        .unwrap_or(0)
}

/// Whether `kind` is tameable via a real gameplay tame path — the companion
/// species (`companion::tame_food` returns their food) or the Wolf (its
/// `WolfData` bone-taming path; only Wolf mobs carry `WolfData`). Mirrors the
/// two `ChallengeEvent::TameMob` fire sites in game_loop.rs.
fn is_tameable_species(kind: MobType) -> bool {
    crate::companion::tame_food(kind).is_some() || kind == MobType::Wolf
}

/// Whether `kind` can be sheared or milked — the two `ShearOrMilk` fire paths
/// (shear a Sheep, milk a Cow).
fn is_shear_or_milk_species(kind: MobType) -> bool {
    matches!(kind, MobType::Sheep | MobType::Cow)
}

/// How many `arena.blocks` entries resolve (via `/give`'s item registry) to
/// block id `id`.
fn arena_block_count(arena: Option<&ArenaSetup>, id: BlockId) -> u32 {
    arena
        .map(|a| {
            a.blocks
                .iter()
                .filter(|b| resolve_item(&b.block, 1).ok().and_then(|s| s.item.as_block()) == Some(id))
                .count() as u32
        })
        .unwrap_or(0)
}

/// The mature (harvestable) stage of every crop family — see `crop_growth.rs`.
const MATURE_CROP_BLOCK_IDS: &[BlockId] = &[
    block::WHEAT_STAGE_3,
    block::CARROT_STAGE_3,
    block::POTATO_STAGE_3,
    block::CORN_STAGE_3,
    block::PAPYRUS_STAGE_3,
    block::SUGAR_BEET_STAGE_3,
    block::BEETROOT_STAGE_3,
    block::PUMPKIN_STEM_4,
    block::BERRY_BUSH_3,
];

fn arena_mature_crop_count(arena: Option<&ArenaSetup>) -> u32 {
    arena
        .map(|a| {
            a.blocks
                .iter()
                .filter(|b| {
                    resolve_item(&b.block, 1)
                        .ok()
                        .and_then(|s| s.item.as_block())
                        .is_some_and(|id| MATURE_CROP_BLOCK_IDS.contains(&id))
                })
                .count() as u32
        })
        .unwrap_or(0)
}

/// Is there a pairable duo in `species` that can breed `target` (`None` = any
/// offspring)? Encodes the same cross-rule as `breeding::pair_allowed` /
/// `offspring_kind` (same species always pairs; Horse×Donkey is the one cross).
fn breed_pair_available(species: &[MobType], target: Option<MobType>) -> bool {
    for i in 0..species.len() {
        for &b in &species[i + 1..] {
            let a = species[i];
            if pair_allowed(a, b) {
                match target {
                    None => return true,
                    Some(t) => {
                        if offspring_kind(a, b) == t {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Reachability of a single objective leaf against its def's arena — `Ok`
/// when the event isn't one of the "names a concrete arena resource" kinds
/// this rule set understands (out of scope, not a claim either way).
fn leaf_reachability(token: &str, def: &ScenarioDef, event: &ChallengeEvent, count: u32) -> Result<(), String> {
    match event {
        // `KillMob` is genuinely unfiltered — ANY mob death counts, so a raw
        // headcount is the right rule.
        ChallengeEvent::KillMob => {
            let have = arena_mob_total_count(def.arena.as_ref());
            if have < count {
                return Err(format!(
                    "{token}: objective needs {count} x KillMob, but arena.mobs only totals {have} \
                     mob(s) — mobs are the completion fuel for this event. Either author at least \
                     {count} in arena.mobs, or add \"{token}\" to REACHABILITY_MANUALLY_VERIFIED with \
                     a reason if completion genuinely comes from outside the arena."
                ));
            }
        }
        // The next three events are SPECIES-GATED in real gameplay: only certain
        // kinds ever fire them, so a raw headcount would wrongly pass an arena
        // stocked with the wrong species (e.g. a "tame" trial full of cows).
        // Count only the compatible species, using the REAL gameplay predicates.
        ChallengeEvent::TameMob => {
            let have = arena_mob_count_where(def.arena.as_ref(), is_tameable_species);
            if have < count {
                return Err(format!(
                    "{token}: TameMob needs {count} tameable mob(s), but arena.mobs provides only \
                     {have} of a tameable species (cat/parrot/fox via companion food, or wolf via \
                     bones) — total mob count is irrelevant if none of them can actually be tamed. \
                     Stock the arena with a tameable species (or add \"{token}\" to \
                     REACHABILITY_MANUALLY_VERIFIED if the tame target comes from outside the arena)."
                ));
            }
        }
        ChallengeEvent::ShearOrMilk => {
            let have = arena_mob_count_where(def.arena.as_ref(), is_shear_or_milk_species);
            if have < count {
                return Err(format!(
                    "{token}: ShearOrMilk needs {count} shearable/milkable mob(s), but arena.mobs \
                     provides only {have} sheep/cow — no other species can be sheared or milked. \
                     Stock at least {count} sheep and/or cows."
                ));
            }
        }
        ChallengeEvent::RideEntity => {
            let have = arena_mob_count_where(def.arena.as_ref(), crate::mob::is_rideable);
            if have < count {
                return Err(format!(
                    "{token}: RideEntity needs {count} rideable mob(s), but arena.mobs provides only \
                     {have} of a rideable species (horse/donkey/mule/nostrich per mob::is_rideable). \
                     Stock a rideable mount, or add \"{token}\" to REACHABILITY_MANUALLY_VERIFIED if \
                     the ride target is a kit-provided cart rather than an arena mob (e.g. rail-rider)."
                ));
            }
        }
        ChallengeEvent::BreakBlock { block: Some(id) } | ChallengeEvent::PlaceBlock { block: Some(id) } => {
            let have = arena_block_count(def.arena.as_ref(), *id);
            if have < count {
                return Err(format!(
                    "{token}: objective needs {count} x block id {id}, but arena.blocks only \
                     authors {have} matching block(s). Either author at least {count}, or add \
                     \"{token}\" to REACHABILITY_MANUALLY_VERIFIED (e.g. a block formed by a world \
                     reaction rather than placed directly, like obsidian)."
                ));
            }
        }
        ChallengeEvent::HarvestCrop => {
            let have = arena_mature_crop_count(def.arena.as_ref());
            if have < count {
                return Err(format!(
                    "{token}: HarvestCrop needs {count} mature crop block(s), but arena.blocks only \
                     authors {have} — author at least {count} mature-stage crop blocks (e.g. \
                     \"wheat_mature\")."
                ));
            }
        }
        ChallengeEvent::BreedAnimals { offspring } => {
            let species = arena_mob_species(def.arena.as_ref());
            if !breed_pair_available(&species, *offspring) {
                return Err(format!(
                    "{token}: BreedAnimals {{ offspring: {offspring:?} }} needs a breedable parent \
                     pair in arena.mobs (two of the same species, or Horse+Donkey for a Mule) — \
                     none found."
                ));
            }
        }
        // BreakBlock{None}/PlaceBlock{None}/CraftItem/EatFood/etc. don't name a
        // concrete arena resource this rule set can verify — out of scope.
        _ => {}
    }
    Ok(())
}

fn check_arena_provides_objective_targets(defs: &[(&str, ScenarioDef)]) -> Result<(), String> {
    for (token, def) in defs {
        if REACHABILITY_MANUALLY_VERIFIED.iter().any(|(t, _)| t == token) {
            continue;
        }
        for (event, count) in collect_leaf_events(&def.objective) {
            leaf_reachability(token, def, event, count)?;
        }
    }
    Ok(())
}

#[test]
fn arena_provides_objective_targets() {
    check_arena_provides_objective_targets(&all_bundled_defs()).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn reachability_allowlist_entries_are_actually_needed() {
    // Prove the allowlist isn't a rubber stamp: every entry must actually fail
    // the static check on its own merits (else it doesn't need the bypass).
    let defs = all_bundled_defs();
    for (token, reason) in REACHABILITY_MANUALLY_VERIFIED {
        let (_, def) = defs
            .iter()
            .find(|(t, _)| t == token)
            .unwrap_or_else(|| panic!("REACHABILITY_MANUALLY_VERIFIED references unknown trial {token}"));
        let genuinely_fails = collect_leaf_events(&def.objective)
            .into_iter()
            .any(|(event, count)| leaf_reachability(token, def, event, count).is_err());
        assert!(
            genuinely_fails,
            "{token} passes check_arena_provides_objective_targets WITHOUT the allowlist bypass \
             (reason on file: {reason:?}) — remove it from REACHABILITY_MANUALLY_VERIFIED, the \
             harness plan requires the allowlist stay minimal.",
        );
    }
}

#[test]
fn check_arena_provides_bites_on_an_undersupplied_mob_count() {
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::KillMob, count: 5 });
    def.arena = Some(ArenaSetup {
        blocks: vec![],
        mobs: vec![crate::scenario::ArenaMob { at: [1, 0, 0], mob: "brigand".to_string(), count: 1 }],
    });
    let err = check_arena_provides_objective_targets(&[("bad-trial", def)])
        .expect_err("an arena with fewer mobs than the objective count must be rejected");
    assert!(err.contains("bad-trial"));
    assert!(err.contains("needs 5"));
}

#[test]
fn check_arena_provides_bites_on_an_unbreedable_pair() {
    // Cow + Pig can never pair (breeding::pair_allowed is same-species-only,
    // plus the one Horse×Donkey cross) — a "breed a Mule" trial stocked with
    // Cow + Pig can never complete.
    let mut def = fixture_def(Objective::Action {
        event: ChallengeEvent::BreedAnimals { offspring: Some(MobType::Mule) },
        count: 1,
    });
    def.arena = Some(ArenaSetup {
        blocks: vec![],
        mobs: vec![
            crate::scenario::ArenaMob { at: [1, 0, 0], mob: "cow".to_string(), count: 1 },
            crate::scenario::ArenaMob { at: [2, 0, 0], mob: "pig".to_string(), count: 1 },
        ],
    });
    let err = check_arena_provides_objective_targets(&[("bad-trial", def)])
        .expect_err("Cow+Pig can never breed a Mule");
    assert!(err.contains("BreedAnimals"));
}

#[test]
fn check_arena_provides_bites_on_a_tame_trial_with_an_untameable_arena() {
    // The exact review gap: a Cow-only arena has a mob, but a cow is NEVER
    // tameable — a raw headcount would wrongly pass this unwinnable trial.
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::TameMob, count: 1 });
    def.arena = Some(ArenaSetup {
        blocks: vec![],
        mobs: vec![crate::scenario::ArenaMob { at: [1, 0, 0], mob: "cow".to_string(), count: 1 }],
    });
    let err = check_arena_provides_objective_targets(&[("bad-trial", def)])
        .expect_err("a TameMob objective over a cow-only arena can never complete");
    assert!(err.contains("bad-trial"));
    assert!(err.contains("tameable"));
}

#[test]
fn check_arena_provides_bites_on_a_ride_trial_with_a_non_rideable_arena() {
    // A pig is a mob but not rideable — RideEntity can never fire on it.
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::RideEntity, count: 1 });
    def.arena = Some(ArenaSetup {
        blocks: vec![],
        mobs: vec![crate::scenario::ArenaMob { at: [1, 0, 0], mob: "pig".to_string(), count: 1 }],
    });
    let err = check_arena_provides_objective_targets(&[("bad-trial", def)])
        .expect_err("a RideEntity objective over a non-rideable arena can never complete");
    assert!(err.contains("rideable"));
}

#[test]
fn check_arena_provides_bites_on_a_shear_trial_with_the_wrong_species() {
    // A horse can't be sheared or milked — only sheep/cow fire ShearOrMilk.
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::ShearOrMilk, count: 1 });
    def.arena = Some(ArenaSetup {
        blocks: vec![],
        mobs: vec![crate::scenario::ArenaMob { at: [1, 0, 0], mob: "horse".to_string(), count: 1 }],
    });
    let err = check_arena_provides_objective_targets(&[("bad-trial", def)])
        .expect_err("a ShearOrMilk objective over a horse-only arena can never complete");
    assert!(err.contains("shearable"));
}

#[test]
fn check_arena_provides_bites_on_a_block_target_the_arena_never_authors() {
    // Mirrors the real obsidian shape MINUS the allowlist entry — proves the
    // check itself would flag it if it weren't manually verified.
    let mut def = fixture_def(Objective::Action {
        event: ChallengeEvent::BreakBlock { block: Some(block::OBSIDIAN) },
        count: 1,
    });
    def.arena = Some(ArenaSetup {
        blocks: vec![crate::scenario::ArenaBlock { at: [1, 0, 0], block: "cobblestone".to_string() }],
        mobs: vec![],
    });
    let err = check_arena_provides_objective_targets(&[("bad-trial", def)])
        .expect_err("an arena with no matching block must be rejected");
    assert!(err.contains("bad-trial"));
}

// ─────────────────────────────────────────────────────────────────────────
// Check 5 — an unfiltered mob event over a multi-species arena is a
// conscious choice, not an authoring accident
// ─────────────────────────────────────────────────────────────────────────

/// `KillMob`/`TameMob`/`ShearOrMilk`/`RideEntity` carry no per-species filter
/// (v1 taxonomy — see `ChallengeEvent`'s doc comment): ANY matching action
/// anywhere satisfies them. That's invisible when the arena only stocks one
/// species (nothing to be ambiguous about) but becomes a real "which mob did
/// they mean?" question once it stocks more than one — so THAT'S what this
/// check flags. `BreedAnimals` has its own explicit `offspring` filter and is
/// verified structurally by check 4 instead, so it's excluded here.
const UNFILTERED_EVENT_INTENTIONAL: &[(&str, &str)] = &[
    (
        "friend-in-need",
        "any of cat/parrot/fox is fine — Satoshi explicitly says \"your pick\" (arena stocks all 3).",
    ),
    (
        "farmhand",
        "shear OR milk, either counts toward the 2 chores by design (arena stocks a sheep + a cow).",
    ),
    (
        "funny-farm",
        "each farm chore (tame/ride/shear-or-milk) is satisfied by whichever eligible animal from \
         the mixed farm pen is present — not a single specific target.",
    ),
];

fn is_mob_ambiguity_event(e: &ChallengeEvent) -> bool {
    matches!(
        e,
        ChallengeEvent::KillMob | ChallengeEvent::TameMob | ChallengeEvent::ShearOrMilk | ChallengeEvent::RideEntity
    )
}

/// Does `token`'s objective use one of the unfiltered mob events over an
/// arena that stocks more than one distinct species? `None` if the check
/// doesn't apply (no unfiltered mob event in this objective at all).
fn has_ambiguous_unfiltered_mob_event(def: &ScenarioDef) -> bool {
    let uses_ambiguity_prone_event =
        collect_leaf_events(&def.objective).iter().any(|(e, _)| is_mob_ambiguity_event(e));
    if !uses_ambiguity_prone_event {
        return false;
    }
    let distinct: HashSet<MobType> = arena_mob_species(def.arena.as_ref()).into_iter().collect();
    distinct.len() > 1
}

fn check_unfiltered_event_trials_are_intentional(defs: &[(&str, ScenarioDef)]) -> Result<(), String> {
    for (token, def) in defs {
        if !has_ambiguous_unfiltered_mob_event(def) {
            continue;
        }
        if !UNFILTERED_EVENT_INTENTIONAL.iter().any(|(t, _)| t == token) {
            return Err(format!(
                "{token}: uses an unfiltered mob event (Kill/Tame/ShearOrMilk/Ride — no per-species \
                 filter in v1) over an arena that stocks MORE THAN ONE distinct mob species, so any \
                 of them satisfies the objective. Add (\"{token}\", \"<why any-target is fine here>\") \
                 to UNFILTERED_EVENT_INTENTIONAL to consciously confirm that's intended, rather than \
                 an authoring slip where one specific species was meant."
            ));
        }
    }
    Ok(())
}

#[test]
fn unfiltered_event_trials_are_intentional() {
    check_unfiltered_event_trials_are_intentional(&all_bundled_defs()).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn unfiltered_allowlist_entries_are_actually_needed() {
    let defs = all_bundled_defs();
    for (token, reason) in UNFILTERED_EVENT_INTENTIONAL {
        let (_, def) = defs
            .iter()
            .find(|(t, _)| t == token)
            .unwrap_or_else(|| panic!("UNFILTERED_EVENT_INTENTIONAL references unknown trial {token}"));
        assert!(
            has_ambiguous_unfiltered_mob_event(def),
            "{token} doesn't actually have an ambiguous unfiltered mob event (reason on file: \
             {reason:?}) — remove it from UNFILTERED_EVENT_INTENTIONAL, the harness plan requires \
             the allowlist stay minimal.",
        );
    }
}

#[test]
fn check_unfiltered_bites_on_an_unacknowledged_multi_species_arena() {
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::TameMob, count: 1 });
    def.arena = Some(ArenaSetup {
        blocks: vec![],
        mobs: vec![
            crate::scenario::ArenaMob { at: [1, 0, 0], mob: "cow".to_string(), count: 1 },
            crate::scenario::ArenaMob { at: [2, 0, 0], mob: "pig".to_string(), count: 1 },
        ],
    });
    let err = check_unfiltered_event_trials_are_intentional(&[("bad-trial", def)])
        .expect_err("an unacknowledged multi-species arena over TameMob must be rejected");
    assert!(err.contains("bad-trial"));
}

#[test]
fn check_unfiltered_does_not_flag_a_single_species_arena() {
    // A control: the same event over a SINGLE species must pass — there's
    // nothing ambiguous about "tame the only tameable thing here".
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::TameMob, count: 1 });
    def.arena = Some(ArenaSetup {
        blocks: vec![],
        mobs: vec![crate::scenario::ArenaMob { at: [1, 0, 0], mob: "wolf".to_string(), count: 1 }],
    });
    check_unfiltered_event_trials_are_intentional(&[("fine-trial", def)])
        .expect("a single-species arena is never ambiguous");
}

// ─────────────────────────────────────────────────────────────────────────
// Unrecognised-field check (test-time strict, over bundled trials only)
// ─────────────────────────────────────────────────────────────────────────
//
// Runtime `load_scenario_def` is deliberately TOLERANT of unknown fields —
// it must be, because `ScenarioDef` is (a) persisted to `WorldMeta.scenario_def`
// and re-parsed on load for resumable Satori-Rush runs and (b) deserialised
// from community-published open-stash mods authored against other client
// versions. Adding `#[serde(deny_unknown_fields)]` there would make a stale
// save or a forward-published mod fail to parse — a forward-compat regression
// against this project's tolerant-old-decode philosophy.
//
// So the strictness lives HERE, at CI/authoring time, over ONLY the bundled
// trials the engine ships. The #1 silent-typo failure mode is a misspelled
// field name ("objetive", "arena_seeed", "displayName") that serde silently
// drops and defaults — producing a subtly-wrong trial that still parses and
// still passes every other test. We catch it WITHOUT `deny_unknown_fields`:
// parse each bundled JSON to a `serde_json::Value` (the author's LITERAL keys)
// AND round-trip it through `ScenarioDef` (lenient parse → reserialise, giving
// the RECOGNISED keys), then assert every object key the author wrote survives
// the round-trip. A typo'd key is dropped by the lenient parse, so it's absent
// from the reserialised output → flagged, naming the trial and the exact key
// path. Recurses into nested objects and array elements (kit[], arena.mobs[],
// arena.blocks[], objective steps) so a typo at any depth is caught. Keys
// present in the OUTPUT but absent from the INPUT are fine (those are serde
// defaults being filled in) — only an input key missing from the output errs.
//
// MAINTAINER CAUTION: this rests on every `ScenarioDef` field serialising back
// under the exact key it was authored with. A future field carrying
// `#[serde(skip_serializing_if = ...)]`, `#[serde(alias = ...)]`, or
// `#[serde(skip)]` would drop a legitimately-present input key from the
// round-tripped output and thus FALSE-FAIL a valid bundled trial. If you add
// such an attribute, teach this check about it (e.g. skip that key path).

/// Every bundled scenario JSON as `(token, raw bytes)` — the four special
/// scenarios the engine ships plus every `EXPLORER_CHALLENGES` entry. This is
/// the corpus the unrecognised-field check runs over (raw bytes, because the
/// check compares the author's literal JSON keys against the round-tripped
/// recognised keys — a parsed `ScenarioDef` has already lost the stray keys).
fn all_bundled_json_bytes() -> Vec<(&'static str, &'static [u8])> {
    let mut out: Vec<(&'static str, &'static [u8])> = vec![
        ("onboarding", crate::scenario::ONBOARDING_JSON),
        ("hash-dash", crate::scenario::HASH_DASH_JSON),
        ("satori-rush", crate::scenario::SATORI_RUSH_JSON),
    ];
    for (name, bytes) in crate::scenario::EXPLORER_CHALLENGES {
        out.push((name, bytes));
    }
    out
}

/// Recursively assert every object key present in `input` (the author's literal
/// JSON) also survives in `output` (the round-tripped, recognised JSON).
/// `path` is a dotted breadcrumb for the error message. Recurses into matching
/// nested objects and zips matching arrays so a typo at any depth is named.
fn assert_input_keys_survive(
    path: &str,
    input: &serde_json::Value,
    output: &serde_json::Value,
) -> Result<(), String> {
    match (input, output) {
        (serde_json::Value::Object(in_map), serde_json::Value::Object(out_map)) => {
            for (key, in_val) in in_map {
                let child_path =
                    if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                match out_map.get(key) {
                    None => {
                        return Err(format!(
                            "unrecognised field `{child_path}` — it is not a field of \
                             ScenarioDef (or a nested authoring struct), so serde silently \
                             DROPS it at parse time and the trial ships subtly wrong. Fix the \
                             spelling against docs/authoring/trials.md §1, or remove the field."
                        ));
                    }
                    Some(out_val) => assert_input_keys_survive(&child_path, in_val, out_val)?,
                }
            }
            Ok(())
        }
        (serde_json::Value::Array(in_arr), serde_json::Value::Array(out_arr)) => {
            // Recognised arrays (kit[], arena.mobs[], objective steps, …) round-trip
            // element-for-element in order, so zip is sound. A length mismatch would
            // only arise if the whole array key were itself unrecognised, which the
            // object arm above already catches, so a short zip can't hide a typo.
            for (i, (in_el, out_el)) in in_arr.iter().zip(out_arr.iter()).enumerate() {
                assert_input_keys_survive(&format!("{path}[{i}]"), in_el, out_el)?;
            }
            Ok(())
        }
        // Leaves (or a recognised key whose value shape differs, e.g. an enum
        // rendered as a string): the key itself survived, which is all this check
        // asserts. Nothing deeper to compare.
        _ => Ok(()),
    }
}

/// For each bundled JSON, assert every key the author literally wrote is a
/// recognised `ScenarioDef` field (survives a lenient parse → reserialise
/// round-trip). See the section comment above for why this lives here and not
/// as `#[serde(deny_unknown_fields)]` on the runtime type.
fn check_no_unrecognised_fields(corpus: &[(&str, &[u8])]) -> Result<(), String> {
    for (token, bytes) in corpus {
        let authored: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|e| format!("{token}: bundled JSON is not valid JSON: {e}"))?;
        let def = crate::scenario::load_scenario_def(bytes)
            .map_err(|e| format!("{token}: bundled JSON does not parse as ScenarioDef: {e}"))?;
        let recognised: serde_json::Value = serde_json::from_str(
            &def.to_json().map_err(|e| format!("{token}: reserialise failed: {e}"))?,
        )
        .map_err(|e| format!("{token}: reserialised JSON is not valid JSON: {e}"))?;
        assert_input_keys_survive("", &authored, &recognised)
            .map_err(|e| format!("{token}: {e}"))?;
    }
    Ok(())
}

#[test]
fn every_bundled_field_is_recognised() {
    check_no_unrecognised_fields(&all_bundled_json_bytes()).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn check_no_unrecognised_fields_bites_on_a_bogus_key() {
    // A bundled-style JSON with everything valid EXCEPT one typo'd top-level
    // key (`bogus_field`) — the lenient runtime parse drops it, so the strict
    // lint must flag it (naming the trial + the key). This is the guarantee the
    // runtime type deliberately can't give (it must stay forward-compatible).
    let json = br#"{
        "kind":"Challenge",
        "display_name":"X - y",
        "objective":{"Action":{"event":{"BreakBlock":{"block":null}},"count":1}},
        "bogus_field":1
    }"#;
    let err = check_no_unrecognised_fields(&[("bad-trial", json)])
        .expect_err("a bogus top-level key must be flagged by the strict lint");
    assert!(err.contains("bad-trial"), "error must name the trial: {err}");
    assert!(err.contains("bogus_field"), "error must name the bad key: {err}");
}

#[test]
fn check_no_unrecognised_fields_bites_on_a_nested_typo() {
    // A typo of an OPTIONAL field inside a nested array element (arena.mobs[].conut
    // instead of .count) — this is the true silent-drop case: `count` defaults, so
    // the def parses "successfully" and the stray `conut` vanishes. Proves the
    // recursion reaches array elements. (A typo of a REQUIRED nested field, e.g.
    // arena.blocks[].blok, is instead caught as a hard missing-field parse error,
    // which check_no_unrecognised_fields also surfaces — just via the parse step.)
    let json = br#"{
        "kind":"Challenge",
        "display_name":"X - y",
        "objective":{"Action":{"event":"TameMob","count":1}},
        "arena":{"mobs":[{"at":[1,0,0],"mob":"wolf","conut":3}]}
    }"#;
    let err = check_no_unrecognised_fields(&[("bad-trial", json)])
        .expect_err("a typo'd optional key inside arena.mobs[] must be flagged");
    assert!(err.contains("conut"), "error must name the nested bad key: {err}");
    assert!(err.contains("arena.mobs[0]"), "error must give the key path: {err}");
}

// ─────────────────────────────────────────────────────────────────────────
// Check 7 — a weather_lock must name a window the engine actually honours
// ─────────────────────────────────────────────────────────────────────────

/// `ScenarioDef.weather_lock` is a free-form string (the def format stays
/// tolerant of values a future build might add), so at RUNTIME an unrecognised
/// value is simply ignored and the world rolls its own weather. For a BUNDLED
/// trial that is a silent failure: `"stormy"` would leave a windmill trial
/// waiting on a breeze that may never come. Pin it here instead — the valid set
/// is `weather::WEATHER_LOCKS`, and each entry is proved to pin a real window by
/// `weather::tests::every_advertised_lock_value_actually_pins_a_window`.
fn check_weather_lock_values(defs: &[(&str, ScenarioDef)]) -> Result<(), String> {
    for (token, def) in defs {
        let Some(lock) = def.weather_lock.as_deref() else {
            continue;
        };
        if crate::weather::locked_window(Some(lock), 0).is_none() {
            return Err(format!(
                "{token}: weather_lock {lock:?} is not a window the engine honours — valid values \
                 are {:?}. An unrecognised value is IGNORED at runtime, so the trial would quietly \
                 run under normal rolling weather (a storm-locked windmill trial that never gets \
                 its storm). Fix the spelling, or teach `weather::locked_window` the new value.",
                crate::weather::WEATHER_LOCKS,
            ));
        }
    }
    Ok(())
}

#[test]
fn every_bundled_weather_lock_is_a_value_the_engine_honours() {
    check_weather_lock_values(&all_bundled_defs()).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn check_weather_lock_values_bites_on_a_typo() {
    let mut def = fixture_def(Objective::Action { event: ChallengeEvent::CraftItem, count: 1 });
    def.weather_lock = Some("stormy".to_string());
    let err = check_weather_lock_values(&[("bad-trial", def)])
        .expect_err("an unrecognised weather_lock must be rejected");
    assert!(err.contains("bad-trial"), "error must name the trial: {err}");
    assert!(err.contains("stormy"), "error must name the bad value: {err}");
}

#[test]
fn check_weather_lock_values_accepts_every_advertised_lock() {
    for name in crate::weather::WEATHER_LOCKS {
        let mut def = fixture_def(Objective::Action { event: ChallengeEvent::CraftItem, count: 1 });
        def.weather_lock = Some((*name).to_string());
        check_weather_lock_values(&[("ok-trial", def)])
            .unwrap_or_else(|e| panic!("{name} is advertised as valid but the lint rejects it: {e}"));
    }
}
