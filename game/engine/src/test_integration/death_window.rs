//! Server-owned death: effective-window conservation and shared grave requests.
use super::joiner_authority::join_guest;
use super::joiner_death::{
    Inbox, send_input, send_respawn, stand_on_floor, start_dedicated_server, tick_n,
};
use crate::block;
use crate::item::{Item, ItemStack, MaterialId};
use crate::protocol::{self, PlayerEventType};
use crate::transport::{ChannelClientTransport, ClientTransport};

#[test]
fn death_materialises_waiting_grants_once_and_refuses_stale_requests() {
    let mut hs = start_dedicated_server("whole-window");
    let (client, slot) = join_guest(&mut hs, "GraveTester");
    let at = stand_on_floor(&mut hs, slot);
    hs.server.players[slot]
        .inventory
        .set_slot(0, Some(ItemStack::new_block(block::STONE, 4)));
    hs.server.players[slot].inventory.set_slot(
        8,
        Some(ItemStack {
            item: Item::Plan(crate::satoshi::starter_hut_plan()),
            count: 1,
        }),
    );
    hs.server.players[slot].craft_grid[0][0] = Some(ItemStack::new_material(MaterialId::Bread, 2));
    hs.server.players[slot].cursor = Some(ItemStack::new_block(block::GLASS, 2));
    hs.server.players[slot].armour[0] = Some(crate::armour::ArmourItem::new(
        crate::armour::ArmourSlot::Helmet,
        crate::armour::ArmourMaterial::Iron,
    ));
    hs.grant_to_joiner(slot, [ItemStack::new_block(block::DIRT, 3)]);
    let mut inbox = Inbox::default();
    send_input(&client, 1, at, 0.0, 0.0, &[]);
    tick_n(&mut hs, &client, &mut inbox, 3);
    assert_eq!(inbox.died(slot), 1);
    assert_eq!(inbox.syncs.len(), 1);
    let cell = inbox
        .events
        .iter()
        .find_map(|(_, e)| {
            if let PlayerEventType::GraveAt { x, y, z } = e {
                Some((*x, *y, *z))
            } else {
                None
            }
        })
        .expect("shared grave location");
    let grave = hs.server.world.grave_at(cell).unwrap();
    assert_eq!(
        grave
            .remaining()
            .iter()
            .map(|s| u32::from(s.count))
            .sum::<u32>(),
        7
    );
    let eff = crate::window_events::effective_window(&hs.server.players[slot]);
    assert!(eff.inv.slot(0).is_none());
    assert!(eff.inv.slot(8).is_some());
    assert_eq!(eff.grid[0][0].as_ref().unwrap().count, 2);
    assert_eq!(eff.cursor.as_ref().unwrap().count, 2);
    assert!(eff.armour[0].is_some());
    let stone = Item::Block(block::STONE);
    let (held_kind, held_id) = crate::inventory::item_to_ref(&stone).to_wire();
    let drop = protocol::ItemActionPacket {
        seq: 1,
        events_applied: 0,
        action: protocol::ItemAction::Drop {
            hotbar_slot: 0,
            held_kind,
            held_id,
            held_full: crate::inventory::item_to_wire_full(&stone),
        },
    };
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ItemAction,
        &drop,
    ));
    hs.tick();
    assert_eq!(hs.server.players[slot].possession.drops, 0);
    tick_n(&mut hs, &client, &mut inbox, 25);
    send_respawn(&client);
    hs.tick();
    inbox.drain(&client);
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ItemAction,
        &drop,
    ));
    hs.tick();
    assert_eq!(
        hs.server.players[slot].possession.drops, 0,
        "stale request refused after respawn"
    );
    let open = protocol::ItemActionPacket {
        seq: 2,
        events_applied: inbox.syncs[0].window_event,
        action: protocol::ItemAction::OpenGrave {
            x: cell.0,
            y: cell.1,
            z: cell.2,
        },
    };
    hs.server.players[slot].player.pos = at;
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ItemAction,
        &open,
    ));
    hs.tick();
    inbox.drain(&client);
    assert!(hs.server.world.grave_at(cell).is_none());
    assert_eq!(
        hs.server.world.get_block(cell.0, cell.1, cell.2),
        block::AIR
    );
    assert_eq!(
        crate::window_events::effective_inventory(&hs.server.players[slot])
            .slot(0)
            .unwrap()
            .count,
        4
    );
}

#[test]
fn keep_inventory_sync_preserves_inventory_and_no_loaded_column_spills() {
    for keep in [true, false] {
        let mut hs = start_dedicated_server(if keep {
            "keep-window"
        } else {
            "unloaded-window"
        });
        let (client, slot) = join_guest(&mut hs, "Keeper");
        hs.server.world.keep_inventory = keep;
        hs.server.loaded_columns.clear();
        hs.server.players[slot]
            .inventory
            .set_slot(0, Some(ItemStack::new_block(block::STONE, 6)));
        hs.server.players[slot]
            .combat
            .die(crate::survival::DamageCause::Generic);
        let mut inbox = Inbox::default();
        hs.tick();
        inbox.drain(&client);
        assert_eq!(inbox.syncs.len(), 1);
        assert!(
            !inbox
                .events
                .iter()
                .any(|(_, e)| matches!(e, PlayerEventType::GraveAt { .. }))
        );
        let eff = crate::window_events::effective_inventory(&hs.server.players[slot]);
        assert_eq!(eff.slot(0).is_some(), keep);
        if !keep {
            assert!(
                hs.server
                    .ecs
                    .query::<&crate::entity::ItemEntity>()
                    .iter()
                    .any(|(_, item)| item.stack.count == 6)
            );
        }
    }
}

#[test]
fn q_drop_past_believed_bound_cannot_materialise_claimed_items() {
    let mut hs = start_dedicated_server("drop-bound");
    let (client, slot) = join_guest(&mut hs, "Dropper");
    assert!(hs.server.players[slot].container_sent.believed.try_take(
        crate::window_ops::BELIEVED_BUCKET_UNITS,
        hs.server.tick_counter
    ));
    let stone = Item::Block(block::STONE);
    let (held_kind, held_id) = crate::inventory::item_to_ref(&stone).to_wire();
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ItemAction,
        &protocol::ItemActionPacket {
            seq: 1,
            events_applied: 0,
            action: protocol::ItemAction::Drop {
                hotbar_slot: 0,
                held_kind,
                held_id,
                held_full: crate::inventory::item_to_wire_full(&stone),
            },
        },
    ));
    hs.tick();
    assert_eq!(hs.server.players[slot].possession.drops, 0);
    assert_eq!(hs.server.players[slot].possession.drop_refused, 1);
}

fn send_window(
    client: &ChannelClientTransport,
    op_seq: u32,
    events_applied: u32,
    op: protocol::WireWindowOp,
) {
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::WindowOp,
        &protocol::WindowOpPacket {
            op_seq,
            events_applied,
            op,
            digest: 0,
            touched: Vec::new(),
            claims: Vec::new(),
            client_ok: true,
        },
    ));
}

#[test]
fn a_post_hook_deposit_cannot_move_the_dead_window_into_a_shared_chest() {
    let mut hs = start_dedicated_server("dead-deposit");
    let (client, slot) = join_guest(&mut hs, "Depositor");
    let at = stand_on_floor(&mut hs, slot);
    let chest = [at.x as i32 + 1, at.y as i32, at.z as i32];
    hs.server
        .world
        .set_block(chest[0], chest[1], chest[2], block::CHEST);
    hs.server.players[slot]
        .inventory
        .set_slot(0, Some(ItemStack::new_block(block::STONE, 5)));
    send_window(
        &client,
        1,
        0,
        protocol::WireWindowOp::OpenContainer { cell: chest },
    );
    hs.tick();
    assert_eq!(hs.server.players[slot].open_container, Some(chest));
    let mut inbox = Inbox::default();
    inbox.drain(&client);
    hs.server.players[slot]
        .combat
        .die(crate::survival::DamageCause::Generic);
    hs.tick();
    inbox.drain(&client);
    assert!(hs.server.players[slot].open_container.is_none());
    // Also try reopening before the stale deposit: the death guard applies to both.
    send_window(
        &client,
        2,
        0,
        protocol::WireWindowOp::OpenContainer { cell: chest },
    );
    send_window(
        &client,
        3,
        0,
        protocol::WireWindowOp::Container(crate::container_window::ContainerClick::Deposit {
            slot: 0,
            all: true,
        }),
    );
    hs.tick();
    assert!(
        hs.server
            .world
            .chest_at((chest[0], chest[1], chest[2]))
            .unwrap()
            .slots
            .iter()
            .all(Option::is_none)
    );
    let cell = inbox
        .events
        .iter()
        .find_map(|(_, e)| {
            if let PlayerEventType::GraveAt { x, y, z } = e {
                Some((*x, *y, *z))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        hs.server.world.grave_at(cell).unwrap().remaining()[0].count,
        5
    );
}

#[test]
fn another_joiner_can_partially_retrieve_a_grave_and_unfit_returns_to_it() {
    let mut hs = start_dedicated_server("partial-retrieval");
    let (owner, slot) = join_guest(&mut hs, "Owner");
    let (other, borrower) = join_guest(&mut hs, "Borrower");
    let at = stand_on_floor(&mut hs, slot);
    hs.server.players[borrower].player.pos = at;
    hs.server.players[slot]
        .inventory
        .set_slot(0, Some(ItemStack::new_block(block::STONE, 8)));
    hs.server.players[slot]
        .inventory
        .set_slot(1, Some(ItemStack::new_block(block::DIRT, 9)));
    hs.server.players[slot]
        .combat
        .die(crate::survival::DamageCause::Generic);
    let mut inbox = Inbox::default();
    hs.tick();
    inbox.drain(&owner);
    let cell = inbox
        .events
        .iter()
        .find_map(|(_, e)| {
            if let PlayerEventType::GraveAt { x, y, z } = e {
                Some((*x, *y, *z))
            } else {
                None
            }
        })
        .unwrap();
    for i in 0..36 {
        hs.server.players[borrower]
            .inventory
            .set_slot(i, Some(ItemStack::new_block(block::STONE, 64)));
    }
    hs.server.players[borrower]
        .inventory
        .set_slot(0, Some(ItemStack::new_block(block::STONE, 60)));
    let req = protocol::ItemActionPacket {
        seq: 1,
        events_applied: 0,
        action: protocol::ItemAction::OpenGrave {
            x: cell.0,
            y: cell.1,
            z: cell.2,
        },
    };
    other.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ItemAction,
        &req,
    ));
    hs.tick();
    assert_eq!(
        hs.server.world.grave_at(cell).unwrap().slots[0]
            .as_ref()
            .unwrap()
            .count,
        4
    );
    assert_eq!(
        hs.server.world.grave_at(cell).unwrap().slots[1]
            .as_ref()
            .unwrap()
            .count,
        9
    );
    let mut event = None;
    let mut partial = false;
    while let Some(pkt) = other.try_recv_from_server() {
        let Some((tag, bytes)) = protocol::deserialize_header(&pkt) else {
            continue;
        };
        if tag == protocol::PacketType::InventoryGrant {
            event = Some(
                protocol::safe_deserialize::<protocol::InventoryGrantPacket>(bytes)
                    .unwrap()
                    .window_event,
            );
        }
        if tag == protocol::PacketType::ItemActionOutcome {
            let out: protocol::ItemActionOutcomePacket = protocol::safe_deserialize(bytes).unwrap();
            partial =
                out.accepted && out.note == crate::item_actions::ItemNote::InventoryFull.to_wire();
        }
    }
    assert!(partial);
    let event = event.unwrap();
    let req = protocol::ItemActionPacket {
        seq: 2,
        events_applied: event,
        action: protocol::ItemAction::GrantUnfit { event, count: 4 },
    };
    other.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ItemAction,
        &req,
    ));
    hs.tick();
    assert_eq!(
        hs.server.world.grave_at(cell).unwrap().slots[0]
            .as_ref()
            .unwrap()
            .count,
        8,
        "unfit returns to the surviving grave"
    );
}

#[test]
fn hazard_starvation_and_reported_delta_each_make_one_grave_and_sync() {
    for kind in ["hazard", "starvation", "delta"] {
        let mut hs = start_dedicated_server(kind);
        let (client, slot) = join_guest(&mut hs, "AllPaths");
        let at = stand_on_floor(&mut hs, slot);
        hs.server.players[slot]
            .inventory
            .set_slot(0, Some(ItemStack::new_block(block::STONE, 3)));
        hs.server.players[slot].combat.health = 0.5;
        hs.server.tick_counter = crate::survival::CONTACT_HAZARD_PERIOD_TICKS - 1;
        match kind {
            "hazard" => {
                hs.server
                    .world
                    .set_block(at.x as i32, at.y as i32, at.z as i32, block::LAVA)
            }
            "starvation" => {
                hs.server.difficulty = crate::survival::Difficulty::Hard;
                hs.server.players[slot].combat.hunger = 0;
                hs.server.players[slot].combat.starvation_ticks =
                    crate::combat::STARVATION_INTERVAL_TICKS - 1;
            }
            _ => {
                let input = protocol::InputPacket {
                    tick: 1,
                    x: at.x,
                    y: at.y,
                    z: at.z,
                    health: 20.0,
                    health_delta: -1.0,
                    ..Default::default()
                };
                client.send_to_server(&protocol::serialize_packet(
                    protocol::PacketType::ClientInput,
                    &input,
                ));
            }
        }
        let mut inbox = Inbox::default();
        tick_n(&mut hs, &client, &mut inbox, 3);
        assert!(
            hs.server.players[slot].combat.dead,
            "{kind} killed the body"
        );
        assert_eq!(inbox.died(slot), 1, "{kind} announced once");
        assert_eq!(inbox.syncs.len(), 1, "{kind} synchronised once");
        assert_eq!(
            inbox
                .events
                .iter()
                .filter(|(_, e)| matches!(e, PlayerEventType::GraveAt { .. }))
                .count(),
            1,
            "{kind} placed one grave"
        );
    }
}

#[test]
fn testlab_kit_is_server_owned_at_join_and_creative_cannot_report_death() {
    let mut hs = start_dedicated_server("kit");
    hs.server.world.world_type = "testlab".to_string();
    let (client, slot) = join_guest(&mut hs, "Tester");
    let effective = crate::window_events::effective_window(&hs.server.players[slot]);
    assert_eq!(effective.inv.slot(0).unwrap().count, 64);
    let plan = (0..36)
        .find_map(|at| {
            effective.inv.slot(at).and_then(|s| {
                if let Item::Plan(p) = &s.item {
                    Some(p)
                } else {
                    None
                }
            })
        })
        .unwrap();
    assert_eq!(
        crate::plan::marker(plan),
        crate::plan::marker(&crate::satoshi::starter_hut_plan())
    );
    assert!(plan.marker.is_some(), "server holds only the marker");
    let _ = client; // join_guest drains the join's wire packets.
    hs.server.play_mode = crate::play_mode::PlayMode::Creative;
    hs.server.players[slot].combat.health = 20.0;
    assert!(!hs.server.report_player_death(slot));
    hs.tick();
    assert!(!hs.server.players[slot].combat.dead);
    assert!(
        hs.server.players[slot]
            .window_events
            .death_replace
            .is_none()
    );
}

#[test]
fn a_testlab_death_graves_the_unacknowledged_kit_and_keeps_its_plan() {
    let mut hs = start_dedicated_server("kit-death");
    hs.server.world.world_type = "testlab".to_string();
    // The kit's Sync1 must stay unacknowledged: the client installs nothing.
    let (client, slot) = super::joiner_authority::join_guest_uninstalled(&mut hs, "KitGrave");
    let at = stand_on_floor(&mut hs, slot);
    let before = crate::window_events::effective_window(&hs.server.players[slot]);
    let expected: u32 = before
        .inv
        .slots_iter()
        .flatten()
        .filter(|s| !matches!(s.item, Item::Plan(_)))
        .map(|s| u32::from(s.count))
        .sum();
    assert!(expected > 640, "blocks, tools and food in the shared kit");
    assert!(
        hs.server.players[slot]
            .inventory
            .slots_iter()
            .all(|s| s.is_none()),
        "the join kit still waits for its acknowledgement"
    );
    send_input(&client, 1, at, 0.0, 0.0, &[]);
    let mut inbox = Inbox::default();
    tick_n(&mut hs, &client, &mut inbox, 3);
    let cell = inbox
        .events
        .iter()
        .find_map(|(_, e)| match e {
            PlayerEventType::GraveAt { x, y, z } => Some((*x, *y, *z)),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        hs.server
            .world
            .grave_at(cell)
            .unwrap()
            .remaining()
            .iter()
            .map(|s| u32::from(s.count))
            .sum::<u32>(),
        expected
    );
    let after = crate::window_events::effective_window(&hs.server.players[slot]);
    let retained: Vec<_> = after.inv.slots_iter().flatten().collect();
    assert_eq!(retained.len(), 1);
    assert!(matches!(&retained[0].item, Item::Plan(p)
        if crate::plan::marker(p) == crate::plan::marker(&crate::satoshi::starter_hut_plan())));
    assert_eq!(inbox.syncs.len(), 1);
    // The join's kit Sync1 (1) and its Personal2 (2), then the death's Replace.
    assert_eq!(
        inbox.syncs[0].window_event, 3,
        "death follows the kit Replace"
    );
}

#[test]
fn a_window_op_before_the_death_hook_uses_the_old_window_then_replace_follows() {
    let mut hs = start_dedicated_server("before-hook-op");
    let (client, slot) = join_guest(&mut hs, "LastClick");
    let at = stand_on_floor(&mut hs, slot);
    hs.server.players[slot]
        .inventory
        .set_slot(0, Some(ItemStack::new_block(block::STONE, 5)));
    send_window(
        &client,
        1,
        0,
        protocol::WireWindowOp::Click(crate::window::WindowClick::Slot {
            slot: 0,
            right: false,
        }),
    );
    send_input(&client, 1, at, 0.0, 0.0, &[]);
    let mut inbox = Inbox::default();
    hs.tick();
    inbox.drain(&client);
    assert_eq!(inbox.syncs.len(), 1);
    let eff = crate::window_events::effective_window(&hs.server.players[slot]);
    assert!(eff.inv.slot(0).is_none());
    assert_eq!(
        eff.cursor.as_ref().unwrap().count,
        5,
        "the earlier op moved it to the surviving cursor"
    );
    assert!(
        !inbox
            .events
            .iter()
            .any(|(_, e)| matches!(e, PlayerEventType::GraveAt { .. })),
        "nothing remained in the 36 slots"
    );
}
