//! Rail freight Phase 1 — depot load/haul/unload, driven end-to-end through
//! `TestHost` (i.e. `GameServer::tick`).
//!
//! A **depot is just a chest adjacent to a track terminus** (no dedicated
//! block). One right-click loads whatever the depot holds onto the cart and
//! sends it off; on arrival at the far terminus the cart dumps its cargo into
//! the depot chest there. This test exercises the full A → cart → B item
//! movement over real (timed) travel, asserting both depots' before/after
//! contents and that the cart cargo is emptied into B.
//!
//! Task 1.4. Spec: `docs/foundations/2026-06-09-rail-freight-logistics.md`.

use glam::Vec3;

use crate::chest::ChestData;
use crate::item::{ItemStack, MaterialId};
use crate::rail::TRACK;
use crate::test_harness::{TestConfig, TestHost};

/// Build a straight E-W track at `y=64, z=0` from x=0..=2, with a depot chest
/// beside each terminus (off-track, at z=1). Returns the host plus the two
/// depot cells and the two terminus cells.
fn freight_line() -> (TestHost, [i32; 3], [i32; 3], [i32; 3], [i32; 3]) {
    let mut host = TestHost::start_with(TestConfig::default());
    for x in 0..3 {
        host.set_block(x, 64, 0, TRACK);
    }
    let term_a = [0, 64, 0];
    let term_b = [2, 64, 0];
    // Depots sit on the +Z neighbour of each terminus (NOT a track cell).
    let depot_a = [0, 64, 1];
    let depot_b = [2, 64, 1];
    (host, depot_a, depot_b, term_a, term_b)
}

#[test]
fn cart_hauls_freight_from_depot_a_to_depot_b() {
    let (mut host, depot_a, depot_b, term_a, term_b) = freight_line();

    // Stock depot A with two distinct stacks; depot B starts empty.
    host.insert_chest_with(
        depot_a[0],
        depot_a[1],
        depot_a[2],
        &[
            ItemStack::new_material(MaterialId::IronIngot, 12),
            ItemStack::new_material(MaterialId::Bread, 3),
        ],
    );
    host.insert_chest_with(depot_b[0], depot_b[1], depot_b[2], &[]);

    // Pre-conditions: A has 2 stacks (15 units), B empty.
    let before_a = host.chest_at(depot_a[0], depot_a[1], depot_a[2]).unwrap();
    assert_eq!(before_a.occupied(), 2, "depot A starts with the freight");
    let before_b = host.chest_at(depot_b[0], depot_b[1], depot_b[2]).unwrap();
    assert_eq!(before_b.occupied(), 0, "depot B starts empty");

    // Spawn a parked cart on terminus A and dispatch it east — this LOADS the
    // depot-A freight onto the cart before it departs (the production
    // right-click contract: "load what's here and go").
    let cart = host.spawn_cart(term_a[0], term_a[1], term_a[2]);
    let dispatched = host.dispatch_cart(
        term_a[0],
        term_a[1],
        term_a[2],
        Vec3::new(1.0, 0.0, 0.0), // look east toward terminus B
    );
    assert!(dispatched.is_some(), "a parked cart on the terminus must dispatch");

    // Immediately after dispatch: depot A is drained onto the cart, the cart is
    // moving, depot B still empty (haul hasn't completed yet).
    let after_load_a = host.chest_at(depot_a[0], depot_a[1], depot_a[2]).unwrap();
    assert_eq!(after_load_a.occupied(), 0, "depot A emptied onto the cart at dispatch");
    let loaded = host.cart_data(cart);
    assert!(loaded.speed > 0.0, "cart departed");
    assert_eq!(loaded.cargo.occupied(), 2, "freight rode out on the cart");
    let mid_b = host.chest_at(depot_b[0], depot_b[1], depot_b[2]).unwrap();
    assert_eq!(mid_b.occupied(), 0, "depot B still empty mid-haul");

    // Haul: tick until the cart reaches terminus B and parks. CART_SPEED ≈ 0.08
    // cells/tick over 2 cells ⇒ ~25 ticks; 200 is a generous ceiling.
    let mut ticks = 0;
    loop {
        host.tick(1);
        ticks += 1;
        let c = host.cart_data(cart);
        if c.speed == 0.0 && c.cell == (term_b[0], term_b[1], term_b[2]) {
            break;
        }
        assert!(ticks < 200, "cart should reach terminus B well within 200 ticks");
    }

    // Post-conditions: cart parked at B, cargo dumped into depot B, depot A
    // still empty, cart cargo emptied.
    let arrived = host.cart_data(cart);
    assert_eq!(arrived.cell, (term_b[0], term_b[1], term_b[2]), "parked at terminus B");
    assert_eq!(arrived.speed, 0.0, "cart parked");
    assert_eq!(arrived.cargo.occupied(), 0, "cart cargo dumped into depot B");

    let final_a = host.chest_at(depot_a[0], depot_a[1], depot_a[2]).unwrap();
    assert_eq!(final_a.occupied(), 0, "depot A stays empty");

    let final_b = host.chest_at(depot_b[0], depot_b[1], depot_b[2]).unwrap();
    assert_eq!(final_b.occupied(), 2, "both freight stacks landed in depot B");
    let total_b: u32 = final_b
        .slots
        .iter()
        .filter_map(|s| s.as_ref())
        .map(|s| u32::from(s.count))
        .sum();
    assert_eq!(total_b, 15, "all 12 iron + 3 bread units arrived intact");
}

#[test]
fn dispatch_without_a_depot_just_departs() {
    // No chest beside terminus A — dispatch must still send the (empty) cart.
    let (mut host, _depot_a, _depot_b, term_a, term_b) = freight_line();
    let cart = host.spawn_cart(term_a[0], term_a[1], term_a[2]);
    let dispatched = host.dispatch_cart(
        term_a[0],
        term_a[1],
        term_a[2],
        Vec3::new(1.0, 0.0, 0.0),
    );
    assert!(dispatched.is_some(), "loading is best-effort; the cart departs regardless");
    assert!(host.cart_data(cart).speed > 0.0, "cart moving");

    // It still rolls to terminus B and parks (no unload, no chest there).
    for _ in 0..200 {
        host.tick(1);
        if host.cart_data(cart).speed == 0.0 {
            break;
        }
    }
    let c = host.cart_data(cart);
    assert_eq!(c.cell, (term_b[0], term_b[1], term_b[2]), "parked at B");
    assert_eq!(c.cargo.occupied(), 0, "empty cart, nothing to unload");
}

/// Guard the "depot is just a chest adjacent to a terminus" rule via a stub
/// chest map (kept here alongside the integration coverage so the geometry is
/// documented next to the wiring it drives). `ChestData` import keeps this a
/// real type check, not just `rail` unit territory.
#[test]
fn depot_resolves_to_the_adjacent_chest_only() {
    let term = (5, 64, 9);
    let chest_cell = (5, 64, 10); // +Z neighbour
    let found = crate::rail::depot_chest_for(term, |c| c == chest_cell);
    assert_eq!(found, Some(chest_cell));
    // A chest two cells away is NOT a depot for this terminus.
    let far = (5, 64, 11);
    assert_eq!(crate::rail::depot_chest_for(term, |c| c == far), None);
    // Sanity: the helper above operates on the same shape we store.
    let _ = ChestData::new();
}

#[test]
fn full_depot_on_arrival_leaves_overflow_in_the_cart() {
    // Freight must never be silently lost: if the destination depot is full
    // when the cart arrives, the unload keeps the leftover cargo ON the cart.
    // Guards the Phase-B write-back in `tick_carts` at the ECS-tick level
    // (the pure `transfer_all` already has a unit test for the primitive).
    let (mut host, depot_a, depot_b, term_a, term_b) = freight_line();

    // Depot A holds the freight; depot B is FULL (all 27 slots occupied with a
    // different item, so the cart's iron/bread can't stack or find an empty
    // slot and is wholly rejected).
    host.insert_chest_with(
        depot_a[0],
        depot_a[1],
        depot_a[2],
        &[ItemStack::new_material(MaterialId::IronIngot, 12)],
    );
    let full: Vec<ItemStack> = (0..crate::chest::CHEST_SLOTS)
        .map(|_| ItemStack::new_material(MaterialId::Coal, 1))
        .collect();
    host.insert_chest_with(depot_b[0], depot_b[1], depot_b[2], &full);

    // Load + dispatch east.
    let cart = host.spawn_cart(term_a[0], term_a[1], term_a[2]);
    assert!(host.dispatch_cart(term_a[0], term_a[1], term_a[2], Vec3::new(1.0, 0.0, 0.0)).is_some());
    assert_eq!(host.cart_data(cart).cargo.occupied(), 1, "iron rode out on the cart");

    // Haul to terminus B and park.
    for _ in 0..200 {
        host.tick(1);
        if host.cart_data(cart).speed == 0.0 {
            break;
        }
    }

    let arrived = host.cart_data(cart);
    assert_eq!(arrived.cell, (term_b[0], term_b[1], term_b[2]), "parked at B");
    assert_eq!(
        arrived.cargo.occupied(),
        1,
        "full depot rejected the freight — it stays on the cart, not lost"
    );
    let kept: u32 = arrived
        .cargo
        .slots
        .iter()
        .filter_map(|s| s.as_ref())
        .map(|s| u32::from(s.count))
        .sum();
    assert_eq!(kept, 12, "all 12 iron units retained on the cart");

    // Depot B is unchanged — still full with its original 27 coal, no iron.
    let final_b = host.chest_at(depot_b[0], depot_b[1], depot_b[2]).unwrap();
    assert_eq!(final_b.occupied(), crate::chest::CHEST_SLOTS, "depot B still full");
}

// ── CA4: breach-to-break (armour effect + cart pickup) ───────────────────────

#[test]
fn breaching_a_wood_cart_drops_its_item_and_spills_cargo() {
    // The armour effect + the cart-pickup path end-to-end through the shared
    // breach core: a parked Wood cart loaded with two distinct cargo stacks,
    // breached until it breaks, must (a) be gone from the ECS and (b) drop a
    // WoodCart item plus BOTH cargo stacks as ground items at the cart's spot.
    let mut host = TestHost::start_with(TestConfig::default());
    let cell = [3, 64, 0];
    host.set_block(cell[0], cell[1], cell[2], TRACK);
    let cart = host.spawn_cart_with_hull(cell[0], cell[1], cell[2], crate::cart::Hull::Wood);

    // Load cargo onto the cart (two distinct, non-stacking materials).
    {
        let mut c = host
            .server
            .ecs
            .get::<&mut crate::cart::CartData>(cart)
            .unwrap();
        c.cargo.slots[0] = Some(ItemStack::new_material(MaterialId::IronIngot, 8));
        c.cargo.slots[5] = Some(ItemStack::new_material(MaterialId::Bread, 4));
    }

    // Before breaking: no cart item or cargo on the ground yet.
    assert_eq!(host.dropped_material_count(MaterialId::WoodCart), 0);
    assert!(host.cart_alive(cart), "cart alive before breaching");

    // Lay the full Wood breach threshold → it breaks this call.
    let broke = host.breach_cart(cart, crate::cart::Hull::Wood.breach_max());
    assert!(broke, "a fully-breached wood cart breaks");

    // (a) cart entity is gone.
    assert!(!host.cart_alive(cart), "cart despawned after breaching");
    // (b) the WoodCart pickup + both cargo stacks dropped at the cart's spot.
    assert_eq!(host.dropped_material_count(MaterialId::WoodCart), 1, "cart item dropped");
    assert_eq!(host.dropped_material_count(MaterialId::IronIngot), 8, "iron cargo spilled");
    assert_eq!(host.dropped_material_count(MaterialId::Bread), 4, "bread cargo spilled");

    // The TRACK rail is untouched — we broke the cart, not the rail.
    assert_eq!(host.get_block(cell[0], cell[1], cell[2]), TRACK, "rail stays after a cart breach");
}

#[test]
fn diamond_cart_takes_more_breach_to_break_than_wood() {
    // The whole point of "armour resists smashing": at the SAME per-strike
    // breach increment, a diamond cart survives strictly more strikes than wood.
    let strike = 5.0;
    let strikes_to_break = |hull: crate::cart::Hull| -> u32 {
        let mut host = TestHost::start_with(TestConfig::default());
        host.set_block(0, 64, 0, TRACK);
        let cart = host.spawn_cart_with_hull(0, 64, 0, hull);
        let mut n = 0;
        loop {
            n += 1;
            if host.breach_cart(cart, strike) {
                break;
            }
            assert!(n < 10_000, "cart must break eventually");
        }
        n
    };
    let wood = strikes_to_break(crate::cart::Hull::Wood);
    let diamond = strikes_to_break(crate::cart::Hull::Diamond);
    assert!(
        wood < diamond,
        "diamond armour resists more strikes than wood ({wood} vs {diamond})",
    );
}

#[test]
fn the_only_cart_on_a_cell_is_breached_when_its_track_is_mined() {
    // The interception keys off the cart's cell: breaching one cart in a yard
    // must leave a cart on a different cell untouched (cart_at_cell is per-cell).
    let mut host = TestHost::start_with(TestConfig::default());
    host.set_block(0, 64, 0, TRACK);
    host.set_block(2, 64, 0, TRACK);
    let a = host.spawn_cart_with_hull(0, 64, 0, crate::cart::Hull::Wood);
    let b = host.spawn_cart_with_hull(2, 64, 0, crate::cart::Hull::Wood);

    assert!(host.breach_cart(a, crate::cart::Hull::Wood.breach_max()), "cart A breaks");
    assert!(!host.cart_alive(a), "cart A gone");
    assert!(host.cart_alive(b), "cart B on a different cell is untouched");
}
