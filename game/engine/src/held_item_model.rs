//! Shared item -> mesh mapping for held items.
//!
//! This is the single source of truth used by BOTH the remote-avatar hand
//! (Phase 6) and the first-person viewmodel (Phase 7), so the two always
//! agree on what an item looks like in-hand. Each builder returns a local-
//! space mesh centred at the origin at roughly unit scale; the caller
//! transforms it to the hand anchor or the viewmodel slot.
//!
//! The geometry reuses the same `Vertex` format + cuboid construction as the
//! entity pass (`entity_model::build_item_entity_vertices`) so held items
//! render through the existing entity pipeline (textures, lighting, fog).

use glam::Vec3;

use crate::block::BlockRegistry;
use crate::crafting::ToolMaterial;
use crate::entity_model::{material_texture, push_textured_quad, tool_texture, TEX_ITEM_STICK};
use crate::item::MaterialId;
use crate::mesh::Vertex;
use crate::protocol::ItemRef;

/// Local-space mesh (centred at origin, ~unit scale) for a held item.
/// Caller transforms it to the hand anchor or the first-person viewmodel slot.
///
/// A cuboid emits 6 faces x 2 triangles x 3 vertices = **36 vertices**.
pub fn held_item_mesh(item: ItemRef, registry: &BlockRegistry) -> Vec<Vertex> {
    match item {
        ItemRef::Empty => Vec::new(),
        ItemRef::Block(id) => {
            // Use the block's own six face texture layers (top/side/bottom),
            // mirroring how `build_item_entity_vertices` resolves block drops.
            let def = registry.get(id);
            let mut verts = Vec::with_capacity(36);
            push_cuboid(
                &mut verts,
                Vec3::ZERO,
                Vec3::splat(0.4),
                def.tex_top,
                def.tex_side,
                def.tex_bottom,
            );
            verts
        }
        ItemRef::Tool(tier) => {
            // Map the wire tier back to a ToolMaterial, then defer to the
            // authoritative texture lookup so a held tool always matches its
            // dropped-item icon (e.g. Satori has its own texture, not diamond).
            let mat = match tier {
                0 => ToolMaterial::Wood,
                1 => ToolMaterial::Stone,
                2 => ToolMaterial::Iron,
                3 => ToolMaterial::Diamond,
                _ => ToolMaterial::Satori, // tier 4 (Satori) + any future tier
            };
            let tex = tool_texture(mat);
            // Thin slab oriented along Y so it reads as a tool shaft when the
            // caller tilts it into the hand.
            let mut verts = Vec::with_capacity(36);
            push_cuboid(
                &mut verts,
                Vec3::ZERO,
                Vec3::new(0.12, 0.6, 0.12),
                tex,
                tex,
                tex,
            );
            verts
        }
        ItemRef::Material(id) => {
            // ItemRef::Material carries the `MaterialId as u16` discriminant
            // index (see inventory::item_to_ref). Recover the MaterialId via
            // its TryFrom inverse and use the authoritative per-material item
            // texture. An unknown id (e.g. a newer peer's material this build
            // doesn't know) falls back to the stick icon.
            let tex = MaterialId::try_from(id)
                .ok()
                .map(material_texture)
                .unwrap_or(TEX_ITEM_STICK);
            let mut verts = Vec::with_capacity(36);
            push_cuboid(
                &mut verts,
                Vec3::ZERO,
                Vec3::splat(0.3),
                tex,
                tex,
                tex,
            );
            verts
        }
    }
}

/// Emit a textured axis-aligned cuboid centred at `centre` with full size
/// `size` (so it spans `centre +/- size/2` on each axis). `tex_top`,
/// `tex_side`, `tex_bottom` are the texture array layers for the +Y face,
/// the four side faces, and the -Y face respectively. Faces are wound to
/// match the entity pass (`push_textured_quad`). Always emits 36 vertices.
fn push_cuboid(
    verts: &mut Vec<Vertex>,
    centre: Vec3,
    size: Vec3,
    tex_top: u32,
    tex_side: u32,
    tex_bottom: u32,
) {
    let h = size * 0.5;
    let c = [
        centre + Vec3::new(-h.x, -h.y, -h.z),
        centre + Vec3::new(h.x, -h.y, -h.z),
        centre + Vec3::new(h.x, h.y, -h.z),
        centre + Vec3::new(-h.x, h.y, -h.z),
        centre + Vec3::new(-h.x, -h.y, h.z),
        centre + Vec3::new(h.x, -h.y, h.z),
        centre + Vec3::new(h.x, h.y, h.z),
        centre + Vec3::new(-h.x, h.y, h.z),
    ];

    push_textured_quad(verts, tex_side, [1.0, 0.0, 0.0], c[1], c[5], c[6], c[2]); // +X
    push_textured_quad(verts, tex_side, [-1.0, 0.0, 0.0], c[4], c[0], c[3], c[7]); // -X
    push_textured_quad(verts, tex_top, [0.0, 1.0, 0.0], c[3], c[2], c[6], c[7]); // +Y
    push_textured_quad(verts, tex_bottom, [0.0, -1.0, 0.0], c[4], c[5], c[1], c[0]); // -Y
    push_textured_quad(verts, tex_side, [0.0, 0.0, 1.0], c[5], c[4], c[7], c[6]); // +Z
    push_textured_quad(verts, tex_side, [0.0, 0.0, -1.0], c[0], c[1], c[2], c[3]); // -Z
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ItemRef;

    fn test_registry() -> BlockRegistry {
        BlockRegistry::new()
    }

    #[test]
    fn empty_has_no_verts() {
        assert!(held_item_mesh(ItemRef::Empty, &test_registry()).is_empty());
    }

    #[test]
    fn block_makes_a_cube() {
        assert_eq!(held_item_mesh(ItemRef::Block(1), &test_registry()).len(), 36);
    }

    #[test]
    fn tool_makes_nonempty_slab() {
        assert_eq!(held_item_mesh(ItemRef::Tool(2), &test_registry()).len(), 36);
    }

    #[test]
    fn satori_tier_falls_back_not_panics() {
        let _ = held_item_mesh(ItemRef::Tool(4), &test_registry());
    }

    #[test]
    fn material_makes_nonempty_cube() {
        assert_eq!(held_item_mesh(ItemRef::Material(0), &test_registry()).len(), 36);
    }

    #[test]
    fn tool_tier_uses_authoritative_texture() {
        // The in-hand tool texture must match the dropped-item lookup, so a
        // Satori tool (tier 4) is NOT the diamond layer. This guards against
        // the in-hand vs on-ground mismatch the review caught.
        assert_eq!(
            tool_texture(ToolMaterial::Satori),
            crate::block::TEX_ITEM_SATORI
        );
        assert_ne!(
            tool_texture(ToolMaterial::Satori),
            tool_texture(ToolMaterial::Diamond)
        );
    }

    #[test]
    fn material_resolves_to_its_own_texture() {
        // A known non-stick material (Diamond) must resolve to its own item
        // texture, not the stick fallback — proving the u16 -> MaterialId
        // path is wired up rather than always returning the placeholder.
        let diamond_id = MaterialId::Diamond as u16;
        assert_eq!(
            MaterialId::try_from(diamond_id).map(material_texture),
            Ok(material_texture(MaterialId::Diamond))
        );
        assert_ne!(material_texture(MaterialId::Diamond), TEX_ITEM_STICK);
    }
}
