//! Deterministic fresh-world kit shared by the local and server seats.
use crate::inventory::Inventory;
pub fn grant_fresh_kit(inv: &mut Inventory, world_type: &str, creative: bool, workshop: bool) {
    if creative && workshop {
        inv.set_slot(
            8,
            Some(crate::item::ItemStack::new_material(
                crate::item::MaterialId::Bellows,
                1,
            )),
        );
    }

    // Test Lab starter kit — a tester should never be blocked grinding
    // before a mission. Generous building kit + the core tools + food +
    // the starter-hut Plan (so build-along / schematic missions are
    // immediately doable). Applies in Survival or Creative Test Labs.
    if world_type == "testlab" {
        use crate::block;
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::item::{Item, ItemStack, MaterialId};
        let kit = [
            block::OAK_PLANKS,
            block::OAK_LOG,
            block::STONE,
            block::COBBLESTONE,
            block::DIRT,
            block::GLASS,
            block::SAND,
            block::CRAFTING_TABLE,
            crate::rail::TRACK,
            block::CHEST,
        ];
        for (i, &id) in kit.iter().enumerate() {
            inv.set_slot(i, Some(ItemStack::new_block(id, 64)));
        }
        for tt in [
            ToolType::Pickaxe,
            ToolType::Axe,
            ToolType::Shovel,
            ToolType::Sword,
        ] {
            let _ = inv.add_item(ItemStack::new_tool(Tool::new(tt, ToolMaterial::Iron)));
        }
        let _ = inv.add_item(ItemStack::new_material(MaterialId::Bread, 8));
        let _ = inv.add_item(ItemStack {
            item: Item::Plan(crate::satoshi::starter_hut_plan()),
            count: 1,
        });
    }
}
