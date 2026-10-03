//! Player **avatar-skin** wardrobe (Skin Studio foundation). A per-identity
//! collection of named skins; the *equipped* entry derives the live
//! `CosmeticDescriptor` the renderer + multiplayer already consume, so nothing
//! downstream changes.
//!
//! NAMING: distinct from `wardrobe_store.rs` / `player_wardrobe` / `OverrideSet`,
//! which is the WORKSHOP block/mob appearance library (Spec 40). This is the
//! AVATAR-SKIN wardrobe — keep the two apart.

use crate::cosmetics::{CosmeticDescriptor, SkinSource};
use crate::skin_uv::ArmModel;

pub type SkinId = u32;

/// A 64×64 transparent (all-zero RGBA) skin — the starting point for a blank.
fn blank_rgba() -> Vec<u8> {
    vec![0u8; 64 * 64 * 4]
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkinEntry {
    pub id: SkinId,
    pub name: String,
    pub source: SkinSource,
    /// Display handle if imported from a Minecraft username (§8.4); never trusted.
    pub minecraft_handle: Option<String>,
    /// Stable Mojang UUID — the key used by Refresh (§8.4).
    pub minecraft_uuid: Option<String>,
    /// Which player model this skin is drawn on: Classic (4-px arms) or Slim
    /// ("Alex", 3-px arms). Per ENTRY, not per player — a wardrobe can hold
    /// both, and the worn one decides what the avatar looks like. Imports set
    /// it from Mojang's `metadata.model`; everything else defaults to Classic.
    pub arm_model: ArmModel,
    pub created: u64,
    pub modified: u64,
}

/// A per-identity collection of skins with exactly one equipped at all times.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinWardrobe {
    entries: Vec<SkinEntry>,
    equipped: SkinId,
    next_id: SkinId,
}

impl Default for SkinWardrobe {
    fn default() -> Self {
        Self::new()
    }
}

impl SkinWardrobe {
    /// A fresh wardrobe with a single equipped "Default" entry, so there is
    /// always something to wear.
    pub fn new() -> Self {
        let first = SkinEntry {
            id: 1,
            name: "Default".to_string(),
            source: SkinSource::Default,
            minecraft_handle: None,
            minecraft_uuid: None,
            arm_model: ArmModel::Classic,
            created: 0,
            modified: 0,
        };
        Self { entries: vec![first], equipped: 1, next_id: 2 }
    }

    pub fn entries(&self) -> &[SkinEntry] {
        &self.entries
    }

    pub fn equipped_id(&self) -> SkinId {
        self.equipped
    }

    /// The currently-worn entry. Invariant: `equipped` always names a present
    /// entry, so this never panics.
    pub fn equipped_entry(&self) -> &SkinEntry {
        self.entries
            .iter()
            .find(|e| e.id == self.equipped)
            .expect("equipped id always names a present entry")
    }

    pub fn get(&self, id: SkinId) -> Option<&SkinEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn take_id(&mut self) -> SkinId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Add a fresh transparent skin named "Skin N"; returns its id. Does NOT
    /// equip it (the caller decides; the painter opens it).
    pub fn add_blank(&mut self, now: u64) -> SkinId {
        let id = self.take_id();
        self.entries.push(SkinEntry {
            id,
            name: format!("Skin {id}"),
            source: SkinSource::Rgba64(blank_rgba()),
            minecraft_handle: None,
            minecraft_uuid: None,
            arm_model: ArmModel::Classic,
            created: now,
            modified: now,
        });
        id
    }

    /// Add a skin from explicit 64×64 RGBA bytes (upload / Minecraft import).
    pub fn add_from_rgba(&mut self, name: String, rgba: Vec<u8>, now: u64) -> SkinId {
        debug_assert_eq!(rgba.len(), 64 * 64 * 4, "skin must be 16384 RGBA bytes");
        let id = self.take_id();
        self.entries.push(SkinEntry {
            id,
            name,
            source: SkinSource::Rgba64(rgba),
            minecraft_handle: None,
            minecraft_uuid: None,
            arm_model: ArmModel::Classic,
            created: now,
            modified: now,
        });
        id
    }

    /// Add a skin imported from a Minecraft username, recording the display
    /// handle + the stable UUID (the key Refresh re-pulls by, §8.4). Does NOT
    /// auto-equip — the caller decides ("Wear it now?").
    pub fn add_from_minecraft(
        &mut self,
        name: String,
        uuid: String,
        rgba: Vec<u8>,
        arm_model: ArmModel,
        now: u64,
    ) -> SkinId {
        debug_assert_eq!(rgba.len(), 64 * 64 * 4, "skin must be 16384 RGBA bytes");
        let id = self.take_id();
        self.entries.push(SkinEntry {
            id,
            name: name.clone(),
            source: SkinSource::Rgba64(rgba),
            minecraft_handle: Some(name),
            minecraft_uuid: Some(uuid),
            arm_model,
            created: now,
            modified: now,
        });
        id
    }

    /// Fork an entry into an independent copy named "<name> copy". Returns the
    /// new id, or `None` if `id` is absent.
    pub fn duplicate(&mut self, id: SkinId, now: u64) -> Option<SkinId> {
        let src = self.get(id)?.clone();
        let new_id = self.take_id();
        self.entries.push(SkinEntry {
            id: new_id,
            name: format!("{} copy", src.name),
            source: src.source,
            minecraft_handle: src.minecraft_handle,
            minecraft_uuid: src.minecraft_uuid,
            arm_model: src.arm_model,
            created: now,
            modified: now,
        });
        Some(new_id)
    }

    pub fn rename(&mut self, id: SkinId, name: String) -> bool {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.name = name;
            true
        } else {
            false
        }
    }

    /// Remove an entry. Refuses to delete the equipped entry or the last
    /// remaining entry (the player must equip another first).
    pub fn delete(&mut self, id: SkinId) -> bool {
        if id == self.equipped || self.entries.len() <= 1 {
            return false;
        }
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != before
    }

    pub fn set_equipped(&mut self, id: SkinId) -> bool {
        if self.entries.iter().any(|e| e.id == id) {
            self.equipped = id;
            true
        } else {
            false
        }
    }

    /// Overwrite an entry's pixels (the painter's Save). Returns false if absent.
    pub fn update_rgba(&mut self, id: SkinId, rgba: Vec<u8>, now: u64) -> bool {
        debug_assert_eq!(rgba.len(), 64 * 64 * 4, "skin must be 16384 RGBA bytes");
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.source = SkinSource::Rgba64(rgba);
            e.modified = now;
            true
        } else {
            false
        }
    }

    /// Set an entry's arm model (the "Your look" Arms toggle, and the paint
    /// panel's on Pin). `false` if `id` is absent. Does NOT stamp `modified` —
    /// this is a display choice about the same pixels, not an edit to them, and
    /// stamping it would move a pre-v0.2.18 skin past the box-unwrap migration
    /// cutoff and skip a fix it still needs.
    pub fn set_arm_model(&mut self, id: SkinId, arm: ArmModel) -> bool {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.arm_model = arm;
            true
        } else {
            false
        }
    }

    /// An entry's arm model; `Classic` for an id that isn't there (degrade on a
    /// render path, never panic).
    pub fn arm_model(&self, id: SkinId) -> ArmModel {
        self.get(id).map(|e| e.arm_model).unwrap_or_default()
    }

    /// The live look derived from the equipped entry — the existing renderer +
    /// multiplayer path consumes this unchanged.
    pub fn active_descriptor(&self) -> CosmeticDescriptor {
        let e = self.equipped_entry();
        CosmeticDescriptor { version: 1, skin: e.source.clone(), arm_model: e.arm_model }
    }

    /// Crate-internal: expose fields for the serialization layer (Task 3).
    pub(crate) fn parts(&self) -> (&[SkinEntry], SkinId, SkinId) {
        (&self.entries, self.equipped, self.next_id)
    }

    /// Crate-internal: rebuild from deserialized parts (Task 3). Repairs the
    /// equipped invariant if the stored id is missing (degrade, never panic).
    pub(crate) fn from_parts(entries: Vec<SkinEntry>, equipped: SkinId, next_id: SkinId) -> Self {
        if entries.is_empty() {
            return Self::new();
        }
        let equipped = if entries.iter().any(|e| e.id == equipped) {
            equipped
        } else {
            entries[0].id
        };
        // Harden next_id against a stale/corrupt blob: never hand out an id that
        // collides with an existing entry. (entries is non-empty past the early return.)
        let next_id = next_id.max(entries.iter().map(|e| e.id).max().unwrap_or(0) + 1);
        Self { entries, equipped, next_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_one_equipped_default() {
        let w = SkinWardrobe::new();
        assert_eq!(w.entries().len(), 1);
        assert_eq!(w.equipped_id(), 1);
        assert_eq!(w.equipped_entry().source, SkinSource::Default);
        assert_eq!(w.active_descriptor().skin_key(), 0, "default skin hashes to the 0 sentinel");
    }

    #[test]
    fn add_blank_is_transparent_and_unequipped() {
        let mut w = SkinWardrobe::new();
        let id = w.add_blank(100);
        assert_eq!(w.entries().len(), 2);
        assert_ne!(id, w.equipped_id(), "adding does not auto-equip");
        match &w.get(id).unwrap().source {
            SkinSource::Rgba64(px) => {
                assert_eq!(px.len(), 64 * 64 * 4);
                assert!(px.iter().all(|&b| b == 0), "blank skin is fully transparent");
            }
            _ => panic!("blank must be Rgba64"),
        }
    }

    #[test]
    fn duplicate_is_independent_copy() {
        let mut w = SkinWardrobe::new();
        let a = w.add_from_rgba("Knight".into(), vec![7u8; 64 * 64 * 4], 1);
        let b = w.duplicate(a, 2).expect("duplicate of present entry");
        assert_ne!(a, b);
        assert_eq!(w.get(b).unwrap().name, "Knight copy");
        assert_eq!(w.get(a).unwrap().source, w.get(b).unwrap().source);
        // Mutating the copy must not touch the original.
        w.update_rgba(b, vec![9u8; 64 * 64 * 4], 3);
        assert_ne!(w.get(a).unwrap().source, w.get(b).unwrap().source);
    }

    #[test]
    fn delete_refuses_equipped_and_last() {
        let mut w = SkinWardrobe::new();
        assert!(!w.delete(w.equipped_id()), "cannot delete the equipped entry");
        let extra = w.add_blank(1);
        assert!(w.delete(extra), "a non-equipped extra deletes");
        assert_eq!(w.entries().len(), 1);
        assert!(!w.delete(w.equipped_id()), "cannot delete the last entry");
    }

    #[test]
    fn equip_changes_active_descriptor() {
        let mut w = SkinWardrobe::new();
        let id = w.add_from_rgba("Red".into(), vec![3u8; 64 * 64 * 4], 1);
        assert_eq!(w.active_descriptor().skin_key(), 0);
        assert!(w.set_equipped(id));
        assert_ne!(w.active_descriptor().skin_key(), 0, "now wearing a custom skin");
        assert!(!w.set_equipped(9999), "equipping an absent id is rejected");
    }

    #[test]
    fn rename_and_update() {
        let mut w = SkinWardrobe::new();
        let id = w.add_blank(1);
        assert!(w.rename(id, "Pirate".into()));
        assert_eq!(w.get(id).unwrap().name, "Pirate");
        assert!(w.update_rgba(id, vec![5u8; 64 * 64 * 4], 7));
        assert_eq!(w.get(id).unwrap().modified, 7);
        assert!(!w.rename(123, "Nope".into()));
    }

    #[test]
    fn from_parts_repairs_stale_next_id() {
        let entries = vec![
            SkinEntry { id: 5, name: "a".into(), source: SkinSource::Default, minecraft_handle: None, minecraft_uuid: None, arm_model: ArmModel::Classic, created: 0, modified: 0 },
            SkinEntry { id: 9, name: "b".into(), source: SkinSource::Rgba64(vec![1u8; 64 * 64 * 4]), minecraft_handle: None, minecraft_uuid: None, arm_model: ArmModel::Classic, created: 0, modified: 0 },
        ];
        // Corrupt: stored next_id (3) is below the max existing id (9).
        let mut w = SkinWardrobe::from_parts(entries, 9, 3);
        let new_id = w.add_blank(1);
        assert!(new_id > 9, "next id must not collide with an existing entry (got {new_id})");
        assert!(w.get(5).is_some() && w.get(9).is_some(), "existing entries preserved");
    }

    // ── Arm model (Classic / Slim) ───────────────────────────────────────────

    #[test]
    fn entries_default_to_classic_arms() {
        let mut w = SkinWardrobe::new();
        assert_eq!(w.arm_model(w.equipped_id()), ArmModel::Classic);
        let blank = w.add_blank(1);
        let rgba = w.add_from_rgba("R".into(), vec![1u8; 64 * 64 * 4], 1);
        assert_eq!(w.arm_model(blank), ArmModel::Classic);
        assert_eq!(w.arm_model(rgba), ArmModel::Classic);
        assert_eq!(w.arm_model(9999), ArmModel::Classic, "an absent id degrades to Classic");
    }

    #[test]
    fn set_arm_model_flows_into_the_active_descriptor() {
        let mut w = SkinWardrobe::new();
        let id = w.add_from_rgba("Alex".into(), vec![4u8; 64 * 64 * 4], 1);
        assert!(w.set_arm_model(id, ArmModel::Slim));
        assert_eq!(w.arm_model(id), ArmModel::Slim);
        // Not worn yet → the live look is still the classic default entry.
        assert_eq!(w.active_descriptor().arm_model, ArmModel::Classic);
        w.set_equipped(id);
        assert_eq!(
            w.active_descriptor().arm_model,
            ArmModel::Slim,
            "wearing a slim entry makes the live avatar slim"
        );
        assert!(!w.set_arm_model(9999, ArmModel::Slim), "absent id is rejected");
    }

    #[test]
    fn set_arm_model_does_not_stamp_modified() {
        // `modified` gates the v1 box-unwrap migration; bumping it for a display
        // choice would silently disqualify an old skin from a fix it still needs.
        let mut w = SkinWardrobe::new();
        let id = w.add_from_rgba("Old".into(), vec![1u8; 64 * 64 * 4], 100);
        w.set_arm_model(id, ArmModel::Slim);
        assert_eq!(w.get(id).unwrap().modified, 100);
    }

    #[test]
    fn duplicate_copies_the_arm_model() {
        let mut w = SkinWardrobe::new();
        let a = w.add_from_rgba("Alex".into(), vec![7u8; 64 * 64 * 4], 1);
        w.set_arm_model(a, ArmModel::Slim);
        let b = w.duplicate(a, 2).expect("duplicate");
        assert_eq!(w.arm_model(b), ArmModel::Slim, "a copy of a slim skin is slim");
    }

    #[test]
    fn add_from_minecraft_records_the_detected_arm_model() {
        let mut w = SkinWardrobe::new();
        let id = w.add_from_minecraft(
            "Alex".into(),
            "uuid".into(),
            vec![1u8; 64 * 64 * 4],
            ArmModel::Slim,
            9,
        );
        assert_eq!(w.arm_model(id), ArmModel::Slim, "Mojang said slim, so the entry is slim");
    }

    #[test]
    fn add_from_minecraft_records_handle_and_uuid() {
        let mut w = SkinWardrobe::new();
        let id = w.add_from_minecraft("Notch".into(), "069a79f4".into(), vec![1u8; 64 * 64 * 4], ArmModel::Classic, 9);
        let e = w.get(id).unwrap();
        assert_eq!(e.minecraft_handle.as_deref(), Some("Notch"));
        assert_eq!(e.minecraft_uuid.as_deref(), Some("069a79f4"));
        assert_ne!(id, w.equipped_id(), "import does not auto-equip");
    }
}
