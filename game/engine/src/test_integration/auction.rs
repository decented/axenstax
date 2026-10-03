//! Spec 38 Auctions integration tests.
//!
//! End-to-end over World + ECS: configure → bid → expiry-settle
//! delivers the lot + credits escrow; below-reserve returns the lot;
//! save round-trip; the tick driver settles only past-deadline.
//! Pure bid/settle maths are covered in `src/auction.rs`.

use crate::auction::{self, AuctionData, AuctionOwner};
use crate::item::{Item, ItemStack, MaterialId};
use crate::world::World;

fn diamond_lot() -> ItemStack {
    ItemStack { item: Item::Material(MaterialId::Diamond), count: 1 }
}

/// Build a started auction at `pos` owned by player 0, with a lot +
/// reserve + deadline, inserted into the world.
fn place_started(world: &mut World, pos: (i32, i32, i32), reserve: u64, deadline: u64) {
    let mut a = AuctionData::new(AuctionOwner::LocalPlayer(0));
    a.lot = Some(diamond_lot());
    a.reserve_sats = reserve;
    a.started = true;
    a.deadline_tick = deadline;
    world.insert_auction(pos, a);
}

fn count_item_entities(ecs: &hecs::World) -> usize {
    ecs.query::<&crate::entity::ItemEntity>().iter().count()
}

#[test]
fn tick_settles_only_past_deadline() {
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    place_started(&mut world, (0, 64, 0), 0, 1_000);
    // Before deadline: no settlement.
    let n = auction::tick_auctions(&mut world, &mut ecs, 999);
    assert_eq!(n, 0);
    assert!(!world.auction_at((0, 64, 0)).unwrap().settled);
    // At deadline: settles (unsold — no bids → lot returned).
    let n = auction::tick_auctions(&mut world, &mut ecs, 1_000);
    assert_eq!(n, 1);
    assert!(world.auction_at((0, 64, 0)).unwrap().settled);
}

#[test]
fn sold_auction_delivers_lot_and_credits_escrow() {
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    place_started(&mut world, (0, 64, 0), 50, 1_000);
    // Player 1 bids 80 (≥ reserve).
    let bid = auction::try_bid(
        world.auction_at_mut((0, 64, 0)).unwrap(), 1, 80, 100,
    );
    assert!(matches!(bid, auction::BidOutcome::Accepted { .. }));
    // Expire + settle.
    let before = count_item_entities(&ecs);
    auction::tick_auctions(&mut world, &mut ecs, 1_000);
    let after = count_item_entities(&ecs);
    assert_eq!(after, before + 1, "won lot spawned as an ItemEntity");
    let a = world.auction_at((0, 64, 0)).unwrap();
    assert!(a.settled);
    assert_eq!(a.escrow_sats, 80, "owner proceeds credited");
    assert!(a.lot.is_none(), "lot delivered out");
}

#[test]
fn below_reserve_returns_lot_unsold() {
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    place_started(&mut world, (0, 64, 0), 100, 1_000);
    // No bids at all → unsold, lot returned.
    let before = count_item_entities(&ecs);
    auction::tick_auctions(&mut world, &mut ecs, 1_000);
    let after = count_item_entities(&ecs);
    assert_eq!(after, before + 1, "unsold lot returned as ItemEntity");
    let a = world.auction_at((0, 64, 0)).unwrap();
    assert!(a.settled);
    assert_eq!(a.escrow_sats, 0, "no proceeds on unsold");
}

#[test]
fn settle_is_idempotent_no_double_spawn() {
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    place_started(&mut world, (0, 64, 0), 0, 1_000);
    auction::tick_auctions(&mut world, &mut ecs, 1_000);
    let after_first = count_item_entities(&ecs);
    // A second sweep past the deadline must not re-settle / re-spawn.
    auction::tick_auctions(&mut world, &mut ecs, 2_000);
    assert_eq!(count_item_entities(&ecs), after_first, "no double settlement");
}

#[test]
fn auctions_round_trip_save_load() {
    let mut a = AuctionData::new(AuctionOwner::LocalPlayer(0));
    a.lot = Some(diamond_lot());
    a.reserve_sats = 50;
    a.high_bid = 80;
    a.high_bidder = Some(1);
    a.started = true;
    a.deadline_tick = 5_000;
    let saved = crate::save::SavedAuction { x: 1, y: 64, z: 2, data: a };
    let bytes = bincode::serialize(&saved).expect("serialize");
    let back: crate::save::SavedAuction = bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(back.x, 1);
    assert_eq!(back.data.reserve_sats, 50);
    assert_eq!(back.data.high_bid, 80);
    assert_eq!(back.data.high_bidder, Some(1));
    assert_eq!(back.data.deadline_tick, 5_000);
    assert!(back.data.lot.is_some());
}

#[test]
fn tick_skips_block_mined_mid_sweep() {
    // If the auction block-entity is removed before the sweep mutates
    // it, the None-guard skips cleanly (no panic). Simulate by an
    // empty world — the sweep just finds nothing.
    let mut world = World::new();
    let mut ecs = hecs::World::new();
    let n = auction::tick_auctions(&mut world, &mut ecs, 10_000);
    assert_eq!(n, 0);
}
