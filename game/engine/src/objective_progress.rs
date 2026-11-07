//! Event-driven objective progress — pure functions over an [`Objective`] and
//! its flat per-leaf counter slice. `ScenarioState` holds the counters; this
//! module decides what an event advances and when the objective is done.
//!
//! Layout: one `u32` per `Action` leaf, in authored order. An `AnyOf` lays its
//! routes end to end, each route owning a contiguous slice (see
//! [`route_spans`]), so the HUD's per-leaf task lines and the authored labels
//! stay one-to-one with the counters.
//!
//! `AnyOf` rules (Trials in Two Acts §A.5): every route sees every event and
//! keeps its own progress; the trial completes the moment any route does. When
//! one event finishes several routes at once, all of them count as done (for
//! the "ways done" marks) and the first in authored order is the one the end
//! card names, so authors list routes most-specific first.

use crate::scenario::{ChallengeEvent, Objective};

/// Counter slots this objective needs: 1 per `Action` leaf (an `AnyOf` sums its
/// routes); 0 for the passive objectives (Timed / FirstSatori / FreeRoam).
pub fn leaf_count(obj: &Objective) -> usize {
    match obj {
        Objective::Action { .. } => 1,
        Objective::Sequence { steps } => steps.len(),
        Objective::Checklist { items } => items.len(),
        Objective::AnyOf { routes } => routes.iter().map(|r| leaf_count(&r.objective)).sum(),
        Objective::Timed { .. } | Objective::FirstSatori | Objective::FreeRoam => 0,
    }
}

/// `(start, len)` of each `AnyOf` route's counter slice, in authored order.
/// Empty for every other objective.
pub fn route_spans(obj: &Objective) -> Vec<(usize, usize)> {
    let Objective::AnyOf { routes } = obj else { return Vec::new() };
    let mut start = 0;
    routes
        .iter()
        .map(|r| {
            let len = leaf_count(&r.objective);
            let span = (start, len);
            start += len;
            span
        })
        .collect()
}

/// The target count of every leaf, flat (a non-`Action` leaf, which can never
/// complete, is given 1 so it shows as one unfinished line).
pub fn required(obj: &Objective) -> Vec<u32> {
    let of_leaves = |subs: &[Objective]| -> Vec<u32> {
        subs.iter().map(|s| action_count(s).unwrap_or(1)).collect()
    };
    match obj {
        Objective::Action { count, .. } => vec![*count],
        Objective::Sequence { steps } => of_leaves(steps),
        Objective::Checklist { items } => of_leaves(items),
        Objective::AnyOf { routes } => routes.iter().flat_map(|r| required(&r.objective)).collect(),
        Objective::Timed { .. } | Objective::FirstSatori | Objective::FreeRoam => Vec::new(),
    }
}

fn action_count(obj: &Objective) -> Option<u32> {
    match obj {
        Objective::Action { count, .. } => Some(*count),
        _ => None,
    }
}

fn get(progress: &[u32], i: usize) -> u32 {
    progress.get(i).copied().unwrap_or(0)
}

/// Index of the first incomplete step of an ordered `Sequence`. A non-Action
/// leaf can't complete via events, so it blocks the sequence there. `None` ⇒
/// every step is done.
pub fn first_incomplete_step(steps: &[Objective], progress: &[u32]) -> Option<usize> {
    steps
        .iter()
        .enumerate()
        .find(|(i, o)| match action_count(o) {
            Some(count) => get(progress, *i) < count,
            None => true,
        })
        .map(|(i, _)| i)
}

/// Advance `progress` for one fired event. `progress` is this objective's own
/// slice (`leaf_count(obj)` long). Passive objectives ignore events.
pub fn advance(obj: &Objective, progress: &mut [u32], ev: &ChallengeEvent) {
    match obj {
        Objective::Action { event, count } => {
            if let Some(p) = progress.first_mut()
                && event.matches(ev) && *p < *count {
                    *p += 1;
                }
        }
        Objective::Sequence { steps } => {
            // Ordered: only the current (first incomplete) step counts.
            if let Some(i) = first_incomplete_step(steps, progress)
                && let Objective::Action { event, count } = &steps[i]
                && event.matches(ev) && get(progress, i) < *count {
                    progress[i] += 1;
                }
        }
        Objective::Checklist { items } => {
            // Unordered: fill the first incomplete item that matches.
            for (i, item) in items.iter().enumerate() {
                if let Objective::Action { event, count } = item
                    && get(progress, i) < *count && event.matches(ev) {
                        progress[i] += 1;
                        break;
                    }
            }
        }
        Objective::AnyOf { routes } => {
            for (route, (start, len)) in routes.iter().zip(route_spans(obj)) {
                if let Some(slice) = progress.get_mut(start..start + len) {
                    advance(&route.objective, slice, ev);
                }
            }
        }
        Objective::Timed { .. } | Objective::FirstSatori | Objective::FreeRoam => {}
    }
}

/// Whether the event-driven objective is complete. `false` for the passive
/// objectives (they end through the clock or a material, not events).
pub fn is_complete(obj: &Objective, progress: &[u32]) -> bool {
    let all_done = |subs: &[Objective]| -> bool {
        subs.iter().enumerate().all(|(i, o)| match action_count(o) {
            Some(count) => get(progress, i) >= count,
            None => false,
        })
    };
    match obj {
        Objective::Action { count, .. } => get(progress, 0) >= *count,
        Objective::Sequence { steps } => all_done(steps),
        Objective::Checklist { items } => all_done(items),
        Objective::AnyOf { .. } => !complete_routes(obj, progress).is_empty(),
        Objective::Timed { .. } | Objective::FirstSatori | Objective::FreeRoam => false,
    }
}

/// Indices of every `AnyOf` route that is complete, in authored order. Empty
/// for any other objective.
pub fn complete_routes(obj: &Objective, progress: &[u32]) -> Vec<usize> {
    let Objective::AnyOf { routes } = obj else { return Vec::new() };
    routes
        .iter()
        .zip(route_spans(obj))
        .enumerate()
        .filter(|(_, (route, (start, len)))| {
            progress
                .get(*start..*start + *len)
                .is_some_and(|slice| is_complete(&route.objective, slice))
        })
        .map(|(i, _)| i)
        .collect()
}

/// `(done, required)` per leaf, flat — one entry per task line.
pub fn step_status(obj: &Objective, progress: &[u32]) -> Vec<(u32, u32)> {
    required(obj)
        .iter()
        .enumerate()
        .map(|(i, req)| (get(progress, i).min(*req), *req))
        .collect()
}

/// The `AnyOf` route that is furthest along: most finished leaves, then most
/// counted events, then authored order. `None` for any other objective.
pub fn leading_route(obj: &Objective, progress: &[u32]) -> Option<usize> {
    let status = step_status(obj, progress);
    let mut best: Option<(usize, (usize, u32))> = None;
    for (i, (start, len)) in route_spans(obj).into_iter().enumerate() {
        let slice = &status[start..start + len];
        let done = slice.iter().filter(|(d, r)| d >= r).count();
        let counted: u32 = slice.iter().map(|(d, _)| *d).sum();
        if best.is_none_or(|(_, b)| (done, counted) > b) {
            best = Some((i, (done, counted)));
        }
    }
    best.map(|(i, _)| i)
}

/// The HUD's `(finished leaves, total leaves)`; for an `AnyOf`, those of the
/// leading route.
pub fn leaves_done(obj: &Objective, progress: &[u32]) -> (u32, u32) {
    let status = step_status(obj, progress);
    let count_done = |s: &[(u32, u32)]| s.iter().filter(|(d, r)| d >= r).count() as u32;
    match obj {
        Objective::AnyOf { .. } => match leading_route(obj, progress) {
            Some(i) => {
                let (start, len) = route_spans(obj)[i];
                (count_done(&status[start..start + len]), len as u32)
            }
            None => (0, 0),
        },
        _ => (count_done(&status), status.len() as u32),
    }
}

/// Flat index of the step being worked on: the first unfinished leaf (for an
/// `AnyOf`, inside the leading route). `None` when nothing is left.
pub fn current_step(obj: &Objective, progress: &[u32]) -> Option<usize> {
    let status = step_status(obj, progress);
    let first_open = |from: usize, len: usize| {
        (from..from + len).find(|&i| status[i].0 < status[i].1)
    };
    match obj {
        Objective::AnyOf { .. } => {
            let i = leading_route(obj, progress)?;
            let (start, len) = route_spans(obj)[i];
            first_open(start, len)
        }
        _ => first_open(0, status.len()),
    }
}

/// The in-game "What to do" text for a task list: "✓ done / ➜ now / • next",
/// numbered when the steps are ordered, with a `(done/total)` count on an
/// unfinished multi-count step. An `AnyOf` list reads "Pick a way:" then each
/// route by name with its own steps, numbered within the route.
pub fn task_text(tasks: &crate::scenario::TaskList, status: &[(u32, u32)], current: Option<usize>) -> String {
    let line = |i: usize, number: Option<usize>, indent: &str| -> String {
        let text = &tasks.lines[i];
        let (done, total) = status.get(i).copied().unwrap_or((0, 0));
        let complete = total > 0 && done >= total;
        let marker = if complete {
            "\u{2713}" // ✓
        } else if Some(i) == current {
            "\u{279C}" // ➜
        } else {
            "\u{2022}" // •
        };
        let count = if total > 1 && !complete { format!("  ({done}/{total})") } else { String::new() };
        match number {
            Some(n) => format!("{indent}{marker} {n}. {text}{count}\n"),
            None => format!("{indent}{marker} {text}{count}\n"),
        }
    };
    let mut out = String::new();
    if tasks.routes.is_empty() {
        for i in 0..tasks.lines.len() {
            out.push_str(&line(i, tasks.ordered.then_some(i + 1), ""));
        }
    } else {
        out.push_str("Pick a way:\n");
        let mut start = 0;
        for route in &tasks.routes {
            out.push_str(&format!("{}:\n", route.name));
            for (k, i) in (start..(start + route.len).min(tasks.lines.len())).enumerate() {
                out.push_str(&line(i, route.ordered.then_some(k + 1), "   "));
            }
            start += route.len;
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Route;

    #[test]
    fn an_any_of_task_list_reads_pick_a_way_then_each_route() {
        let def = crate::scenario::ScenarioDef {
            objective: camp_cook(),
            ..crate::scenario::builtin_test_def()
        };
        let tasks = crate::scenario::objective_tasks(&def, &[]);
        assert_eq!(tasks.lines.len(), 6, "one line per leaf");
        assert_eq!(tasks.routes.iter().map(|r| r.len).collect::<Vec<_>>(), vec![3, 2, 1]);
        let mut p = vec![0; leaf_count(&def.objective)];
        advance(&def.objective, &mut p, &ChallengeEvent::CookAtCampfire);
        let text = task_text(&tasks, &step_status(&def.objective, &p), current_step(&def.objective, &p));
        assert!(text.starts_with("Pick a way:\nCatch, cook and eat:\n"), "{text}");
        assert!(text.contains("Cook and eat:\n   \u{2713} 1. "), "{text}");
        assert!(text.contains("   \u{279C} 2. "), "the leading route's next step is marked: {text}");
        assert!(text.contains("Roast three:\n   \u{2022} "), "{text}");
        assert!(text.contains("(1/3)"), "{text}");
        let groups = tasks.groups();
        assert_eq!(groups.iter().map(|g| g.name).collect::<Vec<_>>(), vec![
            Some("Catch, cook and eat"),
            Some("Cook and eat"),
            Some("Roast three"),
        ]);
        assert_eq!(groups[2].lines, vec![(5, tasks.lines[5].as_str())]);
        assert!(groups[0].ordered && !groups[2].ordered);
    }

    fn act(event: ChallengeEvent, count: u32) -> Objective {
        Objective::Action { event, count }
    }

    /// Camp Cook's shape: the most specific route first.
    fn camp_cook() -> Objective {
        Objective::AnyOf {
            routes: vec![
                Route {
                    name: "Catch, cook and eat".into(),
                    objective: Objective::Sequence {
                        steps: vec![
                            act(ChallengeEvent::CatchFish, 1),
                            act(ChallengeEvent::CookAtCampfire, 1),
                            act(ChallengeEvent::EatFood, 1),
                        ],
                    },
                },
                Route {
                    name: "Cook and eat".into(),
                    objective: Objective::Sequence {
                        steps: vec![act(ChallengeEvent::CookAtCampfire, 1), act(ChallengeEvent::EatFood, 1)],
                    },
                },
                Route { name: "Roast three".into(), objective: act(ChallengeEvent::CookAtCampfire, 3) },
            ],
        }
    }

    #[test]
    fn any_of_lays_routes_end_to_end() {
        let obj = camp_cook();
        assert_eq!(leaf_count(&obj), 6);
        assert_eq!(route_spans(&obj), vec![(0, 3), (3, 2), (5, 1)]);
        assert_eq!(required(&obj), vec![1, 1, 1, 1, 1, 3]);
    }

    #[test]
    fn every_route_keeps_its_own_progress() {
        let obj = camp_cook();
        let mut p = vec![0; leaf_count(&obj)];
        advance(&obj, &mut p, &ChallengeEvent::CookAtCampfire);
        // The fish route's step 1 is a catch, so a cook does not move it; the
        // other two both count it.
        assert_eq!(p, vec![0, 0, 0, 1, 0, 1]);
        assert!(!is_complete(&obj, &p));
        advance(&obj, &mut p, &ChallengeEvent::CookAtCampfire);
        advance(&obj, &mut p, &ChallengeEvent::CookAtCampfire);
        assert_eq!(complete_routes(&obj, &p), vec![2], "three roasts finish the roast route only");
        assert!(is_complete(&obj, &p));
    }

    #[test]
    fn routes_that_finish_together_are_all_done_and_the_first_is_named() {
        let obj = camp_cook();
        let mut p = vec![0; leaf_count(&obj)];
        for ev in [ChallengeEvent::CatchFish, ChallengeEvent::CookAtCampfire, ChallengeEvent::EatFood] {
            advance(&obj, &mut p, &ev);
        }
        assert_eq!(complete_routes(&obj, &p), vec![0, 1]);
    }

    #[test]
    fn the_hud_follows_the_leading_route() {
        let obj = camp_cook();
        let mut p = vec![0; leaf_count(&obj)];
        assert_eq!(leaves_done(&obj, &p), (0, 3), "nothing done: the first route leads");
        assert_eq!(current_step(&obj, &p), Some(0));
        advance(&obj, &mut p, &ChallengeEvent::CookAtCampfire);
        // "Cook and eat" now has a finished leaf; it leads.
        assert_eq!(leading_route(&obj, &p), Some(1));
        assert_eq!(leaves_done(&obj, &p), (1, 2));
        assert_eq!(current_step(&obj, &p), Some(4), "its eat step, as a flat index");
        assert_eq!(step_status(&obj, &p)[5], (1, 3));
    }

    #[test]
    fn plain_objectives_behave_as_before() {
        let seq = Objective::Sequence {
            steps: vec![act(ChallengeEvent::CatchFish, 1), act(ChallengeEvent::EatFood, 2)],
        };
        let mut p = vec![0; 2];
        advance(&seq, &mut p, &ChallengeEvent::EatFood);
        assert_eq!(p, vec![0, 0], "out of order is ignored");
        advance(&seq, &mut p, &ChallengeEvent::CatchFish);
        advance(&seq, &mut p, &ChallengeEvent::EatFood);
        assert_eq!(leaves_done(&seq, &p), (1, 2));
        advance(&seq, &mut p, &ChallengeEvent::EatFood);
        assert!(is_complete(&seq, &p));
        assert!(complete_routes(&seq, &p).is_empty());
        assert_eq!(current_step(&seq, &p), None);
    }
}
