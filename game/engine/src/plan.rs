//! Build Schematics — Plan data structures + capture algorithm.
//!
//! Per Spec 24 / `docs/foundations/2026-05-19-build-schematics-core.md`.
//! This module owns the `PlanData` struct (captured-building blueprint)
//! plus the engine-generic primitives that operate on it: connectivity
//! flood-fill (capture), content-hash (Save-As detection), placement
//! rotation, and the animated-build state machine.
//!
//! UI lives in `plan_ui.rs`; this module is pure-function-testable + has
//! no egui dependency.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::block::{self, BlockId};
use crate::world::World;

/// CC-licence enum on every captured plan. Default in the capture
/// dialog is CC-BY-SA per the vision doc §5.5 (platform philosophy).
///
/// Wire-stable — variants append-only; never renumber. World saves go through
/// bincode (positional, discriminant-indexed — a pure Rust-side rename is a
/// no-op there), but the bundled `assets/registered_plans/*.plan.json` files
/// (and potentially player-shared plan exports) are serde_json, which encodes
/// by variant NAME — so `Ccbysa`/`Ccbynd` (Phase 4b clippy `upper_case_acronyms`
/// fix) keep the original wire strings via `#[serde(rename)]` rather than
/// renaming the 15 bundled JSON files (or anything shared out in the wild).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanLicense {
    AllRightsReserved,
    CC0,
    #[default]
    #[serde(rename = "CCBYSA")]
    Ccbysa,
    #[serde(rename = "CCBYND")]
    Ccbynd,
}

/// Spec 38 (Blueprint / Cyanotype, 2026-05-27) — captured-plan develop
/// state. Fresh captures start `Latent { exposure_ticks: 0 }` and accrue
/// progress while laid out in direct sunlight under open sky; once
/// `exposure_ticks` reaches `DEVELOP_THRESHOLD_TICKS`, the plan flips
/// to `Developed` and renders as blueprint-blue (white-on-blue per the
/// cyanotype design). Carried items pause accumulation — only laid-out
/// LATENT_PRINT blocks tick. Save-compat: legacy plans without this
/// field default to `Developed` (the conservative assumption — they're
/// already paint-blue in the inventory icon today and every pre-Spec-38
/// capture was effectively finished).
///
/// Wire-stable — variants append-only; never renumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevelopState {
    Developed,
    Latent { exposure_ticks: u16 },
}

/// 90 seconds of cumulative direct sunlight, at 20 TPS, per Spec 38
/// §"Develop". A `u16` field caps at 65 535 ticks (~55 min) — plenty
/// of headroom if playtest retunes the threshold upward.
pub const DEVELOP_THRESHOLD_TICKS: u16 = 1800;

impl DevelopState {
    /// Pure helper used by the LATENT_PRINT develop-tick driver: advance
    /// `exposure_ticks` by 1 if `Latent`, flipping to `Developed` once
    /// the threshold is reached. No-op when already `Developed`. Returns
    /// `true` if this tick transitioned `Latent → Developed`, so the
    /// caller can react (emit a particle, update the block icon).
    pub fn advance_sun_tick(&mut self) -> bool {
        match self {
            DevelopState::Developed => false,
            DevelopState::Latent { exposure_ticks } => {
                *exposure_ticks = exposure_ticks.saturating_add(1);
                if *exposure_ticks >= DEVELOP_THRESHOLD_TICKS {
                    *self = DevelopState::Developed;
                    return true;
                }
                false
            }
        }
    }

    pub fn is_developed(&self) -> bool {
        matches!(self, DevelopState::Developed)
    }
}

/// Serde default for `PlanData.develop_state` — pre-Spec-38 saves load
/// as `Developed` (already-finished blueprints, per the `DevelopState`
/// comment).
fn default_develop_state() -> DevelopState { DevelopState::Developed }

/// Spec 38 art-capture (2026-05-28) — what kind of thing this Plan
/// captures. `Building` is the original 3D-flood-fill capture (Spec 24);
/// `Art` is the 2D wall-slice capture that hangs as a Cyanotype Print
/// rather than building. The two share the same `PlanData` shell so
/// every downstream system (develop, plaque, derivation chain, save)
/// works for both — only the placement / render path differs.
///
/// Wire-stable — variants append-only; never renumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanKind {
    Building,
    Art,
}

/// Serde default — pre-Spec-38 captures load as `Building` (the only
/// kind that existed). Matches `develop_state`'s back-compat shape.
fn default_plan_kind() -> PlanKind { PlanKind::Building }

/// Spec 38 multi-stage develop (2026-05-28) — inventory-icon colour
/// for a captured plan, stepping through 4 visible Latent stages
/// before settling on the Developed Prussian blueprint blue.
///
/// The spec calls out a "visibly deepening pale → faint blue → blue →
/// deep blueprint-blue (~4 stages)" progression. We quantise the
/// `exposure_ticks` axis into 4 buckets so each step is a clear visual
/// jump, not a smooth gradient — the player should be able to glance
/// at the hotbar icon and immediately know roughly how far the print
/// has developed.
///
/// Pure function — easy to unit-test and re-use from rendering paths.
pub fn develop_state_color(state: DevelopState) -> [f32; 3] {
    match state {
        DevelopState::Developed => [0.16, 0.32, 0.62],
        DevelopState::Latent { exposure_ticks } => {
            // 4 stages: [0, 25%) / [25, 50%) / [50, 75%) / [75, 100%).
            // saturating cast on the multiply guards against accidental
            // overflow if exposure_ticks ever exceeds the threshold
            // before the develop-tick driver flips it to Developed.
            let denom = DEVELOP_THRESHOLD_TICKS as u32;
            let raw = (exposure_ticks as u32).saturating_mul(4) / denom.max(1);
            match raw {
                0 => [0.78, 0.82, 0.55], // pale yellow-green — sensitised, untouched by sun
                1 => [0.55, 0.65, 0.58], // faint blue-green — starting to develop
                2 => [0.38, 0.50, 0.62], // blue — half-developed, clearly cyanotype-blue
                _ => [0.24, 0.40, 0.62], // deep blue — almost finished, just shy of Developed
            }
        }
    }
}

/// One ancestor link in a derivation chain. Chains are immutable;
/// new entries can only append at capture time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DerivationLink {
    pub author_npub: String,
    pub plan_name: String,
    pub license: PlanLicense,
    pub captured_at: u64,
    pub plan_hash: [u8; 32],
}

/// Truncate an npub (or any string) to its first `max` characters for compact
/// display. Truncates by **characters**, not bytes — a byte slice
/// (`&s[..max]`) panics on a non-char boundary when the string holds multibyte
/// content, which a non-ASCII handle could carry once Phase 4 lands real
/// handles (engine audit 2026-06-04, E: npub byte-slice panic). Callers append
/// their own ellipsis.
pub fn short_npub(npub: &str, max: usize) -> String {
    npub.chars().take(max).collect()
}

/// One captured cell — relative position + block id. Only non-air cells
/// are stored.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapturedCell {
    pub rx: u8,
    pub ry: u8,
    pub rz: u8,
    pub block_id: BlockId,
}

/// A captured-building blueprint. Embedded in `Item::Plan(PlanData)`.
/// `PartialEq` lets `world::FaceAttachment` (which boxes a `PlanData` in its
/// `Blueprint` variant) derive `PartialEq` for test assertions + diffing.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlanData {
    /// Forward-compat byte. Always 1 for v1 captures; bump on any
    /// breaking content change so older clients can detect "I don't
    /// know how to read this plan" rather than silently misinterpret.
    pub version: u8,
    pub name: String,
    /// Author's Nostr pubkey. Empty in v1 (Signet auth not yet
    /// mandatory on the engine side — Spec 1 Phase 4 cutover).
    pub author_npub: String,
    pub license: PlanLicense,
    /// Ordered ancestor chain `[original, v2, ..., self]`. v1 captures
    /// are length 1 with just self; Save-As (Phase 6) prepends parent.
    pub derivation_chain: Vec<DerivationLink>,
    /// `true` if this plan grants Master rights to the holder. v1
    /// captures are always Master; Licence vs Derivative-Master
    /// distinction lives behind Spec 25.
    pub is_master: bool,
    pub width: u8,
    pub depth: u8,
    pub height: u8,
    /// Non-air cells in the captured volume, ordered as written by
    /// the capture flood-fill. Relative coords: 0..width, 0..height,
    /// 0..depth, anchored to the lowest-XZ tile corner at y=base+1.
    pub cells: Vec<CapturedCell>,
    /// Game-mode the plan was authored in. `"survival"` or `"creative"`
    /// (matching `WorldMeta.game_mode`). Surfaced in the Plaque
    /// attribution dialog + Vendor Block listings (Spec 25). NOT used
    /// to gate trade/derivation — tag, don't gate. See spec §"Creative
    /// vs Survival". `#[serde(default)]` so pre-amendment saves
    /// (PROTOCOL_VERSION ≤ 17 captures) load as Survival, the
    /// conservative assumption.
    #[serde(default = "default_authored_in")]
    pub authored_in: String,
    /// Spec 38 (Blueprint / Cyanotype, 2026-05-27) — develop state.
    /// Fresh post-Spec-38 captures land as `Latent { exposure_ticks: 0 }`
    /// and only flip to `Developed` after `DEVELOP_THRESHOLD_TICKS` of
    /// laid-out direct-sunlight exposure. Pre-Spec-38 captures default
    /// to `Developed` (`default_develop_state`) so existing saves load
    /// as finished blueprints — see the `DevelopState` doc-comment.
    #[serde(default = "default_develop_state")]
    pub develop_state: DevelopState,
    /// Spec 38 art-capture (2026-05-28). `Building` = the original 3D
    /// build flood-fill (Spec 24); `Art` = a 2D wall-slice capture
    /// that hangs as a Cyanotype Print. Defaults to `Building` so
    /// pre-art-capture saves keep loading as buildable plans. Set by
    /// the capture path (`capture` → Building, `capture_art` → Art).
    #[serde(default = "default_plan_kind")]
    pub kind: PlanKind,
    /// C3c-3a (protocol v83) — `Some` only on the server's
    /// stand-in for a joiner's Plan ([`PlanData::marker_placeholder`]): the
    /// [`marker`] of the client's Plan, which the server never holds the body
    /// of. `None` on every real Plan. `#[serde(skip)]`, so no saved or wire
    /// encoding of a `PlanData` (or an `Item`) changes: a placeholder that is
    /// ever bincoded (a dedicated server's `WorldSave.players` rows,
    /// `save::serialize_inventory`) comes back WITHOUT its marker, as a
    /// body-less "Plan" — the sidecar lane stores Plans in
    /// `protocol::WireItem::Plan` form instead.
    #[serde(skip)]
    pub marker: Option<[u8; 32]>,
}

fn default_authored_in() -> String { "survival".to_string() }

/// C3c-3a — a Plan's identity on the wire and in the server's copy of a
/// joiner's inventory: SHA-256 of the bincode of the WHOLE `PlanData`, every
/// field included (`develop_state`, `kind`, `derivation_chain`,
/// `authored_in`), so two Plans that behave or read differently never share
/// one (unlike [`content_hash`], which leaves those four out). A marker
/// placeholder ([`PlanData::marker_placeholder`]) answers the marker it
/// stands for, never a hash of its own stub body.
pub fn marker(data: &PlanData) -> [u8; 32] {
    if let Some(m) = data.marker {
        return m;
    }
    let bytes = bincode::serialize(data).expect("PlanData bincode");
    Sha256::digest(&bytes).into()
}

/// Bincode of a [`PlanData`] — `marker` is `#[serde(skip)]`, so this is the
/// same bytes a Plan always saved as.
#[cfg(test)]
fn plan_bytes(data: &PlanData) -> Vec<u8> {
    bincode::serialize(data).expect("PlanData bincode")
}

impl PlanData {
    /// What a joiner holds for a blueprint the host laid on a wall (Phase
    /// B2a chunk push): an empty plan with only the develop state the wall's
    /// texture shows. The plan itself stays the host's — a push carries the
    /// render stub alone (`protocol::PushedAttachment::Blueprint`).
    pub fn render_stub(developed: bool) -> Self {
        PlanData {
            version: 1,
            name: "Blueprint".to_string(),
            author_npub: String::new(),
            license: PlanLicense::default(),
            derivation_chain: Vec::new(),
            is_master: false,
            width: 0,
            depth: 0,
            height: 0,
            cells: Vec::new(),
            authored_in: default_authored_in(),
            develop_state: if developed {
                DevelopState::Developed
            } else {
                DevelopState::Latent { exposure_ticks: 0 }
            },
            kind: PlanKind::Building,
            marker: None,
        }
    }

    /// C3c-3a — the server's stand-in for a joiner's Plan in its copy of the
    /// joiner's inventory: a body-less Plan carrying the client's Plan's
    /// [`marker`] (`protocol::WireItem::Plan`) and its develop state. It
    /// moves under window clicks and digests exactly as the client's Plan
    /// does (a Plan digests content-free, `window::digest_parts`), and an
    /// owed take matches it by marker (`joiner_actions::same_item`). Never in
    /// a client's own window, never spilled into the world.
    pub fn marker_placeholder(marker: [u8; 32], developed: bool) -> Self {
        PlanData { name: "Plan".to_string(), marker: Some(marker), ..PlanData::render_stub(developed) }
    }

    /// C3c-3a — the same Plan as `other`, by [`marker`]: two real Plans are
    /// the same when they are equal (which is the same thing, without
    /// hashing either); a marker placeholder is the Plan whose marker it
    /// carries.
    pub fn same_plan(&self, other: &PlanData) -> bool {
        if self.marker.is_none() && other.marker.is_none() {
            return self == other;
        }
        marker(self) == marker(other)
    }

    /// C3b-1 — what a joiner's mirror of a shared container holds for a
    /// Plan the host put in it (`protocol::item_kind::PLAN`): a body-less
    /// stand-in it can see but not take (`container_window`). A Plan
    /// digests content-free, so it digests like the real one.
    pub fn placeholder() -> Self {
        PlanData { name: "Plan".to_string(), ..PlanData::render_stub(true) }
    }

    /// Stub used by `/give debug_plan` (Phase 14) + tests. 3×3 footprint,
    /// 1 stone block in each cell at y=0.
    pub fn debug_3x3_stone() -> Self {
        let mut cells = Vec::new();
        for x in 0..3u8 {
            for z in 0..3u8 {
                cells.push(CapturedCell { rx: x, ry: 0, rz: z, block_id: block::STONE });
            }
        }
        let mut data = PlanData {
            version: 1,
            name: "Test Plan 3×3".to_string(),
            author_npub: String::new(),
            license: PlanLicense::Ccbysa,
            derivation_chain: Vec::new(),
            is_master: true,
            width: 3,
            depth: 3,
            height: 1,
            cells,
            authored_in: "survival".to_string(),
            // Debug plans bypass the develop loop — they're handed to a
            // player whole via `/give debug_plan`. Stay aligned with the
            // pre-Spec-38 default so existing tests keep their semantics.
            develop_state: DevelopState::Developed,
            // Spec 38 art-capture — debug plans are Building-kind (the
            // legacy 3D capture); art-kind plans only come from the
            // `capture_art` path.
            kind: PlanKind::Building,
            marker: None,
        };
        let hash = content_hash(&data);
        data.derivation_chain.push(DerivationLink {
            author_npub: String::new(),
            plan_name: data.name.clone(),
            license: data.license,
            captured_at: 0,
            plan_hash: hash,
        });
        data
    }

    /// A small hut — the plan the "Follow the Plan" build-along Trial hands the
    /// player (Wind, Copper & Electricity wave §4). 3×3 footprint, two courses
    /// of oak planks with a doorway gap in the front wall and a glass window in
    /// each side wall, capped by a plank roof: 23 cells, so a build-along is a
    /// satisfying few minutes rather than an afternoon. Developed + Building-kind
    /// like every other handed-over plan, so it lays as a guide immediately.
    pub fn small_hut() -> Self {
        let mut cells = Vec::new();
        for ry in 0..3u8 {
            for rx in 0..3u8 {
                for rz in 0..3u8 {
                    let roof = ry == 2;
                    let wall = rx == 0 || rx == 2 || rz == 0 || rz == 2;
                    if !roof && !wall {
                        continue; // the room inside
                    }
                    // Doorway: the middle of the front (rz == 0) wall, both
                    // courses high, so you can actually walk in.
                    if !roof && rx == 1 && rz == 0 {
                        continue;
                    }
                    // A window in the middle of each side wall, upper course.
                    let window = !roof && ry == 1 && rz == 1 && (rx == 0 || rx == 2);
                    let block_id = if window { block::GLASS } else { block::OAK_PLANKS };
                    cells.push(CapturedCell { rx, ry, rz, block_id });
                }
            }
        }
        let mut data = PlanData {
            version: 1,
            name: "Little Hut".to_string(),
            author_npub: String::new(),
            license: PlanLicense::Ccbysa,
            derivation_chain: Vec::new(),
            is_master: true,
            width: 3,
            depth: 3,
            height: 3,
            cells,
            authored_in: "creative".to_string(),
            develop_state: DevelopState::Developed,
            kind: PlanKind::Building,
            marker: None,
        };
        let hash = content_hash(&data);
        data.derivation_chain.push(DerivationLink {
            author_npub: String::new(),
            plan_name: data.name.clone(),
            license: data.license,
            captured_at: 0,
            plan_hash: hash,
        });
        data
    }

    /// #10 — build a finished, importable plan from raw cells + dimensions (the
    /// schematic-import path). Same shape as a capture: Developed, Building-kind,
    /// Master, with a fresh content-hash derivation link. License is conservative
    /// (`AllRightsReserved`) since imported builds carry no original-author grant.
    pub fn from_imported(
        name: String,
        width: u8,
        depth: u8,
        height: u8,
        cells: Vec<CapturedCell>,
    ) -> Self {
        let mut data = PlanData {
            version: 1,
            name,
            author_npub: String::new(),
            license: PlanLicense::AllRightsReserved,
            derivation_chain: Vec::new(),
            is_master: true,
            width,
            depth,
            height,
            cells,
            authored_in: "creative".to_string(),
            develop_state: DevelopState::Developed,
            kind: PlanKind::Building,
            marker: None,
        };
        let hash = content_hash(&data);
        data.derivation_chain.push(DerivationLink {
            author_npub: String::new(),
            plan_name: data.name.clone(),
            license: data.license,
            captured_at: 0,
            plan_hash: hash,
        });
        data
    }

    /// Rubber feature — wipe this Plan's captured cells back to the
    /// debug 3×3 stone footprint. Used by the Eraser tool's right-
    /// click-on-Plan-item path so a player can re-capture into the
    /// same Plan without having to craft a fresh Plan item from
    /// scratch. Preserves the Plan's `name` so the player's chosen
    /// label survives the wipe.
    /// No live caller found — despite the doc above, no Eraser/Plan
    /// interaction site calls this; tested directly instead.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn wipe_captured_cells(&mut self) {
        let preserved_name = std::mem::take(&mut self.name);
        *self = PlanData::debug_3x3_stone();
        if !preserved_name.is_empty() {
            self.name = preserved_name;
        }
    }
}

/// Content-hash for Save-As detection (and the provenance links' `plan_hash`).
/// SHA-256 over the bincode of a copy of the PlanData with four fields
/// zeroed: `derivation_chain` (emptied), `authored_in` (emptied),
/// `develop_state` (forced `Developed`) and `kind` (forced `Building`). So it
/// HASHES `version`, `name`, `author_npub`, `license`, `is_master`, the size
/// (`width`, `depth`, `height`) and `cells` in their stored order (not
/// sorted: the same cells in another order hash differently), and two Plans
/// that differ only in those four zeroed fields share it — a derivative whose
/// cells match its parent is detected, and a Latent plan and the same plan
/// once Developed share a hash. It is NOT an identity for a Plan item: two
/// Plans with one hash can behave differently (a Latent one lays flat, a
/// Developed one hangs). The item's identity is [`marker`] (C3c-3a).
pub fn content_hash(data: &PlanData) -> [u8; 32] {
    // `authored_in` is zeroed alongside derivation_chain so that two
    // structurally identical plans match across modes — a survival
    // player should be able to detect a creative-authored parent and
    // vice-versa. Authorship is recorded in the derivation chain
    // itself once the link is built. Spec 38 — `develop_state` is also
    // zeroed (forced to `Developed`) so a Latent plan and the same
    // plan once Developed share a hash — develop progress is process
    // state, not content.
    let zeroed = PlanData {
        version: data.version,
        name: data.name.clone(),
        author_npub: data.author_npub.clone(),
        license: data.license,
        derivation_chain: Vec::new(),
        is_master: data.is_master,
        width: data.width,
        depth: data.depth,
        height: data.height,
        cells: data.cells.clone(),
        authored_in: String::new(),
        develop_state: DevelopState::Developed,
        // Spec 38 art-capture — zero `kind` alongside derivation_chain
        // + authored_in + develop_state. A Building-kind plan and an
        // Art-kind plan with otherwise identical content share a content
        // hash; the kind is metadata.
        kind: PlanKind::Building,
        marker: None,
    };
    let bytes = bincode::serialize(&zeroed).expect("PlanData bincode");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher.finalize().into()
}

/// Returned by `commit_capture` to the caller. Tells `game_loop` what
/// to broadcast / rebuild and whether the inventory accepted the plan.
#[derive(Clone, Debug)]
pub struct CommitResult {
    /// The PlanData with its derivation chain finalised (parent-prepend
    /// applied if Save-As was selected, plus the new self-entry).
    pub committed_plan: PlanData,
    /// World-space tile positions whose BlueprintBlank attachment was
    /// consumed by the commit. The floor block at each is UNCHANGED — only
    /// the attachment was removed. Caller rebuilds the chunks they live in
    /// to re-mesh away the decal. NOT broadcast as a BlockChange (the block
    /// didn't change, and attachments don't sync as block changes — same
    /// posture as wallpaper).
    pub tile_positions: Vec<(i32, i32, i32)>,
    /// `true` if the new `Item::Plan(_)` fit into the player's
    /// inventory. `false` = full inventory; caller shows a toast and
    /// the tiles are already-consumed (paper is locked in regardless —
    /// see Spec 24 §"Paper economy"). The plan is dropped.
    pub plan_inserted: bool,
}

/// Spec 24 Phase 5 — commit a captured plan after the Capture dialog
/// returns Confirmed. Pure-ish: mutates `world` (removes the
/// BlueprintBlank attachment from each tile, leaving the floor intact) +
/// `inventory` (inserts the Item::Plan). Returns the affected tile
/// positions so the caller can broadcast + rebuild chunks.
///
/// `parent_plan` is `Some(_)` only when the user has the "Mark as
/// derivative" checkbox ticked AND a parent was detected at capture
/// time — caller looks the plan up from inventory at the
/// `pending.parent_match` hotbar slot.
pub fn commit_capture(
    world: &mut World,
    inventory: &mut crate::inventory::Inventory,
    pending: &PendingCapture,
    parent_plan: Option<&PlanData>,
) -> CommitResult {
    let mut data = pending.candidate.data.clone();

    // Build derivation chain.
    if pending.mark_as_derivative
        && let Some(parent) = parent_plan {
            // Prepend the parent's chain so the new plan inherits the
            // full ancestry. The new self-entry gets pushed below.
            data.derivation_chain = parent.derivation_chain.clone();
        }

    // The content hash ignores the derivation_chain (it zeroes it, with
    // authored_in, develop_state and kind; `content_hash`), so setting the
    // parent's chain first changes nothing about it.
    let hash = content_hash(&data);
    data.derivation_chain.push(DerivationLink {
        author_npub: String::new(),
        plan_name: data.name.clone(),
        license: data.license,
        captured_at: 0,
        plan_hash: hash,
    });

    // Consume the BlueprintBlank top-attachment from each tile, leaving
    // the player's floor block intact. No refund (paper locked in per
    // the 2026-05-20 amendment + Spec 24 §"Paper economy").
    for &(x, y, z) in &pending.candidate.tile_positions {
        world.remove_face_attachment((x, y, z), crate::mesh::Face::Top.index());
    }

    // Insert plan into inventory.
    let stack = crate::item::ItemStack {
        item: crate::item::Item::Plan(data.clone()),
        count: 1,
    };
    let inserted = inventory.add_item(stack).is_none();

    CommitResult {
        committed_plan: data,
        tile_positions: pending.candidate.tile_positions.clone(),
        plan_inserted: inserted,
    }
}

// ─── Capture flood-fill (Phase 4) ─────────────────────────────────────

/// Outcome of a capture attempt.
#[derive(Clone, Debug)]
pub struct PlanCaptureCandidate {
    pub data: PlanData,
    /// World-space positions of every blueprint tile (floor block carrying a
    /// BlueprintBlank top-attachment) consumed by this capture. On Confirm,
    /// the dialog removes the BlueprintBlank attachment from each tile,
    /// leaving the floor block intact (NOT refunded — paper locked into
    /// the plan per Spec 24 §"Paper economy"). If the dialog is Cancelled
    /// the attachments are left in place and the player can erase them if
    /// they want to recover the unused tiles.
    pub tile_positions: Vec<(i32, i32, i32)>,
}

/// Spec 24 Phase 5 — transient state for the Capture egui dialog. Lives
/// on `PlayerSlot.pending_capture: Option<PendingCapture>` between the
/// right-click that triggers capture and the player's Confirm/Cancel on
/// the modal. Cleared on either outcome.
#[derive(Clone, Debug)]
pub struct PendingCapture {
    pub candidate: PlanCaptureCandidate,
    /// Save-As parent detected from the player's inventory at capture
    /// time. `Some((hotbar_slot, kind))` means a Master plan in the
    /// player's inventory is a content-hash or 50%-block match.
    pub parent_match: Option<(usize, ParentMatchKind)>,
    /// Whether the player has the "Mark as derivative of «name»"
    /// checkbox checked. Defaults to `true` whenever `parent_match` is
    /// `Some`; ignored when no parent matched.
    pub mark_as_derivative: bool,
}

/// Why a capture refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRefusal {
    /// Tile cells form 2+ disconnected groups.
    Disconnected,
    /// `capture_art` only: the face_normal was vertical (horizontal paper),
    /// so the caller should route to the building `capture` path instead.
    NotFlat,
    /// Footprint > 32×32 OR captured height > 32.
    TooLarge,
    /// No non-air blocks in the build volume.
    EmptyVolume,
}

/// Max footprint dimensions + max captured height. Per vision §3.5
/// "house-scale envelope; megabuilds out of scope".
pub const MAX_FOOTPRINT: i32 = 32;
pub const MAX_HEIGHT: i32 = 32;

/// True when the floor block at `pos` carries a `BlueprintBlank` attachment
/// on its TOP face — the new-model marker for a laid blueprint tile (replaces
/// the old full `BLUEPRINT_PAPER` cube).
fn has_blank_paper(world: &World, pos: (i32, i32, i32)) -> bool {
    matches!(
        world.face_attachment_at(pos, crate::mesh::Face::Top.index()),
        Some(crate::world::FaceAttachment::BlueprintBlank)
    )
}

/// 4-connected flood-fill collecting every floor block carrying a
/// `BlueprintBlank` top-attachment reachable from `start` at the same Y.
/// Returns positions in BFS order (deterministic per start point).
pub fn flood_fill_tiles(world: &World, start: (i32, i32, i32)) -> Vec<(i32, i32, i32)> {
    use std::collections::VecDeque;
    if !has_blank_paper(world, start) {
        return Vec::new();
    }
    let mut visited = std::collections::HashSet::new();
    let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
    let mut result = Vec::new();
    queue.push_back(start);
    visited.insert(start);
    while let Some(pos) = queue.pop_front() {
        result.push(pos);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let nx = pos.0 + dx;
            let nz = pos.2 + dz;
            let npos = (nx, pos.1, nz);
            if visited.contains(&npos) {
                continue;
            }
            if has_blank_paper(world, (nx, pos.1, nz)) {
                visited.insert(npos);
                queue.push_back(npos);
            }
        }
    }
    result
}

/// Connectivity flood: from every block sitting directly on a paper tile,
/// flood through SOLID (non-air) blocks face-to-face (6-connected), staying
/// in the columns above the paper footprint and under MAX_HEIGHT. Returns
/// every reached non-air cell. Air breaks the flood (disconnected ceilings /
/// overhangs excluded).
///
/// # Panics
///
/// Panics if `tiles` is empty. The caller (`capture`) already guards this;
/// the assert is a defensive precondition for other callers.
pub fn capture_connected_volume(world: &World, tiles: &[(i32, i32, i32)]) -> Vec<(i32, i32, i32)> {
    assert!(!tiles.is_empty(), "capture_connected_volume requires at least one tile");
    use std::collections::{HashSet, VecDeque};
    // Apex bugfix: the captured footprint is the BOUNDING BOX of the laid paper,
    // not only the exact paper cells. You lay paper around a build's base, but a
    // house's roof + apex sit over INTERIOR columns that have no paper directly
    // beneath them — restricting the flood to the exact paper cells dropped every
    // roof cell over the open interior (the "only builds ~5 high, no apex" bug).
    // The flood still requires solid, connected blocks, so it won't grab
    // unrelated builds; the bbox just lets it cross the interior to the roof.
    let min_x = tiles.iter().map(|&(x, _, _)| x).min().unwrap();
    let max_x = tiles.iter().map(|&(x, _, _)| x).max().unwrap();
    let min_z = tiles.iter().map(|&(_, _, z)| z).min().unwrap();
    let max_z = tiles.iter().map(|&(_, _, z)| z).max().unwrap();
    let base_y = tiles[0].1;
    let mut visited: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut found = Vec::new();
    let mut q = VecDeque::new();
    for &(x, _, z) in tiles {
        let s = (x, base_y + 1, z);
        if world.get_block(s.0, s.1, s.2) != block::AIR && visited.insert(s) {
            q.push_back(s);
        }
    }
    while let Some((x, y, z)) = q.pop_front() {
        found.push((x, y, z));
        for (dx, dy, dz) in [(-1, 0, 0), (1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, -1), (0, 0, 1)] {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            if ny < base_y + 1 || ny - base_y > MAX_HEIGHT { continue; }
            if nx < min_x || nx > max_x || nz < min_z || nz > max_z { continue; }
            if world.get_block(nx, ny, nz) == block::AIR { continue; }
            if visited.insert((nx, ny, nz)) { q.push_back((nx, ny, nz)); }
        }
    }
    found
}

/// Entry point: attempt to capture a plan starting from a right-clicked
/// floor tile carrying a BlueprintBlank attachment. Returns a candidate
/// (caller commits via dialog) or a refusal kind.
pub fn capture(
    world: &World,
    click_pos: (i32, i32, i32),
    authored_in: &str,
) -> Result<PlanCaptureCandidate, CaptureRefusal> {
    let tiles = flood_fill_tiles(world, click_pos);
    if tiles.is_empty() {
        return Err(CaptureRefusal::Disconnected);
    }
    // Footprint envelope guard.
    let min_x = tiles.iter().map(|p| p.0).min().unwrap();
    let max_x = tiles.iter().map(|p| p.0).max().unwrap();
    let min_z = tiles.iter().map(|p| p.2).min().unwrap();
    let max_z = tiles.iter().map(|p| p.2).max().unwrap();
    let width = max_x - min_x + 1;
    let depth = max_z - min_z + 1;
    if width > MAX_FOOTPRINT || depth > MAX_FOOTPRINT {
        return Err(CaptureRefusal::TooLarge);
    }
    let base_y = tiles[0].1;
    let captured = capture_connected_volume(world, &tiles);
    if captured.is_empty() {
        return Err(CaptureRefusal::EmptyVolume);
    }
    // Compute height envelope.
    let min_y = base_y + 1;
    let max_y = captured.iter().map(|p| p.1).max().unwrap();
    let height = max_y - min_y + 1;
    if height > MAX_HEIGHT {
        return Err(CaptureRefusal::TooLarge);
    }
    // Build relative-coord cells.
    let cells: Vec<CapturedCell> = captured
        .iter()
        .map(|&(x, y, z)| CapturedCell {
            rx: (x - min_x) as u8,
            ry: (y - min_y) as u8,
            rz: (z - min_z) as u8,
            block_id: world.get_block(x, y, z),
        })
        .collect();
    let data = PlanData {
        version: 1,
        name: format!("Plan: {width}×{depth}"),
        author_npub: String::new(),
        license: PlanLicense::Ccbysa,
        derivation_chain: Vec::new(),
        is_master: true,
        width: width as u8,
        depth: depth as u8,
        height: height as u8,
        cells,
        authored_in: authored_in.to_string(),
        // Spec 38 / R3b — creative captures are instantly Developed
        // (usable immediately, no sun needed); survival captures land as
        // Latent and only develop once the player re-lays the resulting
        // Plan back into the world as a LATENT_PRINT block, under open
        // sky, in daytime.
        develop_state: if authored_in == "creative" {
            DevelopState::Developed
        } else {
            DevelopState::Latent { exposure_ticks: 0 }
        },
        // The horizontal `capture` path is the legacy 3D building
        // flood-fill; `capture_art` (Spec 38 R5) is the vertical
        // 2D-wall-slice sibling that produces Art-kind plans.
        kind: PlanKind::Building,
        marker: None,
    };
    Ok(PlanCaptureCandidate { data, tile_positions: tiles })
}

/// Spec 38 art-capture (2026-05-28) — capture a 2D wall slice as an
/// Art-kind Plan. Sibling to [`capture`]: the building variant flood-
/// fills a 3D volume above horizontal paper tiles; the art variant
/// flood-fills a 2D rectangle on a vertical plane anchored at `anchor`
/// with the paper's `face_normal` pointing AWAY from the wall.
///
/// `face_normal` must be horizontal (±X or ±Z); vertical paper
/// placement (face_normal = ±Y) is the building path, not art. This
/// keeps the two capture flows cleanly partitioned by the player's
/// placement gesture: flat-on-floor → building, against-the-wall →
/// art.
///
/// The captured plane is at the wall-block coordinate (anchor itself —
/// the paper "wraps" the wall surface). Cells store relative
/// coordinates with `depth=1` and `ry` mapping the vertical axis;
/// `rx` maps the in-plane horizontal axis (Z when the wall faces ±X,
/// X when the wall faces ±Z).
///
/// 4-connected flood-fill on the wall plane, bounded to
/// `MAX_FOOTPRINT × MAX_HEIGHT`; refuses an empty wall (the player
/// has to build something on the wall before lifting the paper).
pub fn capture_art(
    world: &World,
    anchor: (i32, i32, i32),
    face_normal: [i32; 3],
    authored_in: &str,
) -> Result<PlanCaptureCandidate, CaptureRefusal> {
    // Reject horizontal placement — that's the building path.
    if face_normal[1] != 0 {
        return Err(CaptureRefusal::NotFlat);
    }
    // Determine in-plane axes. The wall is perpendicular to face_normal;
    // its plane is the two axes other than the face_normal's non-zero
    // axis. We always vary Y (vertical) and either X or Z (in-plane
    // horizontal), keeping the third axis fixed at `anchor`.
    let (fixed_axis, horiz_idx): (usize, usize) =
        if face_normal[0] != 0 { (0, 2) } else { (2, 0) };

    // 4-connected flood-fill on the wall plane. Seed = anchor itself
    // (the wall block under the paper). Expand to neighbours that
    // share the fixed axis + carry a non-AIR block.
    use std::collections::{HashSet, VecDeque};
    let mut visited: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
    let mut found: Vec<(i32, i32, i32)> = Vec::new();
    if world.get_block(anchor.0, anchor.1, anchor.2) == block::AIR {
        return Err(CaptureRefusal::EmptyVolume);
    }
    queue.push_back(anchor);
    visited.insert(anchor);
    found.push(anchor);

    // In-plane neighbour offsets — vary horiz_idx + y, leave fixed_axis at 0.
    let offsets: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    while let Some(p) = queue.pop_front() {
        for (dh, dy) in offsets {
            let mut np = p;
            // horiz_idx component
            if horiz_idx == 0 { np.0 += dh; } else { np.2 += dh; }
            np.1 += dy;
            if visited.contains(&np) {
                continue;
            }
            // Same fixed_axis as anchor (we stay on the wall plane).
            if (fixed_axis == 0 && np.0 != anchor.0)
                || (fixed_axis == 2 && np.2 != anchor.2)
            {
                continue;
            }
            visited.insert(np);
            if world.get_block(np.0, np.1, np.2) != block::AIR {
                found.push(np);
                queue.push_back(np);
            }
        }
    }
    if found.is_empty() {
        return Err(CaptureRefusal::EmptyVolume);
    }
    // Compute envelope in the two varying axes.
    let (h_min, h_max, y_min, y_max) = if horiz_idx == 0 {
        (
            found.iter().map(|p| p.0).min().unwrap(),
            found.iter().map(|p| p.0).max().unwrap(),
            found.iter().map(|p| p.1).min().unwrap(),
            found.iter().map(|p| p.1).max().unwrap(),
        )
    } else {
        (
            found.iter().map(|p| p.2).min().unwrap(),
            found.iter().map(|p| p.2).max().unwrap(),
            found.iter().map(|p| p.1).min().unwrap(),
            found.iter().map(|p| p.1).max().unwrap(),
        )
    };
    let width = h_max - h_min + 1;
    let height = y_max - y_min + 1;
    if width > MAX_FOOTPRINT || height > MAX_HEIGHT {
        return Err(CaptureRefusal::TooLarge);
    }
    // Build relative-coord cells. rx = in-plane horizontal, ry =
    // vertical, rz = 0 (single-plane).
    let cells: Vec<CapturedCell> = found
        .iter()
        .map(|&(x, y, z)| {
            let h = if horiz_idx == 0 { x - h_min } else { z - h_min };
            CapturedCell {
                rx: h as u8,
                ry: (y - y_min) as u8,
                rz: 0,
                block_id: world.get_block(x, y, z),
            }
        })
        .collect();
    let data = PlanData {
        version: 1,
        name: format!("Cyanotype: {width}×{height}"),
        author_npub: String::new(),
        license: PlanLicense::Ccbysa,
        derivation_chain: Vec::new(),
        is_master: true,
        width: width as u8,
        depth: 1,
        height: height as u8,
        cells,
        authored_in: authored_in.to_string(),
        develop_state: DevelopState::Latent { exposure_ticks: 0 },
        kind: PlanKind::Art,
        marker: None,
    };
    Ok(PlanCaptureCandidate {
        data,
        tile_positions: vec![anchor],
    })
}

/// Spec 24 Phase 8 — transient placement preview for a player who has
/// clicked Place in the Inspect dialog. Tracks which inventory plan is
/// being placed, the current rotation, and the world-space anchor the
/// ghost wireframe is following.
///
/// Lives on `PlayerSlot.ghost_state`. Cleared on confirm (left-click on
/// a valid placement) or cancel (right-click / Esc / inventory change).
/// Recomputed each frame from the raycast hit so the player can sweep
/// the cursor across the world to scout sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GhostState {
    /// Inventory slot the plan is held in. The plan_data is fetched
    /// from this slot every frame — if the player swaps hotbar mid-
    /// preview, the ghost survives as long as the slot still holds a
    /// plan. If the slot is empty or holds a non-plan item the ghost
    /// is dismissed by the game-loop handler.
    pub plan_hotbar_slot: usize,
    /// 0..4 step counter for 90° clockwise rotations.
    pub rotations: u8,
    /// World-space anchor — `min_corner` of the plan's footprint after
    /// rotation. The plan's bottom layer sits at `anchor.y`; cells fill
    /// XZ outward from `(anchor.x, anchor.z)`.
    pub anchor: (i32, i32, i32),
    /// Task 17 — rate-limit bookkeeping for the blocked-placement toast.
    /// Defaults fresh whenever the ghost is (re-)seeded (e.g. a new Place
    /// or a commission pick-site); see `placement_toast_decision`.
    pub toast_state: GhostToastState,
}

/// Spec 24 Phase 8 — colour-coding bucket for the ghost wireframe.
/// Computed each frame from the current placement check + materials
/// sufficiency. Distinct from `PlacementCheck` because Yellow requires
/// both "placement is valid" AND "materials insufficient", which
/// PlacementCheck alone doesn't express.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhostTint {
    /// Valid placement and (creative OR materials sufficient).
    Green,
    /// Valid placement but survival inventory short.
    Yellow,
    /// Invalid placement (terrain / volume / chunks / player in volume).
    Red,
}

impl GhostTint {
    /// RGB triple used by the line renderer. Alpha is set by the caller.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            GhostTint::Green => [0.2, 0.95, 0.35],
            GhostTint::Yellow => [0.95, 0.85, 0.15],
            GhostTint::Red => [0.95, 0.2, 0.2],
        }
    }
}

/// Pure helper: pick the ghost tint from a placement check + survival
/// material sufficiency. `is_creative` short-circuits the material
/// check (creative builds always have materials).
pub fn ghost_tint(check: PlacementCheck, materials_sufficient: bool, is_creative: bool) -> GhostTint {
    match check {
        PlacementCheck::Valid => {
            if is_creative || materials_sufficient {
                GhostTint::Green
            } else {
                GhostTint::Yellow
            }
        }
        _ => GhostTint::Red,
    }
}

/// 90°-step rotation of `cells` about the centre of the W×D footprint.
/// `rotations ∈ {0, 1, 2, 3}` for 0° / 90° / 180° / 270° clockwise.
pub fn rotate_cells(cells: &[CapturedCell], rotations: u8, width: u8, depth: u8) -> Vec<CapturedCell> {
    let n = rotations % 4;
    if n == 0 {
        return cells.to_vec();
    }
    cells
        .iter()
        .map(|c| {
            let (nx, nz, _new_w, _new_d) = match n {
                1 => (depth - 1 - c.rz, c.rx, depth, width),
                2 => (width - 1 - c.rx, depth - 1 - c.rz, width, depth),
                _ => (c.rz, width - 1 - c.rx, depth, width),
            };
            CapturedCell { rx: nx, ry: c.ry, rz: nz, block_id: c.block_id }
        })
        .collect()
}

/// Save-As detection: scan the player's inventory for a Master plan
/// whose content-hash matches OR whose cells overlap the candidate's
/// at ≥50%. Returns the index of the matching slot + match kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentMatchKind {
    ContentHash,
    BlockMatch50,
}

pub fn detect_parent<'a>(
    candidate: &PlanData,
    inventory_plans: impl IntoIterator<Item = (usize, &'a PlanData)>,
) -> Option<(usize, ParentMatchKind)> {
    let candidate_hash = content_hash(candidate);
    // Build a candidate-cell lookup for the 50% match path.
    let candidate_cells: std::collections::HashSet<(u8, u8, u8, BlockId)> = candidate
        .cells
        .iter()
        .map(|c| (c.rx, c.ry, c.rz, c.block_id))
        .collect();
    let total = candidate.cells.len() as u32;
    let mut best: Option<(usize, ParentMatchKind)> = None;
    for (slot, parent) in inventory_plans {
        if !parent.is_master {
            // Vision §5.3: Licence-tier holders can't Save-As. v1 has
            // is_master=true on every captured plan, so this guard is
            // dormant; it's wired now for Spec 25.
            continue;
        }
        if content_hash(parent) == candidate_hash {
            return Some((slot, ParentMatchKind::ContentHash));
        }
        if total > 0 {
            let matched: u32 = parent
                .cells
                .iter()
                .filter(|c| candidate_cells.contains(&(c.rx, c.ry, c.rz, c.block_id)))
                .count() as u32;
            // Spec §6 threshold: matched_cells * 2 >= total_cells_in_candidate.
            if matched * 2 >= total {
                best = Some((slot, ParentMatchKind::BlockMatch50));
            }
        }
    }
    best
}

/// Order cells for the animated builder: Y-ascending, then X-then-Z
/// within each Y-layer. Floors first, walls next, roof last — reads
/// visually as construction.
pub fn order_cells_for_build(cells: &[CapturedCell]) -> Vec<CapturedCell> {
    let mut out = cells.to_vec();
    out.sort_by_key(|c| (c.ry, c.rx, c.rz));
    out
}

// ─── ConstructionAnchor (Phase 10) ────────────────────────────────────

/// State stored per-active-build in `World::construction_anchors`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConstructionAnchorData {
    pub plan: PlanData,
    /// 0..=3 quarter-turns clockwise applied at placement.
    pub rotations: u8,
    /// World-space lowest-XZ corner of the build (the anchor block
    /// itself sits at this position).
    pub anchor: (i32, i32, i32),
    /// Number of cells already placed in `order_cells_for_build` order.
    pub placed_index: usize,
    /// Materials locked from the player's inventory at build-start.
    /// Consumed as cells are placed. Refunded on cancel. Empty in
    /// creative mode (see `is_creative_build`).
    pub locked_materials: Vec<crate::item::ItemStack>,
    /// Spec 24 §"Creative vs Survival" amendment — if true, the build
    /// places blocks from thin air without consuming from
    /// `locked_materials`. Set at `start_build` time from
    /// `world.is_creative` (cached on GameState). `#[serde(default)]`
    /// → false = survival, the conservative default for legacy saves.
    #[serde(default)]
    pub is_creative_build: bool,
    /// Spec 26 — pace divider for animated builds. 1 = player-driven
    /// (default; places BLOCKS_PER_BUILD_TICK cells every tick). 2 =
    /// NPC commission (one tick on, one tick off). `#[serde(default)]`
    /// with `default_pace_divider` so legacy saves keep the player-rate
    /// pacing they had pre-Spec-26.
    #[serde(default = "default_pace_divider")]
    pub pace_divider: u8,
    /// Spec 26 — engine-tick counter local to this anchor, incremented
    /// once per `tick_build` call. `current_tick % pace_divider == 0`
    /// gates whether this tick places blocks.
    #[serde(default)]
    pub pace_counter: u64,
    /// Spec 26 — Builder NPC credit. `Some` when the build was
    /// commissioned via a Builder villager; the plaque dropped on
    /// completion carries the credit into `ArchitectPlaqueData`.
    /// `None` for player-driven builds.
    #[serde(default)]
    pub builder_credit: Option<crate::builder::BuilderCredit>,
}

fn default_pace_divider() -> u8 { 1 }

/// Plaque attribution stored per-block in `World::architect_plaques`.
/// Holds the plan's derivation chain plus the root plan's `authored_in`
/// game-mode so the dialog can render the badge without re-looking up
/// the original Plan item.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchitectPlaqueData {
    pub chain: Vec<DerivationLink>,
    /// Mode the root plan was authored in. `#[serde(default)]` for the
    /// same reason as `PlanData::authored_in` — legacy plaques in
    /// pre-amendment saves load as Survival.
    #[serde(default = "default_authored_in")]
    pub authored_in: String,
    /// Spec 26 — Builder NPC credit. `Some` when the plaque was placed
    /// at the end of an NPC commission; renders an extra "Built by
    /// {villager} of {village} for {commissioner}" line in the dialog.
    /// `None` for player-driven builds + legacy plaques.
    #[serde(default)]
    pub builder_credit: Option<crate::builder::BuilderCredit>,
}

impl ArchitectPlaqueData {
    pub fn from_plan(plan: &PlanData) -> Self {
        ArchitectPlaqueData {
            chain: plan.derivation_chain.clone(),
            authored_in: plan.authored_in.clone(),
            builder_credit: None,
        }
    }

    /// Spec 26 — variant carrying a Builder credit line. Used by
    /// `complete_build` when the anchor's `builder_credit` is `Some`.
    pub fn from_plan_with_credit(
        plan: &PlanData,
        builder_credit: Option<crate::builder::BuilderCredit>,
    ) -> Self {
        ArchitectPlaqueData {
            chain: plan.derivation_chain.clone(),
            authored_in: plan.authored_in.clone(),
            builder_credit,
        }
    }
}

/// Backwards-compat alias used by older code-paths. Prefer
/// `ArchitectPlaqueData` going forward.
pub type PlaqueChain = ArchitectPlaqueData;

// ─── Build tick (Phase 11) ────────────────────────────────────────────

/// Cells placed per tick by the animated builder.
pub const BLOCKS_PER_BUILD_TICK: usize = 2;

/// Reasons `start_build` may refuse — used by the caller to surface
/// the correct toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildStartError {
    Placement(PlacementCheck),
    /// Survival-only: the player's inventory lacks one or more
    /// required blocks. Caller should send the player back to the
    /// Inspect dialog.
    MissingMaterials(BlockId, u32, u32), // (block, needed, owned)
}

/// Aggregate the cells of a plan into a `(block_id, count)` shopping
/// list. Used by both `lock_materials_for_mode` and the Inspect dialog.
/// Pure.
pub fn cell_block_counts(cells: &[CapturedCell]) -> ahash::AHashMap<BlockId, u32> {
    let mut counts = ahash::AHashMap::new();
    for c in cells {
        *counts.entry(c.block_id).or_insert(0u32) += 1;
    }
    counts
}

/// Spec 24 Phase 8 — read-only sufficiency check used by the ghost
/// preview to decide between Green and Yellow tints. Mirrors the
/// verify pass at the top of `lock_materials_for_mode` without
/// mutating the inventory. Returns `true` if every block in `cells`
/// is present in `inventory` in sufficient quantity.
pub fn inventory_has_materials(
    cells: &[CapturedCell],
    inventory: &crate::inventory::Inventory,
) -> bool {
    let counts = cell_block_counts(cells);
    for (&block_id, &needed) in &counts {
        if inventory_block_count(inventory, block_id) < needed {
            return false;
        }
    }
    true
}

/// How many of `block_id` the player is carrying, across every inventory slot.
/// The single counter shared by the material check, the survival material lock
/// and the build-guide's "go and gather this" shortfall (`build_steps::shortfall`).
pub fn inventory_block_count(
    inventory: &crate::inventory::Inventory,
    block_id: BlockId,
) -> u32 {
    (0..36)
        .filter_map(|i| inventory.slot(i))
        .filter_map(|s| match s.item {
            crate::item::Item::Block(b) if b == block_id => Some(s.count as u32),
            _ => None,
        })
        .sum()
}

/// Spec 24 §"Creative vs Survival" amendment — single chokepoint for
/// the survival/creative material-lock branch. In creative this
/// returns `Ok(vec![])` without touching the inventory; in survival
/// it verifies + decrements the inventory and returns the locked
/// stack snapshot.
pub fn lock_materials_for_mode(
    mode: &str,
    cells: &[CapturedCell],
    inventory: &mut crate::inventory::Inventory,
) -> Result<Vec<crate::item::ItemStack>, BuildStartError> {
    if mode == "creative" {
        // Creative — no inventory touched, no locking.
        return Ok(Vec::new());
    }
    // Survival — count requirements then verify.
    let counts = cell_block_counts(cells);
    // Verify before mutating so a partial decrement can't leak.
    for (&block_id, &needed) in &counts {
        let owned = inventory_block_count(inventory, block_id);
        if owned < needed {
            return Err(BuildStartError::MissingMaterials(block_id, needed, owned));
        }
    }
    // Decrement + build snapshot.
    let mut locked = Vec::new();
    for (&block_id, &needed) in &counts {
        let mut remaining = needed;
        // Walk slots until decrement satisfied.
        for i in 0..36 {
            if remaining == 0 { break; }
            let mut slot = match inventory.slot(i).cloned() {
                Some(s) => s,
                None => continue,
            };
            if !matches!(slot.item, crate::item::Item::Block(b) if b == block_id) {
                continue;
            }
            let take = (slot.count as u32).min(remaining);
            slot.count -= take as u8;
            remaining -= take;
            if slot.count == 0 {
                inventory.set_slot(i, None);
            } else {
                inventory.set_slot(i, Some(slot));
            }
        }
        debug_assert_eq!(remaining, 0, "verify pass should have caught this");
        // `ItemStack.count` is u8, so a per-block-id total above 255 must be
        // split across multiple stacks — `needed as u8` truncated it (e.g.
        // 300 → 44), under-refunding big survival builds on cancel (engine
        // audit 2026-06-04, A). The consume + refund paths already iterate
        // every stack, so multiple entries are transparent to them.
        let mut rem = needed;
        while rem > 0 {
            let chunk = rem.min(u8::MAX as u32) as u8;
            locked.push(crate::item::ItemStack::new_block(block_id, chunk));
            rem -= chunk as u32;
        }
    }
    Ok(locked)
}

/// Spec 24 Phase 10 — set up a ConstructionAnchor at the player's
/// chosen position. Validates placement first; locks materials per
/// the mode chokepoint; writes the CONSTRUCTION_ANCHOR block + anchor
/// data entry. Returns the anchor position on success.
pub fn start_build(
    world: &mut World,
    registry: &crate::block::BlockRegistry,
    mode: &str,
    plan: PlanData,
    rotations: u8,
    anchor: (i32, i32, i32),
    inventory: &mut crate::inventory::Inventory,
    player_positions: &[(i32, i32, i32)],
) -> Result<(i32, i32, i32), BuildStartError> {
    let check = validate_placement(world, registry, &plan, anchor, rotations, player_positions);
    if check != PlacementCheck::Valid {
        return Err(BuildStartError::Placement(check));
    }
    // Lock materials before placing the anchor — if locking fails the
    // anchor block doesn't get placed.
    let locked_materials = lock_materials_for_mode(mode, &plan.cells, inventory)?;
    let is_creative_build = mode == "creative";
    // Place the anchor block at (anchor.x, anchor.y, anchor.z) — but
    // the foundation cell at anchor.y is already solid (Phase 9 confirmed).
    // Spec 24 §10 puts the anchor on top of the foundation. So the
    // anchor block sits at anchor.y+1... wait, that conflicts with the
    // build volume which also occupies anchor.y+1.. anchor.y+1+height.
    //
    // Resolution: the anchor block-id is stored in the cell at (ax, ay,
    // az), REPLACING the foundation cell at that position. The build
    // volume above is unaffected. On completion the foundation cell is
    // restored to its original block via the placed Plaque.
    //
    // Simpler v1: store anchor data in World::construction_anchors but
    // don't place a literal CONSTRUCTION_ANCHOR block — the world
    // state already knows there's an in-progress build at this
    // position via the AHashMap. Reduces complexity at the cost of a
    // visible "marker block" (which Spec 8 ghost-preview would
    // replace anyway).
    world.construction_anchors.insert(
        anchor,
        ConstructionAnchorData {
            plan,
            rotations,
            anchor,
            placed_index: 0,
            locked_materials,
            is_creative_build,
            pace_divider: 1,
            pace_counter: 0,
            builder_credit: None,
        },
    );
    Ok(anchor)
}

/// Advance one ConstructionAnchor by BLOCKS_PER_BUILD_TICK cells.
/// Returns world-space positions newly placed this tick so the caller
/// can broadcast block-changes + rebuild affected chunks.
///
/// Returns an empty Vec when the build is complete or stalled.
pub fn tick_build(
    world: &mut World,
    anchor_pos: (i32, i32, i32),
) -> Vec<(i32, i32, i32, BlockId)> {
    let Some(data) = world.construction_anchors.get(&anchor_pos).cloned() else {
        return Vec::new();
    };
    // Spec 26 pace gate. Always increment the counter; only place blocks
    // when the counter is on-beat. `pace_divider == 0` would divide by
    // zero — treat it as 1 (player pace) for safety against malformed
    // legacy data.
    let divider = data.pace_divider.max(1);
    let next_counter = data.pace_counter.wrapping_add(1);
    let on_beat = data.pace_counter % divider as u64 == 0;
    if !on_beat {
        if let Some(d) = world.construction_anchors.get_mut(&anchor_pos) {
            d.pace_counter = next_counter;
        }
        return Vec::new();
    }
    let ordered = order_cells_for_build(&rotate_cells(&data.plan.cells, data.rotations, data.plan.width, data.plan.depth));
    let mut placed = Vec::new();
    let mut next_index = data.placed_index;
    let mut new_locked = data.locked_materials.clone();
    for _ in 0..BLOCKS_PER_BUILD_TICK {
        if next_index >= ordered.len() {
            break;
        }
        let cell = ordered[next_index];
        let (ax, ay, az) = data.anchor;
        let wx = ax + cell.rx as i32;
        let wy = ay + 1 + cell.ry as i32;
        let wz = az + cell.rz as i32;
        // Spec 06 §2.2 — builder-stamped blocks are player-placed, so mining
        // the finished structure earns no proof-of-play work.
        world.place_player_block(wx, wy, wz, cell.block_id);
        // Owner-inbox #1/2/3 — a build stamping over a painted block clears its
        // wallpaper overlays so none orphan beneath the newly-placed block.
        world.remove_face_attachments_at((wx, wy, wz));
        placed.push((wx, wy, wz, cell.block_id));
        // Survival: consume from locked_materials. Creative: skip.
        if !data.is_creative_build {
            for stack in new_locked.iter_mut() {
                if matches!(stack.item, crate::item::Item::Block(b) if b == cell.block_id)
                    && stack.count > 0
                {
                    stack.count -= 1;
                    break;
                }
            }
            new_locked.retain(|s| s.count > 0);
        }
        next_index += 1;
    }
    // Update placed_index + locked_materials + pace_counter in-place.
    if let Some(d) = world.construction_anchors.get_mut(&anchor_pos) {
        d.placed_index = next_index;
        d.locked_materials = new_locked;
        d.pace_counter = next_counter;
    }
    placed
}

/// Spec 24 Phase 11 — cancel an in-progress build. Refunds any
/// remaining `locked_materials` to the inventory (already-placed
/// blocks remain in the world) + removes the anchor data.
///
/// In creative `locked_materials` is empty, so the refund is a
/// no-op; the cancel still works. No UI "cancel my self-build" button calls
/// this yet (only NPC-commissioned builds can be cancelled today, via
/// `builder::cancel_commission_refund`) — tested directly.
#[cfg_attr(not(test), allow(dead_code))]
pub fn cancel_build(
    world: &mut World,
    anchor_pos: (i32, i32, i32),
    inventory: &mut crate::inventory::Inventory,
) {
    if let Some(data) = world.construction_anchors.remove(&anchor_pos) {
        for stack in data.locked_materials {
            if stack.count > 0 {
                let _ = inventory.add_item(stack);
            }
        }
    }
}

/// Spec 24 Phase 12 — finalise a completed build. Removes the anchor,
/// places the Architect's Plaque at the most-central XZ tile cell on
/// the build's floor (y = anchor.y + 1). Returns the plaque position
/// so the caller can broadcast + rebuild.
pub fn complete_build(
    world: &mut World,
    anchor_pos: (i32, i32, i32),
) -> Option<(i32, i32, i32)> {
    let data = world.construction_anchors.remove(&anchor_pos)?;
    let (ax, ay, az) = data.anchor;
    // Most-central XZ cell on the floor — pick the cell at (rx, 0, rz)
    // with rx = width/2, rz = depth/2 (rotation-aware).
    let (rotated_w, rotated_d) = match data.rotations % 4 {
        1 | 3 => (data.plan.depth, data.plan.width),
        _ => (data.plan.width, data.plan.depth),
    };
    let plaque_x = ax + (rotated_w / 2) as i32;
    let plaque_y = ay + 1;
    let plaque_z = az + (rotated_d / 2) as i32;
    // Place plaque block. Note that this OVERRIDES whatever cell from
    // the plan sits at the central position. v1 accepts this; v2 could
    // pick a non-overlapping cell or set the plaque next to the build.
    // Spec 06 §2.2 — the plaque is player-placed (part of the built structure).
    world.place_player_block(plaque_x, plaque_y, plaque_z, block::ARCHITECT_PLAQUE);
    // Owner-inbox #1/2/3 — the plaque overrides whatever cell sat here; clear any
    // wallpaper overlays so none orphan beneath it.
    world.remove_face_attachments_at((plaque_x, plaque_y, plaque_z));
    world.architect_plaques.insert(
        (plaque_x, plaque_y, plaque_z),
        ArchitectPlaqueData::from_plan_with_credit(&data.plan, data.builder_credit.clone()),
    );
    Some((plaque_x, plaque_y, plaque_z))
}

// ─── Placement validation (Phase 9) ───────────────────────────────────

/// Result of `validate_placement`. Each variant maps to a specific
/// toast message + Ghost-preview colour (Phase 8, deferred):
/// - `Valid` → green wireframe + Place button enabled
/// - `NotFlat` / `VolumeOccupied` / `PlayerInside` / `ChunksMissing` →
///   red wireframe + tooltip
/// - `MaterialsShort` is computed alongside but is handled separately
///   in survival (the Inspect dialog already gates on it before reaching
///   placement). Kept here for unified call-site logic post-Phase 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementCheck {
    Valid,
    /// One of the cells immediately above the anchor base isn't air —
    /// the player must clear obstructions before placing.
    VolumeOccupied,
    /// The cells in the world that would receive the build aren't all
    /// supported by solid ground at anchor.y. Build site needs a flat
    /// foundation (Spec 24 §3.3 — same rule as capture).
    NotFlat,
    /// One of the player slots is standing in the captured volume's
    /// world-space cells — placing would extrude a player out of the
    /// world. Bail.
    PlayerInside,
    /// Some chunks the build would span are not loaded — can't place
    /// across a chunk boundary if we haven't seen the far side yet.
    ChunksMissing,
}

/// Pure function — validates whether a plan can be placed with its
/// lowest-XZ corner anchored at `(anchor.x, anchor.y, anchor.z)` and
/// rotated by `rotations` 90° steps clockwise. The build occupies
/// `[anchor.x..anchor.x+width)` × `[anchor.y+1..anchor.y+1+height)` ×
/// `[anchor.z..anchor.z+depth)` in world space after rotation.
///
/// `player_positions` is the list of every PlayerSlot's current world-
/// space block-position (`player.pos.floor() as i32`) — used to check
/// no player is standing inside the build volume.
pub fn validate_placement(
    world: &World,
    registry: &crate::block::BlockRegistry,
    data: &PlanData,
    anchor: (i32, i32, i32),
    rotations: u8,
    player_positions: &[(i32, i32, i32)],
) -> PlacementCheck {
    let rotated = rotate_cells(&data.cells, rotations, data.width, data.depth);
    let (rotated_w, rotated_d) = match rotations % 4 {
        1 | 3 => (data.depth, data.width),
        _ => (data.width, data.depth),
    };
    let (ax, ay, az) = anchor;

    // 0. Chunks loaded — must run FIRST so the other checks don't
    //    silently treat "unloaded chunk returns AIR" as "site is open
    //    air". Without this, an empty World with no chunks would
    //    appear infinitely flat-free AND infinitely-clear.
    let min_chunk = (ax.div_euclid(16), ay.div_euclid(16), az.div_euclid(16));
    let max_chunk = (
        (ax + rotated_w as i32 - 1).div_euclid(16),
        (ay + 1 + data.height as i32 - 1).div_euclid(16),
        (az + rotated_d as i32 - 1).div_euclid(16),
    );
    for cx in min_chunk.0..=max_chunk.0 {
        for cy in min_chunk.1..=max_chunk.1 {
            for cz in min_chunk.2..=max_chunk.2 {
                if world.get_chunk(cx, cy, cz).is_none() {
                    return PlacementCheck::ChunksMissing;
                }
            }
        }
    }

    // 1. Flat ground — every (rx, rz) column must have a solid block
    //    at world (ax+rx, ay, az+rz). This mirrors the capture-time
    //    flat-ground check.
    for rx in 0..rotated_w {
        for rz in 0..rotated_d {
            let wx = ax + rx as i32;
            let wz = az + rz as i32;
            // Skip columns the plan doesn't actually occupy at y=0
            // (e.g., L-shapes leave some columns empty). The footprint
            // mask is "any cell in this column" — same as ascii_footprint.
            let column_used = rotated
                .iter()
                .any(|c| c.rx == rx && c.rz == rz);
            if !column_used {
                continue;
            }
            if !world.is_solid(wx, ay, wz, registry) {
                return PlacementCheck::NotFlat;
            }
        }
    }

    // 2. Empty volume — every captured cell's world position must be AIR.
    for c in &rotated {
        let wx = ax + c.rx as i32;
        let wy = ay + 1 + c.ry as i32;
        let wz = az + c.rz as i32;
        if world.get_block(wx, wy, wz) != block::AIR {
            return PlacementCheck::VolumeOccupied;
        }
    }

    // 3. No player inside.
    for &(px, py, pz) in player_positions {
        for c in &rotated {
            let wx = ax + c.rx as i32;
            let wy = ay + 1 + c.ry as i32;
            let wz = az + c.rz as i32;
            if (wx, wy, wz) == (px, py, pz)
                || (wx, wy + 1, wz) == (px, py, pz) // accommodate 2-block-tall player hitbox
            {
                return PlacementCheck::PlayerInside;
            }
        }
    }

    PlacementCheck::Valid
}

/// Human-readable toast for a placement check refusal. `Valid` returns
/// an empty string (caller shouldn't be calling this on success).
/// Surfaced by the game_loop ghost handler via `placement_toast_decision`
/// below — the red/yellow/green tint alone didn't say *why* (Task 17,
/// bug-hardening wave).
pub fn placement_check_message(check: PlacementCheck) -> &'static str {
    match check {
        PlacementCheck::Valid => "",
        PlacementCheck::NotFlat => "Foundation isn't flat — clear or fill underneath",
        PlacementCheck::VolumeOccupied => "Something's in the way — clear the build site first",
        PlacementCheck::PlayerInside => "A player is standing where the build would go",
        PlacementCheck::ChunksMissing => "Too close to unloaded chunks — move closer to a loaded area",
    }
}

/// Task 17 — per-ghost rate-limit bookkeeping for the blocked-placement
/// toast. Lives on `GhostState` so it survives frame-to-frame while the
/// ghost preview is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GhostToastState {
    /// The check a toast was last shown for (`None` = nothing shown yet,
    /// or the ghost was last `Valid`).
    last_shown: Option<PlacementCheck>,
    /// Raw place-attempt input state from the previous frame — lets
    /// `placement_toast_decision` detect the rising edge of a click
    /// without the caller needing its own held-state bookkeeping.
    attempt_down_prev: bool,
}

/// Pure decision: should the blocked-placement message (re-)fire this
/// frame, and what should the caller remember for next frame?
///
/// Fires on:
/// - the RISING EDGE of an explicit place attempt while blocked (so a
///   held mouse button doesn't retrigger every tick at ~20/s), or
/// - the instant the blocking reason changes (e.g. scrubbing the ghost
///   from a not-flat spot to an occupied one) — even without a click, so
///   the player learns why *this* spot is bad as they move the cursor.
///
/// Never fires merely because the same reason persists across frames —
/// that's the "shows red with no explanation" bug's mirror image (an
/// explanation that never shuts up).
pub fn placement_toast_decision(
    check: PlacementCheck,
    attempt_down: bool,
    state: GhostToastState,
) -> (Option<PlacementCheck>, GhostToastState) {
    if check == PlacementCheck::Valid {
        return (None, GhostToastState { last_shown: None, attempt_down_prev: attempt_down });
    }
    let attempt_edge = attempt_down && !state.attempt_down_prev;
    let reason_changed = state.last_shown != Some(check);
    if attempt_edge || reason_changed {
        (Some(check), GhostToastState { last_shown: Some(check), attempt_down_prev: attempt_down })
    } else {
        (None, GhostToastState { last_shown: state.last_shown, attempt_down_prev: attempt_down })
    }
}

/// Whether a build at this anchor has finished. The caller is
/// responsible for placing the Plaque + removing the anchor block
/// once true.
pub fn build_is_complete(world: &World, anchor_pos: (i32, i32, i32)) -> bool {
    match world.construction_anchors.get(&anchor_pos) {
        Some(d) => d.placed_index >= d.plan.cells.len(),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    fn cell(rx: u8, ry: u8, rz: u8, b: BlockId) -> CapturedCell {
        CapturedCell { rx, ry, rz, block_id: b }
    }

    /// The "Follow the Plan" Trial hands this plan over as a build-along guide,
    /// so its shape IS the trial: a hut you can walk into, with a roof, in few
    /// enough blocks to finish in one sitting.
    #[test]
    fn the_little_hut_plan_is_a_hut_you_can_walk_into() {
        let p = PlanData::small_hut();
        assert_eq!((p.width, p.depth, p.height), (3, 3, 3));
        assert_eq!(p.develop_state, DevelopState::Developed, "handed over ready to lay");
        assert_eq!(p.kind, PlanKind::Building);

        let at = |rx: u8, ry: u8, rz: u8| {
            p.cells.iter().find(|c| c.rx == rx && c.ry == ry && c.rz == rz).map(|c| c.block_id)
        };
        // A doorway you can actually walk through, both courses high.
        assert_eq!(at(1, 0, 0), None, "the front wall has a doorway");
        assert_eq!(at(1, 1, 0), None, "…two blocks tall");
        // A hollow room, not a solid lump.
        assert_eq!(at(1, 0, 1), None, "the inside is empty");
        assert_eq!(at(1, 1, 1), None);
        // A window in each side wall, and a roof over the lot.
        assert_eq!(at(0, 1, 1), Some(block::GLASS));
        assert_eq!(at(2, 1, 1), Some(block::GLASS));
        for rx in 0..3 {
            for rz in 0..3 {
                assert_eq!(at(rx, 2, rz), Some(block::OAK_PLANKS), "roofed at ({rx}, {rz})");
            }
        }
        assert_eq!(p.cells.len(), 23, "a build-along you can finish in one sitting");
        assert!(!p.derivation_chain.is_empty(), "every handed-over plan carries its provenance");
    }

    /// Module-level helper: lay a blueprint tile at (x,y,z) — a solid DIRT
    /// floor block carrying a BlueprintBlank top-attachment.
    fn lay_tile(w: &mut World, x: i32, y: i32, z: i32) {
        w.set_block(x, y, z, block::DIRT);
        w.set_face_attachment(
            (x, y, z),
            crate::mesh::Face::Top.index(),
            crate::world::FaceAttachment::BlueprintBlank,
        );
    }

    // ─── Task 1.1 — connectivity-flood capture (build-on-paper model) ──

    // connected build over paper → captured
    #[test]
    fn column_capture_grabs_connected_build() {
        let mut w = World::new();
        lay_tile(&mut w, 0, 70, 0); lay_tile(&mut w, 1, 70, 0);
        w.set_block(0, 71, 0, block::STONE);  // wall sitting ON the paper
        w.set_block(0, 72, 0, block::STONE);  // connected upward
        let c = capture(&w, (0, 70, 0), "creative").expect("capture");
        assert_eq!(c.data.cells.len(), 2);
        assert!(c.data.cells.iter().all(|cell| cell.block_id == block::STONE));
    }

    // cave ceiling above an AIR GAP → excluded
    #[test]
    fn column_capture_excludes_ceiling_across_air_gap() {
        let mut w = World::new();
        lay_tile(&mut w, 0, 70, 0);
        w.set_block(0, 71, 0, block::STONE);   // 1-tall pillar on the paper
        w.set_block(0, 74, 0, block::STONE);   // ceiling, 2-block air gap below
        let c = capture(&w, (0, 70, 0), "creative").expect("capture");
        assert_eq!(c.data.cells.len(), 1, "only the connected pillar, not the ceiling");
    }

    // block outside the footprint (no paper under it) → excluded even if adjacent
    #[test]
    fn column_capture_excludes_block_outside_footprint() {
        let mut w = World::new();
        lay_tile(&mut w, 0, 70, 0);
        w.set_block(0, 71, 0, block::STONE);   // on paper
        w.set_block(1, 71, 0, block::STONE);   // hard against it, but (1,_,0) has NO paper
        let c = capture(&w, (0, 70, 0), "creative").expect("capture");
        assert_eq!(c.data.cells.len(), 1, "neighbour wall over non-papered ground excluded");
    }

    // gazebo: open floor (no floor blocks), pillars + roof → captured WITHOUT a filled floor
    #[test]
    fn column_capture_gazebo_open_floor() {
        let mut w = World::new();
        for x in 0..3 { for z in 0..3 { lay_tile(&mut w, x, 70, z); } }
        for &(px, pz) in &[(0i32,0i32),(2,0),(0,2),(2,2)] {
            w.set_block(px, 71, pz, block::OAK_LOG); w.set_block(px, 72, pz, block::OAK_LOG);
        }
        for x in 0..3i32 { for z in 0..3i32 { w.set_block(x, 73, z, block::OAK_PLANKS); } }
        let c = capture(&w, (0, 70, 0), "creative").expect("capture");
        // 4 pillars*2 + 9 roof = 17 cells.
        assert_eq!(c.data.cells.len(), 17);
        // OPEN FLOOR: the bottom layer (ry==0, the cells sitting on the paper) holds
        // ONLY the 4 pillar feet — NOT a filled 9-cell floor.
        assert_eq!(c.data.cells.iter().filter(|cell| cell.ry == 0).count(), 4,
            "open floor: only the 4 pillar feet sit on the paper, no filled floor layer");
    }

    // nothing built on the paper → EmptyVolume
    #[test]
    fn column_capture_empty_volume_refusal() {
        let mut w = World::new();
        lay_tile(&mut w, 0, 70, 0);
        assert_eq!(capture(&w, (0, 70, 0), "creative").unwrap_err(), CaptureRefusal::EmptyVolume);
    }

    // footprint wider than MAX_FOOTPRINT (32) → TooLarge
    #[test]
    fn column_capture_refuses_oversized_footprint() {
        let mut w = World::new();
        for x in 0..=32 { lay_tile(&mut w, x, 70, 0); }   // 33 wide > MAX_FOOTPRINT
        w.set_block(0, 71, 0, block::STONE);
        assert_eq!(capture(&w, (0, 70, 0), "creative").unwrap_err(), CaptureRefusal::TooLarge);
    }

    #[test]
    fn plan_data_bincode_round_trip() {
        let data = PlanData::debug_3x3_stone();
        let bytes = bincode::serialize(&data).unwrap();
        let back: PlanData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.version, 1);
        assert_eq!(back.width, 3);
        assert_eq!(back.depth, 3);
        assert_eq!(back.cells.len(), 9);
        assert_eq!(back.derivation_chain.len(), 1);
        assert!(back.is_master);
    }

    #[test]
    fn plan_with_long_chain_round_trips() {
        let mut data = PlanData::debug_3x3_stone();
        let h = content_hash(&data);
        for i in 1..10 {
            data.derivation_chain.push(DerivationLink {
                author_npub: format!("npub-{i}"),
                plan_name: format!("v{i}"),
                license: PlanLicense::Ccbysa,
                captured_at: i as u64,
                plan_hash: h,
            });
        }
        let bytes = bincode::serialize(&data).unwrap();
        let back: PlanData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.derivation_chain.len(), 10);
        assert_eq!(back.derivation_chain[9].plan_name, "v9");
    }

    #[test]
    fn content_hash_independent_of_derivation_chain() {
        let mut a = PlanData::debug_3x3_stone();
        let mut b = a.clone();
        b.derivation_chain.push(DerivationLink {
            author_npub: "different".to_string(),
            plan_name: "different".to_string(),
            license: PlanLicense::CC0,
            captured_at: 999,
            plan_hash: [9; 32],
        });
        // Also vary author_npub + license at the top-level — those ARE
        // part of the hash since they identify the plan itself.
        a.author_npub = "x".to_string();
        b.author_npub = "x".to_string();
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn content_hash_changes_when_cells_change() {
        let a = PlanData::debug_3x3_stone();
        let mut b = a.clone();
        b.cells[0].block_id = block::DIRT;
        assert_ne!(content_hash(&a), content_hash(&b));
    }

    // ─── C3c-3a — the marker ─────────────────────────────────────────────

    /// The marker covers EVERY field: two Plans that differ only in what the
    /// content hash leaves out (develop state, derivation chain, kind,
    /// authored-in) have different markers; the same Plan cloned has the same.
    #[test]
    fn a_plans_marker_covers_every_field_the_content_hash_leaves_out() {
        let a = PlanData::debug_3x3_stone();
        assert_eq!(marker(&a), marker(&a.clone()), "the same Plan cloned");
        let latent = PlanData { develop_state: DevelopState::Latent { exposure_ticks: 0 }, ..a.clone() };
        let mut chain = a.clone();
        chain.derivation_chain.push(DerivationLink {
            author_npub: String::new(),
            plan_name: "parent".to_string(),
            license: PlanLicense::CC0,
            captured_at: 0,
            plan_hash: [7; 32],
        });
        let art = PlanData { kind: PlanKind::Art, ..a.clone() };
        let creative = PlanData { authored_in: "creative".to_string(), ..a.clone() };
        for (what, b) in [("develop_state", &latent), ("derivation_chain", &chain), ("kind", &art), ("authored_in", &creative)] {
            assert_eq!(content_hash(&a), content_hash(b), "{what}: the content hash leaves it out");
            assert_ne!(marker(&a), marker(b), "{what}: the marker does not");
            assert!(!a.same_plan(b) && !b.same_plan(&a), "{what}: not the same Plan");
        }
        let other_cells = PlanData { cells: a.cells[1..].to_vec(), ..a.clone() };
        assert_ne!(marker(&a), marker(&other_cells));
    }

    /// A marker placeholder answers the marker it stands for (never a hash
    /// of its stub body), carries the develop state, and is the same Plan as
    /// the real one by marker.
    #[test]
    fn a_marker_placeholder_is_the_plan_whose_marker_it_carries() {
        let real = PlanData { develop_state: DevelopState::Latent { exposure_ticks: 0 }, ..PlanData::debug_3x3_stone() };
        let m = marker(&real);
        let p = PlanData::marker_placeholder(m, false);
        assert!(p.marker.is_some() && real.marker.is_none());
        assert_eq!(marker(&p), m);
        assert_eq!(p.develop_state, DevelopState::Latent { exposure_ticks: 0 });
        assert!(p.cells.is_empty(), "body-less");
        assert!(p.same_plan(&real) && real.same_plan(&p));
        assert!(p.same_plan(&p.clone()));
        let other = PlanData::marker_placeholder([1; 32], false);
        assert!(!other.same_plan(&real) && !other.same_plan(&p));
        assert_eq!(
            PlanData::marker_placeholder(m, true).develop_state,
            DevelopState::Developed
        );
    }

    /// The carrier changes no encoding: the marker is `#[serde(skip)]`, so a
    /// placeholder bincodes exactly as the same stub without it, and a real
    /// Plan as it always did — and a placeholder that is bincoded comes back
    /// without its marker (the open question the sidecar lane closes).
    #[test]
    fn the_marker_changes_no_encoding_and_is_lost_through_bincode() {
        let p = PlanData::marker_placeholder([3; 32], true);
        let bare = PlanData { marker: None, ..p.clone() };
        assert_eq!(plan_bytes(&p), plan_bytes(&bare));
        let back: PlanData = bincode::deserialize(&plan_bytes(&p)).unwrap();
        assert_eq!(back.marker, None, "lost through bincode");
        let real = PlanData::debug_3x3_stone();
        let back: PlanData = bincode::deserialize(&plan_bytes(&real)).unwrap();
        assert_eq!(back, real);
    }

    fn build_test_world_with_tiles(positions: &[(i32, i32, i32)]) -> World {
        let mut w = World::new();
        // New model: each tile cell is itself a solid floor block carrying a
        // BlueprintBlank attachment on its TOP face (foundation = the tile cell).
        for &(x, y, z) in positions {
            w.set_block(x, y, z, block::DIRT);
            w.set_face_attachment(
                (x, y, z),
                crate::mesh::Face::Top.index(),
                crate::world::FaceAttachment::BlueprintBlank,
            );
        }
        w
    }

    #[test]
    fn flood_fill_tiles_returns_single_for_isolated_tile() {
        let world = build_test_world_with_tiles(&[(0, 70, 0)]);
        let tiles = flood_fill_tiles(&world, (0, 70, 0));
        assert_eq!(tiles.len(), 1);
    }

    #[test]
    fn flood_fill_tiles_returns_all_connected() {
        let positions: Vec<(i32, i32, i32)> = (0..3).flat_map(|x| (0..3).map(move |z| (x, 70, z))).collect();
        let world = build_test_world_with_tiles(&positions);
        let tiles = flood_fill_tiles(&world, (1, 70, 1));
        assert_eq!(tiles.len(), 9);
    }

    #[test]
    fn flood_fill_tiles_does_not_cross_y_levels() {
        // Two clusters of tiles, one at y=70 and one at y=75; they
        // must not merge.
        let mut world = World::new();
        for x in 0..3 {
            for z in 0..3 {
                lay_tile(&mut world, x, 70, z);
                lay_tile(&mut world, x, 75, z);
            }
        }
        let tiles = flood_fill_tiles(&world, (0, 70, 0));
        assert_eq!(tiles.len(), 9, "found {:?}", tiles);
    }

    #[test]
    fn flood_fill_tiles_returns_empty_for_non_tile() {
        let world = World::new();
        let tiles = flood_fill_tiles(&world, (0, 70, 0));
        assert!(tiles.is_empty());
    }


    #[test]
    fn capture_refuses_isolated_non_tile() {
        let world = World::new();
        let res = capture(&world, (0, 70, 0), "survival");
        assert!(matches!(res, Err(CaptureRefusal::Disconnected)));
    }

    #[test]
    fn capture_refuses_empty_volume() {
        // Tile + foundation, but nothing built on top.
        let world = build_test_world_with_tiles(&[(0, 70, 0)]);
        let res = capture(&world, (0, 70, 0), "survival");
        assert!(matches!(res, Err(CaptureRefusal::EmptyVolume)));
    }


    #[test]
    fn capture_captures_simple_pillar() {
        // 2-tile footprint with a 2-block stone pillar sitting directly on
        // one of the paper tiles. New build-on-paper model: block at y=71
        // (base_y+1) is the seed; connected block at y=72 is reached by flood.
        let mut world = World::new();
        lay_tile(&mut world, 0, 70, 0);
        lay_tile(&mut world, 1, 70, 0);
        world.set_block(0, 71, 0, block::STONE);  // sits on tile (0,70,0)
        world.set_block(0, 72, 0, block::STONE);  // connected upward
        let candidate = capture(&world, (0, 70, 0), "survival").expect("capture should succeed");
        assert_eq!(candidate.tile_positions.len(), 2);
        assert!(!candidate.data.cells.is_empty());
        assert!(candidate.data.cells.iter().all(|c| c.block_id == block::STONE));
        assert_eq!(candidate.data.width, 2);
        assert_eq!(candidate.data.depth, 1);
    }

    // ─── Spec 24 2026-05-20 amendment — authored_in + no refund ─────

    #[test]
    fn capture_records_authored_in_from_argument() {
        let mut world = World::new();
        lay_tile(&mut world, 0, 70, 0);
        lay_tile(&mut world, 1, 70, 0);
        world.set_block(0, 71, 0, block::STONE);  // build sits ON the paper

        let surv = capture(&world, (0, 70, 0), "survival").unwrap();
        let creat = capture(&world, (0, 70, 0), "creative").unwrap();
        assert_eq!(surv.data.authored_in, "survival");
        assert_eq!(creat.data.authored_in, "creative");
    }

    #[test]
    fn plan_data_authored_in_round_trips_through_bincode() {
        let mut data = PlanData::debug_3x3_stone();
        data.authored_in = "creative".to_string();
        let bytes = bincode::serialize(&data).unwrap();
        let back: PlanData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.authored_in, "creative");
    }

    #[test]
    fn content_hash_independent_of_authored_in() {
        // A survival-authored plan and a creative-authored plan with
        // identical cells must share a content hash — otherwise cross-
        // mode derivation detection (Spec 24 §6 Save-As) breaks.
        let mut a = PlanData::debug_3x3_stone();
        a.authored_in = "survival".to_string();
        let mut b = a.clone();
        b.authored_in = "creative".to_string();
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn architect_plaque_data_from_plan_carries_authored_in() {
        let mut data = PlanData::debug_3x3_stone();
        data.authored_in = "creative".to_string();
        let plaque = ArchitectPlaqueData::from_plan(&data);
        assert_eq!(plaque.authored_in, "creative");
        assert_eq!(plaque.chain.len(), data.derivation_chain.len());
    }

    // ─── Spec 24 Phase 5 — commit_capture ────────────────────────────

    fn make_pending(world: &mut World) -> PendingCapture {
        lay_tile(world, 0, 70, 0);
        lay_tile(world, 1, 70, 0);
        world.set_block(0, 71, 0, block::STONE);  // build sits ON the paper
        world.set_block(1, 71, 0, block::STONE);
        let candidate = capture(world, (0, 70, 0), "survival").expect("capture");
        PendingCapture { candidate, parent_match: None, mark_as_derivative: false }
    }

    #[test]
    fn commit_capture_consumes_attachments_leaving_floor() {
        let mut world = World::new();
        let pending = make_pending(&mut world);
        let mut inventory = crate::inventory::Inventory::new();
        // Pre-capture: tile cells are solid floor blocks carrying the
        // BlueprintBlank attachment.
        assert_eq!(world.get_block(0, 70, 0), block::DIRT);
        assert_eq!(world.get_block(1, 70, 0), block::DIRT);
        assert!(matches!(
            world.face_attachment_at((0, 70, 0), crate::mesh::Face::Top.index()),
            Some(crate::world::FaceAttachment::BlueprintBlank)
        ));

        let result = commit_capture(&mut world, &mut inventory, &pending, None);

        // Post-capture: the player's FLOOR survives (R3b floor fix) —
        // still DIRT, NOT AIR.
        assert_eq!(world.get_block(0, 70, 0), block::DIRT);
        assert_eq!(world.get_block(1, 70, 0), block::DIRT);
        // ...but the BlueprintBlank attachment is consumed.
        assert_eq!(
            world.face_attachment_at((0, 70, 0), crate::mesh::Face::Top.index()),
            None
        );
        assert_eq!(
            world.face_attachment_at((1, 70, 0), crate::mesh::Face::Top.index()),
            None
        );
        // tile_positions reports both affected tiles (attachment consumed).
        assert_eq!(result.tile_positions.len(), 2);
        // No refund — inventory has zero BLUEPRINT_PAPER blocks.
        let tile_count: u32 = (0..36)
            .filter_map(|i| inventory.slot(i))
            .filter(|s| matches!(s.item, crate::item::Item::Block(b) if b == block::BLUEPRINT_PAPER))
            .map(|s| s.count as u32)
            .sum();
        assert_eq!(tile_count, 0, "paper-locked-in rule: tiles must not be refunded");
        // Plan was inserted.
        assert!(result.plan_inserted);
    }

    #[test]
    fn commit_capture_inserts_plan_into_inventory() {
        let mut world = World::new();
        let pending = make_pending(&mut world);
        let mut inventory = crate::inventory::Inventory::new();
        let result = commit_capture(&mut world, &mut inventory, &pending, None);

        let plan_slot = (0..36)
            .filter_map(|i| inventory.slot(i))
            .find(|s| matches!(s.item, crate::item::Item::Plan(_)))
            .expect("Item::Plan must be in inventory after commit");
        if let crate::item::Item::Plan(pd) = &plan_slot.item {
            assert_eq!(pd.name, result.committed_plan.name);
            // The new derivation link was pushed.
            assert_eq!(pd.derivation_chain.len(), 1);
            assert_eq!(pd.derivation_chain[0].plan_name, pd.name);
        } else {
            panic!("slot must be Item::Plan");
        }
    }

    #[test]
    fn commit_capture_prepends_parent_chain_when_mark_as_derivative() {
        let mut world = World::new();
        let mut pending = make_pending(&mut world);
        pending.mark_as_derivative = true;
        pending.parent_match = Some((0, ParentMatchKind::ContentHash));

        // Parent has two chain links — when derivative is committed,
        // the new plan should have 3 chain entries.
        let mut parent = PlanData::debug_3x3_stone();
        parent.derivation_chain.push(DerivationLink {
            author_npub: "ancestor".to_string(),
            plan_name: "Original Hut".to_string(),
            license: PlanLicense::Ccbysa,
            captured_at: 100,
            plan_hash: [1; 32],
        });
        // parent.derivation_chain now has 2 entries (debug_3x3_stone's
        // self-entry + the ancestor we just pushed).
        assert_eq!(parent.derivation_chain.len(), 2);

        let mut inventory = crate::inventory::Inventory::new();
        let result = commit_capture(&mut world, &mut inventory, &pending, Some(&parent));
        assert_eq!(
            result.committed_plan.derivation_chain.len(),
            3,
            "parent chain prepended + new self-entry pushed"
        );
        // Verify ancestor link still present.
        assert!(result
            .committed_plan
            .derivation_chain
            .iter()
            .any(|l| l.plan_name == "Original Hut"));
    }

    #[test]
    fn commit_capture_ignores_parent_when_mark_as_derivative_is_false() {
        let mut world = World::new();
        let mut pending = make_pending(&mut world);
        pending.mark_as_derivative = false;
        pending.parent_match = Some((0, ParentMatchKind::ContentHash));
        let parent = PlanData::debug_3x3_stone();
        let mut inventory = crate::inventory::Inventory::new();
        let result = commit_capture(&mut world, &mut inventory, &pending, Some(&parent));
        // Despite parent being available, chain is just the new self-entry.
        assert_eq!(result.committed_plan.derivation_chain.len(), 1);
    }

    #[test]
    fn commit_capture_reports_full_inventory_via_plan_inserted_false() {
        let mut world = World::new();
        let pending = make_pending(&mut world);
        let mut inventory = crate::inventory::Inventory::new();
        // Pack inventory full so add_item fails on the plan.
        for i in 0..36 {
            inventory.set_slot(
                i,
                Some(crate::item::ItemStack::new_block(block::STONE, 64)),
            );
        }
        let result = commit_capture(&mut world, &mut inventory, &pending, None);
        // Attachment is still consumed — paper is gone regardless — but
        // the player's floor block survives (R3b floor fix).
        assert_eq!(world.get_block(0, 70, 0), block::DIRT);
        assert_eq!(
            world.face_attachment_at((0, 70, 0), crate::mesh::Face::Top.index()),
            None
        );
        // Plan was not inserted.
        assert!(!result.plan_inserted);
    }

    // ─── Spec 24 Phase 9 — placement validation ──────────────────────

    fn plain_3x3_plan() -> PlanData {
        let mut data = PlanData::debug_3x3_stone();
        data.cells = (0..3u8)
            .flat_map(|rx| (0..3u8).map(move |rz| CapturedCell { rx, ry: 0, rz, block_id: block::STONE }))
            .collect();
        data.width = 3;
        data.depth = 3;
        data.height = 1;
        data
    }

    fn world_with_solid_floor(min: (i32, i32, i32), max: (i32, i32, i32)) -> World {
        let mut w = World::new();
        for x in min.0..=max.0 {
            for z in min.2..=max.2 {
                w.set_block(x, min.1, z, block::DIRT);
            }
        }
        w
    }

    #[test]
    fn validate_placement_returns_valid_for_flat_clear_site() {
        let reg = crate::block::BlockRegistry::new();
        let world = world_with_solid_floor((0, 60, 0), (3, 60, 3));
        let plan = plain_3x3_plan();
        let check = validate_placement(&world, &reg, &plan, (0, 60, 0), 0, &[]);
        assert_eq!(check, PlacementCheck::Valid);
    }

    #[test]
    fn validate_placement_refuses_non_flat_foundation() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_solid_floor((0, 60, 0), (3, 60, 3));
        // Punch a hole under one of the plan cells.
        world.set_block(1, 60, 1, block::AIR);
        let plan = plain_3x3_plan();
        let check = validate_placement(&world, &reg, &plan, (0, 60, 0), 0, &[]);
        assert_eq!(check, PlacementCheck::NotFlat);
    }

    #[test]
    fn validate_placement_refuses_occupied_volume() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_solid_floor((0, 60, 0), (3, 60, 3));
        // Place an obstruction inside the build volume (y=61 = anchor+1).
        world.set_block(1, 61, 1, block::OAK_LOG);
        let plan = plain_3x3_plan();
        let check = validate_placement(&world, &reg, &plan, (0, 60, 0), 0, &[]);
        assert_eq!(check, PlacementCheck::VolumeOccupied);
    }

    #[test]
    fn validate_placement_refuses_player_inside() {
        let reg = crate::block::BlockRegistry::new();
        let world = world_with_solid_floor((0, 60, 0), (3, 60, 3));
        let plan = plain_3x3_plan();
        // Player standing at (1, 61, 1) — middle of build site.
        let check = validate_placement(&world, &reg, &plan, (0, 60, 0), 0, &[(1, 61, 1)]);
        assert_eq!(check, PlacementCheck::PlayerInside);
    }

    #[test]
    fn validate_placement_returns_valid_when_player_far_away() {
        let reg = crate::block::BlockRegistry::new();
        let world = world_with_solid_floor((0, 60, 0), (3, 60, 3));
        let plan = plain_3x3_plan();
        let check = validate_placement(&world, &reg, &plan, (0, 60, 0), 0, &[(100, 60, 100)]);
        assert_eq!(check, PlacementCheck::Valid);
    }

    #[test]
    fn validate_placement_refuses_unloaded_chunk() {
        let reg = crate::block::BlockRegistry::new();
        // Empty world — no chunks. validate_placement at anchor (0,60,0)
        // will check (0..1, 3..4, 0..1) for chunk presence — none of which
        // exist.
        let world = World::new();
        let plan = plain_3x3_plan();
        let check = validate_placement(&world, &reg, &plan, (0, 60, 0), 0, &[]);
        assert_eq!(check, PlacementCheck::ChunksMissing);
    }

    #[test]
    fn placement_check_messages_are_non_empty_for_all_refusals() {
        for c in [
            PlacementCheck::NotFlat,
            PlacementCheck::VolumeOccupied,
            PlacementCheck::PlayerInside,
            PlacementCheck::ChunksMissing,
        ] {
            assert!(!placement_check_message(c).is_empty());
        }
        assert_eq!(placement_check_message(PlacementCheck::Valid), "");
    }

    // ─── Task 17 — blocked-placement toast rate-limit ───────────────

    #[test]
    fn toast_fires_on_first_blocked_frame_with_no_attempt() {
        // Reason "changed" from None (nothing shown yet) — fires even
        // without a click, so scrubbing the ghost tells you why.
        let (shown, state) = placement_toast_decision(
            PlacementCheck::NotFlat,
            false,
            GhostToastState::default(),
        );
        assert_eq!(shown, Some(PlacementCheck::NotFlat));
        assert_eq!(state.last_shown, Some(PlacementCheck::NotFlat));
    }

    #[test]
    fn toast_does_not_repeat_every_frame_for_the_same_reason() {
        let (_, state) = placement_toast_decision(
            PlacementCheck::NotFlat,
            false,
            GhostToastState::default(),
        );
        // Same reason, no new attempt, many subsequent frames — silent.
        let mut state = state;
        for _ in 0..10 {
            let (shown, next) = placement_toast_decision(PlacementCheck::NotFlat, false, state);
            assert_eq!(shown, None, "must not spam every frame while the reason is unchanged");
            state = next;
        }
    }

    #[test]
    fn toast_refires_when_the_reason_changes() {
        let (_, state) = placement_toast_decision(
            PlacementCheck::NotFlat,
            false,
            GhostToastState::default(),
        );
        let (shown, _) = placement_toast_decision(PlacementCheck::VolumeOccupied, false, state);
        assert_eq!(shown, Some(PlacementCheck::VolumeOccupied));
    }

    #[test]
    fn toast_refires_on_explicit_attempt_even_for_the_same_reason() {
        let (_, state) = placement_toast_decision(
            PlacementCheck::NotFlat,
            false,
            GhostToastState::default(),
        );
        // Same reason as already shown, but the player explicitly tried
        // to place (rising edge) — re-explain why.
        let (shown, _) = placement_toast_decision(PlacementCheck::NotFlat, true, state);
        assert_eq!(shown, Some(PlacementCheck::NotFlat));
    }

    #[test]
    fn toast_attempt_is_edge_triggered_not_level_triggered() {
        // A held mouse button (attempt_down true for many consecutive
        // frames) must only fire once, on the rising edge — otherwise a
        // held click while blocked would spam a toast every tick.
        let (shown1, state) = placement_toast_decision(
            PlacementCheck::VolumeOccupied,
            true,
            GhostToastState::default(),
        );
        assert_eq!(shown1, Some(PlacementCheck::VolumeOccupied));
        let (shown2, state) = placement_toast_decision(PlacementCheck::VolumeOccupied, true, state);
        assert_eq!(shown2, None, "held button must not retrigger every frame");
        let (shown3, _) = placement_toast_decision(PlacementCheck::VolumeOccupied, true, state);
        assert_eq!(shown3, None);
    }

    #[test]
    fn toast_state_resets_when_placement_becomes_valid() {
        let (_, state) = placement_toast_decision(
            PlacementCheck::NotFlat,
            false,
            GhostToastState::default(),
        );
        let (shown, reset_state) = placement_toast_decision(PlacementCheck::Valid, false, state);
        assert_eq!(shown, None);
        assert_eq!(reset_state, GhostToastState::default());
        // Blocked again for the SAME reason as before the reset — should
        // fire again (not suppressed as "unchanged"), since Valid cleared it.
        let (shown_again, _) = placement_toast_decision(PlacementCheck::NotFlat, false, reset_state);
        assert_eq!(shown_again, Some(PlacementCheck::NotFlat));
    }

    // ─── Spec 24 Phase 11 — mode-aware material lock ────────────────

    fn inv_with_blocks(pairs: &[(BlockId, u32)]) -> crate::inventory::Inventory {
        let mut inv = crate::inventory::Inventory::new();
        for &(bid, n) in pairs {
            let mut remaining = n;
            while remaining > 0 {
                let take = remaining.min(64) as u8;
                let _ = inv.add_item(crate::item::ItemStack::new_block(bid, take));
                remaining -= take as u32;
            }
        }
        inv
    }

    #[test]
    fn lock_materials_for_mode_returns_empty_in_creative_without_touching_inventory() {
        let plan = PlanData::debug_3x3_stone();
        let mut inv = crate::inventory::Inventory::new();
        // Empty inventory — survival would fail; creative succeeds with no lock.
        let locked = lock_materials_for_mode("creative", &plan.cells, &mut inv).unwrap();
        assert!(locked.is_empty());
        // Inventory wasn't touched.
        assert!(inv.slot(0).is_none());
    }

    #[test]
    fn lock_materials_for_mode_decrements_inventory_in_survival() {
        let plan = PlanData::debug_3x3_stone();
        let mut inv = inv_with_blocks(&[(block::STONE, 12)]);
        let locked = lock_materials_for_mode("survival", &plan.cells, &mut inv).unwrap();
        // Snapshot has 9 stone (the plan's requirement).
        assert_eq!(locked.len(), 1);
        assert!(matches!(locked[0].item, crate::item::Item::Block(b) if b == block::STONE));
        assert_eq!(locked[0].count, 9);
        // Inventory has 3 stone remaining (12 - 9).
        let remaining: u32 = (0..36)
            .filter_map(|i| inv.slot(i))
            .filter_map(|s| match s.item {
                crate::item::Item::Block(b) if b == block::STONE => Some(s.count as u32),
                _ => None,
            })
            .sum();
        assert_eq!(remaining, 3);
    }

    #[test]
    fn lock_materials_for_mode_refuses_when_survival_inventory_short() {
        let plan = PlanData::debug_3x3_stone();
        let mut inv = inv_with_blocks(&[(block::STONE, 5)]);
        let res = lock_materials_for_mode("survival", &plan.cells, &mut inv);
        assert!(matches!(
            res,
            Err(BuildStartError::MissingMaterials(b, needed, owned))
                if b == block::STONE && needed == 9 && owned == 5
        ));
        // Inventory unchanged.
        let remaining: u32 = (0..36)
            .filter_map(|i| inv.slot(i))
            .filter_map(|s| match s.item {
                crate::item::Item::Block(b) if b == block::STONE => Some(s.count as u32),
                _ => None,
            })
            .sum();
        assert_eq!(remaining, 5, "no partial decrement on failure");
    }

    fn world_with_3x3_solid_at(min: (i32, i32, i32), max: (i32, i32, i32)) -> World {
        let mut w = World::new();
        for x in min.0..=max.0 {
            for z in min.2..=max.2 {
                w.set_block(x, min.1, z, block::DIRT);
            }
        }
        w
    }

    #[test]
    fn start_build_in_survival_creates_anchor_with_locked_materials() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = inv_with_blocks(&[(block::STONE, 20)]);
        let res = start_build(
            &mut world,
            &reg,
            "survival",
            plan,
            0,
            (0, 60, 0),
            &mut inv,
            &[(100, 100, 100)], // player not in build site
        );
        assert!(res.is_ok());
        let anchor = world.construction_anchors.get(&(0, 60, 0)).unwrap();
        assert!(!anchor.is_creative_build);
        assert_eq!(anchor.locked_materials.len(), 1);
        assert_eq!(anchor.locked_materials[0].count, 9);
    }

    #[test]
    fn start_build_in_creative_creates_anchor_without_locking_inventory() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = crate::inventory::Inventory::new(); // empty
        let res = start_build(
            &mut world,
            &reg,
            "creative",
            plan,
            0,
            (0, 60, 0),
            &mut inv,
            &[(100, 100, 100)],
        );
        assert!(res.is_ok());
        let anchor = world.construction_anchors.get(&(0, 60, 0)).unwrap();
        assert!(anchor.is_creative_build);
        assert!(anchor.locked_materials.is_empty());
    }

    #[test]
    fn build_stamped_blocks_are_flagged_player_placed() {
        // Spec 06 §2.2 — the auto-builder stamps a player's plan into the
        // world; those blocks are player-placed, so mining the built structure
        // earns no proof-of-play work (closes the build->mine farming loop).
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = inv_with_blocks(&[(block::STONE, 20)]);
        start_build(
            &mut world,
            &reg,
            "survival",
            plan,
            0,
            (0, 60, 0),
            &mut inv,
            &[(100, 100, 100)],
        )
        .unwrap();

        // Drive the builder to completion, collecting every stamped cell.
        let mut built = Vec::new();
        for _ in 0..200 {
            built.extend(tick_build(&mut world, (0, 60, 0)));
        }
        assert!(!built.is_empty(), "builder must place at least one block");
        for (x, y, z, _b) in &built {
            assert!(
                world.is_placed(*x, *y, *z),
                "built block at ({x},{y},{z}) must be flagged player-placed"
            );
        }
    }

    #[test]
    fn start_build_refuses_on_placement_failure() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = World::new(); // no chunks
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = inv_with_blocks(&[(block::STONE, 20)]);
        let res = start_build(
            &mut world,
            &reg,
            "survival",
            plan,
            0,
            (0, 60, 0),
            &mut inv,
            &[],
        );
        assert!(matches!(res, Err(BuildStartError::Placement(_))));
        // Inventory unchanged — start_build only locks AFTER placement passes.
        let remaining: u32 = (0..36)
            .filter_map(|i| inv.slot(i))
            .filter_map(|s| match s.item {
                crate::item::Item::Block(b) if b == block::STONE => Some(s.count as u32),
                _ => None,
            })
            .sum();
        assert_eq!(remaining, 20);
    }

    #[test]
    fn tick_build_consumes_locked_materials_in_survival() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = inv_with_blocks(&[(block::STONE, 9)]);
        start_build(&mut world, &reg, "survival", plan, 0, (0, 60, 0), &mut inv, &[(100, 100, 100)]).unwrap();
        // Tick once — 2 blocks placed.
        let placed = tick_build(&mut world, (0, 60, 0));
        assert_eq!(placed.len(), 2);
        let anchor = world.construction_anchors.get(&(0, 60, 0)).unwrap();
        assert_eq!(anchor.locked_materials[0].count, 7, "2 stone consumed");
    }

    #[test]
    fn tick_build_skips_material_consumption_in_creative() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = crate::inventory::Inventory::new();
        start_build(&mut world, &reg, "creative", plan, 0, (0, 60, 0), &mut inv, &[(100, 100, 100)]).unwrap();
        let placed = tick_build(&mut world, (0, 60, 0));
        assert_eq!(placed.len(), 2);
        let anchor = world.construction_anchors.get(&(0, 60, 0)).unwrap();
        // locked_materials stays empty; build progresses normally.
        assert!(anchor.locked_materials.is_empty());
    }

    #[test]
    fn tick_build_completes_after_ceil_n_over_2_ticks() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = inv_with_blocks(&[(block::STONE, 9)]);
        start_build(&mut world, &reg, "survival", plan, 0, (0, 60, 0), &mut inv, &[(100, 100, 100)]).unwrap();
        // 9 cells / 2 per tick = 5 ticks (last tick places 1).
        for _ in 0..5 {
            tick_build(&mut world, (0, 60, 0));
        }
        assert!(build_is_complete(&world, (0, 60, 0)));
    }

    #[test]
    fn cancel_build_refunds_remaining_locked_materials_in_survival() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        let mut inv = inv_with_blocks(&[(block::STONE, 9)]);
        start_build(&mut world, &reg, "survival", plan, 0, (0, 60, 0), &mut inv, &[(100, 100, 100)]).unwrap();
        tick_build(&mut world, (0, 60, 0)); // 2 placed, 7 remain locked
        cancel_build(&mut world, (0, 60, 0), &mut inv);
        // Anchor gone.
        assert!(world.construction_anchors.get(&(0, 60, 0)).is_none());
        // 7 stone refunded.
        let remaining: u32 = (0..36)
            .filter_map(|i| inv.slot(i))
            .filter_map(|s| match s.item {
                crate::item::Item::Block(b) if b == block::STONE => Some(s.count as u32),
                _ => None,
            })
            .sum();
        assert_eq!(remaining, 7);
    }

    #[test]
    fn short_npub_truncates_by_chars_not_bytes() {
        assert_eq!(short_npub("npub1qqqqqqqqqq", 12), "npub1qqqqqqq");
        // A mixed-width string where byte 12 falls mid-char: the old
        // `&s[..s.len().min(12)]` byte-slice panicked here; chars().take is safe.
        let mixed = format!("{}{}", "a".repeat(5), "あ".repeat(10)); // 5 + 30 bytes
        let out = short_npub(&mixed, 12);
        assert_eq!(out.chars().count(), 12, "12 chars kept, no mid-char panic");
    }

    #[test]
    fn lock_materials_does_not_truncate_count_above_255() {
        // A survival plan needing 300 of one block must lock all 300. The old
        // `needed as u8` truncated to 300 % 256 = 44, so a cancel under-refunded
        // by 256 (engine audit 2026-06-04, A: plan material loss).
        let cells: Vec<CapturedCell> = (0..300u32)
            .map(|i| cell((i % 16) as u8, (i / 16) as u8, 0, block::STONE))
            .collect();
        let mut inv = crate::inventory::Inventory::new();
        for s in 0..5 {
            inv.set_slot(s, Some(crate::item::ItemStack::new_block(block::STONE, 60))); // 5×60 = 300
        }
        let locked = lock_materials_for_mode("survival", &cells, &mut inv).unwrap();
        let total: u32 = locked.iter().map(|s| s.count as u32).sum();
        assert_eq!(total, 300, "all 300 locked, not truncated to 44");
        let left: u32 = inv.slots_iter().flatten()
            .filter(|s| matches!(s.item, crate::item::Item::Block(b) if b == block::STONE))
            .map(|s| s.count as u32).sum();
        assert_eq!(left, 0, "all 300 consumed from the inventory");
    }

    #[test]
    fn complete_build_places_plaque_at_centre_with_authored_in() {
        let reg = crate::block::BlockRegistry::new();
        let mut world = world_with_3x3_solid_at((0, 60, 0), (2, 60, 2));
        let mut plan = PlanData::debug_3x3_stone();
        plan.width = 3;
        plan.depth = 3;
        plan.authored_in = "creative".to_string();
        let mut inv = inv_with_blocks(&[(block::STONE, 9)]);
        start_build(&mut world, &reg, "creative", plan, 0, (0, 60, 0), &mut inv, &[(100, 100, 100)]).unwrap();
        // Run all ticks.
        for _ in 0..5 {
            tick_build(&mut world, (0, 60, 0));
        }
        let plaque_pos = complete_build(&mut world, (0, 60, 0)).expect("plaque placed");
        // 3×3 footprint → centre = (anchor.x + 1, anchor.y + 1, anchor.z + 1) = (1, 61, 1).
        assert_eq!(plaque_pos, (1, 61, 1));
        assert_eq!(world.get_block(1, 61, 1), block::ARCHITECT_PLAQUE);
        let plaque_data = world.architect_plaques.get(&plaque_pos).unwrap();
        assert_eq!(plaque_data.authored_in, "creative");
        assert!(!plaque_data.chain.is_empty());
    }

    #[test]
    fn architect_plaque_data_bincode_round_trips() {
        let plaque = ArchitectPlaqueData {
            chain: vec![DerivationLink {
                author_npub: "abc".to_string(),
                plan_name: "Tavern".to_string(),
                license: PlanLicense::Ccbysa,
                captured_at: 42,
                plan_hash: [7; 32],
            }],
            authored_in: "creative".to_string(),
            builder_credit: None,
        };
        let bytes = bincode::serialize(&plaque).unwrap();
        let back: ArchitectPlaqueData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.authored_in, "creative");
        assert_eq!(back.chain.len(), 1);
        assert_eq!(back.chain[0].plan_name, "Tavern");
    }

    #[test]
    fn rotate_cells_zero_rotations_is_identity() {
        let cells = vec![cell(0, 0, 0, block::STONE), cell(1, 0, 0, block::DIRT)];
        let out = rotate_cells(&cells, 0, 2, 1);
        assert_eq!(out, cells);
    }

    #[test]
    fn rotate_cells_360_returns_to_origin() {
        // 4 rotations = identity. The `n % 4 == 0` branch short-circuits
        // to a clone-of-input; this regression-guards that the
        // short-circuit covers `rotations = 4` (and 8, 12, ...) as well
        // as 0.
        let cells = vec![cell(0, 0, 0, block::STONE), cell(2, 1, 1, block::DIRT)];
        let out = rotate_cells(&cells, 4, 3, 2);
        assert_eq!(out, cells);
        let out8 = rotate_cells(&cells, 8, 3, 2);
        assert_eq!(out8, cells);
    }

    #[test]
    fn rotate_cells_quarter_turn_then_back_round_trips() {
        // Forward 90° clockwise → backward 90° (= 270° CW) on the
        // post-rotation dimensions reaches the original. This guards
        // the rotation formula's invertibility — and exercises the
        // dimension-swap that 360 doesn't.
        let cells = vec![cell(0, 0, 0, block::STONE), cell(2, 1, 1, block::DIRT)];
        let rotated = rotate_cells(&cells, 1, 3, 2);
        // Post-rotation: W=2, D=3. Rotate by 3 (270° CW = -90°) to undo.
        let back = rotate_cells(&rotated, 3, 2, 3);
        let mut a = cells;
        let mut b = back;
        a.sort_by_key(|c| (c.rx, c.ry, c.rz));
        b.sort_by_key(|c| (c.rx, c.ry, c.rz));
        assert_eq!(a, b);
    }

    #[test]
    fn rotate_cells_180_inverts_both_axes() {
        let cells = vec![cell(0, 0, 0, block::STONE)];
        let out = rotate_cells(&cells, 2, 3, 2);
        // 180°: (0,0,0) → (W-1, 0, D-1) = (2, 0, 1).
        assert_eq!(out[0].rx, 2);
        assert_eq!(out[0].rz, 1);
    }

    #[test]
    fn order_cells_for_build_y_ascending() {
        let cells = vec![
            cell(0, 2, 0, block::STONE),
            cell(0, 0, 0, block::DIRT),
            cell(0, 1, 0, block::OAK_PLANKS),
        ];
        let ordered = order_cells_for_build(&cells);
        assert_eq!(ordered[0].ry, 0);
        assert_eq!(ordered[1].ry, 1);
        assert_eq!(ordered[2].ry, 2);
    }

    #[test]
    fn detect_parent_via_content_hash() {
        let parent = PlanData::debug_3x3_stone();
        let candidate = parent.clone();
        let result = detect_parent(&candidate, [(5, &parent)]);
        assert_eq!(result, Some((5, ParentMatchKind::ContentHash)));
    }

    #[test]
    fn detect_parent_via_50_percent_block_match() {
        let parent = PlanData::debug_3x3_stone();
        // Candidate shares 5 of 9 cells with parent (≥50%).
        let mut candidate = parent.clone();
        for i in 0..4 {
            candidate.cells[i].block_id = block::DIRT;
        }
        // Cells changed → hash differs.
        assert_ne!(content_hash(&candidate), content_hash(&parent));
        let result = detect_parent(&candidate, [(2, &parent)]);
        assert_eq!(result, Some((2, ParentMatchKind::BlockMatch50)));
    }

    #[test]
    fn detect_parent_returns_none_at_low_match() {
        let parent = PlanData::debug_3x3_stone();
        // Candidate is completely different — single cell, different block.
        let candidate = PlanData {
            version: 1,
            name: "Totally different".to_string(),
            author_npub: String::new(),
            license: PlanLicense::Ccbysa,
            derivation_chain: Vec::new(),
            is_master: true,
            width: 1, depth: 1, height: 1,
            cells: vec![cell(0, 0, 0, block::OAK_PLANKS)],
            authored_in: "survival".to_string(),
            develop_state: DevelopState::Developed,
            kind: PlanKind::Building,
            marker: None,
        };
        let result = detect_parent(&candidate, [(0, &parent)]);
        assert!(result.is_none());
    }

    #[test]
    fn detect_parent_skips_non_master_plans() {
        let mut parent = PlanData::debug_3x3_stone();
        parent.is_master = false;
        let candidate = parent.clone();
        let result = detect_parent(&candidate, [(0, &parent)]);
        assert!(result.is_none());
    }

    // ── Asymmetric-size regression (post-merge review #4, 2026-05-20) ─

    /// Helper: produce a PlanData with a row of stone cells of the
    /// given length, all at y=0, z=0. Useful for asymmetric-size tests
    /// since the row dimensions are predictable.
    fn stone_row(width: u8) -> PlanData {
        let mut cells = Vec::new();
        for x in 0..width {
            cells.push(CapturedCell { rx: x, ry: 0, rz: 0, block_id: block::STONE });
        }
        PlanData {
            version: 1,
            name: format!("row-{width}"),
            author_npub: String::new(),
            license: PlanLicense::Ccbysa,
            derivation_chain: Vec::new(),
            is_master: true,
            width, depth: 1, height: 1,
            cells,
            authored_in: "survival".to_string(),
            develop_state: DevelopState::Developed,
            kind: PlanKind::Building,
            marker: None,
        }
    }

    #[test]
    fn detect_parent_small_candidate_subset_of_large_parent_fires_match() {
        // Subset relationship: candidate's 3 cells all exist inside
        // parent's 20-cell row at matching (rx, ry, rz, block). All
        // 3 candidate cells are in parent → matched=3, total=3,
        // 3*2 ≥ 3 → BlockMatch50 fires. Locks the "Save-As of a
        // sub-section is detected as a derivative" intent.
        let parent = stone_row(20);
        let candidate = stone_row(3);
        let result = detect_parent(&candidate, [(7, &parent)]);
        assert_eq!(result, Some((7, ParentMatchKind::BlockMatch50)));
    }

    #[test]
    fn detect_parent_large_candidate_against_small_parent_below_threshold() {
        // Asymmetric reverse: candidate is 20 cells, parent is 3
        // cells. All 3 parent cells exist in candidate → matched=3,
        // total=20, 3*2=6 < 20 → no match. Locks the "small parent
        // doesn't spuriously claim a big candidate" direction.
        let parent = stone_row(3);
        let candidate = stone_row(20);
        let result = detect_parent(&candidate, [(0, &parent)]);
        assert!(result.is_none(),
            "small parent must not spuriously match a large candidate; got {result:?}");
    }

    #[test]
    fn detect_parent_partial_overlap_below_threshold_does_not_fire() {
        // 9-cell candidate. Parent shares only the first 3 positions.
        // matched=3, total=9, 3*2=6 < 9 → no match.
        let parent = stone_row(3);
        let mut candidate = stone_row(9);
        // Drift candidate so only first 3 overlap (block id flip on
        // last 6 to be explicit about partial mismatch).
        for c in candidate.cells.iter_mut().skip(3) {
            c.block_id = block::DIRT;
        }
        let result = detect_parent(&candidate, [(0, &parent)]);
        assert!(result.is_none(),
            "3/9 overlap is below 50% — must not fire; got {result:?}");
    }

    #[test]
    fn detect_parent_exact_50_percent_overlap_fires() {
        // Edge case: matched=5, total=10 → 5*2=10 ≥ 10 → fires.
        // Locks the ≥ (not >) threshold.
        let parent = stone_row(5);
        let mut candidate = stone_row(10);
        for c in candidate.cells.iter_mut().skip(5) {
            c.block_id = block::DIRT;
        }
        let result = detect_parent(&candidate, [(2, &parent)]);
        assert_eq!(result, Some((2, ParentMatchKind::BlockMatch50)));
    }

    #[test]
    fn detect_parent_content_hash_short_circuits_first() {
        // Post-merge review #9: locks iteration order. When a
        // ContentHash match exists in the inventory, `detect_parent`
        // short-circuits and returns it — even if a later parent
        // would also BlockMatch50. The IntoIterator order is
        // deterministic per call; this test names that contract.
        let parent_exact = PlanData::debug_3x3_stone();
        let mut parent_partial = parent_exact.clone();
        // Make the partial parent visibly different (cells modified
        // so its content_hash differs), but keep 5/9 cells matching
        // so BlockMatch50 WOULD fire on it alone.
        for c in parent_partial.cells.iter_mut().take(4) {
            c.block_id = block::DIRT;
        }
        let candidate = parent_exact.clone();
        // Order: partial first, exact second. ContentHash from
        // `parent_exact` wins — slot index 2.
        let result = detect_parent(
            &candidate,
            [(1, &parent_partial), (2, &parent_exact)],
        );
        assert_eq!(result, Some((2, ParentMatchKind::ContentHash)));
    }

    #[test]
    fn detect_parent_multiple_parents_returns_last_block_match() {
        // When NO ContentHash match exists, `detect_parent` falls
        // through to BlockMatch50 and ends up with the LAST matching
        // parent in iteration order (the loop's `best = Some(...)`
        // updates on every match without breaking). Locks that
        // last-wins contract so a future refactor doesn't silently
        // change which slot is suggested.
        let parent_a = stone_row(9);
        let parent_b = stone_row(9);
        let candidate = stone_row(9);
        // Both parents match content-hash with the candidate
        // (identical stone_row(9)), so ContentHash fires on the
        // first one. To test BlockMatch50 fall-through we need a
        // candidate that DIFFERS from both parents but overlaps ≥50%.
        let mut diff_candidate = candidate.clone();
        diff_candidate.cells[8].block_id = block::DIRT; // drift one cell
        let result = detect_parent(
            &diff_candidate,
            [(7, &parent_a), (8, &parent_b)],
        );
        // ContentHash differs (candidate != either parent); but both
        // parents BlockMatch50 against candidate. Last-wins → slot 8.
        // (If this assert ever fails because the loop short-circuits
        // on first BlockMatch50, the contract has changed — update
        // both this test and the spec.)
        assert_eq!(result, Some((8, ParentMatchKind::BlockMatch50)));
    }

    // ── Spec 24 Phase 8: ghost preview helpers ─────────────────────────

    #[test]
    fn ghost_tint_green_when_valid_and_creative() {
        // Creative short-circuits the material check — even with
        // `materials_sufficient = false` the tint is still Green.
        assert_eq!(
            ghost_tint(PlacementCheck::Valid, false, true),
            GhostTint::Green,
        );
    }

    #[test]
    fn ghost_tint_green_when_valid_and_survival_sufficient() {
        assert_eq!(
            ghost_tint(PlacementCheck::Valid, true, false),
            GhostTint::Green,
        );
    }

    #[test]
    fn ghost_tint_yellow_when_valid_survival_short_materials() {
        assert_eq!(
            ghost_tint(PlacementCheck::Valid, false, false),
            GhostTint::Yellow,
        );
    }

    #[test]
    fn ghost_tint_red_when_invalid_regardless_of_materials() {
        // Every non-Valid placement check should map to Red, even when
        // materials are sufficient and creative is on.
        for check in [
            PlacementCheck::VolumeOccupied,
            PlacementCheck::NotFlat,
            PlacementCheck::PlayerInside,
            PlacementCheck::ChunksMissing,
        ] {
            assert_eq!(ghost_tint(check, true, true), GhostTint::Red);
            assert_eq!(ghost_tint(check, false, false), GhostTint::Red);
        }
    }

    #[test]
    fn ghost_tint_rgb_values_are_distinct() {
        let g = GhostTint::Green.rgb();
        let y = GhostTint::Yellow.rgb();
        let r = GhostTint::Red.rgb();
        assert_ne!(g, y);
        assert_ne!(g, r);
        assert_ne!(y, r);
        // Guard against accidental NaN / out-of-range — the line shader
        // expects sRGB values in [0,1].
        for tint in [g, y, r] {
            for ch in tint {
                assert!(ch >= 0.0 && ch <= 1.0, "channel {ch} out of [0,1]");
            }
        }
    }

    #[test]
    fn inventory_has_materials_true_when_empty_plan() {
        let inv = crate::inventory::Inventory::new();
        // No cells → no requirements → trivially sufficient.
        assert!(inventory_has_materials(&[], &inv));
    }

    #[test]
    fn inventory_has_materials_false_when_short() {
        let cells = vec![cell(0, 0, 0, block::STONE), cell(1, 0, 0, block::STONE)];
        let inv = crate::inventory::Inventory::new();
        // Empty inventory holds zero stone, but the plan needs 2 → short.
        assert!(!inventory_has_materials(&cells, &inv));
    }

    #[test]
    fn inventory_has_materials_true_when_sufficient() {
        let cells = vec![cell(0, 0, 0, block::STONE)];
        let mut inv = crate::inventory::Inventory::new();
        // One stone block in inventory satisfies the one-stone plan.
        let added = inv.add_item(crate::item::ItemStack::new_block(block::STONE, 4)).is_none();
        assert!(added);
        assert!(inventory_has_materials(&cells, &inv));
    }

    #[test]
    fn ghost_state_struct_is_cheap_to_copy() {
        // The struct is `Copy` so the per-frame ghost handler can lift
        // it out of PlayerSlot without cloning. Lock that in.
        fn assert_copy<T: Copy>() {}
        assert_copy::<GhostState>();
    }

    #[test]
    fn tick_build_places_two_cells_per_tick() {
        let mut world = World::new();
        let plan = PlanData::debug_3x3_stone();
        let total_cells = plan.cells.len();
        let anchor_pos = (0, 64, 0);
        world.construction_anchors.insert(
            anchor_pos,
            ConstructionAnchorData {
                plan,
                rotations: 0,
                anchor: anchor_pos,
                placed_index: 0,
                locked_materials: Vec::new(),
                is_creative_build: true,
                pace_divider: 1,
                pace_counter: 0,
                builder_credit: None,
            },
        );
        let placed = tick_build(&mut world, anchor_pos);
        assert_eq!(placed.len(), 2);
        // After one tick, two cells should be set in the world.
        assert_eq!(world.construction_anchors.get(&anchor_pos).unwrap().placed_index, 2);
        // Ensure those world positions actually contain the captured
        // block id (STONE for debug_3x3_stone).
        for (x, y, z, block_id) in placed {
            assert_eq!(world.get_block(x, y, z), block_id);
            assert_eq!(block_id, block::STONE);
        }
        // The build needs ceil(9/2) = 5 ticks to complete.
        let mut ticks = 1;
        while !build_is_complete(&world, anchor_pos) {
            tick_build(&mut world, anchor_pos);
            ticks += 1;
            assert!(ticks <= 10, "build should not run more than 10 ticks for 9 cells");
        }
        assert_eq!(
            world.construction_anchors.get(&anchor_pos).unwrap().placed_index,
            total_cells,
        );
    }

    #[test]
    fn tick_build_returns_empty_for_unknown_anchor() {
        let mut world = World::new();
        let placed = tick_build(&mut world, (99, 99, 99));
        assert!(placed.is_empty());
    }

    // Rubber feature (2026-05-23) — Eraser-driven Plan reset.

    #[test]
    fn wipe_captured_cells_resets_to_debug_footprint() {
        let mut p = PlanData::debug_3x3_stone();
        p.name = "Player's Cottage".to_string();
        // Stamp an extra cell to verify the wipe overrides.
        p.cells.push(CapturedCell {
            rx: 9, ry: 9, rz: 9, block_id: block::OAK_LOG,
        });
        let original_cell_count = PlanData::debug_3x3_stone().cells.len();
        p.wipe_captured_cells();
        assert_eq!(p.cells.len(), original_cell_count,
            "wipe should reset to debug-footprint cell count");
        // Player's chosen name is preserved across wipe.
        assert_eq!(p.name, "Player's Cottage");
    }

    // ─── Spec 38 (Blueprint / Cyanotype) — develop-state pure tests ──

    #[test]
    fn develop_state_default_is_developed() {
        // Legacy saves missing the field deserialise as Developed (the
        // conservative assumption — pre-Spec-38 captures were finished).
        assert_eq!(default_develop_state(), DevelopState::Developed);
    }

    #[test]
    fn develop_state_advance_sun_tick_latent_progress() {
        // One tick of direct sun adds 1 to exposure_ticks, no transition.
        let mut s = DevelopState::Latent { exposure_ticks: 0 };
        let transitioned = s.advance_sun_tick();
        assert!(!transitioned, "still well below threshold");
        assert_eq!(s, DevelopState::Latent { exposure_ticks: 1 });
    }

    #[test]
    fn develop_state_advance_sun_tick_threshold_flips_to_developed() {
        // One tick before the threshold → one more tick flips to
        // Developed and returns true.
        let mut s = DevelopState::Latent { exposure_ticks: DEVELOP_THRESHOLD_TICKS - 1 };
        let transitioned = s.advance_sun_tick();
        assert!(transitioned, "transition tick should signal true");
        assert_eq!(s, DevelopState::Developed);
    }

    #[test]
    fn develop_state_advance_sun_tick_no_op_when_developed() {
        // Developed never re-flips on sun ticks.
        let mut s = DevelopState::Developed;
        let transitioned = s.advance_sun_tick();
        assert!(!transitioned);
        assert_eq!(s, DevelopState::Developed);
    }

    #[test]
    fn develop_state_saturates_at_u16_max() {
        // saturating_add at u16::MAX should NOT panic; the threshold
        // guard then flips to Developed.
        let mut s = DevelopState::Latent { exposure_ticks: u16::MAX };
        let transitioned = s.advance_sun_tick();
        assert!(transitioned);
        assert_eq!(s, DevelopState::Developed);
    }

    #[test]
    fn develop_state_is_developed_helper() {
        assert!(DevelopState::Developed.is_developed());
        assert!(!DevelopState::Latent { exposure_ticks: 0 }.is_developed());
        assert!(!DevelopState::Latent { exposure_ticks: DEVELOP_THRESHOLD_TICKS - 1 }.is_developed());
    }

    // ─── Spec 38 multi-stage develop colour (2026-05-28) ─────────────

    #[test]
    fn develop_state_color_developed_is_prussian_blue() {
        // Final colour matches the blue-look slice from 2026-05-27.
        let c = develop_state_color(DevelopState::Developed);
        assert!((c[0] - 0.16).abs() < 1e-3);
        assert!((c[1] - 0.32).abs() < 1e-3);
        assert!((c[2] - 0.62).abs() < 1e-3);
    }

    #[test]
    fn develop_state_color_stage_0_is_pale_at_zero_exposure() {
        // Freshly Latent — exposure_ticks = 0 — falls into stage 0.
        let c = develop_state_color(DevelopState::Latent { exposure_ticks: 0 });
        assert!((c[0] - 0.78).abs() < 1e-3,
            "stage 0 should be pale yellow-green; got {c:?}");
    }

    #[test]
    fn develop_state_color_four_distinct_latent_stages() {
        // Sample each quarter of the 0..THRESHOLD range — must hit 4
        // distinct colours, plus Developed.
        let stage_quarter = DEVELOP_THRESHOLD_TICKS / 4;
        let s0 = develop_state_color(DevelopState::Latent {
            exposure_ticks: 0,
        });
        let s1 = develop_state_color(DevelopState::Latent {
            exposure_ticks: stage_quarter,
        });
        let s2 = develop_state_color(DevelopState::Latent {
            exposure_ticks: stage_quarter * 2,
        });
        let s3 = develop_state_color(DevelopState::Latent {
            exposure_ticks: stage_quarter * 3,
        });
        let dev = develop_state_color(DevelopState::Developed);
        // All five values must be distinct — the player can read them
        // off the hotbar icon as separate stages.
        let stages = [s0, s1, s2, s3, dev];
        for i in 0..stages.len() {
            for j in (i + 1)..stages.len() {
                assert_ne!(
                    stages[i], stages[j],
                    "stages {i} and {j} should not share a colour"
                );
            }
        }
    }

    #[test]
    fn develop_state_color_progresses_toward_blue() {
        // Pale yellow-green → deep blue: the red component should
        // strictly decrease across the 4 Latent stages (the green tint
        // fades, the blue darkens), and the blue component should
        // strictly increase.
        let quarter = DEVELOP_THRESHOLD_TICKS / 4;
        let mut prev = develop_state_color(DevelopState::Latent { exposure_ticks: 0 });
        for stage in 1..4 {
            let c = develop_state_color(DevelopState::Latent {
                exposure_ticks: stage * quarter,
            });
            assert!(c[0] <= prev[0],
                "red channel should fall or stay (stage {stage}): prev {prev:?} cur {c:?}");
            assert!(c[2] >= prev[2],
                "blue channel should rise or stay (stage {stage}): prev {prev:?} cur {c:?}");
            prev = c;
        }
    }

    #[test]
    fn develop_state_color_handles_over_threshold_exposure() {
        // Defensive — if exposure_ticks somehow exceeds the threshold
        // before the tick driver flips Latent → Developed (shouldn't
        // happen, but `saturating_mul` guards), the colour stays in
        // the deep-blue stage rather than panicking on overflow.
        let c = develop_state_color(DevelopState::Latent {
            exposure_ticks: u16::MAX,
        });
        // It's the deepest Latent stage (raw quotient saturates at >= 3).
        assert_eq!(c, [0.24, 0.40, 0.62]);
    }

    /// Lay a 1×1 tile, then put a STONE block directly on it so the
    /// connectivity-flood catches it as the captured build. Returns the
    /// world so callers can assert post-capture invariants.
    fn capture_1x1_build(authored_in: &str) -> (World, PlanCaptureCandidate) {
        let mut world = World::new();
        world.set_block(0, 70, 0, block::STONE);          // the tile cell = foundation
        world.set_face_attachment(                        // the laid blueprint
            (0, 70, 0),
            crate::mesh::Face::Top.index(),
            crate::world::FaceAttachment::BlueprintBlank,
        );
        world.set_block(0, 71, 0, block::STONE);          // build sits ON the paper
        let candidate = capture(&world, (0, 70, 0), authored_in)
            .expect("capture should succeed on 1×1 build-on-paper");
        (world, candidate)
    }

    #[test]
    fn capture_stamps_latent_zero_on_fresh_survival_plan() {
        // Survival captures land as Latent { exposure_ticks: 0 } and only
        // develop once the player re-lays the resulting Plan under sun.
        let (world, candidate) = capture_1x1_build("survival");
        assert_eq!(
            candidate.data.develop_state,
            DevelopState::Latent { exposure_ticks: 0 },
            "fresh survival capture must be Latent"
        );
        // And the build is untouched in-world (only the attachment is
        // consumed on commit).
        assert_eq!(world.get_block(0, 71, 0), block::STONE,
            "spec: the build is untouched on lift");
        assert_eq!(candidate.data.kind, PlanKind::Building,
            "horizontal capture path produces Building-kind plans");
    }

    #[test]
    fn capture_stamps_developed_on_fresh_creative_plan() {
        // R3b — creative captures are instantly Developed (usable
        // immediately, no sun needed).
        let (_world, candidate) = capture_1x1_build("creative");
        assert_eq!(
            candidate.data.develop_state,
            DevelopState::Developed,
            "fresh creative capture must be instantly Developed"
        );
    }

    // ─── Spec 38 R5 — Art capture (vertical wall slice) ──────────────

    #[test]
    fn capture_art_refuses_horizontal_face_normal() {
        // Vertical paper placement (face_normal = ±Y) is the building
        // path, not art. Refuse with NotFlat so the caller routes to
        // `capture` instead.
        let world = World::new();
        let r = capture_art(&world, (0, 70, 0), [0, 1, 0], "survival");
        assert_eq!(r.err(), Some(CaptureRefusal::NotFlat));
        let r = capture_art(&world, (0, 70, 0), [0, -1, 0], "survival");
        assert_eq!(r.err(), Some(CaptureRefusal::NotFlat));
    }

    #[test]
    fn capture_art_samples_single_block_wall() {
        // A lone STONE at the anchor — the simplest possible art.
        // Should produce a 1×1×1 Art-kind plan.
        let mut world = World::new();
        world.set_block(5, 70, 0, block::STONE);
        let candidate = capture_art(&world, (5, 70, 0), [1, 0, 0], "survival")
            .expect("single-block wall art should capture");
        assert_eq!(candidate.data.kind, PlanKind::Art);
        assert_eq!(candidate.data.width, 1);
        assert_eq!(candidate.data.height, 1);
        assert_eq!(candidate.data.depth, 1);
        assert_eq!(candidate.data.cells.len(), 1);
        assert_eq!(candidate.data.develop_state,
            DevelopState::Latent { exposure_ticks: 0 });
    }

    #[test]
    fn capture_art_floodfills_a_2d_rectangle() {
        // A 3-wide × 2-tall block of wall art (all STONE) at x=5
        // (face_normal = +X). Capture should return all 6 cells as
        // a 3×2 plan.
        let mut world = World::new();
        for dx in 0..3 {
            for dy in 0..2 {
                world.set_block(5, 70 + dy, dx, block::STONE);
            }
        }
        let candidate = capture_art(&world, (5, 70, 1), [1, 0, 0], "survival")
            .expect("3×2 wall art should capture");
        assert_eq!(candidate.data.width, 3);
        assert_eq!(candidate.data.height, 2);
        assert_eq!(candidate.data.depth, 1);
        assert_eq!(candidate.data.cells.len(), 6);
    }

    #[test]
    fn capture_art_refuses_empty_anchor() {
        // No block at anchor → nothing to capture.
        let world = World::new();
        let r = capture_art(&world, (5, 70, 0), [1, 0, 0], "survival");
        assert_eq!(r.err(), Some(CaptureRefusal::EmptyVolume));
    }

    #[test]
    fn capture_art_stays_on_the_wall_plane() {
        // Two STONE blocks: one on the wall plane at (5, 70, 0) and
        // one off-plane at (6, 70, 0). The flood must NOT reach the
        // off-plane block — Art capture is single-plane only.
        let mut world = World::new();
        world.set_block(5, 70, 0, block::STONE); // on plane (x=5)
        world.set_block(6, 70, 0, block::STONE); // off plane
        let candidate = capture_art(&world, (5, 70, 0), [1, 0, 0], "survival")
            .unwrap();
        assert_eq!(candidate.data.cells.len(), 1,
            "off-plane blocks must not flood into the wall slice");
    }

    #[test]
    fn capture_art_works_on_z_aligned_wall_too() {
        // Same test as the X-wall flood, but on a Z-aligned wall
        // (face_normal = ±Z). Locks the axis-handling symmetry.
        let mut world = World::new();
        for dx in 0..3 {
            for dy in 0..2 {
                world.set_block(dx, 70 + dy, 5, block::STONE);
            }
        }
        let candidate = capture_art(&world, (1, 70, 5), [0, 0, 1], "survival")
            .expect("3×2 Z-wall art should capture");
        assert_eq!(candidate.data.width, 3);
        assert_eq!(candidate.data.height, 2);
        assert_eq!(candidate.data.cells.len(), 6);
    }

    #[test]
    fn plan_kind_default_is_building_for_legacy_saves() {
        // Pre-Spec-38-R5 plans (without `kind` in the wire) load as
        // Building-kind via the serde default. Same shape as the
        // develop_state back-compat.
        assert_eq!(default_plan_kind(), PlanKind::Building);
    }

    #[test]
    fn content_hash_independent_of_plan_kind() {
        // A Building-kind plan and an Art-kind plan with identical
        // cells must share a content hash — `kind` is metadata, not
        // content. Mirrors the develop_state / authored_in independence.
        let mut building = PlanData::debug_3x3_stone();
        building.kind = PlanKind::Building;
        let mut art = PlanData::debug_3x3_stone();
        art.kind = PlanKind::Art;
        assert_eq!(content_hash(&building), content_hash(&art),
            "kind must be zeroed in content_hash");
    }

    #[test]
    fn plandata_serde_round_trips_latent_state() {
        // A Latent capture survives a serde round-trip with the same
        // exposure_ticks counter.
        let mut p = PlanData::debug_3x3_stone();
        p.develop_state = DevelopState::Latent { exposure_ticks: 1234 };
        let bytes = bincode::serialize(&p).expect("serialise");
        let back: PlanData = bincode::deserialize(&bytes).expect("deserialise");
        assert_eq!(back.develop_state, DevelopState::Latent { exposure_ticks: 1234 });
    }

    #[test]
    fn plandata_serde_round_trips_developed_state() {
        // Mirror to the Latent round-trip above — Developed must also
        // survive intact. Together these lock the serde wire shape for
        // both variants. NB: bincode v1 reads positionally so the
        // `#[serde(default = "default_develop_state")]` annotation does
        // NOT rescue pre-Spec-38 saves with EOF mid-PlanData — that's
        // an accepted save-format break (cf. fantasy-roster excision).
        // The annotation guards forward-compat for future formats with
        // self-describing field maps (JSON, CBOR) if save ever switches.
        let mut p = PlanData::debug_3x3_stone();
        p.develop_state = DevelopState::Developed;
        let bytes = bincode::serialize(&p).expect("serialise");
        let back: PlanData = bincode::deserialize(&bytes).expect("deserialise");
        assert_eq!(back.develop_state, DevelopState::Developed);
    }

    #[test]
    fn content_hash_independent_of_develop_state() {
        // A Latent plan and the same plan once Developed must share a
        // content hash — develop progress is process state, not content,
        // so derivation detection should not be tricked by it.
        let mut latent = PlanData::debug_3x3_stone();
        latent.develop_state = DevelopState::Latent { exposure_ticks: 42 };
        let mut developed = PlanData::debug_3x3_stone();
        developed.develop_state = DevelopState::Developed;
        assert_eq!(
            content_hash(&latent),
            content_hash(&developed),
            "Spec 38: develop_state must be zeroed in the content hash"
        );
    }
}
