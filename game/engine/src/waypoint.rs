//! Waypoints — named map markers (#6): manual pins + automatic death-markers.
//! Per-world, persisted in the world save (`WorldSave.waypoints`). The mutation
//! ops here are pure free functions over a `Vec<Waypoint>` so they unit-test
//! headless; the command surface (`/waypoint`) and the minimap render call in.
//!
//! Spec: `docs/foundations/2026-06-16-minimap-waypoints-map.md`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaypointKind {
    /// Player-placed pin.
    Manual,
    /// Auto-dropped where the player last died (rolling, capped).
    Death,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Waypoint {
    pub id: u32,
    pub name: String,
    pub pos: [i32; 3],
    pub colour: [u8; 3],
    pub kind: WaypointKind,
}

/// Default pin colour (amber).
pub const MANUAL_COLOUR: [u8; 3] = [255, 210, 64];
/// Death-marker colour (red).
pub const DEATH_COLOUR: [u8; 3] = [220, 60, 60];
/// How many death markers to keep before the oldest rolls off.
pub const MAX_DEATH_MARKERS: usize = 5;

/// Next free id = max existing id + 1 (0 when empty). Unique among the current
/// set, which is all the UI keying needs; reusing a just-removed top id is fine.
pub fn next_id(list: &[Waypoint]) -> u32 {
    list.iter().map(|w| w.id).max().map_or(0, |m| m + 1)
}

/// Append a waypoint with a fresh id; returns that id.
pub fn add(
    list: &mut Vec<Waypoint>,
    name: String,
    pos: [i32; 3],
    colour: [u8; 3],
    kind: WaypointKind,
) -> u32 {
    let id = next_id(list);
    list.push(Waypoint { id, name, pos, colour, kind });
    id
}

/// Remove the first waypoint whose name matches `name` (case-insensitive).
/// Returns true if one was removed.
pub fn remove_by_name(list: &mut Vec<Waypoint>, name: &str) -> bool {
    if let Some(i) = list.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
        list.remove(i);
        true
    } else {
        false
    }
}

/// Find a waypoint by name (case-insensitive).
pub fn find_by_name<'a>(list: &'a [Waypoint], name: &str) -> Option<&'a Waypoint> {
    list.iter().find(|w| w.name.eq_ignore_ascii_case(name))
}

/// Record a death marker at `pos`, evicting the oldest **Death** marker first if
/// already at [`MAX_DEATH_MARKERS`]. Manual pins are never touched. Returns the
/// new marker's id.
pub fn push_death_marker(list: &mut Vec<Waypoint>, pos: [i32; 3]) -> u32 {
    let death_count = list.iter().filter(|w| w.kind == WaypointKind::Death).count();
    if death_count >= MAX_DEATH_MARKERS {
        // Evict the oldest death marker (first in insertion order).
        if let Some(i) = list.iter().position(|w| w.kind == WaypointKind::Death) {
            list.remove(i);
        }
    }
    let name = format!("Death ({}, {})", pos[0], pos[2]);
    add(list, name, pos, DEATH_COLOUR, WaypointKind::Death)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(id: u32, name: &str, kind: WaypointKind) -> Waypoint {
        Waypoint {
            id,
            name: name.to_string(),
            pos: [0, 64, 0],
            colour: MANUAL_COLOUR,
            kind,
        }
    }

    #[test]
    fn next_id_is_zero_for_empty_and_max_plus_one_otherwise() {
        assert_eq!(next_id(&[]), 0);
        let list = vec![wp(3, "a", WaypointKind::Manual), wp(1, "b", WaypointKind::Manual)];
        assert_eq!(next_id(&list), 4);
    }

    #[test]
    fn add_appends_with_a_fresh_id() {
        let mut list = vec![wp(0, "home", WaypointKind::Manual)];
        let id = add(&mut list, "mine".into(), [10, 20, 30], MANUAL_COLOUR, WaypointKind::Manual);
        assert_eq!(id, 1);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].name, "mine");
        assert_eq!(list[1].pos, [10, 20, 30]);
    }

    #[test]
    fn remove_by_name_is_case_insensitive() {
        let mut list = vec![wp(0, "Base", WaypointKind::Manual)];
        assert!(remove_by_name(&mut list, "base"));
        assert!(list.is_empty());
        assert!(!remove_by_name(&mut list, "nope"));
    }

    #[test]
    fn find_by_name_is_case_insensitive() {
        let list = vec![wp(0, "Cave", WaypointKind::Manual)];
        assert!(find_by_name(&list, "CAVE").is_some());
        assert!(find_by_name(&list, "den").is_none());
    }

    #[test]
    fn push_death_marker_tags_kind_and_colour() {
        let mut list = Vec::new();
        let id = push_death_marker(&mut list, [5, 60, 7]);
        assert_eq!(id, 0);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, WaypointKind::Death);
        assert_eq!(list[0].colour, DEATH_COLOUR);
        assert_eq!(list[0].pos, [5, 60, 7]);
    }

    #[test]
    fn death_markers_roll_off_at_the_cap_but_keep_manual_pins() {
        let mut list = vec![wp(0, "home", WaypointKind::Manual)];
        for i in 0..MAX_DEATH_MARKERS + 2 {
            push_death_marker(&mut list, [i as i32, 60, 0]);
        }
        let deaths = list.iter().filter(|w| w.kind == WaypointKind::Death).count();
        let manuals = list.iter().filter(|w| w.kind == WaypointKind::Manual).count();
        assert_eq!(deaths, MAX_DEATH_MARKERS, "death markers capped");
        assert_eq!(manuals, 1, "manual pins never evicted");
        // The oldest death (x=0) rolled off; the newest (x=MAX+1) is present.
        assert!(list.iter().any(|w| w.kind == WaypointKind::Death && w.pos[0] == (MAX_DEATH_MARKERS + 1) as i32));
        assert!(!list.iter().any(|w| w.kind == WaypointKind::Death && w.pos[0] == 0));
    }
}
