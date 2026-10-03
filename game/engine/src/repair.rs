//! Tool Repair — the economy's first sat sink (Spec 35).
//!
//! A damaged tool is restored at a Repair Bench by spending its tier
//! material (the item sink) + a sats repair tax (the sat sink). Pure
//! cost + restore helpers; the UI (`repair_ui.rs`) drives them and
//! the game-loop applies the sats payout + Charter/Vow gate.
//!
//! v1 supports only the metal + gem tiers (Iron / Diamond / Satori) —
//! their repair material is a clean `MaterialId`. Wood + Stone tier
//! tools repair with blocks (planks / cobblestone), which don't map
//! to a `MaterialId`; they're cheap to recraft, so v1 skips them.
//!
//! Spec: `docs/foundations/2026-05-23-tool-repair-anvil.md`.

use crate::crafting::{Tool, ToolMaterial};
use crate::item::MaterialId;

/// Sats charged per point of durability restored. The repair tax is
/// the sat SINK. Server-tunable; this is the alpha default.
pub const REPAIR_TAX_SATS_PER_POINT: u64 = 1;

/// How many durability points one unit of the tier material restores.
pub const RESTORE_PER_MATERIAL: u16 = 50;

/// The `MaterialId` that repairs a given tool, or `None` for tools
/// v1 doesn't support (Wood / Stone tier — block-crafted; utility
/// tools — no tier material).
pub fn repair_material_for(tool: &Tool) -> Option<MaterialId> {
    match tool.material {
        ToolMaterial::Iron => Some(MaterialId::IronIngot),
        ToolMaterial::Diamond => Some(MaterialId::Diamond),
        ToolMaterial::Satori => Some(MaterialId::Satori),
        // Wood + Stone repair with planks / cobblestone (blocks, not
        // MaterialIds) — out of scope for v1. Cheap to recraft anyway.
        ToolMaterial::Wood | ToolMaterial::Stone => None,
    }
}

/// A repair quote — what one repair action would cost + restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepairQuote {
    pub durability_restored: u16,
    pub material_consumed: u16,
    pub sats_tax: u64,
}

impl RepairQuote {
    /// A no-op quote (tool already full, or no material available).
    pub fn none() -> Self {
        RepairQuote { durability_restored: 0, material_consumed: 0, sats_tax: 0 }
    }
    pub fn is_noop(&self) -> bool {
        self.durability_restored == 0
    }
}

/// Compute a repair quote. Given the tool's current + max durability
/// and the material units the player has available, returns how much
/// durability would be restored, how much material consumed, and the
/// sats tax. Restoration is capped at the tool's max durability —
/// never over-repairs, and never consumes more material than needed
/// to reach full.
pub fn repair_quote(current: u16, max: u16, material_available: u16) -> RepairQuote {
    if current >= max || material_available == 0 {
        return RepairQuote::none();
    }
    let missing = max - current;
    // Material units needed to fully repair, rounded up.
    let units_for_full =
        missing.div_ceil(RESTORE_PER_MATERIAL);
    let units = units_for_full.min(material_available);
    // Durability this many units would restore, capped at `missing`.
    let restored = (units.saturating_mul(RESTORE_PER_MATERIAL)).min(missing);
    let sats_tax = restored as u64 * REPAIR_TAX_SATS_PER_POINT;
    RepairQuote {
        durability_restored: restored,
        material_consumed: units,
        sats_tax,
    }
}

/// Apply a quote to a tool in-place. Pure — caller handles the sats
/// payout + material decrement + the Charter/Vow gate. Clamps at the
/// tool's max durability defensively.
pub fn apply_repair(tool: &mut Tool, quote: &RepairQuote) {
    let max = tool.max_durability();
    tool.durability = (tool.durability + quote.durability_restored).min(max);
}

/// Execute a repair against the **live** inventory: re-derive the quote from
/// the tool currently in `hotbar_slot` and the player's *current* material
/// count, consume exactly that material, and restore the matching durability.
/// Returns the applied [`RepairQuote`], or `None` if the slot no longer holds a
/// repairable damaged tool or the player has no material.
///
/// Re-deriving at apply time — rather than trusting a render-time snapshot — is
/// what closes the free/partial repair: the old game-loop path consumed
/// `min(quoted, available)` material but always restored the *full quoted*
/// durability, so a short/empty balance bought a full repair (engine audit
/// 2026-06-04, A). Because the fresh quote's `material_consumed` is always
/// `<= available`, the restore here is always fully paid for.
pub fn execute_repair(
    inventory: &mut crate::inventory::Inventory,
    hotbar_slot: usize,
) -> Option<RepairQuote> {
    let (cur, max, mat) = {
        let stack = inventory.hotbar_slot(hotbar_slot)?;
        let tool = match &stack.item {
            crate::item::Item::Tool(t) => t,
            _ => return None,
        };
        let mat = repair_material_for(tool)?;
        (tool.durability, tool.max_durability(), mat)
    };
    let available = inventory.count_material(mat);
    let quote = repair_quote(cur, max, available);
    if quote.is_noop() {
        return None;
    }
    let consumed = inventory.consume_material(mat, quote.material_consumed);
    debug_assert_eq!(consumed, quote.material_consumed,
        "fresh quote never consumes more than is available");
    if let Some(stack) = inventory.hotbar_slot_mut(hotbar_slot)
        && let crate::item::Item::Tool(t) = &mut stack.item {
            apply_repair(t, &quote);
        }
    Some(quote)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};

    #[test]
    fn repair_material_for_iron_tool_is_iron() {
        let t = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        assert_eq!(repair_material_for(&t), Some(MaterialId::IronIngot));
    }

    #[test]
    fn repair_material_for_diamond_and_satori() {
        let d = Tool::new(ToolType::Pickaxe, ToolMaterial::Diamond);
        assert_eq!(repair_material_for(&d), Some(MaterialId::Diamond));
        let s = Tool::new(ToolType::Pickaxe, ToolMaterial::Satori);
        assert_eq!(repair_material_for(&s), Some(MaterialId::Satori));
    }

    #[test]
    fn repair_material_for_wood_and_stone_is_none() {
        let w = Tool::new(ToolType::Pickaxe, ToolMaterial::Wood);
        assert_eq!(repair_material_for(&w), None);
        let s = Tool::new(ToolType::Pickaxe, ToolMaterial::Stone);
        assert_eq!(repair_material_for(&s), None);
    }

    #[test]
    fn repair_quote_zero_when_already_full() {
        let q = repair_quote(250, 250, 10);
        assert!(q.is_noop());
        assert_eq!(q.material_consumed, 0);
        assert_eq!(q.sats_tax, 0);
    }

    #[test]
    fn repair_quote_zero_when_no_material() {
        let q = repair_quote(10, 250, 0);
        assert!(q.is_noop());
    }

    #[test]
    fn repair_quote_caps_at_max_durability() {
        // Missing 30; one unit restores 50 → cap at 30, consume 1 unit.
        let q = repair_quote(220, 250, 5);
        assert_eq!(q.durability_restored, 30);
        assert_eq!(q.material_consumed, 1);
        assert_eq!(q.sats_tax, 30 * REPAIR_TAX_SATS_PER_POINT);
    }

    #[test]
    fn repair_quote_scales_with_material() {
        // Missing 250 (fully broken iron, max 250). Needs ceil(250/50)=5
        // units; player has 3 → restore 150, consume 3.
        let q = repair_quote(0, 250, 3);
        assert_eq!(q.durability_restored, 150);
        assert_eq!(q.material_consumed, 3);
        assert_eq!(q.sats_tax, 150 * REPAIR_TAX_SATS_PER_POINT);
    }

    #[test]
    fn repair_quote_full_repair_with_enough_material() {
        // Missing 250, player has 5 units → exactly full.
        let q = repair_quote(0, 250, 5);
        assert_eq!(q.durability_restored, 250);
        assert_eq!(q.material_consumed, 5);
    }

    #[test]
    fn apply_repair_restores_durability() {
        let mut t = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        t.durability = 50;
        let q = repair_quote(t.durability, t.max_durability(), 2);
        apply_repair(&mut t, &q);
        assert_eq!(t.durability, 150); // 50 + 2*50
    }

    #[test]
    fn apply_repair_never_exceeds_max() {
        let mut t = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        t.durability = 240; // max 250, missing 10
        let q = repair_quote(t.durability, t.max_durability(), 5);
        apply_repair(&mut t, &q);
        assert_eq!(t.durability, 250, "must clamp at max");
    }

    #[test]
    fn execute_repair_only_restores_what_live_material_pays_for() {
        use crate::inventory::Inventory;
        use crate::item::{Item, ItemStack};
        // Fully-broken iron pickaxe (0/250). A stale render-time quote would
        // have promised a full 250 restore for 5 iron — but the player now
        // holds only 2. Re-deriving at apply must restore 100 (2 units), not a
        // free 250 (engine audit 2026-06-04, A: free/partial repair).
        let mut inv = Inventory::new();
        let mut t = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        t.durability = 0;
        inv.set_slot(0, Some(ItemStack::new_tool(t)));
        let _ = inv.add_item(ItemStack::new_material(MaterialId::IronIngot, 2));

        let applied = execute_repair(&mut inv, 0).expect("repairable tool present");
        assert_eq!(applied.durability_restored, 100, "2 iron → 100, not a free 250");
        assert_eq!(applied.material_consumed, 2);
        let dur = match &inv.hotbar_slot(0).unwrap().item {
            Item::Tool(t) => t.durability,
            _ => panic!("slot 0 should still hold the tool"),
        };
        assert_eq!(dur, 100, "durability moved by exactly the restored amount");
        assert_eq!(inv.count_material(MaterialId::IronIngot), 0, "all live iron consumed");
    }

    #[test]
    fn execute_repair_noop_when_no_material_or_full() {
        use crate::inventory::Inventory;
        use crate::item::ItemStack;
        let mut inv = Inventory::new();
        let mut t = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        t.durability = 10;
        inv.set_slot(0, Some(ItemStack::new_tool(t))); // damaged but no iron
        assert!(execute_repair(&mut inv, 0).is_none(), "no material → no repair, no consume");
    }

    #[test]
    fn tool_max_durability_matches_new() {
        // The method should agree with the durability a fresh tool
        // starts at, for every material on a laddered tool.
        for mat in [
            ToolMaterial::Wood, ToolMaterial::Stone, ToolMaterial::Iron,
            ToolMaterial::Diamond, ToolMaterial::Satori,
        ] {
            let t = Tool::new(ToolType::Pickaxe, mat);
            assert_eq!(t.max_durability(), t.durability,
                "fresh {mat:?} pickaxe durability should equal its max");
        }
    }
}
