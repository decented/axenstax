//! `ItemKind` — WHICH item, with no per-stack data (no count, no durability,
//! no plan contents). A small `Copy` key so a Trials objective can name the
//! thing it wants crafted (`ChallengeEvent::CraftItem { item }`) without
//! making `ChallengeEvent` carry a heap string.
//!
//! Authored in a trial's JSON as a `/give`-style name (`"iron_pickaxe"`,
//! `"orange_dye"`, `"crafting_table"`, `"iron_helmet"`), resolved through the
//! same vocabulary as kits (`give::resolve_item`) plus the armour names
//! (`<material>_<slot>`), which `/give` does not list. It also accepts the
//! typed form it serialises to (`{"Tool": ["Pickaxe", "Iron"]}`), so a def
//! round-trips through `ScenarioDef::to_json` and back unchanged. An unknown
//! name is a parse error: a filter that silently widened to "any item" would
//! make a trial trivially completable.
//!
//! Design: `docs/vision/trials-two-acts.md` §A.5 (internal repo).

use serde::{Deserialize, Deserializer, Serialize};

use crate::armour::{ArmourMaterial, ArmourSlot};
use crate::block::BlockId;
use crate::crafting::{ToolMaterial, ToolType};
use crate::item::{Item, MaterialId};

/// One kind of item, ignoring stack size and per-instance state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum ItemKind {
    Block(BlockId),
    Tool(ToolType, ToolMaterial),
    Material(MaterialId),
    Armour(ArmourSlot, ArmourMaterial),
    /// Any captured-building plan (its contents are per-instance).
    Plan,
}

/// The typed (serialised) form, derived via serde's remote mirror so the
/// variants can never drift from [`ItemKind`] (a mismatch is a compile error).
#[derive(Deserialize)]
#[serde(remote = "ItemKind")]
enum ItemKindTyped {
    Block(BlockId),
    Tool(ToolType, ToolMaterial),
    Material(MaterialId),
    Armour(ArmourSlot, ArmourMaterial),
    Plan,
}

impl ItemKind {
    /// The kind of a live item (what a craft produced, what a slot holds).
    pub fn of(item: &Item) -> Self {
        match item {
            Item::Block(b) => ItemKind::Block(*b),
            Item::Tool(t) => ItemKind::Tool(t.tool_type, t.material),
            Item::Material(m) => ItemKind::Material(*m),
            Item::Plan(_) => ItemKind::Plan,
            Item::Armour(a) => ItemKind::Armour(a.slot, a.material),
        }
    }

    /// Resolve a `/give`-style name (hyphens or underscores, any case), or an
    /// armour name `<material>_<slot>` (`iron_helmet`, `leather_boots`).
    pub fn from_name(name: &str) -> Option<Self> {
        if let Ok(stack) = crate::commands::builtins::give::resolve_item(name, 1) {
            return Some(Self::of(&stack.item));
        }
        armour_by_name(&name.to_lowercase().replace('-', "_"))
    }

    /// A short, friendly description for the auto-derived task line
    /// ("Craft an iron pickaxe"). Authored task labels normally replace it.
    pub fn label(&self) -> String {
        match self {
            ItemKind::Block(_) => "block".to_string(),
            ItemKind::Tool(t, m) => format!("{} {}", words(&format!("{m:?}")), words(&format!("{t:?}"))),
            ItemKind::Material(m) => words(&format!("{m:?}")),
            ItemKind::Armour(s, m) => format!("{} {}", words(&format!("{m:?}")), words(&format!("{s:?}"))),
            ItemKind::Plan => "plan".to_string(),
        }
    }
}

/// `IronHelmet`-style Debug name → `iron helmet`.
fn words(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn armour_by_name(key: &str) -> Option<ItemKind> {
    let (mat, slot) = key.rsplit_once('_')?;
    let material = match mat {
        "leather" => ArmourMaterial::Leather,
        "iron" => ArmourMaterial::Iron,
        "diamond" => ArmourMaterial::Diamond,
        "satori" => ArmourMaterial::Satori,
        "chainmail" => ArmourMaterial::Chainmail,
        "rubber" => ArmourMaterial::Rubber,
        _ => return None,
    };
    let slot = match slot {
        "helmet" => ArmourSlot::Helmet,
        "chestplate" => ArmourSlot::Chestplate,
        "leggings" => ArmourSlot::Leggings,
        "boots" => ArmourSlot::Boots,
        _ => return None,
    };
    Some(ItemKind::Armour(slot, material))
}

impl<'de> Deserialize<'de> for ItemKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        if let serde_json::Value::String(name) = &v {
            return ItemKind::from_name(name).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "unknown item name '{name}' (use a /give name such as iron_pickaxe, or an \
                     armour name such as iron_helmet)"
                ))
            });
        }
        ItemKindTyped::deserialize(v).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_to_kinds() {
        assert_eq!(ItemKind::from_name("iron_pickaxe"), Some(ItemKind::Tool(ToolType::Pickaxe, ToolMaterial::Iron)));
        assert_eq!(ItemKind::from_name("orange-dye"), Some(ItemKind::Material(MaterialId::OrangeDye)));
        assert_eq!(ItemKind::from_name("crafting_table"), Some(ItemKind::Block(crate::block::CRAFTING_TABLE)));
        assert_eq!(ItemKind::from_name("Iron_Helmet"), Some(ItemKind::Armour(ArmourSlot::Helmet, ArmourMaterial::Iron)));
        assert_eq!(ItemKind::from_name("iron_hat"), None);
        assert_eq!(ItemKind::from_name("nonsense"), None);
    }

    #[test]
    fn a_name_or_the_typed_form_parses_and_the_typed_form_round_trips() {
        let k: ItemKind = serde_json::from_str("\"wooden_sword\"").unwrap();
        assert_eq!(k, ItemKind::Tool(ToolType::Sword, ToolMaterial::Wood));
        let typed = serde_json::to_string(&k).unwrap();
        let back: ItemKind = serde_json::from_str(&typed).unwrap();
        assert_eq!(back, k);
        let err = serde_json::from_str::<ItemKind>("\"no_such_thing\"").unwrap_err().to_string();
        assert!(err.contains("no_such_thing"), "{err}");
    }

    #[test]
    fn an_item_reports_its_kind() {
        let sword = crate::commands::builtins::give::resolve_item("iron_sword", 1).unwrap();
        assert_eq!(ItemKind::of(&sword.item), ItemKind::Tool(ToolType::Sword, ToolMaterial::Iron));
        assert_eq!(ItemKind::Armour(ArmourSlot::Boots, ArmourMaterial::Iron).label(), "iron boots");
    }
}
