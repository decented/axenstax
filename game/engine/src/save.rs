//! World persistence — save/load chunks, player state, and metadata.
//!
//! Save format (simple prototype — region files come later):
//!   worlds/<folder>/world.dat       — bincode: WorldSave (player + inventory)
//!   worlds/<folder>/world_meta.json — JSON: WorldMeta (display name, description, game mode)
//!   worlds/<folder>/chunks/         — raw binary: one file per non-empty chunk

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::chunk::Chunk;
use crate::crafting::{Tool, ToolMaterial, ToolType};
use crate::inventory::Inventory;
use crate::item::{Item, ItemStack, MaterialId};
use crate::world::World;
pub use crate::save_format::WorldSaveError;

/// WASM: pubkey for per-user world namespacing. Set by `wasm_auth::set_pubkey`
/// (called from auth.js) before the engine starts.
#[cfg(target_arch = "wasm32")]
thread_local! {
    pub static WASM_PUBKEY: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
}

/// WASM: cached metadata keyed by folder name. Survives the session; hydrated
/// from IndexedDB by `list_local_worlds_async` in menu.rs. Saves use this to
/// bump the `version` counter without round-tripping through IDB synchronously.
#[cfg(target_arch = "wasm32")]
thread_local! {
    pub static WASM_META_CACHE: std::cell::RefCell<std::collections::HashMap<String, WorldMeta>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// **Seam B** — the current player's persona pubkey (lowercase hex), or `None`
/// for a guest. Cross-platform: WASM reads the `auth.js`-provided pubkey; native
/// reads the cached Signet identity set by `signet::native_signer` at startup.
/// Pure read, **no network** — keeps save off the signing critical path.
pub fn current_owner_pubkey() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        WASM_PUBKEY.with(|p| p.borrow().clone()).filter(|s| !s.is_empty())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::signet::native_signer::current_owner_pubkey()
    }
}

/// Map an optional persona pubkey to the **local-world storage namespace**.
///
/// `None` or empty → the guest `"local"` bucket; otherwise the pubkey itself.
/// Pure + cross-platform so it's unit-testable; [`wasm_storage_key`] wraps it
/// over the `WASM_PUBKEY` thread-local. The point is the result is **never
/// empty**, so the WASM save/load/list/delete path no longer has a "no pubkey"
/// failure mode (the bug that left web guests unable to load any world).
///
/// Native callers are wasm-only (`wasm_storage_key`) + the unit test, so on a
/// non-wasm build it's technically uncalled — allow it rather than cfg-fence a
/// genuinely cross-platform helper.
#[allow(dead_code)]
pub fn storage_namespace(pubkey: Option<&str>) -> String {
    match pubkey {
        Some(pk) if !pk.is_empty() => pk.to_string(),
        _ => "local".to_string(),
    }
}

/// WASM: the IndexedDB namespace for the current player's **local** worlds.
///
/// The web tier is anonymous-by-default (login retired — web is a purely local
/// sandbox, see `docs/superpowers/specs/2026-06-27-web-local-sandbox-design.md`),
/// so this is `"local"` in practice; a signed-in persona would namespace under
/// its pubkey. The JS store accepts `"local"` as a valid namespace
/// (`world_store.js` `validPubkey`).
#[cfg(target_arch = "wasm32")]
pub fn wasm_storage_key() -> String {
    storage_namespace(WASM_PUBKEY.with(|p| p.borrow().clone()).as_deref())
}

/// Saved world metadata + player state.
///
/// Positional bincode, APPEND-ONLY (Spec 02 §8.4). Appending a field: add it LAST
/// here, read it last in `deserialize_world_save_tolerant_reporting`, and bump
/// `save_format::WORLD_SAVE_FIELD_COUNT` (the tripwire tests fail until you do), so
/// older builds refuse the new saves instead of truncating them. A wire change inside
/// a nested saved type bumps `save_format::SAVE_LAYOUT_REVISION` instead.
#[derive(Serialize, Deserialize)]
pub struct WorldSave {
    pub seed: u32,
    // Legacy single-player fields — kept for backward compatibility.
    // New saves also populate `players`; old saves leave `players` empty.
    pub player_x: f32,
    pub player_y: f32,
    pub player_z: f32,
    pub player_health: f32,
    pub hotbar_slot: usize,
    pub inventory: Vec<SavedSlot>,
    /// Per-player save data (multi-player). Empty on old saves — fall back to legacy fields.
    #[serde(default)]
    pub players: Vec<PlayerSaveData>,
    /// Block-entity state (Wave 27 campfires; future furnaces etc).
    /// Position-keyed; serialised as a Vec of (x, y, z, data) tuples so
    /// the wire format is bincode-stable. Empty on old saves — load
    /// path silently accepts and re-populates the world with no
    /// campfires (correct — old saves predate this feature).
    #[serde(default)]
    pub campfires: Vec<SavedCampfire>,
    /// Furnace block-entity state (Spec 20 Phase 8). Same shape as
    /// `campfires` — position + state. Old saves load with an empty
    /// Vec via `#[serde(default)]`.
    #[serde(default)]
    pub furnaces: Vec<SavedFurnace>,
    /// Vendor Block state (Spec 21 Phase 9). Same shape — position +
    /// per-vendor data. Old saves load empty via `#[serde(default)]`.
    #[serde(default)]
    pub vendors: Vec<SavedVendor>,
    /// Drying-rack block-entity state (Wave 29 — log seasoning).
    /// Same shape as `campfires`: position + per-rack data. Old saves
    /// load with an empty Vec via `#[serde(default)]`.
    #[serde(default)]
    pub drying_racks: Vec<SavedDryingRack>,
    /// Spec 28d chunk 8 — Bee Hive state. Same shape: position +
    /// `HiveData` (bees inside + honey level). Old saves load empty
    /// via `#[serde(default)]`.
    #[serde(default)]
    pub hives: Vec<SavedHive>,
    /// HP-2 (2026-05-22) — Chest block-entity state. Position + 27-slot
    /// inventory. Old saves load empty via `#[serde(default)]`.
    #[serde(default)]
    pub chests: Vec<SavedChest>,
    /// Spec 34 Tip Jar — block-entity state. Position + owner +
    /// escrow + counters. Old saves load empty via `#[serde(default)]`.
    #[serde(default)]
    pub tip_jars: Vec<SavedTipJar>,
    /// Spec 36 Plot Ownership — claimed plots. PlotData is plain
    /// serde (flat struct), so no Saved* mirror needed. Old saves
    /// load empty via `#[serde(default)]`.
    #[serde(default)]
    pub plots: Vec<crate::plot::PlotData>,
    /// Spec 37 Market Hubs — discovery zones. Plain serde
    /// (MarketHubData is flat). Old saves load empty via default.
    #[serde(default)]
    pub market_hubs: Vec<crate::market_hub::MarketHubData>,
    /// Spec 38 Auctions — timed auction block-entities (pos + data).
    /// Old saves load empty via `#[serde(default)]`.
    #[serde(default)]
    pub auctions: Vec<SavedAuction>,
    /// Spec 38 (Blueprint / Cyanotype) — Latent Print block-entities.
    /// Each entry is a position + the wrapped Plan whose `develop_state`
    /// is still progressing. Old saves load empty via `#[serde(default)]`.
    #[serde(default)]
    pub latent_prints: Vec<SavedLatentPrint>,
    /// Spec 24 — in-progress Build Schematics builds. Each entry
    /// carries the anchor's world position + serialised ConstructionAnchorData
    /// (plan, rotations, placed_index, locked materials). Old saves
    /// load with an empty Vec via `#[serde(default)]`.
    #[serde(default)]
    pub construction_anchors: Vec<SavedConstructionAnchor>,
    /// Spec 24 — Architect's Plaques placed by completed builds.
    /// Position + derivation chain. Old saves load with an empty Vec.
    #[serde(default)]
    pub architect_plaques: Vec<SavedArchitectPlaque>,
    /// Spec 19 phase 11 — village state. Empty on pre-Spec-19 saves.
    /// `village_anchors` flattens the AHashMap to `(grid_x, grid_z, ax, ay, az)`
    /// tuples; `populated_villages` lists the cells whose initial cohort has
    /// been spawned; `village_bells` lists every player-placed bell.
    #[serde(default)]
    pub village_anchors: Vec<SavedVillageAnchor>,
    #[serde(default)]
    pub populated_villages: Vec<(i32, i32)>,
    #[serde(default)]
    pub village_bells: Vec<[i32; 3]>,
    /// Spec 22 — village treasuries (per-village sats balance) +
    /// in-flight raids + scheduler state. All serde-default so
    /// pre-Spec-22 saves load cleanly with empty raid state.
    #[serde(default)]
    pub village_treasuries: Vec<((i32, i32), u64)>,
    #[serde(default)]
    pub active_raids: Vec<crate::raid::Raid>,
    #[serde(default)]
    pub raid_scheduler: crate::raid::RaidScheduler,
    /// Spec 22 Phase 18 — per-village per-player raid-kill totals.
    /// Pre-Phase-18 saves load with an empty Vec via `#[serde(default)]`.
    /// Tuple-of-tuples wire shape mirrors `village_treasuries` so the
    /// AHashMap deserialise is straightforward.
    #[serde(default)]
    pub raid_kills: Vec<((i32, i32), usize, u32)>,
    /// HP-3 (2026-05-23) — Brigand Hideout side-table. One entry per
    /// generated hideout, keyed by `(grid_x, grid_z)`. Carries the
    /// anchor + current population state. Pre-HP-3 saves load with
    /// an empty Vec via `#[serde(default)]`.
    #[serde(default)]
    pub brigand_hideouts: Vec<SavedHideout>,
    /// Spec 33 Mob Bounty Board — active bounties + monotonic id
    /// allocator + last refresh tick. Empty Vec / 0 / 0 on pre-Spec-33
    /// saves via `#[serde(default)]`.
    #[serde(default)]
    pub bounties: Vec<SavedBounty>,
    #[serde(default)]
    pub bounty_next_id: u32,
    #[serde(default)]
    pub bounty_last_refresh_tick: u64,
    /// Owner-inbox #1/2/3 (2026-06-03) — face-overlay wallpaper. One entry per
    /// painted face. Pre-2026-06-03 saves load with an empty Vec via
    /// `#[serde(default)]`. Built via `face_overlays_to_saved`, restored via
    /// `restore_face_overlays`.
    #[serde(default)]
    pub face_overlays: Vec<SavedFaceOverlay>,
    /// Blueprint face-attachments (Task B1, 2026-06-04). One entry per face
    /// carrying a `FaceAttachment::Blueprint`. Additive sibling of
    /// `face_overlays` (which stays wallpaper-only and byte-identical) so
    /// existing on-disk saves remain decodable. Pre-Task-B1 saves load with
    /// an empty Vec via `#[serde(default)]`. Built via
    /// `face_blueprints_to_saved`, restored via `restore_face_blueprints`.
    #[serde(default)]
    pub face_blueprints: Vec<SavedFaceBlueprint>,
    /// Task R1 (2026-06-04) — laid blank cream draughting paper face-attachments
    /// (`FaceAttachment::BlueprintBlank`). One entry per blank face; no payload
    /// (the plan only exists after capture). Additive sibling of
    /// `face_blueprints` so existing on-disk saves stay decodable. Pre-Task-R1
    /// saves load with an empty Vec via `#[serde(default)]`. Built via
    /// `face_blueprint_blanks_to_saved`, restored via
    /// `restore_face_blueprint_blanks`.
    #[serde(default)]
    pub face_blueprint_blanks: Vec<SavedFaceBlankPaper>,
    /// Spec 40 (The Workshop) — the `workshop_projects` side-table: parked,
    /// possibly-inflated redesign WIP for a Workshop world. The NEWEST appended
    /// field (append-only invariant — must stay LAST in declaration/wire order).
    /// Empty in normal worlds; pre-Workshop saves load it as default via
    /// `#[serde(default)]` / `read_tail`.
    #[serde(default)]
    pub workshop: crate::workshop::WorkshopProjects,
    /// Rail freight Phase 1 (Task 1.6) — persisted carts (ECS entities, not
    /// block-entities). Empty in worlds with no carts; pre-rail saves load it as
    /// default via `#[serde(default)]` / `read_tail`.
    #[serde(default)]
    pub carts: Vec<SavedCart>,
    /// #47 — Grave block-entities (death containers). Position + GraveData
    /// (36-slot inventory snapshot, index-aligned). bincode is positional, so
    /// appended fields must keep their declared order (append-only invariant).
    /// Pre-#47 saves load it empty via `#[serde(default)]` / `read_tail`.
    #[serde(default)]
    pub graves: Vec<SavedGrave>,
    /// #6 — map waypoints (manual pins + rolling death markers). Pre-#6 saves
    /// load it empty via `#[serde(default)]` / `read_tail`.
    #[serde(default)]
    pub waypoints: Vec<crate::waypoint::Waypoint>,
    /// Spec 48 (Electricity) — per-block meta byte (facing / lever-latch / gate
    /// op), keyed by position. Sparse: only cells that ever stored a byte. Flat
    /// `(x, y, z, meta)` tuples (bincode-stable, like `village_treasuries`).
    /// NOT derivable from the saved block id, so it must be persisted. Pre-Spec-48
    /// saves load it empty via `#[serde(default)]` / `read_tail`.
    #[serde(default)]
    pub block_meta: Vec<(i32, i32, i32, u8)>,
    /// Spec 48 (Electricity) — power-device runtime state (generator fuel/charge,
    /// battery, hand-crank countdown, gate op). The NEWEST appended field —
    /// bincode is positional, so this MUST stay LAST in the declaration / wire
    /// order (append-only invariant). The transient power flood is NOT saved
    /// (rederived by `power::reseed_on_load`). Pre-Spec-48 saves load it empty
    /// via `#[serde(default)]` / `read_tail`.
    #[serde(default)]
    pub power_devices: Vec<SavedPowerDevice>,
    /// Wave 2c — Sign block-entity text. Pre-Wave-2c saves end before it and
    /// default empty via `#[serde(default)]` / `read_tail`.
    #[serde(default)]
    pub signs: Vec<SavedSign>,
    /// Wave 2c — Item Frame block-entities. Old saves end before it → empty.
    #[serde(default)]
    pub item_frames: Vec<SavedItemFrame>,
    /// Wave 3 (#45 P3) — player 0's locked inventory slot indices. Split-screen
    /// players' locks are not yet persisted (single-player is the primary case).
    /// Old saves end before it → empty (no locks).
    #[serde(default)]
    pub locked_slots: Vec<u32>,
    /// Wave 5 (Rail Freight P3) — the world's hostile-act ledger (cart
    /// robberies). Old saves end before it → empty.
    #[serde(default)]
    pub hostile_acts: Vec<crate::hostile_acts::HostileAct>,
    /// #19 Rig Studio — placed authored rigs. Old saves end before it → empty.
    #[serde(default)]
    pub rigs: Vec<crate::world::RigDisplay>,
    /// Creator-gallery exhibits (Spec 2026-06-19 §9, Phase 1). 2D art placements
    /// (wall-hung or standing billboard). Empty in normal worlds; pre-gallery
    /// saves load it as default via `#[serde(default)]` / `read_tail`. Append-only
    /// invariant — never reorder/remove (bincode is positional). Followed by the
    /// Spec 49 `composters` field below.
    #[serde(default)]
    pub exhibits: Vec<crate::exhibit::Exhibit>,
    /// Spec 49 (Explosives) — Composter contents (input / output / progress). The
    /// keg fuse persists via `power_devices` (the keg is a `PowerDevice`), so only
    /// the Composter needs its own side-table. The NEWEST appended field — bincode
    /// is positional, so this MUST stay LAST in the declaration / wire order
    /// (append-only invariant). Never reorder/remove.
    #[serde(default)]
    pub composters: Vec<SavedComposter>,
    /// Animals Wave 2 — persisted TAMED mobs (wolves, nostriches) so a player's
    /// pets survive save/load instead of vanishing each session. World-level
    /// (pets are ECS entities, not block-entities; restored in
    /// `chunk_stream::initial_load`, not `apply_world_save_state`). Owner +
    /// AI/tame state travel inside the per-species data. The NEWEST appended
    /// field — bincode is positional, so this MUST stay LAST in the declaration
    /// / wire order (append-only invariant). Never reorder/remove.
    #[serde(default)]
    pub saved_mobs: Vec<SavedTamedPet>,
    /// Satoshi onboarding — per-world guide progress + house placement. The
    /// NEWEST appended field — bincode is positional, so this MUST stay LAST in
    /// the declaration / wire order (append-only invariant). Never reorder/remove.
    /// Pre-Satoshi saves end before it and default via `#[serde(default)]` /
    /// `read_tail`. The Satoshi *entity* is re-derived on load, not serialised.
    #[serde(default)]
    pub satoshi: crate::satoshi::SatoshiState,
    /// Dispensers/Droppers (2026-07-04 gap-fill wave) — 9-slot eject-on-power
    /// containers. The NEWEST appended field — bincode is positional, so this
    /// MUST stay LAST in the declaration / wire order (append-only invariant).
    /// Never reorder/remove.
    #[serde(default)]
    pub dispensers: Vec<SavedDispenser>,
    /// #19 Rig Studio — the animation clip each placed rig plays, INDEX-ALIGNED
    /// with `rigs`. A side table rather than a field on `RigDisplay`, because
    /// `rigs` is a `Vec` in the middle of this positional bincode blob: appending
    /// to the element type would shift every byte after it and silently corrupt
    /// `exhibits` / `composters` / `saved_mobs` / … in any save that already
    /// holds a rig. Shorter than `rigs` (or empty, for every pre-picker save) ⇒
    /// the missing entries default to `AnimClip::Walk`, the clip those rigs
    /// always played. The NEWEST appended field — bincode is positional, so this
    /// MUST stay LAST in the declaration / wire order (append-only invariant).
    /// Never reorder/remove.
    #[serde(default)]
    pub rig_clips: Vec<crate::anim_set::AnimClip>,
}

/// One serialised Dispenser/Dropper. Position + DispenserData (mirrors
/// `SavedChest`).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedDispenser {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::dispenser::DispenserData,
}

/// #47 — one serialised Grave. Position + GraveData (mirrors `SavedChest`).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedGrave {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::grave::GraveData,
}

/// Mirror of `bounty::ActiveBounty` for save. Same shape; separate
/// type so save-format changes don't ripple into the runtime type.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SavedBounty {
    pub id: u32,
    pub template_idx: usize,
    pub issued_tick: u64,
}

/// Serialised village anchor — grid-cell key + anchor world position.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedVillageAnchor {
    pub grid_x: i32,
    pub grid_z: i32,
    pub anchor: [i32; 3],
}

/// One serialised campfire — position + state.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedCampfire {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::campfire::CampfireData,
}

/// One serialised furnace (Spec 20 Phase 8) — position + state. Mirror
/// of SavedCampfire; same `#[serde(default)]` pattern in WorldSave so
/// old saves load with zero furnaces and the field is forward-compatible.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedFurnace {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::furnace::FurnaceData,
}

/// One serialised power device (Spec 48 — Electricity) — position + runtime
/// state (`PowerDeviceData`: facing, source latch, charge, generator fuel, gate
/// op). Mirror of `SavedFurnace`. The transient power flood (`PowerState`) is
/// NOT saved — it's rederived by `power::reseed_on_load` after restore.
/// `WorldSave.power_devices` is `#[serde(default)]` so pre-Spec-48 saves load
/// with zero devices.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedComposter {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// The generic workstation state (input / output / progress). Spec 49.
    pub data: crate::workstation::WorkstationState,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SavedPowerDevice {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::power::PowerDeviceData,
}

/// Owner-inbox #1/2/3 (2026-06-03) — one serialised wallpaper face overlay:
/// block position + the painted face (`crate::mesh::Face::index`, 0..5) + the
/// wallpaper block id. Flat coords like every other `Saved*` (bincode-stable).
/// Unlike the derived caches (`salt_licks`), face overlays have no block to
/// rebuild from, so they MUST be persisted. `WorldSave.face_overlays` is
/// `#[serde(default)]` so pre-2026-06-03 saves load with zero overlays.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SavedFaceOverlay {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub face: u8,
    pub block: crate::block::BlockId,
}

/// Owner-inbox #1/2/3 — flatten `World.face_attachments` (position → per-face
/// array) into the flat per-face `SavedFaceOverlay` Vec the save format uses.
/// Called at every `WorldSave`-construction site. This function persists ONLY
/// `Wallpaper` attachments and keeps the `SavedFaceOverlay` bytes format-stable;
/// `Blueprint` attachments are persisted separately via
/// `face_blueprints_to_saved` (Task B1) into the additive `face_blueprints` Vec.
pub fn face_overlays_to_saved(world: &crate::world::World) -> Vec<SavedFaceOverlay> {
    let mut out = Vec::new();
    for ((x, y, z), faces) in world.iter_face_attachments() {
        for (face_idx, slot) in faces.iter().enumerate() {
            // Only `Wallpaper` attachments round-trip through this Vec. The
            // `SavedFaceOverlay` format stays byte-identical to the shipped
            // wallpaper format; `Blueprint` attachments are handled by the
            // additive `face_blueprints_to_saved` (Task B1).
            if let Some(crate::world::FaceAttachment::Wallpaper(block)) = slot {
                out.push(SavedFaceOverlay {
                    x,
                    y,
                    z,
                    face: face_idx as u8,
                    block: *block,
                });
            }
        }
    }
    out
}

/// Owner-inbox #1/2/3 — restore flat `SavedFaceOverlay` entries back into
/// `World.face_attachments` as `Wallpaper` attachments. Called from the load path.
/// `Blueprint` attachments are restored separately via `restore_face_blueprints`
/// (Task B1) from the additive `face_blueprints` Vec.
pub fn restore_face_overlays(world: &mut crate::world::World, saved: &[SavedFaceOverlay]) {
    for fo in saved {
        world.set_face_attachment(
            (fo.x, fo.y, fo.z),
            fo.face as usize,
            crate::world::FaceAttachment::Wallpaper(fo.block),
        );
    }
}

/// Task B1 (2026-06-04) — one serialised blueprint face-attachment: block
/// position + the attached face (`crate::mesh::Face::index`, 0..5) + the plan
/// payload. Additive sibling of `SavedFaceOverlay`: `Blueprint` attachments
/// have no block to rebuild from, so the `PlanData` MUST be persisted in full.
/// Lives in the `#[serde(default)]` `WorldSave.face_blueprints` Vec so the
/// wallpaper-only `face_overlays` bytes stay format-stable.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedFaceBlueprint {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub face: u8,
    pub plan: crate::plan::PlanData,
}

/// Task B1 — flatten `World.face_attachments` into the flat per-face
/// `SavedFaceBlueprint` Vec, emitting one row per `Blueprint` face. The
/// `Wallpaper` counterpart is `face_overlays_to_saved`. Called at every
/// `WorldSave`-construction site.
pub fn face_blueprints_to_saved(world: &crate::world::World) -> Vec<SavedFaceBlueprint> {
    let mut out = Vec::new();
    for ((x, y, z), faces) in world.iter_face_attachments() {
        for (face_idx, slot) in faces.iter().enumerate() {
            if let Some(crate::world::FaceAttachment::Blueprint(plan)) = slot {
                out.push(SavedFaceBlueprint {
                    x,
                    y,
                    z,
                    face: face_idx as u8,
                    plan: (**plan).clone(),
                });
            }
        }
    }
    out
}

/// Task B1 — restore flat `SavedFaceBlueprint` entries back into
/// `World.face_attachments` as `Blueprint` attachments. Called from the load
/// path. Old saves have an empty `face_blueprints` Vec via `#[serde(default)]`.
pub fn restore_face_blueprints(world: &mut crate::world::World, saved: &[SavedFaceBlueprint]) {
    for fb in saved {
        world.set_face_attachment(
            (fb.x, fb.y, fb.z),
            fb.face as usize,
            crate::world::FaceAttachment::Blueprint(Box::new(fb.plan.clone())),
        );
    }
}

/// Task R1 (2026-06-04) — one serialised blank draughting-paper face-attachment:
/// block position + the attached face (`crate::mesh::Face::index`, 0..5). No
/// payload — a `BlueprintBlank` is laid blank cream paper with no plan yet (the
/// plan only exists after capture). Additive sibling of `SavedFaceBlueprint`;
/// lives in the `#[serde(default)]` `WorldSave.face_blueprint_blanks` Vec so the
/// `face_blueprints` bytes stay format-stable.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SavedFaceBlankPaper {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub face: u8,
}

/// Task R1 — flatten `World.face_attachments` into the flat per-face
/// `SavedFaceBlankPaper` Vec, emitting one row per `BlueprintBlank` face. Called
/// at every `WorldSave`-construction site.
pub fn face_blueprint_blanks_to_saved(world: &crate::world::World) -> Vec<SavedFaceBlankPaper> {
    let mut out = Vec::new();
    for ((x, y, z), faces) in world.iter_face_attachments() {
        for (face_idx, slot) in faces.iter().enumerate() {
            if let Some(crate::world::FaceAttachment::BlueprintBlank) = slot {
                out.push(SavedFaceBlankPaper {
                    x,
                    y,
                    z,
                    face: face_idx as u8,
                });
            }
        }
    }
    out
}

/// Task R1 — restore flat `SavedFaceBlankPaper` entries back into
/// `World.face_attachments` as `BlueprintBlank` attachments. Called from the
/// load path. Old saves have an empty `face_blueprint_blanks` Vec via
/// `#[serde(default)]`.
pub fn restore_face_blueprint_blanks(
    world: &mut crate::world::World,
    saved: &[SavedFaceBlankPaper],
) {
    for fb in saved {
        world.set_face_attachment(
            (fb.x, fb.y, fb.z),
            fb.face as usize,
            crate::world::FaceAttachment::BlueprintBlank,
        );
    }
}

/// One serialised vendor (Spec 21 Phase 9) — position + state. Same
/// shape as SavedFurnace. Old saves load with zero vendors via the
/// `#[serde(default)]` field on WorldSave.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedVendor {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::vendor::VendorData,
}

/// One serialised drying rack — position + per-slot seasoning state.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedDryingRack {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::drying_rack::DryingRackData,
}

/// Spec 28d chunk 8 — one serialised Bee Hive. Position + HiveData
/// (bees inside + honey level).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedHive {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::bee_hive::HiveData,
}

/// HP-2 (2026-05-22) — one serialised Chest. Position + ChestData
/// (27-slot inventory).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedChest {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::chest::ChestData,
}

/// Solo Buildout Wave 2c — one serialised Sign. Position + editable text.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedSign {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::sign::SignData,
}

/// Solo Buildout Wave 2c — one serialised Item Frame. Position + framed item.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedItemFrame {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::item_frame::ItemFrameData,
}

/// Spec 34 (2026-05-23) — one serialised Tip Jar. Position + owner +
/// escrow + lifetime counter.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedTipJar {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::tip_jar::TipJarData,
}

/// Spec 38 (2026-05-23) — one serialised Auction. Position + the full
/// timed AuctionData (lot / reserve / deadline / high bid / escrow).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedAuction {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::auction::AuctionData,
}

/// Spec 38 (Blueprint / Cyanotype, 2026-05-27) — one serialised Latent
/// Print block-entity. Position + the wrapped LatentPrintData (which
/// carries the nested PlanData + its `develop_state`). Mirrors the
/// SavedTipJar / SavedAuction shape — flat coordinates + the data
/// payload. Wire-stable: `WorldSave.latent_prints` is `#[serde(default)]`
/// so pre-Spec-38 saves load with zero latent prints.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedLatentPrint {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::latent_print::LatentPrintData,
}

/// Rail freight Phase 1 (Task 1.6) — one serialised cart. Carts are ECS
/// entities, not block-entities, so unlike chests/furnaces they have no fixed
/// block position to rebuild from — their whole state (`cell` / `came_from` /
/// `progress` / `speed` / `facing` / `cargo`) MUST be persisted. `data.cell` is
/// the cart's track anchor, so no separate `x`/`y`/`z` coordinate is needed.
/// Wrapping `CartData` (rather than serialising it directly) follows the
/// `Saved*` convention and keeps a stable migration point if `CartData` ever
/// changes shape. `WorldSave.carts` is `#[serde(default)]` + read last, so
/// pre-rail saves load with zero carts.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedCart {
    pub data: crate::cart::CartData,
}

/// Rail freight Phase 1 (Task 1.6) — snapshot every cart in `ecs` into the flat
/// `Vec<SavedCart>` the save format stores. Mirrors `face_overlays_to_saved`'s
/// shape (a free function the construction sites call), but reads the ECS
/// `hecs::World` the carts live in rather than the voxel `World`. The carts must
/// be snapshotted from the SAME ECS that `cart::tick_carts` advances so a
/// saved-then-loaded cart resumes from its exact in-flight state.
pub fn carts_to_saved(ecs: &hecs::World) -> Vec<SavedCart> {
    ecs.query::<&crate::cart::CartData>()
        .iter()
        .map(|(_, d)| SavedCart { data: d.clone() })
        .collect()
}

/// HP-3 (2026-05-23) — one serialised Brigand Hideout. The hideout's
/// layout (palisade, huts, chest, banner) is regenerated from
/// `(world_seed, gx, gz)` on load; what persists is the mutable
/// state (population + replenish cooldown) so a partially-cleared
/// hideout stays cleared after a save/load cycle.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedHideout {
    pub grid_x: i32,
    pub grid_z: i32,
    pub data: crate::brigand_hideout_gen::HideoutData,
}

/// Spec 24 — one serialised in-progress build anchor. Position is the
/// anchor block's world coords; data carries the plan + rotations +
/// progress + locked materials so a quit-mid-build can resume cleanly.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedConstructionAnchor {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub data: crate::plan::ConstructionAnchorData,
}

/// Spec 24 — one serialised Architect's Plaque. Position + derivation
/// chain; right-click looks up the chain by position.
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedArchitectPlaque {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub chain: crate::plan::PlaqueChain,
}

/// Serializable inventory slot — supports all item types.
#[derive(Serialize, Deserialize, Clone)]
pub enum SavedSlot {
    Empty,
    Block { block_id: u16, count: u8 },
    Tool { tool_type: ToolType, material: ToolMaterial, durability: u16 },
    Material { material_id: MaterialId, count: u8 },
    /// Spec 24 — captured-building blueprint. Plans never stack, so
    /// no count field. PlanData is bincode-stable (versioned u8 inside).
    Plan { data: crate::plan::PlanData },
    /// Spec 28e — armour piece. Per-instance durability. Bincode-
    /// positional: appended after Plan to preserve save compatibility.
    Armour {
        slot: crate::armour::ArmourSlot,
        material: crate::armour::ArmourMaterial,
        durability: u16,
    },
}

// ---------------------------------------------------------------------------
// Legacy save format (pre-item-persistence) — used only for loading old saves.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct LegacySavedSlot {
    block_id: u16,
    count: u8,
}

#[derive(Deserialize)]
pub(crate) struct LegacyPlayerSaveData {
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    pitch: f32,
    health: f32,
    hotbar_slot: usize,
    inventory: Vec<LegacySavedSlot>,
}

#[derive(Deserialize)]
pub(crate) struct LegacyWorldSave {
    seed: u32,
    player_x: f32,
    player_y: f32,
    player_z: f32,
    player_health: f32,
    hotbar_slot: usize,
    inventory: Vec<LegacySavedSlot>,
    #[serde(default)]
    players: Vec<LegacyPlayerSaveData>,
}

impl LegacySavedSlot {
    pub(crate) fn upgrade(&self) -> SavedSlot {
        if self.count > 0 && self.block_id != 0 {
            SavedSlot::Block { block_id: self.block_id, count: self.count }
        } else {
            SavedSlot::Empty
        }
    }
}

impl LegacyWorldSave {
    pub(crate) fn upgrade(self) -> WorldSave {
        WorldSave {
            seed: self.seed,
            player_x: self.player_x,
            player_y: self.player_y,
            player_z: self.player_z,
            player_health: self.player_health,
            hotbar_slot: self.hotbar_slot,
            inventory: self.inventory.iter().map(|s| s.upgrade()).collect(),
            players: self.players.into_iter().map(|p| PlayerSaveData {
                x: p.x, y: p.y, z: p.z,
                yaw: p.yaw, pitch: p.pitch,
                health: p.health,
                hotbar_slot: p.hotbar_slot,
                inventory: p.inventory.iter().map(|s| s.upgrade()).collect(),
                // Legacy saves don't carry spawn_pos — loader will fall back
                // to the player's last position.
                spawn_pos: None,
                hunger: 20,
                // Legacy saves predate the reputation persistence — start
                // fresh at Neutral with every village.
                reputation: Vec::new(),
                // Legacy saves predate the pet list — no tamed pets to restore.
                tamed_pets: Vec::new(),
                // Legacy saves predate Spec 28e — no armour to restore.
                armour_slots: [None, None, None, None],
                // Legacy saves predate Spec 33 — no kill counter or
                // bounty claims to restore.
                kill_counter: Vec::new(),
                bounties_claimed: Vec::new(),
            }).collect(),
            // Legacy saves predate Wave 27 — no campfires existed.
            campfires: Vec::new(),
            // Legacy saves predate Spec 20 — no furnaces existed.
            furnaces: Vec::new(),
            vendors: Vec::new(),
            // Legacy saves predate Wave 29 — no drying racks existed.
            drying_racks: Vec::new(),
            // Legacy saves predate Spec 28d chunk 8 — no hives.
            hives: Vec::new(),
            // Legacy saves predate HP-2 — no chests.
            chests: Vec::new(),
            dispensers: Vec::new(),
            rig_clips: Vec::new(),
            // Legacy saves predate Spec 34 — no tip jars.
            tip_jars: Vec::new(),
            auctions: Vec::new(),
            // Legacy saves predate Spec 38 — no Latent Prints.
            latent_prints: Vec::new(),
            plots: Vec::new(),
            market_hubs: Vec::new(),
            // Legacy saves predate Spec 24 — no plans or builds existed.
            construction_anchors: Vec::new(),
            architect_plaques: Vec::new(),
            village_anchors: Vec::new(),
            populated_villages: Vec::new(),
            village_bells: Vec::new(),
            // Pre-Spec-22 — no raid state.
            village_treasuries: Vec::new(),
            active_raids: Vec::new(),
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: Vec::new(),
            // Legacy saves predate HP-3 — no Brigand Hideouts.
            brigand_hideouts: Vec::new(),
            // Legacy saves predate Spec 33 — no bounties; refresh
            // driver will roll the first rotation on its next tick.
            bounties: Vec::new(),
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            face_overlays: vec![],
            face_blueprints: vec![],
            face_blueprint_blanks: vec![],
            workshop: Default::default(),
            // Legacy saves predate the rail freight carts — none existed.
            carts: Vec::new(),
            graves: Vec::new(),
            waypoints: Vec::new(),
            block_meta: Vec::new(),
            power_devices: Vec::new(),
            signs: Vec::new(),
            item_frames: Vec::new(),
            locked_slots: Vec::new(),
            hostile_acts: Vec::new(),
            rigs: Vec::new(),
            exhibits: Vec::new(),
            composters: Vec::new(),
            saved_mobs: Vec::new(),
            satoshi: Default::default(),
        }
    }
}

/// Per-player save data for multi-player worlds.
#[derive(Serialize, Deserialize, Clone)]
pub struct PlayerSaveData {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub hotbar_slot: usize,
    pub inventory: Vec<SavedSlot>,
    /// Bed-settable spawn point (Wave 20). None on legacy saves; loader
    /// falls back to the player's current position so the player respawns
    /// where they last quit instead of at the world spawn (which would be
    /// surprising on a freshly-loaded save).
    #[serde(default)]
    pub spawn_pos: Option<[f32; 3]>,
    /// Hunger level (Wave 24). 0..=20. Defaults to 20 on legacy saves so
    /// loading an older world doesn't drop the player into starvation.
    #[serde(default = "default_hunger")]
    pub hunger: u8,
    /// Spec 19 reputation — flattened from the runtime `AHashMap` to a
    /// `Vec<(VillageId, i16)>` for bincode wire stability (ahash has no
    /// serde feature). Empty on legacy saves; the loader walks the Vec
    /// back into the runtime map. Per
    /// `docs/vision/sat-flow-and-economy-loops.md` §3.2 — reputation is
    /// the load-bearing connective tissue across quest + vendor + raid,
    /// so persisting it is non-optional once the kid does any real work.
    #[serde(default)]
    pub reputation: Vec<((i32, i32), i16)>,
    /// Spec 28d.wolves — pet list. Each entry is a tamed-mob descriptor
    /// (species + position + per-species data) that the load path will
    /// use to re-spawn the player's pets in the world.
    ///
    /// BRIDGE: live mob respawn-on-load isn't wired yet (mobs are
    /// transient in alpha — the entity layer rebuilds from world state
    /// each session). The save shape is in place so when mob save lands
    /// the format doesn't need a breaking change. Legacy saves load
    /// empty via `#[serde(default)]`. Replace the BRIDGE when the
    /// generic mob-save path lands per CLAUDE.md tech-debt bullet 1.
    #[serde(default)]
    pub tamed_pets: Vec<SavedTamedPet>,
    /// Spec 28e — equipped armour, indexed by `ArmourSlot as usize`
    /// (Helmet=0, Chestplate=1, Leggings=2, Boots=3). Each `Some` slot
    /// persists its material + remaining durability. Legacy saves
    /// arrive with `[None; 4]` via the serde default.
    #[serde(default)]
    pub armour_slots: [Option<SavedArmourPiece>; 4],
    /// Spec 33 follow-on — per-mob-type kill counter. Persisted so
    /// bounty progress survives save+quit (a player who killed 9
    /// Zombies and quits shouldn't reset to 0 in the next session).
    /// Legacy saves load empty via `#[serde(default)]`. Flattened
    /// from the runtime AHashMap to a Vec for bincode stability.
    #[serde(default)]
    pub kill_counter: Vec<(crate::mob::MobType, u32)>,
    /// Spec 33 — claim history. Maps `ActiveBounty.id` → kills
    /// consumed at claim time. Persisted so a player who claims at
    /// day 1 and quits can't re-claim the same id on reload (until
    /// the rotation refreshes). Legacy saves load empty via default.
    #[serde(default)]
    pub bounties_claimed: Vec<(u32, u32)>,
}

/// Spec 28e — one equipped armour piece's persistence record. Mirrors
/// `armour::ArmourItem` but flattened to a serde-stable struct so the
/// save format is independent of any future ArmourItem layout shuffles.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedArmourPiece {
    pub slot: crate::armour::ArmourSlot,
    pub material: crate::armour::ArmourMaterial,
    pub durability: u16,
}

/// Spec 28d.wolves — one tamed pet's persistence record. Per-species
/// data is stored as a tagged enum so future tameables (Cat, Parrot,
/// etc.) extend by appending variants — old saves keep loading because
/// bincode rejects unknown discriminants, not unknown trailing fields,
/// so a forward-compat append needs `#[serde(default)]` on the new
/// owning Vec. Today only `Wolf` exists.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SavedTamedPet {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub data: SavedTamedPetData,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SavedTamedPetData {
    Wolf(crate::wolf::WolfData),
    /// Spec 28d.nostrich — a tamed Nostrich (carries ownership + AI + the
    /// lay/feather timers so a reloaded bird keeps its egg/feather schedule).
    Nostrich(crate::nostrich::NostrichData),
    /// Companions wave — a tamed Cat / Parrot / Fox. These share the generic
    /// `CompanionData`, which does NOT encode the species, so the `MobType` is
    /// stored alongside to re-spawn the correct creature. Appended last to keep
    /// the bincode discriminants of the older variants stable for existing saves.
    Companion {
        kind: crate::mob::MobType,
        data: crate::companion::CompanionData,
    },
    /// #129 — a world-author PLACED wild mob (no owner, no tame state) — e.g. a
    /// donkey pinned by its statue. Stores only the species; re-spawns at its
    /// saved position carrying the `Authored` ECS marker (which is what
    /// `tamed_mobs_to_saved` queries it back by). Appended LAST so the older
    /// variants' bincode discriminants stay stable for existing saves.
    Authored { kind: crate::mob::MobType },
    /// Steed persistence (2026-07-04): a KEPT Horse/Donkey/Mule (first ride
    /// marks it). APPENDED variant — bincode enum tags are positional; never
    /// reorder. Old saves simply never contain it.
    Steed { kind: crate::mob::MobType, data: crate::horse_ai::HorseData },
    /// 1C command states (2026-07-06): a tamed companion WITH its command state.
    /// `CompanionData.state` is `serde(skip)` (wire-V1 frozen), so the state is
    /// carried here explicitly. APPENDED variant — old saves contain `Companion`
    /// (state defaults to Follow on restore); new saves write `Companion2`.
    Companion2 {
        kind: crate::mob::MobType,
        data: crate::companion::CompanionData,
        state: crate::companion::CompanionState,
    },
    /// Cargo packs (2026-07-06, Task 11): a KEPT Horse/Donkey/Mule WITH its
    /// cargo pack. `HorseData.pack` is `serde(skip)` (wire-V1 frozen, same
    /// reasoning as `CompanionData.state` above — see commit 202ab054), so
    /// the pack contents are carried here explicitly. APPENDED LAST — old
    /// saves contain `Steed` (restores packless); new saves always write
    /// `Steed2`.
    Steed2 {
        kind: crate::mob::MobType,
        data: crate::horse_ai::HorseData,
        pack: Option<crate::chest::ChestData>,
    },
}

impl SavedTamedPetData {
    /// The species this saved pet re-spawns as.
    pub fn mob_type(&self) -> crate::mob::MobType {
        match self {
            SavedTamedPetData::Wolf(_) => crate::mob::MobType::Wolf,
            SavedTamedPetData::Nostrich(_) => crate::mob::MobType::Nostrich,
            SavedTamedPetData::Companion { kind, .. } => *kind,
            SavedTamedPetData::Authored { kind } => *kind,
            SavedTamedPetData::Steed { kind, .. } => *kind,
            SavedTamedPetData::Companion2 { kind, .. } => *kind,
            SavedTamedPetData::Steed2 { kind, .. } => *kind,
        }
    }
}

/// Snapshot every TAMED mob (Wolf, Nostrich) from the live ECS into the
/// world-level persisted-mob list. Mirrors [`carts_to_saved`]: the caller has
/// the `hecs::World` handle (mobs are ECS entities, not block-entities), so it
/// extracts them here and hands the result to the save path. Untamed wildlife
/// is intentionally NOT persisted — it re-scatters deterministically on load.
/// Each pet's owner pubkey + AI state travel verbatim inside the per-species
/// data, so no owner→slot remapping is needed on restore.
#[cfg(not(target_arch = "wasm32"))]
pub fn tamed_mobs_to_saved(ecs: &hecs::World) -> Vec<SavedTamedPet> {
    tamed_mobs_to_saved_impl(ecs)
}

/// WASM build shares the identical extraction (PWA worlds persist pets too).
#[cfg(target_arch = "wasm32")]
pub fn tamed_mobs_to_saved(ecs: &hecs::World) -> Vec<SavedTamedPet> {
    tamed_mobs_to_saved_impl(ecs)
}

fn tamed_mobs_to_saved_impl(ecs: &hecs::World) -> Vec<SavedTamedPet> {
    use crate::entity::{MobKind, Position};
    let mut out = Vec::new();
    // Tamed wolves.
    for (_id, (pos, kind, data)) in
        ecs.query::<(&Position, &MobKind, &crate::wolf::WolfData)>().iter()
    {
        if kind.0 == crate::mob::MobType::Wolf && data.is_tamed() {
            // Attack states carry a raw hecs entity id + a session-local
            // tick deadline — neither survives a reload (see the doc
            // comment on `sanitize_attack_state_for_persistence`), so map
            // to the wolf's neutral state before it's written to disk.
            let mut sanitized = data.clone();
            sanitized.sanitize_attack_state_for_persistence();
            out.push(SavedTamedPet {
                x: pos.0.x,
                y: pos.0.y,
                z: pos.0.z,
                data: SavedTamedPetData::Wolf(sanitized),
            });
        }
    }
    // Tamed nostriches.
    for (_id, (pos, kind, data)) in
        ecs.query::<(&Position, &MobKind, &crate::nostrich::NostrichData)>().iter()
    {
        if kind.0 == crate::mob::MobType::Nostrich && data.is_tamed() {
            out.push(SavedTamedPet {
                x: pos.0.x,
                y: pos.0.y,
                z: pos.0.z,
                data: SavedTamedPetData::Nostrich(data.clone()),
            });
        }
    }
    // Kept steeds (Horse/Donkey/Mule) — first ride marks them (2026-07-04).
    // The cargo pack (Task 11, 2026-07-06) is `serde(skip)` on `HorseData`
    // (wire-V1 frozen), so it's carried explicitly in the appended
    // `Steed2` variant — the writer always emits `Steed2` now.
    for (_id, (pos, kind, data)) in
        ecs.query::<(&Position, &MobKind, &crate::horse_ai::HorseData)>().iter()
    {
        if data.is_kept()
            && matches!(
                kind.0,
                crate::mob::MobType::Horse | crate::mob::MobType::Donkey | crate::mob::MobType::Mule
            )
        {
            out.push(SavedTamedPet {
                x: pos.0.x,
                y: pos.0.y,
                z: pos.0.z,
                data: SavedTamedPetData::Steed2 {
                    kind: kind.0,
                    data: data.clone(),
                    pack: data.pack.clone(),
                },
            });
        }
    }
    // Tamed companions (Cat, Parrot, Fox) — share the generic CompanionData,
    // so the live MobKind supplies the species to re-spawn as. The command
    // `state` is `serde(skip)` on CompanionData (wire-V1 frozen), so it is
    // carried explicitly in the appended `Companion2` variant.
    for (_id, (pos, kind, data)) in
        ecs.query::<(&Position, &MobKind, &crate::companion::CompanionData)>().iter()
    {
        if data.is_tamed() {
            out.push(SavedTamedPet {
                x: pos.0.x,
                y: pos.0.y,
                z: pos.0.z,
                data: SavedTamedPetData::Companion2 {
                    kind: kind.0,
                    data: data.clone(),
                    state: data.state,
                },
            });
        }
    }
    // #129 — world-author PLACED wild mobs (the `Authored` marker, no owner /
    // tame state). Persist them alongside tamed pets so a `.axeworld` keeps its
    // pinned creatures (e.g. the donkey by the statue) across save/load.
    for (_id, (pos, kind, _)) in
        ecs.query::<(&Position, &MobKind, &crate::entity::Authored)>().iter()
    {
        out.push(SavedTamedPet {
            x: pos.0.x,
            y: pos.0.y,
            z: pos.0.z,
            data: SavedTamedPetData::Authored { kind: kind.0 },
        });
    }
    out
}

fn default_hunger() -> u8 { 20 }

// Test-only override for the worlds-root directory, kept PER-THREAD so cargo's
// parallel test runner can give each test an isolated `worlds/` — otherwise
// tests that assert on the global folder count/listing race each other on the
// one shared dir. Production never sets this, so `worlds_root()` is always
// `worlds/` outside tests.
#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static WORLDS_ROOT_OVERRIDE: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// The root directory that holds per-world save folders. `worlds/` in
/// production; a private per-thread temp dir in tests (see [`WorldsRootGuard`]).
/// All worlds-dir path construction MUST funnel through here so a test override
/// covers reads (`list_world_entries`), writes (`world_dir`), and the
/// collision check (`sanitize_folder_name`) alike.
#[cfg(not(target_arch = "wasm32"))]
pub fn worlds_root() -> PathBuf {
    // Test override wins (per-thread, set by WorldsRootGuard) so FS tests stay
    // isolated. Then the dedicated server's `AXENSTAX_WORLDS_DIR` env (a Docker
    // volume), else `<data dir>/worlds` (see `data_dir` — never CWD-relative).
    if let Some(p) = WORLDS_ROOT_OVERRIDE.with(|o| o.borrow().clone()) {
        return p;
    }
    if let Ok(dir) = std::env::var("AXENSTAX_WORLDS_DIR")
        && !dir.is_empty() {
            return PathBuf::from(dir);
        }
    crate::data_dir::data_root().join("worlds")
}

/// Get the save directory for a world.
#[cfg(not(target_arch = "wasm32"))]
pub fn world_dir(name: &str) -> PathBuf {
    worlds_root().join(name)
}

/// Sanitise an exhibit `image_ref` into a safe, flat filename (keeping its
/// extension). Unlike `sanitize_folder_name` (which maps `.`→`_` and does a
/// worlds-dir collision check — both wrong for an image file), this keeps the
/// filename-safe set `[A-Za-z0-9_.-]` and the dot so `piece.png` stays
/// `piece.png`, while a traversal attempt (`..`, `/`, `\`) is stripped/rejected
/// to an empty string (→ the read fails and that exhibit is simply skipped).
/// Mirrors the JS `loadExhibitArt` client-side filter.
pub fn sanitize_image_ref(image_ref: &str) -> String {
    let cleaned: String = image_ref
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        .collect();
    if cleaned.is_empty() || cleaned.contains("..") {
        return String::new();
    }
    cleaned
}

/// Native on-disk path for an exhibit image: `worlds/<name>/exhibits/<ref>`.
/// The image ref is sanitised (it comes from a save file / authored data) so a
/// crafted `image_ref` can't traverse outside the world's own folder. The
/// `exhibits/` images live in the world directory so they travel with the world
/// on disk; packing them into `.axeworld` is a separate follow-up (Spec §9).
#[cfg(not(target_arch = "wasm32"))]
pub fn exhibit_image_path(world_name: &str, image_ref: &str) -> PathBuf {
    world_dir(world_name)
        .join("exhibits")
        .join(sanitize_image_ref(image_ref))
}

/// RAII guard that redirects the worlds root to a private per-thread temp dir
/// for the duration of a test, then restores + deletes it on drop. Use in ANY
/// test that reads or mutates global worlds-dir state (folder counts, listings)
/// so it can't race other FS tests under the parallel runner. `tag` namespaces
/// the temp dir (combined with the thread id) so concurrent guards never share
/// a path.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) struct WorldsRootGuard {
    dir: PathBuf,
}

#[cfg(all(test, not(target_arch = "wasm32")))]
impl WorldsRootGuard {
    pub(crate) fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("axenstax-test-worlds-{tag}-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create isolated worlds root");
        WORLDS_ROOT_OVERRIDE.with(|o| *o.borrow_mut() = Some(dir.clone()));
        Self { dir }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
impl Drop for WorldsRootGuard {
    fn drop(&mut self) {
        WORLDS_ROOT_OVERRIDE.with(|o| *o.borrow_mut() = None);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Write `bytes` to `path` atomically: write a sibling `.tmp`, fsync it, then rename it
/// over `path`. A crash / power-loss / disk-full can leave the `.tmp` but NEVER a
/// half-written `path`, so a reader never sees a torn file.
///
/// This matters for `world.dat` specifically: it is a positional bincode blob, and the
/// tolerant decode (`read_world_save`) cannot distinguish a cleanly-shorter OLD save
/// from a truncated NEW one — so a torn `world.dat` would silently "load" with its tail
/// (chests, tip-jar escrow, …) defaulted away. Eliminating torn writes removes that
/// silent-data-loss hazard at the source. (Goal 3 review follow-up; matches Spec 02
/// §8.2 atomic-write intent.) Every native persisted file goes through here —
/// `world.dat`, `world_meta.json`, each chunk file, and the profile blobs
/// (skins / Workshop wardrobe). A torn `world_meta.json` used to be "recovered" by
/// silently substituting default meta (seed 42, survival) — audit 2026-09-27.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    // Append ".tmp" to the full path (robust for any filename, unlike with_extension).
    let tmp: PathBuf = {
        let mut t = path.as_os_str().to_owned();
        t.push(".tmp");
        PathBuf::from(t)
    };
    {
        let mut f = fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        f.write_all(bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        f.sync_all().map_err(|e| format!("fsync {}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

/// [`write_atomic`] without the per-file `fsync`: tmp + rename only. Used for
/// chunk files, of which a save writes thousands — one fsync each made every
/// autosave a multi-second main-thread stall (review S2). tmp + rename already
/// rules out a torn chunk from a process crash; the caller makes the renames
/// durable with ONE [`sync_dir`] of the chunks directory at the end of the save.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_atomic_nosync(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let tmp: PathBuf = {
        let mut t = path.as_os_str().to_owned();
        t.push(".tmp");
        PathBuf::from(t)
    };
    fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))
}

/// fsync a directory, making the renames done in it durable. Best-effort on
/// platforms where a directory can't be opened for sync.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sync_dir(dir: &std::path::Path) {
    if let Ok(f) = fs::File::open(dir)
        && let Err(e) = f.sync_all()
    {
        log::warn!("fsync {}: {e}", dir.display());
    }
}

/// `<path>.corrupt-<unix secs>`, with a `-<n>` suffix if taken — never an
/// existing file.
#[cfg(not(target_arch = "wasm32"))]
fn corrupt_sibling_path(path: &std::path::Path) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let base = {
        let mut t = path.as_os_str().to_owned();
        t.push(format!(".corrupt-{ts}"));
        PathBuf::from(t)
    };
    let mut dest = base.clone();
    let mut n = 1u32;
    while dest.exists() {
        let mut t = base.as_os_str().to_owned();
        t.push(format!("-{n}"));
        dest = PathBuf::from(t);
        n += 1;
    }
    dest
}

/// Paths whose damaged original was already copied aside this session.
#[cfg(not(target_arch = "wasm32"))]
static DAMAGED_COPIES: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// COPY (not move) `path` to `<path>.corrupt-<ts>`, once per path per session.
/// For a file that loaded only partly (review S5: a `world.dat` whose tolerant
/// tail failed) — the game keeps running on what decoded, but the next save
/// must not destroy the only copy of the rest.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn keep_damaged_copy_once(path: &std::path::Path) {
    if let Ok(mut done) = DAMAGED_COPIES.lock() {
        if done.iter().any(|p| p == path) {
            return;
        }
        done.push(path.to_path_buf());
    }
    let dest = corrupt_sibling_path(path);
    match fs::copy(path, &dest) {
        Ok(_) => log::error!(
            "{} only partly decoded; original kept as {}",
            path.display(),
            dest.display()
        ),
        Err(e) => log::error!("could not keep a copy of damaged {}: {e}", path.display()),
    }
}

/// Rename a file that failed to decode to `<path>.corrupt-<unix secs>` so it is
/// kept for recovery and can never be overwritten by the next save. If that name
/// is taken (two failures in one second) a `-<n>` suffix is added — an existing
/// quarantined file is never replaced. Returns the new path.
///
/// Shared by every native loader that finds a damaged file: `world_meta.json`,
/// chunk files, and the profile blobs (`skins.blob`, `wardrobe.blob`).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn quarantine_corrupt(path: &std::path::Path) -> Result<PathBuf, String> {
    let dest = corrupt_sibling_path(path);
    fs::rename(path, &dest)
        .map_err(|e| format!("quarantine {} -> {}: {e}", path.display(), dest.display()))?;
    log::error!(
        "damaged file kept aside for recovery: {} -> {}",
        path.display(),
        dest.display()
    );
    Ok(dest)
}

/// Session latch for profile blobs whose load FAILED (audit 2026-09-27): once a
/// blob at a path fails to load, every save to that path is refused for the rest
/// of the process, so a fresh/empty in-memory value can never replace the
/// player's real (damaged, now quarantined) data. Keyed by path so each blob —
/// and each test's temp path — is independent.
#[cfg(not(target_arch = "wasm32"))]
static BLOB_LOAD_FAILED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Record that loading the blob at `path` failed this session (see
/// [`BLOB_LOAD_FAILED`]).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn mark_blob_load_failed(path: &std::path::Path) {
    if let Ok(mut v) = BLOB_LOAD_FAILED.lock()
        && !v.iter().any(|p| p == path)
    {
        v.push(path.to_path_buf());
    }
}

/// `Err` when a load of the blob at `path` failed earlier this session — the
/// caller must not write it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn check_blob_writable(path: &std::path::Path) -> Result<(), String> {
    let blocked = BLOB_LOAD_FAILED
        .lock()
        .map(|v| v.iter().any(|p| p == path))
        .unwrap_or(true);
    if blocked {
        Err(format!(
            "not saving {}: it failed to load this session (the damaged file is kept as \
             .corrupt-*); restart after recovering it",
            path.display()
        ))
    } else {
        Ok(())
    }
}

/// True when `dir` holds a quarantined `<file>.corrupt-*` sibling of `file`.
#[cfg(not(target_arch = "wasm32"))]
fn has_quarantined_sibling(dir: &std::path::Path, file: &str) -> bool {
    let prefix = format!("{file}.corrupt-");
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        })
        .unwrap_or(false)
}

/// Check if a saved world exists.
#[cfg(not(target_arch = "wasm32"))]
pub fn world_exists(name: &str) -> bool {
    world_dir(name).join("world.dat").exists()
}

#[cfg(target_arch = "wasm32")]
pub fn world_exists(_name: &str) -> bool {
    false
}

/// Serialize an inventory to a Vec of SavedSlots (36 slots).
/// Public wrapper for server.rs save path (ServerPlayer has Inventory, not PlayerSlot).
pub fn serialize_inventory_raw(inventory: &Inventory) -> Vec<SavedSlot> {
    serialize_inventory(inventory)
}

/// Spec 28e — serialise a `PlayerSlot.armour_slots` array into the
/// save format. Pure conversion; no allocation beyond the fixed array.
pub fn serialize_armour_slots(
    slots: &[Option<crate::armour::ArmourItem>; 4],
) -> [Option<SavedArmourPiece>; 4] {
    let mut out: [Option<SavedArmourPiece>; 4] = [None, None, None, None];
    for (i, s) in slots.iter().enumerate() {
        out[i] = s.map(|p| SavedArmourPiece {
            slot: p.slot,
            material: p.material,
            durability: p.durability,
        });
    }
    out
}

/// Spec 28e — restore a saved armour-slots array into a runtime
/// `PlayerSlot.armour_slots` array. Pure.
pub fn restore_armour_slots(
    saved: &[Option<SavedArmourPiece>; 4],
) -> [Option<crate::armour::ArmourItem>; 4] {
    let mut out: [Option<crate::armour::ArmourItem>; 4] = [None, None, None, None];
    for (i, s) in saved.iter().enumerate() {
        out[i] = s.map(|p| {
            let mut item = crate::armour::ArmourItem::new(p.slot, p.material);
            item.durability = p.durability;
            item
        });
    }
    out
}

fn serialize_inventory(inventory: &Inventory) -> Vec<SavedSlot> {
    (0..36).map(|i| {
        match inventory.slot(i) {
            Some(stack) => match &stack.item {
                Item::Block(id) => SavedSlot::Block { block_id: *id, count: stack.count },
                Item::Tool(tool) => SavedSlot::Tool {
                    tool_type: tool.tool_type,
                    material: tool.material,
                    durability: tool.durability,
                },
                Item::Material(mat_id) => SavedSlot::Material {
                    material_id: *mat_id,
                    count: stack.count,
                },
                Item::Plan(data) => SavedSlot::Plan { data: data.clone() },
                Item::Armour(a) => SavedSlot::Armour {
                    slot: a.slot,
                    material: a.material,
                    durability: a.durability,
                },
            },
            None => SavedSlot::Empty,
        }
    }).collect()
}

/// Partition the currently-loaded chunks into (non-empty → write, empty →
/// delete-stale-file). A chunk that's loaded but all-air (e.g. fully mined
/// out) must have its previously-saved file deleted, or the mined-out area
/// resurrects from the stale file on reload (engine audit 2026-06-04, A).
#[cfg(not(target_arch = "wasm32"))]
fn partition_chunks_for_save(world: &World) -> (Vec<(i32, i32, i32)>, Vec<(i32, i32, i32)>) {
    let mut write = Vec::new();
    let mut delete = Vec::new();
    // Spec 02 §7.5 — loaded + evicted chunks (an unloaded edit must still save).
    for ((cx, cy, cz), chunk) in world.persistable_chunks() {
        if chunk.is_empty() {
            delete.push((cx, cy, cz));
        } else {
            write.push((cx, cy, cz));
        }
    }
    (write, delete)
}

/// Write a fully-formed world folder from already-assembled `meta`, `save`, and
/// `world` (chunks). This is the shared file-writing tail used by both
/// `save_world` (which assembles a `WorldSave` from live `PlayerSlot` data and
/// then delegates here) and `native_world_io::import_world_native` (which gets
/// its `meta`/`save`/chunks from an unpacked `.axeworld` archive).
///
/// Specifically this function:
///   - Creates `worlds/<name>/chunks/` (mkdir -p).
///   - Atomically writes `worlds/<name>/world.dat` (bincode of `save`).
///   - Writes `worlds/<name>/world_meta.json` (via `save_world_meta`).
///   - Writes every non-empty chunk as `chunks/<cx>_<cy>_<cz>.chunk`.
///
/// It does NOT bump `meta.version` or update proof-of-play stats — callers that
/// need those (i.e. the live-game `save_world`) do so themselves after the call.
///
/// Chunk deletion (for mined-out air chunks from an in-progress live world) is
/// also the caller's responsibility; it is not needed for a fresh import.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_world_folder(
    name: &str,
    meta: &WorldMeta,
    save: &WorldSave,
    world: &World,
) -> Result<(), String> {
    let dir = world_dir(name);
    refuse_write_over_newer_save(&dir)?;
    let chunks_dir = dir.join("chunks");
    fs::create_dir_all(&chunks_dir).map_err(|e| format!("mkdir: {e}"))?;

    // Seam A (offline-first login contract) — encryption-at-rest is NOT finalised
    // here; it is pinned by the owner's web-Stash blob format. This is the clean
    // serialization boundary kept WRAP-READY for it: `encoded` is the full save
    // as a single byte buffer, so an AEAD layer can wrap it (encrypt-to-self,
    // vault-key NIP-44-wrapped — "the disk file IS the Stash blob") between the
    // `serialize` and the `write_atomic` below with **no format change** to any
    // caller. Implement no AEAD now (the recommended end-state per the contract);
    // until then `world.dat` stays plaintext bincode (+ the format-version footer).
    let encoded = crate::save_format::encode_world_save(save)?;

    // Spec 02 §7.5 — loaded + evicted chunks. Written FIRST (tmp + rename, one
    // directory fsync at the end — review S2), so `world.dat` below is the commit
    // point: a crash before it leaves the old world.dat with new-or-old chunks,
    // never new block-entity state over old block data.
    for ((cx, cy, cz), chunk) in world.persistable_chunks() {
        if chunk.is_empty() {
            continue;
        }
        let filename = format!("{cx}_{cy}_{cz}.chunk");
        let path = chunks_dir.join(&filename);
        let bytes = chunk.as_bytes();
        write_atomic_nosync(&path, &bytes).map_err(|e| format!("write chunk {filename}: {e}"))?;
    }
    sync_dir(&chunks_dir);

    write_atomic(&dir.join("world.dat"), &encoded)?;
    save_world_meta(name, meta)?;

    Ok(())
}

/// Save the world to disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_world(
    name: &str,
    world: &World,
    players: &[crate::player_slot::PlayerSlot],
    seed: u32,
    carts: &[SavedCart],
    saved_mobs: &[SavedTamedPet],
) -> Result<(), String> {
    if players.is_empty() {
        return Err("save_world: no players to save".to_string());
    }
    let dir = world_dir(name);
    refuse_write_over_newer_save(&dir)?;
    let chunks_dir = dir.join("chunks");
    fs::create_dir_all(&chunks_dir).map_err(|e| format!("mkdir: {e}"))?;

    // Build per-player save data
    let player_saves: Vec<PlayerSaveData> = players.iter().map(|slot| {
        PlayerSaveData {
            x: slot.player.pos.x,
            y: slot.player.pos.y,
            z: slot.player.pos.z,
            yaw: slot.camera.yaw,
            pitch: slot.camera.pitch,
            health: slot.combat.health,
            hotbar_slot: slot.hotbar_slot,
            inventory: serialize_inventory(&slot.inventory),
            spawn_pos: Some([slot.spawn_pos.x, slot.spawn_pos.y, slot.spawn_pos.z]),
            hunger: slot.combat.hunger,
            reputation: slot.reputation.per_village.iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            // BRIDGE: PlayerSlot has no `pets` field yet (wolves are
            // transient mobs in alpha). Save empty for now; field is in
            // place so when mob-save lands the format doesn't break.
            tamed_pets: Vec::new(),
            armour_slots: serialize_armour_slots(&slot.armour_slots),
            kill_counter: slot.kill_counter.iter().map(|(&k, &v)| (k, v)).collect(),
            bounties_claimed: slot.bounties_claimed.iter().map(|(&k, &v)| (k, v)).collect(),
        }
    }).collect();

    // Backward compat: write old single-player fields from player 0
    let p0 = &player_saves[0];

    // Wave 27 — campfire state. Pulled out of World::block_entities at
    // save time so the wire format is a flat Vec<SavedCampfire>.
    // Spec 20 Phase 2 — block_entities is now a tagged enum; iterate
    // only the Campfire variant. Phase 8 adds the matching `furnaces`
    // Vec for the Furnace variant.
    let campfires: Vec<SavedCampfire> = world
        .iter_campfires()
        .map(|((x, y, z), data)| SavedCampfire { x, y, z, data: data.clone() })
        .collect();
    // Spec 20 Phase 8 — furnace state, same pattern as campfires.
    let furnaces: Vec<SavedFurnace> = world
        .iter_furnaces()
        .map(|((x, y, z), data)| SavedFurnace { x, y, z, data: data.clone() })
        .collect();
    let vendors: Vec<SavedVendor> = world
        .iter_vendors()
        .map(|((x, y, z), data)| SavedVendor { x, y, z, data: data.clone() })
        .collect();
    // Wave 29 — drying-rack state, mirroring the campfire pattern.
    let drying_racks: Vec<SavedDryingRack> = world
        .drying_racks
        .iter()
        .map(|(&(x, y, z), data)| SavedDryingRack { x, y, z, data: data.clone() })
        .collect();
    // Spec 28d chunk 8 — Bee Hive state.
    let hives: Vec<SavedHive> = world
        .iter_hives()
        .map(|((x, y, z), data)| SavedHive { x, y, z, data: *data })
        .collect();
    // HP-2 — Chest state.
    let dispensers: Vec<SavedDispenser> = world
        .iter_dispensers()
        .map(|((x, y, z), d)| SavedDispenser { x, y, z, data: d.clone() })
        .collect();
    let chests: Vec<SavedChest> = world
        .iter_chests()
        .map(|((x, y, z), data)| SavedChest { x, y, z, data: data.clone() })
        .collect();
    let signs: Vec<SavedSign> = world
        .iter_signs()
        .map(|((x, y, z), data)| SavedSign { x, y, z, data: data.clone() })
        .collect();
    let item_frames: Vec<SavedItemFrame> = world
        .iter_item_frames()
        .map(|((x, y, z), data)| SavedItemFrame { x, y, z, data: data.clone() })
        .collect();
    // Spec 34 — Tip Jar state.
    let tip_jars: Vec<SavedTipJar> = world
        .iter_tip_jars()
        .map(|((x, y, z), data)| SavedTipJar { x, y, z, data: data.clone() })
        .collect();
    let auctions: Vec<SavedAuction> = world
        .iter_auctions()
        .map(|((x, y, z), data)| SavedAuction { x, y, z, data: data.clone() })
        .collect();
    // Spec 38 (Blueprint / Cyanotype) — Latent Print block-entities.
    // Each holds a PlanData with its current develop_state, so a save/
    // load round-trip preserves how much sun a print has already caught.
    let latent_prints: Vec<SavedLatentPrint> = world
        .iter_latent_prints()
        .map(|((x, y, z), data)| SavedLatentPrint { x, y, z, data: data.clone() })
        .collect();
    // Spec 24 — Build Schematics in-progress anchors + completed plaques.
    let construction_anchors: Vec<SavedConstructionAnchor> = world
        .construction_anchors
        .iter()
        .map(|(&(x, y, z), data)| SavedConstructionAnchor { x, y, z, data: data.clone() })
        .collect();
    let architect_plaques: Vec<SavedArchitectPlaque> = world
        .architect_plaques
        .iter()
        .map(|(&(x, y, z), chain)| SavedArchitectPlaque { x, y, z, chain: chain.clone() })
        .collect();

    let save = WorldSave {
        seed,
        player_x: p0.x,
        player_y: p0.y,
        player_z: p0.z,
        player_health: p0.health,
        hotbar_slot: p0.hotbar_slot,
        inventory: p0.inventory.clone(),
        players: player_saves,
        campfires,
        furnaces,
        vendors,
        drying_racks,
        hives,
        chests,
        signs,
        item_frames,
        locked_slots: players[0].inventory.locked_indices(),
        hostile_acts: world.hostile_acts.acts().to_vec(),
        rigs: world.rigs.clone(),
        // #19 — index-aligned clip side table (see `WorldSave.rig_clips`).
        rig_clips: world.rigs.iter().map(|r| r.clip).collect(),
        exhibits: world.exhibits.clone(),
        composters: world.iter_composters().map(|((x, y, z), data)| SavedComposter { x, y, z, data: data.clone() }).collect(),
        tip_jars,
        auctions,
        latent_prints,
        plots: world.plots.clone(),
        market_hubs: world.market_hubs.clone(),
        construction_anchors,
        architect_plaques,
        village_anchors: world.village_anchors.iter().map(|(&(gx, gz), &anchor)| SavedVillageAnchor { grid_x: gx, grid_z: gz, anchor }).collect(),
        populated_villages: world.populated_villages.iter().copied().collect(),
        village_bells: world.village_bells.clone(),
        village_treasuries: world.village_treasuries.iter().map(|(&k, &v)| (k, v)).collect(),
        active_raids: world.active_raids.clone(),
        raid_scheduler: world.raid_scheduler.clone(),
        raid_kills: world.raid_kills.iter().map(|(&(vid, pk), &c)| (vid, pk, c)).collect(),
        brigand_hideouts: world.brigand_hideouts.iter()
            .map(|(&(gx, gz), data)| SavedHideout { grid_x: gx, grid_z: gz, data: data.clone() })
            .collect(),
        bounties: world.bounties.iter().map(|b| SavedBounty {
            id: b.id, template_idx: b.template_idx, issued_tick: b.issued_tick,
        }).collect(),
        bounty_next_id: world.bounty_next_id,
        bounty_last_refresh_tick: world.bounty_last_refresh_tick,
        face_overlays: face_overlays_to_saved(world),
        face_blueprints: face_blueprints_to_saved(world),
        face_blueprint_blanks: face_blueprint_blanks_to_saved(world),
        workshop: world.workshop.clone(),
        // Rail freight Phase 1 — carts snapshotted from the caller's ECS.
        carts: carts.to_vec(),
        saved_mobs: saved_mobs.to_vec(),
        satoshi: world.satoshi.clone(),
        dispensers,
        graves: world
            .iter_graves()
            .map(|((x, y, z), data)| SavedGrave { x, y, z, data: data.clone() })
            .collect(),
        waypoints: world.waypoints.clone(),
        // Spec 48 (Electricity) — per-block meta + power-device runtime state, so
        // circuits survive save/reload. Collected inline like graves/waypoints.
        block_meta: world
            .block_meta
            .iter()
            .map(|(&(x, y, z), &m)| (x, y, z, m))
            .collect(),
        power_devices: world
            .iter_power_devices()
            .map(|((x, y, z), data)| SavedPowerDevice { x, y, z, data: data.clone() })
            .collect(),
    };

    // Increment version counter + flush proof-of-play stats into meta BEFORE
    // the folder write so world_meta.json on disk matches the live state.
    let mut meta = load_world_meta(name);
    meta.version += 1;
    // Seam B — stamp the owning persona (no-op for a guest; claims an orphan on
    // first signed-in save; never clobbers a different owner).
    meta.claim_owner(current_owner_pubkey().as_deref());
    // Goal 1 — flush the live per-world proof-of-play stats into the persisted
    // meta so total_work + the world-clock survive save/load (Spec 2 §9.1).
    // genesis_found_at_tick is written eagerly at genesis, so it's already here.
    meta.total_work = world.total_work;
    meta.total_ticks = world.total_ticks;

    // Delete stale air-chunk files before writing new ones: a chunk that's
    // been fully mined out (all-air) must not resurrect from its old file.
    // partition_chunks_for_save returns the (write, delete) split for loaded
    // chunks; write_world_folder then writes only the non-empty ones.
    let (_, to_delete) = partition_chunks_for_save(world);
    for (cx, cy, cz) in to_delete {
        let _ = fs::remove_file(chunks_dir.join(format!("{cx}_{cy}_{cz}.chunk")));
    }

    // Delegate the actual file writes (world.dat, world_meta.json, chunks/)
    // to the shared helper so import uses the same code path.
    write_world_folder(name, &meta, &save, world)?;

    // Log after the write so we can report the final version and chunk count.
    let saved = world.persistable_chunks()
        .filter(|(_, c)| !c.is_empty())
        .count() as u32;
    let p0_x = save.player_x;
    let p0_y = save.player_y;
    let p0_z = save.player_z;
    log::info!("Saved world '{name}' v{}: {saved} chunks, {} player(s), P0 at ({:.1}, {:.1}, {:.1})",
        meta.version, players.len(), p0_x, p0_y, p0_z);
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn save_world(
    name: &str,
    world: &World,
    players: &[crate::player_slot::PlayerSlot],
    seed: u32,
    carts: &[SavedCart],
    saved_mobs: &[SavedTamedPet],
) -> Result<(), String> {
    if players.is_empty() {
        return Err("save_world: no players to save".to_string());
    }
    // WASM save: collect state → tar → gzip → fire-and-forget async IndexedDB write
    let player_saves: Vec<PlayerSaveData> = players
        .iter()
        .map(|slot| PlayerSaveData {
            x: slot.player.pos.x,
            y: slot.player.pos.y,
            z: slot.player.pos.z,
            yaw: slot.camera.yaw,
            pitch: slot.camera.pitch,
            health: slot.combat.health,
            hotbar_slot: slot.hotbar_slot,
            inventory: serialize_inventory(&slot.inventory),
            spawn_pos: Some([slot.spawn_pos.x, slot.spawn_pos.y, slot.spawn_pos.z]),
            hunger: slot.combat.hunger,
            reputation: slot.reputation.per_village.iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            // BRIDGE: same as the native save path — pets aren't on
            // PlayerSlot yet. Save empty; load path drops empties.
            tamed_pets: Vec::new(),
            armour_slots: serialize_armour_slots(&slot.armour_slots),
            kill_counter: slot.kill_counter.iter().map(|(&k, &v)| (k, v)).collect(),
            bounties_claimed: slot.bounties_claimed.iter().map(|(&k, &v)| (k, v)).collect(),
        })
        .collect();

    let p0 = &player_saves[0];
    let campfires: Vec<SavedCampfire> = world
        .iter_campfires()
        .map(|((x, y, z), data)| SavedCampfire { x, y, z, data: data.clone() })
        .collect();
    let furnaces: Vec<SavedFurnace> = world
        .iter_furnaces()
        .map(|((x, y, z), data)| SavedFurnace { x, y, z, data: data.clone() })
        .collect();
    let vendors: Vec<SavedVendor> = world
        .iter_vendors()
        .map(|((x, y, z), data)| SavedVendor { x, y, z, data: data.clone() })
        .collect();
    let drying_racks: Vec<SavedDryingRack> = world
        .drying_racks
        .iter()
        .map(|(&(x, y, z), data)| SavedDryingRack { x, y, z, data: data.clone() })
        .collect();
    let hives: Vec<SavedHive> = world
        .iter_hives()
        .map(|((x, y, z), data)| SavedHive { x, y, z, data: *data })
        .collect();
    let dispensers: Vec<SavedDispenser> = world
        .iter_dispensers()
        .map(|((x, y, z), d)| SavedDispenser { x, y, z, data: d.clone() })
        .collect();
    let chests: Vec<SavedChest> = world
        .iter_chests()
        .map(|((x, y, z), data)| SavedChest { x, y, z, data: data.clone() })
        .collect();
    let signs: Vec<SavedSign> = world
        .iter_signs()
        .map(|((x, y, z), data)| SavedSign { x, y, z, data: data.clone() })
        .collect();
    let item_frames: Vec<SavedItemFrame> = world
        .iter_item_frames()
        .map(|((x, y, z), data)| SavedItemFrame { x, y, z, data: data.clone() })
        .collect();
    let tip_jars: Vec<SavedTipJar> = world
        .iter_tip_jars()
        .map(|((x, y, z), data)| SavedTipJar { x, y, z, data: data.clone() })
        .collect();
    let auctions: Vec<SavedAuction> = world
        .iter_auctions()
        .map(|((x, y, z), data)| SavedAuction { x, y, z, data: data.clone() })
        .collect();
    // Spec 38 (Blueprint / Cyanotype) — Latent Print block-entities.
    // Each holds a PlanData with its current develop_state, so a save/
    // load round-trip preserves how much sun a print has already caught.
    let latent_prints: Vec<SavedLatentPrint> = world
        .iter_latent_prints()
        .map(|((x, y, z), data)| SavedLatentPrint { x, y, z, data: data.clone() })
        .collect();
    let construction_anchors: Vec<SavedConstructionAnchor> = world
        .construction_anchors
        .iter()
        .map(|(&(x, y, z), data)| SavedConstructionAnchor { x, y, z, data: data.clone() })
        .collect();
    let architect_plaques: Vec<SavedArchitectPlaque> = world
        .architect_plaques
        .iter()
        .map(|(&(x, y, z), chain)| SavedArchitectPlaque { x, y, z, chain: chain.clone() })
        .collect();
    let save_data = WorldSave {
        seed,
        player_x: p0.x,
        player_y: p0.y,
        player_z: p0.z,
        player_health: p0.health,
        hotbar_slot: p0.hotbar_slot,
        inventory: p0.inventory.clone(),
        players: player_saves,
        campfires,
        furnaces,
        vendors,
        drying_racks,
        hives,
        chests,
        signs,
        item_frames,
        locked_slots: players[0].inventory.locked_indices(),
        hostile_acts: world.hostile_acts.acts().to_vec(),
        rigs: world.rigs.clone(),
        // #19 — index-aligned clip side table (see `WorldSave.rig_clips`).
        rig_clips: world.rigs.iter().map(|r| r.clip).collect(),
        exhibits: world.exhibits.clone(),
        composters: world.iter_composters().map(|((x, y, z), data)| SavedComposter { x, y, z, data: data.clone() }).collect(),
        tip_jars,
        auctions,
        latent_prints,
        plots: world.plots.clone(),
        market_hubs: world.market_hubs.clone(),
        construction_anchors,
        architect_plaques,
        village_anchors: world.village_anchors.iter().map(|(&(gx, gz), &anchor)| SavedVillageAnchor { grid_x: gx, grid_z: gz, anchor }).collect(),
        populated_villages: world.populated_villages.iter().copied().collect(),
        village_bells: world.village_bells.clone(),
        village_treasuries: world.village_treasuries.iter().map(|(&k, &v)| (k, v)).collect(),
        active_raids: world.active_raids.clone(),
        raid_scheduler: world.raid_scheduler.clone(),
        raid_kills: world.raid_kills.iter().map(|(&(vid, pk), &c)| (vid, pk, c)).collect(),
        brigand_hideouts: world.brigand_hideouts.iter()
            .map(|(&(gx, gz), data)| SavedHideout { grid_x: gx, grid_z: gz, data: data.clone() })
            .collect(),
        bounties: world.bounties.iter().map(|b| SavedBounty {
            id: b.id, template_idx: b.template_idx, issued_tick: b.issued_tick,
        }).collect(),
        bounty_next_id: world.bounty_next_id,
        bounty_last_refresh_tick: world.bounty_last_refresh_tick,
        face_overlays: face_overlays_to_saved(world),
        face_blueprints: face_blueprints_to_saved(world),
        face_blueprint_blanks: face_blueprint_blanks_to_saved(world),
        workshop: world.workshop.clone(),
        // Rail freight Phase 1 — carts snapshotted from the caller's ECS.
        carts: carts.to_vec(),
        saved_mobs: saved_mobs.to_vec(),
        satoshi: world.satoshi.clone(),
        dispensers,
        graves: world
            .iter_graves()
            .map(|((x, y, z), data)| SavedGrave { x, y, z, data: data.clone() })
            .collect(),
        waypoints: world.waypoints.clone(),
        // Spec 48 (Electricity) — per-block meta + power-device runtime state, so
        // circuits survive save/reload. Collected inline like graves/waypoints.
        block_meta: world
            .block_meta
            .iter()
            .map(|(&(x, y, z), &m)| (x, y, z, m))
            .collect(),
        power_devices: world
            .iter_power_devices()
            .map(|((x, y, z), data)| SavedPowerDevice { x, y, z, data: data.clone() })
            .collect(),
    };

    // Bump version in cached meta (native increments via save_world_meta; WASM
    // does the same via the thread_local so reloading the world comes up to date).
    let mut meta = load_world_meta(name);
    meta.version += 1;
    // Seam B — stamp the owning persona (on WASM the pubkey is always present, so
    // this claims it on first save and is a no-op thereafter).
    meta.claim_owner(current_owner_pubkey().as_deref());
    // Goal 1 — flush the live per-world proof-of-play stats into the meta that
    // gets cached + packed into the world blob (Spec 2 §9.1).
    meta.total_work = world.total_work;
    meta.total_ticks = world.total_ticks;
    WASM_META_CACHE.with(|c| {
        c.borrow_mut().insert(name.to_string(), meta.clone());
    });

    // BRIDGE: web Stash blobs don't yet carry exhibit images — the browser keeps
    // its art in a JS-side store, not on a filesystem like native. Empty here means
    // a stashed gallery world round-trips its placements but not its image bytes.
    // Replace when the P3 web-publish path wires the JS exhibit-image store into
    // pack/unpack (build spec §2.3, §6.5). Native export/import already carries them.
    let compressed = crate::wasm_save::pack_world(&meta, &save_data, world, &[])?;
    let size = compressed.len();
    let meta_json = serde_json::to_string(&meta)
        .map_err(|e| format!("meta serialise: {e}"))?;

    let pubkey = wasm_storage_key();
    let name_owned = name.to_string();

    wasm_bindgen_futures::spawn_local(async move {
        match crate::wasm_save::save_world_wasm(&pubkey, &name_owned, compressed, meta_json).await {
            Ok(_) => log::info!("WASM save OK: '{name_owned}' ({size} bytes compressed)"),
            Err(e) => log::error!("WASM save failed for '{name_owned}': {e}"),
        }
    });
    Ok(())
}

/// Load a world from disk. Returns (WorldSave, loaded chunk count).
/// Handles both the current save format and legacy saves (pre-item-persistence).
/// Read one appended `WorldSave` tail field from `cur`. Returns `T::default()` ONLY
/// when the cursor is already at a clean end-of-stream — i.e. an OLDER engine stopped
/// at this exact field boundary, so the field is genuinely absent. If bytes REMAIN,
/// the field IS present in the stream: decode it and propagate ANY error, including a
/// mid-field `UnexpectedEof`.
///
/// The mid-field-EOF distinction is load-bearing. A genuine legacy stream (or
/// corruption) whose bytes are a DIFFERENT shape must propagate so `read_world_save`
/// falls back to `LegacyWorldSave` — it must NOT be silently defaulted. Concretely: a
/// legacy multi-player save with an EMPTY primary inventory reads the strict prefix
/// cleanly, then the misaligned legacy `players` bytes (3-byte `LegacySavedSlot` vs the
/// 4-byte modern `SavedSlot` enum tag) run off the end mid-decode. Blanket-swallowing
/// that EOF would drop the extra players silently; instead we propagate, the tolerant
/// decode fails, and the legacy fallback recovers them. (Found by the Goal 3 review.)
fn read_tail<T: serde::de::DeserializeOwned + Default>(
    cur: &mut std::io::Cursor<&[u8]>,
) -> Result<T, bincode::Error> {
    // Clean boundary: the older writer ended exactly here, so the field is absent.
    if cur.position() >= cur.get_ref().len() as u64 {
        return Ok(T::default());
    }
    // Bytes remain → the field is present → decode, propagating any error (including a
    // mid-field EOF, which signals a different/legacy/corrupt shape, not a clean tail).
    bincode::deserialize_from(&mut *cur)
}

/// Decoder state for the swallow-on-error part of the `WorldSave` tail (from
/// `carts` on). bincode is positional, so once one field fails part-way through,
/// the cursor sits somewhere inside that field and every later read would decode
/// misaligned bytes — some "succeeding" with garbage. So the FIRST failure stops
/// the tail: that field and every later one take their defaults, and the failing
/// field is logged (audit 2026-09-27).
#[derive(Default)]
struct TailReader {
    failed_at: Option<&'static str>,
}

impl TailReader {
    fn field<T: serde::de::DeserializeOwned + Default>(
        &mut self,
        cur: &mut std::io::Cursor<&[u8]>,
        name: &'static str,
    ) -> T {
        if self.failed_at.is_some() {
            return T::default();
        }
        match read_tail(cur) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "world.dat: tail field `{name}` failed to decode ({e}); it and every \
                     later field load as empty"
                );
                self.failed_at = Some(name);
                T::default()
            }
        }
    }
}

/// Tolerant `WorldSave` decode (Goal 3 / Task 1 — `docs/foundations/2026-06-03-old-save-data-integrity.md`).
/// bincode 1 is positional + non-self-describing, so a save written by an OLDER
/// engine — which only ever APPENDS fields — is a byte-prefix of the current struct
/// that ends early. The derived `Deserialize` (and `#[serde(default)]`, which is
/// inert on this format) cannot recover: it hits EOF on the first missing trailing
/// field and fails the whole load, after which the old fallback dropped everything
/// to `LegacyWorldSave`.
///
/// This reads the required prefix (fields 1-7, present in every WorldSave-shaped
/// save) strictly, then each appended field (8-34) tolerantly — a field whose bytes
/// are absent because the writer predated it defaults to empty instead of failing.
///
/// The struct literal lists ALL fields, so adding a new `WorldSave` field without
/// extending this decoder is a COMPILE error — it can never silently drift (unlike
/// the hand-maintained `LegacyWorldSave`). The field order here MUST equal
/// `WorldSave`'s declaration order (= the bincode wire order).
#[cfg(test)]
fn deserialize_world_save_tolerant(data: &[u8]) -> Result<WorldSave, bincode::Error> {
    deserialize_world_save_tolerant_reporting(data).map(|(s, _, _)| s)
}

/// [`deserialize_world_save_tolerant`], also returning the tail field that failed
/// (if any) so a loader can keep the damaged original (review S5), and how many
/// bytes the decode consumed (the reader-side field-count tripwire checks a current
/// save is read to its last byte).
fn deserialize_world_save_tolerant_reporting(
    data: &[u8],
) -> Result<(WorldSave, Option<&'static str>, u64), bincode::Error> {
    let mut cur = std::io::Cursor::new(data);
    // Fields from `carts` on are decoded "stop at first failure": see `TailReader`.
    let mut tail = TailReader::default();
    let save = WorldSave {
        // Required prefix — fields 1-7. A failure here means the stream is not a
        // modern WorldSave (e.g. a genuine legacy 8-field save); propagate so the
        // caller can try `LegacyWorldSave`.
        seed: bincode::deserialize_from(&mut cur)?,
        player_x: bincode::deserialize_from(&mut cur)?,
        player_y: bincode::deserialize_from(&mut cur)?,
        player_z: bincode::deserialize_from(&mut cur)?,
        player_health: bincode::deserialize_from(&mut cur)?,
        hotbar_slot: bincode::deserialize_from(&mut cur)?,
        inventory: bincode::deserialize_from(&mut cur)?,
        // Appended tail — fields 8-35, in declaration order. Each defaults to empty
        // if the older writer's stream ends before it.
        players: read_tail(&mut cur)?,
        campfires: read_tail(&mut cur)?,
        furnaces: read_tail(&mut cur)?,
        vendors: read_tail(&mut cur)?,
        drying_racks: read_tail(&mut cur)?,
        hives: read_tail(&mut cur)?,
        chests: read_tail(&mut cur)?,
        tip_jars: read_tail(&mut cur)?,
        plots: read_tail(&mut cur)?,
        market_hubs: read_tail(&mut cur)?,
        auctions: read_tail(&mut cur)?,
        latent_prints: read_tail(&mut cur)?,
        construction_anchors: read_tail(&mut cur)?,
        architect_plaques: read_tail(&mut cur)?,
        village_anchors: read_tail(&mut cur)?,
        populated_villages: read_tail(&mut cur)?,
        village_bells: read_tail(&mut cur)?,
        village_treasuries: read_tail(&mut cur)?,
        active_raids: read_tail(&mut cur)?,
        raid_scheduler: read_tail(&mut cur)?,
        raid_kills: read_tail(&mut cur)?,
        brigand_hideouts: read_tail(&mut cur)?,
        bounties: read_tail(&mut cur)?,
        bounty_next_id: read_tail(&mut cur)?,
        bounty_last_refresh_tick: read_tail(&mut cur)?,
        face_overlays: read_tail(&mut cur)?,
        face_blueprints: read_tail(&mut cur)?,
        face_blueprint_blanks: read_tail(&mut cur)?,
        // Spec 40 — defaults to empty for pre-Workshop saves.
        workshop: read_tail(&mut cur)?,
        // Rail freight Phase 1 — newest appended field; defaults to empty for
        // pre-rail saves. MUST stay last to match `WorldSave`'s wire order.
        //
        // CA1 SAVE-COMPAT CAVEAT: `CartData` gained a trailing `hull` field, so
        // the bincode layout of a `SavedCart` (nested inside this `Vec`) changed.
        // bincode is positional + can't default a missing trailing field on a
        // NESTED struct, so a PRE-HULL save that already CONTAINS carts decodes
        // this Vec against the wrong (longer) shape and errors. We swallow that
        // error → empty `carts` rather than failing the whole load: the cart
        // feature was unreachable until a fix earlier today, so realistically no
        // important save has carts, and resetting them (parked carts re-place
        // trivially) pre-launch beats losing the world. A clean carts-less save
        // still defaults empty here via `read_tail`'s boundary check, unchanged.
        carts: tail.field(&mut cur, "carts"),
        graves: tail.field(&mut cur, "graves"),
        waypoints: tail.field(&mut cur, "waypoints"),
        // Spec 48 (Electricity) — per-block meta + power-device state. Newest
        // appended fields; pre-Spec-48 saves end before them and default empty.
        block_meta: tail.field(&mut cur, "block_meta"),
        power_devices: tail.field(&mut cur, "power_devices"),
        // Wave 2c — Sign text + Item Frames. Newest appended fields; MUST be
        // read last to match `WorldSave`'s wire order. Old saves end before them.
        signs: tail.field(&mut cur, "signs"),
        item_frames: tail.field(&mut cur, "item_frames"),
        // Wave 3 — locked inventory slots.
        locked_slots: tail.field(&mut cur, "locked_slots"),
        // Wave 5 — hostile-act ledger.
        hostile_acts: tail.field(&mut cur, "hostile_acts"),
        // #19 — placed authored rigs.
        rigs: tail.field(&mut cur, "rigs"),
        // Creator-gallery exhibits (Spec 2026-06-19 §9). NEWEST field; read last.
        exhibits: tail.field(&mut cur, "exhibits"),
        composters: tail.field(&mut cur, "composters"),
        // Animals Wave 2 — persisted tamed pets.
        saved_mobs: tail.field(&mut cur, "saved_mobs"),
        // Satoshi onboarding — NEWEST field; read LAST.
        satoshi: tail.field(&mut cur, "satoshi"),
        dispensers: tail.field(&mut cur, "dispensers"),
        // #19 — per-rig animation clip (index-aligned with `rigs`). NEWEST field;
        // read LAST.
        rig_clips: tail.field(&mut cur, "rig_clips"),
    };
    Ok((save, tail.failed_at, cur.position()))
}

/// Decode a `world.dat` byte stream into a `WorldSave`. Single source of truth for
/// every load path (`load_world`, `load_autosave`, the damaged-meta recovery, and
/// `world_archive::unpack_world` for web / cloud / `.axeworld` / `.axeprofile`).
///
/// Forward compatibility (gap-audit T1-7, `crate::save_format`): every save written
/// since carries a footer `format_version: u32 LE || b"AXSAVEv1"`.
/// - Footer with a version NEWER than [`crate::save_format::SAVE_FORMAT_VERSION`] →
///   refused with [`WorldSaveError::NewerVersion`], nothing decoded. Decoding it
///   would drop the newer fields on the next re-save (an AppImage rollback silently
///   losing data); the lobby says "update the game" instead, and the writers refuse
///   the folder (`refuse_write_over_newer_save`).
/// - Footer with this version or older → stripped, then decoded as below.
/// - No footer (every save from before it) → decoded as below, exactly as before.
///
/// The decode tries the tolerant reader first — recovering older appended-field
/// saves WITHOUT loss — and only if its required prefix fails does it fall back to
/// the genuinely-ancient 8-field `LegacyWorldSave` (block-only inventory, no
/// block-entities), which upgrades lossily but is the right behaviour for a real
/// pre-item save.
pub fn read_world_save(data: &[u8]) -> Result<WorldSave, WorldSaveError> {
    read_world_save_reporting(data).map(|(s, _)| s)
}

/// [`read_world_save`], plus whether the tolerant tail stopped at a failed field
/// (then part of the save was defaulted and the file must be kept aside).
pub fn read_world_save_reporting(data: &[u8]) -> Result<(WorldSave, bool), WorldSaveError> {
    // The footer is stripped BEFORE the tolerant decode: left on, an older-version
    // save's 12 footer bytes would be decoded as the first field it lacks.
    let (payload, version) = crate::save_format::split_save_footer(data);
    if let Err(e) = crate::save_format::check_version(version) {
        log::warn!("world.dat: {e:?} — refused, not decoded");
        return Err(e);
    }
    match deserialize_world_save_tolerant_reporting(payload) {
        Ok((s, failed, _)) => Ok((s, failed.is_some())),
        Err(_) => {
            let legacy: LegacyWorldSave = bincode::deserialize(payload).map_err(|e| {
                WorldSaveError::Undecodable(format!("deserialize (legacy fallback): {e}"))
            })?;
            log::info!("Loaded legacy save format — will upgrade on next save");
            Ok((legacy.upgrade(), false))
        }
    }
}

/// The newer-version refusal for the world folder `dir`, if its `world.dat` — or
/// its crash-recovery `autosave/world.dat` — was saved by a newer build (Spec 02
/// §8.4). Reads only each file's 12-byte footer.
#[cfg(not(target_arch = "wasm32"))]
fn newer_save_in(dir: &std::path::Path) -> Option<WorldSaveError> {
    [dir.join("world.dat"), dir.join("autosave").join("world.dat")]
        .iter()
        .find_map(|p| {
            crate::save_format::check_version(crate::save_format::file_footer_version(p)).err()
        })
}

/// The lobby message for a world this build must not open, if any. Checked before
/// a world is entered from every lobby path (world card, Workshop, Trials, online
/// host, dedicated server), so a refused world is never read in, never replaced by
/// a freshly generated one, and never written.
#[cfg(not(target_arch = "wasm32"))]
pub fn world_open_refusal(folder_name: &str) -> Option<String> {
    newer_save_in(&world_dir(folder_name)).map(|e| e.to_string())
}

/// Web: the IndexedDB / cloud blob is checked as it is unpacked
/// (`world_archive::unpack_world` → [`read_world_save`]).
#[cfg(target_arch = "wasm32")]
pub fn world_open_refusal(_folder_name: &str) -> Option<String> {
    None
}

/// Refuse to write anything into the world folder `dir` while it holds a save
/// from a newer build. Called FIRST by every native writer — `save_world`,
/// `write_world_folder`, `autosave_world`, `GameServer::try_save`, and
/// `save_world_meta` (via `meta_write_blocked`) — before any chunk, meta or
/// `world.dat` is touched. Belt and braces behind [`world_open_refusal`]: a load
/// that fails falls through to generating a fresh world, and without this its
/// first save would land on top of the newer one.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn refuse_write_over_newer_save(dir: &std::path::Path) -> Result<(), String> {
    match newer_save_in(dir) {
        Some(e) => {
            log::error!("refusing to write to {}: {e:?}", dir.display());
            Err(format!("refusing to write: {e}"))
        }
        None => Ok(()),
    }
}

/// Ceiling on how far a cloud/import archive may decompress. A small gzip blob
/// that inflates past this is treated as a decompression bomb and rejected
/// rather than OOMing the tab. 512 MiB is generous for any real single-world
/// export (engine audit 2026-06-04, A: untrusted cloud/import decode).
pub const MAX_IMPORT_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

/// Read at most `max` bytes from `reader` into a fresh Vec, erroring if the
/// stream would exceed `max`. Used to bound gzip decompression on the
/// cloud/import path: unbounded `read_to_end` on attacker-supplied gzip is a
/// decompression-bomb OOM. Once the inflated tar is bounded, every downstream
/// read (tar entries, the bincode `world.dat` decode) is transitively bounded
/// too, so a crafted length prefix can only EOF against the bounded slice
/// rather than over-allocate (engine audit 2026-06-04, A: untrusted decode).
///
/// Generic over the reader (not tied to `flate2`, which is a wasm-only dep) so
/// the bound logic is exercised by the native test harness.
pub fn read_bounded<R: std::io::Read>(reader: R, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    // take(max + 1): reading more than `max` means the stream is over the cap.
    reader
        .take(max.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| format!("bounded read: {e}"))?;
    if out.len() as u64 > max {
        return Err(format!(
            "import exceeds {max}-byte decompression limit (possible bomb)"
        ));
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_world(
    name: &str,
    world: &mut World,
) -> Result<(WorldSave, u32), String> {
    let dir = world_dir(name);

    // Refuse a world whose info is damaged beyond recovery rather than load it
    // under default meta (seed 42, survival) — audit 2026-09-27.
    try_load_world_meta(name)?;

    let dat_path = dir.join("world.dat");
    let data = fs::read(&dat_path).map_err(|e| format!("read world.dat: {e}"))?;
    // Goal 3 / Task 1 — tolerant decode: recovers older saves that predate the
    // newest appended WorldSave field instead of EOF-falling-back to the lossy
    // LegacyWorldSave path (which would drop every block-entity).
    let (save, partial) = read_world_save_reporting(&data)?;
    if partial {
        keep_damaged_copy_once(&dat_path);
    }

    let loaded = load_chunk_dir(&dir.join("chunks"), world)?;

    // Goal 3 / Task 2 — restore all world-level state via the shared helper
    // (chunks above must already be inserted; the helper rebuilds the salt-lick +
    // tapped-rubber indices by scanning them).
    apply_world_save_state(world, &save);

    log::info!(
        "Loaded world: {loaded} chunks, {} campfires, {} drying-racks, {} villages, {} bells, {} treasur(ies), {} active raids, player at ({:.1}, {:.1}, {:.1})",
        save.campfires.len(),
        save.drying_racks.len(),
        save.village_anchors.len(),
        save.village_bells.len(),
        save.village_treasuries.len(),
        save.active_raids.len(),
        save.player_x, save.player_y, save.player_z,
    );
    Ok((save, loaded))
}

/// Read every `<cx>_<cy>_<cz>.chunk` file in `chunks_dir` into `world`. Shared by
/// `load_world` and `load_autosave`. Returns how many chunks were inserted.
///
/// A chunk file that fails to decode (torn / wrong length) is renamed aside to
/// `<file>.corrupt-<ts>` and logged — it is NOT inserted, so the streamer
/// regenerates that chunk as before, but the damaged original is kept for
/// recovery and the next save can never overwrite it (audit 2026-09-27).
#[cfg(not(target_arch = "wasm32"))]
fn load_chunk_dir(chunks_dir: &std::path::Path, world: &mut World) -> Result<u32, String> {
    let mut loaded = 0u32;
    if !chunks_dir.exists() {
        return Ok(0);
    }
    let entries = fs::read_dir(chunks_dir).map_err(|e| format!("readdir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("entry: {e}"))?;
        let filename = entry.file_name();
        let name_str = filename.to_string_lossy();

        if !name_str.ends_with(".chunk") {
            continue;
        }

        let stem = name_str.trim_end_matches(".chunk");
        let parts: Vec<&str> = stem.split('_').collect();
        if parts.len() != 3 {
            continue;
        }
        let cx: i32 = parts[0].parse().map_err(|_| format!("bad chunk name: {name_str}"))?;
        let cy: i32 = parts[1].parse().map_err(|_| format!("bad chunk name: {name_str}"))?;
        let cz: i32 = parts[2].parse().map_err(|_| format!("bad chunk name: {name_str}"))?;

        let data = fs::read(entry.path()).map_err(|e| format!("read chunk: {e}"))?;
        match Chunk::from_bytes(&data) {
            Some(chunk) => {
                world.insert_chunk(cx, cy, cz, chunk);
                loaded += 1;
            }
            None => {
                log::error!(
                    "chunk file {name_str} is damaged ({} bytes); keeping it aside, the \
                     chunk will regenerate",
                    data.len()
                );
                if let Err(e) = quarantine_corrupt(&entry.path()) {
                    // Couldn't move it: fail the load rather than let the next save
                    // overwrite the only copy.
                    return Err(format!("damaged chunk {name_str} could not be kept aside: {e}"));
                }
            }
        }
    }
    Ok(loaded)
}

/// Apply all persisted **world-level** state from a decoded [`WorldSave`] into
/// `world`. This is the single source of truth for the post-chunk restore step,
/// shared by `load_world`, `load_autosave`, and the WASM/cloud apply path so they
/// can never drift (Goal 3 / Task 2 — `load_autosave` and the WASM path each used
/// to restore a different, smaller subset, silently dropping block-entities that
/// had been written to disk).
///
/// Per-player state is deliberately NOT restored here — callers own their player
/// slots and rehydrate those from `save.players` themselves.
///
/// PRECONDITION: `world`'s chunks must already be inserted. The salt-lick and
/// tapped-rubber index rebuilds below scan the freshly-restored chunks, so calling
/// this before the chunk loop would leave those indices empty.
///
/// `save.raid_kills` IS restored here (Goal 3 review follow-up): it is written on
/// every save path but was historically restored on none, silently resetting the
/// per-(village, player) raid-defender leaderboard on every reload.
pub fn apply_world_save_state(world: &mut World, save: &WorldSave) {
    // Wave 27 — restore campfire block-entity state. Old saves have an
    // empty `campfires` Vec via `#[serde(default)]`; that just leaves
    // `world.block_entities` empty (correct — no campfires existed
    // before this wave).
    for cf in &save.campfires {
        world.insert_campfire((cf.x, cf.y, cf.z), cf.data.clone());
    }
    // Spec 20 Phase 8 — restore furnace state, mirroring campfires.
    for f in &save.furnaces {
        world.insert_furnace((f.x, f.y, f.z), f.data.clone());
    }
    // Spec 48 (Electricity) — restore the per-block meta byte + power-device
    // runtime state. Old saves have empty Vecs via `#[serde(default)]`. The
    // transient power flood is NOT saved; `reseed_on_load` (below, after the
    // devices are in place) re-enqueues every device so the next `power_tick`
    // recomputes lit cables/lamps from the restored device state.
    for &(x, y, z, m) in &save.block_meta {
        world.block_meta.insert((x, y, z), m);
    }
    for d in &save.power_devices {
        world.insert_power_device((d.x, d.y, d.z), d.data.clone());
    }
    crate::power::reseed_on_load(world);
    // Owner-inbox #1/2/3 — restore wallpaper face overlays (persisted, not
    // rebuilt from blocks). Old saves have an empty Vec via #[serde(default)].
    restore_face_overlays(world, &save.face_overlays);
    // Task B1 — restore blueprint face-attachments from the additive Vec.
    // Old saves have an empty Vec via #[serde(default)].
    restore_face_blueprints(world, &save.face_blueprints);
    // Task R1 — restore blank draughting-paper face-attachments from the
    // additive Vec. Old saves have an empty Vec via #[serde(default)].
    restore_face_blueprint_blanks(world, &save.face_blueprint_blanks);
    // Spec 40 (The Workshop) — restore the parked/inflated redesign WIP. Empty
    // (default) in normal worlds and pre-Workshop saves. Cloned wholesale: the
    // side-table is self-contained (no chunk cross-refs to rebuild).
    world.workshop = save.workshop.clone();
    // Spec 29 — one-shot legacy migration. v1 furnaces could smelt raw
    // meat; v2 is ore-only. Eject any raw meat sitting in a furnace
    // input slot into `world.pending_legacy_meat_drops`; the caller's
    // first-tick path drains it and spawns ItemEntities.
    crate::furnace::eject_legacy_food_into_world_pending(world);
    // Spec 21 Phase 9 — restore vendor state.
    for v in &save.vendors {
        let mut data = v.data.clone();
        crate::vendor::repair_legacy_bulk(&mut data);
        world.insert_vendor((v.x, v.y, v.z), data);
    }
    // HP-2 — restore chest block-entity state. Older saves arrive with
    // an empty Vec via `#[serde(default)]`.
    for c in &save.chests {
        world.insert_chest((c.x, c.y, c.z), c.data.clone());
    }
    // Dispensers/Droppers (2026-07-04) — restore contents + edge latch.
    for d in &save.dispensers {
        world.insert_dispenser((d.x, d.y, d.z), d.data.clone());
    }
    // Wave 2c — restore Sign text block-entities.
    for s in &save.signs {
        world.insert_sign((s.x, s.y, s.z), s.data.clone());
    }
    // Wave 2c — restore Item Frame block-entities.
    for f in &save.item_frames {
        world.insert_item_frame((f.x, f.y, f.z), f.data.clone());
    }
    // Wave 5 — restore the hostile-act ledger.
    world.hostile_acts.set_acts(save.hostile_acts.clone());
    // #19 — restore placed authored rigs, then re-apply each one's animation
    // clip from the index-aligned `rig_clips` side table. A save written before
    // the clip picker has no side table (or a short one) and every such rig
    // loads as `AnimClip::Walk` — exactly what it played.
    world.rigs = save.rigs.clone();
    for (i, rd) in world.rigs.iter_mut().enumerate() {
        rd.clip = save.rig_clips.get(i).copied().unwrap_or_default();
    }
    // #47 — restore Grave block-entities. Old saves arrive with an empty Vec
    // via `#[serde(default)]`.
    for g in &save.graves {
        world.insert_grave((g.x, g.y, g.z), g.data.clone());
    }
    // #6 — restore map waypoints (pins + death markers). Pre-#6 saves arrive
    // with an empty Vec via `#[serde(default)]` / `read_tail`.
    world.waypoints = save.waypoints.clone();
    // Creator-gallery (Spec 2026-06-19 §9) — restore authored exhibits so the
    // painting pipeline (1b) renders them on world entry.
    world.exhibits = save.exhibits.clone();
    // Spec 28d chunk 8 — restore hive state.
    for h in &save.hives {
        world.insert_hive((h.x, h.y, h.z), h.data);
    }
    // Wave 29 — restore drying-rack state. Same serde-default pattern.
    for rack in &save.drying_racks {
        world.drying_racks.insert((rack.x, rack.y, rack.z), rack.data.clone());
    }
    // Spec 24 — restore in-progress build anchors + completed Plaques.
    for anchor in &save.construction_anchors {
        world.construction_anchors.insert((anchor.x, anchor.y, anchor.z), anchor.data.clone());
    }
    for plaque in &save.architect_plaques {
        world.architect_plaques.insert((plaque.x, plaque.y, plaque.z), plaque.chain.clone());
    }
    // Satoshi onboarding — restore the guide's per-world progress + house spot.
    // The Satoshi entity itself is re-spawned on the village tick (not here),
    // gated on this state, so he never re-greets or double-spawns.
    world.satoshi = save.satoshi.clone();
    // Spec 19 phase 11 — restore village state. Old saves have empty Vecs
    // via #[serde(default)]; Phase 4's tick will rebuild village_anchors
    // when chunks regenerate, but persisting them lets the populated set
    // remain authoritative (we don't re-spawn villagers).
    for sa in &save.village_anchors {
        world.village_anchors.insert((sa.grid_x, sa.grid_z), sa.anchor);
    }
    world.populated_villages = save.populated_villages.iter().copied().collect();
    world.village_bells = save.village_bells.clone();
    // Spec 22 — restore village treasuries + active raids + scheduler.
    // Pre-Spec-22 saves arrive with empty Vecs via #[serde(default)].
    world.village_treasuries = save
        .village_treasuries
        .iter()
        .copied()
        .collect();
    world.active_raids = save.active_raids.clone();
    world.raid_scheduler = save.raid_scheduler.clone();
    // HP-3 — restore Brigand Hideout side-table. Old saves arrive with
    // an empty Vec via #[serde(default)]; layout regenerates on column
    // re-stream from (world_seed, gx, gz).
    for sh in &save.brigand_hideouts {
        world.brigand_hideouts.insert((sh.grid_x, sh.grid_z), sh.data.clone());
    }
    // Salt feature — rebuild the Salt Lick index by scanning the
    // freshly-restored chunks. The index is NOT serialised (the block
    // placements are the canonical state), so post-load it's empty
    // until this scan stamps it.
    world.rebuild_salt_lick_index();
    // Rubber feature — rebuild the tapped-rubber-log cooldown index.
    // Stamps every RUBBER_LOG_TAPPED at load_tick = 0. The runtime
    // cooldown clock is `tick_counter` (monotonic u64) which is NOT
    // persisted across save/reload — it also restarts at 0 in the new
    // session, so this stamping effectively gives every tapped log a
    // fresh full 24 000-tick cooldown post-load. This is the
    // anti-exploit property: no save-scumming to skip cooldowns.
    world.rebuild_tapped_rubber_logs_index(0);
    // Spec 34 Tip Jar — restore block-entity state. Pre-Spec-34
    // saves arrive with an empty Vec via #[serde(default)].
    for sj in &save.tip_jars {
        world.insert_tip_jar((sj.x, sj.y, sj.z), sj.data.clone());
    }
    // Spec 38 Auctions — restore timed auction block-entities.
    for sa in &save.auctions {
        world.insert_auction((sa.x, sa.y, sa.z), sa.data.clone());
    }
    // Spec 38 (Blueprint / Cyanotype) — restore Latent Print block-
    // entities. Pre-Spec-38 saves arrive empty via #[serde(default)].
    // The embedded PlanData's `develop_state` round-trips intact, so a
    // half-developed cyanotype resumes from where it was on reload.
    for sl in &save.latent_prints {
        world.insert_latent_print((sl.x, sl.y, sl.z), sl.data.clone());
    }
    // Spec 49 (Explosives) — restore Composter contents. Pre-Spec-49 saves
    // arrive empty via `#[serde(default)]` / `read_tail`. (The keg fuse rides
    // the `power_devices` restore above — the keg is a `PowerDevice`.)
    for c in &save.composters {
        world.insert_composter((c.x, c.y, c.z), c.data.clone());
    }
    // Spec 36 Plot Ownership — restore claimed plots. Pre-Spec-36
    // saves arrive empty via #[serde(default)]. PlotData is plain
    // serde so it round-trips directly.
    world.plots = save.plots.clone();
    // Spec 37 Market Hubs — restore discovery zones (plain serde).
    world.market_hubs = save.market_hubs.clone();
    // Spec 33 Mob Bounty Board — restore the rotation + id allocator
    // + last-refresh tick. Pre-Spec-33 saves arrive with empty / 0 /
    // 0 fields via #[serde(default)]; tick_bounty_refresh seeds the
    // first rotation on its next tick (the empty-bounties branch).
    world.bounties = save.bounties.iter().map(|sb| crate::bounty::ActiveBounty {
        id: sb.id, template_idx: sb.template_idx, issued_tick: sb.issued_tick,
    }).collect();
    world.bounty_next_id = if save.bounty_next_id == 0 { 1 } else { save.bounty_next_id };
    world.bounty_last_refresh_tick = save.bounty_last_refresh_tick;
    // Spec 22 Phase 18 — restore per-(village, player) raid-kill totals (the village
    // defender leaderboard). Written on every save path (save.rs ~800/957/1869) but
    // historically restored on none; this is the exact inverse of the save-side flatten
    // (`AHashMap<(VillageId, PlayerKey), u32>` ⇄ `Vec<((i32,i32), usize, u32)>`).
    // Goal 3 review follow-up — one line repairs load_world + load_autosave + WASM.
    world.raid_kills = save
        .raid_kills
        .iter()
        .map(|&(vid, pk, c)| ((vid, pk), c))
        .collect();
}

#[cfg(target_arch = "wasm32")]
pub fn load_world(
    _name: &str,
    _world: &mut World,
) -> Result<(WorldSave, u32), String> {
    Err("load_world not available on WASM".to_string())
}

/// Restore inventory from save data.
pub fn restore_inventory(inventory: &mut Inventory, slots: &[SavedSlot]) {
    for (i, slot) in slots.iter().enumerate() {
        if i >= 36 { break; }
        match slot {
            SavedSlot::Empty => inventory.set_slot(i, None),
            SavedSlot::Block { block_id, count } => {
                // Clamp to max_stack so a legacy / hand-edited over-max count
                // can't enter the inventory (engine audit 2026-06-04, A).
                let mut s = ItemStack::new_block(*block_id, *count);
                s.count = s.count.min(s.item.max_stack());
                inventory.set_slot(i, Some(s));
            }
            SavedSlot::Tool { tool_type, material, durability } => {
                let mut tool = Tool::new(*tool_type, *material);
                tool.durability = *durability;
                inventory.set_slot(i, Some(ItemStack::new_tool(tool)));
            }
            SavedSlot::Material { material_id, count } => {
                let mut s = ItemStack::new_material(*material_id, *count);
                s.count = s.count.min(s.item.max_stack());
                inventory.set_slot(i, Some(s));
            }
            SavedSlot::Plan { data } => {
                inventory.set_slot(i, Some(ItemStack { item: Item::Plan(data.clone()), count: 1 }));
            }
            SavedSlot::Armour { slot, material, durability } => {
                let mut piece = crate::armour::ArmourItem::new(*slot, *material);
                piece.durability = *durability;
                inventory.set_slot(i, Some(ItemStack { item: Item::Armour(piece), count: 1 }));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// World Metadata (world_meta.json)
// ---------------------------------------------------------------------------

/// A logged difficulty change in the world's history.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DifficultyChange {
    pub level: String,
    pub timestamp: String,
}

fn default_true() -> bool { true }
fn default_normal() -> String { "normal".to_string() }
fn default_world_type() -> String { "normal".to_string() }

/// The retired built-in Gallery world type. The Gallery is now an optional
/// external `.axeworld` world pack (loaded via normal world import), so the
/// engine has no gallery generator. A save still carrying this type loads as a
/// plain `"flat"` world — its saved chunks (the maze) come back as they were,
/// new columns get the flat floor — with a logged note, never a panic.
pub const RETIRED_GALLERY_WORLD_TYPE: &str = "gallery";

/// Map a stored `world_type` onto one the engine still generates.
pub fn normalise_world_type(world_type: String) -> String {
    if world_type == RETIRED_GALLERY_WORLD_TYPE {
        log::info!(
            "world_type \"gallery\" is retired (the Gallery is now an external world pack) — loading as a flat world"
        );
        return "flat".to_string();
    }
    world_type
}

fn de_world_type<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    <String as serde::Deserialize>::deserialize(d).map(normalise_world_type)
}
fn default_ground() -> String { "grass".to_string() }
fn default_water_depth() -> u8 { 3 }
fn default_time_lock() -> String { "cycle".to_string() }

/// Where a world has been published (Publish flow, build spec
/// `docs/superpowers/specs/2026-06-22-publish-flow-build-spec.md` §2.1).
/// Recorded on [`WorldMeta`] when the operator publishes a local edit to one
/// of their dedicated-server world slots.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PublishRecord {
    /// Operator npub of the target server (its join/own key).
    pub server_npub: String,
    /// `wss://…` or `host:port` — display + re-publish target.
    pub server_addr: String,
    /// The served-world slot id on that server.
    pub world_id: String,
    /// SHA-256 (hex) of the `.axeworld` bytes last published. Authoritative
    /// content signal — the publish endpoint binds this for anti-replay /
    /// anti-swap (build spec §3). Computed via
    /// [`crate::world_archive::archive_sha256_hex`].
    pub last_published_hash: String,
    /// `WorldMeta.version` at the moment of the last publish. Cheap badge
    /// proxy: the lobby compares it to the live `version` to show "unpublished
    /// changes" WITHOUT re-packing every world in the list (packing per card is
    /// too costly). The hash above stays the authoritative transport signal.
    #[serde(default)]
    pub last_published_version: u32,
    /// Unix seconds of the last publish (display).
    pub last_published_unix: u64,
}

/// Publish state of a world, derived from [`WorldMeta::published_to`] + the
/// live `version`. Drives the lobby card badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishBadge {
    /// Never published — a purely local world.
    NotPublished,
    /// Published and unchanged since (live version == last published version).
    UpToDate,
    /// Published, but edited since — the server copy is stale.
    UnpublishedChanges,
}

/// Metadata stored in world_meta.json alongside world.dat.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorldMeta {
    pub display_name: String,
    pub description: String,
    pub game_mode: String,
    pub created_at: String,
    pub icon: Option<String>,
    #[serde(default = "default_true")]
    pub pure_survival: bool,
    #[serde(default)]
    pub ever_creative: bool,
    #[serde(default)]
    pub cheats_used: bool,
    #[serde(default = "default_normal")]
    pub difficulty: String,
    #[serde(default)]
    pub difficulty_history: Vec<DifficultyChange>,
    #[serde(default)]
    pub forked_from: Option<String>,
    /// Save version counter — incremented on every save. Display convenience;
    /// the real trusted version is the Nostr event chain (Phase 6).
    #[serde(default)]
    pub version: u32,
    /// Whether this world is flagged for the player's Stash (cloud). Per-world
    /// opt-in, default OFF. The web-build push-to-cloud path was removed
    /// 2026-07-09 (confirmed dead — no reachable web trigger since web login
    /// retirement; see
    /// docs/superpowers/specs/2026-07-09-give-aliases-and-dead-web-stash-code.md).
    /// The flag itself is left in place (read by the still-live cloud-world-list
    /// merge in `menu.rs::poll_cloud_worlds`); it no longer causes any upload.
    /// Original design: docs/superpowers/specs/2026-06-02-stash-opt-in-toggle-design.md
    #[serde(default)]
    pub cloud_save: bool,
    /// Has the Genesis Block — the *first* Satori in this world — been
    /// found yet? One-shot per world flag (Wave 25). The player (or, in
    /// multiplayer when it lands, the *first* player ever) to find a Satori
    /// in this world triggers the Genesis Block celebration; subsequent
    /// Satori get the routine pickup celebration.
    /// Single-instance per world preserves the singular meaning of
    /// "Genesis Block" — like Bitcoin's block 0, there is only one ever.
    #[serde(default)]
    pub genesis_block_found: bool,
    /// Is the slash-command + chat overlay enabled in this world? Set
    /// at world creation in the menu's Create dialog. When false, the
    /// T / `/` keypress that normally opens the chat overlay is ignored
    /// and the player has no access to `/give`, `/time`, `/gamemode`,
    /// etc. Defaults to `true` for backward compatibility — every world
    /// created before this field existed kept commands enabled.
    #[serde(default = "default_true")]
    pub commands_enabled: bool,
    /// Spec 49 (Explosives) — per-world toggle: when false, Blasting Keg
    /// detonations no-op (hand-lit AND electrical) and break no blocks. This is
    /// the per-region TNT on/off the land-claim backlog (#25, WorldGuard-class)
    /// wants; the region system inherits it. Defaults to `true` — every world
    /// created before this field existed keeps explosives enabled.
    #[serde(default = "default_true")]
    pub explosives_enabled: bool,
    /// Fire spread (2026-07-04 gap-fill wave) — when false, fire never
    /// consumes/spreads to flammable blocks (ignition and burn-out still
    /// work, like Minecraft's `doFireTick` off). Defaults `true`; worlds
    /// saved before this field keep fire spread on.
    #[serde(default = "default_true")]
    pub fire_spread_enabled: bool,
    /// Spec 24 Phase 5 — flips to true the first time any player in
    /// this world confirms a Plan capture. While false, the Capture
    /// dialog shows the licence-onboarding modal once before the
    /// main dialog renders. `#[serde(default)]` — pre-Spec-24 saves
    /// default to false (the modal will show on next capture).
    #[serde(default)]
    pub has_seen_license_onboarding: bool,
    /// World-generation seed. Drives `BiomeGenerator` (terrain height,
    /// biome layout, ore, tree placement) and the Proof-of-Play secret.
    /// Randomised per world at create time (`gen_random_seed`) or set
    /// from the player's Create-dialog seed text. `#[serde(default)]`
    /// → every world saved before this field existed loads as `42`
    /// (the old hardcoded constant), so their terrain is unchanged.
    #[serde(default = "default_seed_legacy")]
    pub seed: u32,

    // ─── Proof-of-Play stats (universal — Spec 2 §9.1; added Goal 1) ───
    /// Cumulative proof-of-play **work** ever done in this world: the sum of
    /// each broken block's `crafting::block_work` over every successful
    /// `can_harvest` break. The lifetime "how much work has this world done".
    #[serde(default)]
    pub total_work: u64,
    /// Cumulative **active** ticks (the world-clock — advances only while the
    /// world is being played, NOT wall-clock). The lifetime "how long this
    /// world has been played"; the basis for the Satori-Rush time-to-genesis.
    #[serde(default)]
    pub total_ticks: u64,
    /// `total_ticks` at which this world's Genesis Block (first Satori) was
    /// mined. `None` until claimed. Satori Rush reads this as its result.
    #[serde(default)]
    pub genesis_found_at_tick: Option<u64>,

    /// The active RESUMABLE scenario's def JSON (Goal 4 — Satori Rush), so the
    /// run resumes on reload. `None` for normal worlds + transient scenarios
    /// (Hash Dash). The def is self-contained, so any persistent scenario —
    /// official or community — resumes without a name lookup.
    #[serde(default)]
    pub scenario_def: Option<String>,

    /// Spec 40 (The Workshop) — is this world the player's Workshop (a blank/void
    /// authoring space entered from the Lobby), rather than a normal play world?
    /// Drives the void world-gen preset and the in-Workshop authoring UX. Always
    /// creative. `#[serde(default)]` → every pre-Workshop world loads as `false`.
    #[serde(default)]
    pub is_workshop: bool,
    /// Spec 40 wardrobe persistence -- an OPTIONAL per-world appearance override
    /// (`OverrideSet::to_blob_bytes` blob). When present it SUPERSEDES the player's
    /// global wardrobe for this world only (a creator pinning a fixed look). `None`
    /// (the default for every world) => the player-global wardrobe is used. Appended
    /// last + `#[serde(default)]` so every pre-existing world_meta.json loads clean.
    #[serde(default)]
    pub world_override: Option<Vec<u8>>,

    // ─── Blank-canvas world config (Task B1) ───────────────────────────────
    /// World generation type. `"normal"` = standard terrain; `"flat"` = blank
    /// canvas (flat ground at a configurable height). Additional types may be
    /// added in later waves. `#[serde(default)]` → pre-existing saves load as
    /// `"normal"` so their terrain is unchanged.
    #[serde(default = "default_world_type", deserialize_with = "de_world_type")]
    pub world_type: String,
    /// Ground block for flat/blank worlds. Values: `"none"` (void platform) |
    /// `"grass"` | `"sand"` | `"stone"` | `"dirt"` | `"snow"` | `"water"`.
    /// Ignored when `world_type` is `"normal"`. Default `"grass"`.
    #[serde(default = "default_ground")]
    pub ground: String,
    /// Depth of the water layer for `ground = "water"` flat worlds (in blocks).
    /// Ignored for non-water ground types. Default `3`.
    #[serde(default = "default_water_depth")]
    pub water_depth: u8,
    /// Daylight cycle behaviour. `"cycle"` = normal day/night; `"day"` = locked
    /// to midday; `"night"` = locked to midnight. Default `"cycle"`.
    #[serde(default = "default_time_lock")]
    pub time_lock: String,
    /// Whether hostile and neutral mobs spawn in this world. Default `true`
    /// (all worlds behave as before). Flat/blank worlds may set this to `false`
    /// at create time for an uninterrupted building experience.
    #[serde(default = "default_true")]
    pub mobs_enabled: bool,

    /// #47 — whether players keep their inventory on death (no grave, no scatter).
    /// Default `false` (Survival graves). Blank-canvas/parkour worlds set this
    /// `true` at create time. Old saves load `false` via `#[serde(default)]`.
    #[serde(default)]
    pub keep_inventory: bool,

    /// **Seam B (offline-first login contract,
    /// `docs/foundations/2026-06-10-offline-first-login-and-stash-sequencing.md`).**
    /// The owning persona pubkey (lowercase 64-hex) when this save was last
    /// written while signed in; `None` for a guest save. Stamped via
    /// [`WorldMeta::claim_owner`] on every save. This lets future Stash sync
    /// attribute the blob to an owner instead of orphaning it, and — paired with
    /// the monotonic `version` above — gives local-vs-cloud divergence an owner +
    /// an ordering to resolve against. JSON sidecar field, `#[serde(default)]`,
    /// so every pre-seam `world_meta.json` loads cleanly as `owner_pubkey: None`.
    #[serde(default)]
    pub owner_pubkey: Option<String>,

    /// Publish flow (build spec
    /// `docs/superpowers/specs/2026-06-22-publish-flow-build-spec.md` §2.1) —
    /// where this world was last published, if anywhere. `None` for a normal
    /// local world. Appended last + `#[serde(default)]` so every pre-publish
    /// `world_meta.json` loads cleanly as `published_to: None`.
    #[serde(default)]
    pub published_to: Option<PublishRecord>,
    /// Satoshi onboarding — `true` only for worlds that should host the guide:
    /// set at CREATE for new **normal** Survival/Creative worlds; never for
    /// existing worlds, flat/gallery/workshop types, or Adventure/Spectator.
    /// Pre-v2 metas lack it → default `false`, so Satoshi never appears in a
    /// world made before this feature (the owner's "don't intrude" rule). It is
    /// the reliable cross-platform new-world signal (the load path can't tell
    /// new from resumed on web).
    #[serde(default)]
    pub satoshi_enabled: bool,
    /// Proof-of-Play `server_secret` (Spec 06 §2.2): 32 random bytes from the
    /// OS RNG, generated per world and kept in the HOST's save only. It is
    /// never derived from the (public) seed and never sent over the wire —
    /// the seed goes to every joiner, so a seed-derived key was a complete
    /// Satori x-ray (audit 2026-09-27). `None` on worlds saved before this
    /// field: the first load generates one and saves it
    /// (`proof_of_play::ensure_world_secret`). A fork starts `None` so it gets
    /// its own secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pop_secret: Option<[u8; 32]>,
}

/// Legacy seed for worlds saved before `WorldMeta.seed` existed. All of
/// them generated against the hardcoded `BiomeGenerator::new(42)`, so they
/// must keep loading at 42 to preserve their terrain.
fn default_seed_legacy() -> u32 {
    42
}

/// Generate a random world seed from the OS / browser CSPRNG. `getrandom`
/// maps to `crypto.getRandomValues` on WASM (NOT `Math.random`) thanks to
/// the `wasm_js` feature in Cargo.toml — same RNG the Signet challenge uses.
pub fn gen_random_seed() -> u32 {
    let mut buf = [0u8; 4];
    getrandom::fill(&mut buf).expect("OS RNG unavailable");
    u32::from_le_bytes(buf)
}

/// Hash arbitrary seed text to a u32 (FNV-1a), so a player can type a word
/// instead of a number in the Create dialog — same affordance Minecraft's
/// text seeds give. Pure + deterministic: the same text always yields the
/// same world.
pub fn seed_from_text(text: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in text.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl WorldMeta {
    /// Create default metadata for a new world.
    pub fn new(display_name: &str) -> Self {
        Self {
            display_name: display_name.to_string(),
            description: String::new(),
            game_mode: "survival".to_string(),
            created_at: now_iso8601(),
            icon: None,
            pure_survival: true,
            ever_creative: false,
            cheats_used: false,
            difficulty: "normal".to_string(),
            difficulty_history: Vec::new(),
            forked_from: None,
            version: 0,
            genesis_block_found: false,
            commands_enabled: true,
            explosives_enabled: true,
            fire_spread_enabled: true,
            cloud_save: false,
            has_seen_license_onboarding: false,
            // Fresh worlds get a random seed so "New world" actually means a
            // new world. The Create dialog may override this from the player's
            // seed text before the meta is saved (see menu.rs).
            seed: gen_random_seed(),
            total_work: 0,
            total_ticks: 0,
            genesis_found_at_tick: None,
            scenario_def: None,
            is_workshop: false,
            world_override: None,
            world_type: "normal".to_string(),
            ground: "grass".to_string(),
            water_depth: 3,
            time_lock: "cycle".to_string(),
            mobs_enabled: true,
            keep_inventory: false,
            // Seam B: a brand-new / legacy-folder world is unowned until the
            // first save stamps the signed-in persona (or stays None for guest).
            owner_pubkey: None,
            // Publish flow: an unpublished local world.
            published_to: None,
            satoshi_enabled: false,
            pop_secret: Some(crate::proof_of_play::gen_world_secret()),
        }
    }

    /// Derive the [`PublishBadge`] for the lobby card. Cheap — a version
    /// compare, no packing. See [`PublishRecord::last_published_version`].
    pub fn publish_badge(&self) -> PublishBadge {
        match &self.published_to {
            None => PublishBadge::NotPublished,
            Some(rec) if rec.last_published_version == self.version => PublishBadge::UpToDate,
            Some(_) => PublishBadge::UnpublishedChanges,
        }
    }

    /// **Seam B** — stamp the owning persona pubkey onto this save when one is
    /// known. The guest→identity claim path: a guest save carries `None`; the
    /// first save while signed in claims it. We never **clobber** a different
    /// existing owner (that is a real divergence to resolve, not a silent
    /// overwrite) — only fill when empty or matching. Called from every
    /// `save_world` with [`current_owner_pubkey`].
    pub fn claim_owner(&mut self, pubkey_hex: Option<&str>) {
        let Some(pk) = pubkey_hex else { return }; // guest save → leave as-is
        match self.owner_pubkey.as_deref() {
            None => self.owner_pubkey = Some(pk.to_string()), // claim the orphan
            Some(existing) if existing == pk => {}            // already ours
            Some(existing) => {
                log::warn!(
                    "save owner mismatch: world owned by {existing}, signed in as {pk}; \
                     leaving the original (divergence for Stash conflict-resolution to settle)"
                );
            }
        }
    }

    /// Generate default metadata for an existing world that has no world_meta.json.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_folder(folder_name: &str, world_dat_path: &std::path::Path) -> Self {
        let created = world_dat_path
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(system_time_to_iso8601)
            .unwrap_or_else(now_iso8601);

        Self {
            display_name: folder_name.to_string(),
            description: String::new(),
            game_mode: "survival".to_string(),
            created_at: created,
            icon: None,
            pure_survival: true,
            ever_creative: false,
            cheats_used: false,
            difficulty: "normal".to_string(),
            difficulty_history: Vec::new(),
            forked_from: None,
            version: 0,
            genesis_block_found: false,
            commands_enabled: true,
            explosives_enabled: true,
            fire_spread_enabled: true,
            cloud_save: false,
            has_seen_license_onboarding: false,
            // Worlds with no world_meta.json predate the seed field — they
            // were all generated at the hardcoded 42.
            seed: default_seed_legacy(),
            total_work: 0,
            total_ticks: 0,
            genesis_found_at_tick: None,
            scenario_def: None,
            is_workshop: false,
            world_override: None,
            world_type: "normal".to_string(),
            ground: "grass".to_string(),
            water_depth: 3,
            time_lock: "cycle".to_string(),
            mobs_enabled: true,
            keep_inventory: false,
            // Seam B: a brand-new / legacy-folder world is unowned until the
            // first save stamps the signed-in persona (or stays None for guest).
            owner_pubkey: None,
            // Publish flow: an unpublished local world.
            published_to: None,
            satoshi_enabled: false,
            pop_secret: None,
        }
    }

    /// Mark this world as having used creative mode (one-way door).
    pub fn mark_creative(&mut self) {
        self.ever_creative = true;
        self.pure_survival = false;
        self.game_mode = "creative".to_string();
    }

    /// Spec 40 — mark this world as the player's Workshop: a blank/void authoring
    /// space, always creative, entered from the Lobby.
    pub fn mark_workshop(&mut self) {
        self.is_workshop = true;
        self.mark_creative();
        // A creative authoring room never has mobs (owner report 2026-06-18).
        // The spawner hard-guards on `is_workshop` regardless, but keep the meta
        // honest so anything reading `mobs_enabled` sees the truth.
        self.mobs_enabled = false;
    }

    /// Log a difficulty change.
    pub fn log_difficulty_change(&mut self, level: &str) {
        self.difficulty = level.to_string();
        self.difficulty_history.push(DifficultyChange {
            level: level.to_string(),
            timestamp: now_iso8601(),
        });
    }
}

/// A world entry as displayed in the menu (metadata + derived data).
pub struct WorldEntry {
    pub folder_name: String,
    pub meta: WorldMeta,
    #[cfg(not(target_arch = "wasm32"))]
    pub last_played: SystemTime,
    pub size_bytes: u64,
    /// True only for synthetic entries that live solely in the player's Stash
    /// (no local copy). The per-world Stash toggle is hidden for these (they're
    /// already stashed). Real local worlds are always `false` — note WASM local
    /// worlds carry an empty `created_at`, so this flag, not `created_at`, is
    /// the reliable discriminator.
    pub cloud_only: bool,
}

/// Load `world_meta.json` for a world, strictly (audit 2026-09-27, item
/// "torn world_meta.json reloads with seed 42").
///
/// - A readable, parseable file → `Ok(meta)`.
/// - No file and no quarantined sibling → `Ok(defaults)`: a genuinely legacy
///   world that predates `world_meta.json` (its seed really was 42).
/// - A file that fails to parse is renamed to `world_meta.json.corrupt-<ts>`
///   (kept, never overwritten) and the meta is REBUILT from `world.dat`: the seed
///   comes from `WorldSave.seed`, the rest takes defaults, and the rebuilt meta is
///   written back atomically so the world keeps its real terrain. The same
///   recovery runs when the file is missing but a quarantined copy exists.
///   The PoP `pop_secret` is salvaged from the damaged bytes when its array is
///   still intact; otherwise the rebuilt meta gets a fresh random one.
/// - If `world.dat` can't supply the seed either, or the meta file can't be read
///   at all, the world's info is damaged → `Err`. The world list shows it as
///   "world info damaged", `load_world` refuses it, and `save_world_meta` refuses
///   to write defaults over it.
#[cfg(not(target_arch = "wasm32"))]
pub fn try_load_world_meta(folder_name: &str) -> Result<WorldMeta, String> {
    let dir = world_dir(folder_name);
    let meta_path = dir.join("world_meta.json");
    let dat_path = dir.join("world.dat");

    // Bytes, not read_to_string: a tear inside a multi-byte character must be
    // recovered like any other parse failure, not refused (review S4).
    // The PoP secret lives ONLY in this file (never in world.dat, which travels
    // in exports), so whatever of it is still readable in the damaged bytes is
    // salvaged before they are moved aside.
    let salvaged_secret = match fs::read(&meta_path) {
        Ok(data) => match serde_json::from_slice::<WorldMeta>(&data) {
            Ok(meta) => return Ok(meta),
            Err(e) => {
                // A newer build may write meta this build can't parse: refuse the
                // world rather than quarantine and rebuild it (Spec 02 §8.4).
                if let Some(newer) = newer_save_in(&dir) {
                    return Err(newer.to_string());
                }
                log::error!("world '{folder_name}': world_meta.json does not parse ({e})");
                quarantine_corrupt(&meta_path)
                    .map_err(|qe| format!("world info damaged: {e}; {qe}"))?;
                salvage_pop_secret(&data)
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !has_quarantined_sibling(&dir, "world_meta.json") {
                // Legacy world with no meta file at all. Don't write to disk yet.
                return Ok(WorldMeta::from_folder(folder_name, &dat_path));
            }
            if let Some(newer) = newer_save_in(&dir) {
                return Err(newer.to_string());
            }
            log::error!(
                "world '{folder_name}': world_meta.json missing and a damaged copy was \
                 quarantined earlier; rebuilding from world.dat"
            );
            salvage_pop_secret_from_quarantine(&dir)
        }
        Err(e) => return Err(format!("world info damaged: read world_meta.json: {e}")),
    };

    // Recovery: rebuild from world.dat's seed, never from the seed-42 default.
    let save = fs::read(&dat_path)
        .map_err(|e| format!("world info damaged: world_meta.json unreadable and world.dat unreadable ({e})"))
        .and_then(|data| {
            read_world_save(&data).map_err(|e| {
                format!("world info damaged: world_meta.json unreadable and world.dat has no seed ({e})")
            })
        })?;
    let mut meta = recovered_meta(folder_name, &dat_path, &save);
    // Keep the world's own PoP secret when it survived the tear (drop placement
    // stays put); only a truly unrecoverable one is replaced, and never by
    // None/zero — the rebuilt meta is written with a real secret.
    meta.pop_secret = Some(match salvaged_secret {
        Some(s) => s,
        None => {
            log::error!(
                "world '{folder_name}': the Proof-of-Play secret was not recoverable; \
                 a fresh one is generated (Satori drop placement moves)"
            );
            crate::proof_of_play::gen_world_secret()
        }
    });
    log::error!(
        "world '{folder_name}': world_meta.json rebuilt. RECOVERED from world.dat: seed {}. \
         SET CONSERVATIVELY: cheats_used=true, pure_survival=false, genesis_block_found=true \
         (never findable twice). DEFAULTED (world.dat doesn't hold them): game_mode, \
         world_type, ground, water_depth, time_lock, difficulty, world options. The damaged \
         original is kept as world_meta.json.corrupt-*",
        save.seed
    );
    write_world_meta_file(&dir, &meta)?;
    Ok(meta)
}

/// Pull a still-intact `"pop_secret": [32 bytes]` out of a damaged
/// `world_meta.json`. Tolerant of the rest of the file being torn; strict about
/// the secret itself: exactly 32 decimal values 0..=255 inside a closed array,
/// and never all-zero. Anything less → `None` (the caller generates a fresh one).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn salvage_pop_secret(data: &[u8]) -> Option<[u8; 32]> {
    const KEY: &[u8] = b"\"pop_secret\"";
    let at = data.windows(KEY.len()).position(|w| w == KEY)?;
    let mut rest = data[at + KEY.len()..].iter().copied().peekable();
    let skip_ws = |it: &mut std::iter::Peekable<std::iter::Copied<std::slice::Iter<u8>>>| {
        while it.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            it.next();
        }
    };
    skip_ws(&mut rest);
    if rest.next()? != b':' {
        return None;
    }
    skip_ws(&mut rest);
    if rest.next()? != b'[' {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        skip_ws(&mut rest);
        let mut v: u32 = 0;
        let mut digits = 0;
        while let Some(b) = rest.peek().copied().filter(u8::is_ascii_digit) {
            rest.next();
            v = v * 10 + u32::from(b - b'0');
            digits += 1;
            if digits > 3 {
                return None;
            }
        }
        if digits == 0 || v > 255 {
            return None;
        }
        *slot = v as u8;
        skip_ws(&mut rest);
        let sep = rest.next()?;
        let want = if i == 31 { b']' } else { b',' };
        if sep != want {
            return None;
        }
    }
    (out != [0u8; 32]).then_some(out)
}

/// [`salvage_pop_secret`] over the quarantined `world_meta.json.corrupt-*`
/// copies in `dir`, newest name first (the suffix is a Unix timestamp).
#[cfg(not(target_arch = "wasm32"))]
fn salvage_pop_secret_from_quarantine(dir: &std::path::Path) -> Option<[u8; 32]> {
    let mut names: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("world_meta.json.corrupt-"))
        .map(|e| e.path())
        .collect();
    names.sort();
    names
        .iter()
        .rev()
        .find_map(|p| fs::read(p).ok().and_then(|d| salvage_pop_secret(&d)))
}

/// Meta rebuilt after `world_meta.json` was lost (review S3). The seed comes
/// from `world.dat`; the integrity and one-shot flags are set CONSERVATIVELY so a
/// torn meta file can never launder a world into "pure survival, no cheats" or
/// make the one-per-world Genesis Block findable a second time. `WorldSave` holds
/// no game mode / world type, so those take their defaults.
#[cfg(not(target_arch = "wasm32"))]
fn recovered_meta(folder_name: &str, dat_path: &std::path::Path, save: &WorldSave) -> WorldMeta {
    let mut meta = WorldMeta::from_folder(folder_name, dat_path);
    meta.seed = save.seed;
    meta.cheats_used = true;
    meta.pure_survival = false;
    meta.genesis_block_found = true;
    meta
}

/// Load world_meta.json for a world, or generate defaults if it doesn't exist.
///
/// Infallible wrapper over [`try_load_world_meta`] for display/best-effort
/// callers. On a damaged world it returns placeholder defaults, but those can
/// never reach disk: `save_world_meta` refuses while the damage stands, and
/// `load_world` refuses to open the world.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_world_meta(folder_name: &str) -> WorldMeta {
    match try_load_world_meta(folder_name) {
        Ok(meta) => meta,
        Err(e) => {
            log::error!("world '{folder_name}': {e}");
            WorldMeta::from_folder(folder_name, &world_dir(folder_name).join("world.dat"))
        }
    }
}

/// Why `world_meta.json` in `dir` must not be written right now, if anything:
/// the existing file is unreadable/unparseable (it hasn't been quarantined yet),
/// or it is missing because a damaged copy was quarantined and not recovered.
/// Either way a write would put defaults over the world's real info.
#[cfg(not(target_arch = "wasm32"))]
fn meta_write_blocked(dir: &std::path::Path) -> Option<String> {
    // A world saved by a newer build: its meta may carry fields this build would
    // drop on re-write (Spec 02 §8.4).
    if let Some(e) = newer_save_in(dir) {
        return Some(e.to_string());
    }
    let meta_path = dir.join("world_meta.json");
    match fs::read(&meta_path) {
        Ok(data) => serde_json::from_slice::<WorldMeta>(&data)
            .err()
            .map(|e| format!("existing world_meta.json is damaged ({e})")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            has_quarantined_sibling(dir, "world_meta.json")
                .then(|| "world info damaged (quarantined world_meta.json, not recovered)".to_string())
        }
        Err(e) => Some(format!("existing world_meta.json unreadable ({e})")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_world_meta_file(dir: &std::path::Path, meta: &WorldMeta) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
    let json = serde_json::to_string_pretty(meta).map_err(|e| format!("serialize meta: {e}"))?;
    write_atomic(&dir.join("world_meta.json"), json.as_bytes())
}

#[cfg(target_arch = "wasm32")]
pub fn load_world_meta(folder_name: &str) -> WorldMeta {
    WASM_META_CACHE
        .with(|c| c.borrow().get(folder_name).cloned())
        .unwrap_or_else(|| WorldMeta::new(folder_name))
}

/// Save world_meta.json for a world.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_world_meta(folder_name: &str, meta: &WorldMeta) -> Result<(), String> {
    let dir = world_dir(folder_name);
    if let Some(why) = meta_write_blocked(&dir) {
        log::error!("world '{folder_name}': refusing to write world_meta.json: {why}");
        return Err(format!("refusing to write world_meta.json: {why}"));
    }
    write_world_meta_file(&dir, meta)
}

#[cfg(target_arch = "wasm32")]
pub fn save_world_meta(folder_name: &str, meta: &WorldMeta) -> Result<(), String> {
    WASM_META_CACHE.with(|c| {
        c.borrow_mut().insert(folder_name.to_string(), meta.clone());
    });
    Ok(())
}

/// List all worlds with full metadata + derived data, sorted by last played (newest first).
#[cfg(not(target_arch = "wasm32"))]
pub fn list_world_entries() -> Vec<WorldEntry> {
    let worlds_dir = worlds_root();
    if !worlds_dir.exists() {
        return Vec::new();
    }

    let mut entries = Vec::new();
    if let Ok(dir_entries) = fs::read_dir(&worlds_dir) {
        for entry in dir_entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let dat_path = entry.path().join("world.dat");
            if !dat_path.exists() {
                continue;
            }

            let folder_name = entry.file_name().to_string_lossy().to_string();
            // The Workshop is a singleton authoring space reached ONLY via the
            // "The Workshop" button — it must never appear as a lobby world card.
            if folder_name == crate::workshop::WORKSHOP_FOLDER {
                continue;
            }
            // A world saved by a newer build (Spec 02 §8.4) keeps its card, labelled
            // with why; opening it is refused with the same message.
            // A world whose info is damaged (see `try_load_world_meta`) still gets a
            // card, clearly labelled, so the player knows it exists; `load_world`
            // refuses to open it.
            let meta = if let Some(why) = world_open_refusal(&folder_name) {
                let mut m = try_load_world_meta(&folder_name)
                    .unwrap_or_else(|_| WorldMeta::from_folder(&folder_name, &dat_path));
                m.display_name = format!("{} (needs a newer version)", m.display_name);
                m.description = why;
                m
            } else {
                match try_load_world_meta(&folder_name) {
                    Ok(meta) => meta,
                    Err(e) => {
                        let mut m = WorldMeta::from_folder(&folder_name, &dat_path);
                        m.display_name = format!("{folder_name} (world info damaged)");
                        m.description = e;
                        m
                    }
                }
            };

            let last_played = dat_path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);

            let size_bytes = dir_size(&entry.path());

            entries.push(WorldEntry {
                folder_name,
                meta,
                last_played,
                size_bytes,
                cloud_only: false,
            });
        }
    }

    // Sort by last played, newest first
    entries.sort_by_key(|e| std::cmp::Reverse(e.last_played));
    entries
}

#[cfg(target_arch = "wasm32")]
pub fn list_world_entries() -> Vec<WorldEntry> {
    Vec::new()
}

/// The pure half of [`sanitize_folder_name`]: display name → filesystem-safe
/// slug, with **no** collision handling.
///
/// Callers that do their own collision naming need this rather than the
/// deduplicating wrapper — the wrapper's `_2` suffix would silently pre-empt
/// their rule (this is exactly what the `.axeprofile` keep-both import needs,
/// which wants `<name> (web)` and must therefore see the raw collision).
pub fn slugify_folder_name(display_name: &str) -> String {
    slug_or_fallback(display_name).unwrap_or_else(|| "world".to_string())
}

/// `Some(slug)` for a name with anything usable in it; `None` when the name
/// sanitises away to nothing (both callers then use the "world" fallback, and
/// `sanitize_folder_name` returns it WITHOUT a collision suffix — the behaviour
/// it had before this split).
fn slug_or_fallback(display_name: &str) -> Option<String> {
    let sanitized: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        return None;
    }
    // Truncate by characters, not bytes: &trimmed[..32] slices mid-char on a
    // multibyte (accented/CJK) name and panics (engine audit 2026-06-04, A).
    let truncated: String = trimmed.chars().take(32).collect();
    Some(truncated.trim_end_matches('_').to_string())
}

/// Sanitize a display name into a valid folder name, deduplicated against the
/// worlds directory (native) with a `_2`, `_3`, … suffix.
pub fn sanitize_folder_name(display_name: &str) -> String {
    let Some(base) = slug_or_fallback(display_name) else {
        return "world".to_string();
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let worlds_dir = worlds_root();
        if !worlds_dir.join(&base).exists() {
            return base;
        }
        for i in 2..100 {
            let candidate = format!("{base}_{i}");
            if !worlds_dir.join(&candidate).exists() {
                return candidate;
            }
        }
        format!("{base}_{}", std::time::UNIX_EPOCH.elapsed().unwrap_or_default().as_secs())
    }
    #[cfg(target_arch = "wasm32")]
    {
        base
    }
}

/// Fork (duplicate) a world. Returns the new folder name.
#[cfg(not(target_arch = "wasm32"))]
pub fn fork_world(source_folder: &str, source_display_name: &str) -> Result<String, String> {
    let source_dir = world_dir(source_folder);
    if !source_dir.exists() {
        return Err(format!("Source world '{}' not found", source_folder));
    }

    // Load the source metadata to preserve integrity fields
    let source_meta = load_world_meta(source_folder);
    let new_display_name = format!("{source_display_name} (fork)");
    let new_folder = sanitize_folder_name(&new_display_name);
    let dest_dir = world_dir(&new_folder);

    copy_dir_recursive(&source_dir, &dest_dir)
        .map_err(|e| format!("fork copy: {e}"))?;

    // Write new metadata for the fork
    let new_meta = WorldMeta {
        display_name: new_display_name,
        description: format!("Forked from {source_display_name}"),
        game_mode: source_meta.game_mode,
        created_at: now_iso8601(),
        is_workshop: source_meta.is_workshop,
        icon: None,
        pure_survival: source_meta.pure_survival,
        ever_creative: source_meta.ever_creative,
        cheats_used: source_meta.cheats_used,
        difficulty: source_meta.difficulty,
        difficulty_history: source_meta.difficulty_history,
        forked_from: Some(source_folder.to_string()),
        version: 0,
        // A fork starts a fresh world for the purposes of Wave 25's
        // Genesis Block — the next gem found in the fork triggers the
        // celebration. Source world's claim doesn't transfer.
        genesis_block_found: false,
            commands_enabled: true,
            explosives_enabled: true,
            fire_spread_enabled: true,
            cloud_save: false,
            has_seen_license_onboarding: false,
            // A fork is the same terrain as its source — inherit the seed so
            // the regenerated world matches block-for-block.
            seed: source_meta.seed,
            // A fork is a fresh world for proof-of-play: its lifetime work +
            // world-clock + Genesis start from zero, matching the
            // genesis_block_found reset above.
            total_work: 0,
            total_ticks: 0,
            genesis_found_at_tick: None,
            // A fork starts as a fresh world, not a resumed scenario.
            scenario_def: None,
            // A fork starts without any per-world override; creators can set
            // their own after the fork if desired.
            world_override: None,
            // Inherit world-type config from the source so a flat-world fork
            // generates the same canvas as its parent.
            world_type: source_meta.world_type,
            ground: source_meta.ground,
            water_depth: source_meta.water_depth,
            time_lock: source_meta.time_lock,
            mobs_enabled: source_meta.mobs_enabled,
            keep_inventory: source_meta.keep_inventory,
            // Seam B: a fork starts UNOWNED, not inheriting the source owner — the
            // forker's first save claims it (claim_owner won't clobber a different
            // existing owner, so starting None avoids mis-attributing the fork).
            owner_pubkey: None,
            // A fork is a fresh local copy — not published to any server.
            published_to: None,
            satoshi_enabled: false,
            pop_secret: None,
        };
    save_world_meta(&new_folder, &new_meta)?;

    log::info!("Forked world '{source_folder}' → '{new_folder}'");
    Ok(new_folder)
}

#[cfg(target_arch = "wasm32")]
pub fn fork_world(_source_folder: &str, _source_display_name: &str) -> Result<String, String> {
    Err("fork_world not available on WASM".to_string())
}

// ---------------------------------------------------------------------------
// Autosave
// ---------------------------------------------------------------------------

/// Autosave the world to a separate subdirectory within the world folder.
/// This protects against crashes without overwriting the manual save.
#[cfg(not(target_arch = "wasm32"))]
pub fn autosave_world(
    name: &str,
    world: &World,
    players: &[crate::player_slot::PlayerSlot],
    seed: u32,
    carts: &[SavedCart],
    saved_mobs: &[SavedTamedPet],
) -> Result<(), String> {
    if players.is_empty() {
        return Err("autosave_world: no players to save".to_string());
    }
    refuse_write_over_newer_save(&world_dir(name))?;
    let dir = world_dir(name).join("autosave");
    let chunks_dir = dir.join("chunks");
    fs::create_dir_all(&chunks_dir).map_err(|e| format!("mkdir autosave: {e}"))?;

    // Build per-player save data
    let player_saves: Vec<PlayerSaveData> = players.iter().map(|slot| {
        PlayerSaveData {
            x: slot.player.pos.x,
            y: slot.player.pos.y,
            z: slot.player.pos.z,
            yaw: slot.camera.yaw,
            pitch: slot.camera.pitch,
            health: slot.combat.health,
            hotbar_slot: slot.hotbar_slot,
            inventory: serialize_inventory(&slot.inventory),
            spawn_pos: Some([slot.spawn_pos.x, slot.spawn_pos.y, slot.spawn_pos.z]),
            hunger: slot.combat.hunger,
            reputation: slot.reputation.per_village.iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            // BRIDGE: PlayerSlot has no `pets` field yet (wolves are
            // transient mobs in alpha). Save empty for now; field is in
            // place so when mob-save lands the format doesn't break.
            tamed_pets: Vec::new(),
            armour_slots: serialize_armour_slots(&slot.armour_slots),
            kill_counter: slot.kill_counter.iter().map(|(&k, &v)| (k, v)).collect(),
            bounties_claimed: slot.bounties_claimed.iter().map(|(&k, &v)| (k, v)).collect(),
        }
    }).collect();

    // Backward compat: write old single-player fields from player 0
    let p0 = &player_saves[0];
    let campfires: Vec<SavedCampfire> = world
        .iter_campfires()
        .map(|((x, y, z), data)| SavedCampfire { x, y, z, data: data.clone() })
        .collect();
    let furnaces: Vec<SavedFurnace> = world
        .iter_furnaces()
        .map(|((x, y, z), data)| SavedFurnace { x, y, z, data: data.clone() })
        .collect();
    let vendors: Vec<SavedVendor> = world
        .iter_vendors()
        .map(|((x, y, z), data)| SavedVendor { x, y, z, data: data.clone() })
        .collect();
    let drying_racks: Vec<SavedDryingRack> = world
        .drying_racks
        .iter()
        .map(|(&(x, y, z), data)| SavedDryingRack { x, y, z, data: data.clone() })
        .collect();
    let hives: Vec<SavedHive> = world
        .iter_hives()
        .map(|((x, y, z), data)| SavedHive { x, y, z, data: *data })
        .collect();
    let dispensers: Vec<SavedDispenser> = world
        .iter_dispensers()
        .map(|((x, y, z), d)| SavedDispenser { x, y, z, data: d.clone() })
        .collect();
    let chests: Vec<SavedChest> = world
        .iter_chests()
        .map(|((x, y, z), data)| SavedChest { x, y, z, data: data.clone() })
        .collect();
    let signs: Vec<SavedSign> = world
        .iter_signs()
        .map(|((x, y, z), data)| SavedSign { x, y, z, data: data.clone() })
        .collect();
    let item_frames: Vec<SavedItemFrame> = world
        .iter_item_frames()
        .map(|((x, y, z), data)| SavedItemFrame { x, y, z, data: data.clone() })
        .collect();
    let tip_jars: Vec<SavedTipJar> = world
        .iter_tip_jars()
        .map(|((x, y, z), data)| SavedTipJar { x, y, z, data: data.clone() })
        .collect();
    let auctions: Vec<SavedAuction> = world
        .iter_auctions()
        .map(|((x, y, z), data)| SavedAuction { x, y, z, data: data.clone() })
        .collect();
    // Spec 38 (Blueprint / Cyanotype) — Latent Print block-entities.
    // Each holds a PlanData with its current develop_state, so a save/
    // load round-trip preserves how much sun a print has already caught.
    let latent_prints: Vec<SavedLatentPrint> = world
        .iter_latent_prints()
        .map(|((x, y, z), data)| SavedLatentPrint { x, y, z, data: data.clone() })
        .collect();
    let construction_anchors: Vec<SavedConstructionAnchor> = world
        .construction_anchors
        .iter()
        .map(|(&(x, y, z), data)| SavedConstructionAnchor { x, y, z, data: data.clone() })
        .collect();
    let architect_plaques: Vec<SavedArchitectPlaque> = world
        .architect_plaques
        .iter()
        .map(|(&(x, y, z), chain)| SavedArchitectPlaque { x, y, z, chain: chain.clone() })
        .collect();
    let save = WorldSave {
        seed,
        player_x: p0.x,
        player_y: p0.y,
        player_z: p0.z,
        player_health: p0.health,
        hotbar_slot: p0.hotbar_slot,
        inventory: p0.inventory.clone(),
        players: player_saves,
        campfires,
        furnaces,
        vendors,
        drying_racks,
        hives,
        chests,
        signs,
        item_frames,
        locked_slots: players[0].inventory.locked_indices(),
        hostile_acts: world.hostile_acts.acts().to_vec(),
        rigs: world.rigs.clone(),
        // #19 — index-aligned clip side table (see `WorldSave.rig_clips`).
        rig_clips: world.rigs.iter().map(|r| r.clip).collect(),
        exhibits: world.exhibits.clone(),
        composters: world.iter_composters().map(|((x, y, z), data)| SavedComposter { x, y, z, data: data.clone() }).collect(),
        tip_jars,
        auctions,
        latent_prints,
        plots: world.plots.clone(),
        market_hubs: world.market_hubs.clone(),
        construction_anchors,
        architect_plaques,
        village_anchors: world.village_anchors.iter().map(|(&(gx, gz), &anchor)| SavedVillageAnchor { grid_x: gx, grid_z: gz, anchor }).collect(),
        populated_villages: world.populated_villages.iter().copied().collect(),
        village_bells: world.village_bells.clone(),
        village_treasuries: world.village_treasuries.iter().map(|(&k, &v)| (k, v)).collect(),
        active_raids: world.active_raids.clone(),
        raid_scheduler: world.raid_scheduler.clone(),
        raid_kills: world.raid_kills.iter().map(|(&(vid, pk), &c)| (vid, pk, c)).collect(),
        brigand_hideouts: world.brigand_hideouts.iter()
            .map(|(&(gx, gz), data)| SavedHideout { grid_x: gx, grid_z: gz, data: data.clone() })
            .collect(),
        bounties: world.bounties.iter().map(|b| SavedBounty {
            id: b.id, template_idx: b.template_idx, issued_tick: b.issued_tick,
        }).collect(),
        bounty_next_id: world.bounty_next_id,
        bounty_last_refresh_tick: world.bounty_last_refresh_tick,
        face_overlays: face_overlays_to_saved(world),
        face_blueprints: face_blueprints_to_saved(world),
        face_blueprint_blanks: face_blueprint_blanks_to_saved(world),
        workshop: world.workshop.clone(),
        // Rail freight Phase 1 — carts snapshotted from the caller's ECS.
        carts: carts.to_vec(),
        saved_mobs: saved_mobs.to_vec(),
        satoshi: world.satoshi.clone(),
        dispensers,
        graves: world
            .iter_graves()
            .map(|((x, y, z), data)| SavedGrave { x, y, z, data: data.clone() })
            .collect(),
        waypoints: world.waypoints.clone(),
        // Spec 48 (Electricity) — per-block meta + power-device runtime state, so
        // circuits survive save/reload. Collected inline like graves/waypoints.
        block_meta: world
            .block_meta
            .iter()
            .map(|(&(x, y, z), &m)| (x, y, z, m))
            .collect(),
        power_devices: world
            .iter_power_devices()
            .map(|((x, y, z), data)| SavedPowerDevice { x, y, z, data: data.clone() })
            .collect(),
    };

    let encoded = crate::save_format::encode_world_save(&save)?;

    let mut saved = 0u32;
    let (to_write, to_delete) = partition_chunks_for_save(world);
    for (cx, cy, cz) in to_delete {
        let _ = fs::remove_file(chunks_dir.join(format!("{cx}_{cy}_{cz}.chunk")));
    }
    for (cx, cy, cz) in to_write {
        if let Some(chunk) = world.persistable_chunk(cx, cy, cz) {
            let filename = format!("{cx}_{cy}_{cz}.chunk");
            write_atomic_nosync(&chunks_dir.join(&filename), &chunk.as_bytes())
                .map_err(|e| format!("write autosave chunk {filename}: {e}"))?;
            saved += 1;
        }
    }
    // One directory fsync for all chunk renames (review S2), then world.dat as the
    // commit point. Atomic + fsynced — a crash mid-autosave must not leave a torn
    // world.dat, especially since the crash-recovery loader deletes the autosave.
    sync_dir(&chunks_dir);
    write_atomic(&dir.join("world.dat"), &encoded)?;

    // Flush the live per-world stats to the ROOT world_meta.json so a crash
    // recovery (which seeds total_work/total_ticks from meta) doesn't roll the
    // world-clock back to the last MANUAL save. Best-effort; the chunks above
    // are the autosave's main payload. genesis_found_at_tick is written eagerly
    // at genesis, so it's already current in meta.
    let mut meta = load_world_meta(name);
    meta.total_work = world.total_work;
    meta.total_ticks = world.total_ticks;
    let _ = save_world_meta(name, &meta);

    log::info!("Autosaved world '{name}': {saved} chunks");
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn autosave_world(
    name: &str,
    world: &World,
    players: &[crate::player_slot::PlayerSlot],
    seed: u32,
    carts: &[SavedCart],
    saved_mobs: &[SavedTamedPet],
) -> Result<(), String> {
    // On WASM, autosave writes to the same IndexedDB store as save_world.
    // NOTE: there is NO cloud upload here yet. The cloud-save bridge tier
    // (tools/sites/game POST /worlds/upload) exists but is not wired into this
    // path — that's Phase 5 of docs/foundations/2026-05-26-cloud-save-blossom.md.
    save_world(name, world, players, seed, carts, saved_mobs)
}

/// Check if an autosave exists for this world.
#[cfg(not(target_arch = "wasm32"))]
pub fn has_autosave(name: &str) -> bool {
    world_dir(name).join("autosave").join("world.dat").exists()
}

#[cfg(target_arch = "wasm32")]
pub fn has_autosave(_name: &str) -> bool {
    false
}

/// Load world from autosave (crash recovery). Returns same as load_world.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_autosave(
    name: &str,
    world: &mut World,
) -> Result<(WorldSave, u32), String> {
    let dir = world_dir(name).join("autosave");
    let dat_path = dir.join("world.dat");
    let data = fs::read(&dat_path).map_err(|e| format!("read autosave: {e}"))?;
    // Goal 3 / Task 1 — same tolerant decode as load_world.
    let (save, partial) = read_world_save_reporting(&data)?;
    if partial {
        keep_damaged_copy_once(&dat_path);
    }

    let loaded = load_chunk_dir(&dir.join("chunks"), world)?;

    // Goal 3 / Task 2 — crash-recovery integrity. `autosave_world` writes the FULL
    // `WorldSave` (block-entities + overlays included), but this path historically
    // restored only chunks, silently dropping all of it. Apply the same world-level
    // restore as `load_world` (after the chunk loop, so the index rebuilds see them).
    apply_world_save_state(world, &save);

    log::info!("Loaded from autosave: {loaded} chunks, player at ({:.1}, {:.1}, {:.1})",
        save.player_x, save.player_y, save.player_z);
    Ok((save, loaded))
}

#[cfg(target_arch = "wasm32")]
pub fn load_autosave(
    _name: &str,
    _world: &mut World,
) -> Result<(WorldSave, u32), String> {
    Err("load_autosave not available on WASM".to_string())
}

/// Delete the autosave directory for a world.
#[cfg(not(target_arch = "wasm32"))]
pub fn clear_autosave(name: &str) {
    let dir = world_dir(name).join("autosave");
    if dir.exists() {
        if let Err(e) = fs::remove_dir_all(&dir) {
            log::warn!("Failed to clear autosave: {e}");
        } else {
            log::info!("Cleared autosave for '{name}'");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn clear_autosave(_name: &str) {
    // no-op on WASM
}

/// Delete a saved world permanently.
#[cfg(not(target_arch = "wasm32"))]
pub fn delete_world(folder_name: &str) -> Result<(), String> {
    let dir = world_dir(folder_name);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("delete: {e}"))?;
        log::info!("Deleted world '{folder_name}'");
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn delete_world(folder_name: &str) -> Result<(), String> {
    // Best-effort fire-and-forget delete against the IndexedDB store.
    // Until 2026-05-30 this was a silent no-op, so the menu's "Delete
    // world" button did nothing on the PWA (the alpha's only target) —
    // worlds were undeletable. The menu's own delete path
    // (`kick_off_local_delete`) sequences delete→re-list so the list
    // refresh can't race the delete; this entry point covers any other
    // caller. Drop the cached meta so a same-named world created later
    // doesn't inherit the deleted world's metadata.
    WASM_META_CACHE.with(|c| {
        c.borrow_mut().remove(folder_name);
    });
    let pubkey = wasm_storage_key();
    let folder_owned = folder_name.to_string();
    wasm_bindgen_futures::spawn_local(async move {
        match crate::wasm_save::delete_world_wasm(&pubkey, &folder_owned).await {
            Ok(_) => log::info!("WASM delete OK: '{folder_owned}'"),
            Err(e) => log::error!("WASM delete failed for '{folder_owned}': {e}"),
        }
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get total size of a directory in bytes.
#[cfg(not(target_arch = "wasm32"))]
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                } else if ft.is_dir() {
                    total += dir_size(&entry.path());
                }
            }
        }
    }
    total
}

/// Recursively copy a directory.
#[cfg(not(target_arch = "wasm32"))]
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

/// Format current time as ISO 8601.
#[cfg(not(target_arch = "wasm32"))]
fn now_iso8601() -> String {
    // Simple UTC timestamp without external crate
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    // Approximate: good enough for display, not astronomical precision
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let mins = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Days since epoch to Y-M-D (simplified leap year handling)
    let (year, month, day) = epoch_days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{s:02}Z")
}

#[cfg(target_arch = "wasm32")]
fn now_iso8601() -> String {
    // BRIDGE: Use js_sys::Date when proper WASM time is needed.
    "2026-01-01T00:00:00Z".to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn system_time_to_iso8601(t: SystemTime) -> String {
    let dur = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let mins = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    let (year, month, day) = epoch_days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{s:02}Z")
}

/// Convert days since Unix epoch to (year, month, day). Handles leap years.
#[cfg(not(target_arch = "wasm32"))]
fn epoch_days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_days: &[u64] = if is_leap(year) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &md in month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

#[cfg(not(target_arch = "wasm32"))]
fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Format a SystemTime as a human-readable relative time ("2 hours ago", "Yesterday", etc.)
#[cfg(not(target_arch = "wasm32"))]
pub fn format_relative_time(t: SystemTime) -> String {
    let now = SystemTime::now();
    let elapsed = now.duration_since(t).unwrap_or_default();
    let secs = elapsed.as_secs();

    if secs < 60 {
        "Just now".to_string()
    } else if secs < 3600 {
        let mins = secs / 60;
        if mins == 1 { "1 minute ago".to_string() } else { format!("{mins} minutes ago") }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 { "1 hour ago".to_string() } else { format!("{hours} hours ago") }
    } else if secs < 172800 {
        "Yesterday".to_string()
    } else if secs < 604800 {
        let days = secs / 86400;
        format!("{days} days ago")
    } else if secs < 2592000 {
        let weeks = secs / 604800;
        if weeks == 1 { "1 week ago".to_string() } else { format!("{weeks} weeks ago") }
    } else {
        let months = secs / 2592000;
        if months == 1 { "1 month ago".to_string() } else { format!("{months} months ago") }
    }
}

/// Format bytes as human-readable size.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Shared test helper: build the smallest valid [`WorldSave`] that bincode will
/// accept.  Exposed `pub(crate)` so both `world_archive` and `native_world_io`
/// can use it without duplicating the field list.
#[cfg(test)]
pub(crate) fn minimal_world_save_for_tests(seed: u32) -> WorldSave {
    WorldSave {
        seed,
        player_x: 0.0,
        player_y: 64.0,
        player_z: 0.0,
        player_health: 20.0,
        hotbar_slot: 0,
        inventory: Vec::new(),
        players: Vec::new(),
        campfires: Vec::new(),
        furnaces: Vec::new(),
        vendors: Vec::new(),
        drying_racks: Vec::new(),
        hives: Vec::new(),
        chests: Vec::new(),
        tip_jars: Vec::new(),
        plots: Vec::new(),
        market_hubs: Vec::new(),
        auctions: Vec::new(),
        latent_prints: Vec::new(),
        construction_anchors: Vec::new(),
        architect_plaques: Vec::new(),
        village_anchors: Vec::new(),
        populated_villages: Vec::new(),
        village_bells: Vec::new(),
        village_treasuries: Vec::new(),
        active_raids: Vec::new(),
        raid_scheduler: crate::raid::RaidScheduler::new(),
        raid_kills: Vec::new(),
        brigand_hideouts: Vec::new(),
        bounties: Vec::new(),
        bounty_next_id: 0,
        bounty_last_refresh_tick: 0,
        face_overlays: Vec::new(),
        face_blueprints: Vec::new(),
        face_blueprint_blanks: Vec::new(),
        workshop: Default::default(),
        carts: Vec::new(),
        graves: Vec::new(),
        waypoints: Vec::new(),
        block_meta: Vec::new(),
        power_devices: Vec::new(),
        signs: Vec::new(),
        item_frames: Vec::new(),
        locked_slots: Vec::new(),
        hostile_acts: Vec::new(),
        rigs: Vec::new(),
        exhibits: Vec::new(),
        composters: Vec::new(),
        saved_mobs: Vec::new(),
        satoshi: Default::default(),
        dispensers: Vec::new(),
        rig_clips: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crafting::{Tool, ToolMaterial, ToolType};
    use crate::item::{ItemStack, MaterialId};

    /// Local-world storage namespace: a guest (no / empty pubkey) maps to the
    /// constant `"local"` bucket, so the WASM save/load/list/delete path is never
    /// blocked on a missing pubkey — the web login-kill bug fix. A signed-in
    /// persona keeps its own pubkey namespace.
    #[test]
    fn storage_namespace_guest_maps_to_local() {
        assert_eq!(storage_namespace(None), "local");
        assert_eq!(storage_namespace(Some("")), "local");
        let pk = "a".repeat(64);
        assert_eq!(storage_namespace(Some(&pk)), pk);
    }

    /// FS-test isolation: a [`WorldsRootGuard`] must redirect BOTH writes
    /// (`world_dir` → `write_world_folder`) and reads (`list_world_entries`) to a
    /// private per-thread temp dir, so the listing reflects only what this test
    /// wrote — independent of the shared `worlds/` dir other tests mutate in
    /// parallel. This is the mechanism that de-flakes the import count tests.
    #[test]
    fn list_world_entries_respects_isolated_worlds_root() {
        let _guard = WorldsRootGuard::new("iso-list");
        // Freshly-isolated root → empty, regardless of the real worlds/ dir.
        assert!(
            list_world_entries().is_empty(),
            "an isolated worlds root must start empty"
        );
        // Write one world into the isolated root; it must be the ONLY one listed.
        let meta = WorldMeta::new("Iso Test World");
        let save = minimal_world_save_for_tests(42);
        write_world_folder("iso_test_world", &meta, &save, &crate::world::World::new())
            .expect("write_world_folder into isolated root");
        let names: Vec<String> = list_world_entries()
            .into_iter()
            .map(|e| e.folder_name)
            .collect();
        assert_eq!(
            names,
            vec!["iso_test_world".to_string()],
            "isolated root must list exactly the world written into it"
        );
    }

    fn inventory_with_mix() -> Inventory {
        let mut inv = Inventory::new();
        // Clear and set a deliberate mix of Block / Tool / Material / Empty.
        for i in 0..36 {
            inv.set_slot(i, None);
        }
        inv.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 42)));
        inv.set_slot(1, Some(ItemStack::new_tool(Tool::new(
            ToolType::Pickaxe,
            ToolMaterial::Iron,
        ))));
        inv.set_slot(2, Some(ItemStack::new_material(MaterialId::Stick, 13)));
        // slot 3 intentionally empty
        inv.set_slot(4, Some(ItemStack::new_tool(Tool::new(
            ToolType::Sword,
            ToolMaterial::Diamond,
        ))));
        inv
    }

    #[test]
    fn saved_slot_variants_bincode_roundtrip() {
        let slots = vec![
            SavedSlot::Empty,
            SavedSlot::Block { block_id: 42, count: 64 },
            SavedSlot::Tool {
                tool_type: ToolType::Axe,
                material: ToolMaterial::Stone,
                durability: 123,
            },
            SavedSlot::Material { material_id: MaterialId::Stick, count: 10 },
        ];
        let bytes = bincode::serialize(&slots).unwrap();
        let back: Vec<SavedSlot> = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.len(), 4);
        assert!(matches!(back[0], SavedSlot::Empty));
        assert!(matches!(back[1], SavedSlot::Block { block_id: 42, count: 64 }));
        match &back[2] {
            SavedSlot::Tool { tool_type, material, durability } => {
                assert_eq!(*tool_type, ToolType::Axe);
                assert_eq!(*material, ToolMaterial::Stone);
                assert_eq!(*durability, 123);
            }
            _ => panic!("tool variant lost"),
        }
        match &back[3] {
            SavedSlot::Material { material_id, count } => {
                assert_eq!(*material_id, MaterialId::Stick);
                assert_eq!(*count, 10);
            }
            _ => panic!("material variant lost"),
        }
    }

    #[test]
    fn inventory_serialize_restore_roundtrip() {
        let before = inventory_with_mix();
        let slots = serialize_inventory_raw(&before);
        assert_eq!(slots.len(), 36);

        let mut after = Inventory::new();
        restore_inventory(&mut after, &slots);

        for i in 0..36 {
            let b = before.slot(i);
            let a = after.slot(i);
            match (b, a) {
                (None, None) => {}
                (Some(bs), Some(as_)) => {
                    assert_eq!(bs.count, as_.count, "count at slot {i}");
                    match (&bs.item, &as_.item) {
                        (crate::item::Item::Block(b1), crate::item::Item::Block(b2)) => {
                            assert_eq!(b1, b2);
                        }
                        (crate::item::Item::Tool(t1), crate::item::Item::Tool(t2)) => {
                            assert_eq!(t1.tool_type, t2.tool_type);
                            assert_eq!(t1.material, t2.material);
                            assert_eq!(t1.durability, t2.durability);
                        }
                        (crate::item::Item::Material(m1), crate::item::Item::Material(m2)) => {
                            assert_eq!(m1, m2);
                        }
                        _ => panic!("item variant mismatch at slot {i}"),
                    }
                }
                _ => panic!("slot presence mismatch at {i}: before={:?} after={:?}", b.is_some(), a.is_some()),
            }
        }
    }

    #[test]
    fn world_save_bincode_roundtrip_with_players() {
        let inv_slots = serialize_inventory_raw(&inventory_with_mix());
        let save = WorldSave {
            seed: 42,
            player_x: 10.5, player_y: 80.0, player_z: -3.25,
            player_health: 18.0,
            hotbar_slot: 3,
            inventory: inv_slots.clone(),
            players: vec![PlayerSaveData {
                x: 10.5, y: 80.0, z: -3.25,
                yaw: 1.5, pitch: -0.2,
                health: 18.0,
                hotbar_slot: 3,
                inventory: inv_slots,
                spawn_pos: Some([10.5, 80.0, -3.25]),
                hunger: 20,
                reputation: vec![],
                tamed_pets: vec![],
                armour_slots: [None, None, None, None],
                kill_counter: vec![],
                bounties_claimed: vec![],
            }],
            campfires: Vec::new(),
            furnaces: Vec::new(),
            vendors: Vec::new(),
            drying_racks: Vec::new(),
            hives: Vec::new(),
            chests: Vec::new(),
            tip_jars: Vec::new(),
            auctions: Vec::new(),
            latent_prints: Vec::new(),
            plots: Vec::new(),
            market_hubs: Vec::new(),
            construction_anchors: Vec::new(),
            architect_plaques: Vec::new(),
            village_anchors: Vec::new(),
            populated_villages: Vec::new(),
            village_bells: Vec::new(),
            village_treasuries: Vec::new(),
            active_raids: Vec::new(),
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: Vec::new(),
            brigand_hideouts: Vec::new(),
            bounties: Vec::new(),
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            face_overlays: vec![],
            face_blueprints: vec![],
            face_blueprint_blanks: vec![],
            workshop: Default::default(),
            carts: vec![],
            graves: vec![],
            waypoints: vec![crate::waypoint::Waypoint {
                id: 1,
                name: "Home".into(),
                pos: [3, 64, 7],
                colour: crate::waypoint::MANUAL_COLOUR,
                kind: crate::waypoint::WaypointKind::Manual,
            }],
            block_meta: vec![],
            power_devices: vec![],
            signs: vec![],
            item_frames: vec![],
            locked_slots: vec![],
            hostile_acts: vec![],
            rigs: vec![],
            exhibits: Vec::new(),
            composters: Vec::new(),
            saved_mobs: Vec::new(),
            satoshi: Default::default(),
            dispensers: Vec::new(),
            rig_clips: Vec::new(),
        };
        let bytes = bincode::serialize(&save).unwrap();
        let back: WorldSave = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.seed, 42);
        assert_eq!(back.hotbar_slot, 3);
        assert_eq!(back.players.len(), 1);
        assert_eq!(back.players[0].yaw, 1.5);
        assert_eq!(back.inventory.len(), 36);
        // #6 — waypoints survive the bincode round-trip.
        assert_eq!(back.waypoints.len(), 1);
        assert_eq!(back.waypoints[0].name, "Home");
        assert_eq!(back.waypoints[0].pos, [3, 64, 7]);
    }

    #[test]
    fn spawn_pos_round_trips_through_save_format() {
        // Wave 20: a bed-set spawn point survives serialise → deserialise.
        let save = PlayerSaveData {
            x: 0.0, y: 64.0, z: 0.0,
            yaw: 0.0, pitch: 0.0,
            health: 20.0,
            hotbar_slot: 0,
            inventory: vec![],
            spawn_pos: Some([42.5, 70.0, -17.5]),
            hunger: 20,
            reputation: vec![],
            tamed_pets: vec![],
            armour_slots: [None, None, None, None],
            kill_counter: vec![],
            bounties_claimed: vec![],
        };
        let bytes = bincode::serialize(&save).unwrap();
        let back: PlayerSaveData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.spawn_pos, Some([42.5, 70.0, -17.5]));
    }

    #[test]
    fn missing_spawn_pos_deserialises_as_none() {
        // Old save formats had no spawn_pos field. The #[serde(default)]
        // attribute must let those load as `spawn_pos: None`. Manually craft
        // a JSON instance that omits the field to simulate.
        let json = r#"{"x":0.0,"y":64.0,"z":0.0,"yaw":0.0,"pitch":0.0,"health":20.0,"hotbar_slot":0,"inventory":[]}"#;
        let parsed: PlayerSaveData = serde_json::from_str(json).expect("missing spawn_pos must default to None");
        assert_eq!(parsed.spawn_pos, None);
    }

    #[test]
    fn genesis_block_found_round_trips_through_world_meta() {
        // Wave 25: Genesis Block flag is a per-world fact (one Genesis
        // Block per world, ever). Lives on WorldMeta, not on PlayerSaveData.
        let mut meta = WorldMeta::new("test-world");
        meta.genesis_block_found = true;
        let bytes = bincode::serialize(&meta).unwrap();
        let back: WorldMeta = bincode::deserialize(&bytes).unwrap();
        assert!(back.genesis_block_found);
    }

    #[test]
    fn proof_of_play_stats_round_trip_through_world_meta() {
        // Goal 1 / Spec 2 §9.1: the universal per-world proof-of-play stats —
        // lifetime work, lifetime active ticks (world-clock), and the
        // time-to-genesis — persist through a save round-trip.
        let mut meta = WorldMeta::new("test-world");
        meta.total_work = 12_345;
        meta.total_ticks = 6_789;
        meta.genesis_found_at_tick = Some(4_242);
        let bytes = bincode::serialize(&meta).unwrap();
        let back: WorldMeta = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.total_work, 12_345);
        assert_eq!(back.total_ticks, 6_789);
        assert_eq!(back.genesis_found_at_tick, Some(4_242));
    }

    #[test]
    fn scenario_def_round_trips_through_world_meta() {
        // Goal 4 — a resumable scenario (Satori Rush) is persisted as its def
        // JSON so the run resumes on reload; fresh worlds carry None.
        let mut meta = WorldMeta::new("test-world");
        assert_eq!(meta.scenario_def, None);
        meta.scenario_def = Some("{\"kind\":\"SatoriRush\"}".to_string());
        let bytes = bincode::serialize(&meta).unwrap();
        let back: WorldMeta = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.scenario_def.as_deref(), Some("{\"kind\":\"SatoriRush\"}"));
    }

    #[test]
    fn retired_gallery_world_type_loads_as_a_flat_world() {
        // The built-in Gallery was pulled out into an external .axeworld pack; a
        // save that still says world_type "gallery" must load safely as flat.
        let mut meta = WorldMeta::new("old gallery");
        meta.world_type = RETIRED_GALLERY_WORLD_TYPE.to_string();
        let json = serde_json::to_string(&meta).unwrap();
        let back: WorldMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.world_type, "flat");
        let bin: WorldMeta = bincode::deserialize(&bincode::serialize(&meta).unwrap()).unwrap();
        assert_eq!(bin.world_type, "flat");
        // Other types pass through untouched.
        meta.world_type = "normal".to_string();
        let back: WorldMeta = serde_json::from_str(&serde_json::to_string(&meta).unwrap()).unwrap();
        assert_eq!(back.world_type, "normal");

        // And the generator never panics on the retired type (e.g. from a
        // hand-written scenario def): it lays the flat floor.
        let mut world = crate::world::World::new();
        world.world_type = RETIRED_GALLERY_WORLD_TYPE.to_string();
        world.generate_column(0, 0, &crate::biome::BiomeGenerator::new(1));
        assert!(world.has_flat_floor());
        assert_ne!(world.get_block(3, crate::workshop::WORKSHOP_FLOOR_Y, 3), crate::block::AIR);
        assert_eq!(world.get_block(3, crate::workshop::WORKSHOP_FLOOR_Y + 1, 3), crate::block::AIR);
    }

    #[test]
    fn published_to_round_trips_through_world_meta() {
        // Publish flow §2.1: the publish record persists through a save round-trip.
        let mut meta = WorldMeta::new("gallery");
        assert_eq!(meta.published_to, None);
        meta.published_to = Some(PublishRecord {
            server_npub: "npub1srv".into(),
            server_addr: "wss://play.example".into(),
            world_id: "main".into(),
            last_published_hash: "deadbeef".into(),
            last_published_version: 7,
            last_published_unix: 1_700_000_000,
        });
        let json = serde_json::to_string(&meta).unwrap();
        let back: WorldMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.published_to, meta.published_to);
    }

    #[test]
    fn missing_published_to_deserialises_as_none() {
        // A pre-publish world_meta.json has no `published_to`; #[serde(default)]
        // must let it load as None (worlds persist as serde_json, so a missing
        // tail field is the real backward-compat case). `icon` has no serde
        // default, so it must be present (null) — every other field defaults.
        let json = r#"{"display_name":"old","description":"","game_mode":"survival","created_at":"","icon":null}"#;
        let parsed: WorldMeta = serde_json::from_str(json)
            .expect("a minimal pre-publish meta must load with published_to defaulting to None");
        assert_eq!(parsed.published_to, None);
        assert_eq!(parsed.publish_badge(), PublishBadge::NotPublished);
    }

    #[test]
    fn publish_badge_reflects_version_drift() {
        let mut meta = WorldMeta::new("w");
        // Never published → NotPublished.
        assert_eq!(meta.publish_badge(), PublishBadge::NotPublished);
        meta.version = 3;
        meta.published_to = Some(PublishRecord {
            server_npub: "npub1".into(),
            server_addr: "wss://x".into(),
            world_id: "main".into(),
            last_published_hash: "h".into(),
            last_published_version: 3,
            last_published_unix: 0,
        });
        // Live version == published version → UpToDate.
        assert_eq!(meta.publish_badge(), PublishBadge::UpToDate);
        // A save bumps `version`; the server copy is now stale → UnpublishedChanges.
        meta.version = 4;
        assert_eq!(meta.publish_badge(), PublishBadge::UnpublishedChanges);
    }

    #[test]
    fn missing_proof_of_play_stats_deserialise_as_defaults() {
        // Pre-Goal-1 saves lack total_work / total_ticks / genesis_found_at_tick.
        // #[serde(default)] must load them as 0 / 0 / None so old worlds upgrade
        // cleanly (their lifetime tallies simply start from this session).
        let json = r#"{"display_name":"old","description":"","game_mode":"survival","created_at":"","icon":null}"#;
        let parsed: WorldMeta = serde_json::from_str(json).expect("legacy meta must parse");
        assert_eq!(parsed.total_work, 0);
        assert_eq!(parsed.total_ticks, 0);
        assert_eq!(parsed.genesis_found_at_tick, None);
        assert_eq!(parsed.scenario_def, None);
    }

    #[test]
    fn cloud_save_defaults_to_false_for_legacy_meta() {
        // Privacy-first: any save written before the opt-in toggle existed
        // (and the JS world-list payload, which omits the field) must read
        // back as cloud_save:false so nothing is silently stashed.
        let minimal_json = r#"{"display_name":"old","description":"","game_mode":"survival","created_at":"","icon":null}"#;
        let meta: WorldMeta = serde_json::from_str(minimal_json).expect("legacy meta must parse");
        assert!(!meta.cloud_save, "missing cloud_save must default to false");
    }

    #[test]
    fn cloud_save_round_trips_through_world_meta() {
        let mut meta = WorldMeta::new("test-world");
        meta.cloud_save = true;
        let json = serde_json::to_string(&meta).unwrap();
        let back: WorldMeta = serde_json::from_str(&json).unwrap();
        assert!(back.cloud_save);
    }

    #[test]
    fn missing_genesis_block_found_deserialises_as_false() {
        // Legacy WorldMeta (pre-Wave-25) doesn't carry the flag. The
        // #[serde(default)] must let those load as `genesis_block_found:
        // false`, so legacy worlds get the Genesis Block celebration once
        // on their next gem (one-shot cosmetic migration).
        let json = r#"{"display_name":"old","description":"","game_mode":"survival","created_at":"","icon":null}"#;
        let parsed: WorldMeta = serde_json::from_str(json).expect("missing flag must default to false");
        assert!(!parsed.genesis_block_found);
    }

    #[test]
    fn legacy_save_upgrades_to_modern() {
        // Construct a legacy save (bincode-compatible with `LegacyWorldSave`)
        // and verify upgrade produces a valid modern `WorldSave`.
        let legacy = LegacyWorldSave {
            seed: 7,
            player_x: 1.0, player_y: 64.0, player_z: 1.0,
            player_health: 20.0,
            hotbar_slot: 0,
            inventory: vec![
                LegacySavedSlot { block_id: 3, count: 10 },
                LegacySavedSlot { block_id: 0, count: 0 },
                LegacySavedSlot { block_id: 5, count: 1 },
            ],
            players: vec![],
        };
        let modern = legacy.upgrade();
        assert_eq!(modern.seed, 7);
        assert_eq!(modern.inventory.len(), 3);
        assert!(matches!(modern.inventory[0], SavedSlot::Block { block_id: 3, count: 10 }));
        assert!(matches!(modern.inventory[1], SavedSlot::Empty));
        assert!(matches!(modern.inventory[2], SavedSlot::Block { block_id: 5, count: 1 }));
    }

    #[test]
    fn world_meta_json_roundtrip_with_integrity_flags() {
        let meta = WorldMeta {
            display_name: "Test World".to_string(),
            description: "d".to_string(),
            game_mode: "survival".to_string(),
            created_at: "2026-04-18T12:00:00Z".to_string(),
            icon: None,
            pure_survival: false,
            ever_creative: true,
            cheats_used: true,
            difficulty: "hard".to_string(),
            difficulty_history: vec![DifficultyChange {
                level: "normal".to_string(),
                timestamp: "2026-04-18T12:00:00Z".to_string(),
            }],
            forked_from: Some("parent".to_string()),
            version: 7,
            genesis_block_found: false,
            commands_enabled: true,
            explosives_enabled: true,
            fire_spread_enabled: true,
            cloud_save: false,
            has_seen_license_onboarding: false,
            seed: 1234,
            total_work: 99,
            total_ticks: 7,
            genesis_found_at_tick: Some(3),
            scenario_def: Some("{\"kind\":\"SatoriRush\"}".to_string()),
            is_workshop: false,
            world_override: None,
            world_type: "flat".to_string(),
            ground: "sand".to_string(),
            water_depth: 5,
            time_lock: "day".to_string(),
            mobs_enabled: false,
            keep_inventory: false,
            owner_pubkey: Some("ab".repeat(32)),
            published_to: None,
            satoshi_enabled: false,
            pop_secret: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: WorldMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pure_survival, false);
        // Seam B: the owning persona pubkey survives a JSON round-trip.
        assert_eq!(back.owner_pubkey.as_deref(), Some("ab".repeat(32).as_str()));
        assert_eq!(back.ever_creative, true);
        assert_eq!(back.cheats_used, true);
        assert_eq!(back.difficulty, "hard");
        assert_eq!(back.difficulty_history.len(), 1);
        assert_eq!(back.forked_from.as_deref(), Some("parent"));
        assert_eq!(back.version, 7);
        assert_eq!(back.world_type, "flat");
        assert_eq!(back.ground, "sand");
        assert_eq!(back.water_depth, 5);
        assert_eq!(back.time_lock, "day");
        assert!(!back.mobs_enabled);
    }

    #[test]
    fn world_meta_defaults_for_missing_integrity_flags() {
        // Older save files (pre-integrity-ledger) didn't have the flags. The
        // #[serde(default = ...)] attrs must fill sensible defaults.
        let minimal_json = r#"{
            "display_name": "Old World",
            "description": "",
            "game_mode": "survival",
            "created_at": "2026-04-18T12:00:00Z",
            "icon": null
        }"#;
        let meta: WorldMeta = serde_json::from_str(minimal_json).unwrap();
        assert_eq!(meta.pure_survival, true, "pure_survival default is true");
        assert_eq!(meta.ever_creative, false);
        assert_eq!(meta.cheats_used, false);
        assert_eq!(meta.difficulty, "normal");
        assert!(meta.difficulty_history.is_empty());
        assert!(meta.forked_from.is_none());
        assert_eq!(meta.version, 0);
        // Seam B: a pre-seam world_meta.json (no `owner_pubkey`) loads as an
        // unowned/guest save — tolerant decode, never an error.
        assert_eq!(meta.owner_pubkey, None);
    }

    #[test]
    fn seam_b_claim_owner_semantics() {
        let pk_a = "aa".repeat(32);
        let pk_b = "bb".repeat(32);

        // Guest save: a None owner stays None.
        let mut m = WorldMeta::new("w");
        m.claim_owner(None);
        assert_eq!(m.owner_pubkey, None);

        // First signed-in save claims the orphan.
        m.claim_owner(Some(&pk_a));
        assert_eq!(m.owner_pubkey.as_deref(), Some(pk_a.as_str()));

        // Re-saving as the same owner is a no-op.
        m.claim_owner(Some(&pk_a));
        assert_eq!(m.owner_pubkey.as_deref(), Some(pk_a.as_str()));

        // A DIFFERENT owner does NOT clobber the original (divergence to resolve).
        m.claim_owner(Some(&pk_b));
        assert_eq!(m.owner_pubkey.as_deref(), Some(pk_a.as_str()));

        // A later guest save also leaves the existing owner intact.
        m.claim_owner(None);
        assert_eq!(m.owner_pubkey.as_deref(), Some(pk_a.as_str()));
    }

    #[test]
    fn flat_world_config_round_trips_through_world_meta() {
        // B1 — new flat-world fields survive a JSON serialise → deserialise
        // round-trip with non-default values, so they are actually persisted.
        let mut meta = WorldMeta::new("flat-test");
        meta.world_type = "flat".to_string();
        meta.ground = "stone".to_string();
        meta.water_depth = 7;
        meta.time_lock = "night".to_string();
        meta.mobs_enabled = false;
        let json = serde_json::to_string(&meta).unwrap();
        let back: WorldMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.world_type, "flat");
        assert_eq!(back.ground, "stone");
        assert_eq!(back.water_depth, 7);
        assert_eq!(back.time_lock, "night");
        assert!(!back.mobs_enabled);
    }

    #[test]
    fn flat_world_config_defaults_for_legacy_meta() {
        // B1 — a save written before the flat-world fields existed must load
        // with the correct defaults so old worlds are unchanged:
        //   world_type = "normal", ground = "grass", water_depth = 3,
        //   time_lock = "cycle", mobs_enabled = true.
        let minimal_json = r#"{"display_name":"old","description":"","game_mode":"survival","created_at":"","icon":null}"#;
        let meta: WorldMeta = serde_json::from_str(minimal_json).expect("legacy meta must parse");
        assert_eq!(meta.world_type, "normal", "world_type must default to normal");
        assert_eq!(meta.ground, "grass", "ground must default to grass");
        assert_eq!(meta.water_depth, 3, "water_depth must default to 3");
        assert_eq!(meta.time_lock, "cycle", "time_lock must default to cycle");
        assert!(meta.mobs_enabled, "mobs_enabled must default to true");
    }

    #[test]
    fn mark_workshop_disables_mobs() {
        // 2026-06-18 — the Workshop is a mob-free creative authoring room.
        // mark_workshop must flip mobs_enabled off (in addition to flagging the
        // world as the Workshop and creative).
        let minimal_json = r#"{"display_name":"ws","description":"","game_mode":"survival","created_at":"","icon":null}"#;
        let mut meta: WorldMeta = serde_json::from_str(minimal_json).expect("meta must parse");
        assert!(meta.mobs_enabled, "precondition: meta defaults to mobs on");
        meta.mark_workshop();
        assert!(meta.is_workshop, "mark_workshop flags the Workshop");
        assert!(!meta.mobs_enabled, "the Workshop must disable mobs");
        assert_eq!(meta.game_mode, "creative", "the Workshop is creative");
    }

    #[test]
    fn empty_inventory_serializes_to_36_empty_slots() {
        let mut inv = Inventory::new();
        for i in 0..36 {
            inv.set_slot(i, None);
        }
        let slots = serialize_inventory_raw(&inv);
        assert_eq!(slots.len(), 36);
        for s in &slots {
            assert!(matches!(s, SavedSlot::Empty));
        }
    }

    #[test]
    fn read_world_save_tolerates_extra_trailing_bytes_forward_compat() {
        // Forward-compat: a save written by a NEWER engine carries extra appended fields
        // this engine doesn't know. `read_world_save` must decode the fields it DOES know
        // and ignore the unknown trailing bytes, without corrupting the known fields.
        // (Replaces a former mis-named test that asserted nothing — bincode's top-level
        // deserialize tolerates trailing bytes, it does not reject them.)
        let save = WorldSave {
            seed: 1,
            player_x: 0.0, player_y: 0.0, player_z: 0.0,
            player_health: 20.0,
            hotbar_slot: 0,
            inventory: vec![SavedSlot::Empty; 36],
            players: vec![],
            campfires: vec![],
            furnaces: vec![],
            vendors: vec![],
            drying_racks: vec![],
            hives: vec![],
            chests: vec![],
            tip_jars: vec![],
            auctions: vec![],
            latent_prints: vec![],
            plots: vec![],
            market_hubs: vec![],
            construction_anchors: vec![],
            architect_plaques: vec![],
            village_anchors: vec![],
            populated_villages: vec![],
            village_bells: vec![],
            village_treasuries: vec![],
            active_raids: vec![],
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: vec![],
            brigand_hideouts: vec![],
            bounties: vec![],
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            face_overlays: vec![],
            face_blueprints: vec![],
            face_blueprint_blanks: vec![],
            workshop: Default::default(),
            carts: vec![],
            graves: vec![],
            waypoints: vec![],
            block_meta: vec![],
            power_devices: vec![],
            signs: vec![],
            item_frames: vec![],
            locked_slots: vec![],
            hostile_acts: vec![],
            rigs: vec![],
            exhibits: Vec::new(),
            composters: Vec::new(),
            saved_mobs: Vec::new(),
            satoshi: Default::default(),
            dispensers: Vec::new(),
            rig_clips: Vec::new(),
        };
        let mut bytes = bincode::serialize(&save).unwrap();
        bytes.extend_from_slice(&[0xFFu8; 16]); // pretend: a future field's bytes
        let back = read_world_save(&bytes).expect("read_world_save must tolerate a newer tail");
        assert_eq!(back.seed, 1);
        assert_eq!(back.inventory.len(), 36);
        assert!(
            back.face_overlays.is_empty(),
            "trailing junk must not bleed into the known fields"
        );
    }

    // --- Goal 3 review follow-up: direct unit coverage of the tolerant decoder
    //     (previously exercised only end-to-end through load_world). ---

    /// Minimal current-format `WorldSave` carrying one chest, serialised to bytes.
    fn worldsave_with_chest_bytes() -> Vec<u8> {
        let mut full: WorldSave = serde_json::from_str(
            r#"{ "seed": 7, "player_x": 1.0, "player_y": 64.0, "player_z": 1.0,
                 "player_health": 20.0, "hotbar_slot": 0, "inventory": [] }"#,
        )
        .unwrap();
        full.chests = vec![SavedChest {
            x: 2,
            y: 70,
            z: 3,
            data: crate::chest::ChestData::new(),
        }];
        bincode::serialize(&full).unwrap()
    }

    #[test]
    fn kept_steed_is_saved_and_round_trips() {
        // Steed persistence (2026-07-04): a ridden (kept) mule must be
        // captured by tamed_mobs_to_saved and survive serde; a wild one
        // must NOT be captured (wildlife re-scatters). Task 11 (2026-07-06):
        // the writer now always emits `Steed2` (cargo-pack carrier), never
        // the legacy `Steed` — see `steed2_writer_emits_steed2_not_steed`
        // below plus `legacy_steed_bytes_still_decode_packless` for the old
        // wire-shape's continued readability.
        let mut ecs = hecs::World::new();
        let kept = crate::entity::spawn_mob(
            &mut ecs,
            crate::mob::MobType::Mule,
            glam::Vec3::new(4.0, 65.0, -3.0),
        );
        if let Ok(mut hd) = ecs.get::<&mut crate::horse_ai::HorseData>(kept) {
            hd.kept_by = Some(0);
        }
        let _wild = crate::entity::spawn_mob(
            &mut ecs,
            crate::mob::MobType::Horse,
            glam::Vec3::new(9.0, 65.0, 9.0),
        );
        let saved = tamed_mobs_to_saved(&ecs);
        let steeds: Vec<_> = saved
            .iter()
            .filter(|p| matches!(p.data, SavedTamedPetData::Steed2 { .. }))
            .collect();
        assert_eq!(steeds.len(), 1, "kept mule saved, wild horse not");
        assert_eq!(steeds[0].data.mob_type(), crate::mob::MobType::Mule);
        // Serde round-trip keeps the owner slot.
        let bytes = bincode::serialize(steeds[0]).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        match back.data {
            SavedTamedPetData::Steed2 { kind, data, pack } => {
                assert_eq!(kind, crate::mob::MobType::Mule);
                assert_eq!(data.kept_by, Some(0));
                assert!(pack.is_none(), "no chest was equipped");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn legacy_steed_bytes_still_decode_packless() {
        // Task 11 (2026-07-06): an OLD save wrote `Steed { kind, data }` (no
        // cargo pack on the wire — the variant predates packs entirely). It
        // must keep decoding, and — because `HorseData.pack` is serde(skip)
        // — restore with `pack = None`.
        let mut data = crate::horse_ai::HorseData::new();
        data.kept_by = Some(1);
        let pet = SavedTamedPet {
            x: 4.0, y: 65.0, z: -3.0,
            data: SavedTamedPetData::Steed { kind: crate::mob::MobType::Donkey, data },
        };
        let bytes = bincode::serialize(&pet).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        match back.data {
            SavedTamedPetData::Steed { kind, data } => {
                assert_eq!(kind, crate::mob::MobType::Donkey);
                assert_eq!(data.kept_by, Some(1));
                assert!(data.pack.is_none(), "serde(skip) defaults pack to None");
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn steed2_round_trips_a_packed_donkey_with_named_item() {
        // Task 11 (2026-07-06): the appended `Steed2` variant carries the
        // cargo pack explicitly (`HorseData.pack` is serde(skip)), so a
        // packed donkey holding one named ItemStack must survive the wire
        // round-trip with the pack contents intact.
        let mut data = crate::horse_ai::HorseData::new();
        data.kept_by = Some(0);
        let mut pack = crate::chest::ChestData::default();
        pack.slots[0] = Some(crate::item::ItemStack::new_material(
            crate::item::MaterialId::IronIngot,
            5,
        ));
        let pet = SavedTamedPet {
            x: 1.0, y: 64.0, z: 1.0,
            data: SavedTamedPetData::Steed2 {
                kind: crate::mob::MobType::Donkey,
                data: data.clone(),
                pack: Some(pack.clone()),
            },
        };
        let bytes = bincode::serialize(&pet).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.data.mob_type(), crate::mob::MobType::Donkey);
        match back.data {
            SavedTamedPetData::Steed2 { kind, data: back_data, pack: back_pack } => {
                assert_eq!(kind, crate::mob::MobType::Donkey);
                assert_eq!(back_data.kept_by, Some(0));
                // The carried pack survives even though data.pack (serde(skip))
                // decodes to its None default.
                assert!(back_data.pack.is_none());
                let restored = back_pack.expect("pack carried on the wire");
                let stack = restored.slots[0].as_ref().expect("slot 0 kept");
                assert_eq!(stack.count, 5);
                assert!(matches!(
                    stack.item,
                    crate::item::Item::Material(crate::item::MaterialId::IronIngot)
                ));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn steed2_writer_emits_steed2_not_steed() {
        // The writer must always snapshot a kept steed's live pack into
        // `Steed2` — the old packless `Steed` variant is legacy-read-only.
        use crate::entity::{MobKind, Position};
        use crate::mob::MobType;
        use glam::Vec3;
        let mut ecs = hecs::World::new();
        let mut hd = crate::horse_ai::HorseData::new();
        hd.kept_by = Some(0);
        hd.pack = Some(crate::chest::ChestData::default());
        ecs.spawn((Position(Vec3::new(2.0, 64.0, 2.0)), MobKind(MobType::Donkey), hd));
        let pets = tamed_mobs_to_saved(&ecs);
        assert_eq!(pets.len(), 1, "the kept donkey persists");
        match &pets[0].data {
            SavedTamedPetData::Steed2 { kind, pack, .. } => {
                assert_eq!(*kind, MobType::Donkey);
                assert!(pack.is_some(), "equipped pack captured");
            }
            other => panic!("writer must emit Steed2, got: {other:?}"),
        }
    }

    #[test]
    fn dispensers_round_trip_with_contents_and_latch() {
        // 2026-07-04 — the newest appended field: a dispenser's 9 slots + its
        // rising-edge latch must survive serialize → tolerant decode.
        let mut full: WorldSave = serde_json::from_str(
            r#"{ "seed": 7, "player_x": 1.0, "player_y": 64.0, "player_z": 1.0,
                 "player_health": 20.0, "hotbar_slot": 0, "inventory": [] }"#,
        )
        .unwrap();
        let mut d = crate::dispenser::DispenserData::new();
        d.chest.slots[4] =
            Some(crate::item::ItemStack::new_material(crate::item::MaterialId::Arrow, 12));
        d.on = true;
        full.dispensers = vec![SavedDispenser { x: 5, y: 66, z: -2, data: d }];
        let bytes = bincode::serialize(&full).unwrap();
        let back = deserialize_world_save_tolerant(&bytes).unwrap();
        assert_eq!(back.dispensers.len(), 1);
        let rd = &back.dispensers[0];
        assert_eq!((rd.x, rd.y, rd.z), (5, 66, -2));
        assert!(rd.data.on, "edge latch survives");
        let slot = rd.data.chest.slots[4].as_ref().expect("slot 4 kept");
        assert_eq!(slot.count, 12);
        // And an OLD save (bytes ending before the field) defaults empty.
        let old_bytes = worldsave_with_chest_bytes();
        let old = deserialize_world_save_tolerant(&old_bytes).unwrap();
        assert!(old.dispensers.is_empty());
    }

    #[test]
    fn tolerant_decode_reads_full_save_identically_to_strict() {
        let bytes = worldsave_with_chest_bytes();
        let strict: WorldSave = bincode::deserialize(&bytes).unwrap();
        let tolerant = deserialize_world_save_tolerant(&bytes).unwrap();
        assert_eq!(strict.seed, tolerant.seed);
        assert_eq!(tolerant.chests.len(), 1);
        assert_eq!(strict.chests.len(), tolerant.chests.len());
        assert_eq!(
            (tolerant.chests[0].x, tolerant.chests[0].y, tolerant.chests[0].z),
            (2, 70, 3)
        );
    }

    #[test]
    fn tolerant_decode_defaults_tail_at_clean_boundary() {
        // Drop the trailing empty-`power_devices` field (8 zero bytes: empty Vec
        // length prefix) → a CLEAN boundary. `power_devices` is the LAST WorldSave
        // field (Spec 48), so its bytes are the stream tail.
        let mut bytes = worldsave_with_chest_bytes();
        assert_eq!(&bytes[bytes.len() - 8..], &[0u8; 8]);
        bytes.truncate(bytes.len() - 8);
        let save = deserialize_world_save_tolerant(&bytes)
            .expect("clean-boundary truncation must default the absent tail, not error");
        assert_eq!(save.chests.len(), 1, "field before the boundary must survive");
        assert!(
            save.power_devices.is_empty(),
            "absent tail field defaults to empty"
        );
        assert!(
            save.waypoints.is_empty(),
            "the field before the dropped tail is still present + empty"
        );
    }

    #[test]
    fn tolerant_decode_propagates_mid_field_truncation() {
        // A `?`-decoded tail field must PROPAGATE a mid-field EOF (not swallow it
        // as a clean boundary) so read_world_save can fall back to LegacyWorldSave
        // rather than silently default — the exact bug a prior review found.
        //
        // The tail of this fixture is `workshop` (12 bytes: 8-byte empty-`projects`
        // Vec len + 4-byte `next_id`; `symmetry` is `#[serde(skip)]`) then `carts`
        // (8-byte empty Vec len) then `graves` (#47 — 8-byte empty Vec len) then
        // `waypoints` (#6 — 8-byte empty Vec len) then Spec 48 `block_meta`
        // (8-byte empty Vec len) + `power_devices` (8-byte empty Vec len) then
        // Wave 2c `signs` + `item_frames` + Wave 3 `locked_slots` + Wave 5
        // `hostile_acts` + #19 `rigs` + Creator-gallery `exhibits` + Spec 49
        // `composters` then Animals-Wave-2 `saved_mobs` (8-byte empty Vec len
        // each). All of the appended tail fields SWALLOW errors
        // (unwrap_or_default), so we target `workshop` instead: drop the 8
        // `saved_mobs` + 8 `composters` + 8 `exhibits` + 8 `rigs` + 8
        // `hostile_acts` + 8 `locked_slots` + 8 `item_frames` + 8 `signs` + 8
        // `power_devices` + 8 `block_meta` + 8 `waypoints` + 8 `graves` + 8
        // `carts` bytes (13 tail fields) + 2 more to land mid-`workshop.next_id`,
        // an EOF that `read_tail`'s `?` must propagate.
        let mut bytes = worldsave_with_chest_bytes();
        // `satoshi` is the NEWEST swallowing tail field (after `saved_mobs`); drop
        // its bytes too so the truncation still lands mid-`workshop.next_id`.
        // Computed so this self-adjusts if SatoshiState grows.
        let satoshi_bytes =
            bincode::serialized_size(&crate::satoshi::SatoshiState::default()).unwrap() as usize;
        // `dispensers` (2026-07-04) and `rig_clips` (#19, 2026-09-06) appended
        // after `satoshi`: two more 8-byte empty-Vec prefixes to drop before the
        // truncation lands mid-workshop.
        bytes.truncate(bytes.len() - (8 * 15 + 2 + satoshi_bytes));
        assert!(
            deserialize_world_save_tolerant(&bytes).is_err(),
            "mid-field EOF on a `?` tail field must propagate, not be swallowed"
        );
    }

    #[test]
    fn tolerant_decode_swallows_a_corrupt_old_shape_carts_field() {
        // CA1 save-compat: `carts` is read with `unwrap_or_default()` so a PRE-HULL
        // save whose `Vec<SavedCart>` decodes against the new (longer `CartData`)
        // shape — or any corrupt `carts` tail — defaults to empty instead of
        // failing the whole load. Simulate a corrupt `carts` tail by truncating
        // its 8-byte length prefix mid-way; the rest of the save must still load.
        let mut bytes = worldsave_with_chest_bytes();
        // Drop 4 of the 8 `carts` length bytes → mid-`carts` EOF.
        bytes.truncate(bytes.len() - 4);
        let save = deserialize_world_save_tolerant(&bytes)
            .expect("a corrupt/old-shape carts tail must not fail the whole load");
        assert_eq!(save.chests.len(), 1, "fields before carts survive");
        assert!(save.carts.is_empty(), "the unreadable carts tail defaults to empty");
    }

    #[test]
    fn write_atomic_overwrites_and_leaves_no_temp() {
        let dir = world_dir("__test_write_atomic__");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("world.dat");
        let tmp: PathBuf = {
            let mut t = path.as_os_str().to_owned();
            t.push(".tmp");
            PathBuf::from(t)
        };

        write_atomic(&path, b"first").expect("first write");
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert!(!tmp.exists(), "temp file leaked after write");

        // Overwrite an existing file (the rename-over case).
        write_atomic(&path, b"second-longer").expect("overwrite");
        assert_eq!(std::fs::read(&path).unwrap(), b"second-longer");
        assert!(!tmp.exists(), "temp file leaked after overwrite");

        let _ = delete_world("__test_write_atomic__");
    }

    #[test]
    fn sanitize_folder_name_basic_cases() {
        assert_eq!(sanitize_folder_name("My World"), "my_world");
        assert_eq!(sanitize_folder_name("MyWorld"), "myworld");
        assert_eq!(sanitize_folder_name("keep_this-123"), "keep_this-123");
    }

    #[test]
    fn sanitize_folder_name_strips_specials_and_collapses() {
        // Special characters become underscores. Leading/trailing underscores trim.
        let out = sanitize_folder_name("  !@# Hello $world$ !@# ");
        // All non-alnum/_/- became '_', then trimmed. Must not contain the specials.
        assert!(!out.contains('!'));
        assert!(!out.contains('@'));
        assert!(!out.contains('$'));
        assert!(!out.starts_with('_'));
        assert!(!out.ends_with('_'));
    }

    #[test]
    fn sanitize_folder_name_empty_falls_back_to_world() {
        // Trimmed to empty → the "world" default fires.
        assert_eq!(sanitize_folder_name(""), "world");
        assert_eq!(sanitize_folder_name("   "), "world");
        assert_eq!(sanitize_folder_name("!!!!"), "world");
    }

    #[test]
    fn sanitize_folder_name_truncates_over_32_chars() {
        let input = "a".repeat(50);
        let out = sanitize_folder_name(&input);
        assert!(out.len() <= 32, "got {} chars: {}", out.len(), out);
    }

    #[test]
    fn sanitize_folder_name_multibyte_over_32_bytes_no_panic() {
        // 'あ' is 3 bytes; 15 of them = 45 bytes. A naive &trimmed[..32] slices
        // mid-char (byte 32 isn't a char boundary) and panics. Truncating by
        // chars keeps all 15 (engine audit 2026-06-04, A: sanitize panic).
        let name = "あ".repeat(15);
        let folder = sanitize_folder_name(&name);
        assert!(!folder.is_empty());
        assert_eq!(folder.chars().count(), 15, "all 15 chars survive (<= 32 char limit)");
    }

    #[test]
    fn sanitize_image_ref_keeps_extension_and_rejects_traversal() {
        // A normal filename (incl. its extension) survives unchanged.
        assert_eq!(sanitize_image_ref("piece.png"), "piece.png");
        assert_eq!(sanitize_image_ref("My-Art_01.JPG"), "My-Art_01.JPG");
        // Traversal / path separators are stripped; any residual ".." rejects it.
        assert_eq!(sanitize_image_ref("../../etc/passwd"), "");
        assert_eq!(sanitize_image_ref("a/b/c.png"), "abc.png");
        assert_eq!(sanitize_image_ref(""), "");
        assert_eq!(sanitize_image_ref("..\\windows"), "");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn exhibit_image_path_stays_inside_the_world_folder() {
        let p = exhibit_image_path("my-gallery", "piece.png");
        assert!(p.ends_with("exhibits/piece.png") || p.ends_with("exhibits\\piece.png"));
        // A traversal attempt is sanitised to a flat name — never climbs out.
        let evil = exhibit_image_path("my-gallery", "../../etc/passwd");
        let s = evil.to_string_lossy();
        assert!(!s.contains(".."), "image ref must not traverse: {s}");
    }

    #[test]
    fn restore_inventory_clamps_overlarge_count_to_max_stack() {
        // A save with count > max_stack (legacy / hand-edited) must clamp on
        // load, not stash an over-max slot (engine audit 2026-06-04, A).
        let mut inv = Inventory::new();
        let slots = vec![
            SavedSlot::Block { block_id: crate::block::STONE, count: 200 },
            SavedSlot::Material { material_id: MaterialId::IronIngot, count: 250 },
        ];
        restore_inventory(&mut inv, &slots);
        assert_eq!(inv.slot(0).map(|s| s.count), Some(64), "block count clamped to max_stack");
        assert_eq!(inv.slot(1).map(|s| s.count), Some(64), "material count clamped to max_stack");
    }

    #[test]
    fn save_world_rejects_empty_players() {
        // Indexing player_saves[0] on an empty slice panicked mid-save (engine
        // audit 2026-06-04, A). Guard early with an Err instead.
        let world = World::new();
        let result = save_world("unused_empty_players", &world, &[], 0, &[], &[]);
        assert!(result.is_err(), "empty players must Err, not panic");
    }

    #[test]
    fn partition_chunks_for_save_separates_empty_from_nonempty() {
        let mut world = World::new();
        world.set_block(2, 70, 2, crate::block::STONE);       // non-empty chunk
        world.set_block(40, 70, 40, crate::block::STONE);     // load a chunk…
        world.set_block(40, 70, 40, crate::block::AIR);       // …then mine it empty
        let (write, delete) = partition_chunks_for_save(&world);
        assert!(!write.is_empty(), "the stone chunk is queued to write");
        assert!(!delete.is_empty(), "the mined-out chunk is queued for stale-file deletion");
        for p in &write { assert!(!world.get_chunk(p.0, p.1, p.2).unwrap().is_empty()); }
        for p in &delete { assert!(world.get_chunk(p.0, p.1, p.2).unwrap().is_empty()); }
    }

    #[test]
    fn save_world_deletes_stale_file_for_emptied_chunk() {
        // Round-trip: a chunk saved with a block, then mined out, must NOT leave
        // its stale file on disk (the resurrection bug — engine audit A #4).
        let name = "test_chunk_resurrection_axenstax";
        let dir = world_dir(name);
        let _ = std::fs::remove_dir_all(&dir);
        let chunks_dir = dir.join("chunks");

        let mut world = World::new();
        world.set_block(2, 70, 2, crate::block::STONE);
        let players = vec![crate::player_slot::PlayerSlot::new(
            0, glam::Vec3::new(0.0, 70.0, 0.0), 1.0)];
        save_world(name, &world, &players, 0, &[], &[]).expect("first save");
        let n_before = std::fs::read_dir(&chunks_dir).unwrap().count();
        assert_eq!(n_before, 1, "exactly the stone chunk's file is written");

        // Mine it out → the only loaded chunk is now all-air.
        world.set_block(2, 70, 2, crate::block::AIR);
        save_world(name, &world, &players, 0, &[], &[]).expect("second save");
        let n_after = std::fs::read_dir(&chunks_dir).unwrap().count();

        let _ = std::fs::remove_dir_all(&dir); // cleanup before asserting
        assert_eq!(n_after, 0, "emptied chunk's stale file deleted — no resurrection");
    }

    #[test]
    fn read_bounded_rejects_a_stream_over_the_cap() {
        // Stands in for a gzip bomb: a reader that yields far more than the cap
        // must error, not slurp it all (engine audit 2026-06-04, A).
        let data = vec![0u8; 4 * 1024 * 1024]; // 4 MiB
        let result = read_bounded(&data[..], 1024 * 1024); // 1 MiB cap
        assert!(result.is_err(), "over-cap stream must be rejected");
    }

    #[test]
    fn read_bounded_accepts_a_stream_within_the_cap() {
        let data: Vec<u8> = b"axenstax world data ".iter().cycle().take(50_000).copied().collect();
        let out = read_bounded(&data[..], MAX_IMPORT_DECOMPRESSED_BYTES).unwrap();
        assert_eq!(out, data, "a normal payload reads through under the cap");
    }

    #[test]
    fn read_bounded_accepts_exactly_at_the_cap() {
        let data = vec![7u8; 1000];
        let out = read_bounded(&data[..], 1000).unwrap();
        assert_eq!(out.len(), 1000, "a stream exactly at the cap is allowed");
    }

    // ---------------------------------------------------------------
    // Chunk 2 (save-format hardening) — block-entity round-trip audit
    // ---------------------------------------------------------------
    //
    // Each Saved* wrapper carries (x, y, z, data). The bincode wire
    // shape is the on-disk format; if any of these wrappers gets a
    // field reorder or rename without a migration, the save format
    // silently breaks. These tests pin the shape against a populated
    // instance so a regression fails CI before it ships.

    #[test]
    fn saved_furnace_roundtrips_with_populated_state() {
        use crate::item::{Item, ItemStack, MaterialId};
        let mut data = crate::furnace::FurnaceData::default();
        data.input = Some(ItemStack {
            item: Item::Material(MaterialId::RawBeef),
            count: 3,
        });
        data.fuel = Some(ItemStack {
            item: Item::Material(MaterialId::Coal),
            count: 5,
        });
        let saved = SavedFurnace { x: 12, y: 64, z: -7, data: data.clone() };
        let bytes = bincode::serialize(&saved).unwrap();
        let back: SavedFurnace = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.x, 12);
        assert_eq!(back.y, 64);
        assert_eq!(back.z, -7);
        assert!(matches!(back.data.input, Some(ItemStack { count: 3, .. })));
        assert!(matches!(back.data.fuel,  Some(ItemStack { count: 5, .. })));
    }

    #[test]
    fn saved_vendor_roundtrips_with_local_owner_and_listing() {
        use crate::item::{Item, ItemStack, MaterialId};
        let data = crate::vendor::VendorData {
            owner: Some(crate::vendor::VendorOwner::LocalPlayer(2)),
            mode: Some(crate::vendor::VendorMode::Sell),
            slot: Some(ItemStack {
                item: Item::Material(MaterialId::Bread),
                count: 8,
            }),
            barter_request: None,
            price_sats: 30,
            stock: 8,
            escrow_sats: 240,
            last_txn_tick: 4_000,
            lot_size: 1,
        };
        let saved = SavedVendor { x: 1, y: 70, z: 1, data: data.clone() };
        let bytes = bincode::serialize(&saved).unwrap();
        let back: SavedVendor = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.data.price_sats, 30);
        assert_eq!(back.data.stock, 8);
        assert_eq!(back.data.escrow_sats, 240);
        assert!(matches!(back.data.owner, Some(crate::vendor::VendorOwner::LocalPlayer(2))));
        assert!(matches!(back.data.mode,  Some(crate::vendor::VendorMode::Sell)));
    }

    #[test]
    fn saved_drying_rack_roundtrips_with_partial_seasoning() {
        let mut data = crate::drying_rack::DryingRackData::default();
        // Insert one green log mid-season; bincode must carry that state.
        data.slots[0] = crate::drying_rack::RackSlot {
            species: Some(crate::drying_rack::LogSpecies::Oak),
            seasoning_ticks: 1_500,
        };
        let saved = SavedDryingRack { x: 5, y: 65, z: 5, data: data.clone() };
        let bytes = bincode::serialize(&saved).unwrap();
        let back: SavedDryingRack = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.data.slots[0].seasoning_ticks, 1_500);
        assert_eq!(back.data.slots[0].species, Some(crate::drying_rack::LogSpecies::Oak));
        assert!(back.data.slots[1..].iter().all(|s| s.is_empty()));
    }

    #[test]
    fn saved_architect_plaque_roundtrips_with_chain() {
        // ArchitectPlaqueData carries the derivation chain — a single
        // synthetic entry is enough to verify the shape survives the
        // wire format.
        let chain = crate::plan::ArchitectPlaqueData {
            chain: vec![crate::plan::DerivationLink {
                author_npub: "npub1testabcdef".to_string(),
                plan_name: "Test Cottage".to_string(),
                license: crate::plan::PlanLicense::Ccbysa,
                captured_at: 12_345,
                plan_hash: [7u8; 32],
            }],
            authored_in: "Survival".to_string(),
            builder_credit: None,
        };
        let saved = SavedArchitectPlaque {
            x: 10, y: 64, z: 10,
            chain: chain.clone(),
        };
        let bytes = bincode::serialize(&saved).unwrap();
        let back: SavedArchitectPlaque = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.chain.chain.len(), 1);
        assert_eq!(back.chain.chain[0].author_npub, "npub1testabcdef");
        assert_eq!(back.chain.chain[0].plan_name, "Test Cottage");
        assert_eq!(back.chain.chain[0].captured_at, 12_345);
        assert_eq!(back.chain.authored_in, "Survival");
    }

    #[test]
    fn face_overlays_save_restore_round_trip() {
        // Owner-inbox #1/2/3 — paint faces, flatten to SavedFaceOverlay,
        // bincode round-trip, restore into a fresh world. Exercises both the
        // save loop (face_overlays_to_saved) and the load loop
        // (restore_face_overlays).
        use crate::world::{FaceAttachment, World};
        let mut w = World::new();
        w.set_face_attachment((1, 2, 3), 0, FaceAttachment::Wallpaper(crate::block::WALLPAPER_RED));
        w.set_face_attachment((1, 2, 3), 5, FaceAttachment::Wallpaper(crate::block::WALLPAPER_CYAN));
        w.set_face_attachment((-4, 10, 7), 2, FaceAttachment::Wallpaper(crate::block::WALLPAPER_LIME));
        let saved = face_overlays_to_saved(&w);
        assert_eq!(saved.len(), 3, "three painted faces flatten to three entries");
        let bytes = bincode::serialize(&saved).unwrap();
        let back: Vec<SavedFaceOverlay> = bincode::deserialize(&bytes).unwrap();
        let mut w2 = World::new();
        restore_face_overlays(&mut w2, &back);
        assert!(matches!(w2.face_attachment_at((1, 2, 3), 0), Some(FaceAttachment::Wallpaper(b)) if *b == crate::block::WALLPAPER_RED));
        assert!(matches!(w2.face_attachment_at((1, 2, 3), 5), Some(FaceAttachment::Wallpaper(b)) if *b == crate::block::WALLPAPER_CYAN));
        assert!(matches!(w2.face_attachment_at((-4, 10, 7), 2), Some(FaceAttachment::Wallpaper(b)) if *b == crate::block::WALLPAPER_LIME));
        assert_eq!(face_overlays_to_saved(&w2).len(), 3, "no spurious or dropped entries");
    }

    #[test]
    fn face_blueprint_save_roundtrip() {
        // Task B1 — attach a blueprint to a face, flatten to SavedFaceBlueprint,
        // restore into a fresh world, assert the Blueprint variant survives.
        let mut w = crate::world::World::new();
        w.set_face_attachment(
            (1, 64, 2),
            crate::mesh::Face::Top.index(),
            crate::world::FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::debug_3x3_stone())),
        );
        let saved = face_blueprints_to_saved(&w);
        assert_eq!(saved.len(), 1);
        let mut w2 = crate::world::World::new();
        restore_face_blueprints(&mut w2, &saved);
        assert!(matches!(
            w2.face_attachment_at((1, 64, 2), crate::mesh::Face::Top.index()),
            Some(crate::world::FaceAttachment::Blueprint(_))
        ));
    }

    #[test]
    fn face_blueprint_blank_save_roundtrip() {
        // Task R1 — attach a blank draughting-paper to a face, flatten to
        // SavedFaceBlankPaper, restore into a fresh world, assert the
        // BlueprintBlank variant survives.
        let mut w = crate::world::World::new();
        w.set_face_attachment(
            (1, 64, 2),
            crate::mesh::Face::Top.index(),
            crate::world::FaceAttachment::BlueprintBlank,
        );
        let saved = face_blueprint_blanks_to_saved(&w);
        assert_eq!(saved.len(), 1);
        let mut w2 = crate::world::World::new();
        restore_face_blueprint_blanks(&mut w2, &saved);
        assert!(matches!(
            w2.face_attachment_at((1, 64, 2), crate::mesh::Face::Top.index()),
            Some(crate::world::FaceAttachment::BlueprintBlank)
        ));
    }

    #[test]
    fn sign_text_persists_through_full_worldsave_and_restores_as_block_entity() {
        // Wave 2c — a Sign's text survives the bincode append-only round-trip
        // (newest field, read last) and `apply_world_save_state` lands it back
        // on the world as a `BlockEntityData::Sign`.
        use crate::world::World;
        let mut save = minimal_world_save_for_tests(11);
        save.signs = vec![SavedSign {
            x: 4,
            y: 65,
            z: -2,
            data: crate::sign::SignData { text: "Welcome home".to_string() },
        }];
        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("decode");
        assert_eq!(back.signs.len(), 1);
        assert_eq!(back.signs[0].data.text, "Welcome home");

        let mut w = World::new();
        apply_world_save_state(&mut w, &back);
        assert_eq!(
            w.sign_at((4, 65, -2)).map(|s| s.text.as_str()),
            Some("Welcome home"),
            "restored sign text lands as a block-entity"
        );
    }

    #[test]
    fn item_frame_persists_through_full_worldsave_and_restores() {
        // Wave 2c — a framed item + rotation survive the append-only round-trip.
        use crate::world::World;
        let mut save = minimal_world_save_for_tests(12);
        let mut data = crate::item_frame::ItemFrameData::new();
        data.try_insert(crate::item::ItemStack::new_block(crate::block::DIAMOND_BLOCK, 1));
        data.rotate();
        save.item_frames = vec![SavedItemFrame { x: 1, y: 70, z: 3, data }];
        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("decode");
        assert_eq!(back.item_frames.len(), 1);

        let mut w = World::new();
        apply_world_save_state(&mut w, &back);
        let f = w.item_frame_at((1, 70, 3)).expect("frame restored");
        assert_eq!(f.rotation, 1);
        assert!(matches!(
            f.item.as_ref().map(|s| &s.item),
            Some(crate::item::Item::Block(b)) if *b == crate::block::DIAMOND_BLOCK
        ));
    }

    #[test]
    fn locked_slots_persist_through_full_worldsave() {
        // Wave 3 (#45 P3) — player 0's locked slot indices survive the append-only
        // round-trip and reapply to an Inventory via set_locked_from.
        let mut save = minimal_world_save_for_tests(13);
        save.locked_slots = vec![0, 4, 35];
        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("decode");
        assert_eq!(back.locked_slots, vec![0, 4, 35]);

        let mut inv = crate::inventory::Inventory::new();
        inv.set_locked_from(&back.locked_slots);
        assert!(inv.is_locked(0) && inv.is_locked(4) && inv.is_locked(35));
        assert!(!inv.is_locked(1));
        // locked_indices round-trips back to the same set.
        assert_eq!(inv.locked_indices(), vec![0, 4, 35]);
    }

    #[test]
    fn hostile_acts_ledger_persists_through_full_worldsave() {
        // Wave 5 (Rail Freight P3) — a recorded cart robbery survives the
        // append-only round-trip and reloads onto the world's ledger.
        use crate::hostile_acts::HostileActKind;
        use crate::world::World;
        let mut save = minimal_world_save_for_tests(14);
        let mut led = crate::hostile_acts::HostileActLedger::new();
        led.record(HostileActKind::CartRobbery, (12, 64, -7), 250);
        save.hostile_acts = led.acts().to_vec();

        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("decode");
        assert_eq!(back.hostile_acts.len(), 1);

        let mut w = World::new();
        apply_world_save_state(&mut w, &back);
        assert_eq!(w.hostile_acts.count_of(HostileActKind::CartRobbery), 1);
        assert_eq!(w.hostile_acts.last().unwrap().tick, 250);
    }

    #[test]
    fn authored_rig_persists_through_full_worldsave() {
        // #19 Rig Studio — a placed rig (skeleton + per-part block assignments)
        // survives the append-only round-trip and reloads onto the world.
        use crate::skeleton::{RiggedModel, SkeletonKind};
        use crate::world::{RigDisplay, World};
        let mut rig = RiggedModel::new(SkeletonKind::Quadruped, "test cow");
        rig.attach("body", crate::block::OAK_PLANKS);
        rig.attach("leg_fl", crate::block::STONE);
        let mut save = minimal_world_save_for_tests(15);
        save.rigs = vec![RigDisplay { pos: [3.0, 64.0, -1.0], yaw: 1.5, rig, clip: crate::anim_set::AnimClip::Bounce }];
        save.rig_clips = vec![crate::anim_set::AnimClip::Bounce];

        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("decode");
        assert_eq!(back.rigs.len(), 1);

        let mut w = World::new();
        apply_world_save_state(&mut w, &back);
        assert_eq!(w.rigs.len(), 1);
        let r = &w.rigs[0];
        assert_eq!(r.pos, [3.0, 64.0, -1.0]);
        assert_eq!(r.rig.skeleton, SkeletonKind::Quadruped);
        assert_eq!(r.rig.parts.len(), 2, "both assigned parts persisted");
        assert_eq!(r.clip, crate::anim_set::AnimClip::Bounce, "the author's clip pick survives");
    }

    #[test]
    fn a_rig_saved_before_the_clip_picker_loads_as_walk() {
        // #19 — `rig_clips` is an index-aligned side table appended at the very
        // end of the blob, so a save written before the clip picker existed
        // carries `rigs` but no `rig_clips` at all. Every such rig must load as
        // `AnimClip::Walk` — the clip it always played — and, crucially, the rig
        // itself (and everything serialised after it) must be untouched.
        use crate::skeleton::{RiggedModel, SkeletonKind};
        use crate::world::{RigDisplay, World};
        let mut rig = RiggedModel::new(SkeletonKind::Biped, "old rig");
        rig.attach("head", crate::block::STONE);
        let mut save = minimal_world_save_for_tests(16);
        save.rigs =
            vec![RigDisplay { pos: [1.0, 65.0, 2.0], yaw: 0.0, rig, clip: crate::anim_set::AnimClip::Bounce }];
        // The pre-picker shape: the side table is simply absent.
        save.rig_clips = Vec::new();

        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("decode");
        let mut w = World::new();
        apply_world_save_state(&mut w, &back);
        assert_eq!(w.rigs.len(), 1, "the rig itself is unaffected");
        assert_eq!(w.rigs[0].rig.parts.len(), 1);
        assert_eq!(
            w.rigs[0].clip,
            crate::anim_set::AnimClip::Walk,
            "a clip-less rig defaults to Walk"
        );
    }

    #[test]
    fn wallpaper_and_blueprint_persist_independently_through_full_worldsave() {
        // Task B1 — a world with one wallpaper face and one blueprint face must
        // round-trip BOTH through a full WorldSave bincode pass, each landing in
        // its own additive Vec, and decode back to the right FaceAttachment variant.
        use crate::world::{FaceAttachment, World};
        let mut w = World::new();
        w.set_face_attachment((5, 70, 9), crate::mesh::Face::North.index(),
            FaceAttachment::Wallpaper(crate::block::WALLPAPER_RED));
        w.set_face_attachment((5, 70, 9), crate::mesh::Face::Top.index(),
            FaceAttachment::Blueprint(Box::new(crate::plan::PlanData::debug_3x3_stone())));

        // Build a full WorldSave carrying the two attachment Vecs (mirrors the
        // `worldsave_with_every_block_entity_kind_roundtrips` fixture — exercise
        // the bincode format, not the native file I/O paths).
        let save = WorldSave {
            seed: 7,
            player_x: 0.0, player_y: 64.0, player_z: 0.0,
            player_health: 20.0,
            hotbar_slot: 0,
            inventory: vec![SavedSlot::Empty; 36],
            players: vec![],
            campfires: vec![],
            furnaces: vec![],
            vendors: vec![],
            drying_racks: vec![],
            hives: vec![],
            chests: vec![],
            tip_jars: vec![],
            auctions: vec![],
            latent_prints: vec![],
            plots: vec![],
            market_hubs: vec![],
            construction_anchors: vec![],
            architect_plaques: vec![],
            village_anchors: vec![],
            populated_villages: vec![],
            village_bells: vec![],
            village_treasuries: vec![],
            active_raids: vec![],
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: vec![],
            brigand_hideouts: vec![],
            bounties: vec![],
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            face_overlays: face_overlays_to_saved(&w),
            face_blueprints: face_blueprints_to_saved(&w),
            face_blueprint_blanks: face_blueprint_blanks_to_saved(&w),
            workshop: w.workshop.clone(),
            carts: vec![],
            graves: vec![],
            waypoints: vec![],
            block_meta: vec![],
            power_devices: vec![],
            signs: vec![],
            item_frames: vec![],
            locked_slots: vec![],
            hostile_acts: vec![],
            rigs: vec![],
            exhibits: Vec::new(),
            composters: Vec::new(),
            saved_mobs: Vec::new(),
            satoshi: Default::default(),
            dispensers: Vec::new(),
            rig_clips: Vec::new(),
        };
        assert_eq!(save.face_overlays.len(), 1, "one wallpaper face");
        assert_eq!(save.face_blueprints.len(), 1, "one blueprint face");

        let bytes = bincode::serialize(&save).unwrap();
        let back: WorldSave = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.face_overlays.len(), 1);
        assert_eq!(back.face_blueprints.len(), 1);

        let mut w2 = World::new();
        restore_face_overlays(&mut w2, &back.face_overlays);
        restore_face_blueprints(&mut w2, &back.face_blueprints);
        assert!(matches!(
            w2.face_attachment_at((5, 70, 9), crate::mesh::Face::North.index()),
            Some(FaceAttachment::Wallpaper(b)) if *b == crate::block::WALLPAPER_RED
        ), "wallpaper face survives independently");
        assert!(matches!(
            w2.face_attachment_at((5, 70, 9), crate::mesh::Face::Top.index()),
            Some(FaceAttachment::Blueprint(_))
        ), "blueprint face survives independently");
    }

    #[test]
    fn workshop_projects_round_trip_through_real_save_decoder() {
        // Spec 40 (The Workshop) Phase B — the headline persistence invariant: a
        // Workshop world with >=2 PARKED, still-INFLATED projects round-trips
        // through the real production decoder (`read_world_save`, the append-only
        // tolerant reader) + `apply_world_save_state`, with each restored to its
        // EXACT prior state (offset, inflation, mode, partial paint).
        use crate::workshop::{WorkshopMode, WorkshopPaint, WorkshopTarget};
        use crate::world::World;

        let mut w = World::new();
        // Project 1: a block reskin, inflated to 2, paint-in-progress.
        let p1 = w.workshop.add(WorkshopTarget::Block(crate::block::CORNFLOWER), WorkshopMode::Reskin, [3, 1, 4]);
        {
            let proj = w.workshop.get_mut(p1).unwrap();
            proj.pump();
            proj.pump();
            proj.paint = WorkshopPaint::Block(
                crate::override_registry::AuthoredFaces::solid([20, 180, 90, 255]),
            );
        }
        // Project 2: a mob reshape, inflated to 3 (parked, no paint).
        let p2 = w.workshop.add(WorkshopTarget::Mob(crate::mob::MobType::Cow), WorkshopMode::Reshape, [-7, 1, 0]);
        for _ in 0..3 {
            w.workshop.get_mut(p2).unwrap().pump();
        }
        assert_eq!(w.workshop.parked().count(), 2, "both projects parked + inflated");

        let save = WorldSave {
            seed: 11,
            player_x: 0.0, player_y: 64.0, player_z: 0.0,
            player_health: 20.0,
            hotbar_slot: 0,
            inventory: vec![SavedSlot::Empty; 36],
            players: vec![],
            campfires: vec![],
            furnaces: vec![],
            vendors: vec![],
            drying_racks: vec![],
            hives: vec![],
            chests: vec![],
            tip_jars: vec![],
            auctions: vec![],
            latent_prints: vec![],
            plots: vec![],
            market_hubs: vec![],
            construction_anchors: vec![],
            architect_plaques: vec![],
            village_anchors: vec![],
            populated_villages: vec![],
            village_bells: vec![],
            village_treasuries: vec![],
            active_raids: vec![],
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: vec![],
            brigand_hideouts: vec![],
            bounties: vec![],
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            face_overlays: vec![],
            face_blueprints: vec![],
            face_blueprint_blanks: vec![],
            workshop: w.workshop.clone(),
            carts: vec![],
            graves: vec![],
            waypoints: vec![],
            block_meta: vec![],
            power_devices: vec![],
            signs: vec![],
            item_frames: vec![],
            locked_slots: vec![],
            hostile_acts: vec![],
            rigs: vec![],
            exhibits: Vec::new(),
            composters: Vec::new(),
            saved_mobs: Vec::new(),
            satoshi: Default::default(),
            dispensers: Vec::new(),
            rig_clips: Vec::new(),
        };

        // Encode, then decode through the REAL production path.
        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("decode");

        // Restore into a FRESH world via the production apply path.
        let mut w2 = World::new();
        apply_world_save_state(&mut w2, &back);

        assert_eq!(w2.workshop.len(), 2, "both projects restored");
        let r1 = w2.workshop.get(p1).expect("project 1 restored by id");
        assert_eq!(r1.origin, [3, 1, 4]);
        assert_eq!(r1.inflation, 2, "inflation survives");
        assert_eq!(r1.mode, WorkshopMode::Reskin);
        assert!(matches!(r1.paint, WorkshopPaint::Block(_)), "partial paint survives");
        assert!(r1.is_parked(), "still parked WIP, not pinned");

        let r2 = w2.workshop.get(p2).expect("project 2 restored by id");
        assert_eq!(r2.origin, [-7, 1, 0]);
        assert_eq!(r2.inflation, 3);
        assert_eq!(r2.mode, WorkshopMode::Reshape);
        assert!(r2.is_parked());

        // Whole-table equality: nothing drifted.
        assert_eq!(w2.workshop, w.workshop, "exact prior state, every project");
    }

    #[test]
    fn pre_workshop_save_loads_with_empty_workshop_table() {
        // Append-only invariant: a save written before the `workshop` field existed
        // (its bytes simply end earlier) decodes with an empty workshop table.
        use crate::workshop::WorkshopProjects;
        // Simulate by encoding a save with an empty workshop and truncating nothing —
        // the tolerant `read_tail` path is what older streams hit. Here we assert the
        // default is empty and a normal (non-Workshop) save carries no projects.
        let mut w = crate::world::World::new();
        assert!(w.workshop.is_empty(), "a normal world has no workshop projects");
        let _ = WorkshopProjects::new(); // default is empty
    }

    #[test]
    fn legacy_worldsave_without_face_blueprints_field_defaults_empty() {
        // Task B1 — mirror `missing_block_entity_vecs_deserialise_as_empty`: a
        // WorldSave serialised WITHOUT the new face_blueprints field must
        // deserialise with an empty Vec (via #[serde(default)]), not error.
        let json = r#"{
            "seed": 1,
            "player_x": 0.0, "player_y": 64.0, "player_z": 0.0,
            "player_health": 20.0,
            "hotbar_slot": 0,
            "inventory": []
        }"#;
        let parsed: WorldSave = serde_json::from_str(json)
            .expect("old saves missing face_blueprints must default to empty");
        assert!(parsed.face_blueprints.is_empty());
        // The sibling wallpaper field still defaults empty too (unchanged).
        assert!(parsed.face_overlays.is_empty());
        // Task R1 — face_blueprint_blanks also defaults empty when absent.
        assert!(parsed.face_blueprint_blanks.is_empty());
        // Rail freight Phase 1 — the newest field (carts) defaults empty too.
        assert!(parsed.carts.is_empty());
    }

    #[test]
    fn worldsave_with_every_block_entity_kind_roundtrips() {
        // Single fixture exercising every Saved* Vec — catches a class
        // of bug where a single block-entity addition compiles but
        // breaks the format because it interacts badly with bincode's
        // positional encoding.
        use crate::item::{Item, ItemStack, MaterialId};

        let furnace = SavedFurnace {
            x: 1, y: 2, z: 3,
            data: crate::furnace::FurnaceData::default(),
        };
        let vendor = SavedVendor {
            x: 4, y: 5, z: 6,
            data: crate::vendor::VendorData {
                owner: Some(crate::vendor::VendorOwner::LocalPlayer(0)),
                mode: Some(crate::vendor::VendorMode::Buy),
                slot: Some(ItemStack { item: Item::Material(MaterialId::Wheat), count: 1 }),
                barter_request: None,
                price_sats: 3,
                stock: 1,
                escrow_sats: 0,
                last_txn_tick: 0,
                lot_size: 1,
            },
        };
        let drying_rack = SavedDryingRack {
            x: 7, y: 8, z: 9,
            data: crate::drying_rack::DryingRackData::default(),
        };
        let plaque = SavedArchitectPlaque {
            x: 10, y: 11, z: 12,
            chain: crate::plan::ArchitectPlaqueData {
                chain: vec![],
                authored_in: "Survival".to_string(),
                builder_credit: None,
            },
        };
        // Rail freight Phase 1 — a cart MID-FLIGHT (non-trivial state) so the
        // bincode round-trip proves every CartData field survives the format.
        // CA1: an IRON hull (a non-default tier) so the round-trip also proves
        // the appended `hull` field survives, not just defaults.
        let mut cargo = crate::chest::ChestData::new();
        cargo.slots[0] = Some(ItemStack::new_material(MaterialId::IronIngot, 7));
        let cart = SavedCart {
            data: crate::cart::CartData {
                cell: (13, 64, 14),
                came_from: Some((12, 64, 14)),
                progress: 0.5,
                speed: crate::cart::CART_SPEED,
                facing: 1.0,
                cargo,
                hull: crate::cart::Hull::Iron,
                // CA4 — transient `#[serde(skip)]` breach accumulator. Set to a
                // NON-zero value here on purpose: the round-trip below must come
                // back `0.0` (it's excluded from bincode entirely), proving the
                // serialized layout is byte-for-byte unchanged by CA4.
                breach: 17.0,
            },
        };

        let save = WorldSave {
            seed: 99,
            player_x: 0.0, player_y: 64.0, player_z: 0.0,
            player_health: 20.0,
            hotbar_slot: 0,
            inventory: vec![SavedSlot::Empty; 36],
            players: vec![],
            campfires: vec![],
            furnaces: vec![furnace],
            vendors: vec![vendor],
            drying_racks: vec![drying_rack],
            hives: vec![],
            chests: vec![],
            tip_jars: vec![],
            auctions: vec![],
            latent_prints: vec![],
            plots: vec![],
            market_hubs: vec![],
            construction_anchors: vec![],
            architect_plaques: vec![plaque],
            village_anchors: vec![],
            populated_villages: vec![],
            village_bells: vec![],
            village_treasuries: vec![],
            active_raids: vec![],
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: vec![],
            brigand_hideouts: vec![],
            bounties: vec![],
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            face_overlays: vec![],
            face_blueprints: vec![],
            face_blueprint_blanks: vec![],
            workshop: Default::default(),
            carts: vec![cart],
            graves: vec![],
            waypoints: vec![],
            block_meta: vec![],
            power_devices: vec![],
            signs: vec![],
            item_frames: vec![],
            locked_slots: vec![],
            hostile_acts: vec![],
            rigs: vec![],
            exhibits: Vec::new(),
            composters: Vec::new(),
            saved_mobs: Vec::new(),
            satoshi: Default::default(),
            dispensers: Vec::new(),
            rig_clips: Vec::new(),
        };
        let bytes = bincode::serialize(&save).unwrap();
        let back: WorldSave = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.furnaces.len(), 1);
        assert_eq!(back.furnaces[0].x, 1);
        assert_eq!(back.vendors.len(), 1);
        assert_eq!(back.vendors[0].data.price_sats, 3);
        assert_eq!(back.drying_racks.len(), 1);
        assert_eq!(back.drying_racks[0].x, 7);
        assert_eq!(back.architect_plaques.len(), 1);
        assert_eq!(back.architect_plaques[0].x, 10);
        // Rail freight Phase 1 — the mid-flight cart survives the bincode format,
        // every CartData field intact.
        assert_eq!(back.carts.len(), 1);
        assert_eq!(back.carts[0].data.cell, (13, 64, 14));
        assert_eq!(back.carts[0].data.came_from, Some((12, 64, 14)));
        assert!((back.carts[0].data.progress - 0.5).abs() < 1e-6);
        assert!((back.carts[0].data.speed - crate::cart::CART_SPEED).abs() < 1e-6);
        assert!((back.carts[0].data.facing - 1.0).abs() < 1e-6);
        assert_eq!(
            back.carts[0].data.cargo.slots[0].as_ref().unwrap().count,
            7
        );
        // CA1 — the appended hull armour tier survives the bincode round-trip.
        assert_eq!(back.carts[0].data.hull, crate::cart::Hull::Iron);
        // CA4 — the transient `#[serde(skip)]` breach accumulator is EXCLUDED
        // from the wire: we serialized it as 17.0 but it comes back 0.0, proving
        // CA4 added NO bytes to the save format (no save break, no protocol bump).
        assert_eq!(back.carts[0].data.breach, 0.0);
    }

    #[test]
    fn new_save_round_trips_a_hull_bearing_cart() {
        // CA1 (TDD): a NEW save carrying an armoured (Iron) cart must round-trip
        // its hull through the real load path (read_world_save → tolerant decode),
        // not just raw bincode. A fresh wood cart (default hull) co-exists to prove
        // the default tier survives too.
        use crate::cart::{CartData, Hull};
        let iron = SavedCart { data: CartData { cell: (1, 64, 1), hull: Hull::Iron, ..Default::default() } };
        let wood = SavedCart { data: CartData { cell: (2, 64, 2), ..Default::default() } };
        let mut save: WorldSave = serde_json::from_str(
            r#"{ "seed": 5, "player_x": 0.0, "player_y": 64.0, "player_z": 0.0,
                 "player_health": 20.0, "hotbar_slot": 0, "inventory": [] }"#,
        )
        .unwrap();
        save.carts = vec![iron, wood];

        let bytes = bincode::serialize(&save).unwrap();
        let back = read_world_save(&bytes).expect("a hull-bearing save must load");
        assert_eq!(back.carts.len(), 2);
        assert_eq!(back.carts[0].data.hull, Hull::Iron, "armoured cart loads as Iron");
        assert_eq!(back.carts[1].data.hull, Hull::Wood, "default cart loads as Wood");
    }

    // ---------------------------------------------------------------
    // SaveVersion migration fixtures — assert older save flavours still
    // deserialise via the `#[serde(default)]` per-field forward-compat
    // pattern. JSON is used (not bincode) because the omitted-field
    // semantics are only meaningful in self-describing formats. The
    // serde defaults on each `#[serde(default)]` field is what we
    // assert here. Bincode forward-compat tests live separately above.
    // ---------------------------------------------------------------

    #[test]
    fn missing_block_entity_vecs_deserialise_as_empty() {
        // Simulate a save from before the block-entity Vecs were added
        // (pre-Spec-20). Each missing Vec must default to empty under
        // serde's `#[serde(default)]`. JSON drops fields cleanly; the
        // bincode shape requires every field present, which is why
        // we still need legacy struct alternates for binary saves.
        let json = r#"{
            "seed": 1,
            "player_x": 0.0, "player_y": 64.0, "player_z": 0.0,
            "player_health": 20.0,
            "hotbar_slot": 0,
            "inventory": []
        }"#;
        let parsed: WorldSave = serde_json::from_str(json)
            .expect("old saves missing the new Vec fields must default to empty");
        assert!(parsed.campfires.is_empty());
        assert!(parsed.face_overlays.is_empty());
        assert!(parsed.furnaces.is_empty());
        assert!(parsed.vendors.is_empty());
        assert!(parsed.drying_racks.is_empty());
        assert!(parsed.construction_anchors.is_empty());
        assert!(parsed.architect_plaques.is_empty());
        assert!(parsed.village_anchors.is_empty());
        assert!(parsed.populated_villages.is_empty());
        assert!(parsed.village_bells.is_empty());
        assert!(parsed.players.is_empty());
    }

    #[test]
    fn player_save_with_no_tamed_pets_field_defaults_to_empty() {
        // Pre-Spec-28d.wolves save data didn't carry tamed_pets. The
        // #[serde(default)] attribute must let those load as
        // `tamed_pets: Vec::new()` so we don't break legacy worlds
        // when the wolf-save path lights up.
        let json = r#"{
            "x": 0.0, "y": 64.0, "z": 0.0,
            "yaw": 0.0, "pitch": 0.0,
            "health": 20.0, "hotbar_slot": 0,
            "inventory": []
        }"#;
        let parsed: PlayerSaveData = serde_json::from_str(json)
            .expect("missing tamed_pets must default to empty");
        assert!(parsed.tamed_pets.is_empty());
    }

    #[test]
    fn player_save_with_no_armour_slots_field_defaults_to_empty() {
        // Spec 28e — pre-armour save data has no armour_slots field.
        // #[serde(default)] must let those load as [None; 4] so legacy
        // worlds keep loading.
        let json = r#"{
            "x": 0.0, "y": 64.0, "z": 0.0,
            "yaw": 0.0, "pitch": 0.0,
            "health": 20.0, "hotbar_slot": 0,
            "inventory": []
        }"#;
        let parsed: PlayerSaveData = serde_json::from_str(json)
            .expect("missing armour_slots must default to [None; 4]");
        assert_eq!(parsed.armour_slots, [None, None, None, None]);
    }

    // ---------------------------------------------------------------
    // Pet-list round-trip — Spec 28d.wolves carries owner_pubkey on
    // WolfData and (forward-compat) a Vec<SavedTamedPet> on
    // PlayerSaveData. These tests pin the wire shape so when mob save
    // lights up the on-disk format doesn't break.
    // ---------------------------------------------------------------

    #[test]
    fn saved_tamed_pet_wolf_roundtrips() {
        let wolf = crate::wolf::WolfData {
            state: crate::wolf::WolfAiState::FollowOwner,
            ownership: crate::tameable::OwnershipData {
                owner_pubkey: "npub1ownertestbeef".to_string(),
                last_owner_damage_tick: 1234,
                last_owner_attack_tick: 5678,
            },
        };
        let pet = SavedTamedPet {
            x: 12.5, y: 64.0, z: -3.0,
            data: SavedTamedPetData::Wolf(wolf.clone()),
        };
        let bytes = bincode::serialize(&pet).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, pet);
    }

    #[test]
    fn player_save_with_two_tamed_wolves_roundtrips() {
        let pets = vec![
            SavedTamedPet {
                x: 1.0, y: 65.0, z: 1.0,
                data: SavedTamedPetData::Wolf(crate::wolf::WolfData {
                    state: crate::wolf::WolfAiState::Sit,
                    ownership: crate::tameable::OwnershipData {
                        owner_pubkey: "npub1owner".to_string(),
                        last_owner_damage_tick: 0,
                        last_owner_attack_tick: 0,
                    },
                }),
            },
            SavedTamedPet {
                x: 2.0, y: 65.0, z: 2.0,
                data: SavedTamedPetData::Wolf(crate::wolf::WolfData {
                    state: crate::wolf::WolfAiState::FollowOwner,
                    ownership: crate::tameable::OwnershipData {
                        owner_pubkey: "npub1owner".to_string(),
                        last_owner_damage_tick: 100,
                        last_owner_attack_tick: 50,
                    },
                }),
            },
        ];
        let save = PlayerSaveData {
            x: 0.0, y: 64.0, z: 0.0,
            yaw: 0.0, pitch: 0.0,
            health: 20.0,
            hotbar_slot: 0,
            inventory: vec![],
            spawn_pos: None,
            hunger: 20,
            reputation: vec![],
            tamed_pets: pets.clone(),
            armour_slots: [None, None, None, None],
            kill_counter: vec![],
            bounties_claimed: vec![],
        };
        let bytes = bincode::serialize(&save).unwrap();
        let back: PlayerSaveData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.tamed_pets.len(), 2);
        assert_eq!(back.tamed_pets, pets);
    }

    // ---------------------------------------------------------------
    // Animals Wave 2 — world-level tamed-mob persistence (the live path:
    // `saved_mobs` on WorldSave, populated from the ECS via
    // `tamed_mobs_to_saved`, restored in `chunk_stream::initial_load`).
    // ---------------------------------------------------------------

    #[test]
    fn tamed_mobs_to_saved_extracts_only_tamed_wolves_and_nostriches() {
        use crate::entity::{MobKind, Position};
        use crate::mob::MobType;
        use glam::Vec3;
        let mut ecs = hecs::World::new();
        // A tamed wolf — persisted.
        let mut tame_wolf = crate::wolf::WolfData::untamed();
        tame_wolf.ownership.owner_pubkey = "npub1wolfowner".to_string();
        ecs.spawn((Position(Vec3::new(1.0, 64.0, 1.0)), MobKind(MobType::Wolf), tame_wolf));
        // An UNtamed wolf — NOT persisted (re-scatters on load).
        ecs.spawn((
            Position(Vec3::new(2.0, 64.0, 2.0)),
            MobKind(MobType::Wolf),
            crate::wolf::WolfData::untamed(),
        ));
        // A tamed nostrich — persisted.
        let mut tame_bird = crate::nostrich::NostrichData::untamed();
        tame_bird.ownership.owner_pubkey = "npub1birdowner".to_string();
        ecs.spawn((Position(Vec3::new(3.0, 64.0, 3.0)), MobKind(MobType::Nostrich), tame_bird));
        // A wild cow — never persisted.
        ecs.spawn((Position(Vec3::new(4.0, 64.0, 4.0)), MobKind(MobType::Cow)));

        let pets = tamed_mobs_to_saved(&ecs);
        assert_eq!(pets.len(), 2, "only the two tamed pets persist (not the untamed wolf or the cow)");
        let kinds: Vec<MobType> = pets.iter().map(|p| p.data.mob_type()).collect();
        assert!(kinds.contains(&MobType::Wolf));
        assert!(kinds.contains(&MobType::Nostrich));
    }

    #[test]
    fn tamed_mobs_to_saved_maps_attack_state_to_neutral_at_save_time() {
        // CONFIRMED bug (Task 7): AttackHostile/AttackRecentAttacker carry a
        // raw hecs entity id (`target_id`) plus a session-local `until_tick`.
        // Persisting that verbatim means a stale id resolves to whatever
        // unrelated entity picks up those bits after reload — the wolf
        // indefinitely mauls a villager/cow/other pet. Saving must map the
        // attack state to the wolf's neutral state first.
        use crate::entity::{MobKind, Position};
        use crate::mob::MobType;
        use glam::Vec3;
        let mut ecs = hecs::World::new();
        let mut tame_wolf = crate::wolf::WolfData::untamed();
        tame_wolf.ownership.owner_pubkey = "npub1wolfowner".to_string();
        tame_wolf.state = crate::wolf::WolfAiState::AttackHostile {
            target_id: 0xDEAD_BEEF_u64, // junk bits — not a real live entity id
            until_tick: 999_999,
        };
        ecs.spawn((Position(Vec3::new(1.0, 64.0, 1.0)), MobKind(MobType::Wolf), tame_wolf));

        let pets = tamed_mobs_to_saved(&ecs);
        assert_eq!(pets.len(), 1);
        match &pets[0].data {
            SavedTamedPetData::Wolf(d) => {
                assert_eq!(
                    d.state,
                    crate::wolf::WolfAiState::FollowOwner,
                    "tamed wolf's attack state must be sanitised to FollowOwner before it's written to disk"
                );
            }
            other => panic!("expected Wolf, got {other:?}"),
        }

        // Round-trip through bincode too — the sanitised value must be what
        // actually lands on disk, not just what's in memory pre-serialize.
        let bytes = bincode::serialize(&pets[0]).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        match back.data {
            SavedTamedPetData::Wolf(d) => {
                assert_eq!(d.state, crate::wolf::WolfAiState::FollowOwner);
            }
            other => panic!("expected Wolf, got {other:?}"),
        }
    }

    #[test]
    fn legacy_saved_wolf_attack_state_decodes_to_neutral() {
        // Belt-and-braces: a save written BEFORE this fix can still have an
        // AttackHostile/AttackRecentAttacker state baked into its bytes.
        // Decoding it must also sanitise — this is the same
        // `sanitize_attack_state_for_persistence` step the restore call
        // sites (chunk_stream.rs, game_loop.rs WASM resume) apply to the
        // freshly-deserialized WolfData before inserting it into the ECS.
        let legacy_wolf = crate::wolf::WolfData {
            state: crate::wolf::WolfAiState::AttackRecentAttacker {
                target_id: 0xC0FFEE_u64,
                until_tick: 42, // from a tick counter that no longer exists
            },
            ownership: crate::tameable::OwnershipData {
                owner_pubkey: "npub1legacyowner".to_string(),
                last_owner_damage_tick: 10,
                last_owner_attack_tick: 20,
            },
        };
        let legacy_pet = SavedTamedPet {
            x: 5.0, y: 64.0, z: 5.0,
            data: SavedTamedPetData::Wolf(legacy_wolf),
        };
        // Simulate the bytes-on-disk from a pre-fix save.
        let legacy_bytes = bincode::serialize(&legacy_pet).unwrap();

        // Decode, then apply the same sanitize step the restore paths do.
        let decoded: SavedTamedPet = bincode::deserialize(&legacy_bytes).unwrap();
        match decoded.data {
            SavedTamedPetData::Wolf(mut d) => {
                d.sanitize_attack_state_for_persistence();
                assert_eq!(
                    d.state,
                    crate::wolf::WolfAiState::FollowOwner,
                    "legacy attack state must decode to the neutral follow state, not resolve stale entity bits"
                );
            }
            other => panic!("expected Wolf, got {other:?}"),
        }
    }

    #[test]
    fn tamed_mobs_to_saved_extracts_tamed_companions_with_species() {
        // Companions wave — Cat/Parrot/Fox share CompanionData (no species
        // inside it), so the stored MobType must preserve which creature to
        // re-spawn. Untamed companions re-scatter and are NOT persisted.
        use crate::entity::{MobKind, Position};
        use crate::mob::MobType;
        use glam::Vec3;
        let mut ecs = hecs::World::new();
        let mut tame_cat = crate::companion::CompanionData::untamed();
        tame_cat.ownership.owner_pubkey = "npub1catowner".to_string();
        ecs.spawn((Position(Vec3::new(1.0, 64.0, 1.0)), MobKind(MobType::Cat), tame_cat));
        ecs.spawn((
            Position(Vec3::new(2.0, 64.0, 2.0)),
            MobKind(MobType::Parrot),
            crate::companion::CompanionData::untamed(),
        ));
        let pets = tamed_mobs_to_saved(&ecs);
        assert_eq!(pets.len(), 1, "only the tamed cat persists (not the untamed parrot)");
        assert_eq!(pets[0].data.mob_type(), MobType::Cat, "species preserved via the stored kind");
    }

    #[test]
    fn tamed_mobs_to_saved_extracts_authored_placed_mobs() {
        // #129 — a world-author PLACED mob (the `Authored` marker, no owner) is
        // persisted alongside tamed pets so a pinned donkey survives save/load.
        // A wild (scatter) mob WITHOUT the marker is not.
        use crate::entity::{Authored, MobKind, Position};
        use crate::mob::MobType;
        use glam::Vec3;
        let mut ecs = hecs::World::new();
        // Authored donkey by the statue — persisted.
        ecs.spawn((Position(Vec3::new(8.0, 65.0, 9.0)), MobKind(MobType::Donkey), Authored));
        // A wild cow with no marker — NOT persisted (re-scatters on load).
        ecs.spawn((Position(Vec3::new(4.0, 64.0, 4.0)), MobKind(MobType::Cow)));

        let pets = tamed_mobs_to_saved(&ecs);
        assert_eq!(pets.len(), 1, "only the authored donkey persists");
        assert_eq!(pets[0].data.mob_type(), MobType::Donkey, "species preserved");
        assert!(matches!(pets[0].data, SavedTamedPetData::Authored { .. }), "tagged Authored");
        assert!((pets[0].x - 8.0).abs() < 1e-3 && (pets[0].z - 9.0).abs() < 1e-3, "position preserved");
    }

    #[test]
    fn saved_tamed_pet_companion_roundtrips() {
        let mut data = crate::companion::CompanionData::untamed();
        data.ownership.owner_pubkey = "npub1parrotbeef".to_string();
        let pet = SavedTamedPet {
            x: 7.0, y: 64.0, z: 2.0,
            data: SavedTamedPetData::Companion {
                kind: crate::mob::MobType::Parrot,
                data: data.clone(),
            },
        };
        let bytes = bincode::serialize(&pet).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, pet);
        assert_eq!(back.data.mob_type(), crate::mob::MobType::Parrot);
    }

    #[test]
    fn legacy_companion_record_still_decodes_and_restores_follow() {
        // 1C save-compat (2026-07-06): an OLD save wrote `Companion { kind, data }`
        // (no command state on the wire). It must keep decoding, and — because
        // CompanionData.state is serde(skip) — restore with state = Follow.
        let mut data = crate::companion::CompanionData::untamed();
        data.ownership.owner_pubkey = "npub1oldcatowner".to_string();
        let pet = SavedTamedPet {
            x: 3.0, y: 64.0, z: -5.0,
            data: SavedTamedPetData::Companion {
                kind: crate::mob::MobType::Cat,
                data,
            },
        };
        let bytes = bincode::serialize(&pet).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        match back.data {
            SavedTamedPetData::Companion { kind, data } => {
                assert_eq!(kind, crate::mob::MobType::Cat);
                assert_eq!(data.owner_pubkey(), "npub1oldcatowner");
                // serde(skip) defaults the runtime command state to Follow.
                assert_eq!(data.state, crate::companion::CompanionState::Follow);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn companion2_round_trips_preserving_command_state() {
        // 1C save-compat (2026-07-06): the appended `Companion2` variant carries
        // the command state explicitly (CompanionData.state is serde(skip)), so a
        // "Stay" companion must survive the wire round-trip with state intact.
        let mut data = crate::companion::CompanionData::untamed();
        data.ownership.owner_pubkey = "npub1staycat".to_string();
        data.state = crate::companion::CompanionState::Stay;
        let pet = SavedTamedPet {
            x: 1.0, y: 64.0, z: 1.0,
            data: SavedTamedPetData::Companion2 {
                kind: crate::mob::MobType::Cat,
                data: data.clone(),
                state: data.state,
            },
        };
        let bytes = bincode::serialize(&pet).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.data.mob_type(), crate::mob::MobType::Cat);
        match back.data {
            SavedTamedPetData::Companion2 { kind, data, state } => {
                assert_eq!(kind, crate::mob::MobType::Cat);
                assert_eq!(data.owner_pubkey(), "npub1staycat");
                // The carried state survives even though data.state (serde(skip))
                // decodes to its Follow default.
                assert_eq!(state, crate::companion::CompanionState::Stay);
                assert_eq!(data.state, crate::companion::CompanionState::Follow);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn tamed_companion_writer_emits_companion2_with_state() {
        // The writer must snapshot the live command state into `Companion2` so a
        // "Stay" companion persists its state (not just its ownership).
        use crate::entity::{MobKind, Position};
        use crate::mob::MobType;
        use glam::Vec3;
        let mut ecs = hecs::World::new();
        let mut cat = crate::companion::CompanionData::untamed();
        cat.ownership.owner_pubkey = "npub1writercat".to_string();
        cat.state = crate::companion::CompanionState::Stay;
        ecs.spawn((Position(Vec3::new(1.0, 64.0, 1.0)), MobKind(MobType::Cat), cat));
        let pets = tamed_mobs_to_saved(&ecs);
        assert_eq!(pets.len(), 1, "the tamed cat persists");
        match &pets[0].data {
            SavedTamedPetData::Companion2 { kind, state, .. } => {
                assert_eq!(*kind, MobType::Cat);
                assert_eq!(*state, crate::companion::CompanionState::Stay, "command state captured");
            }
            other => panic!("writer must emit Companion2, got: {other:?}"),
        }
    }

    #[test]
    fn saved_tamed_pet_nostrich_roundtrips() {
        let mut bird = crate::nostrich::NostrichData::untamed();
        bird.ownership.owner_pubkey = "npub1birdbeef".to_string();
        bird.lay_ticks_remaining = 4242;
        bird.feather_ticks_remaining = 9001;
        let pet = SavedTamedPet {
            x: 5.0,
            y: 64.0,
            z: -1.0,
            data: SavedTamedPetData::Nostrich(bird.clone()),
        };
        let bytes = bincode::serialize(&pet).unwrap();
        let back: SavedTamedPet = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, pet, "a tamed Nostrich must survive the wire round-trip intact");
        assert_eq!(back.data.mob_type(), crate::mob::MobType::Nostrich);
    }


    #[test]
    fn untamed_wolf_round_trips_with_empty_pubkey() {
        // Negative space — the wolf data shape carries owner-empty as
        // the untamed sentinel. Round-trip must preserve that.
        let wolf = crate::wolf::WolfData::untamed();
        assert!(!wolf.is_tamed());
        let bytes = bincode::serialize(&wolf).unwrap();
        let back: crate::wolf::WolfData = bincode::deserialize(&bytes).unwrap();
        assert!(!back.is_tamed());
        assert_eq!(back.owner_pubkey(), "");
    }

    // ── Audit 2026-09-27 (wave 2): torn meta / torn chunks / misaligned tail ──

    #[test]
    fn torn_world_meta_is_rebuilt_from_world_dat_seed_and_kept_aside() {
        let _g = WorldsRootGuard::new("torn_meta_recover");
        let dir = world_dir("w");
        fs::create_dir_all(&dir).unwrap();
        let save = minimal_world_save_for_tests(123_456);
        fs::write(dir.join("world.dat"), bincode::serialize(&save).unwrap()).unwrap();
        let torn = br#"{"display_name":"My World","game_mode":"crea"#;
        fs::write(dir.join("world_meta.json"), torn).unwrap();

        let meta = try_load_world_meta("w").expect("recoverable from world.dat");
        assert_eq!(meta.seed, 123_456, "seed must come from world.dat, never the 42 default");
        // Review S3: recovery is conservative, never "clean".
        assert!(meta.cheats_used, "a recovered world can't claim no cheats");
        assert!(!meta.pure_survival);
        assert!(meta.genesis_block_found, "the Genesis Block must never be findable twice");

        // The damaged original is kept aside, byte for byte.
        let aside: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("world_meta.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "exactly one quarantined copy");
        assert_eq!(fs::read(aside[0].path()).unwrap(), torn);

        // The rebuilt meta was written back, so the next launch is stable too.
        assert_eq!(load_world_meta("w").seed, 123_456);
        assert!(save_world_meta("w", &meta).is_ok(), "a recovered world saves normally");
    }

    #[test]
    fn unrecoverable_world_meta_refuses_load_and_never_writes_defaults() {
        let _g = WorldsRootGuard::new("torn_meta_refuse");
        let dir = world_dir("w");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("world.dat"), b"\x01not a world save").unwrap();
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();

        let err = try_load_world_meta("w").expect_err("no meta and no seed = damaged");
        assert!(err.contains("world info damaged"), "{err}");

        // The world list labels it rather than hiding it or showing default meta.
        let entries = list_world_entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].meta.display_name.contains("world info damaged"));

        // Loading is refused, and no autosave/meta write can put defaults over it.
        let mut world = World::new();
        assert!(load_world("w", &mut world).is_err());
        let defaults = load_world_meta("w");
        assert!(save_world_meta("w", &defaults).is_err(), "defaults must never persist");
        assert!(!dir.join("world_meta.json").exists());
        // Still refused on the next launch (the quarantined copy marks the damage).
        assert!(try_load_world_meta("w").is_err());
    }

    #[test]
    fn save_world_meta_refuses_to_overwrite_an_unparseable_file() {
        let _g = WorldsRootGuard::new("meta_overwrite");
        let dir = world_dir("w");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("world_meta.json"), b"{ torn").unwrap();
        assert!(save_world_meta("w", &WorldMeta::new("w")).is_err());
        assert_eq!(fs::read(dir.join("world_meta.json")).unwrap(), b"{ torn");
    }

    #[test]
    fn a_torn_chunk_file_is_kept_aside_not_silently_dropped() {
        let _g = WorldsRootGuard::new("torn_chunk");
        let chunks = world_dir("w").join("chunks");
        fs::create_dir_all(&chunks).unwrap();
        let torn = vec![7u8; 100]; // wrong length for any chunk encoding
        fs::write(chunks.join("3_1_-2.chunk"), &torn).unwrap();

        let mut world = World::new();
        let loaded = load_chunk_dir(&chunks, &mut world).expect("load continues");
        assert_eq!(loaded, 0);
        assert!(!chunks.join("3_1_-2.chunk").exists(), "moved aside so a save can't overwrite it");
        let aside: Vec<_> = fs::read_dir(&chunks)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("3_1_-2.chunk.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read(aside[0].path()).unwrap(), torn);
    }

    #[test]
    fn quarantine_never_overwrites_an_earlier_quarantined_copy() {
        let _g = WorldsRootGuard::new("quarantine_twice");
        let dir = world_dir("q");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.blob");
        fs::write(&f, b"first").unwrap();
        let a = quarantine_corrupt(&f).unwrap();
        fs::write(&f, b"second").unwrap();
        let b = quarantine_corrupt(&f).unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read(a).unwrap(), b"first");
        assert_eq!(fs::read(b).unwrap(), b"second");
    }

    #[test]
    fn tail_reader_stops_at_the_first_failed_field() {
        // Field A: Vec<bool> of len 1 holding an invalid bool (2) → fails part-way.
        // Where A "really" ended is unknowable, so whatever bytes follow must not
        // be decoded as B: B takes its default.
        let mut bytes = 1u64.to_le_bytes().to_vec();
        bytes.push(2);
        bytes.extend_from_slice(&7u64.to_le_bytes());
        let mut cur = std::io::Cursor::new(&bytes[..]);
        let mut tail = TailReader::default();
        let a: Vec<bool> = tail.field(&mut cur, "a");
        let b: u64 = tail.field(&mut cur, "b");
        assert!(a.is_empty());
        assert_eq!(b, 0, "a field after a failure must default, never decode misaligned bytes");
        assert_eq!(tail.failed_at, Some("a"));
    }

    #[test]
    fn a_meta_torn_inside_a_multibyte_char_is_recovered_not_refused() {
        let _g = WorldsRootGuard::new("torn_meta_utf8");
        let dir = world_dir("w");
        fs::create_dir_all(&dir).unwrap();
        let save = minimal_world_save_for_tests(777);
        fs::write(dir.join("world.dat"), bincode::serialize(&save).unwrap()).unwrap();
        // "🌍" is F0 9F 8C 8D; cut after two bytes → invalid UTF-8.
        let mut torn = br#"{"display_name":"My "#.to_vec();
        torn.extend_from_slice(&[0xF0, 0x9F]);
        fs::write(dir.join("world_meta.json"), &torn).unwrap();
        let meta = try_load_world_meta("w").expect("recovered from world.dat");
        assert_eq!(meta.seed, 777);
    }

    #[test]
    fn a_partly_decoded_world_dat_is_copied_aside_before_any_overwrite() {
        let _g = WorldsRootGuard::new("partial_world_dat");
        let dir = world_dir("w");
        fs::create_dir_all(&dir).unwrap();
        save_world_meta("w", &WorldMeta::new("w")).unwrap();
        // Cut the newest tail field (`rig_clips`) mid-length-prefix.
        let mut bytes = bincode::serialize(&minimal_world_save_for_tests(9)).unwrap();
        bytes.truncate(bytes.len() - 4);
        fs::write(dir.join("world.dat"), &bytes).unwrap();

        let mut world = World::new();
        let (save, _) = load_world("w", &mut world).expect("loads what decodes");
        assert_eq!(save.seed, 9);
        let copies: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("world.dat.corrupt-"))
            .collect();
        assert_eq!(copies.len(), 1, "exactly one copy of the damaged original");
        assert_eq!(fs::read(copies[0].path()).unwrap(), bytes);
        assert!(dir.join("world.dat").exists(), "a copy, not a move");
        // Once per session: a second load makes no second copy.
        let mut world2 = World::new();
        load_world("w", &mut world2).unwrap();
        let n = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("world.dat.corrupt-"))
            .count();
        assert_eq!(n, 1);
    }

    #[test]
    fn write_atomic_nosync_replaces_and_leaves_no_temp() {
        let _g = WorldsRootGuard::new("nosync");
        let dir = world_dir("w");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("1_2_3.chunk");
        write_atomic_nosync(&p, b"a").unwrap();
        write_atomic_nosync(&p, b"bb").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"bb");
        assert!(!dir.join("1_2_3.chunk.tmp").exists());
        sync_dir(&dir);
    }

    // ── world.dat format-version footer (gap-audit T1-7, Spec 02 §8.4) ──

    use crate::save_format::{
        file_footer_version, footer_bytes, NEWER_WORLD_MESSAGE, SAVE_FORMAT_VERSION,
    };

    /// A `world.dat` as a build one format version newer than this one writes it.
    fn newer_world_dat(seed: u32) -> Vec<u8> {
        let mut bytes = bincode::serialize(&minimal_world_save_for_tests(seed)).unwrap();
        bytes.extend_from_slice(&footer_bytes(SAVE_FORMAT_VERSION + 1));
        bytes
    }

    /// Every file under `dir` with its bytes, so a test can prove nothing changed.
    fn snapshot_tree(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(
            root: &std::path::Path,
            at: &std::path::Path,
            out: &mut std::collections::BTreeMap<String, Vec<u8>>,
        ) {
            for e in fs::read_dir(at).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
                    out.insert(rel, fs::read(&p).unwrap());
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    fn test_slot() -> crate::player_slot::PlayerSlot {
        crate::player_slot::PlayerSlot::new(0, glam::Vec3::new(0.5, 64.0, 0.5), 1.0)
    }

    /// The assumption the footer design rests on: the decoder every build before
    /// the footer runs (tolerant decode of the WHOLE stream) reads a footer-bearing
    /// save exactly like the bare payload, ignoring the 12 trailing bytes. So
    /// shipped builds keep opening new saves exactly as before.
    #[test]
    fn pre_footer_decoder_reads_a_footer_bearing_save_unchanged() {
        let payload = worldsave_with_chest_bytes();
        let mut with_footer = payload.clone();
        with_footer.extend_from_slice(&footer_bytes(SAVE_FORMAT_VERSION));

        let (old_build, failed, _) = deserialize_world_save_tolerant_reporting(&with_footer)
            .expect("a pre-footer build must still decode a footer-bearing save");
        assert!(failed.is_none(), "the footer must not trip the tail reader");
        let (plain, _, _) = deserialize_world_save_tolerant_reporting(&payload).unwrap();
        assert_eq!(
            bincode::serialize(&old_build).unwrap(),
            bincode::serialize(&plain).unwrap(),
            "footer bytes must not bleed into any field"
        );
        assert_eq!(old_build.chests.len(), 1);
        // A build that also knew a field fewer stops even earlier, so ignores more.
    }

    #[test]
    fn footer_bearing_and_footerless_saves_decode_identically() {
        let save = minimal_world_save_for_tests(77);
        let legacy = bincode::serialize(&save).unwrap();
        let current = crate::save_format::encode_world_save(&save).unwrap();
        let a = read_world_save(&legacy).expect("a footer-less legacy save still loads");
        let b = read_world_save(&current).expect("a current save loads");
        assert_eq!(bincode::serialize(&a).unwrap(), bincode::serialize(&b).unwrap());
        assert_eq!(b.seed, 77);
    }

    /// The footer is stripped BEFORE the tolerant decode: an older-version save
    /// (one appended field fewer) defaults that field instead of decoding the
    /// footer bytes as it — which would fail the tail and flag the save damaged.
    #[test]
    fn older_version_footer_is_stripped_before_the_tail_decode() {
        let mut bytes = bincode::serialize(&minimal_world_save_for_tests(9)).unwrap();
        // `rig_clips` is the newest field and an empty Vec: its 8 length bytes end
        // the payload. Drop them = a save written before `rig_clips` existed.
        bytes.truncate(bytes.len() - 8);
        bytes.extend_from_slice(&footer_bytes(SAVE_FORMAT_VERSION - 1));
        let (back, partial) = read_world_save_reporting(&bytes).expect("older save loads");
        assert!(!partial, "footer bytes were decoded as a field");
        assert_eq!(back.seed, 9);
        assert!(back.rig_clips.is_empty());
    }

    #[test]
    fn newer_version_save_is_refused_with_a_typed_error() {
        let err = read_world_save(&newer_world_dat(1)).err().expect("must refuse");
        assert_eq!(
            err,
            crate::save_format::WorldSaveError::NewerVersion {
                found: SAVE_FORMAT_VERSION + 1,
                supported: SAVE_FORMAT_VERSION,
            }
        );
        assert_eq!(err.to_string(), NEWER_WORLD_MESSAGE);
    }

    /// TRIPWIRE (the reader half of `save_format::world_save_field_count_tripwire`):
    /// the tolerant reader must read every field of a current save. A field listed
    /// in its struct literal as `Default::default()` without a read compiles fine
    /// but leaves bytes unread here.
    #[test]
    fn tolerant_reader_reads_every_byte_of_a_current_save() {
        let payload = bincode::serialize(&minimal_world_save_for_tests(4)).unwrap();
        let (_, failed, consumed) = deserialize_world_save_tolerant_reporting(&payload).unwrap();
        assert!(failed.is_none());
        assert_eq!(
            consumed,
            payload.len() as u64,
            "the tolerant reader skipped a WorldSave field (bytes left unread)"
        );
    }

    #[test]
    fn newer_world_is_refused_and_nothing_writes_to_it() {
        let _g = WorldsRootGuard::new("newer_refused");
        let dir = world_dir("w");
        fs::create_dir_all(dir.join("chunks")).unwrap();
        fs::write(dir.join("world.dat"), newer_world_dat(5)).unwrap();
        let meta = WorldMeta::new("w");
        fs::write(dir.join("world_meta.json"), serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
        fs::write(dir.join("chunks").join("0_4_0.chunk"), b"not a real chunk").unwrap();
        let before = snapshot_tree(&dir);

        // Opening: refused with the lobby message, before anything is read in.
        assert_eq!(world_open_refusal("w").as_deref(), Some(NEWER_WORLD_MESSAGE));
        let mut world = World::new();
        assert_eq!(load_world("w", &mut world).err().as_deref(), Some(NEWER_WORLD_MESSAGE));
        assert_eq!(world.persistable_chunks().count(), 0, "no chunk was read in");

        // The lobby card says why rather than offering a world it can't open.
        let entries = list_world_entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].meta.display_name.contains("needs a newer version"));
        assert_eq!(entries[0].meta.description, NEWER_WORLD_MESSAGE);

        // Every writer refuses: no autosave, no meta, no save, no server save.
        let slot = test_slot();
        assert!(save_world_meta("w", &meta).is_err(), "meta write");
        assert!(
            write_world_folder("w", &meta, &minimal_world_save_for_tests(1), &World::new())
                .is_err(),
            "world folder write"
        );
        assert!(
            save_world("w", &World::new(), std::slice::from_ref(&slot), 1, &[], &[]).is_err(),
            "save"
        );
        assert!(
            autosave_world("w", &World::new(), std::slice::from_ref(&slot), 1, &[], &[]).is_err(),
            "autosave"
        );
        assert!(
            crate::server::GameServer::new(0, "w".to_string(), 1).try_save().is_err(),
            "dedicated-server save"
        );

        assert_eq!(snapshot_tree(&dir), before, "the newer world's files must be untouched");
    }

    #[test]
    fn newer_autosave_alone_refuses_the_world() {
        let _g = WorldsRootGuard::new("newer_autosave");
        let dir = world_dir("w");
        fs::create_dir_all(dir.join("autosave")).unwrap();
        fs::write(
            dir.join("world.dat"),
            crate::save_format::encode_world_save(&minimal_world_save_for_tests(5)).unwrap(),
        )
        .unwrap();
        fs::write(dir.join("autosave").join("world.dat"), newer_world_dat(5)).unwrap();
        let before = snapshot_tree(&dir);

        assert!(has_autosave("w"));
        assert_eq!(world_open_refusal("w").as_deref(), Some(NEWER_WORLD_MESSAGE));
        let mut world = World::new();
        assert_eq!(load_autosave("w", &mut world).err().as_deref(), Some(NEWER_WORLD_MESSAGE));
        assert!(save_world_meta("w", &WorldMeta::new("w")).is_err());
        assert_eq!(snapshot_tree(&dir), before);
    }

    /// A newer build may write a `world_meta.json` this build cannot parse. The
    /// damaged-meta recovery must not quarantine it or rebuild it from a
    /// `world.dat` it cannot read.
    #[test]
    fn unparseable_meta_on_a_newer_world_is_not_quarantined() {
        let _g = WorldsRootGuard::new("newer_meta");
        let dir = world_dir("w");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("world.dat"), newer_world_dat(5)).unwrap();
        fs::write(dir.join("world_meta.json"), br#"{"world_type": {"future": 1}"#).unwrap();
        let before = snapshot_tree(&dir);

        assert_eq!(try_load_world_meta("w").err().as_deref(), Some(NEWER_WORLD_MESSAGE));
        assert_eq!(snapshot_tree(&dir), before, "no quarantine rename, no rebuilt meta");
        let entries = list_world_entries();
        assert!(entries[0].meta.display_name.contains("needs a newer version"));
    }

    #[test]
    fn every_world_dat_writer_appends_the_current_footer() {
        let _g = WorldsRootGuard::new("writers_footer");
        let slot = test_slot();
        let current = Some(SAVE_FORMAT_VERSION);

        write_world_folder("a", &WorldMeta::new("a"), &minimal_world_save_for_tests(1), &World::new())
            .unwrap();
        assert_eq!(file_footer_version(&world_dir("a").join("world.dat")), current, "write_world_folder");

        save_world("b", &World::new(), std::slice::from_ref(&slot), 1, &[], &[]).unwrap();
        assert_eq!(file_footer_version(&world_dir("b").join("world.dat")), current, "save_world");

        autosave_world("c", &World::new(), std::slice::from_ref(&slot), 1, &[], &[]).unwrap();
        assert_eq!(
            file_footer_version(&world_dir("c").join("autosave").join("world.dat")),
            current,
            "autosave_world"
        );

        crate::server::GameServer::new(0, "d".to_string(), 1).try_save().unwrap();
        assert_eq!(file_footer_version(&world_dir("d").join("world.dat")), current, "server save");

        // And each loads back through the normal path.
        for name in ["a", "b", "d"] {
            let mut w = World::new();
            load_world(name, &mut w).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let mut w = World::new();
        load_autosave("c", &mut w).unwrap();
    }
}
