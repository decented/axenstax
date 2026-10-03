//! The Workshop — per-asset appearance override layer (Spec 40, Phases A + 5).
//!
//! The one load-bearing new system behind the community-redesign pipeline: a
//! **personal, per-asset appearance override** applied at render time. Everything
//! the Workshop authors is an *override of an existing asset* — repaint a
//! flower's faces and **every** flower updates; repaint a cow and **every** cow
//! updates. It is **render-only** (block ids, mob types, saves, inventory,
//! crafting, behaviour are all untouched), which is what makes it
//! multiplayer-safe and lets it sidestep server authority entirely.
//!
//! This module is the concrete realisation of the resource-pack / texture-override
//! path (`docs/spec/03-rendering.md` §3.2–3.3): authored 16×16 RGBA faces are
//! **appended as extra layers** to the block texture array, and an
//! `(asset, face) → layer` map is consulted at the two render seams (the block
//! mesher in `mesh.rs` and the entity vertex builder in `entity_model.rs`)
//! **before** the asset's default texture.
//!
//! ## Phase 5 — the per-block wardrobe
//! Each key (block id / mob part) no longer maps to a *single* reskin: it maps to a
//! [`DesignLibrary`] — a small bounded collection of [`NamedDesign`]s plus the id of
//! the one currently *worn* (`active`). Pin **appends** a design (never overwrites);
//! adopt **slots** the other player's designs in *inactive*; a Wardrobe panel
//! set-actives / renames / deletes / reverts-to-stock. Only the ACTIVE design of a
//! library contributes any texture layers, so a fat wardrobe of inactive designs
//! can never blow the texture-array layer cap. A design can carry a paint reskin
//! (`faces`) and/or a shape reskin (`micro_model`).
//!
//! ## Two forms
//! - [`OverrideSet`] — the **serializable authored data**. Provenance-bearing
//!   (`author_npub` + a derivation chain). It is the wire format for Beacon
//!   publish/adopt and the embedded official catalogue (`to_blob_bytes` /
//!   `from_blob_bytes`). It is ALSO the **persisted** form of a player's wardrobe:
//!   the player-global wardrobe rides the private Stash (WASM) / a `profile/`
//!   profile file (native) as an `OverrideSet` blob, and a per-world override rides
//!   `WorldMeta.world_override`. See `wardrobe_store` (the storage seam) +
//!   `official_overrides::resolve_render_set` (the load-order resolver).
//!   `world.dat` itself carries no `OverrideSet` — appearance is render-only.
//! - [`OverrideRegistry`] — the **runtime** view that owns an [`OverrideSet`] plus
//!   the derived `(asset, face) → layer` map and the ordered list of appended RGBA
//!   buffers. Default-empty ⇒ no overrides ⇒ byte-identical render.
//!   [`crate::world::World`] holds TWO: `player_wardrobe` (the AUTHORED player-global
//!   set, restored from the wardrobe store on world entry) and `override_registry`
//!   (the DERIVED render view, rebuilt from official + player_wardrobe + per-world
//!   override by `GameState::reapply_overrides`).
//!
//! ## bincode migration (v1 → v2)
//! [`OverrideSet`] is serialised with **bincode** (positional, non-self-describing)
//! and persisted as a version-byte-prefixed blob ([`OverrideSet::to_blob_bytes`] /
//! [`OverrideSet::from_blob_bytes`]). These blobs are LIVE: user-published Beacon
//! designs + the embedded official catalogue. bincode does NOT honour
//! `#[serde(default)]` for added/changed fields, so changing the element type of
//! `block_tex`/`mob_tex` would silently corrupt every v1 blob. The migration is
//! therefore concentrated in ONE function: [`OverrideSet::from_blob_bytes`] branches
//! on the version byte (v1 bytes → [`OverrideSetV1`] → [`migrate_v1`]; v2 bytes →
//! v2; v>2 → reject). `world.dat` carries NO `OverrideSet`, so there is no
//! world-save migration.
//!
//! Face indexing is the consumer's convention: **block** faces use
//! `mesh::Face::index()` (Top=0, Bottom=1, North=2, South=3, East=4, West=5);
//! **mob** part faces use the part's `tex_faces` slot order (+x, -x, +y, -y, +z, -z).

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::block::BlockId;
use crate::mob::MobType;

/// Bytes in one authored face: 16×16 RGBA. Matches `texture_gen::SIZE` (16) — one
/// sub-voxel ↔ one texel — so the painter grid is 16×16.
pub const FACE_BYTES: usize = 16 * 16 * 4;

/// Number of faces on any asset part / block: the 6 cube directions.
pub const FACES: usize = 6;

/// A full per-face reskin of an asset part: 6 authored 16×16 RGBA faces.
///
/// Interpretation of the 6 slots depends on the keying context:
/// - **Block** override → indexed by `mesh::Face::index()` (Top=0 … West=5).
/// - **Mob** part override → indexed by the part's `tex_faces` slot (+x, -x, +y,
///   -y, +z, -z).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredFaces {
    /// 6 faces, each a 16×16 RGBA buffer of length [`FACE_BYTES`].
    pub faces: [Vec<u8>; 6],
}

impl AuthoredFaces {
    /// A uniform reskin — all six faces filled with one RGBA colour. The common
    /// painter starting point and the simplest thing to assert in tests.
    pub fn solid(rgba: [u8; 4]) -> Self {
        let face: Vec<u8> = rgba.iter().copied().cycle().take(FACE_BYTES).collect();
        Self {
            faces: [
                face.clone(),
                face.clone(),
                face.clone(),
                face.clone(),
                face.clone(),
                face,
            ],
        }
    }

    /// Build from six explicit face buffers.
    pub fn from_faces(faces: [Vec<u8>; 6]) -> Self {
        Self { faces }
    }

    /// Every face must be exactly [`FACE_BYTES`] long to be a valid 16×16 RGBA
    /// layer. Authoring/adoption paths reject malformed sets rather than uploading
    /// a wrongly-sized texture layer. No production caller — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_valid(&self) -> bool {
        self.faces.iter().all(|f| f.len() == FACE_BYTES)
    }
}

/// Identifies a single overridable mob part: a mob type plus the part's index in
/// its `entity_model` model (`mob_model(kind)`), which is positional — there is no
/// part-name field, so the index *is* the stable identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MobPartKey {
    pub mob: MobType,
    /// Index into `entity_model::mob_model(mob)`.
    pub part: u8,
}

/// One ancestor link in an override's derivation chain. Mirrors the Plaque /
/// `plan::DerivationLink` pattern: chains are append-only and carry the author's
/// **npub** (NIP-19, never hex — `feedback_npub_only_display`) plus the content
/// hash of the ancestor, so a future "this reskin descends from theirs" view is a
/// drop-in once sharing/adoption ship (Spec 40 Phases D/E).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverrideDerivation {
    pub author_npub: String,
    pub content_hash: [u8; 32],
}

/// Current [`OverrideSet`] format version. Bumped 1→2 for the per-block wardrobe
/// (`block_tex`/`mob_tex` → `block_designs`/`mob_designs`). The version byte in
/// [`OverrideSet::from_blob_bytes`] selects the migration path; v1 blobs upgrade
/// clean, v>2 blobs are rejected.
pub const OVERRIDE_SET_VERSION: u8 = 2;

/// Stable id for one design within a [`DesignLibrary`]. Assigned monotonically from
/// the library's `next_id`; never reused, so a UI / derivation can reference a
/// design by id without ambiguity across renames or deletes.
pub type DesignId = u32;

/// One named appearance in a block / mob-part's wardrobe. A design can carry a paint
/// reskin (`faces`), a shape reskin (`micro_model`), or both. Provenance-bearing so
/// an adopted design remembers who authored it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamedDesign {
    /// Stable within the owning library (assigned by `DesignLibrary::push_design`).
    pub id: DesignId,
    /// Player-facing label (defaults like "Original" / "Design 2"; renameable).
    pub name: String,
    /// Paint reskin — the 6 authored faces. `None` for a shape-only design.
    pub faces: Option<AuthoredFaces>,
    /// Shape reskin — a coloured micro-model. `None` for a paint-only design.
    pub micro_model: Option<crate::micro_model::MicroModelData>,
    /// Authoring player's npub (NIP-19 bech32). Empty for local / unattributed.
    pub author_npub: String,
    /// Append-only attribution chain for this individual design.
    pub derivation_chain: Vec<OverrideDerivation>,
}

/// A bounded, per-key collection of [`NamedDesign`]s plus the id of the one
/// currently *worn*. `active = None` ⇒ the asset renders stock. Pin appends (and
/// makes the new design active); adopt slots a design in *inactive*; the cap evicts
/// the oldest non-active design so the wardrobe stays bounded.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DesignLibrary {
    /// The wardrobe, oldest-first (push order).
    pub designs: Vec<NamedDesign>,
    /// The id of the worn design, or `None` for stock.
    pub active: Option<DesignId>,
    /// Monotonic id source — never reused, so deletes/renames don't alias.
    pub next_id: DesignId,
}

impl DesignLibrary {
    /// Per-key wardrobe cap. Pushing past this evicts the oldest non-active design.
    pub const MAX_DESIGNS: usize = 16;

    /// The currently-worn design, or `None` if `active` is unset / dangling.
    pub fn active_design(&self) -> Option<&NamedDesign> {
        let active = self.active?;
        self.designs.iter().find(|d| d.id == active)
    }

    /// Append a design, assign it a fresh id, and make it active. If this pushes the
    /// library past [`MAX_DESIGNS`], evict the oldest design whose id != active and
    /// return its id; otherwise return `None`. (The just-pushed design is active, so
    /// it is never the eviction target.)
    pub fn push_design(&mut self, mut d: NamedDesign) -> Option<DesignId> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        d.id = id;
        self.designs.push(d);
        self.active = Some(id);
        if self.designs.len() > Self::MAX_DESIGNS {
            // Evict the FIRST (oldest) design that isn't the active one.
            if let Some(pos) = self.designs.iter().position(|d| Some(d.id) != self.active) {
                return Some(self.designs.remove(pos).id);
            }
        }
        None
    }

    /// Wear a different design. No-op + `false` if no design with that id exists.
    pub fn set_active(&mut self, id: DesignId) -> bool {
        if self.designs.iter().any(|d| d.id == id) {
            self.active = Some(id);
            true
        } else {
            false
        }
    }

    /// Revert to stock — wear nothing.
    pub fn use_original(&mut self) {
        self.active = None;
    }

    /// Rename a design. `false` if no design with that id exists.
    pub fn rename(&mut self, id: DesignId, name: String) -> bool {
        if let Some(d) = self.designs.iter_mut().find(|d| d.id == id) {
            d.name = name;
            true
        } else {
            false
        }
    }

    /// Remove a design. If it was the active one, revert to stock. `false` if no
    /// design with that id exists.
    pub fn remove(&mut self, id: DesignId) -> bool {
        if let Some(pos) = self.designs.iter().position(|d| d.id == id) {
            self.designs.remove(pos);
            if self.active == Some(id) {
                self.active = None;
            }
            true
        } else {
            false
        }
    }

    /// Slot a design in **inactive** (adopt path): assign a fresh id and push, but
    /// do NOT change `active`. The adopting player keeps wearing what they wore.
    /// Bounded at [`MAX_DESIGNS`] like [`push_design`] (evicting the oldest
    /// non-active design) so a hostile/large adopted blob can't grow the wardrobe —
    /// and hence the player's public, append-only published blob — without limit.
    pub fn adopt_inactive(&mut self, mut d: NamedDesign) {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        d.id = id;
        self.designs.push(d);
        if self.designs.len() > Self::MAX_DESIGNS {
            // Evict the oldest design that isn't the active one (never the worn one);
            // with `active == None` this is simply the oldest design.
            if let Some(pos) = self.designs.iter().position(|d| Some(d.id) != self.active) {
                self.designs.remove(pos);
            }
        }
    }
}

/// The **serializable authored override data** (v2). Rides world saves + Beacon /
/// Stash blobs. Each key (block id / mob part) maps to a [`DesignLibrary`].
///
/// Stored as `Vec<(key, value)>` rather than maps so the format serialises cleanly
/// across **both** bincode and JSON (JSON map keys must be strings, which a
/// `BlockId` / [`MobPartKey`] is not). The runtime [`OverrideRegistry`] builds hash
/// maps from the active designs on load.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OverrideSet {
    #[serde(default)]
    pub version: u8,
    /// Authoring player's npub (NIP-19 bech32). Empty until authored.
    #[serde(default)]
    pub author_npub: String,
    /// Append-only attribution chain (set-level — adoption links).
    #[serde(default)]
    pub derivation_chain: Vec<OverrideDerivation>,
    /// Per-block wardrobes.
    #[serde(default)]
    pub block_designs: Vec<(BlockId, DesignLibrary)>,
    /// Per-mob-part wardrobes.
    #[serde(default)]
    pub mob_designs: Vec<(MobPartKey, DesignLibrary)>,
}

/// The **exact** v1 `OverrideSet` shape — preserved so LIVE v1 blobs (published
/// Beacon designs + the embedded official catalogue) can still be decoded and
/// migrated. Do NOT change this struct; it mirrors the bytes already in the wild.
#[derive(Serialize, Deserialize)]
pub struct OverrideSetV1 {
    #[serde(default)]
    pub version: u8,
    #[serde(default)]
    pub author_npub: String,
    #[serde(default)]
    pub derivation_chain: Vec<OverrideDerivation>,
    #[serde(default)]
    pub block_tex: Vec<(BlockId, AuthoredFaces)>,
    #[serde(default)]
    pub mob_tex: Vec<(MobPartKey, AuthoredFaces)>,
}

/// Migrate a decoded v1 set to v2: each single `(key, faces)` becomes a
/// [`DesignLibrary`] holding one active "Original" [`NamedDesign`]. Set-level
/// provenance (`author_npub` + `derivation_chain`) is carried onto both the set and
/// each migrated design.
fn migrate_v1(v1: OverrideSetV1) -> OverrideSet {
    let author = v1.author_npub;
    let chain = v1.derivation_chain;
    let make_lib = |faces: AuthoredFaces| DesignLibrary {
        designs: vec![NamedDesign {
            id: 0,
            name: "Original".into(),
            faces: Some(faces),
            micro_model: None,
            author_npub: author.clone(),
            derivation_chain: chain.clone(),
        }],
        active: Some(0),
        next_id: 1,
    };
    // Dedup keys defensively: a crafted/untrusted v1 blob could list the same block
    // (or mob part) twice. `rebuild_layers` is last-wins but the library mutators
    // (`find`-first) would target a different entry — so collapse to one library per
    // key here, last v1 entry winning (matches the render path).
    let mut block_designs: Vec<(BlockId, DesignLibrary)> = Vec::new();
    for (b, f) in v1.block_tex {
        let lib = make_lib(f);
        match block_designs.iter_mut().find(|(k, _)| *k == b) {
            Some(slot) => slot.1 = lib,
            None => block_designs.push((b, lib)),
        }
    }
    let mut mob_designs: Vec<(MobPartKey, DesignLibrary)> = Vec::new();
    for (k, f) in v1.mob_tex {
        let lib = make_lib(f);
        match mob_designs.iter_mut().find(|(mk, _)| *mk == k) {
            Some(slot) => slot.1 = lib,
            None => mob_designs.push((k, lib)),
        }
    }
    OverrideSet {
        version: OVERRIDE_SET_VERSION,
        author_npub: author,
        derivation_chain: chain,
        block_designs,
        mob_designs,
    }
}

impl OverrideSet {
    /// True when nothing is overridden — the default state, which renders
    /// byte-identically to a stock game.
    pub fn is_empty(&self) -> bool {
        self.block_designs.is_empty() && self.mob_designs.is_empty()
    }

    /// Adopt another creator's override set into mine: slot every one of THEIR
    /// designs into the matching library **inactive** (never changing what I wear),
    /// and record exactly one derivation link (who I adopted from + their appearance
    /// hash). Appearance-only: ids/behaviour/saves are untouched.
    /// Consumed only on the wasm adopt path (+ tests) — hence the native dead-code allow.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn merge_adopted(&mut self, other: &OverrideSet) {
        let link = OverrideDerivation {
            author_npub: other.author_npub.clone(),
            content_hash: content_hash(other),
        };
        for (block, their_lib) in &other.block_designs {
            let my_lib = match self.block_designs.iter_mut().find(|(b, _)| b == block) {
                Some((_, lib)) => lib,
                None => {
                    self.block_designs.push((*block, DesignLibrary::default()));
                    &mut self.block_designs.last_mut().unwrap().1
                }
            };
            for d in &their_lib.designs {
                my_lib.adopt_inactive(d.clone());
            }
        }
        for (key, their_lib) in &other.mob_designs {
            let my_lib = match self.mob_designs.iter_mut().find(|(k, _)| k == key) {
                Some((_, lib)) => lib,
                None => {
                    self.mob_designs.push((*key, DesignLibrary::default()));
                    &mut self.mob_designs.last_mut().unwrap().1
                }
            };
            for d in &their_lib.designs {
                my_lib.adopt_inactive(d.clone());
            }
        }
        self.derivation_chain.push(link);
        if self.version == 0 {
            self.version = OVERRIDE_SET_VERSION;
        }
    }

    /// Serialise to the bytes published as a Beacon `override-set` blob: a 1-byte
    /// format version tag followed by the bincode encoding (v2 shape). The version
    /// byte is the permanent forward-compat guard — a future format bump can be
    /// detected and rejected by an older client rather than silently misparsed.
    pub fn to_blob_bytes(&self) -> Result<Vec<u8>, String> {
        let mut blob = vec![OVERRIDE_SET_VERSION];
        blob.extend(bincode::serialize(self).map_err(|e| format!("serialise override set: {e}"))?);
        Ok(blob)
    }

    /// Parse an adopted Beacon `override-set` blob, migrating older formats. Rejects
    /// an empty blob, a newer-than-supported version, and malformed bincode — never
    /// panics. v0/v1 bytes decode as [`OverrideSetV1`] and migrate to v2; v2 bytes
    /// decode directly.
    pub fn from_blob_bytes(bytes: &[u8]) -> Result<Self, String> {
        let (&ver, rest) = bytes
            .split_first()
            .ok_or_else(|| "empty override-set blob".to_string())?;
        if ver > OVERRIDE_SET_VERSION {
            return Err(format!(
                "override set v{ver} is newer than supported v{OVERRIDE_SET_VERSION} — update the game"
            ));
        }
        if ver <= 1 {
            // v0 (old default) and v1 share the legacy single-design shape.
            let v1: OverrideSetV1 =
                bincode::deserialize(rest).map_err(|e| format!("parse override set v1: {e}"))?;
            Ok(migrate_v1(v1))
        } else {
            bincode::deserialize(rest).map_err(|e| format!("parse override set: {e}"))
        }
    }
}

/// Content hash of the **authored appearance only** — set-level provenance
/// (`author_npub`, `derivation_chain`) is zeroed before hashing, mirroring
/// `plan::content_hash` and the v1 hash semantics (the hash is about *appearance*,
/// so two sets with identical wardrobes but different authors hash the same, and the
/// hash is stable across a serde round-trip). Per-design author/derivation are left
/// intact — they're part of the appearance payload that travels with a design. The
/// version byte is included (v1 and v2 of the same wardrobe legitimately differ).
/// SHA-256 over a bincode encoding, like the Plan path.
pub fn content_hash(set: &OverrideSet) -> [u8; 32] {
    let canon = OverrideSet {
        version: set.version,
        author_npub: String::new(),
        derivation_chain: Vec::new(),
        block_designs: set.block_designs.clone(),
        mob_designs: set.mob_designs.clone(),
    };
    let bytes = bincode::serialize(&canon).expect("OverrideSet bincode");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher.finalize().into()
}

/// SHA-256 of one face buffer — used to deduplicate identical authored faces so a
/// reskin with repeated faces (e.g. a uniform block) consumes one appended layer,
/// not six. This is the layer-reuse care the cosmetics native-crash note flagged
/// (`project_player_cosmetics_plan`, Spec 40 Open Question #3): don't leak texture
/// array layers.
fn face_hash(buf: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(buf);
    hasher.finalize().into()
}

/// Outcome of appending a design to a library: the id the new design was assigned
/// (now active) plus the id of any design evicted to keep the wardrobe under the
/// cap. The caller (e.g. the pin path) uses `evicted` to surface a "wardrobe full"
/// toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AddDesignOutcome {
    pub active_id: DesignId,
    pub evicted: Option<DesignId>,
}

/// Runtime override registry — owns an [`OverrideSet`] plus the derived
/// `(asset, face) → layer` map and the ordered RGBA buffers appended to the block
/// texture array. Lives on [`crate::world::World`]. Default-empty renders identically
/// to a stock game. Only each library's ACTIVE design contributes layers.
#[derive(Clone, Debug, Default)]
pub struct OverrideRegistry {
    set: OverrideSet,
    /// `(block, face_index 0..6) → appended array layer`.
    block_layer: AHashMap<(BlockId, u8), u32>,
    /// `(mob part, face slot 0..6) → appended array layer`.
    mob_layer: AHashMap<(MobPartKey, u8), u32>,
    /// RGBA buffers appended after the base block textures, in layer order. Index
    /// `i` in this list is GPU array layer `base + i`.
    appended: Vec<Vec<u8>>,
    /// The base block-texture layer count this map was built against
    /// (`texture_gen::texture_count()` at rebuild time).
    base: u32,
}

impl OverrideRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a runtime registry from authored data and assign appended layers
    /// against `base` (= `texture_gen::texture_count()`).
    pub fn from_set(set: OverrideSet, base: u32) -> Self {
        let mut reg = Self {
            set,
            ..Default::default()
        };
        reg.rebuild_layers(base);
        reg
    }

    /// Borrow the authored data (for saving / hashing / the Wardrobe panel).
    pub fn set(&self) -> &OverrideSet {
        &self.set
    }

    /// True when nothing is overridden.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Append a design into a block's wardrobe (making it active) and re-derive the
    /// layer map. Pass `base` = `texture_gen::texture_count()`. Returns the new
    /// active id + any evicted id (cap = [`DesignLibrary::MAX_DESIGNS`]).
    pub fn add_block_design(
        &mut self,
        block: BlockId,
        design: NamedDesign,
        base: u32,
    ) -> AddDesignOutcome {
        let lib = match self.set.block_designs.iter_mut().find(|(b, _)| *b == block) {
            Some((_, lib)) => lib,
            None => {
                self.set.block_designs.push((block, DesignLibrary::default()));
                &mut self.set.block_designs.last_mut().unwrap().1
            }
        };
        let evicted = lib.push_design(design);
        let active_id = lib.active.expect("push_design sets active");
        self.rebuild_layers(base);
        AddDesignOutcome { active_id, evicted }
    }

    /// Append a design into a mob part's wardrobe (making it active) and re-derive
    /// the layer map.
    pub fn add_mob_design(
        &mut self,
        key: MobPartKey,
        design: NamedDesign,
        base: u32,
    ) -> AddDesignOutcome {
        let lib = match self.set.mob_designs.iter_mut().find(|(k, _)| *k == key) {
            Some((_, lib)) => lib,
            None => {
                self.set.mob_designs.push((key, DesignLibrary::default()));
                &mut self.set.mob_designs.last_mut().unwrap().1
            }
        };
        let evicted = lib.push_design(design);
        let active_id = lib.active.expect("push_design sets active");
        self.rebuild_layers(base);
        AddDesignOutcome { active_id, evicted }
    }

    /// Wear a different design for a block, then rebuild. `false` if no such block
    /// library or design id.
    ///
    pub fn set_block_active(&mut self, block: BlockId, id: DesignId, base: u32) -> bool {
        let ok = self
            .set
            .block_designs
            .iter_mut()
            .find(|(b, _)| *b == block)
            .map(|(_, lib)| lib.set_active(id))
            .unwrap_or(false);
        if ok {
            self.rebuild_layers(base);
        }
        ok
    }

    /// Revert a block to stock (wear nothing), then rebuild.
    pub fn block_use_original(&mut self, block: BlockId, base: u32) {
        if let Some((_, lib)) = self.set.block_designs.iter_mut().find(|(b, _)| *b == block) {
            lib.use_original();
            self.rebuild_layers(base);
        }
    }

    /// Rename a block design. `false` if no such block library or design id. No
    /// rebuild needed (names don't affect layers).
    pub fn rename_block_design(&mut self, block: BlockId, id: DesignId, name: String) -> bool {
        self.set
            .block_designs
            .iter_mut()
            .find(|(b, _)| *b == block)
            .map(|(_, lib)| lib.rename(id, name))
            .unwrap_or(false)
    }

    /// Delete a block design, then rebuild (the active design may have changed to
    /// `None`). `false` if no such block library or design id.
    pub fn delete_block_design(&mut self, block: BlockId, id: DesignId, base: u32) -> bool {
        let ok = self
            .set
            .block_designs
            .iter_mut()
            .find(|(b, _)| *b == block)
            .map(|(_, lib)| lib.remove(id))
            .unwrap_or(false);
        if ok {
            self.rebuild_layers(base);
        }
        ok
    }

    /// The active SHAPE designs across all block libraries: `(block, micro_model)`
    /// for each block whose active design carries a `micro_model`. The game loop
    /// syncs `world.micro_registry` from this so a set-active / pin / adopt shows the
    /// live shape.
    pub fn active_micro_models(&self) -> Vec<(BlockId, crate::micro_model::MicroModelData)> {
        let mut out = Vec::new();
        for (block, lib) in &self.set.block_designs {
            if let Some(d) = lib.active_design()
                && let Some(model) = &d.micro_model
            {
                out.push((*block, model.clone()));
            }
        }
        out
    }

    /// Recompute the `(asset, face) → layer` map and the ordered appended-buffer
    /// list from the authored data — sourcing layers from each library's ACTIVE
    /// design only (inactive designs and `active = None` contribute no layers, i.e.
    /// stock). Identical face buffers share one layer (content-addressed dedup) so
    /// the texture array doesn't leak layers when a reskin repeats a face.
    pub fn rebuild_layers(&mut self, base: u32) {
        self.base = base;
        self.block_layer.clear();
        self.mob_layer.clear();
        self.appended.clear();
        let mut by_hash: AHashMap<[u8; 32], u32> = AHashMap::new();

        for (block, lib) in &self.set.block_designs {
            if let Some(d) = lib.active_design()
                && let Some(faces) = &d.faces
            {
                for fi in 0..FACES as u8 {
                    let layer = assign_layer(
                        &faces.faces[fi as usize],
                        base,
                        &mut self.appended,
                        &mut by_hash,
                    );
                    self.block_layer.insert((*block, fi), layer);
                }
            }
        }
        for (key, lib) in &self.set.mob_designs {
            if let Some(d) = lib.active_design()
                && let Some(faces) = &d.faces
            {
                for fi in 0..FACES as u8 {
                    let layer = assign_layer(
                        &faces.faces[fi as usize],
                        base,
                        &mut self.appended,
                        &mut by_hash,
                    );
                    self.mob_layer.insert((*key, fi), layer);
                }
            }
        }
    }

    /// The override array layer for a block face, if any. `face_index` is
    /// `mesh::Face::index()`. Consulted by the block mesher **before** the default
    /// `tex_top/tex_bottom/tex_side`.
    pub fn block_face_layer(&self, block: BlockId, face_index: u8) -> Option<u32> {
        self.block_layer.get(&(block, face_index)).copied()
    }

    /// Resolve a mob part's effective `tex_faces`, applying any per-face overrides
    /// over the part's defaults. Returns `Some(faces)` if **any** face of this
    /// `(mob, part)` is overridden, else `None` (so an unmodified part stays
    /// byte-identical). `face_slot` is the part's `tex_faces` slot order.
    pub fn mob_part_faces(
        &self,
        mob: MobType,
        part: u8,
        default_faces: &[u32; FACES],
    ) -> Option<[u32; FACES]> {
        let key = MobPartKey { mob, part };
        let mut out = *default_faces;
        let mut any = false;
        for slot in 0..FACES as u8 {
            if let Some(layer) = self.mob_layer.get(&(key, slot)).copied() {
                out[slot as usize] = layer;
                any = true;
            }
        }
        if any {
            Some(out)
        } else {
            None
        }
    }

    /// The RGBA buffers to append to the block texture array, in layer order. The
    /// renderer uploads these starting at array layer `self.base`. Empty ⇒ no GPU
    /// change ⇒ array stays at `texture_count()` layers (byte-identical).
    pub fn appended_layers(&self) -> &[Vec<u8>] {
        &self.appended
    }

    /// Number of appended override layers (for sizing the texture array).
    pub fn appended_len(&self) -> u32 {
        self.appended.len() as u32
    }

    /// Content hash of the authored appearance (provenance-independent).
    /// No caller — the free function `content_hash` above is called directly
    /// wherever this is needed, not through this `OverrideRegistry` wrapper.
    #[allow(dead_code)]
    pub fn content_hash(&self) -> [u8; 32] {
        content_hash(&self.set)
    }
}

/// Assign (or reuse, by content hash) an appended layer for one authored face
/// buffer. Returns the GPU array layer index (`base + position`).
fn assign_layer(
    buf: &[u8],
    base: u32,
    appended: &mut Vec<Vec<u8>>,
    by_hash: &mut AHashMap<[u8; 32], u32>,
) -> u32 {
    let h = face_hash(buf);
    if let Some(&layer) = by_hash.get(&h) {
        return layer;
    }
    let layer = base + appended.len() as u32;
    appended.push(buf.to_vec());
    by_hash.insert(h, layer);
    layer
}

/// Apply an adopted Beacon `override-set` blob to a registry, BENEATH a texture-
/// array safety cap. Decodes (version-checked + migrated) → slots their designs in
/// inactive (merge_adopted) → rebuilds layers → but REJECTS (without mutating the
/// registry) if the resulting layer count would exceed `max_total_layers`
/// (the device's max_texture_array_layers). Because only the ACTIVE design of each
/// library contributes layers and adoption slots designs *inactive*, adopting many
/// designs cannot blow the cap; the check stays as a hard safety floor.
/// `base` is `texture_gen::texture_count()`. On success the registry is replaced
/// with the merged, rebuilt one; the caller re-textures + re-meshes.
/// Consumed only on the wasm adopt path (+ tests) — hence the native dead-code allow.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn apply_adopted_override_bytes(
    registry: &mut OverrideRegistry,
    bytes: &[u8],
    base: u32,
    max_total_layers: u32,
) -> Result<(), String> {
    let adopted = OverrideSet::from_blob_bytes(bytes)?; // version + garbage guard + migration
    let mut merged = registry.set().clone();
    merged.merge_adopted(&adopted);
    let candidate = OverrideRegistry::from_set(merged, base);
    let total = base.saturating_add(candidate.appended_layers().len() as u32);
    if total > max_total_layers {
        return Err(format!(
            "that redesign needs {total} texture layers but this device allows {max_total_layers} — can't adopt it safely"
        ));
    }
    *registry = candidate;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Arbitrary base layer for the relative-append assertions below: these
    // tests only check that designs append at `BASE`, `BASE + 1`, … so the
    // absolute value is irrelevant and intentionally NOT coupled to
    // `texture_gen::texture_count()` (no drift to chase when textures grow).
    const BASE: u32 = 373;

    fn distinct_faces(seed: u8) -> AuthoredFaces {
        // Six *distinct* faces so dedup doesn't collapse them.
        let mut faces: [Vec<u8>; 6] = Default::default();
        for (i, f) in faces.iter_mut().enumerate() {
            *f = vec![seed.wrapping_add(i as u8); FACE_BYTES];
        }
        AuthoredFaces::from_faces(faces)
    }

    /// A bare paint design (no shape, no provenance) — the common test fixture.
    fn paint_design(name: &str, faces: AuthoredFaces) -> NamedDesign {
        NamedDesign {
            id: 0,
            name: name.into(),
            faces: Some(faces),
            micro_model: None,
            author_npub: String::new(),
            derivation_chain: vec![],
        }
    }

    #[test]
    fn authored_faces_solid_is_valid_and_right_size() {
        let f = AuthoredFaces::solid([10, 20, 30, 255]);
        assert!(f.is_valid());
        for face in &f.faces {
            assert_eq!(face.len(), FACE_BYTES);
        }
    }

    #[test]
    fn malformed_faces_rejected() {
        let bad = AuthoredFaces::from_faces([
            vec![0; FACE_BYTES],
            vec![0; 3], // wrong length
            vec![0; FACE_BYTES],
            vec![0; FACE_BYTES],
            vec![0; FACE_BYTES],
            vec![0; FACE_BYTES],
        ]);
        assert!(!bad.is_valid());
    }

    #[test]
    fn empty_registry_overrides_nothing() {
        let reg = OverrideRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.block_face_layer(1, 0), None);
        assert_eq!(reg.mob_part_faces(MobType::Cow, 0, &[7; 6]), None);
        assert!(reg.appended_layers().is_empty());
    }

    #[test]
    fn block_design_assigns_six_distinct_layers_from_base() {
        let mut reg = OverrideRegistry::new();
        reg.add_block_design(5, paint_design("a", distinct_faces(1)), BASE);
        // Six distinct faces ⇒ six appended layers, base..base+6.
        assert_eq!(reg.appended_len(), 6);
        for fi in 0..6u8 {
            assert_eq!(reg.block_face_layer(5, fi), Some(BASE + fi as u32));
        }
        // A different block has no override.
        assert_eq!(reg.block_face_layer(6, 0), None);
    }

    #[test]
    fn identical_faces_dedup_to_one_layer() {
        let mut reg = OverrideRegistry::new();
        // Solid = all six faces identical ⇒ one appended layer, all faces point at it.
        reg.add_block_design(9, paint_design("solid", AuthoredFaces::solid([1, 2, 3, 255])), BASE);
        assert_eq!(reg.appended_len(), 1);
        for fi in 0..6u8 {
            assert_eq!(reg.block_face_layer(9, fi), Some(BASE));
        }
    }

    #[test]
    fn mob_part_design_mixes_with_defaults() {
        let mut reg = OverrideRegistry::new();
        let key = MobPartKey {
            mob: MobType::Cow,
            part: 1,
        };
        reg.add_mob_design(key, paint_design("body", AuthoredFaces::solid([9, 9, 9, 255])), BASE);
        let resolved = reg
            .mob_part_faces(MobType::Cow, 1, &[100, 101, 102, 103, 104, 105])
            .expect("part 1 overridden");
        // Solid ⇒ all six faces map to the single appended layer (BASE).
        assert_eq!(resolved, [BASE; 6]);
        // A different part of the same mob is untouched.
        assert_eq!(reg.mob_part_faces(MobType::Cow, 2, &[1, 2, 3, 4, 5, 6]), None);
        // A different mob is untouched.
        assert_eq!(reg.mob_part_faces(MobType::Pig, 1, &[1, 2, 3, 4, 5, 6]), None);
    }

    #[test]
    fn adding_a_second_active_design_does_not_leak_layers() {
        let mut reg = OverrideRegistry::new();
        reg.add_block_design(3, paint_design("a", AuthoredFaces::solid([1, 1, 1, 255])), BASE);
        assert_eq!(reg.appended_len(), 1);
        // Add a second design to the SAME block: it becomes active, so still only one
        // active design contributes a layer (the inactive one costs nothing).
        reg.add_block_design(3, paint_design("b", AuthoredFaces::solid([2, 2, 2, 255])), BASE);
        assert_eq!(reg.appended_len(), 1, "rebuild must not accumulate stale layers");
        assert_eq!(reg.block_face_layer(3, 0), Some(BASE));
    }

    // ---- Phase 5: library data-model + migration ----

    #[test]
    fn library_serde_round_trips() {
        let lib = DesignLibrary {
            designs: vec![
                NamedDesign {
                    id: 0,
                    name: "red".into(),
                    faces: Some(AuthoredFaces::solid([255, 0, 0, 255])),
                    micro_model: None,
                    author_npub: String::new(),
                    derivation_chain: vec![],
                },
                NamedDesign {
                    id: 1,
                    name: "blue".into(),
                    faces: Some(AuthoredFaces::solid([0, 0, 255, 255])),
                    micro_model: None,
                    author_npub: String::new(),
                    derivation_chain: vec![],
                },
            ],
            active: Some(1),
            next_id: 2,
        };
        let mut set = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        set.block_designs.push((10, lib));
        let blob = set.to_blob_bytes().unwrap();
        let back = OverrideSet::from_blob_bytes(&blob).unwrap();
        assert_eq!(back.block_designs.len(), 1);
        assert_eq!(back.block_designs[0].1.designs.len(), 2);
        assert_eq!(back.block_designs[0].1.active, Some(1));
    }

    #[test]
    fn v1_blob_upgrades_to_one_entry_library() {
        // Build a v1-shape blob by hand and confirm it migrates clean.
        let v1 = OverrideSetV1 {
            version: 1,
            author_npub: "npub1abc".into(),
            derivation_chain: vec![],
            block_tex: vec![(10, AuthoredFaces::solid([1, 2, 3, 255]))],
            mob_tex: vec![],
        };
        let mut blob = vec![1u8]; // version byte 1
        blob.extend(bincode::serialize(&v1).unwrap());
        let back = OverrideSet::from_blob_bytes(&blob).unwrap();
        assert_eq!(back.block_designs.len(), 1, "one block migrated");
        let (_, lib) = &back.block_designs[0];
        assert_eq!(lib.designs.len(), 1, "one design");
        assert!(lib.active.is_some(), "the migrated design is active");
        assert_eq!(lib.active, Some(lib.designs[0].id));
        assert_eq!(
            lib.designs[0].faces.as_ref().unwrap(),
            &AuthoredFaces::solid([1, 2, 3, 255])
        );
        assert_eq!(lib.designs[0].author_npub, "npub1abc");
    }

    #[test]
    fn newer_version_blob_is_rejected() {
        let blob = vec![OVERRIDE_SET_VERSION + 1, 0, 0];
        assert!(OverrideSet::from_blob_bytes(&blob).is_err());
    }

    #[test]
    fn v1_blob_with_duplicate_keys_dedups_to_one_library_last_wins() {
        // A crafted/untrusted v1 blob could list the same block twice. Migration
        // must collapse it to ONE library (last entry wins, matching rebuild_layers)
        // so the render path and the library mutators agree.
        let v1 = OverrideSetV1 {
            version: 1,
            author_npub: String::new(),
            derivation_chain: vec![],
            block_tex: vec![
                (10, AuthoredFaces::solid([1, 1, 1, 255])),
                (10, AuthoredFaces::solid([2, 2, 2, 255])), // dup key — should win
            ],
            mob_tex: vec![],
        };
        let mut blob = vec![1u8];
        blob.extend(bincode::serialize(&v1).unwrap());
        let back = OverrideSet::from_blob_bytes(&blob).unwrap();
        assert_eq!(back.block_designs.len(), 1, "duplicate key collapsed to one library");
        let (_, lib) = &back.block_designs[0];
        assert_eq!(lib.designs.len(), 1);
        assert_eq!(
            lib.designs[0].faces.as_ref().unwrap(),
            &AuthoredFaces::solid([2, 2, 2, 255]),
            "last v1 entry wins (matches rebuild_layers last-wins)"
        );
        // And the runtime registry resolves a layer for it (no find-first/render-last split).
        let reg = OverrideRegistry::from_set(back, 100);
        assert!(reg.block_face_layer(10, 0).is_some());
    }

    // ---- Phase 5: active-design rendering + library writes ----

    #[test]
    fn rebuild_layers_uses_active_design_only() {
        let mut set = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("a", AuthoredFaces::solid([10, 10, 10, 255])));
        let active = lib.push_design(paint_design("b", AuthoredFaces::solid([20, 20, 20, 255])));
        let _ = active;
        set.block_designs.push((10, lib));
        let reg = OverrideRegistry::from_set(set, 100);
        // "b" is active (last pushed). Its face layer resolves.
        assert!(reg.block_face_layer(10, 0).is_some());
        // Only ONE active design contributes (solid ⇒ one layer), even with two designs.
        assert_eq!(reg.appended_len(), 1, "only the active design costs a layer");
    }

    #[test]
    fn use_original_reverts_to_stock() {
        let mut set = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("a", AuthoredFaces::solid([10, 10, 10, 255])));
        lib.use_original();
        set.block_designs.push((10, lib));
        let reg = OverrideRegistry::from_set(set, 100);
        assert!(reg.block_face_layer(10, 0).is_none(), "active=None → stock");
    }

    #[test]
    fn add_block_design_caps_at_16_keeping_active() {
        let mut reg = OverrideRegistry::new();
        let mut last_active = None;
        for i in 0..20u32 {
            let out = reg.add_block_design(
                10,
                paint_design(
                    &format!("d{i}"),
                    AuthoredFaces::solid([i as u8, 0, 0, 255]),
                ),
                100,
            );
            last_active = Some(out.active_id);
        }
        let lib = &reg.set().block_designs.iter().find(|(b, _)| *b == 10).unwrap().1;
        assert!(lib.designs.len() <= DesignLibrary::MAX_DESIGNS, "bounded at 16");
        assert_eq!(lib.active, last_active, "newest stays active");
        assert!(
            lib.designs.iter().any(|d| Some(d.id) == lib.active),
            "active still present"
        );
    }

    #[test]
    fn set_block_active_and_delete_round_trip() {
        let mut reg = OverrideRegistry::new();
        let a = reg.add_block_design(7, paint_design("a", AuthoredFaces::solid([1, 1, 1, 255])), BASE);
        let b = reg.add_block_design(7, paint_design("b", AuthoredFaces::solid([2, 2, 2, 255])), BASE);
        // b is active; switch back to a.
        assert!(reg.set_block_active(7, a.active_id, BASE));
        let lib = &reg.set().block_designs.iter().find(|(blk, _)| *blk == 7).unwrap().1;
        assert_eq!(lib.active, Some(a.active_id));
        // Rename a.
        assert!(reg.rename_block_design(7, a.active_id, "renamed".into()));
        // Delete the active design → reverts to stock.
        assert!(reg.delete_block_design(7, a.active_id, BASE));
        assert!(reg.block_face_layer(7, 0).is_none(), "deleting active reverts to stock");
        // b still present (inactive); switch to it.
        assert!(reg.set_block_active(7, b.active_id, BASE));
        assert!(reg.block_face_layer(7, 0).is_some());
    }

    #[test]
    fn active_micro_models_reports_active_shape_only() {
        let mut reg = OverrideRegistry::new();
        // Paint design — no micro-model.
        reg.add_block_design(5, paint_design("paint", AuthoredFaces::solid([1, 2, 3, 255])), BASE);
        assert!(reg.active_micro_models().is_empty(), "paint design has no shape");
        // Shape design — carries a micro-model.
        let model = crate::micro_model::MicroModelData {
            version: 1,
            scale: crate::micro_model::MICRO_SCALE_8,
            voxels: vec![],
            author_npub: String::new(),
            derivation_chain: vec![],
        };
        let shape = NamedDesign {
            id: 0,
            name: "shape".into(),
            faces: None,
            micro_model: Some(model),
            author_npub: String::new(),
            derivation_chain: vec![],
        };
        reg.add_block_design(6, shape, BASE);
        let shapes = reg.active_micro_models();
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].0, 6);
    }

    // ---- Phase 5: adoption slots inactive ----

    #[test]
    fn adopt_slots_designs_inactive() {
        // Mine: block 10 has an active design.
        let mut mine = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("mine", AuthoredFaces::solid([1, 1, 1, 255])));
        let mine_active = lib.active;
        mine.block_designs.push((10, lib));
        // Theirs: block 10 too.
        let mut theirs = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            author_npub: "npub1them".into(),
            ..Default::default()
        };
        let mut t10 = DesignLibrary::default();
        t10.push_design(NamedDesign {
            id: 0,
            name: "theirs10".into(),
            faces: Some(AuthoredFaces::solid([2, 2, 2, 255])),
            micro_model: None,
            author_npub: "npub1them".into(),
            derivation_chain: vec![],
        });
        theirs.block_designs.push((10, t10));
        mine.merge_adopted(&theirs);
        let lib10 = &mine.block_designs.iter().find(|(b, _)| *b == 10).unwrap().1;
        assert_eq!(lib10.active, mine_active, "adopt must NOT change my active design");
        assert!(lib10.designs.len() >= 2, "their design slotted in alongside mine");
        assert_eq!(mine.derivation_chain.len(), 1, "one adoption link recorded");
    }

    #[test]
    fn adopt_into_a_new_key_slots_inactive() {
        // Mine empty; theirs has block 11. Adopting a brand-new key must leave my
        // active = None on that key (I haven't chosen to wear it).
        let mut mine = OverrideSet::default();
        let mut theirs = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            author_npub: "npub1them".into(),
            ..Default::default()
        };
        let mut t11 = DesignLibrary::default();
        t11.push_design(paint_design("theirs11", AuthoredFaces::solid([3, 3, 3, 255])));
        theirs.block_designs.push((11, t11));
        mine.merge_adopted(&theirs);
        let lib11 = &mine.block_designs.iter().find(|(b, _)| *b == 11).unwrap().1;
        assert_eq!(lib11.active, None, "adopted-into-new-key is NOT auto-worn");
        assert_eq!(lib11.designs.len(), 1, "their design slotted in");
        assert_eq!(mine.version, OVERRIDE_SET_VERSION, "version stamped");
    }

    #[test]
    fn adopting_many_inactive_designs_does_not_exceed_layer_cap() {
        // Theirs: one block with 30 designs (all inactive once adopted). Mine wears
        // nothing. The layer cap must hold because only the ACTIVE design (none) costs
        // layers — adopting 30 inactive designs adds 0 layers.
        let base = 100u32;
        let mut theirs = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            author_npub: "npub1them".into(),
            ..Default::default()
        };
        let mut tlib = DesignLibrary::default();
        for i in 0..30u32 {
            // bypass the cap to forge a fat library on their side
            tlib.designs.push(NamedDesign {
                id: i,
                name: format!("t{i}"),
                faces: Some(distinct_faces(i as u8)),
                micro_model: None,
                author_npub: "npub1them".into(),
                derivation_chain: vec![],
            });
        }
        tlib.active = None; // theirs aren't worn either
        tlib.next_id = 30;
        theirs.block_designs.push((10, tlib));
        let bytes = theirs.to_blob_bytes().unwrap();
        let mut reg = OverrideRegistry::new();
        // A tight cap (base + 0 room): adoption must still succeed because inactive
        // designs add zero layers.
        apply_adopted_override_bytes(&mut reg, &bytes, base, base).unwrap();
        assert_eq!(
            reg.appended_len(),
            0,
            "no active design ⇒ no appended layers despite 30 adopted designs"
        );
        let lib = &reg.set().block_designs.iter().find(|(b, _)| *b == 10).unwrap().1;
        // Adopt is bounded at MAX_DESIGNS (oldest non-active evicted), so a hostile
        // fat library can't grow mine without limit — even though none cost layers.
        assert_eq!(
            lib.designs.len(),
            DesignLibrary::MAX_DESIGNS,
            "adopt bounded at the per-library cap, not 30"
        );
        assert_eq!(lib.active, None, "still wearing nothing");
    }

    // ---- serde + hashing ----

    #[test]
    fn serde_round_trips_and_preserves_overrides() {
        let mut reg = OverrideRegistry::new();
        reg.set.author_npub = "npub1example".to_string();
        reg.set.version = OVERRIDE_SET_VERSION;
        reg.add_block_design(5, paint_design("b", distinct_faces(7)), BASE);
        reg.add_mob_design(
            MobPartKey {
                mob: MobType::Sheep,
                part: 0,
            },
            paint_design("m", distinct_faces(40)),
            BASE,
        );

        let bytes = bincode::serialize(reg.set()).expect("serialize");
        let decoded: OverrideSet = bincode::deserialize(&bytes).expect("deserialize");
        let reg2 = OverrideRegistry::from_set(decoded, BASE);

        assert_eq!(reg2.set().author_npub, "npub1example");
        for fi in 0..6u8 {
            assert_eq!(
                reg.block_face_layer(5, fi),
                reg2.block_face_layer(5, fi),
                "block layer map survives round-trip"
            );
        }
        assert_eq!(
            reg.mob_part_faces(MobType::Sheep, 0, &[0; 6]),
            reg2.mob_part_faces(MobType::Sheep, 0, &[0; 6]),
            "mob layer map survives round-trip"
        );
    }

    #[test]
    fn content_hash_ignores_provenance_but_tracks_appearance() {
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("a", distinct_faces(3)));
        let mut a = OverrideSet::default();
        a.block_designs.push((5, lib));
        let mut b = a.clone();
        b.author_npub = "npub1different".to_string();
        b.derivation_chain.push(OverrideDerivation {
            author_npub: "npub1ancestor".to_string(),
            content_hash: [9; 32],
        });
        assert_eq!(
            content_hash(&a),
            content_hash(&b),
            "set-level provenance must not change the content hash"
        );

        let mut c = a.clone();
        c.block_designs[0].1.designs[0].faces = Some(distinct_faces(99)); // different appearance
        assert_ne!(
            content_hash(&a),
            content_hash(&c),
            "a different reskin must change the content hash"
        );
    }

    #[test]
    fn override_set_blob_round_trips_with_provenance() {
        let mut set = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        set.author_npub = "abc123".to_string();
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("orig", AuthoredFaces::solid([1, 2, 3, 255])));
        set.block_designs.push((crate::block::CORNFLOWER, lib));
        set.derivation_chain.push(OverrideDerivation {
            author_npub: "anc".to_string(),
            content_hash: [7; 32],
        });
        let bytes = set.to_blob_bytes().unwrap();
        let back = OverrideSet::from_blob_bytes(&bytes).unwrap();
        assert_eq!(back.author_npub, "abc123");
        assert_eq!(back.block_designs.len(), 1);
        assert_eq!(back.derivation_chain.len(), 1);
        assert_eq!(
            content_hash(&back),
            content_hash(&set),
            "appearance hash stable across blob round-trip"
        );
    }

    #[test]
    fn from_blob_bytes_rejects_garbage() {
        // empty + truncated-after-version + non-bincode all error, never panic.
        assert!(OverrideSet::from_blob_bytes(&[]).is_err());
        assert!(OverrideSet::from_blob_bytes(&[0xff, 0x00, 0x13, 0x37]).is_err());
    }

    #[test]
    fn from_blob_bytes_rejects_a_newer_version() {
        // Forge a blob whose version byte is one past what we support → must reject loudly.
        let set = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let mut bytes = set.to_blob_bytes().unwrap();
        bytes[0] = OVERRIDE_SET_VERSION + 1;
        let err = OverrideSet::from_blob_bytes(&bytes).unwrap_err();
        assert!(
            err.to_lowercase().contains("newer"),
            "error should explain the version is too new: {err}"
        );
    }

    #[test]
    fn blob_first_byte_is_the_version_tag() {
        let set = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let bytes = set.to_blob_bytes().unwrap();
        assert_eq!(bytes[0], OVERRIDE_SET_VERSION, "blob is version-prefixed");
    }

    /// Regenerates assets/official_overrides.fixture.json (run: cargo test --bin
    /// axenstax-engine generate_official_fixture -- --ignored --nocapture). One
    /// CORNFLOWER reskin, bytes = version-prefixed bincode as a u8 JSON array.
    #[test]
    #[ignore]
    fn generate_official_fixture() {
        let mut set = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        set.author_npub = String::new(); // official content has no personal author
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design(
            "Official Cornflower",
            AuthoredFaces::solid([255, 140, 0, 255]),
        )); // a warm "official" orange
        set.block_designs.push((crate::block::CORNFLOWER, lib));
        let bytes = set.to_blob_bytes().unwrap();
        let nums = bytes
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            "{{\"version\":1,\"items\":[{{\"name\":\"Official Cornflower\",\"contentType\":\"override-set\",\"bytes\":[{nums}]}}]}}"
        );
        std::fs::write(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/official_overrides.fixture.json"
            ),
            json,
        )
        .unwrap();
    }

    #[test]
    fn merge_adopted_slots_inactive_and_records_provenance() {
        let mut mine = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            author_npub: "me".into(),
            ..Default::default()
        };
        let mut stone = DesignLibrary::default();
        stone.push_design(paint_design("mine-stone", AuthoredFaces::solid([1, 1, 1, 255])));
        mine.block_designs.push((crate::block::STONE, stone));
        let mut cf = DesignLibrary::default();
        cf.push_design(paint_design("mine-cf", AuthoredFaces::solid([2, 2, 2, 255])));
        let mine_cf_active = cf.active;
        mine.block_designs.push((crate::block::CORNFLOWER, cf));

        let mut theirs = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            author_npub: "them".into(),
            ..Default::default()
        };
        let mut tcf = DesignLibrary::default();
        tcf.push_design(paint_design("theirs-cf", AuthoredFaces::solid([9, 9, 9, 255])));
        theirs.block_designs.push((crate::block::CORNFLOWER, tcf));
        let mut tcow = DesignLibrary::default();
        tcow.push_design(paint_design("theirs-cow", AuthoredFaces::solid([3, 3, 3, 255])));
        theirs.mob_designs.push((
            MobPartKey {
                mob: crate::mob::MobType::Cow,
                part: 0,
            },
            tcow,
        ));
        let their_hash = content_hash(&theirs);

        mine.merge_adopted(&theirs);

        // STONE untouched (no clash), CORNFLOWER gains their design INACTIVE (mine stays
        // active), COW part added.
        let stone_lib = &mine
            .block_designs
            .iter()
            .find(|(b, _)| *b == crate::block::STONE)
            .unwrap()
            .1;
        assert_eq!(stone_lib.designs.len(), 1, "stone untouched");
        let cf_lib = &mine
            .block_designs
            .iter()
            .find(|(b, _)| *b == crate::block::CORNFLOWER)
            .unwrap()
            .1;
        assert_eq!(cf_lib.designs.len(), 2, "their cf design slotted in");
        assert_eq!(cf_lib.active, mine_cf_active, "my cf active unchanged by adopt");
        assert_eq!(mine.mob_designs.len(), 1, "their cow part adopted");
        // Exactly one derivation link to the adopted set.
        assert_eq!(mine.derivation_chain.len(), 1);
        assert_eq!(mine.derivation_chain[0].author_npub, "them");
        assert_eq!(mine.derivation_chain[0].content_hash, their_hash);
    }

    #[test]
    fn merge_adopted_into_empty_takes_all_inactive() {
        let mut mine = OverrideSet::default();
        let mut theirs = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            author_npub: "them".into(),
            ..Default::default()
        };
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("t", AuthoredFaces::solid([4, 5, 6, 255])));
        theirs.block_designs.push((crate::block::CORNFLOWER, lib));
        mine.merge_adopted(&theirs);
        assert_eq!(mine.block_designs.len(), 1);
        let cf = &mine.block_designs[0].1;
        assert_eq!(cf.designs.len(), 1);
        assert_eq!(cf.active, None, "adopted-into-empty is not auto-worn");
        assert_eq!(mine.derivation_chain.len(), 1);
        assert_eq!(
            mine.version, OVERRIDE_SET_VERSION,
            "version stamped when merging into a default set"
        );
    }

    #[test]
    fn adopting_then_setting_active_textures_the_asset() {
        let base = crate::texture_gen::texture_count();
        let mut theirs = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("t", AuthoredFaces::solid([4, 5, 6, 255])));
        theirs.block_designs.push((crate::block::CORNFLOWER, lib));
        let mut mine = OverrideSet::default();
        mine.merge_adopted(&theirs);
        let mut reg = OverrideRegistry::from_set(mine, base);
        // Adopted inactive ⇒ stock until set-active.
        assert!(
            reg.block_face_layer(crate::block::CORNFLOWER, 0).is_none(),
            "adopted design is inactive until worn"
        );
        // Wear the adopted design.
        let id = reg
            .set()
            .block_designs
            .iter()
            .find(|(b, _)| *b == crate::block::CORNFLOWER)
            .unwrap()
            .1
            .designs[0]
            .id;
        assert!(reg.set_block_active(crate::block::CORNFLOWER, id, base));
        assert!(
            reg.block_face_layer(crate::block::CORNFLOWER, 0).is_some(),
            "adopted reskin is live once worn"
        );
    }

    #[test]
    fn apply_adopted_bytes_textures_the_asset_when_worn() {
        let base = crate::texture_gen::texture_count();
        let mut theirs = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("t", AuthoredFaces::solid([4, 5, 6, 255])));
        theirs.block_designs.push((crate::block::CORNFLOWER, lib));
        let bytes = theirs.to_blob_bytes().unwrap();
        let mut reg = OverrideRegistry::new();
        apply_adopted_override_bytes(&mut reg, &bytes, base, base + 1000).unwrap();
        // Adopted inactive ⇒ no layers yet.
        assert_eq!(reg.appended_len(), 0, "adopted inactive ⇒ no layers");
        let id = reg
            .set()
            .block_designs
            .iter()
            .find(|(b, _)| *b == crate::block::CORNFLOWER)
            .unwrap()
            .1
            .designs[0]
            .id;
        reg.set_block_active(crate::block::CORNFLOWER, id, base);
        assert!(
            reg.block_face_layer(crate::block::CORNFLOWER, 0).is_some(),
            "adopted reskin is live once worn"
        );
    }

    #[test]
    fn apply_adopted_bytes_rejects_garbage_and_newer_version() {
        let base = crate::texture_gen::texture_count();
        let mut reg = OverrideRegistry::new();
        assert!(apply_adopted_override_bytes(&mut reg, &[0xff, 0, 1, 2], base, base + 1000).is_err());
        let mut s = OverrideSet {
            version: OVERRIDE_SET_VERSION,
            ..Default::default()
        };
        let mut lib = DesignLibrary::default();
        lib.push_design(paint_design("t", AuthoredFaces::solid([1, 1, 1, 255])));
        s.block_designs.push((crate::block::STONE, lib));
        let mut bytes = s.to_blob_bytes().unwrap();
        bytes[0] = OVERRIDE_SET_VERSION + 1; // forge a newer version
        assert!(apply_adopted_override_bytes(&mut reg, &bytes, base, base + 1000).is_err());
        assert!(reg.is_empty(), "nothing applied on any rejection");
    }
}
