//! Market Hubs v1 — vendor clustering + compass navigation (Spec 37).
//!
//! A Market Bell designates a discovery zone. `nearest_hub` powers the
//! `/market` compass hint; `vendors_in_hub` rolls up every Vendor
//! Block in radius for the directory panel. Pure helpers; the game-
//! loop owns place/break + the command + the UI.
//!
//! v1 is a discovery + aggregation overlay — it reads Vendor Block
//! state, it does NOT own or rent the land. Stall rentals → v2 (needs
//! plot rentals).
//!
//! Owner model reuses the LocalPlayer/Npub pattern shared with Vendor
//! / Tip Jar / Plot — see [[project_economy_block_owner_convergence]].
//!
//! Spec: `docs/foundations/2026-05-23-market-hubs.md`.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::world::World;

/// Discovery-zone radius in blocks (XZ). A vendor within this
/// Chebyshev-ish range of the bell rolls up into the hub directory.
pub const MARKET_HUB_RADIUS: i32 = 32;

/// Owner of a Market Hub. LocalPlayer(pidx) on alpha; Npub for the
/// Spec-1-Phase-4 convergence.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum HubOwner {
    LocalPlayer(usize),
    Npub(String),
}

/// One Market Hub — a discovery zone centred on its Market Bell.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarketHubData {
    pub owner: HubOwner,
    pub bell: (i32, i32, i32),
    pub radius: i32,
}

impl MarketHubData {
    pub fn from_bell(owner: HubOwner, bx: i32, by: i32, bz: i32) -> Self {
        MarketHubData { owner, bell: (bx, by, bz), radius: MARKET_HUB_RADIUS }
    }
    /// XZ Chebyshev containment for column (x, z).
    pub fn contains_column(&self, x: i32, z: i32) -> bool {
        (x - self.bell.0).abs() <= self.radius && (z - self.bell.2).abs() <= self.radius
    }
}

/// 8-point compass direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompassDir { N, NE, E, SE, S, SW, W, NW }

impl CompassDir {
    pub fn label(self) -> &'static str {
        match self {
            CompassDir::N => "north",
            CompassDir::NE => "north-east",
            CompassDir::E => "east",
            CompassDir::SE => "south-east",
            CompassDir::S => "south",
            CompassDir::SW => "south-west",
            CompassDir::W => "west",
            CompassDir::NW => "north-west",
        }
    }
}

/// 8-point compass direction from an XZ delta (dx east, dz south in
/// the engine's +Z-south convention). Zero delta returns N as a
/// stable default.
pub fn compass_dir(dx: f32, dz: f32) -> CompassDir {
    if dx == 0.0 && dz == 0.0 {
        return CompassDir::N;
    }
    // atan2 with +X = east, +Z = south. Bucket into 8 sectors.
    let ang = dz.atan2(dx); // radians, -pi..pi; 0 = east, +pi/2 = south
    let deg = ang.to_degrees().rem_euclid(360.0);
    // 0=east; sectors of 45° centred on each compass point.
    match ((deg + 22.5) / 45.0) as u32 % 8 {
        0 => CompassDir::E,
        1 => CompassDir::SE,
        2 => CompassDir::S,
        3 => CompassDir::SW,
        4 => CompassDir::W,
        5 => CompassDir::NW,
        6 => CompassDir::N,
        _ => CompassDir::NE,
    }
}

/// Nearest hub to `from`, with its planar distance + compass
/// direction. `None` if there are no hubs.
pub fn nearest_hub(hubs: &[MarketHubData], from: Vec3) -> Option<(&MarketHubData, f32, CompassDir)> {
    hubs.iter()
        .map(|h| {
            let dx = h.bell.0 as f32 - from.x;
            let dz = h.bell.2 as f32 - from.z;
            let dist = (dx * dx + dz * dz).sqrt();
            (h, dist, dx, dz)
        })
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(h, dist, dx, dz)| (h, dist, compass_dir(dx, dz)))
}

/// One row in the hub directory — a Vendor Block's offer summary.
#[derive(Clone, Debug, PartialEq)]
pub struct VendorListing {
    pub pos: (i32, i32, i32),
    pub item_name: String,
    pub price_sats: u32,
    pub stock: u32,
    pub owner_label: String,
}

/// Roll up every Vendor Block within `hub`'s radius into directory
/// rows. Reads `world.iter_vendors()`; skips vendors with no listed
/// item.
pub fn vendors_in_hub(hub: &MarketHubData, world: &World, registry: &crate::block::BlockRegistry) -> Vec<VendorListing> {
    let mut out = Vec::new();
    for ((x, y, z), data) in world.iter_vendors() {
        if !hub.contains_column(x, z) {
            continue;
        }
        let Some(slot) = data.slot.as_ref() else { continue };
        let item_name = slot.item.name(registry);
        let owner_label = match &data.owner {
            Some(crate::vendor::VendorOwner::LocalPlayer(p)) => format!("Player {}", p + 1),
            Some(crate::vendor::VendorOwner::Npub(n)) => n.chars().take(10).collect::<String>(),
            None => "—".to_string(),
        };
        out.push(VendorListing {
            pos: (x, y, z),
            item_name,
            price_sats: data.price_sats,
            stock: data.stock,
            owner_label,
        });
    }
    out
}

/// True iff `owner` is the given local player.
pub fn is_local_owner(owner: &HubOwner, pidx: usize) -> bool {
    matches!(owner, HubOwner::LocalPlayer(p) if *p == pidx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_dir_cardinals() {
        // +X = east, +Z = south.
        assert_eq!(compass_dir(10.0, 0.0), CompassDir::E);
        assert_eq!(compass_dir(-10.0, 0.0), CompassDir::W);
        assert_eq!(compass_dir(0.0, 10.0), CompassDir::S);
        assert_eq!(compass_dir(0.0, -10.0), CompassDir::N);
    }

    #[test]
    fn compass_dir_diagonals() {
        assert_eq!(compass_dir(10.0, 10.0), CompassDir::SE);
        assert_eq!(compass_dir(-10.0, -10.0), CompassDir::NW);
        assert_eq!(compass_dir(10.0, -10.0), CompassDir::NE);
        assert_eq!(compass_dir(-10.0, 10.0), CompassDir::SW);
    }

    #[test]
    fn compass_dir_zero_is_stable() {
        assert_eq!(compass_dir(0.0, 0.0), CompassDir::N);
    }

    #[test]
    fn contains_column_radius() {
        let h = MarketHubData::from_bell(HubOwner::LocalPlayer(0), 0, 64, 0);
        assert!(h.contains_column(0, 0));
        assert!(h.contains_column(MARKET_HUB_RADIUS, MARKET_HUB_RADIUS));
        assert!(!h.contains_column(MARKET_HUB_RADIUS + 1, 0));
        assert!(!h.contains_column(0, MARKET_HUB_RADIUS + 1));
    }

    #[test]
    fn nearest_hub_picks_closest() {
        let hubs = vec![
            MarketHubData::from_bell(HubOwner::LocalPlayer(0), 100, 64, 0),
            MarketHubData::from_bell(HubOwner::LocalPlayer(1), 10, 64, 0),
        ];
        let (h, dist, dir) = nearest_hub(&hubs, Vec3::new(0.0, 64.0, 0.0)).unwrap();
        assert_eq!(h.bell, (10, 64, 0), "closest bell wins");
        assert!((dist - 10.0).abs() < 0.01);
        assert_eq!(dir, CompassDir::E);
    }

    #[test]
    fn nearest_hub_none_when_empty() {
        assert!(nearest_hub(&[], Vec3::ZERO).is_none());
    }

    #[test]
    fn is_local_owner_matches() {
        assert!(is_local_owner(&HubOwner::LocalPlayer(2), 2));
        assert!(!is_local_owner(&HubOwner::LocalPlayer(2), 0));
        assert!(!is_local_owner(&HubOwner::Npub("npub1".to_string()), 0));
    }
}
