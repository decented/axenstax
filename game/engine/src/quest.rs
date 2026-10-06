//! Spec 19 phase 6 — quest data model + per-profession pools + completion
//! detection.
//!
//! Each villager offers exactly one quest at a time, drawn from a pool keyed
//! on their `Profession`. Quests have one of three flavours:
//!   * `Fetch` — bring N of an item. Resolved at re-dialogue time.
//!   * `Make`  — craft N of an item. Same resolution as Fetch (we don't track
//!     "crafted vs found" in alpha; the kid checks their inventory).
//!   * `Kill`  — slay N of a mob kind. Tracked via `PlayerSlot.kill_counter`.
//!
//! Phase 7 wires the reward payout; this phase ships the data model + pool
//! + generation + completion-check helpers.

use serde::{Deserialize, Serialize};

use crate::item::{ItemStack, MaterialId};
use crate::mob::MobType;
use crate::villager::Profession;

/// What a quest asks the player to do.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum QuestFlavour {
    Fetch { item: MaterialId, count: u8 },
    Make  { item: MaterialId, count: u8 },
    Kill  { mob: MobType, count: u8 },
}

/// What the villager pays out on success.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuestReward {
    pub items: Vec<ItemStack>,
    /// Satoshi payout — honoured on Bitcoin-enabled servers when the
    /// guardian flag allows it. Off-Bitcoin or guardian-disabled players
    /// still see the quest succeed; the sats line is just suppressed.
    pub sats: u64,
    /// Reputation delta with the village. Spec 19 phase 9 tier system.
    pub reputation: i16,
}

/// One pending or accepted quest. `accepted_by` is the player-slot index on
/// alpha (single-player + split-screen); multiplayer-pubkey identity moves in
/// when Spec 1 phase 4 lands.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quest {
    pub flavour: QuestFlavour,
    pub reward: QuestReward,
    pub accepted_by: Option<u32>,
}

/// Deterministically generate a quest offer for the given profession.
/// `seed` typically derived from `(world_tick, villager_id)` so the same
/// villager offers the same quest within a single talk session, but a fresh
/// quest after re-engagement (decline-and-return).
pub fn generate_quest(profession: Profession, seed: u32) -> Option<Quest> {
    let table = pool_for(profession)?;
    if table.is_empty() {
        return None;
    }
    let idx = (seed as usize) % table.len();
    Some(table[idx].clone())
}

/// Produce a human-readable one-liner for the dialogue UI. `sats_visible` is
/// [`crate::economy::sats_ui_visible`] for the viewing player: while it's false
/// (always, on the web taster) the sats term is dropped from the reward text.
pub fn summarise(q: &Quest, sats_visible: bool) -> String {
    let action = match q.flavour {
        QuestFlavour::Fetch { item, count } => {
            format!("Bring me {} × {}", count, item_name(item))
        }
        QuestFlavour::Make { item, count } => {
            format!("Make me {} × {}", count, item_name(item))
        }
        QuestFlavour::Kill { mob, count } => {
            format!("Kill {} × {}", count, mob_name(mob))
        }
    };
    let mut reward = Vec::new();
    for stack in &q.reward.items {
        reward.push(format!("{}× {}", stack.count, stack_name(stack)));
    }
    if sats_visible && q.reward.sats > 0 {
        reward.push(format!("{} sats", q.reward.sats));
    }
    if q.reward.reputation != 0 {
        reward.push(format!("{:+} rep", q.reward.reputation));
    }
    let reward = if reward.is_empty() {
        "(no reward)".to_string()
    } else {
        reward.join(" + ")
    };
    format!("{action}. Reward: {reward}.")
}

/// How much the player has progressed toward completing `q`.
/// Returns `(progress, target)`.
pub fn progress_against(q: &Quest, inv: &crate::inventory::Inventory, kill_count: u32) -> (u32, u32) {
    match q.flavour {
        QuestFlavour::Fetch { item, count } | QuestFlavour::Make { item, count } => {
            let have = count_material(inv, item);
            (have.min(count as u32), count as u32)
        }
        QuestFlavour::Kill { count, .. } => {
            (kill_count.min(count as u32), count as u32)
        }
    }
}

/// True when the player has met or exceeded the quest's target.
pub fn is_complete(q: &Quest, inv: &crate::inventory::Inventory, kill_count: u32) -> bool {
    let (have, need) = progress_against(q, inv, kill_count);
    have >= need
}

/// The reward-payout gate for `DialogueAction::TurnIn`. The payout MUST pass
/// this before paying — it re-checks completion against the player's CURRENT
/// inventory + kill count, so a player can't accept a quest, satisfy it, drop
/// the items (or have the kill undone), and still collect the reward. The
/// previous TurnIn path discarded `fetch_make_consume`'s result and paid
/// unconditionally (engine audit 2026-06-04, A: quest reward without re-check).
/// Currently equivalent to [`is_complete`], but named as the single gate so the
/// payout path's precondition is explicit and protected by its own test.
pub fn may_turn_in(q: &Quest, inv: &crate::inventory::Inventory, kill_count: u32) -> bool {
    is_complete(q, inv, kill_count)
}

/// On accept, claim the resources the quest needs FROM the player's
/// inventory iff it's a Fetch/Make and they already have them — Spec 19
/// phase 7 will wire this into the payout. For now, this helper exists so
/// callers can preview the resource cost.
pub fn fetch_make_consume(q: &Quest, inv: &mut crate::inventory::Inventory) -> bool {
    let (item, count) = match q.flavour {
        QuestFlavour::Fetch { item, count } => (item, count),
        QuestFlavour::Make { item, count } => (item, count),
        QuestFlavour::Kill { .. } => return true, // nothing to consume
    };
    if count_material(inv, item) < count as u32 {
        return false;
    }
    let mut remaining = count as u32;
    for i in 0..36 {
        if remaining == 0 {
            break;
        }
        let take = inv.slot(i).cloned();
        if let Some(stack) = take
            && let crate::item::Item::Material(id) = stack.item
                && id == item {
                    if stack.count as u32 <= remaining {
                        remaining -= stack.count as u32;
                        inv.set_slot(i, None);
                    } else {
                        let new_count = stack.count - remaining as u8;
                        remaining = 0;
                        inv.set_slot(i, Some(ItemStack {
                            item: stack.item.clone(),
                            count: new_count,
                        }));
                    }
                }
    }
    true
}

fn count_material(inv: &crate::inventory::Inventory, want: MaterialId) -> u32 {
    let mut total = 0u32;
    for i in 0..36 {
        if let Some(stack) = inv.slot(i)
            && let crate::item::Item::Material(id) = stack.item
                && id == want {
                    total += stack.count as u32;
                }
    }
    total
}

fn item_name(id: MaterialId) -> &'static str {
    match id {
        MaterialId::Wheat => "Wheat",
        MaterialId::Bread => "Bread",
        MaterialId::Carrot => "Carrot",
        MaterialId::Potato => "Potato",
        MaterialId::Stick => "Stick",
        MaterialId::Wool => "Wool",
        MaterialId::Bone => "Bone",
        MaterialId::Leather => "Leather",
        MaterialId::Feather => "Feather",
        MaterialId::String => "String",
        MaterialId::IronIngot => "Iron Ingot",
        MaterialId::Coal => "Coal",
        MaterialId::RawBeef => "Raw Beef",
        MaterialId::RawChicken => "Raw Chicken",
        MaterialId::RawMutton => "Raw Mutton",
        MaterialId::RawPorkchop => "Raw Pork",
        MaterialId::CookedBeef => "Cooked Beef",
        MaterialId::CookedChicken => "Cooked Chicken",
        MaterialId::CookedMutton => "Cooked Mutton",
        MaterialId::CookedPorkchop => "Cooked Pork",
        // Historical Pivot Sub 7 — labels for the Miller/Baker/Brewer
        // quest pools so summarise() reads cleanly.
        MaterialId::Berries => "Berries",
        MaterialId::HoneyBottle => "Honey Jar",
        _ => "Mystery Item",
    }
}

fn mob_name(kind: MobType) -> &'static str {
    crate::mob::mob_def(kind).name.as_str()
}

fn stack_name(stack: &ItemStack) -> String {
    match &stack.item {
        crate::item::Item::Material(id) => item_name(*id).to_string(),
        crate::item::Item::Block(id) => format!("Block #{id}"),
        crate::item::Item::Tool(tool) => format!("{:?}", tool.tool_type),
        crate::item::Item::Plan(p) => p.name.clone(),
        crate::item::Item::Armour(a) => format!("{:?} {:?}", a.material, a.slot),
    }
}

// --- Profession quest pools ---

fn pool_for(prof: Profession) -> Option<&'static [Quest]> {
    match prof {
        Profession::Farmer => Some(FARMER_POOL.as_slice()),
        Profession::Cook => Some(COOK_POOL.as_slice()),
        Profession::Carpenter => Some(CARPENTER_POOL.as_slice()),
        Profession::Blacksmith => Some(BLACKSMITH_POOL.as_slice()),
        Profession::Scribe => Some(SCRIBE_POOL.as_slice()),
        // Spec 26 — Builder profession has no quest pool on alpha;
        // the Builder offers commissions (Phases 5+) instead of
        // quests. None => "no offer" through the existing fall-back.
        Profession::Builder => None,
        // Historical Pivot Sub 7 (2026-05-23) — T1.5 trades.
        Profession::Miller => Some(MILLER_POOL.as_slice()),
        Profession::Baker => Some(BAKER_POOL.as_slice()),
        Profession::Brewer => Some(BREWER_POOL.as_slice()),
        Profession::None => None,
    }
}

// `Vec<ItemStack>` is not const-constructible, so the pools live behind
// `LazyLock` rather than `const`. Built once at first use; immutable
// thereafter. `accepted_by: None` template — the dialogue path clones the
// template and stamps the slot index on accept.
fn fetch(item: MaterialId, count: u8, sats: u64, rep: i16) -> Quest {
    Quest {
        flavour: QuestFlavour::Fetch { item, count },
        reward: QuestReward { items: Vec::new(), sats, reputation: rep },
        accepted_by: None,
    }
}
fn make(item: MaterialId, count: u8, sats: u64, rep: i16) -> Quest {
    Quest {
        flavour: QuestFlavour::Make { item, count },
        reward: QuestReward { items: Vec::new(), sats, reputation: rep },
        accepted_by: None,
    }
}
fn kill(mob: MobType, count: u8, sats: u64, rep: i16) -> Quest {
    Quest {
        flavour: QuestFlavour::Kill { mob, count },
        reward: QuestReward { items: Vec::new(), sats, reputation: rep },
        accepted_by: None,
    }
}

static FARMER_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::Wheat, 5, 12, 5),
    fetch(MaterialId::Carrot, 8, 14, 6),
    fetch(MaterialId::Potato, 8, 14, 6),
    kill(MobType::Brigand, 3, 18, 8),
]);
static COOK_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::RawBeef, 4, 15, 7),
    fetch(MaterialId::Wheat, 6, 12, 5),
    make(MaterialId::Bread, 3, 20, 10),
]);
static CARPENTER_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::Stick, 16, 10, 5),
    fetch(MaterialId::Wool, 4, 16, 7),
    kill(MobType::Brigand, 2, 18, 8),
]);
static BLACKSMITH_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::IronIngot, 3, 25, 10),
    fetch(MaterialId::Coal, 6, 18, 8),
]);
static SCRIBE_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::Feather, 3, 14, 6),
    kill(MobType::Marauder, 4, 22, 10),
]);

// Historical Pivot Sub 7 (2026-05-23). Entries swapped from the
// spec sketch because Flour / Cake / Pancakes / BeetrootSoup have no
// craftable or harvestable path in survival on alpha — Mill / Oven /
// Aging Rack workstation-interaction recipes are deferred per
// `item.rs` §"T1.5 Processed Economy Base". Pools below restrict to
// materials the player can actually obtain today (Wheat, Bread,
// Berries) — plus HoneyBottle since 2026-07-04 (wild hives + Bucket
// harvest are live; the guard test below pins the still-unreachable set).
static MILLER_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::Wheat, 8, 14, 6),
    fetch(MaterialId::Wheat, 16, 22, 9),
    make(MaterialId::Bread, 4, 22, 9),
    make(MaterialId::Bread, 8, 35, 14),
]);
static BAKER_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::Wheat, 6, 12, 5),
    fetch(MaterialId::Bread, 3, 18, 8),
    make(MaterialId::Bread, 3, 20, 10),
    make(MaterialId::Bread, 6, 32, 14),
]);
static BREWER_POOL: std::sync::LazyLock<Vec<Quest>> = std::sync::LazyLock::new(|| vec![
    fetch(MaterialId::Berries, 8, 16, 7),
    fetch(MaterialId::Berries, 16, 26, 11),
    // HoneyBottle RESTORED (2026-07-04): the hive loop is live — wild hives
    // spawn on ~1 in 12 trees, honey accumulates while bees work nearby, and
    // right-click + Bucket harvests a Honey Jar. (Removed 2026-06-22 while
    // unobtainable; see the guard test for the still-unreachable list.)
    fetch(MaterialId::HoneyBottle, 2, 30, 13),
    make(MaterialId::Bread, 4, 44, 17),
]);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::Inventory;

    #[test]
    fn pool_lookup_returns_at_least_one_quest_per_employed_profession() {
        for prof in [Profession::Farmer, Profession::Cook, Profession::Carpenter,
                     Profession::Blacksmith, Profession::Scribe,
                     // Historical Pivot Sub 7 — T1.5 trades.
                     Profession::Miller, Profession::Baker, Profession::Brewer] {
            let pool = pool_for(prof).expect("expected non-empty pool");
            assert!(!pool.is_empty(), "{prof:?} pool empty");
        }
        assert!(pool_for(Profession::None).is_none());
    }

    #[test]
    fn may_turn_in_blocks_after_required_items_dropped() {
        // Exploit: accept a Fetch quest, satisfy it, then drop the items before
        // clicking Turn In. The payout gate must re-check + reject (engine audit
        // 2026-06-04, A: reward paid without completion re-check).
        let q = fetch(MaterialId::Wheat, 5, 12, 5);
        let mut inv = crate::inventory::Inventory::new();
        inv.set_slot(0, Some(crate::item::ItemStack::new_material(MaterialId::Wheat, 5)));
        assert!(may_turn_in(&q, &inv, 0), "turn-in-able while holding the 5 wheat");
        inv.set_slot(0, None); // player drops the wheat after the dialogue opened
        assert!(!may_turn_in(&q, &inv, 0), "gate rejects once the items are gone");
        assert!(!fetch_make_consume(&q, &mut inv), "and the consume reports failure");
    }

    #[test]
    fn turn_in_gate_rejects_incomplete_kill_quest_that_consume_would_accept() {
        // The OLD payout gated on fetch_make_consume, which is vacuously TRUE
        // for Kill quests (nothing to consume) regardless of kill count — that
        // was the bug. The gate the payout now uses checks the kill count.
        let q = kill(MobType::Brigand, 3, 18, 8);
        let mut inv = crate::inventory::Inventory::new();
        assert!(fetch_make_consume(&q, &mut inv), "consume is vacuously true for kill quests");
        assert!(!may_turn_in(&q, &inv, 0), "0 of 3 kills must NOT be turn-in-able");
        assert!(may_turn_in(&q, &inv, 3), "3 of 3 kills is turn-in-able");
    }

    #[test]
    fn hp7_quest_pools_use_obtainable_materials_only() {
        // Materials swapped from the spec sketch — guard that we only
        // ask for items the player can actually produce or harvest in
        // alpha. Flour/Cake/Pancakes/BeetrootSoup are NOT yet reachable
        // (Mill/Oven/Aging Rack workstation right-click interactions
        // are deferred) so quest pools must avoid them.
        let unreachable = [
            MaterialId::Flour,
            MaterialId::Cake,
            MaterialId::Pancakes,
            MaterialId::BeetrootSoup,
            MaterialId::Dough,
            MaterialId::Cream,
            MaterialId::Butter,
            MaterialId::Cheese,
            // HoneyBottle came OFF this list 2026-07-04 — wild hives spawn on
            // ~1-in-12 trees and Bucket-harvest to a Honey Jar (bee_hive.rs).
        ];
        for prof in [Profession::Miller, Profession::Baker, Profession::Brewer] {
            let pool = pool_for(prof).unwrap();
            for q in pool {
                if let QuestFlavour::Fetch { item, .. } | QuestFlavour::Make { item, .. } = q.flavour {
                    assert!(
                        !unreachable.contains(&item),
                        "{prof:?} quest pool references unreachable material {item:?}",
                    );
                }
            }
        }
    }

    #[test]
    fn hp7_miller_quest_summarises_with_human_readable_item_label() {
        // Generate a Miller quest at a few seeds; ensure summarise()
        // produces "Mystery Item"-free output (all referenced
        // materials have item_name entries).
        for seed in 0..16u32 {
            let q = generate_quest(Profession::Miller, seed).unwrap();
            let s = summarise(&q, false);
            assert!(!s.contains("Mystery Item"), "Miller seed {seed}: {s}");
        }
        for seed in 0..16u32 {
            let q = generate_quest(Profession::Baker, seed).unwrap();
            let s = summarise(&q, false);
            assert!(!s.contains("Mystery Item"), "Baker seed {seed}: {s}");
        }
        for seed in 0..16u32 {
            let q = generate_quest(Profession::Brewer, seed).unwrap();
            let s = summarise(&q, false);
            assert!(!s.contains("Mystery Item"), "Brewer seed {seed}: {s}");
        }
    }

    #[test]
    fn generate_quest_is_deterministic_per_seed() {
        let a = generate_quest(Profession::Farmer, 42).unwrap();
        let b = generate_quest(Profession::Farmer, 42).unwrap();
        // Compare the parts that matter — flavour discriminant + reward sats.
        // Quest itself isn't PartialEq because ItemStack isn't.
        assert_eq!(a.flavour, b.flavour, "same seed must yield same flavour");
        assert_eq!(a.reward.sats, b.reward.sats);
        assert_eq!(a.reward.reputation, b.reward.reputation);
    }

    /// Compliance lint (audit 2026-09-28 A#23): with the sats gate off, no
    /// quest offer in any profession's pool may render a money/earning word.
    #[test]
    fn summarise_never_shows_money_words_when_sats_hidden() {
        let profs = [
            Profession::Farmer, Profession::Cook, Profession::Carpenter,
            Profession::Blacksmith, Profession::Scribe, Profession::Miller,
            Profession::Baker, Profession::Brewer, Profession::Builder, Profession::None,
        ];
        let mut checked = 0;
        for prof in profs {
            for q in pool_for(prof).unwrap_or(&[]) {
                let s = summarise(q, false);
                let hits = crate::copy_lint::banned_in(&s, crate::copy_lint::BANNED_MONEY_WORDS);
                assert!(hits.is_empty(), "{prof:?}: {hits:?} in {s:?}");
                checked += 1;
            }
        }
        assert!(checked > 10, "lint must actually cover the pools");
    }

    #[test]
    fn summarise_includes_action_and_reward() {
        let q = Quest {
            flavour: QuestFlavour::Fetch { item: MaterialId::Wheat, count: 5 },
            reward: QuestReward { items: Vec::new(), sats: 12, reputation: 5 },
            accepted_by: None,
        };
        let s = summarise(&q, true);
        assert!(s.contains("Wheat"), "{s}");
        assert!(s.contains("5"), "{s}");
        assert!(s.contains("12 sats"), "{s}");
        assert!(s.contains("+5 rep"), "{s}");
    }

    #[test]
    fn fetch_completion_checks_inventory_count() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 5)));
        let q = Quest {
            flavour: QuestFlavour::Fetch { item: MaterialId::Wheat, count: 5 },
            reward: QuestReward::default(),
            accepted_by: None,
        };
        assert!(is_complete(&q, &inv, 0));

        let q_too_many = Quest {
            flavour: QuestFlavour::Fetch { item: MaterialId::Wheat, count: 6 },
            reward: QuestReward::default(),
            accepted_by: None,
        };
        assert!(!is_complete(&q_too_many, &inv, 0));
    }

    #[test]
    fn kill_completion_checks_counter() {
        let inv = Inventory::new();
        let q = Quest {
            flavour: QuestFlavour::Kill { mob: MobType::Brigand, count: 3 },
            reward: QuestReward::default(),
            accepted_by: None,
        };
        assert!(!is_complete(&q, &inv, 2));
        assert!(is_complete(&q, &inv, 3));
        assert!(is_complete(&q, &inv, 99));
    }

    #[test]
    fn fetch_make_consume_decrements_inventory_exactly() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 8)));
        let q = Quest {
            flavour: QuestFlavour::Fetch { item: MaterialId::Wheat, count: 5 },
            reward: QuestReward::default(),
            accepted_by: None,
        };
        assert!(fetch_make_consume(&q, &mut inv));
        let remaining: u32 = (0..36)
            .filter_map(|i| inv.slot(i))
            .map(|s| match s.item {
                crate::item::Item::Material(MaterialId::Wheat) => s.count as u32,
                _ => 0,
            })
            .sum();
        assert_eq!(remaining, 3, "should have consumed exactly the requested 5");
    }

    #[test]
    fn fetch_make_consume_refuses_when_short() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 2)));
        let q = Quest {
            flavour: QuestFlavour::Fetch { item: MaterialId::Wheat, count: 5 },
            reward: QuestReward::default(),
            accepted_by: None,
        };
        assert!(!fetch_make_consume(&q, &mut inv), "expected refusal when inventory short");
        // Inventory untouched on refusal.
        assert_eq!(inv.slot(0).unwrap().count, 2);
    }

    #[test]
    fn kill_quests_consume_nothing() {
        let mut inv = Inventory::new();
        inv.set_slot(0, Some(ItemStack::new_material(MaterialId::Wheat, 2)));
        let q = Quest {
            flavour: QuestFlavour::Kill { mob: MobType::Brigand, count: 2 },
            reward: QuestReward::default(),
            accepted_by: None,
        };
        assert!(fetch_make_consume(&q, &mut inv));
        assert_eq!(inv.slot(0).unwrap().count, 2);
    }
}
