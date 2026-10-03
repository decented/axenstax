//! Official override catalogue — baked at build time by tools/bake-beacon.js,
//! embedded here, applied BENEATH personal overrides (personal wins). Offline on
//! both platforms — no runtime Beacon. See beacon/CONSUMING.md §4-5. Ships EMPTY
//! (byte-identical to a stock game) until the owner runs the live bake.
use crate::override_registry::{OverrideSet, OVERRIDE_SET_VERSION};
use serde::Deserialize;

#[derive(Deserialize)]
struct Catalogue {
    #[serde(default)]
    version: u8,
    #[serde(default)]
    items: Vec<CatItem>,
}
#[derive(Deserialize)]
struct CatItem {
    #[allow(dead_code)]
    #[serde(default)]
    name: String,
    #[serde(default)]
    bytes: Vec<u8>, // u8 JSON array; element 0 is the OverrideSet version prefix
}

/// Parse the embedded catalogue JSON into ONE merged official OverrideSet (all
/// official items combined in publish order). Empty catalogue -> empty set (no-op).
/// Each item's bytes are a version-prefixed OverrideSet blob (from_blob_bytes).
pub fn load_official_set(json: &str) -> Result<OverrideSet, String> {
    let cat: Catalogue =
        serde_json::from_str(json).map_err(|e| format!("official catalogue: {e}"))?;
    let _ = cat.version;
    let mut out = OverrideSet::default();
    for it in &cat.items {
        let set = OverrideSet::from_blob_bytes(&it.bytes)?; // version + garbage guard
        // Combine official items: later items overwrite earlier on a key clash.
        layer_beneath(&mut out, &set); // out is base, set wins
    }
    if !out.is_empty() && out.version == 0 {
        out.version = OVERRIDE_SET_VERSION;
    }
    Ok(out)
}

/// Layer `top`'s wardrobes OVER `base` (top wins per key, new keys added) — a pure
/// appearance merge that does NOT touch derivation chains (this is base-layering,
/// not provenance-bearing adoption like OverrideSet::merge_adopted). `base` is
/// mutated in place. Per-block-wardrobe (Phase 5): a key present in `top` SHADOWS
/// `base` for that key entirely (top's whole library replaces base's); keys only in
/// `base` are kept; keys only in `top` are added. So official applies only where the
/// upper layer hasn't claimed that key.
fn layer_beneath(base: &mut OverrideSet, top: &OverrideSet) {
    for (b, lib) in &top.block_designs {
        base.block_designs.retain(|(x, _)| x != b);
        base.block_designs.push((*b, lib.clone()));
    }
    for (k, lib) in &top.mob_designs {
        base.mob_designs.retain(|(x, _)| x != k);
        base.mob_designs.push((*k, lib.clone()));
    }
}

/// The render set = official catalogue UNDERNEATH `personal` (personal wins per
/// key). Empty official -> returns `personal` unchanged (true no-op). Keeps
/// `personal`'s provenance (author + derivation chain); official is a base layer.
pub fn apply_official_beneath(official: &OverrideSet, personal: &OverrideSet) -> OverrideSet {
    if official.is_empty() {
        return personal.clone();
    }
    let mut out = official.clone();
    layer_beneath(&mut out, personal); // personal wins on top of official
    out.author_npub = personal.author_npub.clone();
    out.derivation_chain = personal.derivation_chain.clone();
    if out.version == 0 {
        out.version = OVERRIDE_SET_VERSION;
    }
    out
}

/// The render set for a world, composing the three appearance layers in order
/// (Spec 40 wardrobe persistence): official catalogue BENEATH -> the player's
/// global wardrobe -> the per-world override SUPERSEDES. `remember == false`
/// ("Start at standard") drops the personal + world-override layers entirely,
/// leaving stock + official only (the saved wardrobe is NOT erased -- that is the
/// caller's storage concern). Pure: the single source of truth for load order,
/// unit-tested independently of any I/O.
pub fn resolve_render_set(
    official: &OverrideSet,
    personal: &OverrideSet,
    world_override: Option<&OverrideSet>,
    remember: bool,
) -> OverrideSet {
    if !remember {
        // Stock + official only (the saved personal wardrobe is left untouched).
        return official.clone();
    }
    let mut render = apply_official_beneath(official, personal); // personal wins over official
    if let Some(wo) = world_override
        && !wo.is_empty()
    {
        layer_beneath(&mut render, wo); // world override wins per key over everything
        if render.version == 0 {
            render.version = OVERRIDE_SET_VERSION;
        }
    }
    render
}

/// The embedded official catalogue JSON (empty until the owner runs the live bake).
pub fn official_catalogue() -> &'static str {
    include_str!("../assets/official_overrides.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::override_registry::{
        content_hash, AuthoredFaces, DesignLibrary, NamedDesign, OverrideRegistry,
    };
    const FIXTURE: &str = include_str!("../assets/official_overrides.fixture.json");
    const EMPTY: &str = include_str!("../assets/official_overrides.json");

    /// One-design active library wrapping a solid reskin — the wardrobe equivalent
    /// of the v1 `block_tex.push((block, faces))`.
    fn one_active(faces: AuthoredFaces) -> DesignLibrary {
        let mut lib = DesignLibrary::default();
        lib.push_design(NamedDesign {
            id: 0,
            name: "test".into(),
            faces: Some(faces),
            micro_model: None,
            author_npub: String::new(),
            derivation_chain: vec![],
        });
        lib
    }

    /// The first face's first RGBA texel of a library's active design.
    fn active_first_texel(lib: &DesignLibrary) -> [u8; 4] {
        let d = lib.active_design().expect("active design");
        let f = &d.faces.as_ref().expect("paint design").faces[0];
        [f[0], f[1], f[2], f[3]]
    }

    #[test]
    fn empty_catalogue_is_a_noop() {
        let set = load_official_set(EMPTY).unwrap();
        assert!(set.is_empty());
    }
    #[test]
    fn fixture_catalogue_loads_an_override_set() {
        let set = load_official_set(FIXTURE).unwrap();
        assert!(!set.block_designs.is_empty(), "official reskin present");
        assert!(set
            .block_designs
            .iter()
            .any(|(b, _)| *b == crate::block::CORNFLOWER));
    }
    #[test]
    fn personal_overrides_take_precedence_over_official() {
        let official = load_official_set(FIXTURE).unwrap(); // official: CORNFLOWER = orange
        let mut personal = OverrideSet::default();
        personal
            .block_designs
            .push((crate::block::CORNFLOWER, one_active(AuthoredFaces::solid([7, 7, 7, 255])))); // personal grey
        let combined = apply_official_beneath(&official, &personal);
        let cf = combined
            .block_designs
            .iter()
            .find(|(b, _)| *b == crate::block::CORNFLOWER)
            .unwrap();
        assert_eq!(
            active_first_texel(&cf.1),
            [7u8, 7, 7, 255],
            "personal wins over official"
        );
        assert_eq!(
            combined
                .block_designs
                .iter()
                .filter(|(b, _)| *b == crate::block::CORNFLOWER)
                .count(),
            1,
            "no dup key"
        );
    }
    #[test]
    fn empty_official_returns_personal_unchanged() {
        let personal = {
            let mut p = OverrideSet::default();
            p.block_designs
                .push((crate::block::STONE, one_active(AuthoredFaces::solid([1, 2, 3, 255]))));
            p
        };
        let combined = apply_official_beneath(&OverrideSet::default(), &personal);
        assert_eq!(
            content_hash(&combined),
            content_hash(&personal),
            "empty official = personal untouched"
        );
    }
    #[test]
    fn fixture_official_applies_for_a_fresh_player() {
        // No personal overrides -> official shows through, and rebuilds to a live layer.
        let official = load_official_set(FIXTURE).unwrap();
        let combined = apply_official_beneath(&official, &OverrideSet::default());
        let base = crate::texture_gen::texture_count();
        let reg = OverrideRegistry::from_set(combined, base);
        assert!(
            reg.block_face_layer(crate::block::CORNFLOWER, 0).is_some(),
            "official reskin is live for a fresh player"
        );
    }

    #[test]
    fn resolver_layers_personal_over_official() {
        let official = load_official_set(FIXTURE).unwrap(); // CORNFLOWER = orange
        let mut personal = OverrideSet::default();
        personal
            .block_designs
            .push((crate::block::CORNFLOWER, one_active(AuthoredFaces::solid([7, 7, 7, 255]))));
        let render = resolve_render_set(&official, &personal, None, true);
        let cf = render.block_designs.iter().find(|(b, _)| *b == crate::block::CORNFLOWER).unwrap();
        assert_eq!(active_first_texel(&cf.1), [7, 7, 7, 255], "personal beats official");
    }

    #[test]
    fn resolver_world_override_supersedes_personal() {
        let official = OverrideSet::default();
        let mut personal = OverrideSet::default();
        personal
            .block_designs
            .push((crate::block::CORNFLOWER, one_active(AuthoredFaces::solid([1, 1, 1, 255]))));
        let mut world_override = OverrideSet::default();
        world_override
            .block_designs
            .push((crate::block::CORNFLOWER, one_active(AuthoredFaces::solid([9, 9, 9, 255]))));
        let render = resolve_render_set(&official, &personal, Some(&world_override), true);
        let cf = render.block_designs.iter().find(|(b, _)| *b == crate::block::CORNFLOWER).unwrap();
        assert_eq!(active_first_texel(&cf.1), [9, 9, 9, 255], "world override supersedes personal");
        assert_eq!(
            render.block_designs.iter().filter(|(b, _)| *b == crate::block::CORNFLOWER).count(),
            1,
            "no duplicate key"
        );
    }

    #[test]
    fn resolver_start_at_standard_drops_personal_and_override() {
        let official = load_official_set(FIXTURE).unwrap(); // official CORNFLOWER survives
        let mut personal = OverrideSet::default();
        personal
            .block_designs
            .push((crate::block::STONE, one_active(AuthoredFaces::solid([1, 1, 1, 255]))));
        let mut world_override = OverrideSet::default();
        world_override
            .block_designs
            .push((crate::block::DIRT, one_active(AuthoredFaces::solid([2, 2, 2, 255]))));
        let render = resolve_render_set(&official, &personal, Some(&world_override), false);
        assert!(render.block_designs.iter().all(|(b, _)| *b != crate::block::STONE));
        assert!(render.block_designs.iter().all(|(b, _)| *b != crate::block::DIRT));
        assert!(render.block_designs.iter().any(|(b, _)| *b == crate::block::CORNFLOWER), "official kept");
    }

    #[test]
    fn resolver_empty_inputs_are_stock() {
        let render = resolve_render_set(&OverrideSet::default(), &OverrideSet::default(), None, true);
        assert!(render.is_empty(), "nothing anywhere => byte-identical stock");
    }
}
