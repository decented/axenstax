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

/// `n` rounds of one frame then one logical tick: the frame path decides (raise, mine, use),
/// the tick counts and advances (the raise count, the break progress, the swing). The
/// frame waits a tick's worth of wall clock first: the break progress and the attack are
/// counted on frames that ran a time-based tick (`ticks_run > 0`), so without the wait a
/// frame can run none and "no break" would hold for the wrong reason.
fn run(hg: &mut HeadlessGame, n: u32) {
    for _ in 0..n {
        std::thread::sleep(std::time::Duration::from_millis(55));
        hg.frames(1);
        hg.ticks(1);
    }
}

/// Review 2 finding 5 and review 3 decision A: the target's own use wins over raising a
/// shield, and a mob wins only when a hand really does something to it. A main-hand
/// shield, the use button held on a door: the door toggles and the shield stays down.
/// On stone it rises. At a cow with nothing in the other hand that works on it, it
/// rises (the click is the shield's); with wheat in the other hand the cow is fed and
/// the shield stays down; at a brigand it rises.
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

    // A cow under the crosshair that nothing held works on: the shield rises at it.
    let at = glam::Vec3::new(feet[0] as f32 + 0.5, feet[1] as f32, feet[2] as f32 - 1.5);
    let cow = crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Cow, at);
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.7, 0.0));
    hold(&mut hg, winit::event::MouseButton::Right, true);
    hg.frames(3);
    assert!(hg.state.local_guard.contains(&0), "at a cow nothing works on, the shield rises");
    hold(&mut hg, winit::event::MouseButton::Right, false);
    hg.frames(2);

    // The same cow with wheat in the other hand: the cow is fed, the shield stays down.
    hg.state.players[0].offhand = Some(ItemStack::new_material(MaterialId::Wheat, 3));
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.7, 0.0));
    hold(&mut hg, winit::event::MouseButton::Right, true);
    // The press frame: the wheat's use wins, the cow is fed and the shield stays down.
    hg.frames(1);
    assert!(!hg.state.local_guard.contains(&0), "the wheat's use wins: the shield stays down");
    assert!(hg.state.ecs.get::<&crate::breeding::InLove>(cow).is_ok(), "and the cow is fed");
    // Review 4: the press is classified once. The cow is in love now and the wheat has no
    // use on it, but the button is still held: the shield stays down for the whole press.
    hg.frames(4);
    assert!(!hg.state.local_guard.contains(&0), "the same press never turns into a raise");
    hold(&mut hg, winit::event::MouseButton::Right, false);
    hg.frames(2);
    // Release and press again: the cow in love takes no wheat, so the shield raises.
    hold(&mut hg, winit::event::MouseButton::Right, true);
    hg.frames(3);
    assert!(hg.state.local_guard.contains(&0), "a new press asks again: the shield raises");
    hold(&mut hg, winit::event::MouseButton::Right, false);
    hg.frames(2);
    hg.state.players[0].offhand = None;
    hg.state.ecs.despawn(cow).ok();

    // A brigand: the enemy in front of you. The shield rises.
    let at = glam::Vec3::new(feet[0] as f32 + 0.5, feet[1] as f32, feet[2] as f32 - 1.5);
    crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Brigand, at);
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.9, 0.0));
    hold(&mut hg, winit::event::MouseButton::Right, true);
    hg.frames(2);
    assert!(hg.state.local_guard.contains(&0), "at a brigand the shield rises");
    hold(&mut hg, winit::event::MouseButton::Right, false);
}

/// Review 2 finding 6 and review 3 decision B: a raised shield neither mines nor attacks,
/// and once raised it stays raised while the button is held, whatever comes under the
/// crosshair. A pickaxe in the main hand, a shield in the off-hand, both buttons held:
/// the shield is raised in open air, then the aim turns, still holding, to a villager and a
/// door (targets whose use would win at a press: trade, a toggle; review 4 decision 5) and
/// to stone. It stays up, the villager is not hit or talked to, the door does not move and
/// the stone gets no break progress. Releasing the shield lets both work again.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_a_raised_shield_neither_mines_nor_attacks() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-shield-no-mining");
    fresh(&mut hg, PlayMode::Survival);
    hg.state.players[0].inventory.set_slot(0, Some(ItemStack::new_tool(Tool::new(ToolType::Pickaxe, ToolMaterial::Iron))));
    hg.state.players[0].offhand = Some(shield_item());
    let cell = aim_at_stone(&mut hg);
    let eye = hg.state.players[0].player.eye_pos();

    // Raise it in open air (the aim well above everything), both buttons down.
    aim_at(&mut hg, eye + glam::Vec3::new(0.0, 5.0, -1.0));
    hold(&mut hg, winit::event::MouseButton::Right, true);
    hold(&mut hg, winit::event::MouseButton::Left, true);
    run(&mut hg, 4);
    assert!(hg.state.local_guard.contains(&0), "the shield is up in open air");

    // Review 4 decision 5: turn to targets whose use would WIN at a press, still holding.
    // A villager (the right click would open its trade, the left would hit it) and a
    // door (it would toggle): the shield stays up, the villager is neither hit nor
    // talked to, the door does not move.
    let at = glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32, cell[2] as f32 + 1.2);
    let villager = crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Villager, at);
    let max = hg.state.ecs.get::<&crate::combat::Health>(villager).unwrap().current;
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.9, 0.0));
    run(&mut hg, 12);
    assert!(hg.state.local_guard.contains(&0), "turned over a villager: held on");
    assert_eq!(hg.state.ecs.get::<&crate::combat::Health>(villager).unwrap().current, max, "no hit while guarding");
    assert!(hg.state.players[0].dialogue_villager.is_none(), "and no trade opened");
    hg.state.ecs.despawn(villager).ok();
    hg.state.world.set_block(cell[0], cell[1], cell[2], block::OAK_DOOR);
    let door_before = (hg.state.world.get_block(cell[0], cell[1], cell[2]), hg.state.world.meta_at(cell[0], cell[1], cell[2]));
    aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
    run(&mut hg, 12);
    assert!(hg.state.local_guard.contains(&0), "turned to a door: held on");
    let door_after = (hg.state.world.get_block(cell[0], cell[1], cell[2]), hg.state.world.meta_at(cell[0], cell[1], cell[2]));
    assert_eq!(door_before, door_after, "the door did not toggle");

    // Turn to the stone: still up, and no break progress.
    hg.state.world.set_block(cell[0], cell[1], cell[2], block::STONE);
    aim_at(&mut hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
    run(&mut hg, 12);
    assert!(hg.state.local_guard.contains(&0), "turned to stone: held on");
    assert!(hg.state.players[0].breaking_pos.is_none() && hg.state.players[0].break_progress == 0, "no break in progress");
    assert_eq!(hg.state.world.get_block(cell[0], cell[1], cell[2]), block::STONE, "the stone stands");

    // Lowered (the shield button released, the left still held), both work again.
    hold(&mut hg, winit::event::MouseButton::Right, false);
    run(&mut hg, 8);
    assert!(!hg.state.local_guard.contains(&0), "released: lowered");
    assert!(
        hg.state.players[0].breaking_pos.is_some() || hg.state.world.get_block(cell[0], cell[1], cell[2]) != block::STONE,
        "lowered: the break begins (breaking_pos {:?}, progress {}, guard {:?})",
        hg.state.players[0].breaking_pos,
        hg.state.players[0].break_progress,
        hg.state.local_guard,
    );
    let cow = crate::entity::spawn_mob(&mut hg.state.ecs, crate::mob::MobType::Cow, at);
    let max = hg.state.ecs.get::<&crate::combat::Health>(cow).unwrap().current;
    aim_at(&mut hg, at + glam::Vec3::new(0.0, 0.7, 0.0));
    run(&mut hg, 12);
    assert!(hg.state.ecs.get::<&crate::combat::Health>(cow).is_ok_and(|h| h.current < max) || !hg.state.ecs.contains(cow), "lowered: the cow takes a hit");
}

/// Review 5, should-fix 1: a Strict seat runs only its main hand's actions, so an
/// off-hand action it cannot run must not block the click. A Strict joiner holds shears in
/// the main hand and a shield in the off-hand and holds use on a tethered sheep whose wool
/// is still growing. The shears refuse, and the off-hand's Lead removal never runs on a
/// Strict seat (its typed actions carry no hand), so the click is the shield's: it rises,
/// the server hears the raise, and the Lead stays on. It used to do nothing at all: the
/// raise was declined for the off-hand's Lead removal and execution, kept to the main
/// hand, sent nothing.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_strict_seat_raises_a_shield_past_a_lead_it_cannot_take_off() {
    use crate::entity::{Position, ProtocolId};
    isolate_saves();
    let (mut hg, mut server, slot) =
        joined_window_client_mode("strict-shears-shield", crate::action_identity::InventoryMode::Strict);
    clear_pad(&mut hg, Some(&mut server));
    let shears = ItemStack::new_tool(Tool::new(ToolType::Shears, ToolMaterial::Iron));
    // Both copies of the window: the client's and the server's.
    hg.state.players[0].hotbar_slot = 0;
    hg.state.players[0].inventory.set_slot(0, Some(shears.clone()));
    hg.state.players[0].offhand = Some(shield_item());
    let sp = &mut server.server.players[slot];
    sp.hotbar_slot = 0;
    sp.inventory.set_slot(0, Some(shears));
    sp.offhand = Some(shield_item());
    // The sheep, just shorn (its wool growing, by the server's clock) and on a Lead the
    // joiner holds.
    let at = server.server.players[slot].player.pos + glam::Vec3::new(0.0, 0.0, -2.0);
    let sheep = crate::entity::spawn_mob(&mut server.server.ecs, crate::mob::MobType::Sheep, at);
    let now = server.server.tick_counter;
    let mut shorn = crate::animal_products::AnimalProductState::new();
    shorn.last_action_tick = Some(now);
    server.server.ecs.insert_one(sheep, shorn).unwrap();
    server
        .server
        .ecs
        .insert_one(sheep, crate::tether::Tethered { target: crate::tether::TetherTarget::Player(slot) })
        .unwrap();
    for _ in 0..6 {
        server.server.ecs.get::<&mut Position>(sheep).unwrap().0 = at;
        harness_step(&mut server, &mut hg);
    }
    server.server.ecs.get::<&mut Position>(sheep).unwrap().0 = at;
    let id = server.server.ecs.get::<&ProtocolId>(sheep).expect("broadcast").0;
    let drawn = hg.state.remote_mobs.position_of(id).expect("the joiner mirrors the sheep");
    aim_at(&mut hg, drawn + glam::Vec3::new(0.0, 0.7, 0.0));

    // The premise: a Strict seat, and under its crosshair that sheep, tethered, wool growing.
    assert!(hg.state.strict_window_connection(), "a Strict seat");
    let p = &hg.state.players[0];
    let target = hg
        .state
        .remote_mobs
        .ray_target(p.player.eye_pos(), p.camera.forward(), crate::combat::ATTACK_REACH)
        .expect("a mirrored mob under the crosshair");
    assert_eq!(target.id, id, "the sheep is under the crosshair");
    assert!(target.tethered && target.product_not_ready, "tethered, its wool growing: {target:?}");

    // Hold use on it.
    hg.state.players[0].place_cooldown = 0;
    hold(&mut hg, winit::event::MouseButton::Right, true);
    for _ in 0..3 {
        harness_step(&mut server, &mut hg);
    }
    assert!(
        hg.state.local_guard.contains(&0),
        "the shears refuse and a Strict seat cannot take the Lead off with its off-hand: the shield rises"
    );
    for _ in 0..40 {
        if server.server.players[slot].shield_latch {
            break;
        }
        harness_step(&mut server, &mut hg);
    }
    assert!(server.server.players[slot].shield_latch, "the server hears the raise");
    assert!(server.server.ecs.get::<&crate::tether::Tethered>(sheep).is_ok(), "the Lead stays on");
    hold(&mut hg, winit::event::MouseButton::Right, false);
    harness_step(&mut server, &mut hg);
}
