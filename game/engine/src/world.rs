//! World — collection of sub-chunks with terrain generation and chunk streaming.
//!
//! Spec 02: The world is made of 16x16x16 sub-chunks identified by (cx, cy, cz).
//! Terrain generation uses the BiomeGenerator for biome-aware terrain, caves, and water.
//! Chunk streaming generates columns on demand as the player moves.

use ahash::AHashMap;
use crate::biome::{Biome, BiomeGenerator, LAVA_CAVE_LEVEL, SEA_LEVEL};
use crate::block::{self, BlockId, AIR};
use crate::chunk::{Chunk, CHUNK_SIZE};

/// Maximum chunk Y coordinate for terrain generation.
pub const MAX_CHUNK_Y: i32 = 5;

/// Version of the terrain generator's output (gap-audit T2-9, Spec 02 §5).
///
/// **Bump this whenever generation output for a given seed + world flags
/// changes** — terrain shape, biomes, caves, ore, trees, vegetation, villages,
/// hideouts, ravines, mineshafts, or the flat/void presets. Everything reached
/// from [`World::generate_column`] counts. The golden test in
/// `test_integration/worldgen_golden.rs` pins the output and fails until the
/// bump (and its new hash) land together.
///
/// A joiner generates the host's terrain locally from the seed in
/// `JoinAcceptPacket`, so the host sends [`worldgen_fingerprint`] (this
/// version folded with the bundled plan registry's content hash) there and the
/// joiner sends its own in `JoinRequestPacket`. On a mismatch the joiner warns
/// the player (terrain may differ) and the host records it on the player
/// (`ServerPlayer::worldgen_mismatch`) for a later pass to push real chunks.
///
/// **Generation is a pure function of (seed, world flags, fingerprint)**
/// (Phase B0): independent of the order columns are generated in and of
/// platform float maths. What keeps it so — break one and joiners desync:
/// - deepslate variants bake the fixed `biome::WORLDGEN_RESERVE_RICHNESS`,
///   never live Reserve state;
/// - every decoration pass writes only inside the column being generated and
///   reads neighbour columns only through pure functions of `biome_gen`
///   (`tree_at_column_cell`, `terrain_block_at`, `village_site`), never
///   `get_block` on a column that may not exist yet;
/// - the Brigand Hideout village gate asks `village_gen::village_site_within`,
///   not `village_anchors` (which fills in generation order);
/// - villages sample `PlanRegistry::bundled()`, not `World::plan_registry`
///   (runtime `/importschem` additions);
/// - layout trig is `libm` (`sinf`/`cosf`/`sincosf`), no `powi`/`powf`;
///   only IEEE-exact `+ - * / sqrt floor round` otherwise;
/// - no hash-map iteration order reaches a block write.
///
/// The golden test pins the output and generates a village + hideout area in
/// two column orders; Spec 02 §5.2 has the version log.
pub const WORLDGEN_VERSION: u32 = 2;

/// The value host and joiner exchange as `worldgen_version` in `JoinAccept` /
/// `JoinRequest`: [`WORLDGEN_VERSION`] folded with the content hash of the
/// bundled plan registry villages are built from
/// ([`crate::plan_registry::PlanRegistry::bundled`]). Two builds agree iff
/// both their generator version and their bundled plans match, so a plan edit
/// with no code change still reads as "terrain may differ". Never 0 (a peer
/// that sent no value decodes as 0, which must read as a mismatch).
pub fn worldgen_fingerprint() -> u32 {
    static FINGERPRINT: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| {
        worldgen_fingerprint_of(
            WORLDGEN_VERSION,
            &crate::plan_registry::PlanRegistry::bundled().content_hash(),
        )
    });
    *FINGERPRINT
}

/// Pure core of [`worldgen_fingerprint`]: the first four bytes (LE) of
/// SHA-256(domain, version, plan-registry hash), with 0 mapped to 1.
pub(crate) fn worldgen_fingerprint_of(version: u32, plans_hash: &[u8; 32]) -> u32 {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"axenstax-worldgen-fingerprint\0");
    h.update(version.to_le_bytes());
    h.update(plans_hash);
    let d = h.finalize();
    match u32::from_le_bytes([d[0], d[1], d[2], d[3]]) {
        0 => 1,
        fp => fp,
    }
}

// `BlockPos`/`ChunkPos` (plain (x,y,z) wrapper structs) were removed here —
// zero references anywhere; every call site in this codebase addresses
// positions as raw `(i32, i32, i32)` tuples instead, so these predate that
// convention and were never adopted.

/// Spec 20 Phase 2 — tagged-enum block-entity payload. Lives in
/// `World.block_entities` at per-block-position keys. Each variant is a
/// concrete state struct owned by the system that defines it. New
/// block-entity types (Mill, Oven, Aging Rack, Chest, …) land as
/// additional variants here.
///
/// Tagged-enum chosen over `Box<dyn BlockEntity>` per the spec's
/// Design choice 1: alpha block-entity count is tractable, bincode
/// serialises the enum for free, and the variant tag survives save
/// migrations cleanly.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum BlockEntityData {
    Campfire(crate::campfire::CampfireData),
    Furnace(crate::furnace::FurnaceData),
    Vendor(crate::vendor::VendorData),
    /// Spec 28d chunk 8 — Bee Hive. State: bees_inside + honey_level.
    Hive(crate::bee_hive::HiveData),
    /// Historical Pivot Sub-Foundation 2 (HP-2, 2026-05-22) — Chest.
    /// 27-slot storage container. See `crate::chest::ChestData`.
    Chest(crate::chest::ChestData),
    /// Spec 34 Tip Jar — owner + escrow + lifetime counter. See
    /// `crate::tip_jar::TipJarData`.
    TipJar(crate::tip_jar::TipJarData),
    /// Spec 38 Auction — timed lot + reserve + high bid. See
    /// `crate::auction::AuctionData`.
    Auction(crate::auction::AuctionData),
    /// Spec 38 (Blueprint / Cyanotype, 2026-05-27) — Latent Print.
    /// Holds a captured Plan whose `develop_state` is still progressing
    /// under direct sun. See `crate::latent_print::LatentPrintData`.
    LatentPrint(crate::latent_print::LatentPrintData),
    /// #47 — Grave. A death container holding the player's 36-slot inventory
    /// snapshot (index-aligned). See `crate::grave::GraveData`.
    Grave(crate::grave::GraveData),
    /// Spec 48 (Electricity) — a power device (lever/button/plate/gate/crank/
    /// generator/battery/lamp). Rich per-device state (switch, gate op, fuel,
    /// charge, facing). Stateless power blocks (Cable) have NO block-entity —
    /// their on/off is read from `World::power.energised`.
    PowerDevice(crate::power::PowerDeviceData),
    /// Dispenser/Dropper (2026-07-04) — 9-slot eject-on-power container.
    /// See `crate::dispenser::DispenserData`.
    Dispenser(crate::dispenser::DispenserData),
    /// Solo Buildout Wave 2c — a Sign's editable text. See `crate::sign`.
    Sign(crate::sign::SignData),
    /// Solo Buildout Wave 2c — an Item Frame's displayed item + rotation. See
    /// `crate::item_frame`.
    ItemFrame(crate::item_frame::ItemFrameData),
    /// Spec 49 (Explosives) — a Composter ageing plant/food waste into Compost,
    /// then Compost into Saltpetre. Reuses the generic `WorkstationState`
    /// (input/output/progress, fuel-free). See `crate::composter`.
    Composter(crate::workstation::WorkstationState),
}

/// The block-entity families whose state lives on the HOST's client world and
/// is mirrored into its `HostedServer` (`HostedServer::mirror_host_world_state`):
/// the containers a joiner's break spills, and the economy blocks whose owner
/// the server checks. A lit/unlit furnace or a chest tier change stays in its
/// family, so the entity survives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirroredFamily {
    Chest,
    Furnace,
    Dispenser,
    Grave,
    Vendor,
    TipJar,
    Auction,
}

impl MirroredFamily {
    /// The containers — the families whose contents spill when broken.
    pub fn is_container(self) -> bool {
        matches!(self, Self::Chest | Self::Furnace | Self::Dispenser | Self::Grave)
    }
}

/// Which mirrored family a block id carries, if any.
pub fn mirrored_family(b: BlockId) -> Option<MirroredFamily> {
    use block::*;
    match b {
        CHEST | COPPER_CHEST | IRON_CHEST | DIAMOND_CHEST | SATORI_CHEST => {
            Some(MirroredFamily::Chest)
        }
        FURNACE | FURNACE_LIT => Some(MirroredFamily::Furnace),
        DISPENSER | DROPPER => Some(MirroredFamily::Dispenser),
        GRAVE => Some(MirroredFamily::Grave),
        VENDOR_BLOCK => Some(MirroredFamily::Vendor),
        TIP_JAR => Some(MirroredFamily::TipJar),
        AUCTION_BLOCK => Some(MirroredFamily::Auction),
        _ => None,
    }
}

/// Does [`World::apply_remote_block_change`] run side effects beyond writing the
/// block id when this block id enters or leaves a cell: a power-device
/// block-entity (register / drop), a mirrored container / economy block-entity
/// (drop on break), or a plot-marker claim (release)?
///
/// Those effects fire only when the apply SEES the block change, so a delivery
/// path that folds `A → B → A′` at one cell into `A′` (`state_outbox`'s backlog
/// coalescing) must not do it when either end carries them — the joiner would
/// see no change at all and keep a stale entity. Composed from the very
/// predicates the apply uses, so there is no second list to drift; **add a new
/// side-effect branch to the apply and it belongs here too.**
pub fn block_has_remote_apply_effects(b: BlockId) -> bool {
    crate::power::device_kind_for_block(b).is_some()
        || mirrored_family(b).is_some()
        || b == block::PLOT_MARKER
}

impl BlockEntityData {
    /// The mirrored family this entity belongs to, if any.
    pub fn mirrored_family(&self) -> Option<MirroredFamily> {
        match self {
            Self::Chest(_) => Some(MirroredFamily::Chest),
            Self::Furnace(_) => Some(MirroredFamily::Furnace),
            Self::Dispenser(_) => Some(MirroredFamily::Dispenser),
            Self::Grave(_) => Some(MirroredFamily::Grave),
            Self::Vendor(_) => Some(MirroredFamily::Vendor),
            Self::TipJar(_) => Some(MirroredFamily::TipJar),
            Self::Auction(_) => Some(MirroredFamily::Auction),
            _ => None,
        }
    }
}

/// A décor/data attachment coating ONE face of a block (generalises the
/// 2026-06-03 wallpaper overlay). Render-only — no collision, no cell, no
/// height. Stored in `World.face_attachments` keyed by block position with one
/// optional entry per face (index via `crate::mesh::Face::index`). Unlike
/// `salt_licks`, this has no block to rebuild from, so it IS serialised —
/// mirroring the `block_entities` / `SavedCampfire` pattern (see
/// `save::SavedFaceOverlay`). `Wallpaper` recovers as `Item::Block(block)`;
/// `Blueprint` carries a laid, possibly-still-developing plan and recovers as
/// `Item::Plan(*plan)`. NOT `Copy` (the Blueprint payload boxes a `PlanData`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FaceAttachment {
    Wallpaper(BlockId),
    /// Laid blank cream draughting paper, pre-capture. Carries no payload — the
    /// plan only exists after capture, at which point it becomes `Blueprint`.
    BlueprintBlank,
    Blueprint(Box<crate::plan::PlanData>),
}

/// One optional attachment per face. Index by `crate::mesh::Face::index()`
/// (Top=0, Bottom=1, North=2, South=3, East=4, West=5).
pub type FaceAttachments = [Option<FaceAttachment>; 6];

impl BlockEntityData {
    /// Convenience: borrow the inner Campfire if this entry is one.
    pub fn as_campfire(&self) -> Option<&crate::campfire::CampfireData> {
        match self {
            BlockEntityData::Campfire(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_campfire_mut(&mut self) -> Option<&mut crate::campfire::CampfireData> {
        match self {
            BlockEntityData::Campfire(c) => Some(c),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Furnace if this entry is one.
    pub fn as_furnace(&self) -> Option<&crate::furnace::FurnaceData> {
        match self {
            BlockEntityData::Furnace(f) => Some(f),
            _ => None,
        }
    }
    pub fn as_furnace_mut(&mut self) -> Option<&mut crate::furnace::FurnaceData> {
        match self {
            BlockEntityData::Furnace(f) => Some(f),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Vendor if this entry is one.
    pub fn as_vendor(&self) -> Option<&crate::vendor::VendorData> {
        match self {
            BlockEntityData::Vendor(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_vendor_mut(&mut self) -> Option<&mut crate::vendor::VendorData> {
        match self {
            BlockEntityData::Vendor(v) => Some(v),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Hive if this entry is one.
    pub fn as_hive(&self) -> Option<&crate::bee_hive::HiveData> {
        match self {
            BlockEntityData::Hive(h) => Some(h),
            _ => None,
        }
    }
    pub fn as_hive_mut(&mut self) -> Option<&mut crate::bee_hive::HiveData> {
        match self {
            BlockEntityData::Hive(h) => Some(h),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Sign if this entry is one.
    pub fn as_sign(&self) -> Option<&crate::sign::SignData> {
        match self {
            BlockEntityData::Sign(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_sign_mut(&mut self) -> Option<&mut crate::sign::SignData> {
        match self {
            BlockEntityData::Sign(s) => Some(s),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Item Frame if this entry is one.
    pub fn as_item_frame(&self) -> Option<&crate::item_frame::ItemFrameData> {
        match self {
            BlockEntityData::ItemFrame(f) => Some(f),
            _ => None,
        }
    }
    pub fn as_item_frame_mut(&mut self) -> Option<&mut crate::item_frame::ItemFrameData> {
        match self {
            BlockEntityData::ItemFrame(f) => Some(f),
            _ => None,
        }
    }
    /// Spec 49 (Explosives) — borrow the inner Composter if this entry is one.
    pub fn as_composter(&self) -> Option<&crate::workstation::WorkstationState> {
        match self {
            BlockEntityData::Composter(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_composter_mut(&mut self) -> Option<&mut crate::workstation::WorkstationState> {
        match self {
            BlockEntityData::Composter(c) => Some(c),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Chest if this entry is one.
    pub fn as_chest(&self) -> Option<&crate::chest::ChestData> {
        match self {
            BlockEntityData::Chest(c) => Some(c),
            _ => None,
        }
    }
    pub fn as_chest_mut(&mut self) -> Option<&mut crate::chest::ChestData> {
        match self {
            BlockEntityData::Chest(c) => Some(c),
            _ => None,
        }
    }
    /// #47 — borrow the inner Grave if this entry is one.
    pub fn as_grave(&self) -> Option<&crate::grave::GraveData> {
        match self {
            BlockEntityData::Grave(g) => Some(g),
            _ => None,
        }
    }
    /// Kept for API consistency with the other `as_X_mut` block-entity
    /// getters (`as_tip_jar_mut`, `as_auction_mut`, both live) — Grave
    /// mutation currently goes through the clone → mutate → `insert_grave`
    /// idiom (see `game_loop.rs`) instead, so this one has no caller yet.
    #[allow(dead_code)]
    pub fn as_grave_mut(&mut self) -> Option<&mut crate::grave::GraveData> {
        match self {
            BlockEntityData::Grave(g) => Some(g),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Tip Jar if this entry is one.
    pub fn as_tip_jar(&self) -> Option<&crate::tip_jar::TipJarData> {
        match self {
            BlockEntityData::TipJar(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_tip_jar_mut(&mut self) -> Option<&mut crate::tip_jar::TipJarData> {
        match self {
            BlockEntityData::TipJar(t) => Some(t),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Auction if this entry is one.
    pub fn as_auction(&self) -> Option<&crate::auction::AuctionData> {
        match self {
            BlockEntityData::Auction(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_auction_mut(&mut self) -> Option<&mut crate::auction::AuctionData> {
        match self {
            BlockEntityData::Auction(a) => Some(a),
            _ => None,
        }
    }
    /// Convenience: borrow the inner Latent Print if this entry is one.
    pub fn as_latent_print(&self) -> Option<&crate::latent_print::LatentPrintData> {
        match self {
            BlockEntityData::LatentPrint(l) => Some(l),
            _ => None,
        }
    }
    /// Same story as `as_grave_mut` — kept for API consistency; LatentPrint
    /// mutation goes through clone → mutate → `insert_latent_print` instead.
    #[allow(dead_code)]
    pub fn as_latent_print_mut(&mut self) -> Option<&mut crate::latent_print::LatentPrintData> {
        match self {
            BlockEntityData::LatentPrint(l) => Some(l),
            _ => None,
        }
    }
}

/// One row of a `/ws browse` result: a published override set discovered on
/// Beacon. Display-only (npub already encoded at fill time); `/ws adopt <n>`
/// reads the `blob_hash`. Filled by the Task 9 browse driver.
#[derive(Clone, Debug, Default)]
pub struct BrowseEntry {
    pub label: String,
    // `author_npub`/`size` are read by the `/ws browse` chat listing
    // (game_loop.rs), which is wasm32-only — invisible to a native
    // `cargo clippy` run.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub author_npub: String, // npub for display (encoded at fill time)
    pub blob_hash: String,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub size: u64,
}

pub struct World {
    chunks: AHashMap<(i32, i32, i32), Chunk>,
    /// Spec 02 §7.5 — the evicted-chunk store. A column streamed out of range
    /// whose chunks carry `persist` (edited after generation, or loaded from a
    /// save / the network) moves here instead of being dropped, so re-entry
    /// restores it (`restore_column`) rather than regenerating over the edit,
    /// and every save path still writes it (`persistable_chunks`). Whole-column
    /// granularity. In memory only (not paged to disk) so native and WASM behave
    /// the same. Bounded by the edited + saved columns the player has visited.
    evicted: AHashMap<(i32, i32, i32), Chunk>,
    /// Columns currently in `evicted` (a column can have evicted chunks at only
    /// some cys; a write to an absent cy of an evicted column still belongs in
    /// the store). Block reads/writes that miss `chunks` go through to the
    /// store for these columns; light writes to them are dropped.
    evicted_columns: ahash::AHashSet<(i32, i32)>,
    /// Non-zero while `generate_column` runs: block writes made by world-gen (terrain,
    /// trees, villages, structures — including spill into neighbouring chunks)
    /// don't mark a chunk `persist`. Runtime-only.
    worldgen_depth: u32,
    /// Spec 02 §8.4 — chunk coordinates whose `.chunk` file this session read in
    /// (`save::load_chunk_dir`, from `chunks/` or `autosave/chunks/`) or wrote (every
    /// native save path). A save deletes a chunk's file — the all-air, mined-out
    /// case — only for a coordinate in here: a file this session never read is
    /// unknown data and is left alone, never deleted because the in-memory chunk
    /// there happens to be empty. Interior-mutable because the savers take
    /// `&World`. Runtime-only; forgotten on a change of world folder.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    disk_chunks: std::sync::Mutex<ahash::AHashSet<(i32, i32, i32)>>,
    /// Per-block-position state for blocks that need it (Wave 27 campfires;
    /// Spec 20 furnaces; future brewing stands, signs, etc). Keyed by
    /// world-space block position. The value is the tagged
    /// `BlockEntityData` enum; per-variant convenience getters on World
    /// (`campfire_at`, `furnace_at`, etc.) downcast for the common
    /// call sites that don't need the enum directly.
    pub block_entities: AHashMap<(i32, i32, i32), BlockEntityData>,
    /// Wave 29 (log seasoning) — per-position state for Drying Rack blocks.
    /// Mirrors the campfire pattern: a parallel map until Spec 20 Furnace
    /// introduces the BlockEntityData enum framework. Then this folds in
    /// as a one-variant addition; the field name + struct shape are picked
    /// to make that migration mechanical.
    pub drying_racks: AHashMap<(i32, i32, i32), crate::drying_rack::DryingRackData>,
    /// Spec 19 phase 4 — village anchors that have been touched by
    /// `place_villages_for_column`. Keyed by `(grid_x, grid_z)` so each
    /// village appears once even if multiple columns contribute blocks to
    /// it. The value is the anchor's world position; used downstream to
    /// (a) spawn villagers near the anchor exactly once per village,
    /// (b) drive Iron Golem auto-spawn checks (Phase 8), and (c) serialise
    /// village positions on save (Phase 11).
    pub village_anchors: AHashMap<(i32, i32), [i32; 3]>,
    /// Village cells whose initial villager cohort has been spawned. Stops
    /// the spawn pass from duplicating villagers when chunks repeatedly
    /// stream in. Persisted on save (Phase 11).
    pub populated_villages: ahash::AHashSet<(i32, i32)>,
    /// Spec 19 phase 10 — Village Bells the player has placed. World-space
    /// block positions; each acts as a Wandering-Villager beacon over a
    /// 32-block radius and (after a wanderer reaches it) becomes a
    /// player-founded village anchor. Persisted on save (Phase 11).
    pub village_bells: Vec<[i32; 3]>,
    /// Spec 24 (Build Schematics Core) — in-progress builds. Keyed by
    /// the anchor block's world position. The animated builder ticks
    /// each entry at 2 cells/tick; entry is removed on completion or
    /// player-abandon. Persisted on save (Phase 13).
    pub construction_anchors: AHashMap<(i32, i32, i32), crate::plan::ConstructionAnchorData>,
    /// Spec 24 — Architect's Plaques placed by completed builds.
    /// Keyed by plaque block position; value is the derivation chain
    /// for the build. Right-click the plaque opens an attribution
    /// dialog reading from this map. Persisted on save (Phase 13).
    pub architect_plaques: AHashMap<(i32, i32, i32), crate::plan::PlaqueChain>,
    /// Spec 29 — one-shot legacy migration queue. When a v1 save with
    /// raw meat in a furnace input slot loads, the meat is ejected
    /// (the v2 furnace is ore-only per Axolittle's 2026-05-21 playtest
    /// call) and queued here for the GameState / GameServer first-tick
    /// path to spawn as ItemEntities near the furnace block. NOT
    /// persisted on save — drained on each tick.
    pub pending_legacy_meat_drops: Vec<((i32, i32, i32), crate::item::ItemStack)>,
    /// Spec 22 Phase 13 — per-village sats treasury. Funded by 20 % of
    /// quest payouts + 5 % of Vendor Block sales inside the village
    /// radius. Drained by raid bounties. Persisted on save (Phase 11).
    pub village_treasuries: AHashMap<(i32, i32), u64>,
    /// Spec 22 — in-flight raids. Single-round MVP allows multiple
    /// raids in parallel (one per village). The scheduler appends new
    /// entries on the daily-tick roll; the per-tick raid update drains
    /// terminal entries after bounty payout / reputation effects fire.
    /// Persisted on save (Phase 11).
    pub active_raids: Vec<crate::raid::Raid>,
    /// Spec 22 — raid scheduler state (monotonic id counter + last day
    /// the daily roll fired). Persisted alongside `active_raids`.
    pub raid_scheduler: crate::raid::RaidScheduler,
    /// Spec 22 Phase 18 — per-village per-player kill totals across
    /// every raid defended there. Updated on raid-Cleared resolution
    /// (`tally_raid_kills_into_leaderboard`); read by the villager
    /// dialogue to surface top-3 defenders. PlayerKey is the local
    /// PlayerSlot index on alpha (pubkey when Spec 1 Phase 4 lands).
    /// Persisted on save with `#[serde(default)]` so older saves load
    /// with an empty leaderboard.
    pub raid_kills: AHashMap<(crate::reputation::VillageId, crate::raid::PlayerKey), u32>,
    /// Spec 27 — registry of `.plan.json` plans for `/buildguide` and
    /// friends: the engine bundle (`World::load_bundled_plans()` at world
    /// init) plus runtime additions (`/importschem`). **Not read by world
    /// generation** — village procgen samples the immutable
    /// `PlanRegistry::bundled()`, so an import never changes what a seed
    /// generates (Phase B0). Persisted? No — rebuilt on load.
    pub plan_registry: crate::plan_registry::PlanRegistry,
    /// Owner-inbox #18 — `block_id → micro-model` render override table. A
    /// registered block draws its baked sub-voxel shell instead of its default
    /// cube/cross. Populated at world init (`load_bundled_micro_models`); empty
    /// until a block is bound (flowers in Phase C). Not persisted — rebuilt from
    /// the engine bundle on load, like `plan_registry`.
    pub micro_registry: crate::micro_model_registry::MicroModelRegistry,
    /// Spec 40 (The Workshop) — per-asset appearance override layer. Authored
    /// 16×16 reskins of existing blocks/mobs, applied at the render seams (block
    /// mesher + entity vertex builder) before the asset's default texture.
    /// Default-empty ⇒ byte-identical render. The authored data is persisted with
    /// the save; the `(asset,face)→layer` map is rebuilt on load against
    /// `texture_gen::texture_count()`.
    pub override_registry: crate::override_registry::OverrideRegistry,
    /// Spec 40 (The Workshop) — the player's authored, GLOBAL wardrobe: the
    /// designs + per-block active selections that follow the player into every
    /// world and are saved to the private Stash (WASM) / a profile file (native).
    /// This is the AUTHORITATIVE authored set; `override_registry` above is the
    /// DERIVED render view built from
    /// `official catalogue → player_wardrobe → per-world override`
    /// (see `official_overrides::resolve_render_set` + `GameState::reapply_overrides`).
    /// Authoring (pin / Wardrobe panel / adopt) mutates THIS; the render view is
    /// rebuilt from it. Kept across `clear()` — like the render catalogue — because
    /// the player's designs are a cross-world asset, not per-world state.
    ///
    /// Typed as an `OverrideRegistry` (not the bare authored `OverrideSet`) purely to
    /// reuse its tested mutation API (`add_block_design`, `set_block_active`, …) —
    /// only its `.set()` is live here; its GPU layer maps / appended buffers are NOT
    /// sampled (the renderer reads `override_registry`). Persistence serialises
    /// `player_wardrobe.set().to_blob_bytes()`.
    pub player_wardrobe: crate::override_registry::OverrideRegistry,
    /// Spec 40 (The Workshop) — the `workshop_projects` side-table: parked,
    /// possibly-inflated redesign WIP that lives in (and persists with) a Workshop
    /// world. Empty in normal worlds. Rides the save (`#[serde(default)]`).
    pub workshop: crate::workshop::WorkshopProjects,
    /// Transient: a `/ws publish <name>` awaiting an explicit `/ws publish confirm`.
    /// Never serialised (publishing is public + permanent — it must be a deliberate,
    /// re-confirmed act; CONSUMING.md §7). `World` has no serde derive — persistence
    /// goes through the `WorldSave` mirror in `save.rs`, which never touches this field,
    /// so a world reload leaves it `None`. Cleared by confirm and by any other /ws
    /// subcommand. (Mirrors the runtime-only `is_workshop` field below: no save hook.)
    pub beacon_publish_pending: Option<String>,
    /// Spec 40 — runtime flag: is this the player's Workshop world? Set from
    /// `WorldMeta.is_workshop` when the world is entered. Drives the **void**
    /// world-gen preset (a blank platform, no terrain) in `generate_column`. Not
    /// serialised — a runtime mirror of the meta flag, default `false`.
    pub is_workshop: bool,

    // ─── Blank-canvas world config (Task B1) — runtime mirrors of WorldMeta ──
    // Set from meta at every load/entry seam so world-gen + gameplay systems can
    // read these without going through the save layer. Never serialised — the
    // canonical values live in WorldMeta (world_meta.json).
    /// Runtime mirror of `WorldMeta.world_type`. `"normal"` or `"flat"`.
    pub world_type: String,
    /// Runtime mirror of `WorldMeta.ground`. Ground block for flat worlds.
    pub ground: String,
    /// Runtime mirror of `WorldMeta.water_depth`. Water-layer depth for flat
    /// worlds with `ground = "water"`.
    pub water_depth: u8,
    /// Runtime mirror of `WorldMeta.time_lock`. `"cycle"` | `"day"` | `"night"`.
    pub time_lock: String,
    /// Runtime mirror of `WorldMeta.mobs_enabled`. When `false`, the mob-spawn
    /// driver skips this world entirely.
    pub mobs_enabled: bool,
    /// #47 — runtime mirror of `WorldMeta.keep_inventory`. When `true`, death
    /// leaves the inventory intact and creates no grave (default `false` =
    /// graves). Set from meta at every load/entry seam, mirroring `mobs_enabled`.
    pub keep_inventory: bool,
    /// #6 — map waypoints (manual pins + rolling death markers). Persisted per
    /// world in `WorldSave.waypoints` (append-only, serde-default).
    pub waypoints: Vec<crate::waypoint::Waypoint>,
    /// Creator-gallery exhibits (Spec 2026-06-19 §9). The live, authoritative
    /// list of placed 2D art surfaces — rendered by the painting pipeline (1b)
    /// and authored via `/exhibit` (1c). Restored from `WorldSave.exhibits` on
    /// load; snapshotted back on save (1c). Empty in normal worlds. Mirrors the
    /// `waypoints` runtime/save pattern.
    pub exhibits: Vec<crate::exhibit::Exhibit>,
    /// Transient (session-only, never persisted): the last `/ws browse` result rows,
    /// indexed by the number shown to the player; `/ws adopt <n>` reads this. Filled
    /// by the game-loop browse driver (Task 9). Empty until a browse runs.
    pub beacon_browse_cache: Vec<BrowseEntry>,
    /// Spec 27 Phase 8 — registry of Architect's Plaques the village
    /// procgen pass placed (as opposed to player builds). Looked up
    /// by the plaque dialog to surface the "Sampled by village
    /// procgen" line + the Village Bell "Houses" tab to enumerate
    /// procgen-attributable buildings. Keyed by plaque position.
    /// Persisted? Mirror of `architect_plaques` membership; tracked
    /// separately so we don't need to bump the SaveV1 schema for the
    /// flag — derived from village_anchors on load (a plaque inside
    /// any village's radius is presumed procgen).
    pub procgen_plaque_sources: ahash::AHashSet<(i32, i32, i32)>,
    /// Spec 40 §5 (2026-06-18) — world cells to skip when meshing (render-only:
    /// block DATA is untouched, so raycast / charging / collapse / save are all
    /// unaffected). Used to hide a block while it's blown up in the Workshop so
    /// its original texture doesn't z-fight through the inflated working copy.
    /// Transient — rebuilt each tick by `reconcile_workshop_render_hidden`.
    pub render_hidden: ahash::AHashSet<(i32, i32, i32)>,
    /// HP-3 (2026-05-23) — Brigand Hideout side-table. Keyed by the
    /// hideout's grid cell (HIDEOUT_GRID-sized chunks); the value is
    /// the mutable state (anchor world position, current/target
    /// population, replenish cooldown). The structure itself is
    /// regenerated from `(world_seed, gx, gz)` on column re-stream, so
    /// what persists across save/load is only the spawn-loop state.
    pub brigand_hideouts: AHashMap<(i32, i32), crate::brigand_hideout_gen::HideoutData>,
    /// Salt feature — index of every placed SALT_LICK block.
    /// Populated at place-time, drained at break-time, rebuilt on
    /// load via `rebuild_salt_lick_index`. NOT serialised — the
    /// block placement IS the canonical state; this is just a
    /// derived cache for fast aura lookups (`regen_multiplier_at_pos`,
    /// `wander_bias_for_salt_lick`, `within_any_salt_lick_aura`).
    pub salt_licks: ahash::AHashSet<(i32, i32, i32)>,
    /// Rubber feature — cooldown index for tapped rubber logs.
    /// Maps `(x, y, z)` to the tick the tap occurred. Drained when
    /// the cooldown driver restores the live RUBBER_LOG. NOT
    /// serialised — rebuilt from `RUBBER_LOG_TAPPED` block scan on
    /// load (cooldown clock continues from the load tick so a save
    /// + reload doesn't grant instant ready-again).
    pub tapped_rubber_logs: ahash::AHashMap<(i32, i32, i32), u64>,
    /// Spec 33 Mob Bounty Board — active bounties in the current
    /// rotation. Refreshed every BOUNTY_REFRESH_TICKS by
    /// `bounty::tick_bounty_refresh`. Persisted via WorldSave.
    pub bounties: Vec<crate::bounty::ActiveBounty>,
    /// Monotonic bounty id allocator. Persisted so claim history
    /// (per-player `bounties_claimed`) stays meaningful across save/
    /// reload. 0 = sentinel (uninitialised); allocator starts at 1.
    pub bounty_next_id: u32,
    /// `tick_counter` when `bounties` was last rolled. The refresh
    /// driver compares `monotonic_tick / BOUNTY_REFRESH_TICKS` to
    /// `bounty_last_refresh_tick / BOUNTY_REFRESH_TICKS` to decide
    /// whether to re-roll.
    pub bounty_last_refresh_tick: u64,
    /// Satoshi onboarding — per-world guide progress + house placement.
    /// Mirrors `WorldSave.satoshi`; round-trips through save like `bounties`.
    /// The Satoshi *entity* is re-derived on load (not serialised); only this
    /// state persists.
    pub satoshi: crate::satoshi::SatoshiState,
    /// Spec 36 Plot Ownership — claimed plots. Linear scan on every
    /// place/break (alpha-scale; mirrors the salt_licks posture).
    /// Persisted via WorldSave.plots.
    pub plots: Vec<crate::plot::PlotData>,
    /// Spec 37 Market Hubs — vendor-discovery zones. Linear scan
    /// (alpha-scale). Persisted via WorldSave.market_hubs.
    pub market_hubs: Vec<crate::market_hub::MarketHubData>,
    /// Owner-inbox #1/2/3 (2026-06-03) — face attachments (generalises the
    /// face-overlay wallpaper). Maps a block position to one optional
    /// `FaceAttachment` (`Wallpaper` or `Blueprint`) per face. Sparse (only
    /// painted blocks appear). Persisted via `WorldSave.face_overlays` (NOT a
    /// derived cache like `salt_licks` — a face attachment has no block to
    /// rebuild from). Drawn as per-chunk decal geometry by the mesher.
    pub face_attachments: AHashMap<(i32, i32, i32), FaceAttachments>,

    // ─── Proof-of-Play per-world stats (Spec 2 §9.1) ───
    // Runtime mirrors of the WorldMeta fields: seeded from meta on world load,
    // accrued during play, flushed back to meta on save. NOT part of world.dat
    // (World isn't serialised) — WorldMeta is the on-disk source of truth.
    /// Lifetime proof-of-play work (sum of `crafting::block_work` over
    /// successful `can_harvest` breaks). See work-based-hashing foundations.
    pub total_work: u64,
    /// Lifetime active ticks — the world-clock; advances only while the world
    /// is being played (NOT wall-clock). Basis for Satori-Rush time-to-genesis.
    pub total_ticks: u64,

    // ─── Spec 48 (Electricity / Power & Logic) ───
    /// Sparse per-block metadata byte (facing/state/aux — see [`crate::meta`]).
    /// Absent ⇒ 0, the default for every plain block; only directional or
    /// stateful blocks (levers, gates, mirrors; the building-blocks backlog)
    /// ever store a byte. Persisted via `WorldSave.block_meta`.
    pub block_meta: AHashMap<(i32, i32, i32), u8>,
    /// Neighbour-update / scheduled-tick scheduler. Transient — never
    /// serialised; re-seeded on world load by enqueuing every power source so
    /// `energised` state is rederived rather than saved.
    pub scheduler: crate::block_update::UpdateScheduler,
    /// Transient energised-conductor state. Never serialised — rederived on
    /// load from saved device state via [`crate::power::reseed_on_load`].
    pub power: crate::power::PowerState,
    /// Rail Freight P3 — in-world hostile-act ledger (cart robberies, future
    /// griefing). Persisted via `WorldSave.hostile_acts`.
    pub hostile_acts: crate::hostile_acts::HostileActLedger,
    /// #19 Rig Studio — placed authored rigs (standing, animated displays). Stored
    /// as world data (not ECS — no physics/AI), rendered by
    /// `entity_model::build_rigged_vertices`. Persisted via `WorldSave.rigs`.
    pub rigs: Vec<RigDisplay>,
}

/// #19 Rig Studio — one placed authored rig: where it stands, which way it
/// faces, and the `RiggedModel` (skeleton + per-part block assignments).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RigDisplay {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub rig: crate::skeleton::RiggedModel,
    /// #19 — which inherited clip this rig plays (author's pick in the Rig
    /// Studio; Walk for every rig placed before the picker existed).
    ///
    /// **Deliberately `serde(skip)`.** `RigDisplay` is serialised INSIDE
    /// `WorldSave.rigs`, a `Vec` in the middle of a positional bincode blob:
    /// appending a field to the element type would shift every byte after it and
    /// silently corrupt the fields that follow (`exhibits`, `composters`,
    /// `saved_mobs`, …) in any save that already contains a rig. The clip
    /// therefore rides the index-aligned `WorldSave.rig_clips` side table, which
    /// IS a genuine append at the end of the blob.
    #[serde(skip)]
    pub clip: crate::anim_set::AnimClip,
}

impl World {
    pub fn new() -> Self {
        Self {
            chunks: AHashMap::new(),
            evicted: AHashMap::new(),
            evicted_columns: ahash::AHashSet::new(),
            worldgen_depth: 0,
            disk_chunks: std::sync::Mutex::new(ahash::AHashSet::new()),
            block_entities: AHashMap::new(),
            drying_racks: AHashMap::new(),
            village_anchors: AHashMap::new(),
            populated_villages: ahash::AHashSet::new(),
            village_bells: Vec::new(),
            construction_anchors: AHashMap::new(),
            architect_plaques: AHashMap::new(),
            pending_legacy_meat_drops: Vec::new(),
            village_treasuries: AHashMap::new(),
            active_raids: Vec::new(),
            raid_scheduler: crate::raid::RaidScheduler::new(),
            raid_kills: AHashMap::new(),
            plan_registry: crate::plan_registry::PlanRegistry::new(),
            micro_registry: crate::micro_model_registry::MicroModelRegistry::new(),
            override_registry: crate::override_registry::OverrideRegistry::new(),
            player_wardrobe: crate::override_registry::OverrideRegistry::new(),
            workshop: crate::workshop::WorkshopProjects::new(),
            beacon_publish_pending: None,
            is_workshop: false,
            world_type: "normal".to_string(),
            ground: "grass".to_string(),
            water_depth: 3,
            time_lock: "cycle".to_string(),
            mobs_enabled: true,
            keep_inventory: false,
            waypoints: Vec::new(),
            exhibits: Vec::new(),
            beacon_browse_cache: Vec::new(),
            procgen_plaque_sources: ahash::AHashSet::new(),
            render_hidden: ahash::AHashSet::new(),
            brigand_hideouts: AHashMap::new(),
            salt_licks: ahash::AHashSet::new(),
            tapped_rubber_logs: ahash::AHashMap::new(),
            bounties: Vec::new(),
            bounty_next_id: 0,
            bounty_last_refresh_tick: 0,
            satoshi: crate::satoshi::SatoshiState::default(),
            plots: Vec::new(),
            market_hubs: Vec::new(),
            face_attachments: AHashMap::new(),
            total_work: 0,
            total_ticks: 0,
            block_meta: AHashMap::new(),
            scheduler: crate::block_update::UpdateScheduler::new(),
            power: crate::power::PowerState::default(),
            hostile_acts: crate::hostile_acts::HostileActLedger::new(),
            rigs: Vec::new(),
        }
    }

    /// Advance the per-world active-tick clock (the world-clock). Called once
    /// per sim tick from both `GameState::tick` and `GameServer::tick` while
    /// the world is being played. Saturating — a world-clock never wraps.
    pub fn tick_world_clock(&mut self) {
        self.total_ticks = self.total_ticks.saturating_add(1);
    }

    /// Tally `work` proof-of-play units into the world's lifetime `total_work`.
    /// `work` is `crafting::block_work(block)` for a successful `can_harvest`
    /// break, or `0` for non-harvest / creative / instant breaks (no work done).
    pub fn add_work(&mut self, work: u64) {
        self.total_work = self.total_work.saturating_add(work);
    }

    /// Apply the world's `time_lock` to a raw cyclic world-time value (0–23999)
    /// and return the **effective** world-time that sky, lighting, and mob-spawn
    /// systems should use.
    ///
    /// - `"cycle"` (default) — returns `raw` unchanged; normal day/night cycle.
    /// - `"day"`   — returns `12000` (solar noon, peak brightness).
    /// - `"night"` — returns `0` (midnight, minimum brightness).
    ///
    /// All consumers that need day/night information should call this rather than
    /// using the raw world-time directly, so blank-canvas worlds with a locked
    /// time work consistently across sky colour, mob spawning, and brigand AI.
    #[inline]
    pub fn effective_world_time(&self, raw: u32) -> u32 {
        match self.time_lock.as_str() {
            "day"   => 12000,
            "night" => 0,
            _       => raw, // "cycle" or any unrecognised value → live clock
        }
    }

    /// Salt feature — full scan of every loaded chunk for SALT_LICK
    /// blocks; rebuilds `salt_licks`. Called at load-time from
    /// `WorldSave::load_finalise` (or equivalent post-load hook) so
    /// the index is consistent with the freshly-restored chunk data
    /// without needing to serialise the index separately.
    pub fn rebuild_salt_lick_index(&mut self) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        // Collect into a Vec first so the iter_chunks borrow is released
        // before we mutate self.salt_licks.
        let found: Vec<(i32, i32, i32)> = self
            .iter_chunks()
            .flat_map(|((cx, cy, cz), chunk)| {
                let mut hits: Vec<(i32, i32, i32)> = Vec::new();
                for lx in 0..cs {
                    for ly in 0..cs {
                        for lz in 0..cs {
                            let block = chunk.get(
                                lx as usize, ly as usize, lz as usize,
                            );
                            if block == crate::block::SALT_LICK {
                                hits.push((
                                    cx * cs + lx,
                                    cy * cs + ly,
                                    cz * cs + lz,
                                ));
                            }
                        }
                    }
                }
                hits
            })
            .collect();
        self.salt_licks.clear();
        for entry in found {
            self.salt_licks.insert(entry);
        }
    }

    /// Rubber feature — full scan of loaded chunks for
    /// RUBBER_LOG_TAPPED blocks; rebuilds `tapped_rubber_logs` with
    /// each entry stamped at `load_tick`. The cooldown clock continues
    /// from the load tick so players don't get instant ready-again
    /// from a save+load cycle.
    pub fn rebuild_tapped_rubber_logs_index(&mut self, load_tick: u64) {
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let found: Vec<(i32, i32, i32)> = self
            .iter_chunks()
            .flat_map(|((cx, cy, cz), chunk)| {
                let mut hits: Vec<(i32, i32, i32)> = Vec::new();
                for lx in 0..cs {
                    for ly in 0..cs {
                        for lz in 0..cs {
                            let block = chunk.get(
                                lx as usize, ly as usize, lz as usize,
                            );
                            if block == crate::block::RUBBER_LOG_TAPPED {
                                hits.push((
                                    cx * cs + lx,
                                    cy * cs + ly,
                                    cz * cs + lz,
                                ));
                            }
                        }
                    }
                }
                hits
            })
            .collect();
        self.tapped_rubber_logs.clear();
        for entry in found {
            self.tapped_rubber_logs.insert(entry, load_tick);
        }
    }

    /// Spec 27 Phase 5 — populate `plan_registry` from the engine-
    /// bundled `.plan.json` files. Called once during world init so
    /// village procgen has content to sample from. Idempotent; safe
    /// to call multiple times (replaces the existing registry).
    pub fn load_bundled_plans(&mut self) {
        self.plan_registry = crate::plan_registry::PlanRegistry::bundled().clone();
    }

    /// Owner-inbox #18 — register the engine's built-in micro-model overrides
    /// into `micro_registry`. Called at client world init (after
    /// `load_bundled_plans`). A registered block renders its baked sub-voxel
    /// shell instead of its default billboard/cube; the renderer uploads the
    /// per-type shell geometry via `Renderer::sync_micro_models`. Per-block
    /// bindings: the three wild dye flowers (CORNFLOWER/FIELD_POPPY/BUTTERCUP).
    pub fn load_bundled_micro_models(&mut self, registry: &crate::block::BlockRegistry) {
        crate::micro_model_assets::register_builtin_micro_models(&mut self.micro_registry, registry);
    }

    pub fn get_chunk(&self, cx: i32, cy: i32, cz: i32) -> Option<&Chunk> {
        self.chunks.get(&(cx, cy, cz))
    }

    /// Kept for API symmetry with `get_chunk` — every current mutation path
    /// (block placement, worldgen, save/load) goes through higher-level
    /// setters rather than a raw `&mut Chunk`, so this has no caller yet.
    #[allow(dead_code)]
    pub fn get_chunk_mut(&mut self, cx: i32, cy: i32, cz: i32) -> Option<&mut Chunk> {
        self.chunks.get_mut(&(cx, cy, cz))
    }

    pub fn insert_chunk(&mut self, cx: i32, cy: i32, cz: i32, chunk: Chunk) {
        self.chunks.insert((cx, cy, cz), chunk);
    }

    pub fn has_chunk(&self, cx: i32, cy: i32, cz: i32) -> bool {
        self.chunks.contains_key(&(cx, cy, cz))
    }

    /// Record that the `.chunk` file for `key` was read in or written by this
    /// session (see the `disk_chunks` field).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn note_disk_chunk(&self, key: (i32, i32, i32)) {
        self.disk_chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key);
    }

    /// Forget every known chunk file — on a change of world folder only.
    pub(crate) fn forget_disk_chunks(&mut self) {
        self.disk_chunks
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Whether this session read in or wrote the `.chunk` file for `key` — the
    /// only chunk files a save may delete (Spec 02 §8.4).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn knows_disk_chunk(&self, key: (i32, i32, i32)) -> bool {
        self.disk_chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&key)
    }

    /// Spec 02 §7.5 — stream a column out. If ANY of its chunks has `persist`,
    /// move ALL of the column's chunks into the evicted store (whole-column, so a
    /// restore never mixes regenerated and restored slices); otherwise drop them
    /// (pristine world-gen regenerates identically). Side tables
    /// (`block_entities` etc.) stay in `World` untouched. Returns true if the
    /// column was kept.
    pub fn evict_column(&mut self, cx: i32, cz: i32) -> bool {
        let keep = (0..=MAX_CHUNK_Y)
            .any(|cy| self.chunks.get(&(cx, cy, cz)).is_some_and(|c| c.persist()));
        for cy in 0..=MAX_CHUNK_Y {
            if let Some(chunk) = self.chunks.remove(&(cx, cy, cz))
                && keep
            {
                self.evicted.insert((cx, cy, cz), chunk);
            }
        }
        if keep {
            self.evicted_columns.insert((cx, cz));
        }
        keep
    }

    /// Spec 02 §7.5 — bring an evicted column back into `chunks`. Returns true
    /// if any evicted chunk was present; the caller must then NOT run
    /// `generate_column` for it (it would refill dug-out empty chunks). An
    /// evicted chunk overwrites any world-gen spill a neighbour's generation left
    /// at the same position while the column was out.
    pub fn restore_column(&mut self, cx: i32, cz: i32) -> bool {
        self.evicted_columns.remove(&(cx, cz));
        let mut any = false;
        for cy in 0..=MAX_CHUNK_Y {
            if let Some(mut chunk) = self.evicted.remove(&(cx, cy, cz)) {
                chunk.mesh_dirty = true;
                self.chunks.insert((cx, cy, cz), chunk);
                any = true;
            }
        }
        any
    }

    /// True if the column has chunks in the evicted store. Test-only today.
    #[cfg(test)]
    pub fn is_column_evicted(&self, cx: i32, cz: i32) -> bool {
        self.evicted_columns.contains(&(cx, cz))
    }

    /// Spec 02 §7.5 — is the column holding block `(x, z)` in the evicted
    /// store? Block reads see its real blocks (read-through); a block write
    /// there goes through to the store, so a remote edit is kept. The sims use
    /// the wider [`World::is_column_present_at`] as their barrier.
    #[inline]
    pub fn is_evicted_at(&self, x: i32, z: i32) -> bool {
        !self.evicted_columns.is_empty()
            && self.evicted_columns.contains(&(
                x.div_euclid(CHUNK_SIZE as i32),
                z.div_euclid(CHUNK_SIZE as i32),
            ))
    }

    /// Spec 02 §7.5 — is the column holding block `(x, z)` present: generated
    /// or restored into the live chunks, so at least one of its chunks holds a
    /// real block? False for a column never loaded, one dropped on stream-out,
    /// and an evicted one (its chunks sit in the evicted store). An empty chunk
    /// does not count: block-light BFS leaves light-only chunks in a
    /// never-loaded neighbour, and `generate_column` refills those as absent.
    ///
    /// The world sims (water, lava, fire, sapling growth, entity physics and
    /// mob AI) treat a column that is not present as a barrier: they neither
    /// write into it nor tick inside it. A `set_block` into a never-loaded
    /// column conjures a chunk (known debt), and `generate_column` skips a
    /// non-empty chunk, so a stray lava cell there left the streamed-in column
    /// without its bedrock and stone in that slice (Phase B1 review).
    #[inline]
    pub fn is_column_present_at(&self, x: i32, z: i32) -> bool {
        let (cx, cz) = (x.div_euclid(CHUNK_SIZE as i32), z.div_euclid(CHUNK_SIZE as i32));
        (0..=MAX_CHUNK_Y)
            .any(|cy| self.chunks.get(&(cx, cy, cz)).is_some_and(|c| !c.is_empty()))
    }

    /// The chunk a block/placed-bit write at `(cx, cy, cz)` lands in: the
    /// evicted store for an evicted column (write-through — never a stray in
    /// `chunks` that restore/save would silently discard), else `chunks`.
    /// Creates the chunk if absent, as before.
    fn chunk_for_block_write(&mut self, cx: i32, cy: i32, cz: i32) -> &mut Chunk {
        if self.evicted_columns.contains(&(cx, cz)) {
            self.evicted.entry((cx, cy, cz)).or_insert_with(Chunk::new)
        } else {
            self.chunks.entry((cx, cy, cz)).or_insert_with(Chunk::new)
        }
    }

    /// Read-side twin of `chunk_for_block_write`: `chunks`, then (on a miss)
    /// the evicted store.
    #[inline]
    fn chunk_for_block_read(&self, cx: i32, cy: i32, cz: i32) -> Option<&Chunk> {
        match self.chunks.get(&(cx, cy, cz)) {
            Some(c) => Some(c),
            None if !self.evicted.is_empty() => self.evicted.get(&(cx, cy, cz)),
            None => None,
        }
    }

    /// True if light writes to this chunk column must be dropped (evicted:
    /// light isn't persisted and restore recomputes it).
    #[inline]
    fn light_write_dropped(&self, cx: i32, cz: i32) -> bool {
        !self.evicted_columns.is_empty() && self.evicted_columns.contains(&(cx, cz))
    }

    /// Does this world's generator lay the fixed flat floor (y 79) rather than
    /// biome terrain? True for `"flat"`, and for the retired built-in Gallery
    /// type ([`crate::save::RETIRED_GALLERY_WORLD_TYPE`]) — a save still carrying
    /// it loads as a plain flat world (the Gallery is now an external
    /// `.axeworld` pack), never biome terrain under its maze, never a panic.
    /// World metas are normalised on load, so the retired string only reaches
    /// here via a hand-written scenario def.
    pub fn has_flat_floor(&self) -> bool {
        self.world_type == "flat" || self.world_type == crate::save::RETIRED_GALLERY_WORLD_TYPE
    }

    /// Mirror a world meta's generation + rule flags onto this live `World`
    /// (the runtime mirrors above). Every world-entry seam calls it BEFORE any
    /// column is generated — a local load, the hosted server's `initial_load`,
    /// and a joiner building the host's world from `JoinAccept` (T2-9) — so
    /// they can't drift apart. Explosives and fire spread live on the game /
    /// server state, not here.
    pub fn apply_meta_rules(&mut self, meta: &crate::save::WorldMeta) {
        self.is_workshop = meta.is_workshop;
        self.world_type = meta.world_type.clone();
        self.ground = meta.ground.clone();
        self.water_depth = meta.water_depth;
        self.time_lock = meta.time_lock.clone();
        self.mobs_enabled = meta.mobs_enabled;
        self.keep_inventory = meta.keep_inventory;
    }

    /// Spec 02 §7.5 — every chunk a save must write: the loaded chunks plus the
    /// evicted store. An evicted chunk wins over a loaded one at the same
    /// position (the loaded one can only be world-gen spill from a neighbour).
    /// The all-air delete rule applies to both.
    pub fn persistable_chunks(&self) -> impl Iterator<Item = ((i32, i32, i32), &Chunk)> {
        self.evicted.iter().map(|(&k, c)| (k, c)).chain(
            self.chunks
                .iter()
                .filter(|(k, _)| !self.evicted.contains_key(k))
                .map(|(&k, c)| (k, c)),
        )
    }

    /// Look up one chunk the way `persistable_chunks` sees it (evicted first).
    pub fn persistable_chunk(&self, cx: i32, cy: i32, cz: i32) -> Option<&Chunk> {
        self.evicted.get(&(cx, cy, cz)).or_else(|| self.chunks.get(&(cx, cy, cz)))
    }

    // ── Spec 20 Phase 2: block-entity convenience accessors ──
    // Pre-refactor call sites that used `world.block_entities.get(&pos)`
    // on the `AHashMap<.., CampfireData>` migrate to `world.campfire_at(pos)`
    // — same semantics, one extra match-on-enum hidden inside the helper.
    // The raw `block_entities` map is still accessible for callers that
    // genuinely need to iterate every block-entity variant (the
    // game-loop tick sweep + save serialisation).

    /// Get the Campfire state at `pos`, if there is one.
    pub fn campfire_at(&self, pos: (i32, i32, i32)) -> Option<&crate::campfire::CampfireData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_campfire)
    }

    /// Mutable get for the Campfire state at `pos`.
    pub fn campfire_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::campfire::CampfireData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_campfire_mut)
    }

    /// Insert a Campfire entry at `pos`. Replaces any prior entry —
    /// callers that care about prior-state preservation must check
    /// first with `campfire_at`.
    pub fn insert_campfire(&mut self, pos: (i32, i32, i32), data: crate::campfire::CampfireData) {
        self.block_entities.insert(pos, BlockEntityData::Campfire(data));
    }

    /// `Entry::or_insert_with` analog — returns a mutable Campfire
    /// reference, creating a default one if no entry existed yet.
    /// Used by the right-click-fuel-add path that doesn't care whether
    /// the slot was lit prior. If the slot held a different variant
    /// (e.g. a Furnace at this position — should never happen but
    /// guard against it) the entry is overwritten with a default
    /// Campfire.
    pub fn campfire_at_mut_or_default(&mut self, pos: (i32, i32, i32)) -> &mut crate::campfire::CampfireData {
        let is_campfire = matches!(self.block_entities.get(&pos), Some(BlockEntityData::Campfire(_)));
        if !is_campfire {
            self.block_entities.insert(pos, BlockEntityData::Campfire(crate::campfire::CampfireData::default()));
        }
        match self.block_entities.get_mut(&pos) {
            Some(BlockEntityData::Campfire(c)) => c,
            _ => unreachable!("just inserted Campfire above"),
        }
    }

    /// Iterate every (position, Campfire) pair currently in the
    /// block-entity map. Skips non-Campfire variants. Used by the
    /// per-tick campfire sweep + save serialisation.
    pub fn iter_campfires(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::campfire::CampfireData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_campfire().map(|c| (pos, c)))
    }

    /// Get the Furnace state at `pos`, if there is one.
    pub fn furnace_at(&self, pos: (i32, i32, i32)) -> Option<&crate::furnace::FurnaceData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_furnace)
    }

    pub fn furnace_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::furnace::FurnaceData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_furnace_mut)
    }

    pub fn insert_furnace(&mut self, pos: (i32, i32, i32), data: crate::furnace::FurnaceData) {
        self.block_entities.insert(pos, BlockEntityData::Furnace(data));
    }

    pub fn iter_furnaces(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::furnace::FurnaceData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_furnace().map(|f| (pos, f)))
    }

    // ── Spec 49 (Explosives) Composter ──

    pub fn composter_at(&self, pos: (i32, i32, i32)) -> Option<&crate::workstation::WorkstationState> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_composter)
    }

    pub fn composter_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::workstation::WorkstationState> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_composter_mut)
    }

    pub fn insert_composter(&mut self, pos: (i32, i32, i32), data: crate::workstation::WorkstationState) {
        self.block_entities.insert(pos, BlockEntityData::Composter(data));
    }

    pub fn iter_composters(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::workstation::WorkstationState)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_composter().map(|c| (pos, c)))
    }

    // ── Spec 21 Vendor Block ──

    pub fn vendor_at(&self, pos: (i32, i32, i32)) -> Option<&crate::vendor::VendorData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_vendor)
    }

    pub fn vendor_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::vendor::VendorData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_vendor_mut)
    }

    pub fn insert_vendor(&mut self, pos: (i32, i32, i32), data: crate::vendor::VendorData) {
        self.block_entities.insert(pos, BlockEntityData::Vendor(data));
    }

    pub fn iter_vendors(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::vendor::VendorData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_vendor().map(|v| (pos, v)))
    }

    /// Spec 28d chunk 8 — Hive accessors.
    pub fn hive_at(&self, pos: (i32, i32, i32)) -> Option<&crate::bee_hive::HiveData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_hive)
    }
    pub fn hive_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::bee_hive::HiveData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_hive_mut)
    }
    pub fn insert_hive(&mut self, pos: (i32, i32, i32), data: crate::bee_hive::HiveData) {
        self.block_entities.insert(pos, BlockEntityData::Hive(data));
    }
    pub fn iter_hives(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::bee_hive::HiveData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_hive().map(|h| (pos, h)))
    }

    // ── HP-2 Chest accessors ──

    pub fn chest_at(&self, pos: (i32, i32, i32)) -> Option<&crate::chest::ChestData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_chest)
    }
    pub fn chest_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::chest::ChestData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_chest_mut)
    }
    pub fn insert_chest(&mut self, pos: (i32, i32, i32), data: crate::chest::ChestData) {
        self.block_entities.insert(pos, BlockEntityData::Chest(data));
    }
    pub fn iter_chests(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::chest::ChestData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_chest().map(|c| (pos, c)))
    }

    // ── Wave 2c Sign accessors ──
    pub fn sign_at(&self, pos: (i32, i32, i32)) -> Option<&crate::sign::SignData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_sign)
    }
    pub fn sign_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::sign::SignData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_sign_mut)
    }
    pub fn insert_sign(&mut self, pos: (i32, i32, i32), data: crate::sign::SignData) {
        self.block_entities.insert(pos, BlockEntityData::Sign(data));
    }
    pub fn iter_signs(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::sign::SignData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_sign().map(|s| (pos, s)))
    }

    // ── Wave 2c Item Frame accessors ──
    pub fn item_frame_at(&self, pos: (i32, i32, i32)) -> Option<&crate::item_frame::ItemFrameData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_item_frame)
    }
    pub fn item_frame_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::item_frame::ItemFrameData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_item_frame_mut)
    }
    pub fn insert_item_frame(&mut self, pos: (i32, i32, i32), data: crate::item_frame::ItemFrameData) {
        self.block_entities.insert(pos, BlockEntityData::ItemFrame(data));
    }
    pub fn iter_item_frames(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::item_frame::ItemFrameData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_item_frame().map(|f| (pos, f)))
    }

    // ── #47 Grave accessors ──

    pub fn grave_at(&self, pos: (i32, i32, i32)) -> Option<&crate::grave::GraveData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_grave)
    }
    /// See `as_grave_mut` — kept for API symmetry with `grave_at`; the live
    /// mutation path clones + `insert_grave`s instead.
    #[allow(dead_code)]
    pub fn grave_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::grave::GraveData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_grave_mut)
    }
    pub fn insert_grave(&mut self, pos: (i32, i32, i32), data: crate::grave::GraveData) {
        self.block_entities.insert(pos, BlockEntityData::Grave(data));
    }
    pub fn iter_graves(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::grave::GraveData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_grave().map(|g| (pos, g)))
    }

    // ── Spec 34 Tip Jar accessors ─────────────────────────────────
    pub fn tip_jar_at(&self, pos: (i32, i32, i32)) -> Option<&crate::tip_jar::TipJarData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_tip_jar)
    }
    pub fn tip_jar_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::tip_jar::TipJarData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_tip_jar_mut)
    }
    pub fn insert_tip_jar(&mut self, pos: (i32, i32, i32), data: crate::tip_jar::TipJarData) {
        self.block_entities.insert(pos, BlockEntityData::TipJar(data));
    }
    pub fn iter_tip_jars(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::tip_jar::TipJarData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_tip_jar().map(|t| (pos, t)))
    }

    // ── Spec 38 Auction accessors ─────────────────────────────────
    pub fn auction_at(&self, pos: (i32, i32, i32)) -> Option<&crate::auction::AuctionData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_auction)
    }
    pub fn auction_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::auction::AuctionData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_auction_mut)
    }
    pub fn insert_auction(&mut self, pos: (i32, i32, i32), data: crate::auction::AuctionData) {
        self.block_entities.insert(pos, BlockEntityData::Auction(data));
    }
    pub fn iter_auctions(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::auction::AuctionData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_auction().map(|a| (pos, a)))
    }

    // ── Spec 38 (Blueprint / Cyanotype) — Latent Print accessors ─
    pub fn latent_print_at(&self, pos: (i32, i32, i32)) -> Option<&crate::latent_print::LatentPrintData> {
        self.block_entities.get(&pos).and_then(BlockEntityData::as_latent_print)
    }
    /// See `as_latent_print_mut` — kept for API symmetry with
    /// `latent_print_at`; the live mutation path clones + `insert_latent_print`s
    /// instead.
    #[allow(dead_code)]
    pub fn latent_print_at_mut(&mut self, pos: (i32, i32, i32)) -> Option<&mut crate::latent_print::LatentPrintData> {
        self.block_entities.get_mut(&pos).and_then(BlockEntityData::as_latent_print_mut)
    }
    pub fn insert_latent_print(&mut self, pos: (i32, i32, i32), data: crate::latent_print::LatentPrintData) {
        self.block_entities.insert(pos, BlockEntityData::LatentPrint(data));
    }
    pub fn iter_latent_prints(&self) -> impl Iterator<Item = ((i32, i32, i32), &crate::latent_print::LatentPrintData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| be.as_latent_print().map(|l| (pos, l)))
    }

    /// Remove any block-entity entry at `pos`. Variant-agnostic — used
    /// by the universal break-block / set-block-to-air path.
    pub fn remove_block_entity(&mut self, pos: (i32, i32, i32)) {
        self.block_entities.remove(&pos);
    }

    /// Owner-inbox #1/2/3 — attach décor/data (`Wallpaper` or `Blueprint`) to
    /// one face of the block at `pos`. `face_idx` is `crate::mesh::Face::index()`
    /// (0..6). Replaces any existing attachment on that face.
    pub fn set_face_attachment(
        &mut self,
        pos: (i32, i32, i32),
        face_idx: usize,
        data: FaceAttachment,
    ) {
        if face_idx >= 6 {
            return;
        }
        self.face_attachments
            .entry(pos)
            .or_insert_with(|| std::array::from_fn(|_| None))[face_idx] = Some(data);
    }

    /// Owner-inbox #1/2/3 — the attachment on one face, if any. By reference —
    /// `FaceAttachment` is not `Copy` (the Blueprint payload boxes a `PlanData`).
    pub fn face_attachment_at(
        &self,
        pos: (i32, i32, i32),
        face_idx: usize,
    ) -> Option<&FaceAttachment> {
        self.face_attachments
            .get(&pos)
            .and_then(|faces| faces.get(face_idx).and_then(Option::as_ref))
    }

    /// Owner-inbox #1/2/3 — mutable borrow of the attachment on one face, if
    /// any. Later phases mutate a laid Blueprint's develop state in place.
    pub fn face_attachment_at_mut(
        &mut self,
        pos: (i32, i32, i32),
        face_idx: usize,
    ) -> Option<&mut FaceAttachment> {
        self.face_attachments
            .get_mut(&pos)
            .and_then(|faces| faces.get_mut(face_idx).and_then(Option::as_mut))
    }

    /// Owner-inbox #1/2/3 — remove the attachment on one face (the peel action),
    /// returning what was there so the caller can recover the item. Drops the
    /// whole map entry once its last face clears.
    pub fn remove_face_attachment(
        &mut self,
        pos: (i32, i32, i32),
        face_idx: usize,
    ) -> Option<FaceAttachment> {
        let mut removed = None;
        if let Some(faces) = self.face_attachments.get_mut(&pos) {
            if face_idx < 6 {
                removed = faces[face_idx].take();
            }
            if faces.iter().all(Option::is_none) {
                self.face_attachments.remove(&pos);
            }
        }
        removed
    }

    /// Owner-inbox #1/2/3 — remove ALL attachments on the block at `pos` (the
    /// destroy action), returning what was attached so the caller can drop the
    /// recovered items.
    pub fn remove_face_attachments_at(&mut self, pos: (i32, i32, i32)) -> FaceAttachments {
        self.face_attachments
            .remove(&pos)
            .unwrap_or_else(|| std::array::from_fn(|_| None))
    }

    /// Owner-inbox #1/2/3 — iterate every block with attachments + its per-face
    /// array. Used by the mesher to emit decal geometry.
    pub fn iter_face_attachments(
        &self,
    ) -> impl Iterator<Item = ((i32, i32, i32), &FaceAttachments)> {
        self.face_attachments.iter().map(|(&pos, faces)| (pos, faces))
    }

    /// Drop every chunk + every block-entity (campfires, drying-racks).
    /// Used by the menu → Playing transition so a stale world from a
    /// previous session can't bleed into a freshly-created one — without
    /// this clear, `world.generate_column` is idempotent on existing
    /// chunks (skip-if-present) and `chunk_stream::stream_chunks` only
    /// calls `initial_load` when `loaded_columns.is_empty()`, so a
    /// "new world" after a quit-to-menu would silently retain every
    /// chunk + block-entity from the prior world.
    ///
    /// Deliberately preserved across clears (cross-world / cross-session assets):
    /// - `override_registry` — the DERIVED render catalogue (official + player designs);
    ///   rebuilt from `player_wardrobe` via `GameState::reapply_overrides` on entry.
    /// - `player_wardrobe`   — the player's AUTHORED global wardrobe (Spec 40
    ///   persistence); saved to Stash (WASM) / profile file (native); never dropped on
    ///   world reset.
    /// - `plan_registry`     — engine-bundled content constant across world resets.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.evicted.clear();
        self.evicted_columns.clear();
        self.worldgen_depth = 0;
        // `disk_chunks` is deliberately KEPT: it describes the world folder, which
        // a Workshop reset clears in memory but keeps saving to — forgetting it
        // would leave the old build's chunk files to resurrect. A world change
        // forgets it (`world_exit::clear_per_world_fields`).
        self.block_entities.clear();
        self.face_attachments.clear();
        self.drying_racks.clear();
        self.village_anchors.clear();
        self.populated_villages.clear();
        self.village_bells.clear();
        self.construction_anchors.clear();
        self.architect_plaques.clear();
        self.village_treasuries.clear();
        self.active_raids.clear();
        self.raid_scheduler = crate::raid::RaidScheduler::new();
        self.raid_kills.clear();
        self.procgen_plaque_sources.clear();
        self.brigand_hideouts.clear();
        self.bounties.clear();
        self.bounty_next_id = 0;
        self.bounty_last_refresh_tick = 0;
        self.satoshi = crate::satoshi::SatoshiState::default();
        self.plots.clear();
        self.market_hubs.clear();
        // Plan registry deliberately preserved — engine-bundled
        // content is constant across world resets.
    }

    // ── Spec 36 Plot Ownership accessors ──────────────────────────
    /// The plot whose footprint contains column `(x, z)`, if any.
    pub fn plot_at_column(&self, x: i32, z: i32) -> Option<&crate::plot::PlotData> {
        self.plots.iter().find(|p| p.contains_column(x, z))
    }
    /// Remove the plot anchored at `marker_pos` (called when the owner
    /// breaks their marker). No-op if no plot is anchored there.
    pub fn release_plot(&mut self, marker_pos: (i32, i32, i32)) {
        self.plots.retain(|p| p.marker != marker_pos);
    }

    // ── Spec 37 Market Hub accessors ──────────────────────────────
    /// The hub anchored at `bell_pos`, if any.
    pub fn market_hub_at(&self, bell_pos: (i32, i32, i32)) -> Option<&crate::market_hub::MarketHubData> {
        self.market_hubs.iter().find(|h| h.bell == bell_pos)
    }
    /// Remove the hub anchored at `bell_pos` (owner broke the bell).
    pub fn release_market_hub(&mut self, bell_pos: (i32, i32, i32)) {
        self.market_hubs.retain(|h| h.bell != bell_pos);
    }

    /// Iterate every loaded chunk as ((cx, cy, cz), &Chunk). Used by the
    /// crop growth tick (Spec 16 Phase 6) and anything else that needs to
    /// scan world-state across all loaded chunks.
    pub fn iter_chunks(&self) -> impl Iterator<Item = ((i32, i32, i32), &Chunk)> {
        self.chunks.iter().map(|(&k, c)| (k, c))
    }

    /// Get block at world-space position.
    /// Spec 40 §5 — is this world cell currently render-hidden (skipped by the
    /// mesher)? Cheap: short-circuits on the common empty set so the per-cell
    /// meshing loop pays nothing when nothing is blown up.
    #[inline]
    pub fn is_render_hidden(&self, x: i32, y: i32, z: i32) -> bool {
        !self.render_hidden.is_empty() && self.render_hidden.contains(&(x, y, z))
    }

    pub fn get_block(&self, x: i32, y: i32, z: i32) -> BlockId {
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;

        // Spec 02 §7.5 — read-through: an evicted column reads its real blocks.
        self.chunk_for_block_read(cx, cy, cz)
            .map(|c| c.get(lx, ly, lz))
            .unwrap_or(AIR)
    }

    /// Topmost non-air block in the world column `(x, z)` and its Y, scanning
    /// the full loaded vertical range top-down. `None` when the column is
    /// entirely air/unloaded. The surface-height primitive for the minimap
    /// (#6) and the spawn-proof overlay (#8). Delegates the scan to the pure
    /// `minimap::topmost_map_block` so there is one source of truth.
    pub fn highest_block(&self, x: i32, z: i32) -> Option<(i32, BlockId)> {
        let max_y = (MAX_CHUNK_Y + 1) * CHUNK_SIZE as i32 - 1;
        crate::minimap::topmost_map_block(
            max_y,
            0,
            |y| self.get_block(x, y, z),
            |id| id != AIR,
        )
    }

    // ── Spec 48 (Electricity) — per-block metadata + update scheduling ──

    /// Read the metadata byte at `(x, y, z)`. Absent ⇒ 0.
    pub fn meta_at(&self, x: i32, y: i32, z: i32) -> u8 {
        self.block_meta.get(&(x, y, z)).copied().unwrap_or(0)
    }

    /// Set the metadata byte at `pos`. Writing 0 removes the entry so the map
    /// stays sparse (the plain-block default is "no entry").
    pub fn set_meta(&mut self, pos: (i32, i32, i32), m: u8) {
        if m == 0 {
            self.block_meta.remove(&pos);
        } else {
            self.block_meta.insert(pos, m);
        }
    }

    /// Enqueue `pos` itself for power/logic re-evaluation this tick.
    pub fn mark_dirty(&mut self, pos: (i32, i32, i32)) {
        self.scheduler.enqueue(pos);
    }

    /// Enqueue the six cardinal neighbours of `pos` for re-evaluation.
    pub fn notify_neighbours(&mut self, pos: (i32, i32, i32)) {
        self.scheduler.enqueue_neighbours(pos);
    }

    /// Schedule `pos` for re-evaluation `delay` ticks from `now`
    /// (absolute target tick = `now + delay`).
    pub fn schedule_update(
        &mut self,
        pos: (i32, i32, i32),
        delay: u64,
        kind: crate::block_update::ScheduleKind,
        now: u64,
    ) {
        self.scheduler
            .schedule(pos, now.saturating_add(delay), kind);
    }

    /// THE gameplay block-edit path (Spec 48 §2.3): set the block + its meta,
    /// mark the chunk dirty (via `set_block`), queue a `BlockChange` for
    /// broadcast, and notify neighbours so power/logic re-evaluates. Worldgen
    /// and chunk-fill keep calling raw `set_block` (no notify, no broadcast) —
    /// we must not schedule a power re-eval per worldgen block.
    /// Tested below. The live player place/break arms (`game_loop.rs`) don't
    /// call this helper — their edits interleave lighting/drops/block-entity
    /// cleanup and don't broadcast — but as of 2026-07-10 they perform the
    /// same neighbour notify (+ ghost-PowerDevice removal on break); the
    /// regression tests pinning that sequence live in `power.rs`
    /// (`manual_break_sequence_kills_the_network_and_the_ghost_device`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn edit_block(
        &mut self,
        pos: (i32, i32, i32),
        new_block: BlockId,
        meta: u8,
        out: &mut Vec<crate::protocol::BlockChange>,
    ) {
        self.set_block(pos.0, pos.1, pos.2, new_block);
        self.set_meta(pos, meta);
        out.push(crate::protocol::BlockChange::with_meta(
            pos.0, pos.1, pos.2, new_block, meta,
        ));
        self.notify_neighbours(pos);
    }

    /// Apply a server-broadcast [`BlockChange`] to this (client) world: block
    /// id AND metadata. Returns `true` if anything changed so the caller can
    /// re-mesh the chunk. Before this helper the client apply loops set only
    /// the block id and silently dropped `bc.meta` — which broke any state the
    /// server encodes in meta (water depth levels, facing on server-placed
    /// blocks, …).
    ///
    /// The metadata write also folds a Lever's latch bit back into the device
    /// standing there ([`crate::power::sync_device_from_meta`]). BOTH client
    /// apply loops — a joined client's, and the HOST's loopback consumption of
    /// its own server's broadcast — come through here, and either of them
    /// leaving `PowerDeviceData.on` stale lets that machine's own (still
    /// duplicated) power sim darken a wire the server holds lit, then push the
    /// correction back. Doing it inside this one function is deliberate: a
    /// third apply site cannot forget it.
    ///
    /// For the same reason it also rebuilds the `PowerDevice` block-entity when
    /// the broadcast changes the device KIND standing in the cell — dropping an
    /// orphaned device the broadcast deleted (the ghost source), registering one
    /// the broadcast placed. Keyed on the kind, exactly as `hosted_server`'s
    /// joiner-placement arm is, so a lit/unlit twin swap leaves a generator's
    /// fuel and a battery's charge alone.
    pub fn apply_remote_block_change(&mut self, bc: &crate::protocol::BlockChange) -> bool {
        let pos = (bc.x, bc.y, bc.z);
        let old_block = self.get_block(bc.x, bc.y, bc.z);
        let block_changed = old_block != bc.new_block;
        let meta_changed = self.meta_at(bc.x, bc.y, bc.z) != bc.meta;
        if block_changed {
            self.set_block(bc.x, bc.y, bc.z, bc.new_block);
            // Spec 48 (Electricity) — the block-entity behind a power block has
            // to travel with the block id, exactly as it does on the server
            // (`hosted_server`'s joiner-placement arm, same kind table, same
            // KIND key so a lit/unlit twin swap keeps the device's fuel and
            // charge). Latching the lever from the meta byte below was only
            // half of it: when the HOST breaks a Lever / Windmill / Battery the
            // broadcast is "power block → AIR", and without this the receiving
            // client kept the orphaned `PowerDevice` — a ghost source that its
            // own (still duplicated) power sim used to hold the run lit and
            // push `CABLE_LIT` back up to the host. The inverse matters too: a
            // broadcast that puts a power block somewhere this client had none
            // (a joiner's own placement echoed back, a host-side build) needs
            // the device registered or the block is inert here.
            let old_kind = crate::power::device_kind_for_block(old_block);
            let new_kind = crate::power::device_kind_for_block(bc.new_block);
            if old_kind != new_kind {
                self.block_entities.remove(&pos);
                if let Some(kind) = new_kind {
                    self.insert_power_device(
                        pos,
                        crate::power::PowerDeviceData::new(kind, crate::meta::facing(bc.meta)),
                    );
                }
                // …and wake the run so this client's sim re-evaluates it now,
                // rather than leaving the wire lit until something else
                // happens to dirty the network.
                self.mark_dirty(pos);
                self.notify_neighbours(pos);
            }
            // A broadcast break of a container / economy block (a joiner's,
            // validated by the host) leaves no orphan entity here. No spill:
            // the server already spilled its live copy
            // (`HostedServer::spill_container_on_change`), so spilling again
            // would duplicate the contents (audit 2026-09-27, review B1).
            let old_family = mirrored_family(old_block);
            if old_family.is_some()
                && old_family != mirrored_family(bc.new_block)
                && self.block_entities.get(&pos).and_then(BlockEntityData::mirrored_family)
                    == old_family
            {
                self.block_entities.remove(&pos);
            }
            if old_block == block::PLOT_MARKER && bc.new_block != block::PLOT_MARKER {
                self.release_plot(pos);
            }
        }
        if meta_changed {
            self.set_meta(pos, bc.meta);
            crate::power::sync_device_from_meta(self, pos, bc.meta);
        }
        block_changed || meta_changed
    }

    /// Borrow the power device at `pos`, if a `PowerDevice` block-entity lives
    /// there.
    pub fn power_device_at(&self, pos: (i32, i32, i32)) -> Option<&crate::power::PowerDeviceData> {
        match self.block_entities.get(&pos) {
            Some(BlockEntityData::PowerDevice(d)) => Some(d),
            _ => None,
        }
    }

    /// Mutably borrow the power device at `pos`.
    pub fn power_device_at_mut(
        &mut self,
        pos: (i32, i32, i32),
    ) -> Option<&mut crate::power::PowerDeviceData> {
        match self.block_entities.get_mut(&pos) {
            Some(BlockEntityData::PowerDevice(d)) => Some(d),
            _ => None,
        }
    }

    /// Insert / replace the power-device block-entity at `pos` (Spec 48). Used
    /// by the place handler and the save-restore path. Mirrors `insert_furnace`.
    pub fn insert_power_device(&mut self, pos: (i32, i32, i32), data: crate::power::PowerDeviceData) {
        self.block_entities.insert(pos, BlockEntityData::PowerDevice(data));
    }

    /// Borrow the dispenser/dropper at `pos`, if present.
    pub fn dispenser_at(&self, pos: (i32, i32, i32)) -> Option<&crate::dispenser::DispenserData> {
        match self.block_entities.get(&pos) {
            Some(BlockEntityData::Dispenser(d)) => Some(d),
            _ => None,
        }
    }

    /// Mutably borrow the dispenser/dropper at `pos`.
    pub fn dispenser_at_mut(
        &mut self,
        pos: (i32, i32, i32),
    ) -> Option<&mut crate::dispenser::DispenserData> {
        match self.block_entities.get_mut(&pos) {
            Some(BlockEntityData::Dispenser(d)) => Some(d),
            _ => None,
        }
    }

    /// Insert / replace the dispenser block-entity at `pos` (place path +
    /// save restore). Mirrors `insert_power_device`.
    pub fn insert_dispenser(&mut self, pos: (i32, i32, i32), data: crate::dispenser::DispenserData) {
        self.block_entities.insert(pos, BlockEntityData::Dispenser(data));
    }

    /// Iterate all dispenser/dropper block-entities (save snapshot).
    pub fn iter_dispensers(
        &self,
    ) -> impl Iterator<Item = ((i32, i32, i32), &crate::dispenser::DispenserData)> {
        self.block_entities.iter().filter_map(|(p, be)| match be {
            BlockEntityData::Dispenser(d) => Some((*p, d)),
            _ => None,
        })
    }

    /// Iterate every power-device block-entity (Spec 48 save collection).
    /// Mirrors `iter_furnaces`.
    pub fn iter_power_devices(
        &self,
    ) -> impl Iterator<Item = ((i32, i32, i32), &crate::power::PowerDeviceData)> {
        self.block_entities.iter().filter_map(|(&pos, be)| match be {
            BlockEntityData::PowerDevice(d) => Some((pos, d)),
            _ => None,
        })
    }

    /// Set block at world-space position. Creates chunk if needed.
    ///
    /// Vertical bounds guard (Spec 02 — fixed-height column, Y `0..=95`, i.e.
    /// `MAX_CHUNK_Y+1` chunks tall). Horizontal `(x, z)` is unbounded by design,
    /// but `Y` is not: without this guard an out-of-range structure write (a
    /// mineshaft/ravine/village piece that runs off the top or bottom of the
    /// column) would `or_insert_with` a *phantom* chunk at a `cy` outside the
    /// playable range — leaking a chunk that height/lighting/topmost queries
    /// never expect and that nothing ever streams out. Drop the write instead.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block: BlockId) {
        if y < 0 || y >= (MAX_CHUNK_Y + 1) * CHUNK_SIZE as i32 {
            return;
        }
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;

        let in_worldgen = self.worldgen_depth > 0;
        // Spec 02 §7.5 — write-through for an evicted column.
        let chunk = self.chunk_for_block_write(cx, cy, cz);
        // Spec 02 §7.5 — a real change outside world-gen makes the chunk
        // persist-worthy (it no longer matches what `generate_column` produces).
        if !in_worldgen && chunk.get(lx, ly, lz) != block {
            chunk.mark_persist();
        }
        chunk.set(lx, ly, lz, block);
    }

    // ── Spec 06 §2.2 — player-placed mask (anti-farming) ─────────────

    /// True if a player placed the block at `(x, y, z)`. Unloaded chunks
    /// read as natural (`false`). See [`World::place_player_block`].
    pub fn is_placed(&self, x: i32, y: i32, z: i32) -> bool {
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;
        self.chunk_for_block_read(cx, cy, cz)
            .map(|c| c.is_placed(lx, ly, lz))
            .unwrap_or(false)
    }

    /// Set / clear the player-placed bit at `(x, y, z)`. Setting `true`
    /// creates the chunk if needed; clearing on an unloaded chunk is a no-op.
    pub fn set_placed(&mut self, x: i32, y: i32, z: i32, placed: bool) {
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;
        let in_worldgen = self.worldgen_depth > 0;
        let chunk = if placed {
            Some(self.chunk_for_block_write(cx, cy, cz))
        } else if self.evicted_columns.contains(&(cx, cz)) {
            self.evicted.get_mut(&(cx, cy, cz))
        } else {
            self.chunks.get_mut(&(cx, cy, cz))
        };
        if let Some(c) = chunk {
            // Spec 02 §7.5 — the placed mask is saved, so flipping it is an edit.
            if !in_worldgen && c.is_placed(lx, ly, lz) != placed {
                c.mark_persist();
            }
            c.set_placed(lx, ly, lz, placed);
        }
    }

    /// Set a block AND flag it as player-placed (the placement hot path).
    /// Breaking a player-placed block earns no proof-of-play hash/work,
    /// closing the place→break / re-mine farming loop (Spec 06 §2.2). Use
    /// plain [`World::set_block`] for natural / world-gen / system changes.
    pub fn place_player_block(&mut self, x: i32, y: i32, z: i32, block: BlockId) {
        self.set_block(x, y, z, block);
        self.set_placed(x, y, z, true);
    }

    // ── Spec 30 — per-voxel light accessors ──────────────────────────

    /// Block-light level (0..=15) at world position. Returns 0 for
    /// unloaded chunks (no propagation has reached there yet).
    pub fn block_light_at(&self, x: i32, y: i32, z: i32) -> u8 {
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;
        self.chunks
            .get(&(cx, cy, cz))
            .map(|c| c.block_light_at(lx, ly, lz))
            .unwrap_or(0)
    }

    /// Sky-light level (0..=15) at world position. Returns 15 for
    /// unloaded chunks above the build height (open sky by default).
    pub fn sky_light_at(&self, x: i32, y: i32, z: i32) -> u8 {
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;
        self.chunks
            .get(&(cx, cy, cz))
            .map(|c| c.sky_light_at(lx, ly, lz))
            .unwrap_or(15)
    }

    /// Set block-light at world position. Creates chunk if needed.
    pub fn set_block_light_at(&mut self, x: i32, y: i32, z: i32, v: u8) {
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;
        if self.light_write_dropped(cx, cz) {
            return; // Spec 02 §7.5 — no light-only strays for evicted columns
        }
        let chunk = self.chunks.entry((cx, cy, cz)).or_insert_with(Chunk::new);
        chunk.set_block_light_at(lx, ly, lz, v);
    }

    /// Set sky-light at world position. Creates chunk if needed.
    pub fn set_sky_light_at(&mut self, x: i32, y: i32, z: i32, v: u8) {
        let cx = x.div_euclid(CHUNK_SIZE as i32);
        let cy = y.div_euclid(CHUNK_SIZE as i32);
        let cz = z.div_euclid(CHUNK_SIZE as i32);
        let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let ly = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;
        if self.light_write_dropped(cx, cz) {
            return; // Spec 02 §7.5 — no light-only strays for evicted columns
        }
        let chunk = self.chunks.entry((cx, cy, cz)).or_insert_with(Chunk::new);
        chunk.set_sky_light_at(lx, ly, lz, v);
    }

    /// Effective combined light (max of block_light and sky_light minus
    /// 4 — Minecraft's "night threshold" offset). Used by mob-spawn
    /// gating; sky-light goes from 15 at noon to 4 at midnight per
    /// Spec 5 §7, so the -4 bumps the night floor up to 0.
    pub fn effective_light_at(&self, x: i32, y: i32, z: i32) -> u8 {
        let bl = self.block_light_at(x, y, z);
        let sl = self.sky_light_at(x, y, z).saturating_sub(4);
        bl.max(sl)
    }

    /// Per-entity light sample: `(block/15, sky/15)` at the cell 0.9 above
    /// the given FEET position (mid-body — never the solid ground block).
    /// The two channels feed the shader's `max(block, sky * sun.w)` night
    /// formula, so callers bake them into `Vertex.light` / `Vertex.sky_light`
    /// verbatim. Unloaded chunks inherit the per-channel fallbacks (block 0,
    /// sky 15) so distant entities render daylit rather than black.
    pub fn light_channels_at(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        let bx = x.floor() as i32;
        let by = (y + 0.9).floor() as i32;
        let bz = z.floor() as i32;
        (
            self.block_light_at(bx, by, bz) as f32 / 15.0,
            self.sky_light_at(bx, by, bz) as f32 / 15.0,
        )
    }

    /// Get the chunk coordinates for a world-space block position.
    pub fn block_to_chunk(x: i32, y: i32, z: i32) -> (i32, i32, i32) {
        (
            x.div_euclid(CHUNK_SIZE as i32),
            y.div_euclid(CHUNK_SIZE as i32),
            z.div_euclid(CHUNK_SIZE as i32),
        )
    }

    /// Check if a block position is solid.
    pub fn is_solid(&self, x: i32, y: i32, z: i32, registry: &crate::block::BlockRegistry) -> bool {
        registry.is_solid(self.get_block(x, y, z))
    }

    /// F1 — the world-space solid collision boxes at a cell.
    ///
    /// - non-solid / AIR → empty (no collision);
    /// - ordinary full-cube solid → one unit cube at the cell;
    /// - shaped block (slab, stairs, …) → its meta-derived sub-boxes, offset to
    ///   world space.
    ///
    /// Physics resolves the player against these instead of a boolean. Full-cube
    /// solids return exactly the box the old `is_solid` path assumed, so their
    /// behaviour is unchanged.
    /// P-bugfix: is `(x, y, z)` inside a LOCKED Workshop blow-up's cage (the
    /// committed inflated working copy)? Used to give that copy solid collision.
    /// Gated to the Workshop world so it's a cheap `false` everywhere else.
    pub fn is_in_locked_blowup_cage(&self, x: i32, y: i32, z: i32) -> bool {
        self.is_workshop && self.workshop.locked_cage_contains((x, y, z))
    }

    pub fn collision_boxes_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        registry: &crate::block::BlockRegistry,
    ) -> Vec<crate::block_shape::Aabb> {
        use crate::block_shape::{self, Aabb, BlockShape};
        // P-bugfix: a LOCKED Workshop blow-up's inflated working copy is solid —
        // the player should bump into it, not walk through. Its cage cells are
        // otherwise air, so give every cage cell a full-cube box. Gated to the
        // Workshop world (cheap no-op everywhere else).
        if self.is_in_locked_blowup_cage(x, y, z) {
            let (fx, fy, fz) = (x as f32, y as f32, z as f32);
            return vec![Aabb::new([fx, fy, fz], [fx + 1.0, fy + 1.0, fz + 1.0])];
        }
        let id = self.get_block(x, y, z);
        let shape = block_shape::shape_of(id);
        let (fx, fy, fz) = (x as f32, y as f32, z as f32);
        if shape == BlockShape::FullCube {
            if registry.is_solid(id) {
                return vec![Aabb::new([fx, fy, fz], [fx + 1.0, fy + 1.0, fz + 1.0])];
            }
            return Vec::new();
        }
        // Shaped blocks collide via their authored boxes (they are `solid: true`).
        // Connecting shapes (Wall, Pane) derive their geometry from a live
        // neighbour mask rather than stored meta.
        let m = if block_shape::is_connecting(shape) {
            self.connection_mask(x, y, z, shape, registry)
        } else {
            self.meta_at(x, y, z)
        };
        block_shape::collision_aabbs(shape, m)
            .into_iter()
            .map(|b| {
                Aabb::new(
                    [fx + b.min[0], fy + b.min[1], fz + b.min[2]],
                    [fx + b.max[0], fy + b.max[1], fz + b.max[2]],
                )
            })
            .collect()
    }

    /// 4-bit horizontal connection mask for a *connecting* shaped block (Wall,
    /// Pane) at `(x,y,z)`, derived from its live cardinal neighbours. Each set
    /// bit (`block_shape::CONN_*`) means an arm grows toward that neighbour.
    /// Pure read — no stored state, so a wall/pane restyles the moment a
    /// neighbour is placed or broken (the chunk re-mesh + per-tick collision
    /// both call this fresh).
    pub fn connection_mask(
        &self,
        x: i32,
        y: i32,
        z: i32,
        shape: crate::block_shape::BlockShape,
        registry: &crate::block::BlockRegistry,
    ) -> u8 {
        use crate::block_shape::{self, CONN_E, CONN_N, CONN_S, CONN_W};
        let mut m = 0u8;
        if block_shape::connects(shape, self.get_block(x, y, z - 1), registry) {
            m |= CONN_N;
        }
        if block_shape::connects(shape, self.get_block(x, y, z + 1), registry) {
            m |= CONN_S;
        }
        if block_shape::connects(shape, self.get_block(x - 1, y, z), registry) {
            m |= CONN_W;
        }
        if block_shape::connects(shape, self.get_block(x + 1, y, z), registry) {
            m |= CONN_E;
        }
        m
    }

    /// Check if a block position contains water.
    pub fn is_water(&self, x: i32, y: i32, z: i32) -> bool {
        self.get_block(x, y, z) == block::WATER
    }

    /// #30 — true if a climbable block (ladder; future vines) sits at this cell.
    /// Mirrors `is_water` (block-id check, no registry) so the physics climb
    /// branch can read it cheaply.
    pub fn is_climbable(&self, x: i32, y: i32, z: i32) -> bool {
        self.get_block(x, y, z) == block::LADDER
    }

    /// Iterate over all chunk positions.
    pub fn chunk_positions(&self) -> impl Iterator<Item = (i32, i32, i32)> + '_ {
        self.chunks.keys().copied()
    }

    /// Spec 40 (The Workshop) — generate one column of the **void** Workshop
    /// preset: a single flat floor at `workshop::WORKSHOP_FLOOR_Y`, all else
    /// air. No terrain, ores, trees, villages or hideouts. A clean, well-lit
    /// platform to place + inflate assets on.
    ///
    /// The visible surface is **sand** — a light, neutral floor so dark or
    /// coloured assets read clearly against it (the old solid-bedrock floor was
    /// too dark to see your work on). Directly beneath it sits one layer of
    /// **bedrock**: it's unbreakable (the mining handler refuses bedrock — see
    /// `game_loop.rs`), so even if you mine the sand surface you land on bedrock
    /// rather than dropping into the infinite void, and it supports the sand so
    /// the surface never falls. To build downward, build up off the floor first.
    fn generate_workshop_column(&mut self, cx: i32, cz: i32) {
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let cy = floor_y.div_euclid(CHUNK_SIZE as i32);
        let ly = floor_y.rem_euclid(CHUNK_SIZE as i32) as usize;
        if self.chunks.contains_key(&(cx, cy, cz)) {
            return;
        }
        let mut chunk = Chunk::new();
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                // Light sand surface to stand/build on...
                chunk.set(lx, ly, lz, block::SAND);
                // ...over an unbreakable bedrock base — catches a dug surface so
                // you can't fall into the void, and supports the sand so it
                // can't fall. (ly is 15 for the default floor_y=79, so ly-1 is
                // in the same chunk; the guard is belt-and-braces.)
                if ly > 0 {
                    chunk.set(lx, ly - 1, lz, block::BEDROCK);
                }
            }
        }
        self.insert_chunk(cx, cy, cz, chunk);
    }

    /// Blank-canvas flat-world generation. Produces a void column with a flat
    /// floor at `FLAT_FLOOR_Y` (= `WORKSHOP_FLOOR_Y` = 79) so the player spawns
    /// at y=80 onto the surface — the same Y that the Workshop uses, keeping
    /// spawn handling consistent.
    ///
    /// The floor stack is determined by `self.ground`:
    /// - `"none"`:  a single BEDROCK layer at FLAT_FLOOR_Y. No void-fall.
    /// - `"grass"` | `"sand"` | `"stone"` | `"dirt"` | `"snow"`:
    ///   that block at FLAT_FLOOR_Y, BEDROCK at FLAT_FLOOR_Y-1.
    /// - `"water"`: BEDROCK base, then one SAND bottom, then
    ///   `self.water_depth.clamp(1,8)` WATER layers up to FLAT_FLOOR_Y.
    ///   Player spawning at y=80 lands in the top water cell.
    /// - Unknown ground value: treated as `"grass"` (safe default).
    ///
    /// Everything else is air. No terrain, ores, trees, villages, or mobs.
    fn generate_flat_column(&mut self, cx: i32, cz: i32) {
        // All flat worlds share the same floor Y as the Workshop so the
        // existing spawn-at-80 logic works unmodified on all flat presets.
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y; // 79
        let cy = floor_y.div_euclid(CHUNK_SIZE as i32);
        let ly = floor_y.rem_euclid(CHUNK_SIZE as i32) as usize; // 15 for floor_y=79
        if self.chunks.contains_key(&(cx, cy, cz)) {
            return;
        }

        let ground = self.ground.as_str();

        // Resolve the surface block and the layer stack we need to place.
        // Each entry is (local_y_offset_from_floor, block_id) where offset 0
        // is floor_y, offset -1 is floor_y-1, etc.
        // All offsets must satisfy:  ly as i32 + offset >= 0  (stay in chunk).
        // With floor_y=79 → ly=15, the deepest safe offset is -15 (y=64),
        // well above the maximum water depth of 8.
        let layers: Vec<(i32, u16)> = match ground {
            "none" => {
                // Just an unbreakable bedrock slab — no sand overlay.
                vec![(0, block::BEDROCK)]
            }
            "water" => {
                // Bottom-up stack: BEDROCK base, SAND bottom, then N WATER
                // layers with the topmost at floor_y.
                let depth = self.water_depth.clamp(1, 8) as i32;
                let mut stack = Vec::with_capacity((depth + 2) as usize);
                // Bedrock at floor_y - depth - 1
                stack.push((-(depth + 1), block::BEDROCK));
                // Sand at floor_y - depth
                stack.push((-depth, block::SAND));
                // Water from floor_y - depth + 1 up to floor_y (inclusive)
                for i in (-(depth - 1))..=0 {
                    stack.push((i, block::WATER));
                }
                stack
            }
            surface_str => {
                // Map ground name → block id; unknown → grass (safe default).
                let surface = match surface_str {
                    "sand"  => block::SAND,
                    "stone" => block::STONE,
                    "dirt"  => block::DIRT,
                    "snow"  => block::SNOW,
                    _       => block::GRASS, // "grass" + unknown
                };
                vec![(-1, block::BEDROCK), (0, surface)]
            }
        };

        let mut chunk = Chunk::new();
        for lz in 0..CHUNK_SIZE {
            for lx in 0..CHUNK_SIZE {
                for &(offset, blk) in &layers {
                    let local_y = ly as i32 + offset;
                    debug_assert!(
                        local_y >= 0 && local_y < CHUNK_SIZE as i32,
                        "flat layer offset {offset} puts local_y={local_y} outside chunk"
                    );
                    chunk.set(lx, local_y as usize, lz, blk);
                }
            }
        }
        self.insert_chunk(cx, cy, cz, chunk);
    }

    /// Generate all chunks in a column at (cx, cz).
    /// Fills terrain using the BiomeGenerator, then decorates with trees.
    ///
    /// Spec 02 §7.5 — world-gen writes (including trees/structures that spill
    /// into neighbouring chunks) never mark a chunk `persist`; a neighbour's
    /// existing `persist` flag is left as it was.
    pub fn generate_column(&mut self, cx: i32, cz: i32, biome_gen: &BiomeGenerator) {
        self.worldgen_depth += 1;
        self.generate_column_inner(cx, cz, biome_gen);
        self.worldgen_depth -= 1;
    }

    fn generate_column_inner(&mut self, cx: i32, cz: i32, biome_gen: &BiomeGenerator) {
        // Spec 40 (The Workshop) — the Workshop is a BLANK/void authoring space:
        // a single flat floor platform, no terrain, no trees/villages/hideouts.
        // (Keeps the editor distraction-free; the floor gives somewhere to stand
        // and place assets on.)
        if self.is_workshop {
            self.generate_workshop_column(cx, cz);
            return;
        }
        // Blank-canvas flat worlds — void column with a configurable flat floor.
        // Must come AFTER the is_workshop guard so the Workshop keeps its own
        // sand-over-bedrock behaviour unchanged.
        // `has_flat_floor` also catches the retired built-in Gallery type (now an
        // external world pack), so an old gallery world gets a flat floor rather
        // than biome terrain under its saved maze.
        if self.has_flat_floor() {
            self.generate_flat_column(cx, cz);
            return;
        }
        for cy in 0..=MAX_CHUNK_Y {
            // Skip a cy only if its chunk already holds REAL blocks. An *empty*
            // existing chunk — a phantom born when a `set_block(AIR)` (e.g. a
            // structure carve) touches an ungenerated column and `or_insert`s a
            // blank chunk — must still be filled. Skipping it left the cy (and,
            // for cy=0, the unconditional bedrock floor) permanently absent: the
            // "void column / floor-grid-holes" bug where the player drops through.
            if self.chunks.get(&(cx, cy, cz)).is_some_and(|c| !c.is_empty()) {
                continue;
            }
            let mut chunk = Chunk::new();
            let base_y = cy * CHUNK_SIZE as i32;

            for lz in 0..CHUNK_SIZE {
                for lx in 0..CHUNK_SIZE {
                    let wx = cx * CHUNK_SIZE as i32 + lx as i32;
                    let wz = cz * CHUNK_SIZE as i32 + lz as i32;
                    let surface = biome_gen.terrain_height(wx, wz);
                    let biome = biome_gen.biome_at(wx, wz);

                    for ly in 0..CHUNK_SIZE {
                        let wy = base_y + ly as i32;
                        let blk = biome_block_at(wy, surface, biome, biome_gen, wx, wz);
                        if blk != AIR {
                            chunk.set(lx, ly, lz, blk);
                        }
                    }
                }
            }

            if !chunk.is_empty() {
                self.insert_chunk(cx, cy, cz, chunk);
            }
        }

        // Decorate: scatter oak trees
        self.place_trees(cx, cz, biome_gen);

        // Decorate: scatter ground vegetation (tall grass, berry bushes,
        // shoreline papyrus). Runs after trees so it can skip cells a
        // trunk already occupies.
        self.place_vegetation(cx, cz, biome_gen);

        // Decorate: place village blocks that fall inside this column. The
        // anchor lives in a virtual grid cell (`VILLAGE_GRID` chunks wide);
        // every cell within reach gets a partial structure-build for the
        // overlapping blocks here, so villages straddle chunk boundaries
        // correctly without a global pre-pass.
        let mut placed = std::collections::BTreeMap::<(i32, i32), [i32; 3]>::new();
        crate::village_gen::place_villages_for_column(
            self, cx, cz, biome_gen, biome_gen.seed, &mut placed,
        );
        for (k, v) in placed {
            self.village_anchors.insert(k, v);
        }

        // HP-3 — Brigand Hideouts. Sparser than villages (1 per 64×64
        // chunks). The village-distance gate uses pure village sites, not
        // the anchors registered above (Phase B0: order-independent).
        crate::brigand_hideout_gen::place_hideouts_for_column(
            self, cx, cz, biome_gen, biome_gen.seed,
        );

        // Underworld C1 — Ravines. Carve any nearby ravine's slice of this
        // column. Runs last among the terrain features so it cuts cleanly
        // through whatever's above (it only opens to the local surface).
        crate::ravine_gen::place_ravines_for_column(self, cx, cz, biome_gen, biome_gen.seed);

        // Underworld C2 — Mineshafts. Bore the corridor network + supports +
        // loot for any nearby mineshaft that reaches this column.
        crate::mineshaft_gen::place_mineshafts_for_column(self, cx, cz, biome_gen, biome_gen.seed);
    }

    /// Place trees per the column's biome. Consults
    /// `biome_properties(biome).tree_species` for the per-biome species
    /// pool and delegates the actual trunk + canopy shape to
    /// `tree_shapes::place_tree`. Surface must match
    /// `biome_properties(biome).surface_block` (GRASS for forested
    /// biomes today). Tree blocks that fall outside the chunk column
    /// are clipped — matches the legacy oak placer's behaviour.
    fn place_trees(&mut self, cx: i32, cz: i32, biome_gen: &BiomeGenerator) {
        let cs = CHUNK_SIZE as i32;
        let col_min_x = cx * cs;
        let col_min_z = cz * cs;

        // Consult this column AND its 8 neighbours. A tree rooted in a
        // neighbour can have canopy that overflows into this column; we
        // keep only the blocks that land inside (cx, cz). Because
        // eligibility (`tree_at_column_cell`) and shape
        // (`tree_shapes::place_tree`) are both pure — deterministic per
        // world coords + seed — every block of every tree is placed
        // exactly once, by whichever column it falls in, regardless of
        // the order columns are generated. This replaces the legacy
        // clip-at-boundary placer that simply dropped cross-chunk leaves
        // and left bare half-canopies (2026-05-30 playtest). The
        // canopy-extent invariant (< CHUNK_SIZE, enforced by a
        // tree_shapes test) guarantees a canopy never reaches past an
        // immediate neighbour, so this 3×3 window is complete.
        for nz in (cz - 1)..=(cz + 1) {
            for nx in (cx - 1)..=(cx + 1) {
                let src_min_x = nx * cs;
                let src_min_z = nz * cs;
                for lz in 0..cs {
                    for lx in 0..cs {
                        let wx = src_min_x + lx;
                        let wz = src_min_z + lz;
                        let Some((species, surface)) = tree_at_column_cell(wx, wz, biome_gen)
                        else {
                            continue;
                        };
                        for tb in crate::tree_shapes::place_tree(species, wx, wz, biome_gen.seed) {
                            let bx = wx + tb.dx;
                            let by = surface + 1 + tb.dy;
                            let bz = wz + tb.dz;
                            // Keep only the blocks that fall in this column.
                            if bx < col_min_x
                                || bx >= col_min_x + cs
                                || bz < col_min_z
                                || bz >= col_min_z + cs
                            {
                                continue;
                            }
                            if self.get_block(bx, by, bz) == AIR {
                                self.set_block(bx, by, bz, tb.id);
                            }
                        }
                        // Wild bee hive (2026-07-04 gap-fill wave): ~1 tree in
                        // 12 carries a hive on its trunk side (the MC bee-nest
                        // idiom) — THE world source of honey (no recipe exists;
                        // hives were previously registered but unobtainable).
                        // Deterministic per (wx, wz, seed); same column-clip
                        // rule as the tree blocks; block-entity inserted here
                        // so the honey sweep sees it without lazy creation.
                        let hh = (wx as u32)
                            .wrapping_mul(0x9E37_79B9)
                            .wrapping_add((wz as u32).wrapping_mul(0x85EB_CA6B))
                            ^ biome_gen.seed;
                        if hh.is_multiple_of(12) {
                            let (hx, hy, hz) = (wx + 1, surface + 3, wz);
                            if hx >= col_min_x
                                && hx < col_min_x + cs
                                && hz >= col_min_z
                                && hz < col_min_z + cs
                                && self.get_block(hx, hy, hz) == AIR
                            {
                                self.set_block(hx, hy, hz, crate::block::BEE_HIVE);
                                // Found hives start part-stocked so the first
                                // discovery pays off without waiting a cycle.
                                self.insert_hive(
                                    (hx, hy, hz),
                                    crate::bee_hive::HiveData { bees_inside: 1, honey_level: 2 },
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Surface-decoration densities (per-cell roll out of 100). Halved on
    // 2026-05-30 after Axolittle's playtest: flower/grass density was both
    // visually cluttered ("too many flowers") and a frame-rate sink (each
    // plant is an individual non-solid cross-quad mesh). The renderer-side
    // lever is plant-mesh instancing; these constants trim the raw count.
    // Originals: flowers 4%, tall grass 12%.

    /// Scatter ground vegetation across a freshly generated column:
    /// tall grass + berry bushes on grassy surfaces, and papyrus reeds
    /// along the waterline. These are single-block plants, so unlike
    /// trees there's no cross-chunk clipping to worry about.
    ///
    /// Worldgen for these never existed before 2026-05-27 — the blocks
    /// were registered but nothing placed them, so fresh worlds had bare
    /// grass with no tufts, no berries, and no papyrus (the Build
    /// Schematics paper source). Deterministic per `(wx, wz, seed)`.
    /// Spec 02 §5.4 — surface decoration pass.
    fn place_vegetation(&mut self, cx: i32, cz: i32, biome_gen: &BiomeGenerator) {
        const FLOWER_PERCENT: u32 = 2;
        const TALL_GRASS_PERCENT: u32 = 6;
        let cs = CHUNK_SIZE as i32;
        let col_min_x = cx * cs;
        let col_min_z = cz * cs;

        for lz in 0..cs {
            for lx in 0..cs {
                let wx = col_min_x + lx;
                let wz = col_min_z + lz;
                let surface = biome_gen.terrain_height(wx, wz);
                // Nothing grows below the waterline.
                if surface <= SEA_LEVEL {
                    continue;
                }
                let above = surface + 1;
                // The cell we'd plant into must be empty (skip tree trunks,
                // structure blocks, etc.).
                if self.get_block(wx, above, wz) != AIR {
                    continue;
                }
                let surf_block = self.get_block(wx, surface, wz);
                let h = veg_hash(wx, wz, biome_gen.seed);

                // Papyrus reeds — only on the immediate shoreline (one block
                // above sea level) bordering open water. Gives a reed fringe
                // around lakes/oceans rather than scattering them inland.
                if surface == SEA_LEVEL + 1
                    && matches!(surf_block, block::GRASS | block::DIRT | block::SAND)
                {
                    // Pure terrain, not `self.is_water`: at the column edge
                    // the neighbour cell is in another column, which reads
                    // AIR until it generates — so papyrus depended on
                    // generation order (Phase B0).
                    let water_near = [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dz)| {
                        terrain_block_at(biome_gen, wx + dx, SEA_LEVEL, wz + dz) == block::WATER
                    });
                    if water_near {
                        if h % 100 < 35 {
                            self.set_block(wx, above, wz, block::PAPYRUS_STAGE_3);
                        }
                        continue; // a waterline cell doesn't also get grass
                    }
                }

                // Tall grass + berry bushes + flowers + fibre crops —
                // grassy surfaces only, so sand/gravel/rock/snow surfaces
                // are excluded implicitly. One plant per cell, prioritised
                // rare → common; sub-rolls use decorrelated slices of `h`.
                if surf_block != block::GRASS {
                    continue;
                }
                let biome = biome_gen.biome_at(wx, wz);
                if h % 1000 < 10 {
                    // Berry bush — rare resource; placed mature so it's
                    // immediately harvestable.
                    self.set_block(wx, above, wz, block::BERRY_BUSH_3);
                } else if matches!(biome, Biome::Savanna) && h % 100 < 6 {
                    // Cotton (Spec 36) — warm-biome fibre crop → String.
                    self.set_block(wx, above, wz, block::COTTON_PLANT);
                } else if matches!(biome, Biome::Plains | Biome::Forest) && (h / 7) % 100 < 5 {
                    // Hemp (Spec 36) — temperate fibre crop → Rope.
                    self.set_block(wx, above, wz, block::HEMP_PLANT);
                } else if (h / 13) % 100 < FLOWER_PERCENT {
                    // Dye flowers (Spec 35) — species from the position hash.
                    let flower = match (h / 17) % 3 {
                        0 => block::CORNFLOWER,
                        1 => block::FIELD_POPPY,
                        _ => block::BUTTERCUP,
                    };
                    self.set_block(wx, above, wz, flower);
                } else if h % 100 < TALL_GRASS_PERCENT {
                    // Tall grass — common, for a lived-in meadow look.
                    self.set_block(wx, above, wz, block::TALL_GRASS);
                }
            }
        }
    }
}

/// The block plain terrain generation puts at `(x, y, z)` — before trees,
/// vegetation and structures. Pure (only `biome_gen`), so a decoration pass
/// can ask about a neighbour column that has not been generated yet and get
/// the same answer it would after (Phase B0 worldgen purity).
pub(crate) fn terrain_block_at(biome_gen: &BiomeGenerator, x: i32, y: i32, z: i32) -> BlockId {
    let surface = biome_gen.terrain_height(x, z);
    biome_block_at(y, surface, biome_gen.biome_at(x, z), biome_gen, x, z)
}

/// Pure tree-eligibility for one column cell, plus the species + the y of
/// the surface block a tree there would root on. Returns `None` when no
/// tree belongs at `(wx, wz)`.
///
/// Crucially this is derived entirely from `biome_gen` noise (never
/// `self.get_block`), so it gives the same answer whether or not the
/// containing chunk has been generated yet. `place_trees` relies on that:
/// it consults the 8 neighbouring columns so a canopy straddling a chunk
/// boundary is placed whole by whichever column each block lands in. If
/// eligibility read `self.get_block`, a not-yet-generated neighbour would
/// read AIR and the cross-boundary canopy would silently vanish — the
/// 2026-05-30 "lots of trees without leaves" playtest bug.
///
/// The `surface > SEA_LEVEL` gate fixes the sibling "trees spawning in
/// water" bug: the grass-biome column keeps GRASS at `y == surface` even
/// when submerged, so the old surface-block match let trees root on a
/// lakebed. `place_vegetation` already gated on this; trees did not.
pub(crate) fn tree_at_column_cell(
    wx: i32,
    wz: i32,
    biome_gen: &BiomeGenerator,
) -> Option<(crate::block::WoodSpecies, i32)> {
    let biome = biome_gen.biome_at(wx, wz);
    let threshold = tree_threshold_for_biome(biome)?;
    if tree_hash(wx, wz, biome_gen.seed).rem_euclid(threshold) != 0 {
        return None;
    }
    let props = crate::biome::biome_properties(biome);
    if props.tree_species.is_empty() {
        return None;
    }
    let surface = biome_gen.terrain_height(wx, wz);
    // Reject only genuinely submerged surfaces. A surface *at* SEA_LEVEL is
    // dry land at the waterline — water fills `y > surface && y <= SEA_LEVEL`,
    // which is empty there — so a tree roots fine on it. The earlier `<=`
    // stripped every waterline cell, a big slice of lake-prone biomes like
    // Jungle, which is why jungles read as bare (2026-05-30).
    if surface < SEA_LEVEL {
        return None; // below the waterline → would grow out of water
    }
    // Surface block must be the biome's land surface (GRASS/SAND/SNOW…),
    // computed purely so neighbour columns can be consulted pre-generation.
    if biome_block_at(surface, surface, biome, biome_gen, wx, wz) != props.surface_block {
        return None;
    }
    let species_idx = species_pick_for(wx, wz, biome_gen.seed, props.tree_species.len());
    Some((props.tree_species[species_idx], surface))
}

/// Pure species-pick from world position + seed. Independent of
/// `tree_hash` so the pick stays uniform even when the spawn-gate
/// threshold and pool size share a common factor (see place_trees
/// comment). Folding `seed` in means two worlds with different seeds
/// get different Rubber/Jungle distributions at identical coords.
pub(crate) fn species_pick_for(wx: i32, wz: i32, seed: u32, pool_len: usize) -> usize {
    if pool_len == 0 {
        return 0;
    }
    let mut h: u32 = (wx as u32).wrapping_mul(0xCAFE_BABE)
        ^ (wz as u32).wrapping_mul(0xDEAD_BEEF)
        ^ seed.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    (h as usize) % pool_len
}

/// Per-biome tree-placement threshold. `tree_hash % threshold == 0` is
/// the spawn gate. Returns `None` for treeless biomes (Desert, Ocean,
/// Mountains today). Forest + Plains preserve their pre-Spec-28b
/// rates exactly to avoid jarring Axo's existing terrain. Jungle is
/// dense (1/10) to match Minecraft expectations; Birch/Taiga match
/// Forest; Savanna + SnowyTundra are sparse.
pub(crate) fn tree_threshold_for_biome(biome: Biome) -> Option<i32> {
    match biome {
        Biome::Forest | Biome::BirchForest => Some(20),
        Biome::Plains => Some(50),
        Biome::Jungle => Some(10),
        Biome::Taiga => Some(15),
        Biome::Savanna => Some(80),
        Biome::SnowyTundra => Some(80),
        Biome::Desert | Biome::Ocean | Biome::Mountains => None,
    }
}

/// Determine block type at a given world position using biome rules.
fn biome_block_at(y: i32, surface: i32, biome: Biome, biome_gen: &BiomeGenerator, wx: i32, wz: i32) -> BlockId {
    if y == 0 {
        return block::BEDROCK;
    }

    // Cave carving (only below surface, above bedrock)
    if y > 1 && y < surface - 1 && biome_gen.is_cave(wx, y, wz) {
        // P10 — deep caves pool with lava (Minecraft's deep-cave lava lakes),
        // giving survival players a real lava source + light + hazard down low.
        if y <= LAVA_CAVE_LEVEL {
            return block::LAVA;
        }
        // Below the water table (but above the lava level), caves fill with water.
        if y <= SEA_LEVEL {
            return block::WATER;
        }
        return block::AIR;
    }

    if y < surface - 3 {
        // Ore overlay first; if no ore, fall through to the underlying base
        // rock (stone or pure deepslate per Spec 2 §5.3.1a). Both ore_at +
        // base_rock_at are deterministic on (x,y,z,seed).
        return biome_gen
            .ore_at(wx, y, wz)
            .unwrap_or_else(|| biome_gen.base_rock_at(wx, y, wz));
    }

    match biome {
        Biome::Desert => {
            if y < surface - 1 { block::SANDSTONE }
            else if y <= surface { block::SAND }
            else if y <= SEA_LEVEL { block::WATER }
            else { block::AIR }
        }
        Biome::Ocean => {
            if y < surface { biome_gen.base_rock_at(wx, y, wz) }
            else if y == surface { block::GRAVEL }
            else if y <= SEA_LEVEL { block::WATER }
            else { block::AIR }
        }
        Biome::Mountains => {
            if y < surface {
                if y > 80 {
                    biome_gen
                        .ore_at(wx, y, wz)
                        .unwrap_or_else(|| biome_gen.base_rock_at(wx, y, wz))
                } else { block::DIRT }
            } else if y == surface {
                if surface > 85 { block::SNOW } else { biome_gen.base_rock_at(wx, y, wz) }
            } else { block::AIR }
        }
        Biome::SnowyTundra => {
            // Snow surface so it reads as tundra and matches
            // `biome_properties(SnowyTundra).surface_block` (SNOW), which
            // tree placement checks before dropping a spruce.
            if y < surface { block::DIRT }
            else if y == surface { block::SNOW }
            else if y <= SEA_LEVEL { block::WATER }
            else { block::AIR }
        }
        _ => {
            // Plains, Forest, BirchForest, Taiga, Jungle, Savanna — all
            // grass-surfaced launch biomes.
            if y < surface { block::DIRT }
            else if y == surface { block::GRASS }
            else if y <= SEA_LEVEL { block::WATER }
            else { block::AIR }
        }
    }
}

/// Deterministic hash for tree placement. Now seeded — before, it ignored
/// the world seed, so tree *positions* were identical in every world (owner
/// inbox #8) and `tree_hash(0,0) == 0` meant a tree was always placed at the
/// origin in a tree biome (the canopy the player then spawned on top of, #9).
/// Folding the seed in (mirroring `veg_hash`) varies tree layout per world
/// and removes the fixed origin tree.
fn tree_hash(x: i32, z: i32, seed: u32) -> i32 {
    let mut h = x.wrapping_mul(374761393)
        ^ z.wrapping_mul(668265263)
        ^ (seed as i32).wrapping_mul(-1640531527); // 0x9E3779B9
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h = h ^ (h >> 16);
    h & i32::MAX // non-negative without the i32::MIN.abs() panic
}

/// Deterministic hash for vegetation placement. Seeded (unlike
/// `tree_hash`) so two worlds with different seeds scatter grass,
/// berries, and reeds differently. Distinct constants from `tree_hash`
/// so a tree and a grass tuft don't gate on a correlated value.
fn veg_hash(x: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27D4_EB2F)
        ^ (z as u32).wrapping_mul(0x1656_67B1)
        ^ seed.wrapping_mul(0x9E37_79B9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_channels_at_samples_mid_body_cell() {
        // Entities pass their FEET position; the sampler reads the cell 0.9 up
        // so a mob standing on the ground samples the air it occupies, not the
        // solid block under it.
        let mut w = World::new();
        w.set_block_light_at(5, 5, 5, 12); // torch-lit cave cell
        w.set_sky_light_at(5, 5, 5, 0);
        let (block, sky) = w.light_channels_at(5.2, 4.6, 5.9); // floor(4.6+0.9)=5
        assert!((block - 12.0 / 15.0).abs() < 1e-6);
        assert_eq!(sky, 0.0);
    }

    #[test]
    fn light_channels_at_open_sky() {
        let mut w = World::new();
        w.set_sky_light_at(0, 10, 0, 15);
        let (block, sky) = w.light_channels_at(0.5, 9.5, 0.5);
        assert_eq!(block, 0.0);
        assert!((sky - 1.0).abs() < 1e-6);
    }

    #[test]
    fn light_channels_at_unloaded_chunk_defaults_to_open_sky() {
        // Inherits the per-channel fallbacks: block 0 (no propagation yet),
        // sky 15 (open sky) — so far-away entities render daylit, never black.
        let w = World::new();
        let (block, sky) = w.light_channels_at(9999.0, 40.0, 9999.0);
        assert_eq!(block, 0.0);
        assert!((sky - 1.0).abs() < 1e-6);
    }

    #[test]
    fn block_has_remote_apply_effects_names_every_block_the_apply_keeps_an_entity_for() {
        use crate::block::*;
        for b in [
            CHEST, COPPER_CHEST, FURNACE, FURNACE_LIT, DISPENSER, GRAVE, VENDOR_BLOCK, TIP_JAR,
            AUCTION_BLOCK, LEVER, BATTERY, ELECTRIC_LAMP, ELECTRIC_LAMP_LIT, WINDMILL,
            PLOT_MARKER,
        ] {
            assert!(block_has_remote_apply_effects(b), "block {b} carries apply side effects");
        }
        for b in [AIR, STONE, DIRT, COBBLESTONE, CABLE, CABLE_LIT, WATER] {
            assert!(!block_has_remote_apply_effects(b), "block {b} is a plain write");
        }
    }

    #[test]
    fn apply_remote_block_change_sets_block_and_meta() {
        // Regression: the client apply loops used to write only the block id
        // and drop `bc.meta`, so server-encoded state (water depth, facing)
        // never reached remote players.
        let mut w = World::new();
        let bc = crate::protocol::BlockChange::with_meta(3, 10, 5, crate::block::WATER, 0b0110_0000);
        assert!(w.apply_remote_block_change(&bc), "first apply reports a change");
        assert_eq!(w.get_block(3, 10, 5), crate::block::WATER);
        assert_eq!(w.meta_at(3, 10, 5), 0b0110_0000);
        // Re-applying the identical change is a no-op (no wasted re-mesh).
        assert!(!w.apply_remote_block_change(&bc), "identical re-apply is a no-op");
        // A meta-only change (level drop) still reports true.
        let bc2 = crate::protocol::BlockChange::with_meta(3, 10, 5, crate::block::WATER, 0b0100_0000);
        assert!(w.apply_remote_block_change(&bc2), "meta-only change re-meshes");
        assert_eq!(w.meta_at(3, 10, 5), 0b0100_0000);
    }

    #[test]
    fn apply_remote_block_change_latches_the_lever_the_broadcast_describes() {
        // Task 2b review fix. A joiner asks the host to flip a lever and never
        // touches it locally, so the latch reaches every OTHER copy of the
        // world — including the HOST's own client, which consumes its server's
        // broadcast over the loopback transport — as nothing but the metadata
        // state bit. Both apply loops go through this one function, and if it
        // leaves `PowerDeviceData.on` stale then that machine's own power sim
        // recomputes the run as unpowered the next time anything nearby dirties
        // it, and pushes the correction back: the wire goes dark for everyone.
        let pos = (2, 64, 2);
        let mut w = World::new();
        w.set_block(pos.0, pos.1, pos.2, crate::block::LEVER);
        w.insert_power_device(
            pos,
            crate::power::PowerDeviceData::new(
                crate::power::PowerDeviceKind::Lever,
                crate::meta::Facing::East,
            ),
        );
        assert!(!w.power_device_at(pos).unwrap().on, "fixture: the lever starts down");

        let on_meta = crate::meta::with_state(crate::meta::with_facing(0, crate::meta::Facing::East), 1);
        let lit = crate::protocol::BlockChange::with_meta(
            pos.0, pos.1, pos.2, crate::block::LEVER, on_meta,
        );
        assert!(w.apply_remote_block_change(&lit));
        assert!(
            w.power_device_at(pos).unwrap().on,
            "the broadcast latch bit reaches the device, not just the meta byte"
        );

        // …and back down again.
        let off_meta = crate::meta::with_state(on_meta, 0);
        let dark = crate::protocol::BlockChange::with_meta(
            pos.0, pos.1, pos.2, crate::block::LEVER, off_meta,
        );
        assert!(w.apply_remote_block_change(&dark));
        assert!(!w.power_device_at(pos).unwrap().on, "and switching off travels too");
    }

    #[test]
    fn apply_remote_block_change_removes_the_device_a_broadcast_break_deletes() {
        // Whole-branch review, IMPORTANT. The host breaks a Lever (or Windmill,
        // or Battery) and broadcasts "power block → AIR". Pre-fix the receiving
        // client wrote the AIR and left the `PowerDevice` block-entity standing
        // there — a ghost source. That client's own (still duplicated) power sim
        // then kept sourcing the run from a lever nobody can see, held the wire
        // at CABLE_LIT, and pushed that back up to the host.
        let pos = (4, 64, 4);
        let mut w = World::new();
        w.set_block(pos.0, pos.1, pos.2, crate::block::LEVER);
        w.insert_power_device(
            pos,
            crate::power::PowerDeviceData::new(
                crate::power::PowerDeviceKind::Lever,
                crate::meta::Facing::East,
            ),
        );
        w.power_device_at_mut(pos).unwrap().on = true;
        // Drain anything the fixture queued so the notify assertion below is
        // about THIS apply.
        let _ = w.scheduler.take_pending(100);

        let broken = crate::protocol::BlockChange::with_meta(pos.0, pos.1, pos.2, crate::block::AIR, 0);
        assert!(w.apply_remote_block_change(&broken), "the break re-meshes");
        assert_eq!(w.get_block(pos.0, pos.1, pos.2), crate::block::AIR);
        assert!(
            w.power_device_at(pos).is_none(),
            "the orphaned PowerDevice must go with the block, or it sources the run forever"
        );
        let woken = w.scheduler.take_pending(100);
        assert!(
            woken.contains(&pos) && woken.contains(&(pos.0 + 1, pos.1, pos.2)),
            "the cell and its neighbours are queued so the client's sim darkens the run now, \
             not whenever something else happens to dirty it (got {woken:?})"
        );
    }

    #[test]
    fn apply_remote_block_change_registers_a_device_a_broadcast_places() {
        // The inverse of the ghost: a broadcast that puts a power block where
        // this client had none (a host-side build, or a joiner's own placement
        // echoed back before it ever touched the cell) has to register the
        // device, or the block sits inert in this copy of the world. Mirrors
        // `hosted_server`'s joiner-placement arm — same kind table, same
        // meta-borne facing.
        let pos = (5, 64, 5);
        let mut w = World::new();
        let facing_meta = crate::meta::with_facing(0, crate::meta::Facing::South);
        let placed = crate::protocol::BlockChange::with_meta(
            pos.0, pos.1, pos.2, crate::block::WINDMILL, facing_meta,
        );
        assert!(w.apply_remote_block_change(&placed));
        let d = w.power_device_at(pos).expect("the broadcast placement registers a device");
        assert_eq!(d.kind, crate::power::PowerDeviceKind::Windmill);
        assert_eq!(d.facing, crate::meta::Facing::South, "facing rides the meta byte");
    }

    #[test]
    fn apply_remote_block_change_keeps_the_device_across_a_lit_twin_swap() {
        // Keyed on the device KIND, exactly as the server's arm is: a Steam
        // Generator lighting up is STEAM_GENERATOR → STEAM_GENERATOR_LIT, the
        // same kind, so the device — and the fuel burning inside it — must
        // survive. Rebuilding on every block id would have emptied the hopper
        // every time the fire caught.
        let pos = (6, 64, 6);
        let mut w = World::new();
        w.set_block(pos.0, pos.1, pos.2, crate::block::STEAM_GENERATOR);
        w.insert_power_device(
            pos,
            crate::power::PowerDeviceData::new(
                crate::power::PowerDeviceKind::SteamGenerator,
                crate::meta::Facing::North,
            ),
        );
        w.power_device_at_mut(pos).unwrap().charge = 7;

        let lit = crate::protocol::BlockChange::with_meta(
            pos.0, pos.1, pos.2, crate::block::STEAM_GENERATOR_LIT, 0,
        );
        assert!(w.apply_remote_block_change(&lit));
        assert_eq!(
            w.power_device_at(pos).map(|d| d.charge),
            Some(7),
            "a lit/unlit twin swap is the same kind — the device is left alone"
        );
    }

    #[test]
    fn generate_column_refills_empty_phantom_chunk_lays_bedrock() {
        // Root cause of the "void column / floor full of grid holes" bug: a
        // `set_block(.., AIR, ..)` into an UNGENERATED column `or_insert`s an
        // EMPTY phantom chunk (Chunk::new is all-air; setting AIR is a no-op).
        // Pre-fix, generate_column's `contains_key` skip stepped over that cy and
        // never laid its terrain — and for cy=0 that meant no bedrock floor, so
        // the player dropped through forever. The fix skips a cy only when its
        // chunk is NON-empty, so an empty phantom is refilled.
        let biome_gen = crate::biome::BiomeGenerator::new(1);
        let mut w = World::new();
        let (cx, cz) = (5, 5);
        let (sx, sz) = (cx * CHUNK_SIZE as i32 + 4, cz * CHUNK_SIZE as i32 + 4);
        w.set_block(sx, 3, sz, crate::block::AIR);
        assert!(w.has_chunk(cx, 0, cz), "set_block(AIR) created a phantom chunk at cy=0");
        assert!(
            w.chunks.get(&(cx, 0, cz)).unwrap().is_empty(),
            "the phantom chunk is empty (would block the bedrock fill pre-fix)",
        );
        w.generate_column(cx, cz, &biome_gen);
        assert_eq!(
            w.get_block(cx * CHUNK_SIZE as i32 + 8, 0, cz * CHUNK_SIZE as i32 + 8),
            crate::block::BEDROCK,
            "generate_column must refill the empty phantom chunk's bedrock floor",
        );
    }

    #[test]
    fn generate_column_floors_and_surfaces_every_column() {
        // Fresh-world repro: generate a wide grid (covering the village/mineshaft/
        // ravine/hideout structure grids) and assert EVERY column has its bedrock
        // floor at y=0. A "fall-forever" void column = a missing or bedrock-less
        // bottom chunk. Several seeds because structures are sparse + seed-driven.
        for seed in [1u32, 7, 42, 1337, 90210] {
            let biome_gen = crate::biome::BiomeGenerator::new(seed);
            let mut w = World::new();
            let r = 12; // 25×25 columns ≈ 400×400 blocks — wide enough for structures
            for cx in -r..=r {
                for cz in -r..=r {
                    w.generate_column(cx, cz, &biome_gen);
                }
            }
            let mut void_surface = 0u32;
            let mut examples = Vec::new();
            for cx in -r..=r {
                for cz in -r..=r {
                    for &(ox, oz) in &[(2, 2), (8, 8), (13, 13)] {
                        let wx = cx * CHUNK_SIZE as i32 + ox;
                        let wz = cz * CHUNK_SIZE as i32 + oz;
                        assert_eq!(
                            w.get_block(wx, 0, wz),
                            crate::block::BEDROCK,
                            "seed {seed}: ({wx},0,{wz}) no bedrock — bottomless VOID",
                        );
                        // Surface must be solid ground. A skipped surface chunk
                        // (pre-created by a neighbour's tree/structure) shows up as
                        // air all the way down from the expected surface height.
                        let surface = biome_gen.terrain_height(wx, wz).max(1);
                        let s = w.get_block(wx, surface, wz);
                        let below = w.get_block(wx, surface - 1, wz);
                        if s == crate::block::AIR && below == crate::block::AIR {
                            void_surface += 1;
                            if examples.len() < 6 {
                                examples.push((wx, wz, surface));
                            }
                        }
                    }
                }
            }
            assert_eq!(
                void_surface, 0,
                "seed {seed}: {void_surface} void-surface cells (skipped chunks); e.g. {examples:?}",
            );
        }
    }

    // ── Spec 48 (Electricity) — block_meta + edit_block ────────────────────────

    #[test]
    fn meta_defaults_to_zero_and_stays_sparse() {
        let mut w = World::new();
        assert_eq!(w.meta_at(3, 4, 5), 0);
        w.set_meta((3, 4, 5), 0b101);
        assert_eq!(w.meta_at(3, 4, 5), 0b101);
        // Writing 0 must remove the entry, not store a zero.
        w.set_meta((3, 4, 5), 0);
        assert_eq!(w.meta_at(3, 4, 5), 0);
        assert!(w.block_meta.is_empty(), "zero meta must not occupy the map");
    }

    #[test]
    fn set_block_drops_out_of_vertical_range_writes_no_phantom_chunks() {
        let mut w = World::new();
        let top = (MAX_CHUNK_Y + 1) * CHUNK_SIZE as i32; // exclusive top (Y 0..=95)
        // In-range writes land at the very bottom and very top of the column.
        w.set_block(4, 0, 4, crate::block::STONE);
        w.set_block(4, top - 1, 4, crate::block::STONE);
        assert_eq!(w.get_block(4, 0, 4), crate::block::STONE);
        assert_eq!(w.get_block(4, top - 1, 4), crate::block::STONE);
        // Out-of-range structure writes (a gen piece running off the top/bottom)
        // are dropped — no block stored, and crucially no phantom chunk born.
        for y in [-1, -1000, top, top + 1, 5000] {
            w.set_block(4, y, 4, crate::block::STONE);
            assert_eq!(
                w.get_block(4, y, 4),
                crate::block::AIR,
                "out-of-range write at y={y} must not stick",
            );
        }
        assert!(
            w.iter_chunks().all(|((_, cy, _), _)| (0..=MAX_CHUNK_Y).contains(&cy)),
            "no chunk may exist outside the playable column 0..=MAX_CHUNK_Y",
        );
    }

    #[test]
    fn wall_connection_mask_and_collision_track_live_neighbours() {
        use crate::block_shape::{self, BlockShape, CONN_E, CONN_W};
        let reg = crate::block::BlockRegistry::new();
        let mut w = World::new();
        // A lone wall: no connections → just the central post box.
        w.set_block(0, 64, 0, crate::block::COBBLESTONE_WALL);
        assert_eq!(w.connection_mask(0, 64, 0, BlockShape::Wall, &reg), 0);
        assert_eq!(w.collision_boxes_at(0, 64, 0, &reg).len(), 1);
        // Drop a solid block to the west and a wall to the east.
        w.set_block(-1, 64, 0, crate::block::STONE);
        w.set_block(1, 64, 0, crate::block::COBBLESTONE_WALL);
        let mask = w.connection_mask(0, 64, 0, BlockShape::Wall, &reg);
        assert_eq!(mask, CONN_W | CONN_E, "arms grow toward both neighbours");
        // Post + two arms now collide.
        assert_eq!(w.collision_boxes_at(0, 64, 0, &reg).len(), 3);
        // Remove the east wall → that arm disappears again, no stored state.
        w.set_block(1, 64, 0, crate::block::AIR);
        assert_eq!(w.connection_mask(0, 64, 0, BlockShape::Wall, &reg), CONN_W);
        assert!(block_shape::is_connecting(BlockShape::Wall));
    }

    #[test]
    fn edit_block_sets_block_meta_change_and_notifies_neighbours() {
        let mut w = World::new();
        let mut out = Vec::new();
        w.edit_block((10, 20, 30), crate::block::STONE, 0b011, &mut out);
        // Block + meta are written.
        assert_eq!(w.get_block(10, 20, 30), crate::block::STONE);
        assert_eq!(w.meta_at(10, 20, 30), 0b011);
        // One BlockChange queued, carrying the meta byte.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].new_block, crate::block::STONE);
        assert_eq!(out[0].meta, 0b011);
        // The six cardinal neighbours are enqueued for re-evaluation.
        assert_eq!(w.scheduler.take_pending(100).len(), 6);
    }

    // ── time_lock / effective_world_time ──────────────────────────────────────

    /// `"cycle"` (the default) passes the raw tick through unchanged — two
    /// different raw values produce two different effective values, confirming
    /// the day/night cycle is live.
    #[test]
    fn effective_world_time_cycle_passes_raw_through() {
        let w = World::new(); // default time_lock = "cycle"
        assert_eq!(w.effective_world_time(0),     0);
        assert_eq!(w.effective_world_time(12000),  12000);
        assert_eq!(w.effective_world_time(18000),  18000);
        // Two distinct ticks stay distinct — the cycle is live.
        assert_ne!(w.effective_world_time(100), w.effective_world_time(5000));
    }

    /// `"day"` always returns noon (12000), regardless of the raw tick.
    #[test]
    fn effective_world_time_day_always_returns_noon() {
        let mut w = World::new();
        w.time_lock = "day".to_string();
        assert_eq!(w.effective_world_time(0),     12000, "midnight raw → noon");
        assert_eq!(w.effective_world_time(6000),  12000, "sunrise raw → noon");
        assert_eq!(w.effective_world_time(12000), 12000, "noon raw → noon");
        assert_eq!(w.effective_world_time(20000), 12000, "night raw → noon");
    }

    /// `"night"` always returns midnight (0), regardless of the raw tick.
    #[test]
    fn effective_world_time_night_always_returns_midnight() {
        let mut w = World::new();
        w.time_lock = "night".to_string();
        assert_eq!(w.effective_world_time(0),     0, "midnight raw → midnight");
        assert_eq!(w.effective_world_time(12000), 0, "noon raw → midnight");
        assert_eq!(w.effective_world_time(18000), 0, "sunset raw → midnight");
    }

    /// Locked worlds are visually stable: the same raw tick always yields
    /// the same effective value (locked worlds have no visible cycle).
    #[test]
    fn effective_world_time_locked_worlds_are_time_stable() {
        let ticks = [0u32, 1000, 6000, 12000, 20000, 23999];

        let mut day_world = World::new();
        day_world.time_lock = "day".to_string();
        let day_vals: Vec<u32> = ticks.iter().map(|&t| day_world.effective_world_time(t)).collect();
        assert!(day_vals.windows(2).all(|w| w[0] == w[1]),
            "day-lock: not all effective times equal: {day_vals:?}");

        let mut night_world = World::new();
        night_world.time_lock = "night".to_string();
        let night_vals: Vec<u32> = ticks.iter().map(|&t| night_world.effective_world_time(t)).collect();
        assert!(night_vals.windows(2).all(|w| w[0] == w[1]),
            "night-lock: not all effective times equal: {night_vals:?}");
    }

    /// Verify that `compute_sun` called with the locked effective times actually
    /// produces the expected lighting: noon = high brightness, midnight = low.
    #[test]
    fn effective_world_time_drives_correct_brightness() {
        let mut day_world = World::new();
        day_world.time_lock = "day".to_string();
        let (_, day_brightness) = crate::camera::compute_sun(day_world.effective_world_time(0));
        assert!(day_brightness > 0.9, "day-lock should be near peak brightness; got {day_brightness}");

        let mut night_world = World::new();
        night_world.time_lock = "night".to_string();
        let (_, night_brightness) = crate::camera::compute_sun(night_world.effective_world_time(12000));
        assert!(night_brightness < 0.3, "night-lock should be below night threshold; got {night_brightness}");
    }

    #[test]
    fn workshop_preset_generates_only_a_flat_floor() {
        // Spec 40 (The Workshop) — the void preset: a Workshop column is a single
        // flat floor at WORKSHOP_FLOOR_Y — a light SAND surface over an
        // unbreakable BEDROCK base — everything else air. No terrain/trees/villages.
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let mut w = World::new();
        w.is_workshop = true;
        // biome_gen is unused on the workshop path, but the signature needs one.
        let biome_gen = crate::biome::BiomeGenerator::new(1);
        w.generate_column(0, 0, &biome_gen);

        // Exactly one chunk in the column (the one holding the floor layer).
        let chunks_in_col: Vec<_> = w.chunk_positions().filter(|(cx, _, cz)| *cx == 0 && *cz == 0).collect();
        assert_eq!(chunks_in_col.len(), 1, "void column has a single floor chunk");

        // The visible floor is a light SAND surface over an unbreakable BEDROCK
        // base, across the chunk footprint...
        for lx in 0..CHUNK_SIZE as i32 {
            for lz in 0..CHUNK_SIZE as i32 {
                assert_eq!(w.get_block(lx, floor_y, lz), block::SAND, "floor surface is sand");
                assert_eq!(w.get_block(lx, floor_y - 1, lz), block::BEDROCK, "floor base is unbreakable bedrock");
                // ...and the layer above the floor is open air (somewhere to stand).
                assert_eq!(w.get_block(lx, floor_y + 1, lz), block::AIR, "above floor is air");
            }
        }
        // No terrain below the bedrock base either.
        assert_eq!(w.get_block(3, floor_y - 5, 3), block::AIR, "no terrain under the platform");
    }

    #[test]
    fn clear_wipes_the_room_but_keeps_the_reskin_catalogue() {
        // Spec 40 — the Lobby "Reset Workshop" wipe keeps your skins. The invariant
        // lives here: World::clear() drops the built blocks but must PRESERVE the
        // in-memory override_registry (the reskin catalogue is NOT in world.dat, so a
        // clear that dropped it would lose the skins for the session).
        let mut w = World::new();
        w.is_workshop = true;
        let biome_gen = crate::biome::BiomeGenerator::new(1);
        w.generate_column(0, 0, &biome_gen);
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        assert_eq!(w.get_block(0, floor_y, 0), block::SAND, "room has a floor");
        // Author a reskin into the catalogue.
        w.override_registry.add_block_design(
            block::STONE,
            crate::override_registry::NamedDesign {
                id: 0,
                name: "test".into(),
                faces: Some(crate::override_registry::AuthoredFaces::solid([20, 180, 90, 255])),
                micro_model: None,
                author_npub: String::new(),
                derivation_chain: vec![],
            },
            0,
        );
        assert!(!w.override_registry.is_empty(), "catalogue has a reskin");

        w.clear();

        // Room gone...
        assert_eq!(w.get_block(0, floor_y, 0), block::AIR, "clear wipes built blocks");
        // ...catalogue kept.
        assert!(!w.override_registry.is_empty(), "clear KEEPS the reskin catalogue");
    }

    #[test]
    fn world_tracks_lifetime_work_and_clock() {
        // Goal 1 / Spec 2 §9.1: the per-world proof-of-play stats live on World
        // (runtime) and flush to WorldMeta on save. add_work tallies block work;
        // tick_world_clock advances the active world-clock. Both start at zero.
        let mut w = World::new();
        assert_eq!(w.total_work, 0);
        assert_eq!(w.total_ticks, 0);
        w.tick_world_clock();
        w.tick_world_clock();
        assert_eq!(w.total_ticks, 2);
        w.add_work(crate::crafting::block_work(block::STONE)); // 50
        w.add_work(crate::crafting::block_work(block::OAK_LEAVES)); // 1
        assert_eq!(w.total_work, 51);
    }

    #[test]
    fn place_player_block_marks_placed() {
        // Spec 06 §2.2 — placing through the player path both sets the block
        // and flags the voxel so re-mining earns no work.
        let mut w = World::new();
        w.place_player_block(2, 3, 4, block::STONE);
        assert_eq!(w.get_block(2, 3, 4), block::STONE);
        assert!(w.is_placed(2, 3, 4), "player placement must flag the voxel");
    }

    #[test]
    fn set_block_does_not_mark_placed() {
        // Natural / world-gen / falling-block writes go through set_block and
        // must NEVER flag placed — otherwise natural blocks would earn no work.
        let mut w = World::new();
        w.set_block(2, 3, 4, block::STONE);
        assert!(!w.is_placed(2, 3, 4), "set_block must leave the voxel natural");
    }

    #[test]
    fn is_placed_false_for_unloaded_chunk() {
        let w = World::new();
        assert!(!w.is_placed(1000, 5, -2000));
    }

    #[test]
    fn set_placed_false_clears_the_flag() {
        let mut w = World::new();
        w.place_player_block(2, 3, 4, block::STONE);
        w.set_placed(2, 3, 4, false);
        assert!(!w.is_placed(2, 3, 4), "breaking clears the placed flag");
    }

    #[test]
    fn set_block_preserves_an_existing_placed_flag() {
        // Spec 06 §2.2 invariant the growth tick + state transitions rely on:
        // changing a block in place via set_block (crop stage advance,
        // fertiliser, campfire lit<->unlit) must NOT clear the placed flag —
        // else an in-flight player-sown crop would silently lose its
        // anti-farming protection as it grows.
        let mut w = World::new();
        w.place_player_block(0, 1, 0, block::WHEAT_STAGE_0);
        assert!(w.is_placed(0, 1, 0));
        w.set_block(0, 1, 0, block::WHEAT_STAGE_1); // a growth stage change
        assert!(
            w.is_placed(0, 1, 0),
            "set_block must preserve the placed flag on an in-place change"
        );
    }

    #[test]
    fn placed_flag_persists_across_chunk_serialization() {
        // The mask travels with the chunk blob, so a save/reload round-trip
        // keeps player-placed voxels excluded from work.
        let mut w = World::new();
        w.place_player_block(5, 6, 7, block::OAK_PLANKS);
        let cx = 5i32.div_euclid(16);
        let cy = 6i32.div_euclid(16);
        let cz = 7i32.div_euclid(16);
        let bytes = w.chunks.get(&(cx, cy, cz)).unwrap().as_bytes();
        let restored = crate::chunk::Chunk::from_bytes(&bytes).unwrap();
        assert!(restored.is_placed(5, 6, 7), "placed bit must survive serialization");
    }

    #[test]
    fn face_attachment_set_get_remove() {
        // Owner-inbox #1/2/3 — set a wallpaper attachment on a face, read it
        // back, remove one face and confirm the others survive.
        let mut w = World::new();
        let pos = (3, 4, 5);
        assert!(w.face_attachment_at(pos, 2).is_none());
        w.set_face_attachment(pos, 2, FaceAttachment::Wallpaper(block::WALLPAPER_RED));
        w.set_face_attachment(pos, 4, FaceAttachment::Wallpaper(block::WALLPAPER_BLUE));
        assert!(matches!(w.face_attachment_at(pos, 2), Some(FaceAttachment::Wallpaper(b)) if *b == block::WALLPAPER_RED));
        assert!(matches!(w.face_attachment_at(pos, 4), Some(FaceAttachment::Wallpaper(b)) if *b == block::WALLPAPER_BLUE));
        let removed = w.remove_face_attachment(pos, 2);
        assert!(matches!(removed, Some(FaceAttachment::Wallpaper(b)) if b == block::WALLPAPER_RED));
        assert!(w.face_attachment_at(pos, 2).is_none());
        assert!(matches!(w.face_attachment_at(pos, 4), Some(FaceAttachment::Wallpaper(b)) if *b == block::WALLPAPER_BLUE));
    }

    #[test]
    fn remove_face_attachments_at_clears_and_returns() {
        // The destroy action: clearing a block returns every attached face so
        // the caller can drop the recovered items, and the map entry is fully
        // gone afterwards.
        let mut w = World::new();
        let pos = (0, 0, 0);
        w.set_face_attachment(pos, 0, FaceAttachment::Wallpaper(block::WALLPAPER_GREEN));
        w.set_face_attachment(pos, 5, FaceAttachment::Wallpaper(block::WALLPAPER_PINK));
        let removed = w.remove_face_attachments_at(pos);
        assert!(matches!(&removed[0], Some(FaceAttachment::Wallpaper(b)) if *b == block::WALLPAPER_GREEN));
        assert!(matches!(&removed[5], Some(FaceAttachment::Wallpaper(b)) if *b == block::WALLPAPER_PINK));
        assert!(removed[1].is_none());
        assert!(w.face_attachment_at(pos, 0).is_none());
        assert!(w.iter_face_attachments().next().is_none());
    }

    #[test]
    fn face_attachment_store_holds_wallpaper_and_blueprint() {
        use crate::plan::PlanData;
        let mut w = World::new();
        w.set_face_attachment((0, 70, 0), 0, FaceAttachment::Wallpaper(crate::block::WALLPAPER_RED));
        w.set_face_attachment((0, 70, 0), 1, FaceAttachment::Blueprint(Box::new(PlanData::debug_3x3_stone())));
        assert!(matches!(w.face_attachment_at((0, 70, 0), 0), Some(FaceAttachment::Wallpaper(b)) if *b == crate::block::WALLPAPER_RED));
        assert!(matches!(w.face_attachment_at((0, 70, 0), 1), Some(FaceAttachment::Blueprint(_))));
        w.remove_face_attachment((0, 70, 0), 0);
        assert!(w.face_attachment_at((0, 70, 0), 0).is_none());
        assert!(w.face_attachment_at((0, 70, 0), 1).is_some());
    }

    #[test]
    fn clear_empties_chunks_and_block_entities() {
        // Regression: pre-2026-05-19 a "new world" after a quit-to-menu
        // would inherit the previous world's chunks because GameState
        // never reset world state on transition. World::clear is the
        // primitive that backs the menu-side reset; it MUST empty every
        // per-world map.
        let mut world = World::new();
        // Populate: chunks, campfire, drying-rack.
        world.set_block(0, 70, 0, block::STONE);
        world.set_block(5, 75, 5, block::CAMPFIRE);
        let mut cf = crate::campfire::CampfireData::default();
        cf.fuel_ticks = 100;
        world.insert_campfire((5, 75, 5), cf);
        let mut rack = crate::drying_rack::DryingRackData::default();
        rack.try_place_green(crate::drying_rack::LogSpecies::Oak);
        world.drying_racks.insert((10, 75, 10), rack);

        assert!(world.chunk_positions().count() > 0);
        assert_eq!(world.block_entities.len(), 1);
        assert_eq!(world.drying_racks.len(), 1);

        world.clear();

        assert_eq!(world.chunk_positions().count(), 0, "chunks must be empty after clear");
        assert!(world.block_entities.is_empty(), "block_entities must be empty after clear");
        assert!(world.drying_racks.is_empty(), "drying_racks must be empty after clear");
        // Block reads from a cleared world return AIR — same as a fresh
        // World::new(). This is what callers rely on (chunk_stream's
        // initial_load assumes empty world → fresh generation).
        assert_eq!(world.get_block(0, 70, 0), block::AIR);
        assert_eq!(world.get_block(5, 75, 5), block::AIR);
    }

    #[test]
    fn clear_on_already_empty_world_is_a_noop() {
        let mut world = World::new();
        world.clear();
        assert_eq!(world.chunk_positions().count(), 0);
        assert!(world.block_entities.is_empty());
        assert!(world.drying_racks.is_empty());
    }

    #[test]
    fn player_wardrobe_is_separate_from_render_registry_and_survives_clear() {
        let mut w = crate::world::World::new();
        assert!(w.player_wardrobe.is_empty(), "fresh wardrobe is empty");
        w.player_wardrobe.add_block_design(
            crate::block::STONE,
            crate::override_registry::NamedDesign {
                id: 0,
                name: "p".into(),
                faces: Some(crate::override_registry::AuthoredFaces::solid([1, 2, 3, 255])),
                micro_model: None,
                author_npub: String::new(),
                derivation_chain: vec![],
            },
            crate::texture_gen::texture_count(),
        );
        assert!(!w.player_wardrobe.is_empty());
        w.clear();
        assert!(!w.player_wardrobe.is_empty(), "clear KEEPS the player wardrobe");
    }

    #[test]
    fn render_registry_built_from_resolver_reflects_player_wardrobe() {
        // Mirrors what GameState::reapply_overrides does for the render registry:
        // render = from_set(resolve(official, player_wardrobe, world_override, remember)).
        let base = crate::texture_gen::texture_count();
        let mut player = crate::override_registry::OverrideRegistry::new();
        player.add_block_design(
            crate::block::STONE,
            crate::override_registry::NamedDesign {
                id: 0, name: "p".into(),
                faces: Some(crate::override_registry::AuthoredFaces::solid([5, 5, 5, 255])),
                micro_model: None, author_npub: String::new(), derivation_chain: vec![],
            },
            base,
        );
        let render_set = crate::official_overrides::resolve_render_set(
            &crate::override_registry::OverrideSet::default(),
            player.set(),
            None,
            true,
        );
        let render = crate::override_registry::OverrideRegistry::from_set(render_set, base);
        assert!(render.block_face_layer(crate::block::STONE, 0).is_some(), "player design renders");

        let stock_set = crate::official_overrides::resolve_render_set(
            &crate::override_registry::OverrideSet::default(), player.set(), None, false);
        let stock = crate::override_registry::OverrideRegistry::from_set(stock_set, base);
        assert!(stock.block_face_layer(crate::block::STONE, 0).is_none(), "start-at-standard => stock");
    }

    #[test]
    fn vegetation_scatters_grass_and_berries() {
        // Regression: pre-2026-05-27 worldgen placed trees but no ground
        // vegetation, so fresh worlds had bare grass — no tufts, no
        // berries. Generate a 5×5 chunk area and confirm both appear on
        // the grassy surface.
        let bg = BiomeGenerator::new(42);
        let mut world = World::new();
        for cx in 0..5 {
            for cz in 0..5 {
                world.generate_column(cx, cz, &bg);
            }
        }
        let mut grass = 0;
        let mut berries = 0;
        for x in 0..(5 * CHUNK_SIZE as i32) {
            for z in 0..(5 * CHUNK_SIZE as i32) {
                for y in SEA_LEVEL..(SEA_LEVEL + 40) {
                    match world.get_block(x, y, z) {
                        block::TALL_GRASS => grass += 1,
                        block::BERRY_BUSH_3 => berries += 1,
                        _ => {}
                    }
                }
            }
        }
        assert!(grass > 0, "expected tall grass scattered on grassy surfaces");
        assert!(berries > 0, "expected at least one berry bush in a 5×5 chunk area");
    }

    #[test]
    fn flower_density_is_trimmed_for_playtest_lag() {
        // 2026-05-30 playtest: "too many flowers = lots of lag". Each
        // flower is an individual non-solid cross-quad mesh, so raw
        // density drives both visual clutter and frame cost. Guard that
        // flower coverage of grassy surface stays modest — the original
        // 4% roll read as "too many" to Axolittle.
        let bg = BiomeGenerator::new(42);
        let mut world = World::new();
        for cx in 0..6 {
            for cz in 0..6 {
                world.generate_column(cx, cz, &bg);
            }
        }
        let mut flowers = 0u32;
        let mut grass_surface = 0u32;
        for x in 0..(6 * CHUNK_SIZE as i32) {
            for z in 0..(6 * CHUNK_SIZE as i32) {
                let s = bg.terrain_height(x, z);
                if s > SEA_LEVEL && world.get_block(x, s, z) == block::GRASS {
                    grass_surface += 1;
                    if matches!(
                        world.get_block(x, s + 1, z),
                        block::CORNFLOWER | block::FIELD_POPPY | block::BUTTERCUP
                    ) {
                        flowers += 1;
                    }
                }
            }
        }
        assert!(grass_surface > 0, "no grassy surface generated");
        assert!(flowers > 0, "flowers should still appear, just fewer");
        let pct = flowers as f32 / grass_surface as f32;
        assert!(
            pct <= 0.03,
            "flower density {pct:.3} of grassy surface is too high (>3%) — playtest lag regression",
        );
    }

    #[test]
    fn vegetation_sits_on_top_of_a_solid_surface() {
        // Every plant must rest on the surface block, never float. Spot
        // check: wherever a tall-grass/berry block exists, the block
        // directly below it is non-air.
        let bg = BiomeGenerator::new(7);
        let mut world = World::new();
        for cx in 0..3 {
            for cz in 0..3 {
                world.generate_column(cx, cz, &bg);
            }
        }
        for x in 0..(3 * CHUNK_SIZE as i32) {
            for z in 0..(3 * CHUNK_SIZE as i32) {
                for y in (SEA_LEVEL + 1)..(SEA_LEVEL + 40) {
                    let b = world.get_block(x, y, z);
                    if matches!(b, block::TALL_GRASS | block::BERRY_BUSH_3 | block::PAPYRUS_STAGE_3) {
                        assert_ne!(
                            world.get_block(x, y - 1, z),
                            AIR,
                            "plant at ({x},{y},{z}) is floating",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn tree_eligibility_rejects_below_sea_level() {
        // 2026-05-30 playtest "trees spawning in water". Scan a window wide
        // enough to include sub-sea-level terrain at this seed and assert no
        // cell whose surface is strictly *below* the waterline is ever
        // tree-eligible. Surface *at* SEA_LEVEL is dry waterline land and is
        // allowed (it carries no water above it).
        let bg = BiomeGenerator::new(42);
        let mut submerged_cells = 0;
        for wx in -200..200 {
            for wz in -200..200 {
                if bg.terrain_height(wx, wz) < SEA_LEVEL {
                    submerged_cells += 1;
                    assert!(
                        tree_at_column_cell(wx, wz, &bg).is_none(),
                        "tree eligible at ({wx},{wz}) where surface < SEA_LEVEL",
                    );
                }
            }
        }
        assert!(submerged_cells > 0, "test window contained no water — not exercising the guard");
    }

    #[test]
    fn jungle_is_mostly_dry_land() {
        // 2026-05-30: jungles were generating ~56% below the waterline (deep
        // lake basins), so they read as drowned and treeless. After the +5
        // baseline lift the bulk of jungle should be dry land (surface at or
        // above the waterline) so its Rubber/Jungle trees actually grow,
        // while deeper dips remain real lakes.
        let bg = BiomeGenerator::new(42);
        let mut jungle = 0u32;
        let mut dry = 0u32;
        for wx in (-1200..1200).step_by(7) {
            for wz in (-1200..1200).step_by(7) {
                if bg.biome_at(wx, wz) == Biome::Jungle {
                    jungle += 1;
                    if bg.terrain_height(wx, wz) >= SEA_LEVEL {
                        dry += 1;
                    }
                }
            }
        }
        assert!(jungle > 200, "need a meaningful jungle sample, got {jungle}");
        let frac = dry as f32 / jungle as f32;
        assert!(frac > 0.6, "jungle should be mostly dry land, got {frac:.2}");
    }

    #[test]
    fn no_tree_logs_at_or_below_the_waterline() {
        // End-to-end form of the same bug: generate a region and confirm
        // no tree-log block exists at or below SEA_LEVEL.
        let bg = BiomeGenerator::new(42);
        let mut world = World::new();
        for cx in -2..3 {
            for cz in -2..3 {
                world.generate_column(cx, cz, &bg);
            }
        }
        for x in (-2 * CHUNK_SIZE as i32)..(3 * CHUNK_SIZE as i32) {
            for z in (-2 * CHUNK_SIZE as i32)..(3 * CHUNK_SIZE as i32) {
                for y in 0..=SEA_LEVEL {
                    assert!(
                        !block::is_any_log_block(world.get_block(x, y, z)),
                        "tree log at ({x},{y},{z}) is at/below SEA_LEVEL ({SEA_LEVEL})",
                    );
                }
            }
        }
    }

    #[test]
    fn cross_chunk_tree_canopies_are_whole() {
        // 2026-05-30 playtest "lots of trees without leaves": canopy
        // blocks falling outside a tree's own chunk column were clipped
        // and the neighbour never back-filled them. For every tree rooted
        // in the inner 3×3 chunks of a generated 5×5 area (all neighbours
        // present), every block of its pure shape must be present in the
        // world — no leaf silently dropped at a chunk seam.
        let bg = BiomeGenerator::new(42);
        let cs = CHUNK_SIZE as i32;
        let mut world = World::new();
        for cx in 0..5 {
            for cz in 0..5 {
                world.generate_column(cx, cz, &bg);
            }
        }
        let mut trees_checked = 0;
        for wx in cs..(4 * cs) {
            for wz in cs..(4 * cs) {
                let Some((species, surface)) = tree_at_column_cell(wx, wz, &bg) else {
                    continue;
                };
                trees_checked += 1;
                for tb in crate::tree_shapes::place_tree(species, wx, wz, bg.seed) {
                    let bx = wx + tb.dx;
                    let by = surface + 1 + tb.dy;
                    let bz = wz + tb.dz;
                    assert_ne!(
                        world.get_block(bx, by, bz),
                        AIR,
                        "tree at ({wx},{wz}) {species:?}: shape block missing at ({bx},{by},{bz})",
                    );
                }
            }
        }
        assert!(trees_checked > 0, "expected at least one tree in the inner area");
    }

    #[test]
    fn tree_canopies_never_span_more_than_one_chunk() {
        // place_trees only consults *immediate* neighbour columns, so the
        // neighbour-overflow scheme is only complete if no canopy reaches
        // further than one chunk horizontally. Guard that invariant.
        let cs = CHUNK_SIZE as i32;
        for species in [
            crate::block::WoodSpecies::Oak,
            crate::block::WoodSpecies::Birch,
            crate::block::WoodSpecies::Spruce,
            crate::block::WoodSpecies::Jungle,
            crate::block::WoodSpecies::Acacia,
            crate::block::WoodSpecies::DarkOak,
            crate::block::WoodSpecies::Rubber,
        ] {
            for seed in 0..32u32 {
                for tb in crate::tree_shapes::place_tree(species, 0, 0, seed) {
                    assert!(
                        tb.dx.abs() < cs && tb.dz.abs() < cs,
                        "{species:?} canopy extent {},{} >= CHUNK_SIZE",
                        tb.dx, tb.dz,
                    );
                }
            }
        }
    }

    #[test]
    fn tree_threshold_table_excludes_treeless_biomes() {
        assert!(tree_threshold_for_biome(Biome::Desert).is_none());
        assert!(tree_threshold_for_biome(Biome::Ocean).is_none());
        assert!(tree_threshold_for_biome(Biome::Mountains).is_none());
        // Spawn-gate biomes return some positive period.
        for b in [
            Biome::Forest, Biome::Plains, Biome::Jungle,
            Biome::BirchForest, Biome::Taiga, Biome::Savanna,
            Biome::SnowyTundra,
        ] {
            let t = tree_threshold_for_biome(b).unwrap_or_else(|| panic!("{:?} should have a threshold", b));
            assert!(t > 0, "{:?} threshold must be positive, got {}", b, t);
        }
    }

    #[test]
    fn jungle_appears_in_warm_humid_zones() {
        // Spec 32 follow-on: confirm the Jungle wedge in `biome_at` is
        // reachable so Rubber trees actually have somewhere to spawn.
        // Sample a large range; assert ≥1 Jungle column.
        let biome_gen = BiomeGenerator::new(42);
        let mut seen_jungle = false;
        'scan: for x in (-2000..=2000).step_by(50) {
            for z in (-2000..=2000).step_by(50) {
                if biome_gen.biome_at(x, z) == Biome::Jungle {
                    seen_jungle = true;
                    break 'scan;
                }
            }
        }
        assert!(seen_jungle, "Jungle biome should be reachable at seed 42");
    }

    #[test]
    fn tree_hash_is_seeded_and_origin_not_forced() {
        // #8/#9 — tree_hash now folds the seed. Before, it ignored the seed so
        // (a) tree_hash(0,0) == 0 ⇒ a tree was ALWAYS placed at the origin (the
        // canopy the player spawned on), and (b) every world had identical tree
        // positions. Lock in: origin varies by seed, output is deterministic
        // per (coord, seed), and different seeds diverge across a swath.
        assert_ne!(
            tree_hash(0, 0, 42),
            tree_hash(0, 0, 1337),
            "origin tree_hash must depend on the seed"
        );
        assert_eq!(tree_hash(5, 9, 42), tree_hash(5, 9, 42), "must be deterministic");
        assert!(tree_hash(13, 7, 99) >= 0, "result is non-negative for rem_euclid use");
        let differing = (0..64)
            .filter(|&i| tree_hash(i, i * 3, 7) != tree_hash(i, i * 3, 99))
            .count();
        assert!(
            differing > 50,
            "different seeds should change most placements, got {differing}/64"
        );
    }

    #[test]
    fn place_trees_in_jungle_can_produce_rubber_log() {
        // Spec 32 follow-on: scanning every chunk in a range that contains
        // Jungle cells should produce ≥1 RUBBER_LOG block — proves the
        // Rubber-in-Jungle wiring is live end-to-end. Range is broad
        // because the Jungle wedge in `biome_at` (temp > 0.3 && humidity
        // > 0.3) is a minority slice and per-chunk Jungle-cell density
        // is variable; broad sampling makes the test robust to the
        // noise-driven distribution.
        //
        // The chunk centre must also sit ABOVE the waterline: jungle
        // terrain (`SEA_LEVEL + scale_lake(14.0)`) dips below sea level in
        // a majority of cells, and since 2026-05-30 trees no longer root
        // in water. Before that fix this loop happened to find flooded
        // jungle and the test "passed" only on the above-water tips of
        // submerged trees — i.e. it was asserting the very trees-in-water
        // bug we fixed. Require above-water jungle so it tests the real
        // Rubber wiring rather than a worldgen artefact.
        let biome_gen = BiomeGenerator::new(42);
        let cs = CHUNK_SIZE as i32;

        // Find chunks whose centre classifies as above-water Jungle.
        let mut jungle_chunks: Vec<(i32, i32)> = Vec::new();
        for cx in -100..=100 {
            for cz in -100..=100 {
                let centre_x = cx * cs + cs / 2;
                let centre_z = cz * cs + cs / 2;
                if biome_gen.biome_at(centre_x, centre_z) == Biome::Jungle
                    && biome_gen.terrain_height(centre_x, centre_z) > SEA_LEVEL
                {
                    jungle_chunks.push((cx, cz));
                    if jungle_chunks.len() >= 64 { break; }
                }
            }
            if jungle_chunks.len() >= 64 { break; }
        }
        assert!(!jungle_chunks.is_empty(), "expected at least one above-water Jungle chunk in scan range");

        let mut world = World::new();
        let mut rubber_logs = 0;
        for (cx, cz) in &jungle_chunks {
            world.generate_column(*cx, *cz, &biome_gen);
            for lx in 0..cs {
                for lz in 0..cs {
                    let wx = cx * cs + lx;
                    let wz = cz * cs + lz;
                    for y in 60..130 {
                        if world.get_block(wx, y, wz) == block::RUBBER_LOG {
                            rubber_logs += 1;
                        }
                    }
                }
            }
        }
        assert!(
            rubber_logs > 0,
            "expected ≥1 RUBBER_LOG across {} Jungle chunks; got 0",
            jungle_chunks.len(),
        );
    }

    #[test]
    fn species_pick_distributes_uniformly_over_pool() {
        // Regression: an earlier implementation used the same hash for
        // both the spawn gate and the species pick, which collapsed
        // the pick to index 0 whenever the threshold shared a factor
        // with the pool size. The current implementation derives the
        // species pick from a separate hash so a 2-species pool sees
        // both species across a sample range.
        let mut counts = [0usize; 2];
        for wx in 0..1000 {
            for wz in 0..2 {
                let idx = species_pick_for(wx, wz, 42, 2);
                counts[idx] += 1;
            }
        }
        // Each bucket should have a non-trivial share — within ±30 %
        // of the perfectly-uniform 1000 hits.
        assert!(counts[0] > 700 && counts[0] < 1300,
            "species 0 share lopsided: {:?}", counts);
        assert!(counts[1] > 700 && counts[1] < 1300,
            "species 1 share lopsided: {:?}", counts);
    }

    #[test]
    fn species_pick_varies_with_seed() {
        // Two different seeds at the same (wx, wz) should diverge
        // somewhere — otherwise every world has identical Rubber/Jungle
        // patterns.
        let mut differ = 0;
        for wx in 0..200 {
            for wz in 0..50 {
                if species_pick_for(wx, wz, 1, 2) != species_pick_for(wx, wz, 2, 2) {
                    differ += 1;
                }
            }
        }
        assert!(differ > 1000, "expected the species pick to vary with seed, got {differ} diffs");
    }

    // ─── Flat-world generation tests (Task B2) ───────────────────────────────

    /// Helper: build a flat world, generate column (0,0), return the world.
    fn flat_world(ground: &str, water_depth: u8) -> World {
        let mut w = World::new();
        w.world_type = "flat".to_string();
        w.ground = ground.to_string();
        w.water_depth = water_depth;
        let biome_gen = crate::biome::BiomeGenerator::new(1); // unused on flat path
        w.generate_column(0, 0, &biome_gen);
        w
    }

    #[test]
    fn flat_world_grass_surface() {
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("grass", 0);
        // Exactly one chunk (the one containing the floor).
        let chunks: Vec<_> = w.chunk_positions().filter(|(cx,_,cz)| *cx==0 && *cz==0).collect();
        assert_eq!(chunks.len(), 1, "flat column has exactly one chunk");
        assert_eq!(w.get_block(0, floor_y,     0), block::GRASS,   "grass surface at floor_y");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::BEDROCK, "bedrock below surface");
        assert_eq!(w.get_block(0, floor_y + 1, 0), block::AIR,     "air above floor");
        assert_eq!(w.get_block(0, floor_y - 5, 0), block::AIR,     "no terrain below bedrock");
    }

    #[test]
    fn flat_world_sand_surface() {
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("sand", 0);
        assert_eq!(w.get_block(0, floor_y,     0), block::SAND,    "sand surface at floor_y");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::BEDROCK, "bedrock below surface");
        assert_eq!(w.get_block(0, floor_y + 1, 0), block::AIR,     "air above floor");
    }

    #[test]
    fn flat_world_stone_surface() {
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("stone", 0);
        assert_eq!(w.get_block(0, floor_y,     0), block::STONE,   "stone surface at floor_y");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::BEDROCK, "bedrock below surface");
        assert_eq!(w.get_block(0, floor_y + 1, 0), block::AIR,     "air above floor");
    }

    #[test]
    fn flat_world_dirt_surface() {
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("dirt", 0);
        assert_eq!(w.get_block(0, floor_y,     0), block::DIRT,    "dirt surface at floor_y");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::BEDROCK, "bedrock below surface");
        assert_eq!(w.get_block(0, floor_y + 1, 0), block::AIR,     "air above floor");
    }

    #[test]
    fn flat_world_snow_surface() {
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("snow", 0);
        assert_eq!(w.get_block(0, floor_y,     0), block::SNOW,    "snow surface at floor_y");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::BEDROCK, "bedrock below surface");
        assert_eq!(w.get_block(0, floor_y + 1, 0), block::AIR,     "air above floor");
    }

    #[test]
    fn flat_world_none_surface() {
        // "none" = bedrock IS the visible floor; nothing below it.
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("none", 0);
        assert_eq!(w.get_block(0, floor_y,     0), block::BEDROCK, "bedrock at floor_y for 'none'");
        assert_eq!(w.get_block(0, floor_y + 1, 0), block::AIR,     "air above 'none' floor");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::AIR,     "no block below bare bedrock floor");
    }

    #[test]
    fn flat_world_unknown_ground_defaults_to_grass() {
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("bogus_type", 0);
        assert_eq!(w.get_block(0, floor_y,     0), block::GRASS,   "unknown ground falls back to grass");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::BEDROCK, "bedrock below fallback grass");
    }

    #[test]
    fn flat_world_water_depth_3() {
        // depth=3 → top water at floor_y, two more below, sand under water, bedrock base.
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("water", 3);
        assert_eq!(w.get_block(0, floor_y,     0), block::WATER,   "top water layer at floor_y");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::WATER,   "second water layer");
        assert_eq!(w.get_block(0, floor_y - 2, 0), block::WATER,   "third water layer");
        assert_eq!(w.get_block(0, floor_y - 3, 0), block::SAND,    "sand below water");
        assert_eq!(w.get_block(0, floor_y - 4, 0), block::BEDROCK, "bedrock below sand");
        assert_eq!(w.get_block(0, floor_y + 1, 0), block::AIR,     "air above water surface");
    }

    #[test]
    fn flat_world_water_depth_clamped_to_1() {
        // depth=0 is clamped to 1 → one WATER layer at floor_y, sand below, bedrock base.
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("water", 0);
        assert_eq!(w.get_block(0, floor_y,     0), block::WATER,   "single water layer at floor_y (depth clamped to 1)");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::SAND,    "sand below single water layer");
        assert_eq!(w.get_block(0, floor_y - 2, 0), block::BEDROCK, "bedrock below sand");
    }

    #[test]
    fn flat_world_water_depth_clamped_to_8() {
        // depth=255 is clamped to 8 → 8 WATER layers down to floor_y-7.
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let w = flat_world("water", 255);
        for i in 0i32..8 {
            assert_eq!(
                w.get_block(0, floor_y - i, 0), block::WATER,
                "water layer at floor_y-{i} (depth clamped to 8)"
            );
        }
        assert_eq!(w.get_block(0, floor_y - 8,  0), block::SAND,    "sand below 8 water layers");
        assert_eq!(w.get_block(0, floor_y - 9,  0), block::BEDROCK, "bedrock below sand");
        assert_eq!(w.get_block(0, floor_y + 1,  0), block::AIR,     "air above water surface");
    }

    #[test]
    fn flat_world_does_not_affect_workshop_preset() {
        // Workshop path must remain unchanged — sand over bedrock at WORKSHOP_FLOOR_Y.
        let floor_y = crate::workshop::WORKSHOP_FLOOR_Y;
        let mut w = World::new();
        // is_workshop=true but world_type deliberately NOT "flat" to
        // confirm the workshop branch runs first and wins.
        w.is_workshop = true;
        w.world_type = "flat".to_string(); // even if flat is also set, workshop wins
        w.ground = "stone".to_string();    // different ground; must be ignored
        let biome_gen = crate::biome::BiomeGenerator::new(1);
        w.generate_column(0, 0, &biome_gen);
        // Workshop's sand-over-bedrock must be intact.
        assert_eq!(w.get_block(0, floor_y,     0), block::SAND,    "workshop floor is still sand, not stone");
        assert_eq!(w.get_block(0, floor_y - 1, 0), block::BEDROCK, "workshop bedrock unchanged");
    }

    #[test]
    fn highest_block_finds_the_topmost_non_air() {
        let mut w = World::new();
        // Stack: stone at y=10, dirt at y=11, grass at y=12 in column (3, 7).
        w.set_block(3, 10, 7, block::STONE);
        w.set_block(3, 11, 7, block::DIRT);
        w.set_block(3, 12, 7, block::GRASS);
        assert_eq!(w.highest_block(3, 7), Some((12, block::GRASS)));
    }

    #[test]
    fn highest_block_is_none_for_an_empty_column() {
        let w = World::new();
        assert_eq!(w.highest_block(0, 0), None);
    }

    // ── Spec 02 §7.5 — evicted-chunk store (chunk unload keeps edits) ──

    fn column_chunks(w: &World, cx: i32, cz: i32) -> usize {
        (0..=MAX_CHUNK_Y).filter(|&cy| w.has_chunk(cx, cy, cz)).count()
    }

    #[test]
    fn worldgen_leaves_persist_false() {
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut w = World::new();
        for cx in -1..=1 {
            for cz in -1..=1 {
                w.generate_column(cx, cz, &bg);
            }
        }
        assert!(
            w.iter_chunks().all(|(_, c)| !c.persist()),
            "world-gen (terrain, trees, structures, neighbour spill) must not mark persist"
        );
    }

    #[test]
    fn light_only_writes_do_not_set_persist() {
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut w = World::new();
        w.generate_column(0, 0, &bg);
        w.set_block_light_at(3, 70, 3, 12);
        w.set_sky_light_at(3, 70, 3, 9);
        // Light into an ungenerated column creates a chunk — still not persist.
        w.set_block_light_at(100, 10, 100, 5);
        let reg = crate::block::BlockRegistry::new();
        crate::lighting::run_initial_pass_for_column(&mut w, 0, 0, &reg);
        assert!(w.iter_chunks().all(|(_, c)| !c.persist()));
    }

    #[test]
    fn block_change_after_generation_sets_persist_same_value_does_not() {
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut w = World::new();
        w.generate_column(0, 0, &bg);
        let b = w.get_block(2, 1, 2);
        w.set_block(2, 1, 2, b); // no-op write
        assert!(!w.get_chunk(0, 0, 0).unwrap().persist());
        w.set_block(2, 1, 2, crate::block::GLASS);
        assert!(w.get_chunk(0, 0, 0).unwrap().persist());
        // The placed bit is saved too, so flipping it counts as an edit.
        w.set_placed(2, 17, 2, true);
        assert!(w.get_chunk(0, 1, 0).unwrap().persist());
    }

    #[test]
    fn pristine_column_unload_is_dropped_not_evicted() {
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut w = World::new();
        w.generate_column(0, 0, &bg);
        assert!(!w.evict_column(0, 0));
        assert_eq!(column_chunks(&w, 0, 0), 0);
        assert!(!w.is_column_evicted(0, 0));
        assert_eq!(w.persistable_chunks().count(), 0, "evicted store must stay empty");
        assert!(!w.restore_column(0, 0));
    }

    #[test]
    fn edited_column_evicts_and_restores_with_its_edit() {
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut w = World::new();
        w.generate_column(0, 0, &bg);
        w.set_block(4, 40, 4, crate::block::GLASS);
        // Dig one whole chunk out (all air). generate_column would REFILL an
        // empty chunk, so it staying empty after restore proves no regen ran.
        for x in 0..16 {
            for y in 16..32 {
                for z in 0..16 {
                    w.set_block(x, y, z, AIR);
                }
            }
        }
        let before = column_chunks(&w, 0, 0);
        assert!(w.evict_column(0, 0), "an edited column must be kept");
        assert_eq!(column_chunks(&w, 0, 0), 0, "evicted chunks leave `chunks`");
        assert!(w.is_column_evicted(0, 0));
        // Evicted chunks are still saved.
        assert_eq!(w.persistable_chunks().count(), before);
        assert_eq!(
            w.persistable_chunk(0, 2, 0).unwrap().get(4, 40 - 32, 4),
            crate::block::GLASS
        );

        assert!(w.restore_column(0, 0));
        assert!(!w.is_column_evicted(0, 0));
        assert_eq!(column_chunks(&w, 0, 0), before, "whole column comes back");
        assert_eq!(w.get_block(4, 40, 4), crate::block::GLASS);
        assert!(w.get_chunk(0, 1, 0).unwrap().is_empty(), "restore must not regenerate");
    }

    #[test]
    fn loaded_chunk_is_persist_and_evicts() {
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut src = World::new();
        src.generate_column(0, 0, &bg);
        let bytes = src.get_chunk(0, 0, 0).unwrap().as_bytes();
        let mut w = World::new();
        w.insert_chunk(0, 0, 0, Chunk::from_bytes(&bytes).unwrap());
        assert!(w.get_chunk(0, 0, 0).unwrap().persist());
        assert!(w.evict_column(0, 0));
        assert!(w.is_column_evicted(0, 0));
    }

    #[test]
    fn evicted_chunk_wins_over_neighbour_spill_and_clear_empties_store() {
        let mut w = World::new();
        w.set_block(1, 1, 1, crate::block::GLASS);
        assert!(w.evict_column(0, 0));
        // A neighbour's world-gen spills a block into the evicted column.
        w.worldgen_depth += 1;
        w.set_block(2, 2, 2, crate::block::STONE);
        w.worldgen_depth -= 1;
        assert_eq!(w.persistable_chunks().count(), 1, "no duplicate positions");
        assert_eq!(w.persistable_chunk(0, 0, 0).unwrap().get(1, 1, 1), crate::block::GLASS);
        assert!(w.restore_column(0, 0));
        assert_eq!(w.get_block(1, 1, 1), crate::block::GLASS);
        assert!(w.evict_column(0, 0));
        w.clear();
        assert!(!w.is_column_evicted(0, 0));
        assert_eq!(w.persistable_chunks().count(), 0);
    }

    // ── Spec 02 §7.5 — read-through / write-through to evicted chunks ──

    fn evicted_world() -> World {
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut w = World::new();
        w.generate_column(0, 0, &bg);
        w.place_player_block(4, 40, 4, crate::block::GLASS);
        assert!(w.evict_column(0, 0));
        w
    }

    #[test]
    fn get_block_and_is_placed_read_through_to_evicted() {
        let w = evicted_world();
        assert_eq!(w.get_block(4, 40, 4), crate::block::GLASS);
        assert!(w.is_placed(4, 40, 4));
        assert_eq!(w.get_block(4, 0, 4), crate::block::BEDROCK);
    }

    #[test]
    fn writes_to_evicted_column_go_through_not_into_a_stray() {
        let mut w = evicted_world();
        w.set_block(5, 40, 5, crate::block::STONE); // existing cy
        w.place_player_block(5, 90, 5, crate::block::GLASS); // sky cy, absent
        w.set_placed(4, 40, 4, false);
        assert_eq!(
            (0..=MAX_CHUNK_Y).filter(|&cy| w.has_chunk(0, cy, 0)).count(),
            0,
            "no stray chunk in `chunks`"
        );
        assert_eq!(w.get_block(5, 40, 5), crate::block::STONE);
        assert!(w.persistable_chunk(0, 5, 0).is_some_and(|c| c.persist()));
        assert!(!w.is_placed(4, 40, 4));
        assert!(w.restore_column(0, 0));
        assert_eq!(w.get_block(5, 40, 5), crate::block::STONE);
        assert_eq!(w.get_block(5, 90, 5), crate::block::GLASS);
        assert!(w.is_placed(5, 90, 5));
    }

    #[test]
    fn light_writes_to_evicted_column_are_dropped() {
        let mut w = evicted_world();
        w.set_block_light_at(4, 40, 4, 9);
        w.set_sky_light_at(4, 90, 4, 9);
        assert!((0..=MAX_CHUNK_Y).all(|cy| !w.has_chunk(0, cy, 0)));
        assert!(w.is_evicted_at(4, 4));
        assert!(!w.is_evicted_at(40, 4));
    }

    #[test]
    fn never_loaded_column_keeps_old_air_read_and_stray_write() {
        let mut w = World::new();
        assert_eq!(w.get_block(100, 10, 100), AIR);
        w.set_block(100, 10, 100, crate::block::STONE);
        assert!(w.has_chunk(6, 0, 6), "no evicted data: behaviour unchanged");
    }

    // ── column-presence cost (B1 review perf follow-up) ──────────────────
    //
    // `is_column_present_at` runs per entity per tick on the client AND the
    // server, and per fluid/fire/sapling spread step. It used to scan chunk
    // cells for a non-air block (~3,800 reads for a flat floor in a chunk's top
    // layer); `Chunk` now maintains a non-air count, so it is O(1) per chunk.

    /// The question `is_column_present_at` answers, computed the slow, obvious
    /// way: one of the column's live chunks holds a non-air cell, by full recount.
    fn column_present_by_full_scan(w: &World, x: i32, z: i32) -> bool {
        let (cx, cz) = (x.div_euclid(CHUNK_SIZE as i32), z.div_euclid(CHUNK_SIZE as i32));
        (0..=MAX_CHUNK_Y)
            .any(|cy| w.chunks.get(&(cx, cy, cz)).is_some_and(|c| c.recount_non_air() > 0))
    }

    /// Every chunk the world holds — live and evicted — carries an exact
    /// non-air count, and the O(1) presence answer agrees with a full scan over
    /// a grid of columns (including ones that were never loaded).
    fn assert_world_counts_exact(w: &World, what: &str) {
        for (key, c) in w.chunks.iter().chain(w.evicted.iter()) {
            assert_eq!(
                c.non_air_count(),
                c.recount_non_air(),
                "{what}: chunk {key:?} count != full recount",
            );
        }
        for cx in -2..=3 {
            for cz in -2..=3 {
                let (x, z) = (cx * CHUNK_SIZE as i32 + 5, cz * CHUNK_SIZE as i32 + 9);
                assert_eq!(
                    w.is_column_present_at(x, z),
                    column_present_by_full_scan(w, x, z),
                    "{what}: presence at ({x}, {z}) disagrees with a full scan",
                );
            }
        }
    }

    /// A flat world with column (0, 0) generated.
    fn flat_world_with_one_column() -> World {
        let bg = crate::biome::BiomeGenerator::new(1);
        let mut w = World::new();
        w.world_type = "flat".to_string();
        w.generate_column(0, 0, &bg);
        w
    }

    #[test]
    fn column_presence_cost_micro_benchmark() {
        // Rough and non-flaky: this only MEASURES and reports (run with
        // `--nocapture`). The deterministic bound — no full scans — is
        // `column_presence_does_no_full_chunk_scans`. Before the maintained count
        // this read ~3,800 cells per present column on a flat world.
        let w = flat_world_with_one_column();
        let t = std::time::Instant::now();
        let mut present = 0usize;
        for i in 0..1_000 {
            // Spread over the column's 16x16 footprint, plus an absent column.
            let (x, z) = (i % 16, (i / 16) % 16);
            present += usize::from(w.is_column_present_at(x, z));
            assert!(!w.is_column_present_at(200 + x, 200 + z));
        }
        let per_call_ns = t.elapsed().as_nanos() / 2_000;
        eprintln!("is_column_present_at: {per_call_ns} ns/call over 2,000 calls (half present, half absent)");
        assert_eq!(present, 1_000, "every footprint cell of the generated flat column is present");
    }

    #[test]
    fn column_presence_does_no_full_chunk_scans() {
        // Deterministic bound on the per-call work: 1,000 calls on a flat world
        // (present and absent columns) never fall back to counting a chunk's
        // cells. The counter bumps only in the test-only full recount.
        let w = flat_world_with_one_column();
        let before = crate::chunk::full_scans_on_this_thread();
        for i in 0..1_000 {
            let (x, z) = (i % 16, (i / 16) % 16);
            assert!(w.is_column_present_at(x, z));
            assert!(!w.is_column_present_at(200 + x, 200 + z));
        }
        assert_eq!(
            crate::chunk::full_scans_on_this_thread(),
            before,
            "is_column_present_at must read the maintained count, not scan cells",
        );
    }

    #[test]
    fn chunk_counts_stay_exact_through_worldgen() {
        // Biome terrain (trees and vegetation spill into the neighbours),
        // a flat world, and the Workshop floor — every bulk writer.
        let bg = crate::biome::BiomeGenerator::new(11);
        let mut normal = World::new();
        normal.generate_column(0, 0, &bg);
        normal.generate_column(1, 0, &bg);
        assert_world_counts_exact(&normal, "biome worldgen");

        let flat = flat_world_with_one_column();
        assert_world_counts_exact(&flat, "flat worldgen");

        let mut workshop = World::new();
        workshop.is_workshop = true;
        workshop.generate_column(0, 0, &bg);
        assert_world_counts_exact(&workshop, "workshop worldgen");
        assert!(workshop.is_column_present_at(3, 3));
    }

    #[test]
    fn chunk_counts_stay_exact_through_random_world_edits() {
        // set_block / place_player_block over generated, empty-phantom and
        // never-loaded columns, with AIR and non-air writes. Schematic pastes and
        // `/we` reach cells only through these setters (`Chunk::blocks` is private).
        let bg = crate::biome::BiomeGenerator::new(5);
        let mut w = World::new();
        w.generate_column(0, 0, &bg);
        let mut rng = 0x1234_5678_9ABC_DEF1u64;
        let mut next = || {
            rng ^= rng >> 12;
            rng ^= rng << 25;
            rng ^= rng >> 27;
            rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        for step in 0..4_000 {
            let r = next();
            let x = (r % 80) as i32 - 24; // spills into never-loaded neighbours
            let z = ((r >> 8) % 80) as i32 - 24;
            let y = ((r >> 16) % 110) as i32;
            let block = if (r >> 28) % 3 == 0 { AIR } else { 1 + ((r >> 32) % 30) as u16 };
            if (r >> 40) % 4 == 0 {
                w.place_player_block(x, y, z, block);
            } else {
                w.set_block(x, y, z, block);
            }
            if step % 1_000 == 999 {
                assert_world_counts_exact(&w, &format!("random edits, step {step}"));
            }
        }
        assert_world_counts_exact(&w, "after random edits");
    }

    #[test]
    fn chunk_counts_stay_exact_through_load_from_bytes_and_decompression() {
        // The wire path a joiner takes (game_loop's chunk packet handler):
        // as_bytes -> LZ4 compress -> decompress -> from_bytes -> insert_chunk.
        let bg = crate::biome::BiomeGenerator::new(7);
        let mut src = World::new();
        src.generate_column(0, 0, &bg);
        src.set_block(4, 40, 4, crate::block::GLASS);
        let mut dst = World::new();
        for cy in 0..=MAX_CHUNK_Y {
            let Some(chunk) = src.get_chunk(0, cy, 0) else { continue };
            let wire = crate::protocol::compress_chunk(&chunk.as_bytes());
            let raw = crate::protocol::decompress_chunk(&wire).expect("valid LZ4");
            let loaded = Chunk::from_bytes(&raw).expect("valid chunk bytes");
            assert_eq!(loaded.non_air_count(), chunk.non_air_count(), "cy {cy}");
            dst.insert_chunk(0, cy, 0, loaded);
        }
        assert_world_counts_exact(&dst, "decompressed + loaded");
        assert!(dst.is_column_present_at(4, 4));
        // An edit on top of a loaded chunk keeps the count exact.
        dst.set_block(4, 40, 4, AIR);
        dst.set_block(5, 41, 5, crate::block::STONE);
        assert_world_counts_exact(&dst, "edit after load");
    }

    #[test]
    fn chunk_counts_and_presence_stay_exact_through_eviction_and_restore() {
        let mut w = evicted_world();
        assert_world_counts_exact(&w, "evicted");
        assert!(!w.is_column_present_at(4, 4), "an evicted column is absent from the sims");
        // Write-through into the evicted store keeps ITS counts exact too.
        w.set_block(5, 40, 5, crate::block::STONE);
        w.set_block(4, 40, 4, AIR);
        w.place_player_block(5, 90, 5, crate::block::GLASS); // an absent cy, created in the store
        assert_world_counts_exact(&w, "write-through to evicted");
        assert!(w.restore_column(0, 0));
        assert_world_counts_exact(&w, "restored");
        assert!(w.is_column_present_at(4, 4), "a restored column is present again");
        // Evict again: drop-vs-keep, then a never-persisted column is dropped.
        assert!(w.evict_column(0, 0));
        assert!(!w.is_column_present_at(4, 4));
        assert_world_counts_exact(&w, "re-evicted");
        w.clear();
        assert!(!w.is_column_present_at(4, 4));
        assert_world_counts_exact(&w, "cleared");
    }
}
