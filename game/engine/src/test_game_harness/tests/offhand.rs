//! The off-hand (`hands.rs`) through the real input and frame path: a right
//! click uses the main hand unless its item has no use, F swaps the hands, and
//! death leaves the off-hand on the body.
use super::*;
use crate::block;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{ItemStack, MaterialId};
use crate::play_mode::PlayMode;

fn right_click(hg: &mut HeadlessGame) {
    hg.state.input.cursor_captured = true;
    hg.state.input.mouse_button_pressed(winit::event::MouseButton::Right);
    hg.frames(1);
    hg.state.input.mouse_button_released(winit::event::MouseButton::Right);
    hg.frames(1);
}

fn press_f(hg: &mut HeadlessGame) {
    hg.state.input.cursor_captured = true;
    hg.state.input.key_pressed(winit::keyboard::KeyCode::KeyF);
    hg.state.input.key_released(winit::keyboard::KeyCode::KeyF);
    hg.frames(1);
}

/// A stone block two ahead of player 0 on a cleared pad, aimed at; the cell's
/// neighbours are returned so a placement can be found.
fn aim_at_stone(hg: &mut HeadlessGame) -> [i32; 3] {
    let feet = clear_pad(hg, None);
    let cell = [feet[0], feet[1], feet[2] - 2];
    hg.state.world.set_block(cell[0], cell[1], cell[2], block::STONE);
    aim_at(hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
    cell
}

fn count_blocks(hg: &HeadlessGame, around: [i32; 3], id: block::BlockId) -> usize {
    let mut n = 0;
    for x in around[0] - 2..=around[0] + 2 {
        for y in around[1] - 2..=around[1] + 3 {
            for z in around[2] - 3..=around[2] + 3 {
                if hg.state.world.get_block(x, y, z) == id {
                    n += 1;
                }
            }
        }
    }
    n
}

fn fresh(hg: &mut HeadlessGame, mode: PlayMode) {
    hg.state.set_play_mode(mode);
    hg.state.players[0].inventory = crate::inventory::Inventory::new();
    hg.state.players[0].offhand = None;
    hg.state.players[0].hotbar_slot = 0;
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_offhand_torch_places_from_the_off_hand_while_a_pickaxe_is_held() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-offhand-place");
    fresh(&mut hg, PlayMode::Survival);
    let pick = ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron));
    hg.state.players[0].inventory.set_slot(0, Some(pick.clone()));
    hg.state.players[0].offhand = Some(ItemStack::new_block(block::TORCH, 5));
    let cell = aim_at_stone(&mut hg);
    let before = count_blocks(&hg, cell, block::TORCH);

    right_click(&mut hg);
    assert_eq!(count_blocks(&hg, cell, block::TORCH), before + 1, "a torch was placed");
    assert_eq!(hg.state.players[0].offhand, Some(ItemStack::new_block(block::TORCH, 4)), "from the off-hand");
    assert_eq!(hg.state.players[0].inventory.slot(0), Some(&pick), "the pickaxe is untouched");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_offhand_a_block_in_hand_places_from_the_main_hand_and_food_eats_from_it() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-offhand-main");
    fresh(&mut hg, PlayMode::Survival);
    hg.state.players[0].inventory.set_slot(0, Some(ItemStack::new_block(block::DIRT, 9)));
    hg.state.players[0].offhand = Some(ItemStack::new_block(block::TORCH, 5));
    let cell = aim_at_stone(&mut hg);
    right_click(&mut hg);
    assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(8), "dirt placed from the main hand");
    assert_eq!(hg.state.players[0].offhand, Some(ItemStack::new_block(block::TORCH, 5)), "the torches stay");
    assert_eq!(count_blocks(&hg, cell, block::TORCH), 0);

    // Food in the main hand is eaten from the main hand, when hungry.
    hg.state.players[0].inventory.set_slot(0, Some(ItemStack::new_material(MaterialId::Bread, 3)));
    hg.state.players[0].combat.hunger = 2;
    hg.frames(30); // the placement's cooldown runs out
    right_click(&mut hg);
    assert_eq!(hg.state.players[0].inventory.slot(0).map(|s| s.count), Some(2), "one bite from the main stack");
    assert_eq!(hg.state.players[0].offhand, Some(ItemStack::new_block(block::TORCH, 5)));
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_offhand_f_swaps_the_hands_and_death_keeps_the_off_hand() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-offhand-swap");
    fresh(&mut hg, PlayMode::Survival);
    hg.state.players[0].inventory.set_slot(0, Some(ItemStack::new_block(block::STONE, 9)));
    hg.state.players[0].inventory.set_slot(3, Some(ItemStack::new_block(block::DIRT, 4)));
    hg.state.players[0].offhand = Some(ItemStack::new_block(block::TORCH, 5));

    press_f(&mut hg);
    assert_eq!(hg.state.players[0].inventory.slot(0), Some(&ItemStack::new_block(block::TORCH, 5)));
    assert_eq!(hg.state.players[0].offhand, Some(ItemStack::new_block(block::STONE, 9)));
    press_f(&mut hg);
    assert_eq!(hg.state.players[0].inventory.slot(0), Some(&ItemStack::new_block(block::STONE, 9)));

    // Death: the bag goes to a grave, the off-hand stays on the body.
    hg.state.players[0].combat.take_damage(1000.0);
    assert!(hg.state.players[0].combat.dead);
    hg.ticks(3);
    let p = &hg.state.players[0];
    assert!(p.inventory.slot(3).is_none(), "the inventory left the body");
    assert_eq!(p.offhand, Some(ItemStack::new_block(block::TORCH, 5)), "the off-hand did not");
}

fn shield_item() -> ItemStack {
    ItemStack::new_tool(Tool::new(ToolType::Shield, ToolMaterial::Wood))
}

fn hold(hg: &mut HeadlessGame, button: winit::event::MouseButton, down: bool) {
    hg.state.input.cursor_captured = true;
    if down {
        hg.state.input.mouse_button_pressed(button);
    } else {
        hg.state.input.mouse_button_released(button);
    }
}

/// Review 2 finding 5: the target's own use wins over raising a shield. A main-hand
/// shield with an empty off-hand, the use button held on a door: the door toggles and
/// the shield stays down. On stone it rises. Over a mob it stays down.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_a_shield_raises_only_where_the_target_has_no_other_use() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-shield-yields");
    fresh(&mut hg, PlayMode::Survival);
    hg.state.players[0].inventory.set_slot(0, Some(shield_item()));
    let feet = clear_pad(&mut hg, None);
    let cell = [feet[0], feet[1], feet[2] - 2];

    // A door: it toggles, the shield does not rise.
    hg.state.world.set_block(cell[0], cell[1], cell[2], block::OAK_DOOR);
    aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
    let before = (hg.state.world.get_block(cell[0], cell[1], cell[2]), hg.state.world.meta_at(cell[0], cell[1], cell[2]));
    hold(&mut hg, winit::event::MouseButton::Right, true);
    hg.frames(3);
    hold(&mut hg, winit::event::MouseButton::Right, false);
    hg.frames(1);
    let after = (hg.state.world.get_block(cell[0], cell[1], cell[2]), hg.state.world.meta_at(cell[0], cell[1], cell[2]));
    assert_ne!(before, after, "the door toggled");
    assert!(!hg.state.local_guard.contains(&0), "and the shield did not rise");

    // Stone: it rises.
    hg.state.world.set_block(cell[0], cell[1], cell[2], block::STONE);
    aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
    hold(&mut hg, winit::event::MouseButton::Right, true);
    hg.frames(3);
    assert!(hg.state.local_guard.contains(&0), "on stone the shield rises");
    hold(&mut hg, winit::event::MouseButton::Right, false);
    hg.frames(2);

    // A mob under the crosshair: the shield stays down.
    let at = glam::Vec3::new(feet[0] as f32 + 0.5, feet[1] as f32, feet[2] as f32 - 1.5);
    crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Cow, at);
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.7, 0.0));
    hold(&mut hg, winit::event::MouseButton::Right, true);
    hg.frames(3);
    assert!(!hg.state.local_guard.contains(&0), "over a mob the shield stays down");
    hold(&mut hg, winit::event::MouseButton::Right, false);
}

/// Review 2 finding 6: a raised shield neither mines nor attacks. A pickaxe in the
/// main hand, a shield in the off-hand, both buttons held on stone: no break
/// progress. A mob in front takes no hit. Lowering the shield lets mining start.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_a_raised_shield_neither_mines_nor_attacks() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-shield-no-mining");
    fresh(&mut hg, PlayMode::Survival);
    hg.state.players[0].inventory.set_slot(0, Some(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
    hg.state.players[0].offhand = Some(shield_item());
    let cell = aim_at_stone(&mut hg);

    hold(&mut hg, winit::event::MouseButton::Right, true);
    hold(&mut hg, winit::event::MouseButton::Left, true);
    hg.frames(12);
    assert!(hg.state.local_guard.contains(&0), "the shield is up");
    assert!(hg.state.players[0].breaking_pos.is_none() && hg.state.players[0].break_progress == 0, "no break in progress");
    assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), block::STONE, "the stone stands");

    // A mob in front takes no hit while the shield is up.
    let at = glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32, cell[2] as f32 + 1.2);
    let cow = crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Cow, at);
    let max = hg.state.ecs.get::<&crate::combat::Health>(cow).unwrap().current;
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.7, 0.0));
    hg.frames(12);
    assert_eq!(hg.state.ecs.get::<&crate::combat::Health>(cow).unwrap().current, max, "no hit while guarding");
    hg.state.ecs.despawn(cow).ok();

    // Lowered, mining starts again.
    hold(&mut hg, winit::event::MouseButton::Right, false);
    aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
    hg.frames(6);
    assert!(
        hg.state.players[0].breaking_pos.is_some() || hg.state.world.get_block(cell[0], cell[1], cell[2]) != block::STONE,
        "lowered: the break begins"
    );
}
