//! Review 5, should-fix 2: a use press is classified once and kept until the
//! button is released (`hands::shield_press`). A release the frame path never sees (the
//! game paused, a menu over the world, the player dead) must still end the press, so the
//! next press asks the target again. Every test feeds a cow with the shield down, crosses
//! one of those boundaries with a release and a new press, and comes back holding: the
//! cow is in love now (or the body is far away), nothing else claims the click, and the
//! shield must rise.
use super::*;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::item::{ItemStack, MaterialId};
use crate::play_mode::PlayMode;
use winit::event::MouseButton;

fn shield() -> ItemStack {
    ItemStack::new_tool(Tool::new(ToolType::Shield, ToolMaterial::Wood))
}

fn wheat() -> ItemStack {
    ItemStack::new_material(MaterialId::Wheat, 3)
}

/// A Survival world with `main` in the hotbar and `off` in the off-hand, a cow 1.5 blocks
/// ahead on a cleared pad, aimed at, and the use button pressed on it: the cow is fed and
/// the shield stays down for the whole press. The button is still held.
fn fed_with_the_button_held(tag: &str, main: ItemStack, off: ItemStack) -> HeadlessGame {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world(tag);
    hg.state.set_play_mode(PlayMode::Survival);
    let p = &mut hg.state.players[0];
    p.inventory = crate::inventory::Inventory::new();
    p.hotbar_slot = 0;
    p.inventory.set_slot(0, Some(main));
    p.offhand = Some(off);
    let feet = clear_pad(&mut hg, None);
    let at = glam::Vec3::new(feet[0] as f32 + 0.5, feet[1] as f32, feet[2] as f32 - 1.5);
    let cow = crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Cow, at);
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.7, 0.0));
    hg.state.input.cursor_captured = true;
    hg.state.input.mouse_button_pressed(MouseButton::Right);
    hg.frames(1);
    assert!(hg.state.ecs.get::<&crate::breeding::InLove>(cow).is_ok(), "the press fed the cow");
    assert!(!hg.state.local_guard.contains(&0), "and the shield stayed down");
    hg.frames(2);
    assert!(!hg.state.local_guard.contains(&0), "for the whole press");
    hg
}

/// One frame of the pause menu. The menu draws at most 20 times a second and a frame
/// sooner returns before doing anything, so wait out the throttle: every paused frame
/// here really runs (and ends the frame's input edges).
fn paused_frame(hg: &mut HeadlessGame) {
    std::thread::sleep(std::time::Duration::from_millis(60));
    hg.frames(1);
}

fn press_e(hg: &mut HeadlessGame) {
    hg.state.input.key_pressed(winit::keyboard::KeyCode::KeyE);
    hg.state.input.key_released(winit::keyboard::KeyCode::KeyE);
    hg.frames(1);
}

/// Shield main, wheat off: paused, the button released and pressed again behind the
/// menu, then resumed with it held. That is a new press: the cow in love takes no wheat,
/// so the shield rises. (The old press's "down" used to survive the pause.)
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_use_press_a_release_while_paused_ends_the_press() {
    let mut hg = fed_with_the_button_held("harness-press-pause", shield(), wheat());
    hg.state.mode = crate::GameMode::Paused { confirm_quit: false, confirm_creative: false };
    hg.state.release_cursor();
    paused_frame(&mut hg);
    hg.state.input.mouse_button_released(MouseButton::Right);
    paused_frame(&mut hg);
    paused_frame(&mut hg);
    hg.state.input.mouse_button_pressed(MouseButton::Right);
    paused_frame(&mut hg);
    paused_frame(&mut hg);
    // Resume (the pause menu's Resume), still holding.
    hg.state.mode = crate::GameMode::Playing;
    hg.state.reset_tick_timing();
    hg.state.capture_cursor();
    hg.frames(3);
    assert!(
        hg.state.local_guard.contains(&0),
        "the press after the pause asks the target again: the shield rises"
    );
}

/// The same across the inventory screen: E opens it (it frees the cursor), the button is
/// released and pressed again behind it, E closes it with the button held.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_use_press_a_release_behind_the_inventory_ends_the_press() {
    let mut hg = fed_with_the_button_held("harness-press-modal", shield(), wheat());
    press_e(&mut hg);
    assert!(hg.state.players[0].crafting_ui.open, "the inventory is open");
    hg.state.input.mouse_button_released(MouseButton::Right);
    hg.frames(2);
    hg.state.input.mouse_button_pressed(MouseButton::Right);
    hg.frames(2);
    press_e(&mut hg);
    assert!(!hg.state.players[0].crafting_ui.open, "the inventory is closed");
    hg.frames(3);
    assert!(
        hg.state.local_guard.contains(&0),
        "the press after the inventory asks the target again: the shield rises"
    );
}

/// Wheat main (it goes to the grave), shield off (it stays on the body): killed with the
/// button held, released and pressed again on the death screen, respawned with it held and
/// aimed at open sky. A new press: the off-hand shield rises.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_use_press_a_release_while_dead_ends_the_press() {
    let mut hg = fed_with_the_button_held("harness-press-death", wheat(), shield());
    hg.state.players[0].combat.take_damage(1000.0);
    hg.ticks(2);
    assert!(hg.state.players[0].combat.dead, "dead");
    hg.frames(2);
    hg.state.input.mouse_button_released(MouseButton::Right);
    hg.frames(2);
    hg.state.input.mouse_button_pressed(MouseButton::Right);
    hg.frames(2);
    assert!(hg.state.players[0].combat.request_respawn());
    hg.ticks(2);
    assert!(!hg.state.players[0].combat.dead, "respawned");
    assert_eq!(hg.state.players[0].offhand, Some(shield()), "the shield stayed on the body");
    hg.state.capture_cursor();
    let eye = hg.state.players[0].player.eye_pos();
    aim_at(&mut hg, eye + glam::Vec3::new(0.0, 5.0, -1.0));
    hg.frames(3);
    assert!(
        hg.state.local_guard.contains(&0),
        "the press after the respawn asks the target again: the shield rises"
    );
}
