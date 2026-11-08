//! Unlock by doing (Trials in Two Acts §A.5): which trials the gallery shows,
//! which carry a quiet "new" mark, and the nudges toward the next ones.
//!
//! A bundled trial with no `unlock` is always visible (the front shelf, and
//! Scavenger in the clock corner). A trial with an `unlock` appears when
//! EITHER clause holds:
//! - `on`: ANY of its events has happened, anywhere (the player's own world or
//!   any arena) — recorded by `GameState::fire_challenge` into
//!   `TrialBests::events_seen`;
//! - `after`: ALL of its listed trials (by token) are completed.
//!
//! A completed trial is always visible, and once a trial has appeared it stays
//! (`TrialBests::unlocked`, seeded from existing completions on load). Every
//! stored set only grows, so stores merge as plain unions.
//!
//! Pure: the public functions read the bundled catalogue (built once); the
//! `*_in` forms take any catalogue so tests can use fixtures.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::scenario::ChallengeEvent;
use crate::trials::{TrialBests, TrialRef, TrialType, TRIAL_ORDER};

/// When a trial appears — the optional `unlock` field of a `ScenarioDef`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialUnlock {
    /// Any of these events, the first time it happens anywhere.
    #[serde(default)]
    pub on: Vec<ChallengeEvent>,
    /// All of these trials (tokens) completed.
    #[serde(default)]
    pub after: Vec<String>,
    /// A one-line nudge shown while the trial is still hidden
    /// ("Something about horses…"). UK English, no money words (lint).
    #[serde(default)]
    pub hint: String,
}

/// One bundled challenge, as the unlock rules see it.
#[derive(Clone, Debug)]
pub struct TrialEntry {
    pub token: &'static str,
    /// The key completion is stored under (`TrialBests::completed_challenges`).
    pub display_name: String,
    pub shelf: TrialType,
    pub unlock: Option<TrialUnlock>,
    /// `AnyOf` route names, in authored order (empty for a one-way trial).
    pub routes: Vec<String>,
}

/// Every challenge in `TRIAL_ORDER`, parsed once.
static CATALOGUE: LazyLock<Vec<TrialEntry>> = LazyLock::new(|| {
    TRIAL_ORDER
        .iter()
        .filter_map(|(shelf, tref)| {
            let TrialRef::Challenge(token) = tref else { return None };
            let def = crate::scenario::named_builtin_def(token)?;
            let routes = match &def.objective {
                crate::scenario::Objective::AnyOf { routes } => routes.iter().map(|r| r.name.clone()).collect(),
                _ => Vec::new(),
            };
            Some(TrialEntry { token, display_name: def.display_name, shelf: *shelf, unlock: def.unlock.map(|u| *u), routes })
        })
        .collect()
});

/// The bundled catalogue.
pub fn catalogue() -> &'static [TrialEntry] {
    &CATALOGUE
}

fn entry<'a>(cat: &'a [TrialEntry], token: &str) -> Option<&'a TrialEntry> {
    cat.iter().find(|e| e.token == token)
}

fn done(cat: &[TrialEntry], token: &str, bests: &TrialBests) -> bool {
    entry(cat, token).is_some_and(|e| bests.is_challenge_done(&e.display_name))
}

/// The stable key an event is remembered under in `TrialBests::events_seen`.
pub fn event_key(ev: &ChallengeEvent) -> String {
    serde_json::to_string(ev).unwrap_or_default()
}

/// Does `unlock` hold right now (either clause), ignoring what is stored?
fn rule_met(cat: &[TrialEntry], unlock: &TrialUnlock, bests: &TrialBests) -> bool {
    let after = !unlock.after.is_empty() && unlock.after.iter().all(|t| done(cat, t, bests));
    let on = !unlock.on.is_empty()
        && bests.events_seen.iter().any(|key| {
            serde_json::from_str::<ChallengeEvent>(key)
                .is_ok_and(|seen| unlock.on.iter().any(|p| p.matches(&seen)))
        });
    after || on
}

/// Whether a trial is shown. Anything not in the catalogue (the races) and
/// any trial with no `unlock` is always shown.
pub fn is_unlocked_in(cat: &[TrialEntry], token: &str, bests: &TrialBests) -> bool {
    let Some(e) = entry(cat, token) else { return true };
    let Some(unlock) = &e.unlock else { return true };
    bests.is_challenge_done(&e.display_name)
        || bests.unlocked.contains(token)
        || rule_met(cat, unlock, bests)
}

/// Whether the gallery shows this trial (token, or race id).
pub fn is_unlocked(token: &str, bests: &TrialBests) -> bool {
    is_unlocked_in(catalogue(), token, bests)
}

/// Record a fired event for unlocking. Cheap when nothing listens for it (a
/// scan of the few `on` patterns, no allocation). Returns `true` when the
/// store changed, so the caller saves once.
pub fn record_event_in(cat: &[TrialEntry], bests: &mut TrialBests, ev: &ChallengeEvent) -> bool {
    let listening = |e: &&TrialEntry| e.unlock.as_ref().is_some_and(|u| u.on.iter().any(|p| p.matches(ev)));
    if !cat.iter().any(|e| listening(&e)) {
        return false;
    }
    let mut changed = bests.events_seen.insert(event_key(ev));
    for e in cat.iter().filter(listening) {
        changed |= bests.unlocked.insert(e.token.to_string());
    }
    changed
}

/// Store every trial whose unlock now holds (or that is already completed:
/// the load-time seed, so nobody loses a visible trial). `true` if any were
/// added.
pub fn refresh_in(cat: &[TrialEntry], bests: &mut TrialBests) -> bool {
    let newly: Vec<&'static str> = cat
        .iter()
        .filter(|e| e.unlock.is_some() && !bests.unlocked.contains(e.token))
        .filter(|e| {
            bests.is_challenge_done(&e.display_name)
                || e.unlock.as_ref().is_some_and(|u| rule_met(cat, u, bests))
        })
        .map(|e| e.token)
        .collect();
    for t in &newly {
        bests.unlocked.insert(t.to_string());
    }
    !newly.is_empty()
}

/// Hidden trials worth a nudge, `(token, hint)`, in `TRIAL_ORDER` order: a
/// locked trial with at least one `after` prerequisite completed, or an
/// `on`-only trial whose shelf already has a completed trial. A locked trial
/// with an `on` alternative that is the only trial on its shelf (the Grey
/// Quarry) is nudged from the start: nothing else on its shelf could ever be
/// done to start it, and a single `after` is either not done or unlocks it,
/// so neither rule above could ever show its hint.
pub fn next_hints_in(cat: &[TrialEntry], bests: &TrialBests) -> Vec<(&'static str, String)> {
    let shelf_started = |shelf: TrialType| {
        cat.iter().any(|e| e.shelf == shelf && bests.is_challenge_done(&e.display_name))
    };
    let sole_on_shelf = |e: &TrialEntry| cat.iter().filter(|o| o.shelf == e.shelf).count() == 1;
    cat.iter()
        .filter(|e| !is_unlocked_in(cat, e.token, bests))
        .filter_map(|e| {
            let u = e.unlock.as_ref()?;
            let near = if u.after.is_empty() {
                !u.on.is_empty() && (shelf_started(e.shelf) || sole_on_shelf(e))
            } else {
                u.after.iter().any(|t| done(cat, t, bests)) || (!u.on.is_empty() && sole_on_shelf(e))
            };
            near.then(|| (e.token, u.hint.clone()))
        })
        .collect()
}

/// See [`next_hints_in`].
pub fn next_hints(bests: &TrialBests) -> Vec<(&'static str, String)> {
    next_hints_in(catalogue(), bests)
}

/// Whether a trial wears the quiet "new" mark: it has appeared by doing, is
/// not yet completed, and its card has not been opened since
/// (`TrialBests::mark_card_opened` clears it).
pub fn is_new_in(cat: &[TrialEntry], token: &str, bests: &TrialBests) -> bool {
    entry(cat, token).is_some_and(|e| {
        e.unlock.is_some()
            && !bests.is_challenge_done(&e.display_name)
            && !bests.cards_opened.contains(token)
            && is_unlocked_in(cat, token, bests)
    })
}

/// See [`is_new_in`].
pub fn is_new(token: &str, bests: &TrialBests) -> bool {
    is_new_in(catalogue(), token, bests)
}

/// Every trial currently wearing the "new" mark, in `TRIAL_ORDER` order.
/// (The gallery reads `is_new` per card; this is the whole list, for tests and tools.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn newly_unlocked(bests: &TrialBests) -> Vec<&'static str> {
    catalogue().iter().filter(|e| is_new(e.token, bests)).map(|e| e.token).collect()
}

/// A trial's `AnyOf` route names, in authored order (empty for a one-way
/// trial or an unknown token). Pair with `TrialBests::ways_done` for the
/// "one mark per way done" display.
pub fn route_names(token: &str) -> &'static [String] {
    entry(catalogue(), token).map(|e| e.routes.as_slice()).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial(token: &'static str, shelf: TrialType, unlock: Option<TrialUnlock>) -> TrialEntry {
        TrialEntry { token, display_name: format!("{token} \u{2014} premise"), shelf, unlock, routes: Vec::new() }
    }

    fn after(tokens: &[&str], hint: &str) -> Option<TrialUnlock> {
        Some(TrialUnlock { on: vec![], after: tokens.iter().map(|t| t.to_string()).collect(), hint: hint.into() })
    }

    fn on(ev: ChallengeEvent, hint: &str) -> Option<TrialUnlock> {
        Some(TrialUnlock { on: vec![ev], after: vec![], hint: hint.into() })
    }

    fn fixture() -> Vec<TrialEntry> {
        vec![
            trial("forge", TrialType::Make, None),
            trial("raiders", TrialType::Brave, None),
            trial("tame", TrialType::Tend, None),
            trial("suit", TrialType::Brave, after(&["forge", "raiders"], "Something about iron armour\u{2026}")),
            trial("angler", TrialType::Tend, on(ChallengeEvent::CatchFish, "Something about a quiet pond\u{2026}")),
            trial(
                "breed",
                TrialType::Tend,
                Some(TrialUnlock {
                    on: vec![ChallengeEvent::BreedAnimals { offspring: None }],
                    after: vec!["tame".into()],
                    hint: "Something about baby animals\u{2026}".into(),
                }),
            ),
        ]
    }

    fn complete(cat: &[TrialEntry], bests: &mut TrialBests, token: &str) {
        let display = entry(cat, token).unwrap().display_name.clone();
        bests.mark_challenge_done(&display);
    }

    #[test]
    fn front_shelf_and_unknown_tokens_are_always_visible() {
        let cat = fixture();
        let bests = TrialBests::default();
        assert!(is_unlocked_in(&cat, "forge", &bests));
        assert!(is_unlocked_in(&cat, "sprint", &bests), "a race is never locked");
        assert!(!is_unlocked_in(&cat, "suit", &bests));
        assert!(!is_unlocked_in(&cat, "angler", &bests));
    }

    #[test]
    fn after_needs_every_listed_trial() {
        let cat = fixture();
        let mut bests = TrialBests::default();
        complete(&cat, &mut bests, "forge");
        assert!(!is_unlocked_in(&cat, "suit", &bests), "one of two is not enough");
        complete(&cat, &mut bests, "raiders");
        assert!(is_unlocked_in(&cat, "suit", &bests));
        assert!(refresh_in(&cat, &mut bests), "the unlock is stored");
        assert!(bests.unlocked.contains("suit"));
        assert!(!refresh_in(&cat, &mut bests), "and only once");
    }

    #[test]
    fn an_event_seen_anywhere_unlocks_and_is_remembered() {
        let cat = fixture();
        let mut bests = TrialBests::default();
        assert!(!record_event_in(&cat, &mut bests, &ChallengeEvent::KillMob), "nobody listens: no change");
        assert!(bests.events_seen.is_empty());
        assert!(record_event_in(&cat, &mut bests, &ChallengeEvent::CatchFish));
        assert!(bests.unlocked.contains("angler"));
        assert!(!record_event_in(&cat, &mut bests, &ChallengeEvent::CatchFish), "a repeat changes nothing");
        // Either clause unlocks: a breeding does it without the tame trial.
        let mule = ChallengeEvent::BreedAnimals { offspring: Some(crate::mob::MobType::Mule) };
        assert!(record_event_in(&cat, &mut bests, &mule));
        assert!(is_unlocked_in(&cat, "breed", &bests));
        // The rule also reads the stored events (a store whose unlocked set
        // was lost still shows it).
        let mut lost = bests.clone();
        lost.unlocked.clear();
        assert!(is_unlocked_in(&cat, "angler", &lost));
    }

    #[test]
    fn completions_seed_the_unlocked_set() {
        let cat = fixture();
        let mut bests = TrialBests::default();
        complete(&cat, &mut bests, "suit"); // done before unlocks existed
        assert!(is_unlocked_in(&cat, "suit", &bests));
        assert!(refresh_in(&cat, &mut bests));
        assert!(bests.unlocked.contains("suit"));
    }

    #[test]
    fn hints_point_at_the_next_trials_only() {
        let cat = fixture();
        let mut bests = TrialBests::default();
        assert!(next_hints_in(&cat, &bests).is_empty(), "nothing started, nothing nudged");
        complete(&cat, &mut bests, "forge");
        let hints = next_hints_in(&cat, &bests);
        assert_eq!(hints, vec![("suit", "Something about iron armour\u{2026}".to_string())]);
        complete(&cat, &mut bests, "tame");
        let tokens: Vec<_> = next_hints_in(&cat, &bests).into_iter().map(|(t, _)| t).collect();
        // The angler is on-only and the Tend shelf now has a completion; the
        // breeder has its `after` prerequisite met, so it is already visible.
        assert_eq!(tokens, vec!["suit", "angler"]);
        assert!(is_unlocked_in(&cat, "breed", &bests));
    }

    #[test]
    fn a_locked_on_trial_alone_on_its_shelf_is_hinted_from_the_start() {
        let quarry = trial(
            "quarry",
            TrialType::Bloom,
            Some(TrialUnlock {
                on: vec![ChallengeEvent::PlaceWentGrey],
                after: vec!["raiders".into()],
                hint: "Somewhere grey is waiting.".into(),
            }),
        );
        let mut cat = fixture();
        cat.push(quarry);
        let mut bests = TrialBests::default();
        let tokens = |b: &TrialBests| next_hints_in(&cat, b).into_iter().map(|(t, _)| t).collect::<Vec<_>>();
        assert_eq!(tokens(&bests), vec!["quarry"], "shown while locked, nothing started");
        // Other trials keep their rules: the breeder (on + after, shared shelf) stays quiet.
        complete(&cat, &mut bests, "forge");
        assert!(!tokens(&bests).contains(&"breed"));
        assert!(tokens(&bests).contains(&"quarry"), "still locked, still hinted");
        // Once the place goes grey the trial is unlocked and the nudge goes.
        record_event_in(&cat, &mut bests, &ChallengeEvent::PlaceWentGrey);
        assert!(!tokens(&bests).contains(&"quarry"));
    }

    #[test]
    fn the_bundled_grey_quarry_is_hinted_while_locked_and_other_hints_are_unchanged() {
        let none = TrialBests::default();
        let hints = next_hints(&none);
        assert!(
            hints.iter().any(|(t, h)| *t == "grey-quarry" && h == "Somewhere grey is waiting."),
            "the frosted hint shows for the locked Grey Quarry: {hints:?}"
        );
        // A fresh store hints only the Grey Quarry: no other trial's hint moved.
        assert_eq!(hints.len(), 1, "{hints:?}");
        // The mixed Rancher (on + after) keeps its old rule: quiet until it unlocks.
        assert!(!hints.iter().any(|(t, _)| *t == "rancher"));
        let mut grey = TrialBests::default();
        record_event_in(catalogue(), &mut grey, &ChallengeEvent::PlaceWentGrey);
        assert!(!next_hints(&grey).iter().any(|(t, _)| *t == "grey-quarry"), "unlocked: no longer a hint");
    }

    #[test]
    fn the_new_mark_lasts_until_the_card_is_opened_or_the_trial_done() {
        let cat = fixture();
        let mut bests = TrialBests::default();
        assert!(!is_new_in(&cat, "forge", &bests), "the front shelf is never new");
        record_event_in(&cat, &mut bests, &ChallengeEvent::CatchFish);
        assert!(is_new_in(&cat, "angler", &bests));
        assert!(bests.mark_card_opened("angler"));
        assert!(!bests.mark_card_opened("angler"), "opening again changes nothing");
        assert!(!is_new_in(&cat, "angler", &bests));
        complete(&cat, &mut bests, "suit");
        assert!(!is_new_in(&cat, "suit", &bests), "a done trial is not new");
    }

    #[test]
    fn the_bundled_catalogue_has_every_challenge_and_its_routes() {
        let cat = catalogue();
        assert!(cat.iter().any(|e| e.token == "onboarding" && e.unlock.is_none()));
        assert_eq!(route_names("nope"), &[] as &[String]);
    }

    #[test]
    fn on_and_after_are_either_or_never_both_required() {
        // The Grey Quarry names `on: [PlaceWentGrey]` AND `after: [generator]`.
        // Either one alone must show it (living world D13).
        let cat = catalogue();
        let quarry = entry(cat, "grey-quarry").expect("the Grey Quarry is in the catalogue");
        let u = quarry.unlock.as_ref().expect("it appears by doing");
        assert!(!u.on.is_empty() && !u.after.is_empty(), "both clauses are authored");
        assert_eq!(quarry.shelf, TrialType::Bloom);

        let none = TrialBests::default();
        assert!(!is_unlocked("grey-quarry", &none), "hidden until one clause holds");

        // Only the event: a place went grey, Power Plant never done.
        let mut grey = TrialBests::default();
        assert!(record_event_in(cat, &mut grey, &ChallengeEvent::PlaceWentGrey));
        assert!(is_unlocked("grey-quarry", &grey));
        assert!(!done(cat, "generator", &grey));

        // Only the trial: Power Plant done, no place ever went grey.
        let mut plant = TrialBests::default();
        let display = entry(cat, "generator").unwrap().display_name.clone();
        plant.mark_challenge_done(&display);
        assert!(plant.events_seen.is_empty());
        assert!(is_unlocked("grey-quarry", &plant));
        assert!(refresh_in(cat, &mut plant));
        assert!(plant.unlocked.contains("grey-quarry"));
    }
}
