//! Server Bazaar v1 — the trade-value sell-floor (Spec 39).
//!
//! A stateless, server-run merchant: a player sells any held stack
//! for its `trade_value × count` in sats. The market-of-last-resort
//! that gives every item guaranteed liquidity. No owner, no block-
//! entity, no save state — just a pure quote off the item.
//!
//! v1 is sell-only; the buy-side (Bazaar sells at trade_value × 1.5)
//! needs a catalogue UI and is deferred to v2.
//!
//! Spec: `docs/foundations/2026-05-23-server-bazaar.md`.

use crate::item::ItemStack;

/// What the Bazaar pays for a stack: the item's `trade_value × count`.
/// `None` when the stack is empty (count 0) or the item isn't
/// tradeable (AIR / WATER → `trade_value()` returns None).
pub fn sell_quote(stack: &ItemStack) -> Option<u64> {
    if stack.count == 0 {
        return None;
    }
    // A chance drop (Satori, or anything crafted from it) never sells for sats (Spec 06 §2.3).
    if crate::economy::is_chance_drop(&stack.item) {
        return None;
    }
    let unit = stack.item.trade_value()?;
    Some(unit.saturating_mul(stack.count as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, MaterialId};

    #[test]
    fn sell_quote_scales_with_count() {
        // IronIngot is tier 1 → trade_value 3. A stack of 5 → 15.
        let one = ItemStack { item: Item::Material(MaterialId::IronIngot), count: 1 };
        let five = ItemStack { item: Item::Material(MaterialId::IronIngot), count: 5 };
        let unit = one.item.trade_value().unwrap();
        assert_eq!(sell_quote(&one), Some(unit));
        assert_eq!(sell_quote(&five), Some(unit * 5));
    }

    #[test]
    fn sell_quote_none_for_air() {
        let air = ItemStack { item: Item::Block(crate::block::AIR), count: 1 };
        assert_eq!(sell_quote(&air), None);
    }

    #[test]
    fn sell_quote_none_for_zero_count() {
        let empty = ItemStack { item: Item::Material(MaterialId::IronIngot), count: 0 };
        assert_eq!(sell_quote(&empty), None);
    }

    #[test]
    fn sell_quote_some_for_a_sample_of_real_items() {
        // Every real item has a floor — spot-check across tiers.
        for m in [
            MaterialId::Stick,        // tier 0 → 1
            MaterialId::IronIngot,    // tier 1 → 3
            MaterialId::Cheese,       // tier 2 → 8
            MaterialId::Cake,         // tier 5 → 90
        ] {
            let s = ItemStack { item: Item::Material(m), count: 1 };
            assert!(sell_quote(&s).is_some(), "{m:?} should have a sell floor");
            assert!(sell_quote(&s).unwrap() > 0);
        }
    }

    #[test]
    fn sell_quote_higher_tier_pays_more() {
        let stick = ItemStack { item: Item::Material(MaterialId::Stick), count: 1 };
        let cake = ItemStack { item: Item::Material(MaterialId::Cake), count: 1 };
        assert!(sell_quote(&cake).unwrap() > sell_quote(&stick).unwrap(),
            "a higher-complexity item must fetch a higher floor");
    }
}
