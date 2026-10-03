//! Spec 28e — Armour data layer.
//!
//! Four equipment slots (Helmet / Chestplate / Leggings / Boots) × five
//! material tiers (Leather / Iron / Diamond / Satori / Chainmail).
//! `armour_points(slot, mat)` gives the per-piece point value;
//! `damage_after_armour(raw, total_points)` runs the reduction formula
//! (4% per point, capped at 80%).
//!
//! Live wiring (inventory armour slots, equip-on-pickup, HUD bar,
//! durability decrement on hit, combat hookup) is deferred to a
//! follow-up PR that lands after Axolittle playtest on the maths.
//! This module is pure data + functions, ready to plug in.
//!
//! Spec: `docs/foundations/2026-05-20-tools-armour-roster.md`.

use serde::{Deserialize, Serialize};

/// Equipment slot. Each player has exactly one of each slot's worth of
/// armour equipped at a time. The four slots cover the player from head
/// to toe — Minecraft's canonical four — and bias damage by region in
/// proportion to how often a player gets hit there (chest > legs > head
/// > feet in real play).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArmourSlot {
    Helmet,
    Chestplate,
    Leggings,
    Boots,
}

/// Armour tier. Crafted tiers (Leather/Iron/Diamond/Satori) cover the
/// durable ladder; **Chainmail** is a never-craftable rarity tier with no
/// live drop source yet (its old fantasy-mob source was retired in the
/// 2026-05-24 roster excision — re-source from a kept mob if it's ever
/// wanted). Its point values sit between Iron and Diamond. The crafted
/// ladder mirrors the tool ladder exactly, so a player who's mining Diamond
/// is also already at Diamond armour without an extra resource hunt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArmourMaterial {
    Leather,
    Iron,
    Diamond,
    Satori,
    Chainmail,
    /// Rubber feature (2026-05-23). Boots-only material — the other
    /// armour slots intentionally don't have a Rubber recipe path.
    /// Equipping Rubber Boots multiplies the player's sprint speed
    /// (see `sprint_multiplier`).
    Rubber,
}

/// A single armour item. Per-instance (carries its own durability —
/// each piece wears independently). Per the spec the durability ladder
/// is decoupled from the point ladder so a Diamond Chestplate is more
/// durable than a Diamond Helmet, but both have the same per-point
/// reduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmourItem {
    pub slot: ArmourSlot,
    pub material: ArmourMaterial,
    /// Remaining durability. 0 = broken (caller should auto-unequip on
    /// the next damage event).
    pub durability: u16,
}

impl ArmourItem {
    pub fn new(slot: ArmourSlot, material: ArmourMaterial) -> Self {
        Self {
            slot,
            material,
            durability: max_durability(slot, material),
        }
    }

    /// Convenience — is this piece broken? Broken armour contributes
    /// zero points to total reduction and should be silently unequipped.
    pub fn is_broken(&self) -> bool {
        self.durability == 0
    }
}

/// Per-piece point value driving the reduction formula. Chest has the
/// most points by slot (chests get hit most); Helmet/Boots are roughly
/// equal at the low end; Leggings sit between. Satori is the top tier;
/// Chainmail sits between Iron and Diamond (its rarity-tier pedigree
/// justifies the awkward-yet-real placement).
///
/// Numbers tuned so a full Iron set (2+6+5+2 = 15 points) gives a
/// player a meaningful but not absurd survivability boost (15 × 4% =
/// 60% damage reduction). A full Satori set (4+9+7+4 = 24 points)
/// would exceed the 80% cap, so the cap is the actual ceiling — Satori
/// armour buys you cap + durability, not raw reduction.
pub fn armour_points(slot: ArmourSlot, mat: ArmourMaterial) -> u8 {
    use ArmourMaterial::*;
    use ArmourSlot::*;
    match (slot, mat) {
        (Helmet, Leather) => 1,
        (Chestplate, Leather) => 3,
        (Leggings, Leather) => 2,
        (Boots, Leather) => 1,

        (Helmet, Iron) => 2,
        (Chestplate, Iron) => 6,
        (Leggings, Iron) => 5,
        (Boots, Iron) => 2,

        (Helmet, Diamond) => 3,
        (Chestplate, Diamond) => 8,
        (Leggings, Diamond) => 6,
        (Boots, Diamond) => 3,

        (Helmet, Satori) => 4,
        (Chestplate, Satori) => 9,
        (Leggings, Satori) => 7,
        (Boots, Satori) => 4,

        (Helmet, Chainmail) => 2,
        (Chestplate, Chainmail) => 5,
        (Leggings, Chainmail) => 4,
        (Boots, Chainmail) => 1,

        // Rubber feature — boots-only material. The recipe table
        // (armour_material_from_slot in crafting.rs) resolves Rubber
        // for boots; the helmet / chestplate / leggings recipes fall
        // through and never produce a Rubber piece. These 0-point
        // arms exist only to satisfy the exhaustiveness checker — a
        // Rubber Helmet shouldn't be obtainable in normal play.
        (Helmet, Rubber) => 0,
        (Chestplate, Rubber) => 0,
        (Leggings, Rubber) => 0,
        (Boots, Rubber) => 2,
    }
}

/// Maximum durability per (slot, material). Chestplates and Leggings
/// last roughly 2× as long as Helmets/Boots because they absorb the
/// most hits. Tiers scale ~2× per step (matches the tool ladder shape,
/// so Iron armour feels durable in the same way Iron pickaxe feels
/// durable).
pub fn max_durability(slot: ArmourSlot, mat: ArmourMaterial) -> u16 {
    use ArmourMaterial::*;
    use ArmourSlot::*;
    let base: u16 = match mat {
        Leather => 55,
        Iron => 165,
        Chainmail => 165, // mirrors Iron — same hit-count tank
        Diamond => 363,
        Satori => 750,
        Rubber => 80, // between Leather (55) and Iron (165)
    };
    let slot_mult: u16 = match slot {
        // Chest gets the most hits → highest durability.
        Chestplate => 6,
        Leggings => 5,
        Helmet => 4,
        Boots => 4,
    };
    // Divide by a tunable so the ladder lands at familiar Minecraft-ish
    // numbers (Iron Chestplate ~ 165 × 6 / 6 = 165; Diamond Chestplate
    // ~363 × 6 / 6 = 363). The /6 keeps the formula symmetric — if a
    // future tier needs different scaling, override here.
    base.saturating_mul(slot_mult) / 6
}

/// The full set of armour points currently equipped, summed across all
/// non-broken slots. Pure: take an iterator of `Option<&ArmourItem>` —
/// caller passes its PlayerSlot.armour array.
pub fn total_armour_points<'a>(equipped: impl IntoIterator<Item = Option<&'a ArmourItem>>) -> u8 {
    let mut sum: u16 = 0;
    for piece in equipped {
        if let Some(a) = piece
            && !a.is_broken() {
                sum = sum.saturating_add(armour_points(a.slot, a.material) as u16);
            }
    }
    sum.min(255) as u8
}

/// Damage-reduction formula. 4% reduction per armour point, capped at
/// 80% (so even a full Satori set leaves the player taking 20% of the
/// raw damage — no godmode tier). Returns the damage value to actually
/// subtract from health. Pure.
pub fn damage_after_armour(raw_damage: f32, total_points: u8) -> f32 {
    let pct_reduction = ((total_points as f32) * 4.0).min(80.0);
    let multiplier = 1.0 - pct_reduction / 100.0;
    raw_damage * multiplier
}

/// All `(ArmourSlot, ArmourMaterial)` combos that exist as items. Used
/// by future inventory-explorer integration. Order: by material then
/// slot — keeps the explorer's All view legible.
pub fn all_armour_combos() -> Vec<(ArmourSlot, ArmourMaterial)> {
    use ArmourMaterial::*;
    use ArmourSlot::*;
    let mats = [Leather, Iron, Chainmail, Diamond, Satori];
    let slots = [Helmet, Chestplate, Leggings, Boots];
    let mut out = Vec::with_capacity(mats.len() * slots.len() + 1);
    for &m in &mats {
        for &s in &slots {
            out.push((s, m));
        }
    }
    // Rubber feature — boots-only single combo.
    out.push((Boots, Rubber));
    out
}

/// Display label for an `(slot, material)` pair. UK English ("Armour"
/// not "Armor", "Chestplate" not "Chestpiece", "Leggings" not "Pants").
pub fn armour_label(slot: ArmourSlot, mat: ArmourMaterial) -> &'static str {
    use ArmourMaterial::*;
    use ArmourSlot::*;
    match (mat, slot) {
        (Leather, Helmet) => "Leather Helmet",
        (Leather, Chestplate) => "Leather Chestplate",
        (Leather, Leggings) => "Leather Leggings",
        (Leather, Boots) => "Leather Boots",

        (Iron, Helmet) => "Iron Helmet",
        (Iron, Chestplate) => "Iron Chestplate",
        (Iron, Leggings) => "Iron Leggings",
        (Iron, Boots) => "Iron Boots",

        (Chainmail, Helmet) => "Chainmail Helmet",
        (Chainmail, Chestplate) => "Chainmail Chestplate",
        (Chainmail, Leggings) => "Chainmail Leggings",
        (Chainmail, Boots) => "Chainmail Boots",

        (Diamond, Helmet) => "Diamond Helmet",
        (Diamond, Chestplate) => "Diamond Chestplate",
        (Diamond, Leggings) => "Diamond Leggings",
        (Diamond, Boots) => "Diamond Boots",

        (Satori, Helmet) => "Satori Helmet",
        (Satori, Chestplate) => "Satori Chestplate",
        (Satori, Leggings) => "Satori Leggings",
        (Satori, Boots) => "Satori Boots",

        // Rubber feature — boots-only obtainable in normal play; the
        // other three arms exist to satisfy match exhaustiveness.
        (Rubber, Helmet) => "Rubber Helmet",
        (Rubber, Chestplate) => "Rubber Chestplate",
        (Rubber, Leggings) => "Rubber Leggings",
        (Rubber, Boots) => "Rubber Boots",
    }
}

/// Rubber feature — true if the player has Rubber boots equipped.
/// Drives the sprint-speed multiplier in the physics tick.
pub fn has_rubber_boots(equipped_boots: Option<ArmourMaterial>) -> bool {
    matches!(equipped_boots, Some(ArmourMaterial::Rubber))
}

/// Rubber feature — sprint-speed multiplier for the player. Applied to
/// `Player::sprint_boots_mult` (physics.rs) each tick from the equipped
/// Boots slot; 1.0 (no-op) when no Rubber boots are worn.
///
/// Capped at 1.4, NOT the originally-designed 2.0 (Task 15, 2026-07-07),
/// to stay clear of the server anti-cheat speed gate
/// (`MAX_HORIZONTAL_PER_TICK` in `GameServer::tick_player_physics`,
/// server.rs). That constant is `1.089 × 1.5 ≈ 1.6335` b/tick — and
/// 1.089 b/tick is FLY_SPRINT_SPEED (21.78 b/s) at 20 TPS, not grounded
/// SPRINT_SPEED (5.612 b/s → 0.2806 b/tick) as its label implies.
/// Against the gate as it actually stands, booted sprint (1.4 ×
/// 0.2806 ≈ 0.397 b/tick steady-state) plus the additive sprint-jump
/// boost (+0.2) peaks at ≈0.597 b/tick — about 37% of the 1.6335 gate,
/// generous headroom. HOWEVER: if the server constant is ever corrected
/// to its documented intent (SPRINT_SPEED-derived: 0.2806 × 1.5 ≈ 0.42
/// b/tick), booted sprint-jump (≈0.597) — and even a bare, no-boots
/// sprint-jump (≈0.484) — would exceed it, so this multiplier (and the
/// gate's handling of the sprint-jump boost) must be revisited together
/// with that change. 1.4 was chosen over raising the server cap, which
/// would widen the window for a real speed-hack.
pub fn sprint_multiplier(equipped_boots: Option<ArmourMaterial>) -> f32 {
    if has_rubber_boots(equipped_boots) { 1.4 } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ArmourMaterial::*;
    use ArmourSlot::*;

    #[test]
    fn point_table_exhaustive_across_all_combos() {
        // Every (slot, material) pair returns a positive point value.
        for (s, m) in all_armour_combos() {
            assert!(armour_points(s, m) > 0,
                "armour_points({:?}, {:?}) must be > 0", s, m);
        }
    }

    #[test]
    fn chestplate_is_strongest_slot_for_every_tier() {
        // The slot ladder rule: Chestplate ≥ Leggings ≥ Helmet ≥ Boots
        // per tier (matches the "where do you get hit most" intuition).
        for m in [Leather, Iron, Chainmail, Diamond, Satori] {
            assert!(armour_points(Chestplate, m) >= armour_points(Leggings, m),
                "chestplate < leggings for {:?}", m);
            assert!(armour_points(Leggings, m) >= armour_points(Helmet, m),
                "leggings < helmet for {:?}", m);
        }
    }

    #[test]
    fn material_ladder_monotonic_per_slot() {
        // For each slot, the material ladder is non-decreasing through
        // Leather → Iron → Diamond → Satori. Chainmail sits between
        // Iron and Diamond per spec.
        for s in [Helmet, Chestplate, Leggings, Boots] {
            let l = armour_points(s, Leather);
            let i = armour_points(s, Iron);
            let c = armour_points(s, Chainmail);
            let d = armour_points(s, Diamond);
            let sa = armour_points(s, Satori);
            assert!(l <= i, "leather > iron for {:?}", s);
            // Chainmail sits between Iron and Diamond (inclusive).
            assert!(i >= c && c <= d, "chainmail out of (iron..diamond) for {:?}", s);
            assert!(d <= sa, "diamond > satori for {:?}", s);
        }
    }

    #[test]
    fn full_leather_set_has_seven_points() {
        let pieces = [
            ArmourItem::new(Helmet, Leather),
            ArmourItem::new(Chestplate, Leather),
            ArmourItem::new(Leggings, Leather),
            ArmourItem::new(Boots, Leather),
        ];
        let opts: Vec<Option<&ArmourItem>> = pieces.iter().map(Some).collect();
        let total = total_armour_points(opts);
        // 1 + 3 + 2 + 1 = 7
        assert_eq!(total, 7);
    }

    #[test]
    fn full_iron_set_yields_60_percent_reduction() {
        // 2 + 6 + 5 + 2 = 15 points × 4% = 60% reduction.
        let pieces = [
            ArmourItem::new(Helmet, Iron),
            ArmourItem::new(Chestplate, Iron),
            ArmourItem::new(Leggings, Iron),
            ArmourItem::new(Boots, Iron),
        ];
        let opts: Vec<Option<&ArmourItem>> = pieces.iter().map(Some).collect();
        let total = total_armour_points(opts);
        assert_eq!(total, 15);
        // Damage maths.
        let after = damage_after_armour(10.0, total);
        assert!((after - 4.0).abs() < 0.01, "expected 4.0 hp through Iron set, got {after}");
    }

    #[test]
    fn full_satori_set_caps_at_eighty_percent() {
        // 4 + 9 + 7 + 4 = 24 points × 4% = 96% raw — capped at 80%.
        let pieces = [
            ArmourItem::new(Helmet, Satori),
            ArmourItem::new(Chestplate, Satori),
            ArmourItem::new(Leggings, Satori),
            ArmourItem::new(Boots, Satori),
        ];
        let opts: Vec<Option<&ArmourItem>> = pieces.iter().map(Some).collect();
        let total = total_armour_points(opts);
        assert_eq!(total, 24);
        let after = damage_after_armour(10.0, total);
        // 20% of 10.0 = 2.0
        assert!((after - 2.0).abs() < 0.01,
            "expected 2.0 hp through capped Satori, got {after}");
    }

    #[test]
    fn broken_armour_contributes_zero_points() {
        let mut helmet = ArmourItem::new(Helmet, Diamond);
        let mut chest = ArmourItem::new(Chestplate, Diamond);
        helmet.durability = 0;
        chest.durability = 0;
        let pieces = [Some(&helmet), Some(&chest), None, None];
        assert_eq!(total_armour_points(pieces), 0);
    }

    #[test]
    fn no_armour_means_full_damage() {
        let pieces: Vec<Option<&ArmourItem>> = vec![None, None, None, None];
        let total = total_armour_points(pieces);
        assert_eq!(total, 0);
        assert_eq!(damage_after_armour(8.0, total), 8.0);
    }

    #[test]
    fn one_helmet_alone_gives_proportional_reduction() {
        // Iron helmet alone = 2 points = 8% reduction.
        let helmet = ArmourItem::new(Helmet, Iron);
        let pieces = [Some(&helmet), None, None, None];
        let total = total_armour_points(pieces);
        assert_eq!(total, 2);
        let after = damage_after_armour(10.0, total);
        assert!((after - 9.2).abs() < 0.01);
    }

    #[test]
    fn durability_scales_with_chest_being_tankiest() {
        // For Iron tier, Chestplate has the highest durability.
        let h = max_durability(Helmet, Iron);
        let c = max_durability(Chestplate, Iron);
        let l = max_durability(Leggings, Iron);
        let b = max_durability(Boots, Iron);
        assert!(c >= l, "chestplate {} < leggings {}", c, l);
        assert!(l >= h, "leggings {} < helmet {}", l, h);
        assert!(c > b, "chestplate {} not > boots {}", c, b);
    }

    #[test]
    fn material_durability_ladder_monotonic() {
        // Leather → Iron/Chainmail → Diamond → Satori across the same slot.
        let l = max_durability(Chestplate, Leather);
        let i = max_durability(Chestplate, Iron);
        let c = max_durability(Chestplate, Chainmail);
        let d = max_durability(Chestplate, Diamond);
        let sa = max_durability(Chestplate, Satori);
        assert!(l < i, "leather durability >= iron");
        assert_eq!(i, c, "chainmail and iron share durability tier");
        assert!(c < d, "chainmail durability >= diamond");
        assert!(d < sa, "diamond durability >= satori");
    }

    #[test]
    fn freshly_crafted_item_is_at_max_durability() {
        let item = ArmourItem::new(Chestplate, Iron);
        assert_eq!(item.durability, max_durability(Chestplate, Iron));
        assert!(!item.is_broken());
    }

    #[test]
    fn all_armour_combos_count_matches_grid() {
        let combos = all_armour_combos();
        // 5 standard materials × 4 slots + Rubber-Boots only = 21.
        assert_eq!(combos.len(), 4 * 5 + 1);
    }

    #[test]
    fn labels_are_uk_english() {
        assert_eq!(armour_label(Chestplate, Iron), "Iron Chestplate");
        assert_eq!(armour_label(Boots, Satori), "Satori Boots");
        // No "Armor"-suffixed labels — UK English throughout.
        for (s, m) in all_armour_combos() {
            let label = armour_label(s, m);
            assert!(!label.contains("Armor"), "label contains US spelling: {label}");
        }
    }

    #[test]
    fn damage_after_armour_never_negative() {
        // High point counts shouldn't produce negative damage.
        let result = damage_after_armour(10.0, 200);
        assert!(result >= 0.0, "damage went negative: {result}");
        // And the cap is 80% — minimum 20% of raw gets through.
        assert!((result - 2.0).abs() < 0.01,
            "even with 200 points, 20% of 10 = 2 should land — got {result}");
    }
}
