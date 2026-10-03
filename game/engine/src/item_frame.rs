//! Item-frame block-entity (Solo Buildout Wave 2c).
//!
//! A thin plate mounted flush on a block face (an F1 `ItemFrame` shape) that
//! displays one item with a rotation. Right-click an empty frame holding an
//! item to mount it; right-click a filled frame to rotate it; break the frame
//! to drop the framed item. State lives in `BlockEntityData::ItemFrame` and
//! persists via `save::SavedItemFrame`.
//!
//! Pass-through (no collision). The framed item is rendered as a small textured
//! cube on the plate for *block* items (`mesh`); other item kinds surface via
//! the read-on-look HUD. The geometry + facing live in `block_shape`.

use serde::{Deserialize, Serialize};

use crate::item::ItemStack;

/// Rotation steps a framed item can sit at (Minecraft uses 8 × 45°).
pub const FRAME_ROTATIONS: u8 = 8;

// NB: no `PartialEq` — `ItemStack`/`Item` (Plan, Armour) aren't comparable.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ItemFrameData {
    /// The framed item (exactly one), or `None` for an empty frame.
    pub item: Option<ItemStack>,
    /// Display rotation, `0..FRAME_ROTATIONS`.
    pub rotation: u8,
}

impl ItemFrameData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.item.is_none()
    }

    /// Mount one item if the frame is empty. Returns true on success. Always
    /// frames a single unit (count 1) regardless of the incoming stack size.
    pub fn try_insert(&mut self, mut stack: ItemStack) -> bool {
        if self.item.is_some() {
            return false;
        }
        stack.count = 1;
        self.item = Some(stack);
        self.rotation = 0;
        true
    }

    /// Advance the rotation one step (wraps). No-op on an empty frame.
    pub fn rotate(&mut self) {
        if self.item.is_some() {
            self.rotation = (self.rotation + 1) % FRAME_ROTATIONS;
        }
    }

    /// Remove and return the framed item (clears rotation).
    pub fn take(&mut self) -> Option<ItemStack> {
        self.rotation = 0;
        self.item.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block;

    #[test]
    fn insert_frames_a_single_unit_then_refuses_a_second() {
        let mut f = ItemFrameData::new();
        assert!(f.is_empty());
        assert!(f.try_insert(ItemStack::new_block(block::DIAMOND_BLOCK, 5)));
        // Only one unit is framed, never the whole stack.
        assert_eq!(f.item.as_ref().unwrap().count, 1);
        assert!(!f.is_empty());
        // A second insert is refused while occupied.
        assert!(!f.try_insert(ItemStack::new_block(block::STONE, 1)));
    }

    #[test]
    fn rotate_wraps_and_is_inert_when_empty() {
        let mut f = ItemFrameData::new();
        f.rotate();
        assert_eq!(f.rotation, 0, "empty frame doesn't rotate");
        f.try_insert(ItemStack::new_block(block::STONE, 1));
        for expected in 1..FRAME_ROTATIONS {
            f.rotate();
            assert_eq!(f.rotation, expected);
        }
        f.rotate();
        assert_eq!(f.rotation, 0, "rotation wraps back to 0");
    }

    #[test]
    fn take_returns_the_item_and_empties() {
        let mut f = ItemFrameData::new();
        f.try_insert(ItemStack::new_block(block::OAK_LOG, 1));
        f.rotate();
        let got = f.take().expect("framed item comes back");
        assert!(matches!(got.item, crate::item::Item::Block(b) if b == block::OAK_LOG));
        assert!(f.is_empty());
        assert_eq!(f.rotation, 0);
    }
}
