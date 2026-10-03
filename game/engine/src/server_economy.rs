//! Spec 6 §13 — Server Economy Config (T1.5 Phase 11 → Bitcoin §13).
//!
//! Pure data holding the server operator's per-item trade-value
//! overrides + sats-per-trade-unit conversion rate. The Vendor Block
//! pricing path + future Bitcoin economy reads this; default values
//! come from `Item::trade_value()` (the spec value ladder).
//!
//! Off-by-default: a fresh config means "use defaults everywhere".
//! Operators opt in by populating `overrides` (per-item map) and/or
//! setting `sats_per_unit > 0` for Bitcoin servers.
//!
//! Parent-control gate per Spec 6 §10.3 lives in the consumer (the
//! per-player `charter_allows_sats` flag) — this config is the
//! server-wide knob; the per-player gate is downstream.
//!
//! No operator config surface exists yet to populate/read this (the Vendor
//! Block pricing path + Bitcoin economy the doc above describes as future
//! readers haven't adopted it), so the whole module is tested but otherwise
//! unreferenced. Module-scoped allow rather than repeating
//! `cfg_attr(not(test), allow(dead_code))` on every item.

#![allow(dead_code)]

use std::collections::HashMap;

use crate::item::{Item, MaterialId};
use crate::block::BlockId;

/// Server-side economy configuration. Pure data; no engine state.
#[derive(Clone, Debug, Default)]
pub struct ServerEconomyConfig {
    /// Per-MaterialId overrides on default `trade_value`. `None` here
    /// means "non-tradeable on this server" (operator can explicitly
    /// remove specific items from the trade economy).
    pub material_overrides: HashMap<MaterialId, Option<u64>>,
    /// Per-BlockId overrides. Mirrors `material_overrides` for placed
    /// blocks.
    pub block_overrides: HashMap<BlockId, Option<u64>>,
    /// Sats per trade-unit. `0` = trade-value is internal-score-only.
    /// On Bitcoin-enabled servers, operator sets a positive value;
    /// item sats price = `trade_value * sats_per_unit`. Default 0
    /// keeps every server economy-only until the operator opts in.
    pub sats_per_unit: u64,
}

impl ServerEconomyConfig {
    /// New config with no overrides + sats off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a per-item override. `Some(v)` overrides the default;
    /// `None` removes the item from the trade economy.
    pub fn override_material(&mut self, id: MaterialId, value: Option<u64>) {
        self.material_overrides.insert(id, value);
    }

    pub fn override_block(&mut self, id: BlockId, value: Option<u64>) {
        self.block_overrides.insert(id, value);
    }

    /// Lookup the trade value of an item against this config.
    /// Falls back to `Item::trade_value()` when no override is set.
    pub fn trade_value(&self, item: &Item) -> Option<u64> {
        match item {
            Item::Material(m) => match self.material_overrides.get(m) {
                Some(v) => *v,
                None => item.trade_value(),
            },
            Item::Block(b) => match self.block_overrides.get(b) {
                Some(v) => *v,
                None => item.trade_value(),
            },
            // Tools / Plans / Armour aren't on the override map (per-
            // instance items). Fall back to the default.
            _ => item.trade_value(),
        }
    }

    /// Convert a trade-value to sats per the per-server rate.
    /// `None` = the item isn't tradeable; `Some(0)` = tradeable but
    /// sats-disabled (internal economy only).
    pub fn sats_for(&self, item: &Item) -> Option<u64> {
        // A chance drop never carries a sats value, whatever the operator's
        // override table says (Spec 06 §2.3).
        if crate::economy::is_chance_drop(item) {
            return None;
        }
        let value = self.trade_value(item)?;
        Some(value.saturating_mul(self.sats_per_unit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::MaterialId;

    #[test]
    fn default_config_has_no_overrides() {
        let cfg = ServerEconomyConfig::new();
        assert!(cfg.material_overrides.is_empty());
        assert!(cfg.block_overrides.is_empty());
        assert_eq!(cfg.sats_per_unit, 0);
    }

    #[test]
    fn default_falls_through_to_item_trade_value() {
        let cfg = ServerEconomyConfig::new();
        let cake = Item::Material(MaterialId::Cake);
        assert_eq!(cfg.trade_value(&cake), cake.trade_value());
    }

    #[test]
    fn material_override_replaces_default() {
        let mut cfg = ServerEconomyConfig::new();
        let wheat = Item::Material(MaterialId::Wheat);
        let default_val = wheat.trade_value().unwrap();
        cfg.override_material(MaterialId::Wheat, Some(default_val + 100));
        assert_eq!(cfg.trade_value(&wheat), Some(default_val + 100));
    }

    #[test]
    fn material_override_to_none_makes_untradeable() {
        let mut cfg = ServerEconomyConfig::new();
        cfg.override_material(MaterialId::Cake, None);
        assert_eq!(cfg.trade_value(&Item::Material(MaterialId::Cake)), None);
    }

    #[test]
    fn block_override_replaces_default() {
        let mut cfg = ServerEconomyConfig::new();
        let stone = Item::Block(crate::block::STONE);
        cfg.override_block(crate::block::STONE, Some(42));
        assert_eq!(cfg.trade_value(&stone), Some(42));
    }

    #[test]
    fn sats_default_zero_means_internal_economy_only() {
        let cfg = ServerEconomyConfig::new();
        let cake = Item::Material(MaterialId::Cake);
        // Tradeable but worth 0 sats — internal score only.
        assert_eq!(cfg.sats_for(&cake), Some(0));
    }

    #[test]
    fn sats_with_rate_multiplies_trade_value() {
        let mut cfg = ServerEconomyConfig::new();
        cfg.sats_per_unit = 10;
        let cake = Item::Material(MaterialId::Cake); // tier 5 = trade value 90
        let expected = 90u64 * 10;
        assert_eq!(cfg.sats_for(&cake), Some(expected));
    }

    #[test]
    fn untradeable_item_remains_untradeable_for_sats() {
        let cfg = ServerEconomyConfig::new();
        let water = Item::Block(crate::block::WATER);
        assert_eq!(water.trade_value(), None);
        assert_eq!(cfg.sats_for(&water), None);
    }

    #[test]
    fn sats_does_not_overflow_on_huge_rate() {
        let mut cfg = ServerEconomyConfig::new();
        cfg.sats_per_unit = u64::MAX;
        let cake = Item::Material(MaterialId::Cake);
        // saturating_mul should clamp at u64::MAX, not panic.
        let result = cfg.sats_for(&cake);
        assert!(result.is_some());
    }

    #[test]
    fn override_specific_items_without_disturbing_others() {
        let mut cfg = ServerEconomyConfig::new();
        cfg.override_material(MaterialId::Wheat, Some(999));
        // Wheat overridden, Cake still default.
        assert_eq!(cfg.trade_value(&Item::Material(MaterialId::Wheat)), Some(999));
        assert_eq!(
            cfg.trade_value(&Item::Material(MaterialId::Cake)),
            Item::Material(MaterialId::Cake).trade_value()
        );
    }

    #[test]
    fn tools_fall_through_to_default_trade_value() {
        // Tools aren't in the override map; should always use Item default.
        let cfg = ServerEconomyConfig::new();
        let pickaxe = Item::Tool(crate::crafting::Tool::new(
            crate::crafting::ToolType::Pickaxe,
            crate::crafting::ToolMaterial::Iron,
        ));
        assert_eq!(cfg.trade_value(&pickaxe), pickaxe.trade_value());
    }
}
