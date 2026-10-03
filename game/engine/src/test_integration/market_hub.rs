//! Spec 37 Market Hubs integration tests.
//!
//! End-to-end over World.market_hubs + vendors_in_hub rollup +
//! save/load round-trip. Pure compass/zone maths in `src/market_hub.rs`.

use crate::item::{Item, ItemStack, MaterialId};
use crate::market_hub::{self, HubOwner, MarketHubData};
use crate::vendor::{VendorData, VendorOwner};
use crate::world::World;

fn vendor_with(item: MaterialId, price: u32, stock: u32, owner: usize) -> VendorData {
    VendorData {
        owner: Some(VendorOwner::LocalPlayer(owner)),
        mode: None,
        slot: Some(ItemStack { item: Item::Material(item), count: stock as u8 }),
        barter_request: None,
        price_sats: price,
        stock,
        escrow_sats: 0,
        last_txn_tick: 0,
        lot_size: 1,
    }
}

#[test]
fn vendors_in_hub_filters_by_radius() {
    let mut world = World::new();
    let registry = crate::block::BlockRegistry::new();
    // One vendor inside the hub, one far outside.
    world.insert_vendor((5, 64, 5), vendor_with(MaterialId::Bread, 3, 8, 0));
    world.insert_vendor((9000, 64, 9000), vendor_with(MaterialId::Carrot, 2, 4, 0));
    let hub = MarketHubData::from_bell(HubOwner::LocalPlayer(0), 0, 64, 0);
    let listings = market_hub::vendors_in_hub(&hub, &world, &registry);
    assert_eq!(listings.len(), 1, "only the in-radius vendor rolls up");
    assert_eq!(listings[0].pos, (5, 64, 5));
    assert_eq!(listings[0].price_sats, 3);
    assert_eq!(listings[0].stock, 8);
}

#[test]
fn vendors_in_hub_skips_empty_vendors() {
    let mut world = World::new();
    let registry = crate::block::BlockRegistry::new();
    let mut empty = vendor_with(MaterialId::Bread, 3, 0, 0);
    empty.slot = None; // no item listed
    world.insert_vendor((2, 64, 2), empty);
    let hub = MarketHubData::from_bell(HubOwner::LocalPlayer(0), 0, 64, 0);
    let listings = market_hub::vendors_in_hub(&hub, &world, &registry);
    assert!(listings.is_empty(), "vendors with no listed item are skipped");
}

#[test]
fn vendor_owner_label_formats() {
    let mut world = World::new();
    let registry = crate::block::BlockRegistry::new();
    world.insert_vendor((1, 64, 1), vendor_with(MaterialId::Bread, 5, 2, 2));
    let hub = MarketHubData::from_bell(HubOwner::LocalPlayer(0), 0, 64, 0);
    let listings = market_hub::vendors_in_hub(&hub, &world, &registry);
    assert_eq!(listings[0].owner_label, "Player 3"); // LocalPlayer(2) → Player 3
}

#[test]
fn market_hub_release_removes_only_matching_bell() {
    let mut world = World::new();
    world.market_hubs.push(MarketHubData::from_bell(HubOwner::LocalPlayer(0), 0, 64, 0));
    world.market_hubs.push(MarketHubData::from_bell(HubOwner::LocalPlayer(0), 100, 64, 100));
    world.release_market_hub((0, 64, 0));
    assert_eq!(world.market_hubs.len(), 1);
    assert_eq!(world.market_hubs[0].bell, (100, 64, 100));
}

#[test]
fn market_hubs_round_trip_save_load() {
    let hubs = vec![
        MarketHubData::from_bell(HubOwner::LocalPlayer(0), 0, 64, 0),
        MarketHubData::from_bell(HubOwner::Npub("npub1m".to_string()), -50, 70, 200),
    ];
    let bytes = bincode::serialize(&hubs).expect("serialize");
    let back: Vec<MarketHubData> = bincode::deserialize(&bytes).expect("deserialize");
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].bell, (0, 64, 0));
    assert_eq!(back[1].owner, HubOwner::Npub("npub1m".to_string()));
    assert_eq!(back[1].bell, (-50, 70, 200));
}

#[test]
fn world_clear_resets_market_hubs() {
    let mut world = World::new();
    world.market_hubs.push(MarketHubData::from_bell(HubOwner::LocalPlayer(0), 0, 64, 0));
    world.clear();
    assert!(world.market_hubs.is_empty());
}
