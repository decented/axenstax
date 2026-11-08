//! The Trials island (§C.3), written into a world: what the layout is fed
//! (which trials are open, which are hinted at, their dioramas as blocks) and
//! the column-by-column write of the layout into the island world. The layout
//! itself is pure (`trials_island_layout`); the doorways and the return trip
//! are `trials_island_flow`.
//!
//! A column is written only once its floor chunk exists: a write into a column
//! that has not been generated would conjure an empty chunk that the flat
//! generator then skips, leaving a hole with no floor. Writing is idempotent
//! (only cells that differ are touched), so the island is written again on
//! every visit and a trial unlocked since the last visit gets its doorway.

use crate::block::{self, BlockId};
use crate::trials::{TrialBests, TrialRef, TrialType, TRIAL_ORDER};
use crate::trials_gallery::{row_kind_of, RowKind};
use crate::trials_island_layout::{District, IslandLayout, SlotInput, SlotState, ISLAND_R};
use crate::world::World;

/// The world folder of the island (`ArenaMode::Resume`: one persistent world).
pub fn island_folder() -> String {
    crate::scenario::experience_world_slug(&crate::scenario::trials_island_def().display_name)
}

/// Which district a trial stands in (the gallery's rows).
pub fn district_of(ttype: TrialType, tref: TrialRef) -> District {
    match row_kind_of(ttype, tref) {
        RowKind::Clock => District::Clock,
        RowKind::Shelf(TrialType::Make) => District::Make,
        RowKind::Shelf(TrialType::Tend) => District::Tend,
        RowKind::Shelf(TrialType::Tech) => District::Tech,
        RowKind::Shelf(TrialType::Brave) => District::Brave,
        // The island has four wedges; the Bloom shelf's one trial stands in the
        // Tend wedge (a fifth district is not built).
        RowKind::Shelf(TrialType::Bloom) => District::Tend,
        RowKind::Shelf(TrialType::Race) => District::Clock,
    }
}

fn resolve_block(name: &str) -> Option<BlockId> {
    crate::commands::builtins::give::resolve_item(name, 1).ok()?.item.as_block()
}

/// A trial's card diorama as world blocks (a lone block name is one block).
pub fn diorama_cells(tref: TrialRef) -> Vec<([i32; 3], BlockId)> {
    match tref {
        TrialRef::Race(id) => crate::trials_gallery::race_art_cells(id)
            .iter()
            .filter_map(|(name, at)| Some((*at, resolve_block(name)?)))
            .collect(),
        TrialRef::Challenge(token) => {
            let Some(def) = crate::scenario::named_builtin_def(token) else { return Vec::new() };
            match def.card_art {
                Some(crate::trials_diorama::CardArtDef::Diorama(blocks)) => blocks
                    .iter()
                    .filter_map(|b| Some((b.at, resolve_block(&b.block)?)))
                    .collect(),
                Some(crate::trials_diorama::CardArtDef::Name(name)) => {
                    resolve_block(&name).map(|b| vec![([0, 0, 0], b)]).unwrap_or_default()
                }
                None => Vec::new(),
            }
        }
    }
}

/// Every trial's plot for these bests, in `TRIAL_ORDER` order: open where the
/// gallery shows the card, a hint where `next_hints` names it, hidden
/// otherwise. Titles and hints are the gallery's own words.
pub fn inputs(bests: &TrialBests) -> Vec<SlotInput> {
    let titles: Vec<(&'static str, String)> = crate::trials_gallery::build_rows(bests)
        .into_iter()
        .flat_map(|row| row.cards.into_iter().map(|c| (c.token, c.title)))
        .collect();
    let hints = crate::trials_unlock::next_hints(bests);
    TRIAL_ORDER
        .iter()
        .map(|(ttype, tref)| {
            let token = match tref {
                TrialRef::Race(id) | TrialRef::Challenge(id) => *id,
            };
            let state = if let Some((_, title)) = titles.iter().find(|(t, _)| *t == token) {
                SlotState::Open { title: title.clone() }
            } else if let Some((_, hint)) = hints.iter().find(|(t, h)| *t == token && !h.trim().is_empty()) {
                SlotState::Hint(hint.clone())
            } else {
                SlotState::Hidden
            };
            SlotInput { token, district: district_of(*ttype, *tref), state, diorama: diorama_cells(*tref) }
        })
        .collect()
}

/// The island for these bests.
pub fn layout_for(bests: &TrialBests) -> IslandLayout {
    crate::trials_island_layout::layout(&inputs(bests))
}

/// The chunk-row that holds the flat floor.
pub fn floor_cy() -> i32 {
    crate::trials_island_layout::GROUND_Y.div_euclid(crate::chunk::CHUNK_SIZE as i32)
}

/// Write the layout's cells in chunk column `(cx, cz)` into `world`, with the
/// column's signs (text and facing). Returns the cells that changed. Cells
/// already right are left alone, so a second write changes nothing. A sign
/// the layout no longer has (a hint plot that became a doorway) is removed.
pub fn apply_column(world: &mut World, layout: &IslandLayout, cx: i32, cz: i32) -> Vec<(i32, i32, i32)> {
    let cs = crate::chunk::CHUNK_SIZE as i32;
    let mut changed = Vec::new();
    for (pos, b) in layout.column_cells(cx, cz, cs) {
        if world.get_block(pos.0, pos.1, pos.2) != b {
            if world.sign_at(pos).is_some() && b != block::OAK_SIGN {
                world.remove_block_entity(pos);
            }
            world.set_block(pos.0, pos.1, pos.2, b);
            changed.push(pos);
        }
        if b != block::OAK_SIGN && world.meta_at(pos.0, pos.1, pos.2) != 0 {
            world.set_meta(pos, 0);
        }
    }
    let in_column = |p: [i32; 3]| p[0].div_euclid(cs) == cx && p[2].div_euclid(cs) == cz;
    for sign in layout.signs.iter().filter(|s| in_column(s.pos)) {
        let pos = (sign.pos[0], sign.pos[1], sign.pos[2]);
        let m = crate::meta::with_facing(0, sign.facing);
        if world.meta_at(pos.0, pos.1, pos.2) != m {
            world.set_meta(pos, m);
            changed.push(pos);
        }
        if world.sign_at(pos).map(|s| s.text.as_str()) != Some(sign.text.as_str()) {
            let mut data = crate::sign::SignData::new();
            data.set_text(&sign.text);
            world.insert_sign(pos, data);
            changed.push(pos);
        }
    }
    // Stray signs in the island's built area (left by an older layout).
    let strays: Vec<(i32, i32, i32)> = world
        .iter_signs()
        .map(|(p, _)| p)
        .filter(|p| in_column([p.0, p.1, p.2]) && p.0.abs().max(p.2.abs()) <= ISLAND_R)
        .filter(|p| !layout.signs.iter().any(|s| (s.pos[0], s.pos[1], s.pos[2]) == *p))
        .collect();
    for p in strays {
        world.remove_block_entity(p);
        changed.push(p);
    }
    changed
}

/// The postcard exhibits the layout hangs: `(token, image_ref, exhibit)` for
/// every open doorway whose trial has a postcard (`has_postcard`).
pub fn exhibits(layout: &IslandLayout, has_postcard: impl Fn(&str) -> bool) -> Vec<(&'static str, crate::exhibit::Exhibit)> {
    layout
        .doorways
        .iter()
        .filter(|d| has_postcard(d.token))
        .map(|d| {
            (
                d.token,
                crate::exhibit::Exhibit {
                    x: d.exhibit_at[0],
                    y: d.exhibit_at[1],
                    z: d.exhibit_at[2],
                    presentation: crate::exhibit::Presentation::Wall,
                    image_ref: exhibit_ref(d.token),
                    width: 3.2,
                    height: 1.8,
                    yaw: d.exhibit_yaw,
                    label: String::new(),
                    link: None,
                    sku: None,
                    price: None,
                },
            )
        })
        .collect()
}

/// The island's own exhibit images are named after their trial.
pub fn exhibit_ref(token: &str) -> String {
    format!("{EXHIBIT_PREFIX}{token}.png")
}

pub const EXHIBIT_PREFIX: &str = "island-postcard-";

/// Replace the island's postcard exhibits in `list` with `wanted`, keeping
/// anything else. Returns true when the list changed.
pub fn merge_exhibits(list: &mut Vec<crate::exhibit::Exhibit>, wanted: Vec<crate::exhibit::Exhibit>) -> bool {
    let before = list.clone();
    list.retain(|e| !e.image_ref.starts_with(EXHIBIT_PREFIX));
    list.extend(wanted);
    *list != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_the_island_twice_changes_nothing_the_second_time() {
        let bests = TrialBests::default();
        let layout = layout_for(&bests);
        let mut world = World::new();
        let cols = IslandLayout::columns(crate::chunk::CHUNK_SIZE as i32);
        let first: usize = cols.iter().map(|(cx, cz)| apply_column(&mut world, &layout, *cx, *cz).len()).sum();
        assert!(first > 1000, "the first write builds the island ({first} cells)");
        let second: usize = cols.iter().map(|(cx, cz)| apply_column(&mut world, &layout, *cx, *cz).len()).sum();
        assert_eq!(second, 0, "writing it again changes nothing");
        for s in &layout.signs {
            let pos = (s.pos[0], s.pos[1], s.pos[2]);
            assert_eq!(world.sign_at(pos).map(|d| d.text.clone()), Some(s.text.clone()));
            assert_eq!(crate::meta::facing(world.meta_at(pos.0, pos.1, pos.2)), s.facing);
        }
    }

    #[test]
    fn a_hint_plot_that_unlocks_becomes_a_doorway_and_its_sign_goes() {
        // A player who has done Mine sees the next Make trials hinted at.
        let mut bests = TrialBests::default();
        bests.mark_challenge_done(&crate::scenario::named_builtin_def("mine").unwrap().display_name);
        let before = layout_for(&bests);
        let (token, _) = *crate::trials_unlock::next_hints(&bests).first().expect("a next trial");
        assert!(before.doorway(token).is_none());
        let mut world = World::new();
        let cols = IslandLayout::columns(crate::chunk::CHUNK_SIZE as i32);
        for (cx, cz) in &cols {
            apply_column(&mut world, &before, *cx, *cz);
        }
        bests.unlocked.insert(token.to_string());
        let after = layout_for(&bests);
        assert!(after.doorway(token).is_some(), "{token} has a doorway once unlocked");
        for (cx, cz) in &cols {
            apply_column(&mut world, &after, *cx, *cz);
        }
        let signs: Vec<_> = world.iter_signs().map(|(p, s)| (p, s.text.clone())).collect();
        assert_eq!(signs.len(), after.signs.len(), "no stray hint sign is left: {signs:?}");
        for (cx, cz) in &cols {
            assert!(apply_column(&mut world, &after, *cx, *cz).is_empty());
        }
    }

    #[test]
    fn exhibits_hang_only_for_trials_with_a_postcard_and_merge_keeps_others() {
        let layout = layout_for(&TrialBests::default());
        let hung = exhibits(&layout, |t| t == "mine");
        assert_eq!(hung.len(), 1);
        assert_eq!(hung[0].1.image_ref, "island-postcard-mine.png");
        let mut list = vec![crate::exhibit::Exhibit { image_ref: "art.png".into(), ..hung[0].1.clone() }];
        assert!(merge_exhibits(&mut list, hung.iter().map(|(_, e)| e.clone()).collect()));
        assert_eq!(list.len(), 2);
        assert!(!merge_exhibits(&mut list, hung.iter().map(|(_, e)| e.clone()).collect()), "the same set changes nothing");
        assert!(merge_exhibits(&mut list, Vec::new()));
        assert_eq!(list.len(), 1, "the player's own art stays");
    }
}
