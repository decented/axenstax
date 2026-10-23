//! Death ordering at pickup, Q-drop and post-respawn edit boundaries.
use super::joiner_authority::join_guest;
use super::joiner_death::{
    Inbox, send_input, send_respawn, stand_on_floor, start_dedicated_server, tick_n,
};
use crate::block;
use crate::item::{Item, ItemStack};
use crate::protocol::{self, PlayerEventType};
use crate::transport::ClientTransport;

#[test]
fn cursor_grid_and_unacknowledged_grants_cannot_fund_unlimited_raw_q_drops() {
    for source in ["cursor", "grid", "grant"] {
        let mut hs = start_dedicated_server(source);
        let (client, slot) = join_guest(&mut hs, "BoundedDropper");
        let stack = ItemStack::new_block(block::STONE, 1);
        match source {
            "cursor" => hs.server.players[slot].cursor = Some(stack.clone()),
            "grid" => hs.server.players[slot].craft_grid[0][0] = Some(stack.clone()),
            _ => hs.grant_to_joiner(slot, [stack.clone()]),
        }
        let now = hs.server.tick_counter;
        assert!(
            hs.server.players[slot]
                .container_sent
                .believed
                .try_take(crate::window_ops::BELIEVED_BUCKET_UNITS, now)
        );
        let (held_kind, held_id) = crate::inventory::item_to_ref(&stack.item).to_wire();
        for seq in 1..=8 {
            client.send_to_server(&protocol::serialize_packet(
                protocol::PacketType::ItemAction,
                &protocol::ItemActionPacket {
                    seq,
                    events_applied: 0,
                    action: protocol::ItemAction::Drop {
                        hotbar_slot: 0,
                        held_kind,
                        held_id,
                        held_full: protocol::WireItem::None,
                    },
                },
            ));
        }
        hs.tick();
        assert_eq!(
            hs.server.players[slot].possession.drops, 0,
            "{source} cannot bypass the raw take's believed bound"
        );
        assert!(hs.server.players[slot].possession.drop_refused > 0);
        assert_eq!(
            hs.server
                .ecs
                .query::<&crate::entity::ItemEntity>()
                .iter()
                .count(),
            0
        );
    }
}

#[test]
fn a_pickup_earned_before_a_fatal_hazard_is_in_the_grave_before_replace() {
    let mut hs = start_dedicated_server("pickup-hazard");
    let (client, slot) = join_guest(&mut hs, "LastPickup");
    let at = stand_on_floor(&mut hs, slot);
    let stack = ItemStack::new_block(block::GLASS, 7);
    crate::entity::spawn_item(&mut hs.server.ecs, at, stack, 0);
    for (_, item) in hs
        .server
        .ecs
        .query::<&mut crate::entity::ItemEntity>()
        .iter()
    {
        item.pickup_delay = 0;
    }
    hs.server
        .world
        .set_block(at.x as i32, at.y as i32, at.z as i32, block::LAVA);
    hs.server.players[slot].combat.health = 0.5;
    hs.server.tick_counter = crate::survival::CONTACT_HAZARD_PERIOD_TICKS - 1;
    let mut inbox = Inbox::default();
    hs.tick();
    inbox.drain(&client);
    assert!(hs.server.players[slot].combat.dead);
    let cell = inbox
        .events
        .iter()
        .find_map(|(_, event)| {
            if let PlayerEventType::GraveAt { x, y, z } = event {
                Some((*x, *y, *z))
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        hs.server.world.grave_at(cell).unwrap().remaining()[0].count,
        7
    );
    assert!((0..36).all(|at| {
        crate::window_events::effective_inventory(&hs.server.players[slot])
            .slot(at)
            .is_none()
    }));
    assert_eq!(inbox.syncs.len(), 1);
    // The join's Sync1 (1) and Personal2 (2) come first, then the pickup
    // grant (3): the replacement is the fourth window event.
    assert_eq!(
        inbox.syncs[0].window_event, 4,
        "pickup grant precedes replacement"
    );
}

#[test]
fn stale_post_respawn_placements_and_hang_uses_are_refused_and_undone() {
    let mut hs = start_dedicated_server("stale-edits");
    let (client, slot) = join_guest(&mut hs, "StaleBuilder");
    let at = stand_on_floor(&mut hs, slot);
    let stone = ItemStack::new_block(block::STONE, 6);
    let plan = ItemStack {
        item: Item::Plan(crate::plan::PlanData::marker_placeholder(
            crate::plan::marker(&crate::satoshi::starter_hut_plan()),
            true,
        )),
        count: 1,
    };
    hs.server.players[slot].inventory.set_slot(0, Some(stone));
    hs.server.players[slot]
        .inventory
        .set_slot(8, Some(plan.clone()));
    send_input(&client, 1, at, 0.0, 0.0, &[]);
    let mut inbox = Inbox::default();
    tick_n(&mut hs, &client, &mut inbox, 25);
    send_respawn(&client);
    hs.tick();
    inbox.drain(&client);
    assert!(!hs.server.players[slot].combat.dead);
    assert!(
        hs.server.players[slot]
            .window_events
            .death_replace
            .is_some()
    );
    hs.server.players[slot].player.pos = at;
    let x = at.x as i32 + 1;
    let y = at.y as i32;
    let z = at.z as i32;
    hs.server.world.set_block(x + 2, y, z, block::STONE);
    let input = protocol::InputPacket {
        tick: 2,
        x: at.x,
        y: at.y,
        z: at.z,
        health: 20.0,
        events_applied: 0,
        held_kind: protocol::item_kind::BLOCK,
        held_id: block::STONE,
        block_changes: vec![
            protocol::BlockChange {
                x,
                y,
                z,
                new_block: block::STONE,
                meta: 0,
            },
            protocol::BlockChange {
                x: x + 1,
                y,
                z,
                new_block: block::CYANOTYPE_PRINT,
                meta: 0,
            },
        ],
        use_tags: vec![protocol::UseTag {
            x: x + 1,
            y,
            z,
            kind: crate::use_edits::UseKind::HangPrint as u8,
            slot: 8,
            used: Some(crate::inventory::stack_to_wire(&plan)),
            ..Default::default()
        }],
        ..Default::default()
    };
    client.send_to_server(&protocol::serialize_packet(
        protocol::PacketType::ClientInput,
        &input,
    ));
    hs.tick();
    assert_eq!(hs.server.world.get_block(x, y, z), block::AIR);
    assert_eq!(hs.server.world.get_block(x + 1, y, z), block::AIR);
    assert_eq!(
        hs.server.players[slot].inventory.slot(0).unwrap().count,
        6,
        "raw pre-Replace inventory untouched"
    );
    assert!(hs.server.players[slot].inventory.slot(8).is_some());
    assert_eq!(hs.server.players[slot].possession.use_edit_refused, 1);
    let mut undone = false;
    while let Some(bytes) = client.try_recv_from_server() {
        if let Some((protocol::PacketType::StateUpdate, payload)) =
            protocol::deserialize_header(&bytes)
        {
            let state: protocol::StateUpdatePacket = protocol::safe_deserialize(payload).unwrap();
            undone |= state
                .refused_uses
                .iter()
                .any(|u| u.kind == crate::use_edits::UseKind::HangPrint as u8);
        }
    }
    assert!(undone, "the client's predicted use receives its undo");
}
