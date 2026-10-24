//! Pick block (Spec 05 §8.2) through the real input and frame path: a middle
//! click reaches `InputState`, the frame intent, the aim ray and the
//! inventory, alone and as a joiner (Mirror and Strict).
use super::*;
use crate::action_identity::InventoryMode;
use crate::block;
use crate::item::{Item, ItemStack};
use crate::play_mode::PlayMode;

fn stack(b: crate::block::BlockId, n: u8) -> Option<ItemStack> {
    Some(ItemStack::new_block(b, n))
}

/// One middle click through the real frame path.
fn middle_click(hg: &mut HeadlessGame) {
    hg.state.input.cursor_captured = true;
    hg.state.input.mouse_button_pressed(winit::event::MouseButton::Middle);
    hg.state.input.mouse_button_released(winit::event::MouseButton::Middle);
    hg.frames(1);
}

/// Put `b` two blocks ahead of player 0 on a cleared pad (and in `server`'s
/// world when joined), and aim at its centre.
fn aim_at_block(hg: &mut HeadlessGame, server: Option<&mut crate::hosted_server::HostedServer>, b: crate::block::BlockId) {
    let mut server = server;
    let feet = clear_pad(hg, server.as_deref_mut());
    let cell = [feet[0], feet[1] + 1, feet[2] - 2];
    hg.state.world.set_block(cell[0], cell[1], cell[2], b);
    if let Some(server) = server {
        server.server.world.set_block(cell[0], cell[1], cell[2], b);
    }
    aim_at(hg, glam::Vec3::new(cell[0] as f32 + 0.5, cell[1] as f32 + 0.5, cell[2] as f32 + 0.5));
}

fn hotbar(hg: &HeadlessGame) -> usize {
    hg.state.players[0].hotbar_slot
}

fn slots(inv: &crate::inventory::Inventory) -> Vec<Option<ItemStack>> {
    inv.slots_iter().map(|s| s.cloned()).collect()
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_pick_block_in_survival_selects_or_swaps_and_never_conjures() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-pick-survival");
    hg.state.set_play_mode(PlayMode::Survival);
    hg.state.players[0].inventory = crate::inventory::Inventory::new();
    aim_at_block(&mut hg, None, block::STONE);

    // Carried nowhere: nothing happens.
    hg.state.players[0].inventory.set_slot(0, stack(block::DIRT, 3));
    hg.state.players[0].hotbar_slot = 0;
    let before = slots(&hg.state.players[0].inventory);
    middle_click(&mut hg);
    assert_eq!(slots(&hg.state.players[0].inventory), before, "no stone anywhere: nothing conjured");
    assert_eq!(hotbar(&hg), 0);

    // In the bag: it swaps with the selected slot, the old item going home.
    hg.state.players[0].inventory.set_slot(20, stack(block::STONE, 9));
    middle_click(&mut hg);
    let inv = &hg.state.players[0].inventory;
    assert_eq!(inv.slot(0), stack(block::STONE, 9).as_ref(), "the stone is in hand");
    assert_eq!(inv.slot(20), stack(block::DIRT, 3).as_ref(), "the dirt went where the stone came from");
    assert!(hg.state.players[0].crafting_ui.cursor_item.is_none() && !hg.state.players[0].crafting_ui.open);

    // On the hotbar: select that slot.
    hg.state.players[0].inventory.set_slot(5, stack(block::STONE, 1));
    hg.state.players[0].inventory.set_slot(0, stack(block::DIRT, 1));
    middle_click(&mut hg);
    assert_eq!(hotbar(&hg), 5);

    // Air and water pick nothing and change nothing.
    for fluidish in [block::AIR, block::WATER] {
        aim_at_block(&mut hg, None, fluidish);
        let before = slots(&hg.state.players[0].inventory);
        let sel = hotbar(&hg);
        middle_click(&mut hg);
        assert_eq!(slots(&hg.state.players[0].inventory), before);
        assert_eq!(hotbar(&hg), sel);
    }

    // No pick while a UI is up (chat, the crafting screen, the item search),
    // nor when dead.
    aim_at_block(&mut hg, None, block::STONE);
    hg.state.players[0].hotbar_slot = 0;
    hg.state.chat.open = true;
    middle_click(&mut hg);
    assert_eq!(hotbar(&hg), 0, "chat open: no pick");
    hg.state.chat.open = false;
    hg.state.players[0].crafting_ui.open = true;
    middle_click(&mut hg);
    assert_eq!(hotbar(&hg), 0, "inventory open: no pick");
    hg.state.players[0].crafting_ui.open = false;
    hg.state.players[0].explorer_state = Some(crate::inventory_explorer::ExplorerState::new());
    middle_click(&mut hg);
    assert_eq!(hotbar(&hg), 0, "item search open: no pick");
    hg.state.players[0].explorer_state = None;
    hg.state.players[0].combat.dead = true;
    middle_click(&mut hg);
    assert_eq!(hotbar(&hg), 0, "dead: no pick");
    hg.state.players[0].combat.dead = false;
    middle_click(&mut hg);
    assert_eq!(hotbar(&hg), 5, "and with everything shut it picks again");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_pick_block_in_creative_fills_the_hand() {
    isolate_saves();
    let mut hg = HeadlessGame::boot_into_world("harness-pick-creative");
    hg.state.set_play_mode(PlayMode::Creative);
    hg.state.players[0].inventory = crate::inventory::Inventory::new();
    aim_at_block(&mut hg, None, block::STONE);

    // Empty selected slot: one full stack there.
    hg.state.players[0].hotbar_slot = 3;
    middle_click(&mut hg);
    assert_eq!(hg.state.players[0].inventory.slot(3), stack(block::STONE, 64).as_ref());
    assert_eq!(hotbar(&hg), 3);

    // Already on the hotbar: just select it.
    hg.state.players[0].hotbar_slot = 0;
    middle_click(&mut hg);
    assert_eq!(hotbar(&hg), 3);

    // Selected slot taken: the first empty hotbar slot, selection moves.
    aim_at_block(&mut hg, None, block::DIRT);
    hg.state.players[0].inventory.set_slot(0, stack(block::GLASS, 1));
    hg.state.players[0].hotbar_slot = 0;
    middle_click(&mut hg);
    assert_eq!(hg.state.players[0].inventory.slot(1), stack(block::DIRT, 64).as_ref());
    assert_eq!(hotbar(&hg), 1);

    // Full hotbar: the selected slot is replaced.
    aim_at_block(&mut hg, None, block::SAND);
    for i in 0..9 {
        hg.state.players[0].inventory.set_slot(i, stack(block::GLASS, 1));
    }
    hg.state.players[0].hotbar_slot = 6;
    middle_click(&mut hg);
    assert_eq!(hg.state.players[0].inventory.slot(6), stack(block::SAND, 64).as_ref());
    assert_eq!(hotbar(&hg), 6);
    assert_eq!(hg.state.players[0].inventory.slot(7), stack(block::GLASS, 1).as_ref(), "the rest are untouched");

    // A crop gives its seeds.
    aim_at_block(&mut hg, None, block::WHEAT_STAGE_2);
    hg.state.players[0].hotbar_slot = 2;
    middle_click(&mut hg);
    assert_eq!(
        hg.state.players[0].inventory.slot(2).map(|s| s.item.clone()),
        Some(Item::Material(crate::item::MaterialId::WheatSeeds))
    );
}

/// A joiner's survival swap is the same three window clicks the inventory
/// screen makes: the server's window follows slot for slot with no mismatch.
fn joined_survival_swap(mode: InventoryMode, tag: &str) {
    isolate_saves();
    let (mut hg, mut server, slot) = joined_window_client_mode(tag, mode);
    aim_at_block(&mut hg, Some(&mut server), block::STONE);
    for inv in [&mut hg.state.players[0].inventory, &mut server.server.players[slot].inventory] {
        inv.set_slot(0, stack(block::DIRT, 3));
        inv.set_slot(20, stack(block::STONE, 9));
    }
    hg.state.players[0].hotbar_slot = 0;
    server.server.players[slot].possession = Default::default();
    for _ in 0..3 {
        harness_step(&mut server, &mut hg);
    }
    middle_click(&mut hg);
    for _ in 0..6 {
        harness_step(&mut server, &mut hg);
    }
    let ours = &hg.state.players[0].inventory;
    assert_eq!(ours.slot(0), stack(block::STONE, 9).as_ref());
    assert_eq!(ours.slot(20), stack(block::DIRT, 3).as_ref());
    let sp = &server.server.players[slot];
    assert_eq!(slots(&sp.inventory), slots(ours), "the server's 36 slots follow the swap");
    assert!(sp.cursor.is_none());
    assert_eq!(sp.possession.window_mismatch, 0);
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_pick_block_swap_reaches_a_mirror_joiners_server() {
    joined_survival_swap(InventoryMode::Mirror, "pick-mirror");
}

#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_pick_block_swap_reaches_a_strict_joiners_server() {
    joined_survival_swap(InventoryMode::Strict, "pick-strict");
}

/// A creative joiner's pick is the Explorer's give: the server grants the
/// stack into the lowest empty slot and the client holds that slot.
#[test]
#[ignore = "needs a GPU adapter holding the 506-layer atlas (llvmpipe caps 256) — run: cargo test -- --ignored game_harness"]
fn game_harness_pick_block_in_creative_asks_the_server_like_the_explorer() {
    isolate_saves();
    let (mut hg, mut server, slot) = joined_window_client_mode("pick-creative-join", InventoryMode::Strict);
    hg.state.set_play_mode(PlayMode::Creative);
    server.server.set_play_mode(PlayMode::Creative);
    aim_at_block(&mut hg, Some(&mut server), block::STONE);
    hg.state.players[0].inventory.set_slot(0, stack(block::DIRT, 1));
    server.server.players[slot].inventory.set_slot(0, stack(block::DIRT, 1));
    hg.state.players[0].hotbar_slot = 0;
    for _ in 0..3 {
        harness_step(&mut server, &mut hg);
    }
    middle_click(&mut hg);
    settle(&mut server, &mut hg);
    assert_eq!(server.server.players[slot].inventory.slot(1), stack(block::STONE, 64).as_ref(), "the server granted the stack");
    assert_eq!(hg.state.players[0].inventory.slot(1), stack(block::STONE, 64).as_ref(), "and the client holds it");
    assert_eq!(hotbar(&hg), 1, "the hand moved to where it landed");
}
