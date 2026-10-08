//! C3b-2 (protocol v79) — what joiners are shown of composters, drying
//! racks, campfires, item frames and bee hives (`protocol::BlockEntityView`;
//! Spec 04 §4.2f).
//!
//! A joiner's use of one of these blocks changes the SERVER's block entity
//! (`block_use`); so does a host's click on its lent world, another joiner,
//! and the world's own ticks (cooking, seasoning, composting, bees). None of
//! that is a block edit, so before v79 no joiner ever saw it. Now:
//!
//! - **Server.** Once a tick, after the world's machines ran, the server
//!   takes the view of every such block entity ([`views_in`]: only what the
//!   block id and meta don't carry, with per-tick countdowns carried to the
//!   second so a burning fire changes about once a second). Each joiner's
//!   [`ViewsSent`] says which of them it hasn't been shown as they stand —
//!   a changed view, or any view in a chunk pushed to it since (the push's
//!   render stubs cover only a frame's item and a fire's burn state) — and
//!   those go into its outbox, in line after the chunk pushes queued that
//!   tick (`state_outbox`), on [`crate::protocol::StateUpdatePacket::block_views`].
//!   Only chunks in its sent-set (pushed, or noted local) are covered.
//! - **Client.** A view is applied in its place in the world stream
//!   (`chunk_intake`), after the snapshot it updates ([`apply_view`]): it
//!   replaces the joiner's copy of that state. Nothing on a joined client
//!   changes these blocks itself any more.
//!
//! Composter, drying rack and hive views have no renderer yet (no mesh or
//! HUD reads them; a joiner's own use toasts come from its outcome). They
//! are carried so a joiner's world holds the server's state.

use std::collections::HashMap;

use crate::protocol::{BlockEntityView, BlockView};
use crate::state_outbox::ChunkCoord;
use crate::world::{BlockEntityData, World};

/// Ticks in a second, the step every per-tick counter is carried in.
const SECOND: u32 = 20;

/// A countdown (fuel, smoke, smoulder left), carried UP to the whole second:
/// a fire with any fuel left still reads lit.
fn up_to_second(ticks: u32) -> u32 {
    ticks.div_ceil(SECOND).saturating_mul(SECOND)
}

/// Progress that counts up (cooking), carried DOWN to the whole second:
/// cooking finishes at a multiple of a second (`COOK_TICKS_PER_ITEM`), so
/// "cooked" is exact.
fn down_to_second(ticks: u32) -> u32 {
    ticks / SECOND * SECOND
}

/// A drying rack's seasoning is carried in whole percent of a season, down
/// (`SEASON_TICKS` is a multiple of 100, so "seasoned" is exact).
const RACK_STEP: u32 = crate::drying_rack::SEASON_TICKS / 100;

const _: () = assert!(crate::campfire::COOK_TICKS_PER_ITEM.is_multiple_of(SECOND));
const _: () = assert!(crate::drying_rack::SEASON_TICKS.is_multiple_of(100));

/// The view of block entity `e`, if it is one joiners are shown.
pub fn view_of(e: &BlockEntityData) -> Option<BlockView> {
    match e {
        BlockEntityData::ItemFrame(f) => {
            let (item_kind, item_id) = f.item.as_ref().map_or((crate::protocol::item_kind::EMPTY, 0), |st| {
                crate::inventory::item_to_ref(&st.item).to_wire()
            });
            let full_item = f.item.as_ref().map(|st| crate::inventory::item_to_wire_full(&st.item)).unwrap_or_default();
            Some(BlockView::ItemFrame { item_kind, item_id, full_item, rotation: f.rotation })
        }
        BlockEntityData::Campfire(c) => {
            let mut slots = c.slots.clone();
            for s in &mut slots {
                s.progress_ticks = down_to_second(s.progress_ticks);
            }
            Some(BlockView::Campfire {
                fuel_ticks: up_to_second(c.fuel_ticks),
                smoke_ticks: up_to_second(c.smoke_ticks),
                smoulder_ticks: up_to_second(c.smoulder_ticks),
                raid_warning: c.raid_warning_active,
                slots,
            })
        }
        BlockEntityData::Composter(w) => Some(BlockView::Composter {
            input: w.input.as_ref().map(crate::inventory::stack_to_wire),
            output: w.output.as_ref().map(crate::inventory::stack_to_wire),
        }),
        BlockEntityData::Hive(h) => Some(BlockView::Hive { honey_level: h.honey_level }),
        _ => None,
    }
}

/// The view of a drying rack.
pub fn rack_view(r: &crate::drying_rack::DryingRackData) -> BlockView {
    let mut slots = r.slots;
    for s in &mut slots {
        s.seasoning_ticks = s.seasoning_ticks / RACK_STEP * RACK_STEP;
    }
    BlockView::DryingRack { slots }
}

/// Every view in `world`: its item frames, campfires, composters and hives
/// (`block_entities`) and its drying racks (`drying_racks`).
pub fn views_in(world: &World) -> Vec<BlockEntityView> {
    let entities = world.block_entities.iter().filter_map(|(&cell, e)| view_of(e).map(|view| (cell, view)));
    let racks = world.drying_racks.iter().map(|(&cell, r)| (cell, rack_view(r)));
    entities
        .chain(racks)
        .map(|((x, y, z), view)| BlockEntityView { cell: [x, y, z], kind: view.kind(), view })
        .collect()
}

/// What one joiner has been shown: each cell's view, under the number of
/// the push of its chunk it was shown after (`chunk_push`). Fresh per attach
/// (`ServerPlayer::block_views`).
#[derive(Debug, Default)]
pub struct ViewsSent {
    /// cell → (push number of its chunk, the round it was last seen in, the view shown).
    sent: HashMap<(i32, i32, i32), (u32, u64, BlockView)>,
    round: u64,
}

impl ViewsSent {
    /// Of this tick's `views`, the ones this joiner must be sent now: those in
    /// a chunk it holds (`push_of`: the number of that chunk's latest push,
    /// `None` when it isn't in its sent-set) that it hasn't been shown as
    /// they stand since that push. A cell whose view went (its block was
    /// broken) or whose chunk it let go of is forgotten, so it is shown again
    /// when it comes back.
    pub fn take_changed(
        &mut self,
        views: &[BlockEntityView],
        push_of: impl Fn(ChunkCoord) -> Option<u32>,
    ) -> Vec<BlockEntityView> {
        self.round += 1;
        let round = self.round;
        let mut out = Vec::new();
        for v in views {
            let cell = (v.cell[0], v.cell[1], v.cell[2]);
            let Some(push) = push_of(crate::state_outbox::chunk_of_cell(cell)) else { continue };
            match self.sent.get_mut(&cell) {
                Some((p, seen, shown)) if *p == push && *shown == v.view => *seen = round,
                _ => {
                    self.sent.insert(cell, (push, round, v.view.clone()));
                    out.push(v.clone());
                }
            }
        }
        self.sent.retain(|_, (_, seen, _)| *seen == round);
        out
    }

    /// Cells it holds a record of. Test-only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sent.len()
    }
}

/// A joined client takes in view `v` (in its place in the world stream):
/// its copy of that block entity becomes what the server showed. Returns
/// whether the cell's chunk (and, for a campfire's raid tint, its smoke
/// pillar) must be remeshed: a frame's item is drawn, and so is the tint.
/// A view whose `kind` isn't its own kind (a malformed packet) changes
/// nothing.
pub fn apply_view(world: &mut World, registry: &crate::block::BlockRegistry, v: &BlockEntityView) -> bool {
    if v.kind != v.view.kind() {
        return false;
    }
    let cell = (v.cell[0], v.cell[1], v.cell[2]);
    match &v.view {
        BlockView::ItemFrame { item_kind, item_id, full_item, rotation } => {
            let item = crate::inventory::item_from_wire_full(full_item)
                .or_else(|| crate::inventory::item_from_ref(*item_kind, *item_id, registry));
            let mut frame = crate::item_frame::ItemFrameData::new();
            frame.item = item.map(|item| crate::item::ItemStack { item, count: 1 });
            frame.rotation = *rotation % crate::item_frame::FRAME_ROTATIONS;
            world.insert_item_frame(cell, frame);
            true
        }
        BlockView::Campfire { fuel_ticks, smoke_ticks, smoulder_ticks, raid_warning, slots } => {
            let cf = world.campfire_at_mut_or_default(cell);
            let tint = cf.raid_warning_active != *raid_warning;
            cf.fuel_ticks = *fuel_ticks;
            cf.smoke_ticks = *smoke_ticks;
            cf.smoulder_ticks = *smoulder_ticks;
            cf.raid_warning_active = *raid_warning;
            cf.slots = slots.clone();
            tint
        }
        BlockView::DryingRack { slots } => {
            world.drying_racks.insert(cell, crate::drying_rack::DryingRackData { slots: *slots });
            false
        }
        BlockView::Composter { input, output } => {
            let decode = |w: &Option<crate::protocol::WireStack>| {
                w.as_ref().and_then(|w| crate::inventory::stack_from_wire(w, registry, false))
            };
            let mut state = world.composter_at(cell).cloned().unwrap_or_default();
            state.input = decode(input);
            state.output = decode(output);
            world.insert_composter(cell, state);
            false
        }
        BlockView::Hive { honey_level } => {
            let mut hive = world.hive_at(cell).copied().unwrap_or_default();
            hive.honey_level = (*honey_level).min(crate::bee_hive::MAX_HONEY_LEVEL);
            world.insert_hive(cell, hive);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{self, BlockRegistry};
    use crate::campfire::{CampfireData, CookSlot};
    use crate::drying_rack::{DryingRackData, LogSpecies, RackSlot};
    use crate::item::{ItemStack, MaterialId};
    use crate::protocol::BlockViewKind;

    fn cell_view(views: &[BlockEntityView], cell: [i32; 3]) -> &BlockView {
        &views.iter().find(|v| v.cell == cell).expect("a view at the cell").view
    }

    #[test]
    fn a_burning_fire_changes_its_view_once_a_second_and_cooked_is_exact() {
        let mut cf = CampfireData { fuel_ticks: 41, ..Default::default() };
        cf.slots[0] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: 199 };
        let a = view_of(&BlockEntityData::Campfire(cf.clone())).unwrap();
        cf.fuel_ticks = 40;
        cf.slots[0].progress_ticks = 199;
        let b = view_of(&BlockEntityData::Campfire(cf.clone())).unwrap();
        assert_ne!(a, b, "41 ticks reads 3 s, 40 reads 2 s");
        cf.fuel_ticks = 21;
        let c = view_of(&BlockEntityData::Campfire(cf.clone())).unwrap();
        cf.fuel_ticks = 22;
        assert_eq!(c, view_of(&BlockEntityData::Campfire(cf.clone())).unwrap(), "within a second: unchanged");
        cf.fuel_ticks = 1;
        let BlockView::Campfire { fuel_ticks, slots, .. } = view_of(&BlockEntityData::Campfire(cf.clone())).unwrap() else {
            unreachable!()
        };
        assert_eq!(fuel_ticks, 20, "any fuel left still reads lit");
        assert_eq!(slots[0].progress_ticks, 180, "199 ticks is not cooked yet");
        cf.slots[0].progress_ticks = crate::campfire::COOK_TICKS_PER_ITEM;
        let BlockView::Campfire { slots, .. } = view_of(&BlockEntityData::Campfire(cf)).unwrap() else { unreachable!() };
        assert_eq!(slots[0].progress_ticks, crate::campfire::COOK_TICKS_PER_ITEM, "cooked is exact");
    }

    #[test]
    fn views_in_finds_the_five_kinds_and_nothing_else() {
        let mut w = World::new();
        w.insert_item_frame((1, 70, 1), crate::item_frame::ItemFrameData::new());
        w.insert_campfire((2, 70, 1), CampfireData::default());
        w.insert_composter((3, 70, 1), crate::workstation::WorkstationState::new());
        w.insert_hive((4, 70, 1), crate::bee_hive::HiveData { bees_inside: 1, honey_level: 3 });
        w.drying_racks.insert((5, 70, 1), DryingRackData::default());
        w.insert_chest((6, 70, 1), crate::chest::ChestData::for_tier(crate::chest::ChestTier::Wood));
        let views = views_in(&w);
        assert_eq!(views.len(), 5, "not the chest");
        for v in &views {
            assert_eq!(v.kind, v.view.kind());
        }
        assert_eq!(cell_view(&views, [4, 70, 1]), &BlockView::Hive { honey_level: 3 });
    }

    #[test]
    fn a_joiner_is_sent_a_view_once_until_it_changes_or_its_chunk_is_pushed_again() {
        let mut sent = ViewsSent::default();
        let v = |honey| BlockEntityView {
            cell: [1, 70, 1],
            kind: BlockViewKind::Hive,
            view: BlockView::Hive { honey_level: honey },
        };
        let covered = |n: u32| move |_: ChunkCoord| Some(n);
        assert!(sent.take_changed(&[v(1)], |_| None).is_empty(), "a chunk it doesn't hold: nothing");
        assert_eq!(sent.take_changed(&[v(1)], covered(4)), vec![v(1)], "first sight after the push");
        assert!(sent.take_changed(&[v(1)], covered(4)).is_empty(), "unchanged: nothing");
        assert_eq!(sent.take_changed(&[v(2)], covered(4)), vec![v(2)], "changed: sent");
        assert_eq!(sent.take_changed(&[v(2)], covered(9)), vec![v(2)], "its chunk pushed again: sent after it");
        assert!(sent.take_changed(&[], covered(9)).is_empty());
        assert_eq!(sent.len(), 0, "a view that went is forgotten");
        assert_eq!(sent.take_changed(&[v(2)], covered(9)), vec![v(2)], "and shown again when it comes back");
    }

    #[test]
    fn a_joined_client_takes_in_each_view() {
        let reg = BlockRegistry::new();
        let mut w = World::new();
        let views = {
            let mut s = World::new();
            let mut frame = crate::item_frame::ItemFrameData::new();
            frame.try_insert(ItemStack::new_block(block::STONE, 1));
            frame.rotate();
            s.insert_item_frame((1, 70, 1), frame);
            let mut cf = CampfireData { fuel_ticks: 100, raid_warning_active: true, ..Default::default() };
            cf.slots[2] = CookSlot { item: Some(MaterialId::RawBeef), progress_ticks: 60 };
            s.insert_campfire((2, 70, 1), cf);
            let mut bin = crate::workstation::WorkstationState::new();
            bin.input = Some(ItemStack::new_material(MaterialId::Wheat, 4));
            bin.output = Some(ItemStack::new_material(MaterialId::Compost, 2));
            s.insert_composter((3, 70, 1), bin);
            s.insert_hive((4, 70, 1), crate::bee_hive::HiveData { bees_inside: 0, honey_level: 5 });
            let mut rack = DryingRackData::default();
            rack.slots[1] = RackSlot { species: Some(LogSpecies::Oak), seasoning_ticks: 3_000 };
            s.drying_racks.insert((5, 70, 1), rack);
            views_in(&s)
        };
        let remeshed: Vec<bool> = views.iter().map(|v| apply_view(&mut w, &reg, v)).collect();
        assert_eq!(remeshed.iter().filter(|r| **r).count(), 2, "the frame, and the fire's new raid tint");
        let frame = w.item_frame_at((1, 70, 1)).unwrap();
        assert_eq!(frame.item.as_ref().unwrap().item, crate::item::Item::Block(block::STONE));
        assert_eq!(frame.rotation, 1);
        let cf = w.campfire_at((2, 70, 1)).unwrap();
        assert_eq!((cf.fuel_ticks, cf.raid_warning_active), (100, true));
        assert_eq!(cf.slots[2].item, Some(MaterialId::RawBeef));
        let bin = w.composter_at((3, 70, 1)).unwrap();
        assert_eq!(bin.input.as_ref().unwrap().count, 4);
        assert_eq!(bin.output.as_ref().unwrap().count, 2);
        assert_eq!(w.hive_at((4, 70, 1)).unwrap().honey_level, 5);
        assert_eq!(w.drying_racks[&(5, 70, 1)].slots[1].seasoning_ticks, 3_000);
        // The view is the state: the same views again change nothing drawn.
        assert!(views.iter().all(|v| !apply_view(&mut w, &reg, v) || v.kind == BlockViewKind::ItemFrame));
        // A malformed view (kind ≠ its own) is ignored.
        let bad = BlockEntityView { cell: [9, 70, 9], kind: BlockViewKind::Hive, view: BlockView::Hive { honey_level: 1 } };
        let bad = BlockEntityView { kind: BlockViewKind::Composter, ..bad };
        assert!(!apply_view(&mut w, &reg, &bad));
        assert!(w.hive_at((9, 70, 9)).is_none());
    }
}
