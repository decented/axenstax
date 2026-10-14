//! Current live cargo, physical reach and death at real dispatch boundaries.
use super::*;

fn ground(rig: &Rig, block: u16) -> u32 {
    rig.hs.server.ecs.query::<&crate::entity::ItemEntity>().iter()
        .filter(|(_, drop)| drop.stack.item == Item::Block(block))
        .map(|(_, drop)| u32::from(drop.stack.count)).sum()
}

fn graves(rig: &Rig, block: u16) -> u32 {
    rig.hs.server.world.iter_graves().flat_map(|(_, grave)| grave.slots.iter().flatten())
        .filter(|stack| stack.item == Item::Block(block))
        .map(|stack| u32::from(stack.count)).sum()
}

#[test]
fn d2c2_empty_remove_and_deposit_in_each_execution_order_conserve_both_modes() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for remove_first in [false, true] {
            let mut rig = Rig::new(&format!("empty-race-{remove_first}"), mode);
            rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(crate::chest::ChestData::new());
            rig.hs.server.players[rig.joiner.i].inventory.set_slot(1, Some(ItemStack::new_block(block::STONE, 3)));
            let (_, opened) = rig.execute(PackOperation::Open);
            rig.apply(&opened);
            let old = rig.session();
            let remove = PackOperation::Remove { incarnation: old.incarnation, revision: old.revision };
            let deposit = PackOperation::Click { incarnation: old.incarnation, revision: old.revision, click: ContainerClick::Deposit { slot: 1, all: true } };
            let (_, first) = rig.execute(if remove_first { remove.clone() } else { deposit.clone() });
            assert_eq!(first.note, PackNote::Done);
            rig.apply(&first);
            // The competing request still names the exact old view. It must
            // never silently refresh its authority to the new cargo.
            let (packet, second) = rig.execute(if remove_first { deposit } else { remove });
            assert_ne!(second.note, PackNote::Done);
            assert_eq!(rig.send(&packet), second, "a repeated terminal cannot change either stock");
            rig.apply(&second);
            let cargo = rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.clone();
            let cargo_stone = cargo.as_ref().map_or(0, |pack| pack.slots.iter().flatten().filter(|stack| stack.item == Item::Block(block::STONE)).map(|stack| u32::from(stack.count)).sum::<u32>());
            assert_eq!(rig.bag(block::STONE) + cargo_stone + ground(&rig, block::STONE), 3);
            assert_eq!(rig.bag(block::CHEST) + ground(&rig, block::CHEST) + u32::from(cargo.is_some()), 1);
            assert_eq!(cargo.is_none(), remove_first);
            assert_eq!(rig.bag(block::STONE), if remove_first { 3 } else { 0 });
            assert!(!rig.hs.disconnected[rig.joiner.i]);
        }
    }
}

#[test]
fn d2c2_open_pack_follows_live_reach_then_closes_and_refuses_moved_cargo() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("live-reach", mode);
        let mut pack = crate::chest::ChestData::new();
        pack.slots[0] = Some(ItemStack::new_block(block::STONE, 4));
        rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(pack);
        let (_, opened) = rig.execute(PackOperation::Open);
        rig.apply(&opened);
        let old = rig.session();
        let nearby = rig.hs.server.ecs.get::<&crate::entity::Position>(rig.entity).unwrap().0 + glam::Vec3::X * 0.25;
        rig.hs.server.ecs.get::<&mut crate::entity::Position>(rig.entity).unwrap().0 = nearby;
        rig.hs.tick();
        assert_eq!(rig.session().entity, old.entity, "nearby movement preserves the same generational session");
        let stale = rig.packet(PackOperation::Click { incarnation: old.incarnation, revision: old.revision, click: ContainerClick::TakeAll });
        // Movement happens after the request was formed, before execution.
        rig.hs.server.ecs.get::<&mut crate::entity::Position>(rig.entity).unwrap().0 = nearby + glam::Vec3::X * 20.0;
        let terminal = rig.send(&stale);
        assert_eq!(terminal.note, PackNote::OutOfReach);
        assert!(terminal.view.is_none());
        assert!(rig.hs.server.players[rig.joiner.i].pack_session.is_none());
        assert_eq!(rig.contents().slots[0].as_ref().unwrap().count, 4);
        assert_eq!(rig.bag(block::STONE), 0);
        rig.apply(&terminal);
        assert!(!rig.hs.disconnected[rig.joiner.i]);
    }
}

#[test]
fn d2c2_real_entity_death_before_or_after_take_and_deposit_drains_once() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        for deposit in [false, true] {
            for death_first in [false, true] {
                let mut rig = Rig::new(&format!("cargo-death-{deposit}-{death_first}"), mode);
                let mut pack = crate::chest::ChestData::new();
                pack.slots[0] = Some(ItemStack::new_block(block::STONE, 4));
                rig.hs.server.ecs.get::<&mut crate::horse_ai::HorseData>(rig.entity).unwrap().pack = Some(pack);
                if deposit { rig.hs.server.players[rig.joiner.i].inventory.set_slot(1, Some(ItemStack::new_block(block::STONE, 3))); }
                let (_, opened) = rig.execute(PackOperation::Open);
                rig.apply(&opened);
                let old = rig.session();
                let request = rig.packet(PackOperation::Click { incarnation: old.incarnation, revision: old.revision, click: if deposit { ContainerClick::Deposit { slot: 1, all: true } } else { ContainerClick::TakeAll } });
                if death_first { rig.hs.server.ecs.get::<&mut crate::combat::Health>(rig.entity).unwrap().current = 0.0; }
                let terminal = rig.send(&request);
                assert_eq!(terminal.note == PackNote::Done, !death_first, "actual dispatch honours the live death boundary");
                if !death_first { rig.hs.server.ecs.get::<&mut crate::combat::Health>(rig.entity).unwrap().current = 0.0; }
                // This invokes the actual GameServer death sweep, not a
                // fixture despawn or a fabricated cargo spill.
                rig.hs.tick();
                assert!(!rig.hs.server.ecs.contains(rig.entity));
                assert!(rig.hs.server.players[rig.joiner.i].pack_session.is_none());
                let expected = if deposit { 7 } else { 4 };
                assert_eq!(rig.bag(block::STONE) + ground(&rig, block::STONE), expected);
                assert_eq!(rig.bag(block::CHEST) + ground(&rig, block::CHEST), 1);
                assert_eq!(rig.send(&request), terminal, "duplicate delivery after death cannot replay the cargo mutation");
                rig.apply(&terminal);
                for _ in 0..3 { rig.hs.tick(); }
                assert_eq!(rig.bag(block::STONE) + ground(&rig, block::STONE), expected, "later sweeps cannot drain the same dead cargo twice");
                assert_eq!(rig.bag(block::CHEST) + ground(&rig, block::CHEST), 1);
                assert!(!rig.hs.disconnected[rig.joiner.i]);
            }
        }
    }
}

#[test]
fn d2c2_queued_pack_crossing_actual_respawn_never_uses_the_new_body() {
    for mode in [InventoryMode::Mirror, InventoryMode::Strict] {
        let mut rig = Rig::new("actual-respawn", mode);
        rig.hs.server.players[rig.joiner.i].inventory.set_slot(0, Some(ItemStack::new_block(block::CHEST, 1)));
        let old = rig.packet(PackOperation::Equip);
        assert!(matches!(&old.intent, ActionIntent::Pack(request) if request.actor_respawns == 0));
        // Select the genuine grave branch at the loaded death cell. The
        // Chest's custody is the grave, not an assumed floor scatter.
        let pos = rig.hs.server.players[rig.joiner.i].player.pos;
        let cell = (pos.x.floor() as i32, pos.y.floor() as i32, pos.z.floor() as i32);
        assert!(rig.hs.server.loaded_columns.contains(&(cell.0.div_euclid(16), cell.2.div_euclid(16))));
        rig.hs.server.world.set_block(cell.0, cell.1, cell.2, block::AIR);
        let sp = &mut rig.hs.server.players[rig.joiner.i];
        sp.combat.dead = true;
        sp.combat.health = 0.0;
        sp.death_handled = false;
        rig.hs.handle_joiner_deaths();
        assert!(rig.hs.server.players[rig.joiner.i].window_events.last_number() > old.action.events_applied);
        assert_eq!(graves(&rig, block::CHEST), 1);
        assert_eq!(ground(&rig, block::CHEST), 0);
        // Keep the formed request outside the transport throughout the real
        // death-screen delay. Its first arrival will be after the new life.
        for _ in 0..crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN { rig.hs.tick(); }
        let sp = &rig.hs.server.players[rig.joiner.i];
        assert!(sp.combat.dead && sp.dead_ticks >= crate::server::MIN_DEAD_TICKS_BEFORE_RESPAWN);
        assert!(sp.action_queue.as_ref().unwrap().is_empty());
        assert!(sp.action_results.as_ref().unwrap().peek(old.action).is_none(), "the delayed old request has never been consumed");
        rig.joiner.client.send_to_server(&protocol::serialize_packet(protocol::PacketType::Respawn, &()));
        rig.hs.tick();
        let sp = &rig.hs.server.players[rig.joiner.i];
        assert!(sp.is_present_and_alive());
        assert_eq!(sp.respawns, 1, "the actual Respawn request advances authoritative physical life");
        let before = crate::window_events::pack_canonical_window(sp, &rig.hs.server.registry).unwrap();
        let before = crate::pack_wire::PackWindow::capture(&before.window, &before.debt, &rig.hs.server.registry).unwrap();
        let terminal = rig.send(&old);
        assert_eq!(terminal.note, PackNote::Dead);
        assert_eq!(terminal.fate, crate::action_identity::ActionFate::Superseded);
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.is_none());
        let after = crate::window_events::pack_canonical_window(&rig.hs.server.players[rig.joiner.i], &rig.hs.server.registry).unwrap();
        assert_eq!(crate::pack_wire::PackWindow::capture(&after.window, &after.debt, &rig.hs.server.registry).unwrap(), before, "the old request leaves every new-life personal place and debt unchanged");
        assert_eq!(rig.bag(block::CHEST) + ground(&rig, block::CHEST) + graves(&rig, block::CHEST), 1);
        assert_eq!(graves(&rig, block::CHEST), 1, "the original Chest stays in its actual grave");
        assert_eq!(rig.send(&old), terminal);
        rig.apply(&terminal);
        assert_eq!(rig.bag(block::CHEST) + ground(&rig, block::CHEST) + graves(&rig, block::CHEST), 1, "repeat and exact ACK cannot refund the dead body's Chest");
        assert!(rig.hs.server.ecs.get::<&crate::horse_ai::HorseData>(rig.entity).unwrap().pack.is_none());
        assert!(!rig.hs.disconnected[rig.joiner.i]);
    }
}
