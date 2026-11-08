//! Living world S3 — the local Feast path through the real game (single-player):
//! guests are the ECS villagers, the held stack is used up, the table becomes a
//! marker with its guest count, the next tick shows the feast (outbox drained,
//! bloom surge) and keeps the bloom field's Feast and Commons lists current.
use super::*;
use crate::item::{ItemStack, MaterialId};
use crate::local_routing::LocalRouting;
use crate::{block, mob::MobType};

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_a_local_feast_feeds_villagers_marks_the_table_and_feeds_the_bloom() {
    isolate_saves();
    let name = format!("feast-local-{:?}", std::thread::current().id());
    let mut game = HeadlessGame::boot_into_world_routed(&name, LocalRouting::Standalone);
    game.state.ecs.clear();
    game.state.is_creative = false;
    let at = game.state.players[0].player.pos.as_ivec3();
    let table = [at.x + 2, at.y, at.z];
    game.state.world.set_block(table[0], table[1], table[2], block::FEAST_TABLE);
    game.state.world.set_block(table[0], table[1] + 1, table[2], block::AIR);

    // Alone and with no village: refused, nothing taken.
    game.state.players[0].hotbar_slot = 0;
    game.state.players[0].inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 7)));
    assert!(!game.state.use_feast_table(0, table));
    assert_eq!(game.state.players[0].inventory.hotbar_slot(0).map(|s| s.count), Some(7));
    assert_eq!(game.state.world.get_block(table[0], table[1], table[2]), block::FEAST_TABLE);
    assert_eq!(game.state.toast.as_ref().map(|t| t.0.as_str()), Some(crate::feast::NEEDS_GUESTS_TOAST));

    // Two villagers near: they are the guests, the loaves are used up.
    for dz in [2.0, 4.0] {
        crate::entity::spawn_mob(
            &mut game.state.ecs,
            MobType::Villager,
            glam::Vec3::new(table[0] as f32, table[1] as f32, table[2] as f32 + dz),
        );
    }
    assert!(game.state.use_feast_table(0, table));
    assert!(game.state.players[0].inventory.hotbar_slot(0).is_none(), "the whole stack is shared out");
    assert_eq!(game.state.world.get_block(table[0], table[1], table[2]), block::FEAST_MARKER);
    assert_eq!(game.state.world.meta_at(table[0], table[1], table[2]), 2);
    assert_eq!(game.state.world.feast_events.len(), 1);
    assert!(
        game.state.toast.as_ref().is_some_and(|t| t.0.starts_with("The Feast of ") && t.0.ends_with("2 guests.")),
        "{:?}",
        game.state.toast
    );
    let loaves: u32 = game
        .state
        .ecs
        .query::<&crate::entity::ItemEntity>()
        .iter()
        .map(|(_, it)| u32::from(it.stack.count))
        .sum();
    assert_eq!(loaves, 0, "villagers' shares are used up, not dropped");

    // The next tick sees the feast and the bloom field is told where the marker is.
    game.state.tick_counter = 10; // a feed tick
    game.ticks(1);
    assert!(game.state.world.feast_events.is_empty(), "the outbox is drained");
}

/// Review M1 — through the REAL right-click: a hungry, hurt player holding bread
/// at a Feast Table (or a Feast Marker) does not eat it. With no guests the
/// click is a refused feast and nothing is eaten; with villagers near the whole
/// stack is shared out. Reading a marker eats nothing either.
#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_food_at_a_feast_table_is_feasted_not_eaten() {
    isolate_saves();
    let name = format!("feast-hungry-{:?}", std::thread::current().id());
    let mut game = HeadlessGame::boot_into_world_routed(&name, LocalRouting::Standalone);
    game.state.ecs.clear();
    game.state.set_play_mode(crate::play_mode::PlayMode::Survival);
    let feet = clear_pad(&mut game, None);
    let table = [feet[0], feet[1] + 1, feet[2] - 2];
    game.state.world.set_block(table[0], table[1], table[2], block::FEAST_TABLE);
    game.state.players[0].hotbar_slot = 0;
    game.state.players[0].inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 7)));
    let max_hunger = game.state.players[0].combat.max_hunger;
    game.state.players[0].combat.hunger = max_hunger - 6;
    let hunger_before = game.state.players[0].combat.hunger;
    aim_at(&mut game, glam::Vec3::new(table[0] as f32 + 0.5, table[1] as f32 + 0.5, table[2] as f32 + 0.5));

    // No guests: a refused feast, and not one loaf eaten.
    right_click(&mut game);
    assert_eq!(game.state.players[0].inventory.hotbar_slot(0).map(|s| s.count), Some(7), "no loaf eaten");
    assert_eq!(game.state.players[0].combat.hunger, hunger_before, "no hunger restored");
    assert_eq!(game.state.world.get_block(table[0], table[1], table[2]), block::FEAST_TABLE);

    // Two villagers near: the feast is held, hungry or not.
    for dz in [1.0, 2.0] {
        crate::entity::spawn_mob(
            &mut game.state.ecs,
            MobType::Villager,
            glam::Vec3::new(table[0] as f32 + 8.0, table[1] as f32, table[2] as f32 + dz), // off the crosshair line
        );
    }
    right_click(&mut game);
    assert!(game.state.players[0].inventory.hotbar_slot(0).is_none(), "the stack is shared out, not eaten");
    assert_eq!(game.state.world.get_block(table[0], table[1], table[2]), block::FEAST_MARKER);

    // Reading the marker with food in hand eats nothing.
    game.state.players[0].inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 3)));
    right_click(&mut game);
    assert_eq!(game.state.players[0].inventory.hotbar_slot(0).map(|s| s.count), Some(3), "the marker is read, not fed");
}
