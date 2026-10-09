//! Whole-window wire conversion and client application without replacing preferences.
use crate::{
    item::Item,
    protocol::{InventorySyncPacket, WireWindowSlot},
    window_events::EffectiveWindow,
};
pub fn packet(w: &EffectiveWindow, window_event: u32) -> InventorySyncPacket {
    let slots = crate::container_window::PlayerSlots::of(&w.inv, &w.armour, &w.cursor, &w.grid);
    let ats = (0..36)
        .map(WireWindowSlot::Inv)
        .chain((0..4).map(WireWindowSlot::Armour))
        .chain(std::iter::once(WireWindowSlot::Cursor))
        .chain((0..3).flat_map(|r| (0..3).map(move |c| WireWindowSlot::Grid(r, c))));
    InventorySyncPacket {
        window_event,
        slots: ats.map(|at| (at, slots.wire(at).unwrap())).collect(),
    }
}
pub fn apply(
    w: &mut crate::window::WindowMut,
    pkt: &InventorySyncPacket,
    reg: &crate::block::BlockRegistry,
    uses: &mut crate::use_edits::SentUses,
) {
    let predicted_plans = uses.retire_for_replace();
    let old = crate::container_window::PlayerSlots::of(w.inv, w.armour, w.cursor, w.grid);
    let plans: Vec<_> = (0..36)
        .map(WireWindowSlot::Inv)
        .chain(std::iter::once(WireWindowSlot::Cursor))
        .chain((0..3).flat_map(|r| (0..3).map(move |c| WireWindowSlot::Grid(r, c))))
        .filter_map(|at| match old.stack(at).flatten() {
            Some(s) => {
                if let Item::Plan(p) = s.item
                    && p.marker.is_none()
                {
                    Some((at, p))
                } else {
                    None
                }
            }
            None => None,
        })
        .collect();
    let builtin = crate::satoshi::starter_hut_plan();
    for (at, wire) in &pkt.slots {
        let stack = wire
            .as_ref()
            .and_then(|s| {
                crate::inventory::stack_from_wire(s, reg, crate::inventory::PlanDecode::Marker)
            })
            .map(|mut s| {
                if let Item::Plan(p) = &s.item {
                    let marker = crate::plan::marker(p);
                    if let Some(body) = plans
                        .iter()
                        .filter(|(slot, _)| slot == at)
                        .map(|(_, p)| p)
                        .chain(plans.iter().filter(|(slot, _)| slot != at).map(|(_, p)| p))
                        .chain(predicted_plans.iter())
                        .chain(std::iter::once(&builtin))
                        .find(|p| crate::plan::marker(p) == marker)
                    {
                        s.item = Item::Plan(body.clone());
                    } else {
                        log::warn!("Inventory sync received a Plan marker without a local body");
                    }
                }
                s
            });
        match *at {
            WireWindowSlot::Inv(i) if i < 36 => w.inv.set_slot(usize::from(i), stack),
            WireWindowSlot::Armour(i) if i < 4 => {
                w.armour[usize::from(i)] = stack.and_then(|s| {
                    if let Item::Armour(a) = s.item {
                        Some(a)
                    } else {
                        None
                    }
                })
            }
            WireWindowSlot::Cursor => *w.cursor = stack,
            WireWindowSlot::Grid(r, c) if r < 3 && c < 3 => {
                w.grid[usize::from(r)][usize::from(c)] = stack
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{inventory::Inventory, window::WindowMut};
    #[test]
    fn sync_keeps_preferences_and_resolves_builtin_plan_body() {
        let mut w = EffectiveWindow {
            inv: Inventory::new(),
            armour: [None; 4],
            cursor: None,
            grid: Default::default(),
        };
        crate::starter_kit::grant_fresh_kit(&mut w.inv, "testlab", false, false);
        let packet = packet(&w, 1);
        assert_eq!(packet.slots.len(), 50);
        let mut inv = Inventory::new();
        // A pre-existing body-less stub must not shadow the built-in body.
        inv.set_slot(
            15,
            Some(crate::item::ItemStack {
                item: Item::Plan(crate::plan::PlanData::marker_placeholder(
                    crate::plan::marker(&crate::satoshi::starter_hut_plan()),
                    true,
                )),
                count: 1,
            }),
        );
        inv.auto_refill = false;
        inv.toggle_lock(3);
        let mut armour = [None; 4];
        let mut cursor = None;
        let mut grid = Default::default();
        apply(
            &mut WindowMut {
                inv: &mut inv,
                armour: &mut armour,
                cursor: &mut cursor,
                grid: &mut grid,
                container: None,
            },
            &packet,
            &crate::block::BlockRegistry::new(),
            &mut crate::use_edits::SentUses::default(),
        );
        assert!(!inv.auto_refill);
        assert!(inv.is_locked(3));
        let original = (0..36)
            .find_map(|at| {
                w.inv.slot(at).and_then(|s| {
                    if let Item::Plan(p) = &s.item {
                        Some(p)
                    } else {
                        None
                    }
                })
            })
            .unwrap();
        let client = (0..36)
            .find_map(|at| {
                inv.slot(at).and_then(|s| {
                    if let Item::Plan(p) = &s.item {
                        Some(p)
                    } else {
                        None
                    }
                })
            })
            .unwrap();
        assert_eq!(crate::plan::marker(original), crate::plan::marker(client));
        assert!(!client.cells.is_empty());
    }
    #[test]
    fn replacement_overwrites_empty_slots_and_retains_local_plan_elsewhere() {
        let body = crate::plan::PlanData::debug_3x3_stone();
        let marker = crate::plan::marker(&body);
        let mut w = EffectiveWindow {
            inv: Inventory::new(),
            armour: [None; 4],
            cursor: None,
            grid: Default::default(),
        };
        w.inv.set_slot(
            2,
            Some(crate::item::ItemStack {
                item: Item::Plan(crate::plan::PlanData::marker_placeholder(marker, true)),
                count: 1,
            }),
        );
        let mut inv = Inventory::new();
        inv.set_slot(
            1,
            Some(crate::item::ItemStack {
                item: Item::Plan(body.clone()),
                count: 1,
            }),
        );
        let mut armour = [None; 4];
        let mut cursor = None;
        let mut grid = Default::default();
        apply(
            &mut WindowMut {
                inv: &mut inv,
                armour: &mut armour,
                cursor: &mut cursor,
                grid: &mut grid,
                container: None,
            },
            &packet(&w, 3),
            &crate::block::BlockRegistry::new(),
            &mut crate::use_edits::SentUses::default(),
        );
        assert!(inv.slot(1).is_none());
        if let Item::Plan(p) = &inv.slot(2).unwrap().item {
            assert_eq!(p.cells, body.cells);
        } else {
            panic!("Plan lost");
        }
    }
}

#[cfg(test)]
mod prediction_tests {
    use super::*;
    use crate::{
        inventory::Inventory,
        item::ItemStack,
        use_edits::{SentUses, UseKind, UseRecord},
        window::WindowMut,
    };

    #[test]
    fn a_late_refusal_after_sync_cannot_restore_a_pre_death_cost() {
        let mut uses = SentUses::default();
        uses.record(UseRecord {
            cell: [1, 80, 1],
            kind: UseKind::Sow.to_wire(),
            slot: 0,
            cost: Some(Item::Material(crate::item::MaterialId::WheatSeeds)),
            landed: None,
            made_at: 1,
        });
        let w = EffectiveWindow {
            inv: Inventory::new(),
            armour: [None; 4],
            cursor: None,
            grid: Default::default(),
        };
        let mut inv = Inventory::new();
        let mut ui = crate::craft_ui::CraftingUi::new();
        let mut armour = [None; 4];
        apply(
            &mut WindowMut {
                inv: &mut inv,
                armour: &mut armour,
                cursor: &mut ui.cursor_item,
                grid: &mut ui.grid,
                container: None,
            },
            &packet(&w, 3),
            &crate::block::BlockRegistry::new(),
            &mut uses,
        );
        // A later poll delivers the refusal, after the replacement was applied.
        let out = crate::use_edits::undo_refused(
            &mut uses,
            &mut inv,
            &mut ui,
            &[crate::protocol::RefusedUse {
                x: 1,
                y: 80,
                z: 1,
                kind: UseKind::Sow.to_wire(),
                note: 0,
            }],
        );
        assert!(out[0].1.is_none());
        assert!(inv.slots_iter().all(|s| s.is_none()));
    }

    #[test]
    fn sync_resolves_a_pending_or_parked_hang_body_without_reseating_other_costs() {
        for parked in [false, true] {
            let body = crate::plan::PlanData::debug_3x3_stone();
            let mut uses = SentUses::default();
            uses.record(UseRecord {
                cell: [1, 80, 1],
                kind: UseKind::HangPrint.to_wire(),
                slot: 0,
                cost: Some(Item::Plan(body.clone())),
                landed: None,
                made_at: 1,
            });
            let mut inv = Inventory::new();
            let mut ui = crate::craft_ui::CraftingUi::new();
            if parked {
                for at in 0..36 {
                    inv.set_slot(at, Some(ItemStack::new_block(crate::block::STONE, 64)));
                }
                ui.cursor_item = Some(ItemStack::new_block(crate::block::DIRT, 64));
                crate::use_edits::undo_refused(
                    &mut uses,
                    &mut inv,
                    &mut ui,
                    &[crate::protocol::RefusedUse {
                        x: 1,
                        y: 80,
                        z: 1,
                        kind: UseKind::HangPrint.to_wire(),
                        note: 0,
                    }],
                );
                assert_eq!(uses.parked_len(), 1);
            }
            uses.record(UseRecord {
                cell: [2, 80, 1],
                kind: UseKind::Sow.to_wire(),
                slot: 9,
                cost: Some(Item::Block(crate::block::STONE)),
                landed: None,
                made_at: 2,
            });
            let mut w = EffectiveWindow {
                inv: Inventory::new(),
                armour: [None; 4],
                cursor: None,
                grid: Default::default(),
            };
            w.inv.set_slot(
                2,
                Some(ItemStack {
                    item: Item::Plan(crate::plan::PlanData::marker_placeholder(
                        crate::plan::marker(&body),
                        true,
                    )),
                    count: 1,
                }),
            );
            let mut armour = [None; 4];
            apply(
                &mut WindowMut {
                    inv: &mut inv,
                    armour: &mut armour,
                    cursor: &mut ui.cursor_item,
                    grid: &mut ui.grid,
                    container: None,
                },
                &packet(&w, 3),
                &crate::block::BlockRegistry::new(),
                &mut uses,
            );
            let Item::Plan(recovered) = &inv.slot(2).unwrap().item else {
                panic!("Plan missing");
            };
            assert_eq!(recovered.cells, body.cells);
            assert!(recovered.marker.is_none());
            assert_eq!(uses.parked_len(), 0);
            assert_eq!(uses.len(), 0);
            assert_eq!(uses.reseat_parked(&mut inv), 0);
            assert_eq!(
                inv.slots_iter().flatten().count(),
                1,
                "only the authoritative Plan is seated"
            );
        }
    }
}
