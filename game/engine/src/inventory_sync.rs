//! Whole-window wire conversion and client application without replacing preferences.
//! OC (v88) — the window's correction debt travels with it: the packet
//! carries the debt the server's `Replace` gives its copy, and the client's
//! apply installs exactly that debt on its own copy.
use crate::{
    item::Item,
    protocol::{InventorySyncPacket, WireWindowSlot},
    window_events::EffectiveWindow,
};
pub fn packet(
    w: &EffectiveWindow,
    debt: &crate::container_window::CorrectionDebt,
    window_event: u32,
) -> InventorySyncPacket {
    let slots = crate::container_window::PlayerSlots::of(&w.inv, &w.armour, &w.cursor, &w.grid);
    let ats = (0..36)
        .map(WireWindowSlot::Inv)
        .chain((0..4).map(WireWindowSlot::Armour))
        .chain(std::iter::once(WireWindowSlot::Cursor))
        .chain((0..3).flat_map(|r| (0..3).map(move |c| WireWindowSlot::Grid(r, c))));
    InventorySyncPacket {
        window_event,
        slots: ats.map(|at| (at, slots.wire(at).unwrap())).collect(),
        owed: debt.to_wire(),
        station: None,
        locks: w.inv.locked_indices().iter().fold(0u64, |m, &i| m | (1u64 << i)),
    }
}

/// C3d run 6 — the station a replacement carries, as the window sees it.
pub fn station(pkt: &InventorySyncPacket) -> crate::window::Station {
    match pkt.station {
        Some(cell) => crate::window::Station::Table { cell },
        None => crate::window::Station::Player,
    }
}

/// C3d run 6 — the station's wire form ([`InventorySyncPacket::station`]).
pub fn station_wire(station: crate::window::Station) -> Option<[i32; 3]> {
    match station {
        crate::window::Station::Table { cell } => Some(cell),
        crate::window::Station::Player => None,
    }
}
/// Apply a whole-window replacement to the client's window `w`, and install
/// the correction debt it carries as the client's `debt` (OC, v88: the same
/// debt the server's copy now owes; empty after a death). An owed list the
/// client can't decode whole (`CorrectionDebt::from_wire`) is a protocol
/// error: nothing is applied, neither the window nor the debt, and the
/// caller leaves the world.
pub fn apply(
    w: &mut crate::window::WindowMut,
    pkt: &InventorySyncPacket,
    reg: &crate::block::BlockRegistry,
    uses: &mut crate::use_edits::SentUses,
    debt: &mut crate::container_window::CorrectionDebt,
) -> Result<(), String> {
    let carried = crate::container_window::CorrectionDebt::from_wire(&pkt.owed, reg)?;
    if pkt.locks >> 36 != 0 { return Err("Inventory sync locks a slot that doesn't exist".into()); }
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
    for (_, body) in &plans { crate::plan_store::remember(body); }
    for body in &predicted_plans { crate::plan_store::remember(body); }
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
                    } else if let Some(body) = crate::plan_store::resolve(marker) {
                        s.item = Item::Plan(body);
                    } else {
                        log::warn!("Inventory sync received a Plan marker without a local body");
                    }
                }
                crate::plan_store::remember_item(&s.item);
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
    *debt = carried;
    // C3d run 6 — the lock mask is the replacement's (the digest covers it).
    let locked: Vec<u32> = (0..36u32).filter(|i| pkt.locks & (1u64 << i) != 0).collect();
    w.inv.set_locked_from(&locked);
    Ok(())
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
        w.inv.toggle_lock(5);
        let packet = packet(&w, &Default::default(), 1);
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
            &mut Default::default(),
        ).unwrap();
        assert!(!inv.auto_refill, "auto-refill stays the client's own");
        assert!(!inv.is_locked(3) && inv.is_locked(5), "C3d run 6: the lock mask is the replacement's");
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
            &packet(&w, &Default::default(), 3),
            &crate::block::BlockRegistry::new(),
            &mut crate::use_edits::SentUses::default(),
            &mut Default::default(),
        ).unwrap();
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
            &packet(&w, &Default::default(), 3),
            &crate::block::BlockRegistry::new(),
            &mut uses,
            &mut Default::default(),
        ).unwrap();
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
                &packet(&w, &Default::default(), 3),
                &crate::block::BlockRegistry::new(),
                &mut uses,
                &mut Default::default(),
            ).unwrap();
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

/// OC (v88) — the client's half of the carried debt.
#[cfg(test)]
mod oc_tests {
    use super::*;
    use crate::{
        container_window::CorrectionDebt,
        inventory::Inventory,
        item::ItemStack,
        protocol::{WireItem, WireStack, item_kind},
        window::WindowMut,
    };

    fn stone() -> Item {
        Item::Block(crate::block::STONE)
    }

    fn sync_with(owed: Vec<(WireStack, u32)>) -> InventorySyncPacket {
        let mut w = EffectiveWindow { inv: Inventory::new(), armour: [None; 4], cursor: None, grid: Default::default() };
        w.inv.set_slot(4, Some(ItemStack::new_block(crate::block::DIRT, 9)));
        InventorySyncPacket { owed, ..packet(&w, &Default::default(), 2) }
    }

    /// Apply `pkt` to a client window holding Stone 3 in slot 0, whose debt
    /// owes Dirt 5: what the window and the debt are after.
    fn apply_to_client(pkt: &InventorySyncPacket) -> (Result<(), String>, Inventory, CorrectionDebt) {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 3)));
        let mut debt = CorrectionDebt::from_entries(vec![(Item::Block(crate::block::DIRT), 5)]).unwrap();
        let (mut armour, mut cursor, mut grid) = ([None; 4], None, Default::default());
        let out = apply(
            &mut WindowMut { inv: &mut inv, armour: &mut armour, cursor: &mut cursor, grid: &mut grid, container: None },
            pkt,
            &crate::block::BlockRegistry::new(),
            &mut crate::use_edits::SentUses::default(),
            &mut debt,
        );
        (out, inv, debt)
    }

    #[test]
    fn sync_installs_exactly_the_debt_it_carries() {
        let carried = CorrectionDebt::from_entries(vec![(stone(), 70_000)]).unwrap();
        let (out, inv, debt) = apply_to_client(&sync_with(carried.to_wire()));
        assert!(out.is_ok());
        assert_eq!(debt, carried, "the carried debt replaces the old one");
        assert!(inv.slot(0).is_none() && inv.slot(4).is_some(), "and the window is replaced");
        let (_, _, debt) = apply_to_client(&sync_with(Vec::new()));
        assert!(debt.is_empty(), "an empty owed list (a death's) clears it");
    }

    /// A6 — an owed entry the client can't decode, or two that decode to
    /// the same item, make the whole Sync a protocol error: neither the
    /// window nor the debt is touched.
    #[test]
    fn an_owed_list_the_client_cant_decode_whole_applies_nothing() {
        let unknown_block = WireStack { item_kind: item_kind::BLOCK, item_id: u16::MAX, count: 1, full_item: WireItem::None };
        let pick = WireItem::Tool { tool_type: 0, material: 2, durability: 9 };
        let pick_a = WireStack { item_kind: item_kind::TOOL, item_id: 2, count: 1, full_item: pick };
        let pick_b = WireStack { item_kind: item_kind::BLOCK, item_id: 1, count: 1, full_item: pick };
        let broken = WireStack { full_item: WireItem::Tool { tool_type: 0, material: 2, durability: 0 }, ..pick_a.clone() };
        for (why, owed) in [
            ("an unknown block", vec![(unknown_block, 1)]),
            ("a tool at durability 0", vec![(broken, 1)]),
            ("two entries that decode to one item", vec![(pick_a, 1), (pick_b, 1)]),
        ] {
            let (out, inv, debt) = apply_to_client(&sync_with(owed));
            assert!(out.is_err(), "{why}: a protocol error");
            assert_eq!(inv.slot(0).map(|s| s.count), Some(3), "{why}: the window is untouched");
            assert!(inv.slot(4).is_none(), "{why}: the window is untouched");
            assert_eq!(debt.owed(&Item::Block(crate::block::DIRT)), 5, "{why}: the debt is untouched");
        }
    }
}
