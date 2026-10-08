//! The Workshop — persistent, multi-project authoring space (Spec 40, Phase B).
//!
//! The Workshop is a **saved world** the player enters from the Lobby to redesign
//! assets that already exist. The core gesture: place a real asset (a block, a
//! stack, or a mob mannequin), **pump it up with a bellows** to a comfortable
//! working size, edit it (Phase C paints it; Phase F reshapes it), then **put a
//! pin in it** — the pin deflates it back to size and commits the override.
//!
//! This module is the **persistent, multi-project workspace model**: each inflated
//! project is a real, saved object you can leave half-finished and return to
//! across sessions, with as many on the go as you like. Pinning is *finishing one*
//! on your own schedule — NOT a "complete it now" gate.
//!
//! Two states, and the difference is the whole point:
//! - **Parked WIP** — an un-pinned project, still inflated, half-edited. Lives in
//!   the Workshop world and persists with it (it's just world state).
//! - **Committed** — a pinned, finished redesign. Lives in the
//!   [`crate::override_registry::OverrideRegistry`] and applies to the player's game.
//!
//! Everything here is `#[serde(default)]`-friendly and rides the world save (the
//! `PlanData` forward-compat pattern), so a half-painted cow survives save/quit.

use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::mob::MobType;
use crate::override_registry::{AuthoredFaces, MobPartKey, OverrideRegistry};

/// The two authoring modes — both produce a global appearance override of an
/// existing asset. **Reskin leads** (Phase C); reshape is the final phase (F).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkshopMode {
    /// Mode A — re-paper the faces (reskin). Works on blocks AND mobs.
    Reskin,
    /// Mode B — rebuild the form as a microblock (reshape). Phase F.
    Reshape,
}

/// What existing asset a project redesigns. Every Workshop output in v1 is bound
/// to an asset that already exists — there are no new item identities (parked).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkshopTarget {
    /// A block id (e.g. the flower, the stone).
    Block(BlockId),
    /// A mob type — placed as a still, posable "mannequin".
    Mob(MobType),
    /// The player's own avatar — blown up to paint a skin (Reskin only). Only
    /// ever attached to a transient blow-up; never serialized in a WorldSave
    /// (the mannequin is render-only until blown up, and a blown-up project
    /// collapses/removes itself or pins-without-marking, so no saved project can
    /// carry `Avatar`).
    Avatar,
}

/// Folder name of the player's (single, per-device) Workshop world. The Workshop
/// is a saved world like any other, but with a fixed well-known folder so the
/// Lobby entry always opens the same one.
pub const WORKSHOP_FOLDER: &str = "the_workshop";

/// Display name shown for the Workshop world.
pub const WORKSHOP_DISPLAY_NAME: &str = "The Workshop";

/// The Workshop's floor height (world Y of the platform's top solid layer). One
/// below the default spawn (y = 80, see `reset_for_world_change`) so the player
/// drops straight onto the floor on entry.
pub const WORKSHOP_FLOOR_Y: i32 = 79;

/// The `WorldMeta.world_type` string that names a Workshop-shaped world, so a
/// `ScenarioDef` can ask for one (`"world_type": "workshop"` → the launcher also
/// calls `WorldMeta::mark_workshop`, which is what the void floor, the avatar
/// mannequin and the starter Bellows actually key off).
pub const WORLD_TYPE: &str = "workshop";

// ── Avatar mannequin (Task 1 — skin-paint-in-Workshop) ──────────────────────

/// Where the avatar mannequin stands in the Workshop (feet on the SAND floor,
/// a few blocks in front of the spawn at [0.5, 80, 0.5]). **Playtest knob.**
pub const WORKSHOP_MANNEQUIN_POS: [f32; 3] = [0.5, WORKSHOP_FLOOR_Y as f32 + 1.0, -3.5];
/// Model yaw so the mannequin's FRONT (−Z face) points toward the spawn (+Z).
/// `avatar_front_dir(π/2) == (0, 1)`. **Playtest knob.**
pub const WORKSHOP_MANNEQUIN_YAW: f32 = std::f32::consts::FRAC_PI_2;
/// Uniform scale the avatar inflates to when fully blown up (~4 blocks tall →
/// ~7.4, so a 64-px skin gives ~0.1-block texels you can aim at while flying).
/// **Playtest knob.**
pub const AVATAR_BLOW_UP_SCALE: f32 = 4.0;

// The old hand-typed `AVATAR_AABB_MIN`/`MAX` constants (2026-06-18) only
// covered the limbs-TOGETHER pose, so a separated mannequin (R) — arms to
// x=±0.75, head top to y≈2.075 — was under-covered: the paint ray (real
// per-part boxes via `skin_hit::ray_hit_avatar`) stayed accurate, but the
// Bellows aim highlight / nearest-vs-block compare could miss a separated
// limb. Replaced 2026-09-03 by `skin_pose::avatar_aabb`, which derives the
// union box from the SAME tables `ray_hit_avatar` uses, at the CURRENT
// separation — see `pick_avatar_mannequin`.

/// Transform a WORLD ray into the avatar's fixed model space (feet at origin,
/// front −Z), inverting exactly what the renderer applies in
/// `build_player_avatar_vertices`: translate by −`pos`, inverse-rotate about Y by
/// `−(yaw + π/2)` (the body is rotated by `yaw + π/2`, see `avatar_front_dir`),
/// then unscale by the uniform blow-up `scale`. The param `t` is preserved
/// (origin AND dir are both unscaled), so nearest-hit ordering is world-correct.
/// Pure so the highest-risk transform is unit-tested without a GPU.
///
/// The FORWARD half lives in [`crate::skin_grid::avatar_model_to_world`] — the
/// mannequin's vertex builder and the paint aids (texel grid + hover footprint)
/// both go through it, so there is one transform and one inverse, not several
/// hand-copied ones. `skin_grid`'s round-trip tests hold the pair honest.
pub fn world_ray_to_avatar_model(
    eye: [f32; 3],
    dir: [f32; 3],
    pos: [f32; 3],
    yaw: f32,
    scale: f32,
) -> ([f32; 3], [f32; 3]) {
    let theta = yaw + std::f32::consts::FRAC_PI_2;
    let (c, s) = (theta.cos(), theta.sin());
    // Renderer rotation is R_y(theta): x' = x·c − z·s ; z' = x·s + z·c.
    // Inverse (rotate by −theta):     x = x'·c + z'·s ; z = −x'·s + z'·c.
    let px = eye[0] - pos[0];
    let pz = eye[2] - pos[2];
    let ox = (px * c + pz * s) / scale;
    let oy = (eye[1] - pos[1]) / scale;
    let oz = (-px * s + pz * c) / scale;
    let dx = (dir[0] * c + dir[2] * s) / scale;
    let dy = dir[1] / scale;
    let dz = (-dir[0] * s + dir[2] * c) / scale;
    ([ox, oy, oz], [dx, dy, dz])
}

/// Entry distance (world `t`) where a ray hits the avatar mannequin's bounding
/// box, or `None`. Used for Bellows aim (highlight + nearest-vs-block compare).
/// Reuses `world_ray_to_avatar_model` then a slab test on the union AABB from
/// `skin_pose::avatar_aabb` at the CURRENT limbs-apart `separation` (0.0..=1.0,
/// the same eased factor the paint ray-test and renderer use — see
/// `skin_pose` module docs) and the Workshop edit inflate, so the highlight
/// never under-covers a separated (R) mannequin the way a fixed box would.
pub fn pick_avatar_mannequin(
    eye: [f32; 3],
    dir: [f32; 3],
    pos: [f32; 3],
    yaw: f32,
    scale: f32,
    separation: f32,
) -> Option<f32> {
    let (o, d) = world_ray_to_avatar_model(eye, dir, pos, yaw, scale);
    let (aabb_min, aabb_max) =
        crate::skin_pose::avatar_aabb(separation, crate::skin_pose::EDIT_INFLATE);
    let mut t_enter = f32::NEG_INFINITY;
    let mut t_exit = f32::INFINITY;
    for a in 0..3 {
        if d[a].abs() < 1e-9 {
            if o[a] < aabb_min[a] || o[a] > aabb_max[a] {
                return None;
            }
        } else {
            let inv = 1.0 / d[a];
            let mut t1 = (aabb_min[a] - o[a]) * inv;
            let mut t2 = (aabb_max[a] - o[a]) * inv;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_enter = t_enter.max(t1);
            t_exit = t_exit.min(t2);
            if t_enter > t_exit {
                return None;
            }
        }
    }
    if t_exit < 0.0 {
        return None;
    }
    if t_enter >= 0.0 { Some(t_enter) } else { None }
}

// ── End avatar mannequin consts ──────────────────────────────────────────────

/// Inflation levels: 0 = deflated (normal size), up to [`MAX_INFLATION`] at full
/// working size. Each pump of the bellows steps it up one; a valve steps it back.
pub const MAX_INFLATION: u8 = 6;

/// Per-step growth of the working scale. Working scale = `1.0 + inflation * STEP`,
/// so a flower at inflation 6 is 4× size — "blow up like a photo/bubble, never
/// explode". The exact value is a *feel* knob the Axolittle playtest tunes.
pub const INFLATION_STEP: f32 = 0.5;

/// The visual working scale for an inflation level. Clamped to [`MAX_INFLATION`].
pub fn inflation_scale(inflation: u8) -> f32 {
    1.0 + inflation.min(MAX_INFLATION) as f32 * INFLATION_STEP
}

/// Edge length of the ×4 blow-up working volume (4 blocks per side).
pub const BLOW_UP_SPAN: i32 = 4;

/// The minimum (−X,−Y,−Z) corner of the ×4 blow-up cage for `block`, given the
/// player's horizontal position `player_xz = [x, z]`. The aimed block is the
/// cage's **near-bottom corner**: the cage grows UP (`block.y` is the cage floor)
/// and **horizontally away from the player** (the 4×4 footprint sits on the far
/// side of the block).
pub fn blow_up_cage_min(block: [i32; 3], player_xz: [f32; 2]) -> [i32; 3] {
    let cx = block[0] as f32 + 0.5;
    let cz = block[2] as f32 + 0.5;
    let min_x = if player_xz[0] < cx { block[0] } else { block[0] - (BLOW_UP_SPAN - 1) };
    let min_z = if player_xz[1] < cz { block[2] } else { block[2] - (BLOW_UP_SPAN - 1) };
    [min_x, block[1], min_z]
}

/// The 64 cell positions of a blow-up cage given its min corner.
pub fn blow_up_cage_cells(min: [i32; 3]) -> Vec<(i32, i32, i32)> {
    let mut cells = Vec::with_capacity((BLOW_UP_SPAN * BLOW_UP_SPAN * BLOW_UP_SPAN) as usize);
    for dx in 0..BLOW_UP_SPAN {
        for dy in 0..BLOW_UP_SPAN {
            for dz in 0..BLOW_UP_SPAN {
                cells.push((min[0] + dx, min[1] + dy, min[2] + dz));
            }
        }
    }
    cells
}

/// `true` if the cage is clear to blow up: every cell **except the source
/// `block`** is `AIR`. (The source block is the corner the balloon grows from,
/// so it is allowed to be solid.)
///
/// Precondition: `min` must come from [`blow_up_cage_min`] called with the same
/// `block`, so `block` is one of the cage's corner cells. The source-cell
/// exemption is a no-op if `block` lies outside the cage.
pub fn blow_up_cage_clear(world: &crate::world::World, min: [i32; 3], block: [i32; 3]) -> bool {
    for (x, y, z) in blow_up_cage_cells(min) {
        if [x, y, z] == block {
            continue;
        }
        if world.get_block(x, y, z) != crate::block::AIR {
            return false;
        }
    }
    true
}

// ── Spec 40 blow-up Phase 2 — hold-to-charge fixed-×4 ─────────────────────

/// Ticks of held charge (at 20 TPS) to reach full ×4. ~0.9 s. **Playtest knob.**
pub const BLOW_UP_CHARGE_TICKS: u32 = 18;
/// Ticks for the collapse-on-release slide back to ×1. ~0.35 s. **Playtest knob.**
pub const BLOW_UP_COLLAPSE_TICKS: u32 = 7;

/// Eased visual scale (1.0..=4.0) for a charge fraction `frac` (0..=1), as three
/// swells (×1→×2→×3→×4), each easing out and hard-stopping at its third — the
/// "fill… settle… fill" feel. ×1 and ×4 are the only flat points; never overshoots.
pub fn blow_up_scale_for_charge(frac: f32) -> f32 {
    let c = frac.clamp(0.0, 1.0);
    if c >= 1.0 {
        return 4.0;
    }
    let seg = c * 3.0; // 0.0..3.0
    let i = seg.floor(); // third index 0,1,2
    let t = seg - i; // 0..1 within the third
    let eased = 1.0 - (1.0 - t) * (1.0 - t); // ease-out → settles at each third
    1.0 + i + eased // (1+i) .. (2+i)
}

/// The discrete inflation level (0/2/4/6 — the existing ×1/×2/×3/×4 ceiling) a
/// charge fraction has reached. Kept in sync on the project so a persisted locked
/// ×4 still renders via the existing `inflation_scale` fallback and Phase 3/4 see
/// a sane integer.
pub fn blow_up_inflation_for_charge(frac: f32) -> u8 {
    let c = frac.clamp(0.0, 1.0);
    if c >= 1.0 {
        6
    } else if c >= 2.0 / 3.0 {
        4
    } else if c >= 1.0 / 3.0 {
        2
    } else {
        0
    }
}

/// Lifecycle of a working-copy blow-up. ×1 and ×4 are the only stable states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlowUpPhase {
    /// Holding the bellows; `charge` climbs to [`BLOW_UP_CHARGE_TICKS`].
    Charging,
    /// Reached ×4 and locked; stays until pinned (Phase 4) or sneak-collapsed.
    Locked,
    /// Released before full (or sneak-collapsed): sliding back to ×1; removed at 0.
    Collapsing,
}

/// Transient blow-up animation state for the working copy being inflated. Not
/// saved (`#[serde(skip)]` on the project) — a mid-charge balloon doesn't persist.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlowUp {
    pub phase: BlowUpPhase,
    /// Charge in ticks (0..=BLOW_UP_CHARGE_TICKS) while Charging.
    pub charge: u32,
    /// Collapse ticks remaining while Collapsing.
    pub collapse_left: u32,
    /// Scale at the instant collapse began, for the slide-to-×1 lerp.
    pub collapse_from: f32,
    /// Cage min corner this balloon grows from (locked at charge start so it
    /// doesn't flip if the player moves mid-charge). Render anchor.
    pub corner: [i32; 3],
    /// Set by the input handler each tick the bellows is held on this block;
    /// consumed (reset) by [`WorkshopProjects::tick_blow_ups`].
    pub held_this_tick: bool,
}

impl BlowUp {
    /// A fresh charging balloon at ×1, growing from `corner`. Starts held.
    pub fn charging(corner: [i32; 3]) -> Self {
        Self {
            phase: BlowUpPhase::Charging,
            charge: 0,
            collapse_left: 0,
            collapse_from: 1.0,
            corner,
            held_this_tick: true,
        }
    }

    /// The current animated visual scale (1.0..=4.0).
    pub fn scale(&self) -> f32 {
        match self.phase {
            BlowUpPhase::Charging => {
                blow_up_scale_for_charge(self.charge as f32 / BLOW_UP_CHARGE_TICKS as f32)
            }
            BlowUpPhase::Locked => 4.0,
            BlowUpPhase::Collapsing => {
                let frac = (self.collapse_left as f32 / BLOW_UP_COLLAPSE_TICKS as f32).clamp(0.0, 1.0);
                1.0 + (self.collapse_from - 1.0) * frac
            }
        }
    }

    /// Begin collapsing from the current scale (release-before-full, or sneak on a
    /// lock). Idempotent — calling it again while already collapsing is a no-op.
    pub fn begin_collapse(&mut self) {
        if self.phase != BlowUpPhase::Collapsing {
            self.collapse_from = self.scale();
            self.collapse_left = BLOW_UP_COLLAPSE_TICKS;
            self.phase = BlowUpPhase::Collapsing;
        }
    }

    /// Advance one tick batch. `held` = bellows held on this block this tick.
    /// Returns `false` when the balloon has fully collapsed and the project should
    /// be removed.
    pub fn advance(&mut self, held: bool, ticks_run: u32) -> bool {
        match self.phase {
            BlowUpPhase::Charging => {
                if held {
                    self.charge = (self.charge + ticks_run).min(BLOW_UP_CHARGE_TICKS);
                    if self.charge >= BLOW_UP_CHARGE_TICKS {
                        self.phase = BlowUpPhase::Locked;
                    }
                } else {
                    self.begin_collapse();
                }
                true
            }
            BlowUpPhase::Locked => true, // stays until pin (Phase 4) or sneak-collapse
            BlowUpPhase::Collapsing => {
                self.collapse_left = self.collapse_left.saturating_sub(ticks_run);
                self.collapse_left > 0
            }
        }
    }
}

// ── Spec 40 blow-up Phase 3 — in-world dye editing ─────────────────────────

/// Cells per side of the working-copy grid. At ×4 the cage is 4 blocks per
/// side, so 16 cells = ¼-block cells — and one cell ↔ one 16×16 texel (the
/// spec's unification of the microblock grid and the texel grid).
pub const EDIT_GRID: usize = 16;

/// Flat index into a 16³ buffer — same layout as `micro_model::grid_idx` /
/// `chunk::index`: `x + z*16 + y*256`.
#[inline]
pub fn edit_index(x: usize, y: usize, z: usize) -> usize {
    x + z * EDIT_GRID + y * EDIT_GRID * EDIT_GRID
}

/// `true` if cell `c` is inside the 16³ grid. (Public so the pick's occupancy
/// closure can treat an un-edited balloon as fully solid.)
pub fn cell_in_grid(c: [i32; 3]) -> bool {
    (0..EDIT_GRID as i32).contains(&c[0])
        && (0..EDIT_GRID as i32).contains(&c[1])
        && (0..EDIT_GRID as i32).contains(&c[2])
}

/// The blown-up working copy's editable voxels: a 16³ grid where each cell is
/// either empty (`None`, carved away) or a coloured microblock (`Some(block_id)`
/// — a dye's WALLPAPER block, or the source block for un-recoloured cells). One
/// field carries BOTH colour and occupancy (the paint-with-blocks model). Pin
/// (Phase 4) bakes this: pure recolour → `AuthoredFaces`, any place/carve →
/// coloured micro-model. Transient (`#[serde(skip)]` on the project): a WIP
/// sculpt persists on pin only (spec Open Q #2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditBuffer {
    cells: Vec<Option<BlockId>>, // len EDIT_GRID³
    /// Set once any place/carve changes occupancy from the initial solid fill.
    shape_dirty: bool,
}

impl EditBuffer {
    /// A solid working copy: every cell filled with the source `block_id` (the
    /// ×1 block blown up to fill the ×4 cage). The start point for paint + sculpt.
    pub fn solid(block_id: BlockId) -> Self {
        Self {
            cells: vec![Some(block_id); EDIT_GRID * EDIT_GRID * EDIT_GRID],
            shape_dirty: false,
        }
    }

    /// Is cell `c` an occupied microblock?
    pub fn occupied(&self, c: [i32; 3]) -> bool {
        cell_in_grid(c)
            && self.cells[edit_index(c[0] as usize, c[1] as usize, c[2] as usize)].is_some()
    }

    /// The colour (block id) at cell `c`, if occupied.
    pub fn color_at(&self, c: [i32; 3]) -> Option<BlockId> {
        if !cell_in_grid(c) {
            return None;
        }
        self.cells[edit_index(c[0] as usize, c[1] as usize, c[2] as usize)]
    }

    /// Whether the shape was changed from the initial solid fill (place/carve).
    /// Drives Pin's AuthoredFaces-vs-micro-model choice (Phase 4).
    pub fn shape_changed(&self) -> bool {
        self.shape_dirty
    }

    /// Paint: recolour an occupied cell. No-op (returns `false`) on an empty or
    /// out-of-range cell. Not a shape change.
    pub fn paint(&mut self, c: [i32; 3], block_id: BlockId) -> bool {
        if !cell_in_grid(c) {
            return false;
        }
        let i = edit_index(c[0] as usize, c[1] as usize, c[2] as usize);
        if self.cells[i].is_some() {
            self.cells[i] = Some(block_id);
            true
        } else {
            false
        }
    }

    /// Place: fill an empty cell with a coloured microblock. No-op on an occupied
    /// or out-of-range cell (the cage is the hard boundary). Marks shape changed.
    pub fn place(&mut self, c: [i32; 3], block_id: BlockId) -> bool {
        if !cell_in_grid(c) {
            return false;
        }
        let i = edit_index(c[0] as usize, c[1] as usize, c[2] as usize);
        if self.cells[i].is_none() {
            self.cells[i] = Some(block_id);
            self.shape_dirty = true;
            true
        } else {
            false
        }
    }

    /// Carve: remove a microblock. No-op on an empty or out-of-range cell. Marks
    /// shape changed.
    pub fn carve(&mut self, c: [i32; 3]) -> bool {
        if !cell_in_grid(c) {
            return false;
        }
        let i = edit_index(c[0] as usize, c[1] as usize, c[2] as usize);
        if self.cells[i].is_some() {
            self.cells[i] = None;
            self.shape_dirty = true;
            true
        } else {
            false
        }
    }

    /// The occupied cells as a `MicroModelData` (scale 16) — the bake source for
    /// the live balloon render (Phase 3) and the coloured micro-model pin (Phase 4).
    pub fn to_micro_model(&self) -> crate::micro_model::MicroModelData {
        let mut voxels = Vec::new();
        for y in 0..EDIT_GRID {
            for z in 0..EDIT_GRID {
                for x in 0..EDIT_GRID {
                    if let Some(id) = self.cells[edit_index(x, y, z)] {
                        voxels.push(crate::micro_model::MicroVoxel {
                            mx: x as u8,
                            my: y as u8,
                            mz: z as u8,
                            block_id: id,
                        });
                    }
                }
            }
        }
        crate::micro_model::MicroModelData {
            version: crate::micro_model::MICRO_MODEL_VERSION,
            scale: crate::micro_model::MICRO_SCALE_16,
            voxels,
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        }
    }

    /// Bake the working copy's six cube surfaces into an [`crate::override_registry::AuthoredFaces`]
    /// — the pure-paint pin output (Phase 4). `rgba_of` maps a cell's `BlockId` to its
    /// representative RGBA (wallpaper colour, or the source block's average — see
    /// `texture_gen::wallpaper_rgba`). The four sides are **collapsed to one shared
    /// side image** (the engine's 3-native-surface block model: top, bottom, one side):
    /// the canonical East surface is sampled once and written to all of N/S/E/W, so a
    /// uniform reskin dedups to a single texture-array layer downstream.
    ///
    /// Face/texel orientation is internally consistent; the exact in-world rotation of
    /// a painted patch is a visual-polish detail (playtest look boundary).
    pub fn to_authored_faces(&self, rgba_of: impl Fn(BlockId) -> [u8; 4]) -> crate::override_registry::AuthoredFaces {
        use crate::override_registry::{AuthoredFaces, FACE_BYTES};
        // The face buffer is written as an EDIT_GRID×EDIT_GRID RGBA image; that's
        // exact only while it matches FACE_BYTES. If EDIT_GRID ever changes this
        // compile-time check fails loudly instead of writing out of bounds.
        const _: () = assert!(EDIT_GRID * EDIT_GRID * 4 == FACE_BYTES);
        const N: i32 = EDIT_GRID as i32;
        let at = |c: [i32; 3]| -> [u8; 4] {
            match self.color_at(c) {
                Some(id) => rgba_of(id),
                None => [0, 0, 0, 0],
            }
        };
        let face = |build: &dyn Fn(usize, usize) -> [u8; 4]| -> Vec<u8> {
            let mut out = vec![0u8; FACE_BYTES];
            for v in 0..EDIT_GRID {
                for u in 0..EDIT_GRID {
                    let px = build(u, v);
                    let i = (v * EDIT_GRID + u) * 4;
                    out[i..i + 4].copy_from_slice(&px);
                }
            }
            out
        };
        let top = face(&|u, v| at([u as i32, N - 1, v as i32]));
        let bottom = face(&|u, v| at([u as i32, 0, v as i32]));
        let side = face(&|u, v| at([N - 1, v as i32, u as i32]));
        AuthoredFaces::from_faces([
            top,
            bottom,
            side.clone(),
            side.clone(),
            side.clone(),
            side,
        ])
    }

    /// Decide the pin output path: pure recolour → [`PinKind::Paint`], any place/carve
    /// → [`PinKind::Shape`].
    pub fn pin_kind(&self) -> PinKind {
        if self.shape_changed() { PinKind::Shape } else { PinKind::Paint }
    }
}

/// Which override a Pin writes for a working copy (Phase 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinKind {
    /// Pure recolour → an `AuthoredFaces` texture override.
    Paint,
    /// Shape changed (place/carve) → a coloured micro-model shape override.
    Shape,
}

/// Mirror mode for edits on the blown-up working copy (spec §5). **All-sides is
/// the default** — it matches the native block model where the four sides share
/// one texture, so a uniform reskin is one stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EditSymmetry {
    /// Each stroke hits only the picked cell.
    Off,
    /// Mirror across the X midplane (left ↔ right).
    LeftRight,
    /// 4-fold: mirror to all four sides (90° rotations about the vertical axis).
    #[default]
    AllSides,
}

impl EditSymmetry {
    /// Cycle Off → LeftRight → AllSides → Off (the toggle key).
    pub fn next(self) -> Self {
        match self {
            EditSymmetry::Off => EditSymmetry::LeftRight,
            EditSymmetry::LeftRight => EditSymmetry::AllSides,
            EditSymmetry::AllSides => EditSymmetry::Off,
        }
    }

    /// A short label for the HUD toast.
    pub fn label(self) -> &'static str {
        match self {
            EditSymmetry::Off => "off",
            EditSymmetry::LeftRight => "left-right",
            EditSymmetry::AllSides => "all sides",
        }
    }
}

/// The cells a single edit at `c` touches under `sym` — deduplicated, so a cell
/// on a mirror axis isn't edited twice. Mirroring is about the 16³ grid centre
/// (`15 - i`); all-sides is the 4-fold rotation about the vertical (Y) axis in
/// the XZ plane: `(x,z) → (z, 15-x)`.
pub fn symmetric_cells(c: [i32; 3], sym: EditSymmetry) -> Vec<[i32; 3]> {
    let m = EDIT_GRID as i32 - 1; // 15
    let mut out = vec![c];
    match sym {
        EditSymmetry::Off => {}
        EditSymmetry::LeftRight => out.push([m - c[0], c[1], c[2]]),
        EditSymmetry::AllSides => {
            let (mut x, mut z) = (c[0], c[2]);
            for _ in 0..3 {
                let (nx, nz) = (z, m - x);
                out.push([nx, c[1], nz]);
                x = nx;
                z = nz;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Which editing verb the in-world blow-up editor applies on a click (Spec 40
/// §6, 2026-06-18). The room is in one mode at a time and you switch it
/// **deliberately** (the V key), so carving a textured block can never "just
/// happen" — the old implicit rule (empty hand / any tool carves) is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EditMode {
    /// Texture editing: a left-click paints the one aimed pixel with the held
    /// dye. Never changes shape — a textured block stays a texture. The default,
    /// so the safe, non-destructive verb is what you get without thinking.
    #[default]
    Paint,
    /// Sculpting: left-click carves a microblock away; a held dye + right-click
    /// adds a coloured microblock. This is the one-way texture→microblock switch
    /// (§7) — there is no path back to a flat texture.
    Sculpt,
}

impl EditMode {
    /// Flip Paint ↔ Sculpt (the toggle key).
    pub fn toggled(self) -> Self {
        match self {
            EditMode::Paint => EditMode::Sculpt,
            EditMode::Sculpt => EditMode::Paint,
        }
    }

    /// A short label for the HUD hint / toast.
    pub fn label(self) -> &'static str {
        match self {
            EditMode::Paint => "Paint",
            EditMode::Sculpt => "Sculpt",
        }
    }
}

/// Decide which edit verb a click maps to, from the room's [`EditMode`] and the
/// dye (if any) the player is holding. The single source of truth for the
/// in-world editor's paint-vs-sculpt rules (Spec 40 §6/§8, 2026-06-18):
///
/// - [`EditMode::Paint`]: only a held **dye** paints, only on `break`
///   (left-click), and only the one aimed pixel. `place` does nothing and there
///   is no carve — so a textured block stays a texture, and holding a *block*
///   (dye = `None`) can never stamp that block's texture onto the copy (§8).
/// - [`EditMode::Sculpt`]: `break` carves the aimed microblock; `place` with a
///   held dye adds a coloured microblock. Entered deliberately, so geometry
///   never changes by accident (§6).
///
/// `break` wins if both buttons are down the same tick.
pub fn resolve_edit_action(
    mode: EditMode,
    dye: Option<BlockId>,
    break_pressed: bool,
    place_pressed: bool,
) -> Option<EditOp> {
    match mode {
        EditMode::Paint => {
            if break_pressed {
                dye.map(EditOp::Paint)
            } else {
                None
            }
        }
        EditMode::Sculpt => {
            if break_pressed {
                Some(EditOp::Carve)
            } else if place_pressed {
                dye.map(EditOp::Place)
            } else {
                None
            }
        }
    }
}

/// One edit verb applied to the working copy under the crosshair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOp {
    /// Recolour the picked occupied cell.
    Paint(BlockId),
    /// Add a coloured microblock at the (empty) place cell.
    Place(BlockId),
    /// Remove the picked microblock.
    Carve,
}

impl EditBuffer {
    /// Apply `op` at `cell`, mirrored by `sym`. Returns whether anything changed.
    pub fn apply(&mut self, cell: [i32; 3], op: EditOp, sym: EditSymmetry) -> bool {
        let mut changed = false;
        for c in symmetric_cells(cell, sym) {
            changed |= match op {
                EditOp::Paint(id) => self.paint(c, id),
                EditOp::Place(id) => self.place(c, id),
                EditOp::Carve => self.carve(c),
            };
        }
        changed
    }
}

/// Lifecycle state of a single project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectState {
    /// Placed at normal size, not yet inflated.
    Placed,
    /// Inflated to a working size (`inflation > 0`). An inflated, un-pinned
    /// project that gets saved is a **parked WIP**.
    Inflated,
    /// Pinned — committed to the override registry and deflated. Terminal for the
    /// WIP; the authored appearance now lives in the registry.
    Pinned,
}

/// Mode-A paint-in-progress, persisted so a half-painted asset survives save/quit.
/// A block carries one [`AuthoredFaces`]; a mob carries per-part faces keyed by the
/// part index. `None` until the painter (Phase C) writes the first stroke.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum WorkshopPaint {
    /// Nothing painted yet.
    #[default]
    None,
    /// A block reskin in progress (6 faces).
    Block(AuthoredFaces),
    /// A mob reskin in progress: `(part_index, faces)` for each touched part.
    Mob(Vec<(u8, AuthoredFaces)>),
}

/// One project on the Workshop floor — a real, persistent object, not a throwaway
/// render overlay. Re-entering the Workshop restores each project exactly where it
/// was left (target, inflation, mode, partial paint).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkshopProject {
    /// Stable per-Workshop id.
    pub id: u32,
    /// The existing asset being redesigned.
    pub target: WorkshopTarget,
    /// Reskin (A) or reshape (B).
    pub mode: WorkshopMode,
    /// Where the project sits on the Workshop floor (block coords of its anchor).
    pub origin: [i32; 3],
    /// Current inflation level (0..=[`MAX_INFLATION`]).
    pub inflation: u8,
    /// Lifecycle state.
    pub state: ProjectState,
    /// Mode-A paint-in-progress (Phase C).
    #[serde(default)]
    pub paint: WorkshopPaint,
    /// Transient Phase-2 blow-up animation (not saved — a mid-charge balloon
    /// doesn't persist; a locked ×4 persists via `inflation`/`state`).
    #[serde(skip)]
    pub blow_up: Option<BlowUp>,
    /// Transient Phase-3 in-world edit buffer (16³ colour+occupancy). Created on
    /// the first edit of a locked ×4 copy; not saved (a WIP sculpt persists on
    /// pin only — spec Open Q #2).
    #[serde(skip)]
    pub edit: Option<EditBuffer>,
}

impl WorkshopProject {
    /// A freshly-placed project at normal size.
    pub fn new(id: u32, target: WorkshopTarget, mode: WorkshopMode, origin: [i32; 3]) -> Self {
        Self {
            id,
            target,
            mode,
            origin,
            inflation: 0,
            state: ProjectState::Placed,
            paint: WorkshopPaint::None,
            blow_up: None,
            edit: None,
        }
    }

    /// Pump the bellows once — inflate one step toward [`MAX_INFLATION`]. No-op on a
    /// pinned project or at max. Returns whether anything changed.
    pub fn pump(&mut self) -> bool {
        if self.state == ProjectState::Pinned || self.inflation >= MAX_INFLATION {
            return false;
        }
        self.inflation += 1;
        self.state = ProjectState::Inflated;
        true
    }

    /// Open the valve once — deflate one step. At 0 the project is back to
    /// [`ProjectState::Placed`]. No-op on a pinned project or already deflated.
    pub fn deflate(&mut self) -> bool {
        if self.state == ProjectState::Pinned || self.inflation == 0 {
            return false;
        }
        self.inflation -= 1;
        if self.inflation == 0 {
            self.state = ProjectState::Placed;
        }
        true
    }

    /// Put a pin in it — deflate fully and mark committed. The caller writes the
    /// authored appearance into the override registry (Phase C/F).
    pub fn pin(&mut self) {
        self.inflation = 0;
        self.state = ProjectState::Pinned;
    }

    /// The current visual working scale. No caller — `BlowUp::scale` (a
    /// different type, same name) is what the renderer actually uses.
    #[allow(dead_code)]
    pub fn scale(&self) -> f32 {
        inflation_scale(self.inflation)
    }

    /// A **parked WIP** — un-pinned, so it persists inflated and can be returned to.
    /// (Pinned projects are finished; their override lives in the registry.)
    pub fn is_parked(&self) -> bool {
        self.state != ProjectState::Pinned
    }
}

/// Edge length of the cube the reshape capture scans (Mode B). One micro-voxel ↔
/// one block, so a 16-block build bakes into a 16³ micro-model at native
/// resolution (`micro_model::MICRO_SCALE_16`).
pub const RESHAPE_BOX: i32 = 16;

/// Spec 40 Phase F (Mode B reshape) — capture the structure the player built inside
/// a [`RESHAPE_BOX`]³ cube whose minimum corner is `min`, as a [`crate::plan::PlanData`]
/// ready for `micro_model::MicroModelData::from_plan`. Relative cell coords map 1:1
/// to micro-voxels. Returns `None` if the cube is empty (nothing to reshape).
///
/// Because the scan is bounded to `RESHAPE_BOX` (= the micro scale), the result
/// always fits the exact-build envelope `from_plan` requires — it never refuses
/// `TooLarge`.
pub fn capture_box_as_plan(
    world: &crate::world::World,
    min: [i32; 3],
    size: i32,
) -> Option<crate::plan::PlanData> {
    use crate::plan::{CapturedCell, DevelopState, PlanData, PlanKind, PlanLicense};
    let mut cells = Vec::new();
    let (mut max_x, mut max_y, mut max_z) = (0u8, 0u8, 0u8);
    for dx in 0..size {
        for dy in 0..size {
            for dz in 0..size {
                let b = world.get_block(min[0] + dx, min[1] + dy, min[2] + dz);
                if b != crate::block::AIR {
                    let (rx, ry, rz) = (dx as u8, dy as u8, dz as u8);
                    max_x = max_x.max(rx);
                    max_y = max_y.max(ry);
                    max_z = max_z.max(rz);
                    cells.push(CapturedCell { rx, ry, rz, block_id: b });
                }
            }
        }
    }
    if cells.is_empty() {
        return None;
    }
    Some(PlanData {
        version: 1,
        name: "Workshop reshape".to_string(),
        author_npub: String::new(),
        license: PlanLicense::Ccbysa,
        derivation_chain: Vec::new(),
        is_master: true,
        // Axis convention (see micro_model::from_plan): width↔rx(x), height↔ry(y),
        // depth↔rz(z).
        width: max_x + 1,
        height: max_y + 1,
        depth: max_z + 1,
        cells,
        authored_in: "workshop".to_string(),
        develop_state: DevelopState::Developed,
        kind: PlanKind::Building,
        marker: None,
    })
}

/// Wrap a paint reskin in a fresh, unattributed [`NamedDesign`] for the wardrobe
/// (Phase 5). The library assigns the real id; the name is a short default the
/// Wardrobe panel can rename.
fn paint_named_design(faces: crate::override_registry::AuthoredFaces) -> crate::override_registry::NamedDesign {
    crate::override_registry::NamedDesign {
        id: 0,
        name: "design".to_string(),
        faces: Some(faces),
        micro_model: None,
        author_npub: String::new(),
        derivation_chain: Vec::new(),
    }
}

/// Commit a project's paint-in-progress into the override registry — the moment a
/// reskin stops being WIP and becomes the game's appearance for **every** instance
/// of that asset (Spec 40 Phase C). This is the pin → global-override step.
///
/// Returns `true` if an override was written. A project with no paint, or a Reshape
/// project (Mode B, committed via the micro-model path in Phase F), writes nothing
/// here. `base` is `texture_gen::texture_count()`.
pub fn commit_project_override(
    project: &WorkshopProject,
    overrides: &mut OverrideRegistry,
    base: u32,
) -> bool {
    match (&project.target, &project.paint) {
        (WorkshopTarget::Block(id), WorkshopPaint::Block(faces)) => {
            overrides.add_block_design(*id, paint_named_design(faces.clone()), base);
            true
        }
        (WorkshopTarget::Mob(mob), WorkshopPaint::Mob(parts)) => {
            let mut wrote = false;
            for (part_idx, faces) in parts {
                overrides.add_mob_design(
                    MobPartKey { mob: *mob, part: *part_idx },
                    paint_named_design(faces.clone()),
                    base,
                );
                wrote = true;
            }
            wrote
        }
        // No paint yet, or a target/paint mismatch (Reshape → Phase F microblock path).
        _ => false,
    }
}

/// The Workshop's multi-project container — the `workshop_projects` side-table that
/// rides the Workshop world's save. Holds **many concurrent projects**, each at its
/// own stage, so the author can "potter across days".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkshopProjects {
    #[serde(
        default,
        serialize_with = "serialize_persistent_projects",
        deserialize_with = "deserialize_persistent_projects"
    )]
    projects: Vec<WorkshopProject>,
    /// Monotonic id source so ids stay stable across a session (never reused).
    #[serde(default)]
    next_id: u32,
    /// The room's current edit-symmetry mode (Phase 3). Transient — a feel knob
    /// that resets to the default each session.
    #[serde(skip)]
    symmetry: EditSymmetry,
    /// The room's current Paint/Sculpt edit mode (§6, 2026-06-18). Transient —
    /// resets to the safe default (Paint) each session, so you never re-enter a
    /// world already in carving mode.
    #[serde(skip)]
    edit_mode: EditMode,
}

/// Serialize only the PERSISTENT projects: the transient `Avatar` working copy (a
/// Bellows blow-up of the player's own avatar) is never written to a save, so no
/// loaded `WorkshopProjects` can ever carry an `Avatar` target. The avatar
/// mannequin is render-only until blown up, and a blown-up Avatar project is a
/// `#[serde(skip)]` `BlowUp` over a runtime buffer — collapsing / pinning removes
/// it. This filter is the belt-and-braces guarantee for the edge case of an
/// autosave landing while it is still inflated. Wire format is unchanged (still a
/// `Vec<WorkshopProject>`, just without any Avatar entry).
fn serialize_persistent_projects<S>(
    projects: &[WorkshopProject],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let kept: Vec<&WorkshopProject> = projects
        .iter()
        .filter(|p| !matches!(p.target, WorkshopTarget::Avatar))
        .collect();
    kept.serialize(serializer)
}

/// Belt-and-braces load-side mirror of `serialize_persistent_projects`: drop any
/// `Avatar` project on deserialize. The engine never serializes one (the serialize
/// side filters it), but a hand-edited or foreign save could carry one, which
/// would load as an orphan — a locked Avatar project with no runtime working
/// buffer renders nothing, leaving a stuck invisible mannequin and a paint session
/// that can never open. Filtering on load guarantees no loaded `WorkshopProjects`
/// can ever hold an `Avatar` target.
fn deserialize_persistent_projects<'de, D>(
    deserializer: D,
) -> Result<Vec<WorkshopProject>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut projects = Vec::<WorkshopProject>::deserialize(deserializer)?;
    projects.retain(|p| !matches!(p.target, WorkshopTarget::Avatar));
    Ok(projects)
}

impl WorkshopProjects {
    pub fn new() -> Self {
        Self::default()
    }

    /// Place a new project on the floor; returns its id.
    pub fn add(&mut self, target: WorkshopTarget, mode: WorkshopMode, origin: [i32; 3]) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.projects
            .push(WorkshopProject::new(id, target, mode, origin));
        id
    }

    pub fn get(&self, id: u32) -> Option<&WorkshopProject> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut WorkshopProject> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    /// Remove a project (e.g. discarded). Returns it if present.
    pub fn remove(&mut self, id: u32) -> Option<WorkshopProject> {
        let idx = self.projects.iter().position(|p| p.id == id)?;
        Some(self.projects.remove(idx))
    }

    /// Drop ALL in-progress projects (Workshop "Reset" — wipe the room). The id
    /// allocator resets too, so a fresh room starts numbering at #1 again. Does
    /// NOT touch the committed override catalogue — that lives on the registry.
    pub fn clear(&mut self) {
        self.projects.clear();
        self.next_id = 0;
    }

    pub fn iter(&self) -> impl Iterator<Item = &WorkshopProject> {
        self.projects.iter()
    }

    /// All un-pinned projects — the parked WIP that persists inflated. No
    /// production caller — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn parked(&self) -> impl Iterator<Item = &WorkshopProject> {
        self.projects.iter().filter(|p| p.is_parked())
    }

    /// The id of the parked project whose anchor is nearest `pos` (the bellows
    /// targets it). `None` if there are no parked projects. No production
    /// caller — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn nearest_parked(&self, pos: [f32; 3]) -> Option<u32> {
        self.projects
            .iter()
            .filter(|p| p.is_parked())
            .min_by(|a, b| {
                let d = |p: &WorkshopProject| {
                    let dx = p.origin[0] as f32 - pos[0];
                    let dy = p.origin[1] as f32 - pos[1];
                    let dz = p.origin[2] as f32 - pos[2];
                    dx * dx + dy * dy + dz * dz
                };
                d(a).total_cmp(&d(b))
            })
            .map(|p| p.id)
    }

    /// The parked project whose origin is exactly `block`, if any.
    pub fn find_at_origin(&self, block: [i32; 3]) -> Option<u32> {
        self.projects
            .iter()
            .find(|p| p.origin == block && p.is_parked())
            .map(|p| p.id)
    }

    /// Origins of parked working copies that are currently blown up
    /// (charging / locked / collapsing). Spec 40 §5 — these world blocks are
    /// render-hidden so the original texture doesn't show through (or z-fight at
    /// the corner of) the inflated copy. Pinned projects are excluded: they're
    /// committed, and their block shows the reskinned appearance again.
    pub fn blown_up_origins(&self) -> Vec<[i32; 3]> {
        self.projects
            .iter()
            .filter(|p| p.is_parked() && p.blow_up.is_some())
            .map(|p| p.origin)
            .collect()
    }

    /// P-bugfix (collision): is `cell` inside the cage of a LOCKED blow-up — the
    /// committed ×4 working copy that fills its cage? Used to give that copy solid
    /// collision so the player can't walk through it. Cheap bounds check per
    /// locked project (there are only ever a handful in a room).
    pub fn locked_cage_contains(&self, cell: (i32, i32, i32)) -> bool {
        let (x, y, z) = cell;
        self.projects.iter().any(|p| {
            // The avatar painter is crosshair-raycast and never needs a collision
            // cage — its ×4 mannequin doesn't fill the block-sized cage, so a solid
            // cage would put an invisible wall around it that doesn't match the
            // humanoid silhouette (flying near it feels walled-in). Only block/mob
            // blow-ups get solid collision; the avatar is excluded.
            !matches!(p.target, WorkshopTarget::Avatar)
                && p.blow_up.as_ref().is_some_and(|b| {
                    b.phase == BlowUpPhase::Locked
                        && x >= b.corner[0]
                        && x < b.corner[0] + BLOW_UP_SPAN
                        && y >= b.corner[1]
                        && y < b.corner[1] + BLOW_UP_SPAN
                        && z >= b.corner[2]
                        && z < b.corner[2] + BLOW_UP_SPAN
                })
        })
    }

    /// Note the bellows is held, aimed at `block`, this tick. Creates a charging
    /// working copy (target = `block_id`) growing from `corner` if none exists;
    /// otherwise flags the existing one held so its charge continues. No-op on a
    /// collapsing balloon (release commits to ×1) and on a non-blow-up project
    /// (e.g. a `/ws place` mannequin).
    pub fn note_blow_up_aim(&mut self, block: [i32; 3], corner: [i32; 3], block_id: crate::block::BlockId) {
        if let Some(id) = self.find_at_origin(block) {
            if let Some(p) = self.get_mut(id)
                && let Some(b) = p.blow_up.as_mut()
                    && b.phase != BlowUpPhase::Collapsing {
                        b.held_this_tick = true;
                    }
        } else {
            let id = self.add(WorkshopTarget::Block(block_id), WorkshopMode::Reskin, block);
            if let Some(p) = self.get_mut(id) {
                p.blow_up = Some(BlowUp::charging(corner));
            }
        }
    }

    /// Note the Bellows is aimed at the avatar mannequin this tick. Creates a
    /// charging Avatar working copy (growing from the mannequin's floor cell) if
    /// none exists; otherwise flags the existing one held so its charge continues.
    /// Mirrors `note_blow_up_aim` for the avatar target (no source block id). The
    /// project's origin is the mannequin corner, so `collapse_blow_up_at` /
    /// `note_blow_up_release` (which key on origin) work on it unchanged.
    pub fn note_avatar_blow_up_aim(&mut self, corner: [i32; 3]) {
        if let Some(id) = self.find_avatar() {
            if let Some(p) = self.get_mut(id)
                && let Some(b) = p.blow_up.as_mut()
                    && b.phase != BlowUpPhase::Collapsing {
                        b.held_this_tick = true;
                    }
        } else {
            let id = self.add(WorkshopTarget::Avatar, WorkshopMode::Reskin, corner);
            if let Some(p) = self.get_mut(id) {
                p.blow_up = Some(BlowUp::charging(corner));
            }
        }
    }

    /// Create the Avatar working copy already LOCKED at full inflation, growing
    /// from `corner`. The one-shot used by "Your look → Edit/New" to drop the
    /// player straight into the painter without holding the Bellows. No-op if an
    /// Avatar project already exists. The origin is the mannequin corner, so the
    /// usual pin / `collapse_blow_up_at` paths (which key on origin) apply.
    pub fn note_avatar_blow_up_locked(&mut self, corner: [i32; 3]) {
        if self.find_avatar().is_some() {
            return;
        }
        let id = self.add(WorkshopTarget::Avatar, WorkshopMode::Reskin, corner);
        if let Some(p) = self.get_mut(id) {
            let mut b = BlowUp::charging(corner);
            b.phase = BlowUpPhase::Locked;
            b.charge = BLOW_UP_CHARGE_TICKS;
            p.blow_up = Some(b);
        }
    }

    /// The id of the (single) Avatar project, if one exists.
    fn find_avatar(&self) -> Option<u32> {
        self.projects
            .iter()
            .find(|p| matches!(p.target, WorkshopTarget::Avatar))
            .map(|p| p.id)
    }

    /// The id of the Avatar project iff it is LOCKED (paintable + pinnable).
    pub fn locked_avatar_project(&self) -> Option<u32> {
        self.projects
            .iter()
            .find(|p| {
                matches!(p.target, WorkshopTarget::Avatar)
                    && matches!(p.blow_up, Some(b) if b.phase == BlowUpPhase::Locked)
            })
            .map(|p| p.id)
    }

    /// Sneak-collapse the working copy at `block`, if one is blown up there.
    /// Returns whether anything was collapsed.
    pub fn collapse_blow_up_at(&mut self, block: [i32; 3]) -> bool {
        if let Some(id) = self.find_at_origin(block)
            && let Some(p) = self.get_mut(id)
                && let Some(b) = p.blow_up.as_mut() {
                    b.begin_collapse();
                    return true;
                }
        false
    }

    /// Advance every working-copy blow-up one tick batch: charge held ones,
    /// collapse released ones, remove finished ones, and keep `inflation`/`state`
    /// in sync. Call once per tick AFTER all players' input.
    pub fn tick_blow_ups(&mut self, ticks_run: u32) {
        let mut remove: Vec<u32> = Vec::new();
        for p in self.projects.iter_mut() {
            if let Some(b) = p.blow_up.as_mut() {
                let held = b.held_this_tick;
                let alive = b.advance(held, ticks_run);
                b.held_this_tick = false;
                p.inflation = match b.phase {
                    BlowUpPhase::Locked => MAX_INFLATION,
                    BlowUpPhase::Charging => {
                        blow_up_inflation_for_charge(b.charge as f32 / BLOW_UP_CHARGE_TICKS as f32)
                    }
                    BlowUpPhase::Collapsing => 0,
                };
                // Keep `state` invariant-consistent with `inflation` at all times
                // (Inflated ⟺ inflation > 0). A collapsing balloon drops to 0 and
                // reads back as Placed for the tick(s) before it's removed, so
                // Phase 3/4 callers that gate on `state` never see inflation 0 +
                // Inflated.
                p.state = if p.inflation > 0 {
                    ProjectState::Inflated
                } else {
                    ProjectState::Placed
                };
                if !alive {
                    remove.push(p.id);
                }
            }
        }
        for id in remove {
            self.remove(id);
        }
    }

    /// The room's current edit-symmetry mode. Unlike `edit_mode()` below
    /// (live, read by game_loop.rs), no UI reads this back yet.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn symmetry(&self) -> EditSymmetry {
        self.symmetry
    }

    /// Set the symmetry mode directly (tests / future UI).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_symmetry(&mut self, sym: EditSymmetry) {
        self.symmetry = sym;
    }

    /// Cycle the symmetry mode (the toggle key) and return the new value.
    pub fn cycle_symmetry(&mut self) -> EditSymmetry {
        self.symmetry = self.symmetry.next();
        self.symmetry
    }

    /// The room's current Paint/Sculpt edit mode (§6).
    pub fn edit_mode(&self) -> EditMode {
        self.edit_mode
    }

    /// Set the edit mode directly (tests / future UI). Unlike `set_symmetry`
    /// above, no test currently exercises this one either.
    #[allow(dead_code)]
    pub fn set_edit_mode(&mut self, mode: EditMode) {
        self.edit_mode = mode;
    }

    /// Toggle Paint ↔ Sculpt (the V key) and return the new value.
    pub fn toggle_edit_mode(&mut self) -> EditMode {
        self.edit_mode = self.edit_mode.toggled();
        self.edit_mode
    }

    /// The nearest locked-balloon cell under the crosshair, if any: `(project id,
    /// pick)`. Treats a not-yet-edited balloon as fully solid. Only `Locked`
    /// balloons are editable (charging/collapsing are mid-animation).
    pub fn pick_locked_cell(
        &self,
        eye: [f32; 3],
        dir: [f32; 3],
    ) -> Option<(u32, crate::raycast::CellPick)> {
        let mut best: Option<(u32, crate::raycast::CellPick)> = None;
        for p in self.projects.iter() {
            let Some(b) = p.blow_up else { continue };
            if b.phase != BlowUpPhase::Locked {
                continue;
            }
            let occ = |c: [i32; 3]| match &p.edit {
                Some(buf) => buf.occupied(c),
                None => cell_in_grid(c), // un-edited → fully solid
            };
            if let Some(pick) = crate::raycast::pick_cell_in_cage(eye, dir, b.corner, occ)
                && best.as_ref().is_none_or(|(_, bp)| pick.dist < bp.dist) {
                    best = Some((p.id, pick));
                }
        }
        best
    }

    /// The id of the single locked working copy in the room, iff EXACTLY one exists.
    /// (Used as the P-key pin fallback when the crosshair isn't on a balloon.)
    pub fn sole_locked_project(&self) -> Option<u32> {
        let mut found = None;
        for p in self.projects.iter() {
            if matches!(p.blow_up, Some(b) if b.phase == BlowUpPhase::Locked) {
                if found.is_some() {
                    return None; // more than one → ambiguous
                }
                found = Some(p.id);
            }
        }
        found
    }

    /// Whether any working copy is currently locked at ×4 (drives the HUD edit hint).
    pub fn any_locked(&self) -> bool {
        self.projects.iter().any(|p| matches!(p.blow_up, Some(b) if b.phase == BlowUpPhase::Locked))
    }

    /// Apply an edit to a locked project's buffer (lazily created, solid, from the
    /// project's block target), mirrored by the room symmetry. Returns whether
    /// anything changed. No-op on a non-block target (mob blow-up is out of v1).
    pub fn apply_edit(&mut self, id: u32, cell: [i32; 3], op: EditOp) -> bool {
        let sym = self.symmetry;
        let Some(p) = self.get_mut(id) else {
            return false;
        };
        let WorkshopTarget::Block(src) = p.target else {
            return false;
        };
        // Editing is gated to a LOCKED ×4 copy. A stale id (e.g. the balloon
        // started collapsing between the caller's pick and this frame) must not
        // create a buffer on a charging/collapsing project.
        if p.blow_up.is_none_or(|b| b.phase != BlowUpPhase::Locked) {
            return false;
        }
        let buf = p.edit.get_or_insert_with(|| EditBuffer::solid(src));
        buf.apply(cell, op, sym)
    }

    /// Read the colour at a cell of a project's edit buffer (eyedropper). `None`
    /// if there's no buffer yet or the cell is empty.
    pub fn eyedrop_at(&self, id: u32, cell: [i32; 3]) -> Option<BlockId> {
        self.get(id).and_then(|p| p.edit.as_ref()).and_then(|b| b.color_at(cell))
    }

    /// The bellows was released (button up) while still aimed at `block`. Clears
    /// the charging balloon's `held_this_tick` so the next tick collapses instead
    /// of stealing one stale charge tick (Phase-2 follow-up
    /// `project_workshop_blowup_phase2`). No-op on a locked/collapsing balloon.
    pub fn note_blow_up_release(&mut self, block: [i32; 3]) {
        if let Some(id) = self.find_at_origin(block)
            && let Some(p) = self.get_mut(id)
                && let Some(b) = p.blow_up.as_mut()
                    && b.phase == BlowUpPhase::Charging {
                        b.held_this_tick = false;
                    }
    }

    /// Whether an Avatar working copy currently exists (charging/locked/collapsing).
    /// Drives "hide the resting mannequin while blown up".
    pub fn has_avatar_project(&self) -> bool {
        self.find_avatar().is_some()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.projects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflation_scale_steps_and_clamps() {
        assert_eq!(inflation_scale(0), 1.0);
        assert!((inflation_scale(1) - 1.5).abs() < 1e-6);
        assert!((inflation_scale(MAX_INFLATION) - (1.0 + MAX_INFLATION as f32 * INFLATION_STEP)).abs() < 1e-6);
        // Above max clamps (never explodes past the working ceiling).
        assert_eq!(inflation_scale(250), inflation_scale(MAX_INFLATION));
    }

    #[test]
    fn pump_inflates_up_to_max_then_stops() {
        let mut p = WorkshopProject::new(0, WorkshopTarget::Block(5), WorkshopMode::Reskin, [0, 0, 0]);
        assert_eq!(p.state, ProjectState::Placed);
        for step in 1..=MAX_INFLATION {
            assert!(p.pump(), "pump {step} should inflate");
            assert_eq!(p.inflation, step);
            assert_eq!(p.state, ProjectState::Inflated);
        }
        assert!(!p.pump(), "pump past max is a no-op");
        assert_eq!(p.inflation, MAX_INFLATION);
    }

    #[test]
    fn valve_deflates_back_to_placed() {
        let mut p = WorkshopProject::new(1, WorkshopTarget::Block(5), WorkshopMode::Reskin, [0, 0, 0]);
        p.pump();
        p.pump();
        assert_eq!(p.inflation, 2);
        assert!(p.deflate());
        assert_eq!(p.inflation, 1);
        assert_eq!(p.state, ProjectState::Inflated);
        assert!(p.deflate());
        assert_eq!(p.inflation, 0);
        assert_eq!(p.state, ProjectState::Placed, "fully deflated returns to Placed");
        assert!(!p.deflate(), "deflate at 0 is a no-op");
    }

    #[test]
    fn pin_deflates_and_commits_and_is_terminal() {
        let mut p = WorkshopProject::new(2, WorkshopTarget::Mob(MobType::Cow), WorkshopMode::Reskin, [3, 4, 5]);
        p.pump();
        p.pump();
        p.pump();
        assert!(p.is_parked(), "an inflated un-pinned project is parked WIP");
        p.pin();
        assert_eq!(p.inflation, 0, "pin deflates");
        assert_eq!(p.state, ProjectState::Pinned);
        assert!(!p.is_parked(), "a pinned project is finished, not parked");
        // Pinned is terminal: no more pump/deflate.
        assert!(!p.pump());
        assert!(!p.deflate());
    }

    #[test]
    fn edit_mode_defaults_to_paint_and_toggles() {
        let mut ws = WorkshopProjects::new();
        assert_eq!(ws.edit_mode(), EditMode::Paint, "safe default is Paint");
        assert_eq!(ws.toggle_edit_mode(), EditMode::Sculpt);
        assert_eq!(ws.edit_mode(), EditMode::Sculpt);
        assert_eq!(ws.toggle_edit_mode(), EditMode::Paint);
    }

    #[test]
    fn paint_mode_only_paints_with_a_dye_and_never_carves() {
        // The §6/§8 guard: in Paint mode, break + dye paints the pixel; break
        // WITHOUT a dye does nothing (no accidental carve); place does nothing
        // (no shape change). A held block yields dye=None → None, so a block's
        // texture can never be stamped.
        let red = crate::block::WALLPAPER_RED;
        assert_eq!(
            resolve_edit_action(EditMode::Paint, Some(red), true, false),
            Some(EditOp::Paint(red))
        );
        assert_eq!(
            resolve_edit_action(EditMode::Paint, None, true, false),
            None,
            "no dye in Paint mode must NOT carve"
        );
        assert_eq!(
            resolve_edit_action(EditMode::Paint, Some(red), false, true),
            None,
            "right-click in Paint mode changes nothing"
        );
    }

    #[test]
    fn sculpt_mode_carves_on_break_and_places_with_a_dye() {
        let red = crate::block::WALLPAPER_RED;
        assert_eq!(
            resolve_edit_action(EditMode::Sculpt, None, true, false),
            Some(EditOp::Carve),
            "Sculpt break carves regardless of held item"
        );
        assert_eq!(
            resolve_edit_action(EditMode::Sculpt, Some(red), false, true),
            Some(EditOp::Place(red))
        );
        assert_eq!(
            resolve_edit_action(EditMode::Sculpt, None, false, true),
            None,
            "placing a microblock needs a dye for its colour"
        );
    }

    #[test]
    fn blown_up_origins_lists_parked_balloons_only() {
        let mut ws = WorkshopProjects::new();
        // A plain placed project (no blow_up) is not hidden.
        ws.add(WorkshopTarget::Block(5), WorkshopMode::Reskin, [1, 2, 3]);
        assert!(ws.blown_up_origins().is_empty());
        // A charging balloon's origin IS hidden.
        ws.note_blow_up_aim([10, 11, 12], [10, 11, 12], 7);
        assert_eq!(ws.blown_up_origins(), vec![[10, 11, 12]]);
    }

    #[test]
    fn many_concurrent_projects_with_stable_ids() {
        let mut ws = WorkshopProjects::new();
        let a = ws.add(WorkshopTarget::Block(5), WorkshopMode::Reskin, [0, 0, 0]);
        let b = ws.add(WorkshopTarget::Mob(MobType::Cow), WorkshopMode::Reskin, [4, 0, 0]);
        let c = ws.add(WorkshopTarget::Block(9), WorkshopMode::Reshape, [8, 0, 0]);
        assert_eq!(ws.len(), 3);
        // Distinct, stable ids.
        assert_ne!(a, b);
        assert_ne!(b, c);
        // Mutate one; the others are independent (each at its own stage).
        ws.get_mut(a).unwrap().pump();
        ws.get_mut(b).unwrap().pin();
        assert_eq!(ws.get(a).unwrap().inflation, 1);
        assert_eq!(ws.get(c).unwrap().inflation, 0);
        // `a` and `c` parked; `b` pinned (finished).
        let parked: Vec<u32> = ws.parked().map(|p| p.id).collect();
        assert!(parked.contains(&a) && parked.contains(&c));
        assert!(!parked.contains(&b));
        // Removing keeps ids stable (no reuse).
        ws.remove(a);
        let d = ws.add(WorkshopTarget::Block(1), WorkshopMode::Reskin, [12, 0, 0]);
        assert_ne!(d, a, "ids are never reused");
    }

    #[test]
    fn reset_clears_all_projects_and_restarts_id_numbering() {
        // Spec 40 — the Lobby "Reset Workshop" wipe drops every in-progress
        // project and restarts the id allocator (a fresh room numbers from #1).
        let mut ws = WorkshopProjects::new();
        ws.add(WorkshopTarget::Block(5), WorkshopMode::Reskin, [0, 0, 0]);
        ws.add(WorkshopTarget::Mob(MobType::Cow), WorkshopMode::Reskin, [4, 0, 0]);
        assert_eq!(ws.len(), 2);
        ws.clear();
        assert!(ws.is_empty(), "reset wipes the room's projects");
        // The allocator restarted: the next project is #0 again.
        let first_after = ws.add(WorkshopTarget::Block(1), WorkshopMode::Reskin, [0, 0, 0]);
        assert_eq!(first_after, 0, "id numbering restarts after a reset");
    }

    #[test]
    fn committing_a_painted_block_project_writes_a_global_override() {
        // Spec 40 Phase C (logic) — pin a painted block project → the override
        // registry carries that block's reskin for EVERY instance. (The Phase A
        // mesher test already proves a registered override re-textures every face;
        // this test closes the loop from a Workshop project to that registry.)
        let base = 373;
        let mut ws = WorkshopProjects::new();
        let id = ws.add(WorkshopTarget::Block(7), WorkshopMode::Reskin, [0, 0, 0]);
        {
            let p = ws.get_mut(id).unwrap();
            p.pump();
            p.paint = WorkshopPaint::Block(AuthoredFaces::solid([3, 9, 27, 255]));
        }
        let mut overrides = OverrideRegistry::new();
        assert!(overrides.block_face_layer(7, 0).is_none(), "no override before commit");

        let wrote = commit_project_override(ws.get(id).unwrap(), &mut overrides, base);
        assert!(wrote, "a painted block project commits an override");
        // Every face of block 7 now resolves to the appended override layer.
        for face in 0..6u8 {
            assert_eq!(overrides.block_face_layer(7, face), Some(base));
        }
    }

    #[test]
    fn committing_a_painted_mob_project_writes_per_part_overrides() {
        let base = 373;
        let mut overrides = OverrideRegistry::new();
        let mut proj = WorkshopProject::new(0, WorkshopTarget::Mob(MobType::Cow), WorkshopMode::Reskin, [0, 0, 0]);
        proj.paint = WorkshopPaint::Mob(vec![
            (0, AuthoredFaces::solid([1, 1, 1, 255])),
            (2, AuthoredFaces::solid([2, 2, 2, 255])),
        ]);
        assert!(commit_project_override(&proj, &mut overrides, base));
        // Parts 0 and 2 are reskinned; part 1 is untouched.
        assert!(overrides.mob_part_faces(MobType::Cow, 0, &[0; 6]).is_some());
        assert!(overrides.mob_part_faces(MobType::Cow, 2, &[0; 6]).is_some());
        assert!(overrides.mob_part_faces(MobType::Cow, 1, &[0; 6]).is_none());
    }

    #[test]
    fn capture_box_reads_built_blocks_and_bakes_single_block_envelope() {
        // Spec 40 Phase F — a small build inside the reshape cube captures to a
        // PlanData whose envelope fits the 16³ exact-build grid, so from_plan bakes
        // it cleanly into a micro-model.
        use crate::block;
        let mut w = crate::world::World::new();
        // A tiny L-shape near the origin.
        w.set_block(0, 0, 0, block::STONE);
        w.set_block(1, 0, 0, block::STONE);
        w.set_block(0, 1, 0, block::DIRT);

        let plan = capture_box_as_plan(&w, [0, 0, 0], RESHAPE_BOX).expect("non-empty capture");
        assert_eq!(plan.cells.len(), 3);
        assert_eq!(plan.width, 2, "x extent 0..1");
        assert_eq!(plan.height, 2, "y extent 0..1");
        assert_eq!(plan.depth, 1, "z extent 0");

        // Bakes within the exact-build envelope (never TooLarge for a 16³ scan).
        let data = crate::micro_model::MicroModelData::from_plan(&plan, crate::micro_model::MICRO_SCALE_16)
            .expect("16³-bounded capture bakes");
        assert_eq!(data.voxels.len(), 3);
    }

    #[test]
    fn capture_box_of_empty_space_is_none() {
        let w = crate::world::World::new();
        assert!(capture_box_as_plan(&w, [0, 0, 0], RESHAPE_BOX).is_none());
    }

    #[test]
    fn committing_an_unpainted_project_writes_nothing() {
        let mut overrides = OverrideRegistry::new();
        let proj = WorkshopProject::new(0, WorkshopTarget::Block(7), WorkshopMode::Reskin, [0, 0, 0]);
        assert!(!commit_project_override(&proj, &mut overrides, 373));
        assert!(overrides.is_empty());
    }

    #[test]
    fn bellows_targets_the_nearest_parked_project() {
        // Spec 40 — the bellows inflates the nearest parked project to the player.
        let mut ws = WorkshopProjects::new();
        let near = ws.add(WorkshopTarget::Block(1), WorkshopMode::Reskin, [2, 79, 2]);
        let far = ws.add(WorkshopTarget::Block(2), WorkshopMode::Reskin, [40, 79, 40]);
        // Player standing next to `near`.
        assert_eq!(ws.nearest_parked([3.0, 79.0, 2.0]), Some(near));
        // Pump it (what the bellows does), then it's still parked.
        ws.get_mut(near).unwrap().pump();
        assert_eq!(ws.get(near).unwrap().inflation, 1);
        // Pin the near one → the bellows now targets the far one.
        ws.get_mut(near).unwrap().pin();
        assert_eq!(ws.nearest_parked([3.0, 79.0, 2.0]), Some(far));
        // No parked projects → nothing to target.
        ws.get_mut(far).unwrap().pin();
        assert_eq!(ws.nearest_parked([3.0, 79.0, 2.0]), None);
    }

    #[test]
    fn projects_round_trip_through_serde_preserving_each_state() {
        // The headline Phase B invariant in miniature: >=2 parked, still-inflated
        // projects survive a serialize/deserialize with each restored to its exact
        // prior state (offset, inflation, mode, partial paint).
        let mut ws = WorkshopProjects::new();
        let a = ws.add(WorkshopTarget::Block(5), WorkshopMode::Reskin, [1, 2, 3]);
        let b = ws.add(WorkshopTarget::Mob(MobType::Sheep), WorkshopMode::Reshape, [9, 0, -4]);
        ws.get_mut(a).unwrap().pump(); // inflation 1
        let proj_b = ws.get_mut(b).unwrap();
        proj_b.pump();
        proj_b.pump(); // inflation 2
        proj_b.paint = WorkshopPaint::Block(AuthoredFaces::solid([7, 7, 7, 255]));

        let bytes = bincode::serialize(&ws).expect("serialize");
        let restored: WorkshopProjects = bincode::deserialize(&bytes).expect("deserialize");

        assert_eq!(restored.len(), 2);
        assert_eq!(restored, ws, "every project restored to its exact prior state");
        let ra = restored.get(a).unwrap();
        assert_eq!(ra.origin, [1, 2, 3]);
        assert_eq!(ra.inflation, 1);
        assert_eq!(ra.mode, WorkshopMode::Reskin);
        let rb = restored.get(b).unwrap();
        assert_eq!(rb.inflation, 2);
        assert_eq!(rb.mode, WorkshopMode::Reshape);
        assert!(matches!(rb.paint, WorkshopPaint::Block(_)), "partial paint survives");
        // Both are parked WIP (neither pinned).
        assert_eq!(restored.parked().count(), 2);
    }

    #[test]
    fn avatar_project_is_never_serialized() {
        // Global constraint: no saved WorkshopProjects can carry an `Avatar`
        // target — it is a transient Bellows blow-up of the player's own avatar.
        // Even if an autosave lands while it is inflated, the serialize filter
        // drops it, so a reload never sees an orphan Avatar project.
        let mut ws = WorkshopProjects::new();
        ws.add(WorkshopTarget::Block(5), WorkshopMode::Reskin, [1, 2, 3]);
        ws.note_avatar_blow_up_aim([0, 80, -4]);
        assert!(ws.has_avatar_project(), "avatar working copy exists in memory");

        let bytes = bincode::serialize(&ws).expect("serialize");
        let restored: WorkshopProjects = bincode::deserialize(&bytes).expect("deserialize");

        assert_eq!(restored.len(), 1, "only the persistent block project survives");
        assert!(!restored.has_avatar_project(), "no Avatar project after a save round-trip");
        assert!(
            restored.iter().all(|p| !matches!(p.target, WorkshopTarget::Avatar)),
            "no saved project carries Avatar",
        );
    }

    #[test]
    fn avatar_blow_up_aim_creates_then_continues_one_project() {
        let mut ws = WorkshopProjects::new();
        let corner = [0, 80, -4];
        assert!(!ws.has_avatar_project());
        assert!(ws.locked_avatar_project().is_none());

        ws.note_avatar_blow_up_aim(corner);
        assert!(ws.has_avatar_project(), "first aim creates the avatar working copy");
        assert_eq!(ws.iter().count(), 1, "exactly one project after the first aim");
        let id = ws.find_avatar().unwrap();
        assert_eq!(
            ws.get(id).unwrap().blow_up.unwrap().phase,
            BlowUpPhase::Charging,
        );
        assert!(ws.locked_avatar_project().is_none(), "still charging, not locked");

        // A second aim continues the SAME project (no duplicate).
        ws.note_avatar_blow_up_aim(corner);
        assert_eq!(ws.iter().filter(|p| matches!(p.target, WorkshopTarget::Avatar)).count(), 1);

        // Once locked, locked_avatar_project() reports it; collapse (keyed on the
        // mannequin corner = origin) tears it down.
        ws.get_mut(id).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        assert_eq!(ws.locked_avatar_project(), Some(id));
        assert!(ws.collapse_blow_up_at(corner), "collapse keys on the avatar origin");
        assert_eq!(
            ws.get(id).unwrap().blow_up.unwrap().phase,
            BlowUpPhase::Collapsing,
        );
        assert!(ws.locked_avatar_project().is_none(), "collapsing is not locked");
    }

    #[test]
    fn cage_grows_up_and_away_from_player() {
        let block = [10, 80, 10];
        // Player on the −X / −Z side → cage extends +X / +Z; block is the near corner.
        let min = blow_up_cage_min(block, [8.0, 8.0]);
        assert_eq!(min, [10, 80, 10], "block is the −X,−Z,bottom corner");
        // Player on the +X / +Z side → cage extends −X / −Z.
        let min2 = blow_up_cage_min(block, [14.0, 14.0]);
        assert_eq!(min2, [7, 80, 7], "cage extends away (−X,−Z); block at the +X,+Z corner");
        // Always grows UP: the cage floor == block.y in every case.
        assert_eq!(min[1], 80);
        assert_eq!(min2[1], 80);
    }

    #[test]
    fn cage_has_64_cells_including_the_block() {
        let block = [10, 80, 10];
        let min = blow_up_cage_min(block, [8.0, 8.0]);
        let cells = blow_up_cage_cells(min);
        assert_eq!(cells.len(), 64, "4×4×4");
        assert!(cells.contains(&(10, 80, 10)), "the source block is a corner cell");
        for (x, y, z) in &cells {
            assert!((min[0]..min[0] + BLOW_UP_SPAN).contains(x));
            assert!((min[1]..min[1] + BLOW_UP_SPAN).contains(y));
            assert!((min[2]..min[2] + BLOW_UP_SPAN).contains(z));
        }
    }

    #[test]
    fn cage_clear_unless_a_non_source_cell_is_blocked() {
        let mut world = crate::world::World::new();
        let block = [2, 80, 3];
        let min = blow_up_cage_min(block, [0.0, 0.0]); // extends +X,+Z → min == block
        assert!(blow_up_cage_clear(&world, min, block), "fresh world: nothing in the way");
        // The source block being solid does NOT block it.
        world.set_block(block[0], block[1], block[2], crate::block::STONE);
        assert!(blow_up_cage_clear(&world, min, block), "the source block is allowed");
        // A block in another cage cell DOES block it.
        world.set_block(min[0] + 1, min[1] + 1, min[2] + 1, crate::block::STONE);
        assert!(!blow_up_cage_clear(&world, min, block), "an obstacle in the cage blocks it");
    }

    #[test]
    fn blow_up_scale_hits_each_swell() {
        assert!((blow_up_scale_for_charge(0.0) - 1.0).abs() < 1e-4, "x1 at rest");
        assert!((blow_up_scale_for_charge(1.0 / 3.0) - 2.0).abs() < 1e-4, "x2 at first third");
        assert!((blow_up_scale_for_charge(2.0 / 3.0) - 3.0).abs() < 1e-4, "x3 at second third");
        assert!((blow_up_scale_for_charge(1.0) - 4.0).abs() < 1e-4, "x4 at full");
        assert!((blow_up_scale_for_charge(2.0) - 4.0).abs() < 1e-4, "clamps above full");
        // Monotonic non-decreasing across the range.
        let mut prev = 0.0;
        for i in 0..=100 {
            let s = blow_up_scale_for_charge(i as f32 / 100.0);
            assert!(s + 1e-4 >= prev, "scale must not go backwards");
            prev = s;
        }
    }

    #[test]
    fn blow_up_inflation_levels_track_the_swells() {
        assert_eq!(blow_up_inflation_for_charge(0.0), 0);
        assert_eq!(blow_up_inflation_for_charge(1.0 / 3.0), 2);
        assert_eq!(blow_up_inflation_for_charge(2.0 / 3.0), 4);
        assert_eq!(blow_up_inflation_for_charge(1.0), 6);
    }

    #[test]
    fn blow_up_charges_then_locks_at_full() {
        let mut b = BlowUp::charging([0, 80, 0]);
        assert_eq!(b.phase, BlowUpPhase::Charging);
        // Charge to full one tick at a time.
        for _ in 0..BLOW_UP_CHARGE_TICKS {
            assert!(b.advance(true, 1), "stays alive while charging");
        }
        assert_eq!(b.phase, BlowUpPhase::Locked);
        assert!((b.scale() - 4.0).abs() < 1e-4);
        // Locked stays locked even if the button is released.
        assert!(b.advance(false, 1));
        assert_eq!(b.phase, BlowUpPhase::Locked);
    }

    #[test]
    fn blow_up_collapses_to_x1_when_released_early() {
        let mut b = BlowUp::charging([0, 80, 0]);
        b.advance(true, BLOW_UP_CHARGE_TICKS / 2); // partway up
        assert_eq!(b.phase, BlowUpPhase::Charging);
        let mid = b.scale();
        assert!(mid > 1.0 && mid < 4.0);
        // Release: it begins collapsing from the current scale.
        assert!(b.advance(false, 1));
        assert_eq!(b.phase, BlowUpPhase::Collapsing);
        // Run the collapse out — it returns false (remove) at the end and never rests partial.
        let mut alive = true;
        for _ in 0..BLOW_UP_COLLAPSE_TICKS {
            alive = b.advance(false, 1);
        }
        assert!((b.scale() - 1.0).abs() < 1e-4, "terminal scale must be ×1, not partial");
        assert!(!alive, "fully collapsed → caller removes it");
    }

    #[test]
    fn blow_up_sneak_collapse_from_locked() {
        let mut b = BlowUp::charging([0, 80, 0]);
        b.advance(true, BLOW_UP_CHARGE_TICKS);
        assert_eq!(b.phase, BlowUpPhase::Locked);
        b.begin_collapse();
        assert_eq!(b.phase, BlowUpPhase::Collapsing);
        assert!((b.collapse_from - 4.0).abs() < 1e-4, "collapses from full");
    }

    #[test]
    fn find_at_origin_matches_only_exact_origin() {
        let mut ws = WorkshopProjects::new();
        let id = ws.add(WorkshopTarget::Block(crate::block::STONE), WorkshopMode::Reskin, [2, 79, 3]);
        assert_eq!(ws.find_at_origin([2, 79, 3]), Some(id));
        assert_eq!(ws.find_at_origin([2, 79, 4]), None);
    }

    #[test]
    fn note_blow_up_aim_creates_then_continues_one_project() {
        let mut ws = WorkshopProjects::new();
        let block = [2, 79, 3];
        let corner = [2, 79, 3];
        ws.note_blow_up_aim(block, corner, crate::block::STONE);
        let id = ws.find_at_origin(block).expect("created");
        {
            let b = ws.get(id).unwrap().blow_up.expect("has blow_up");
            assert_eq!(b.phase, BlowUpPhase::Charging);
            assert!(b.held_this_tick);
            assert_eq!(b.corner, corner);
        }
        // A second note doesn't create a duplicate; it re-flags held.
        ws.get_mut(id).unwrap().blow_up.as_mut().unwrap().held_this_tick = false;
        ws.note_blow_up_aim(block, corner, crate::block::STONE);
        assert_eq!(ws.iter().count(), 1, "no duplicate project");
        assert!(ws.get(id).unwrap().blow_up.unwrap().held_this_tick, "re-held");
    }

    #[test]
    fn tick_blow_ups_charges_held_to_locked() {
        let mut ws = WorkshopProjects::new();
        let block = [0, 79, 0];
        // Drive it like the game loop: each tick re-note (held) then advance 1 tick.
        for _ in 0..BLOW_UP_CHARGE_TICKS {
            ws.note_blow_up_aim(block, block, crate::block::STONE);
            ws.tick_blow_ups(1);
        }
        let id = ws.find_at_origin(block).expect("still here, locked");
        let p = ws.get(id).unwrap();
        assert_eq!(p.blow_up.unwrap().phase, BlowUpPhase::Locked);
        assert_eq!(p.inflation, MAX_INFLATION, "inflation synced to x4 at lock");
    }

    #[test]
    fn tick_blow_ups_collapses_and_removes_on_release() {
        let mut ws = WorkshopProjects::new();
        let block = [0, 79, 0];
        // One held tick to create + start charging.
        ws.note_blow_up_aim(block, block, crate::block::STONE);
        ws.tick_blow_ups(1);
        assert!(ws.find_at_origin(block).is_some());
        // Now stop noting (released) and tick until the collapse finishes.
        for _ in 0..(BLOW_UP_COLLAPSE_TICKS + 2) {
            ws.tick_blow_ups(1);
        }
        assert_eq!(ws.find_at_origin(block), None, "fully collapsed → project removed");
    }

    #[test]
    fn collapse_blow_up_at_begins_collapse() {
        let mut ws = WorkshopProjects::new();
        let block = [0, 79, 0];
        ws.note_blow_up_aim(block, block, crate::block::STONE);
        let id = ws.find_at_origin(block).unwrap();
        ws.get_mut(id).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        assert!(ws.collapse_blow_up_at(block));
        assert_eq!(ws.get(id).unwrap().blow_up.unwrap().phase, BlowUpPhase::Collapsing);
        // Nothing to collapse elsewhere.
        assert!(!ws.collapse_blow_up_at([9, 9, 9]));
    }

    #[test]
    fn tick_blow_ups_keeps_state_consistent_while_collapsing() {
        // Invariant: Inflated ⟺ inflation > 0. A collapsing balloon (inflation 0)
        // must read back as Placed, not leave a stale Inflated from when it locked.
        let mut ws = WorkshopProjects::new();
        let block = [0, 79, 0];
        // Charge it to Locked (inflation = MAX, state = Inflated).
        for _ in 0..BLOW_UP_CHARGE_TICKS {
            ws.note_blow_up_aim(block, block, crate::block::STONE);
            ws.tick_blow_ups(1);
        }
        let id = ws.find_at_origin(block).expect("locked, still here");
        assert_eq!(ws.get(id).unwrap().state, ProjectState::Inflated);
        // Sneak-collapse, then one tick: still collapsing (not yet removed), but
        // inflation has dropped to 0 — state must follow back to Placed.
        ws.collapse_blow_up_at(block);
        ws.tick_blow_ups(1);
        let p = ws.get(id).expect("still collapsing, not yet removed");
        assert_eq!(p.blow_up.unwrap().phase, BlowUpPhase::Collapsing);
        assert_eq!(p.inflation, 0);
        assert_eq!(p.state, ProjectState::Placed, "inflation 0 ⟹ Placed, never stale Inflated");
    }

    #[test]
    fn edit_buffer_starts_solid_with_the_source_block() {
        let buf = EditBuffer::solid(crate::block::STONE);
        assert!(buf.occupied([0, 0, 0]));
        assert!(buf.occupied([15, 15, 15]));
        assert_eq!(buf.color_at([8, 8, 8]), Some(crate::block::STONE));
        assert!(!buf.shape_changed(), "a fresh solid copy hasn't changed shape");
        assert!(!buf.occupied([16, 0, 0]));
        assert!(!buf.occupied([-1, 0, 0]));
        assert_eq!(buf.color_at([0, 16, 0]), None);
    }

    #[test]
    fn paint_recolours_an_occupied_cell_only() {
        let mut buf = EditBuffer::solid(crate::block::STONE);
        assert!(buf.paint([3, 4, 5], crate::block::WALLPAPER_RED));
        assert_eq!(buf.color_at([3, 4, 5]), Some(crate::block::WALLPAPER_RED));
        assert!(!buf.shape_changed(), "paint is recolour, not a shape change");
        buf.carve([3, 4, 5]);
        assert!(!buf.paint([3, 4, 5], crate::block::WALLPAPER_BLUE));
        assert_eq!(buf.color_at([3, 4, 5]), None);
        assert!(!buf.paint([99, 0, 0], crate::block::WALLPAPER_RED));
    }

    #[test]
    fn carve_clears_a_cell_and_marks_shape_changed() {
        let mut buf = EditBuffer::solid(crate::block::STONE);
        assert!(buf.carve([1, 2, 3]));
        assert!(!buf.occupied([1, 2, 3]));
        assert!(buf.shape_changed());
        assert!(!buf.carve([1, 2, 3]));
    }

    #[test]
    fn place_fills_an_empty_cell_and_marks_shape_changed() {
        let mut buf = EditBuffer::solid(crate::block::STONE);
        buf.carve([7, 7, 7]);
        assert!(buf.place([7, 7, 7], crate::block::WALLPAPER_GREEN));
        assert!(buf.occupied([7, 7, 7]));
        assert_eq!(buf.color_at([7, 7, 7]), Some(crate::block::WALLPAPER_GREEN));
        assert!(buf.shape_changed());
        assert!(!buf.place([7, 7, 7], crate::block::WALLPAPER_BLUE));
        assert!(!buf.place([16, 0, 0], crate::block::WALLPAPER_BLUE));
    }

    #[test]
    fn to_micro_model_emits_one_voxel_per_occupied_cell() {
        let mut buf = EditBuffer::solid(crate::block::STONE);
        assert_eq!(buf.to_micro_model().voxels.len(), 16 * 16 * 16);
        buf.carve([0, 0, 0]);
        let mm = buf.to_micro_model();
        assert_eq!(mm.voxels.len(), 16 * 16 * 16 - 1);
        assert_eq!(mm.scale, crate::micro_model::MICRO_SCALE_16);
        buf.paint([2, 2, 2], crate::block::WALLPAPER_PINK);
        let mm = buf.to_micro_model();
        let v = mm.voxels.iter().find(|v| v.mx == 2 && v.my == 2 && v.mz == 2).unwrap();
        assert_eq!(v.block_id, crate::block::WALLPAPER_PINK);
    }

    #[test]
    fn to_authored_faces_paints_right_face_and_collapses_sides() {
        use crate::override_registry::FACE_BYTES;
        let mut buf = EditBuffer::solid(crate::block::STONE);
        for z in 0..EDIT_GRID as i32 {
            for x in 0..EDIT_GRID as i32 {
                buf.paint([x, EDIT_GRID as i32 - 1, z], crate::block::WALLPAPER_RED);
                buf.paint([x, 0, z], crate::block::WALLPAPER_BLUE);
            }
        }
        let faces = buf.to_authored_faces(|id| match id {
            crate::block::WALLPAPER_RED => [255, 0, 0, 255],
            crate::block::WALLPAPER_BLUE => [0, 0, 255, 255],
            _ => [9, 9, 9, 255],
        });
        assert!(faces.is_valid());
        assert!(faces.faces[0].chunks_exact(4).all(|p| p == [255, 0, 0, 255]));
        assert!(faces.faces[1].chunks_exact(4).all(|p| p == [0, 0, 255, 255]));
        for f in 3..6 {
            assert_eq!(faces.faces[f], faces.faces[2], "side {f} must match the shared side");
        }
        assert!(faces.faces.iter().all(|f| f.len() == FACE_BYTES));
    }

    #[test]
    fn symmetry_off_touches_only_the_cell() {
        let cells = symmetric_cells([3, 5, 4], EditSymmetry::Off);
        assert_eq!(cells, vec![[3, 5, 4]]);
    }

    #[test]
    fn symmetry_left_right_mirrors_across_x() {
        let mut cells = symmetric_cells([3, 5, 4], EditSymmetry::LeftRight);
        cells.sort();
        assert_eq!(cells, vec![[3, 5, 4], [12, 5, 4]]);
        let same = symmetric_cells([7, 0, 0], EditSymmetry::LeftRight);
        assert_eq!(same.len(), 2);
    }

    #[test]
    fn symmetry_all_sides_is_the_four_fold_rotation_about_y() {
        let cells = symmetric_cells([3, 5, 4], EditSymmetry::AllSides);
        let mut got: Vec<[i32; 3]> = cells.clone();
        got.sort();
        let mut want = vec![[3, 5, 4], [4, 5, 12], [12, 5, 11], [11, 5, 3]];
        want.sort();
        assert_eq!(got, want);
        assert_eq!(cells.len(), 4, "four distinct cells, deduped");
        for c in &cells {
            assert_eq!(c[1], 5);
        }
    }

    #[test]
    fn symmetry_cycle_order() {
        assert_eq!(EditSymmetry::Off.next(), EditSymmetry::LeftRight);
        assert_eq!(EditSymmetry::LeftRight.next(), EditSymmetry::AllSides);
        assert_eq!(EditSymmetry::AllSides.next(), EditSymmetry::Off);
        assert_eq!(EditSymmetry::default(), EditSymmetry::AllSides, "all-sides is the default");
    }

    #[test]
    fn apply_paint_mirrors_to_all_symmetric_cells() {
        let mut buf = EditBuffer::solid(crate::block::STONE);
        assert!(buf.apply([3, 5, 4], EditOp::Paint(crate::block::WALLPAPER_RED), EditSymmetry::AllSides));
        for c in symmetric_cells([3, 5, 4], EditSymmetry::AllSides) {
            assert_eq!(buf.color_at(c), Some(crate::block::WALLPAPER_RED), "cell {c:?} painted");
        }
    }

    #[test]
    fn apply_carve_mirrors_and_marks_shape_changed() {
        let mut buf = EditBuffer::solid(crate::block::STONE);
        assert!(buf.apply([1, 1, 1], EditOp::Carve, EditSymmetry::LeftRight));
        assert!(!buf.occupied([1, 1, 1]));
        assert!(!buf.occupied([14, 1, 1]));
        assert!(buf.shape_changed());
    }

    #[test]
    fn apply_edit_lazily_creates_the_buffer_from_the_source_block() {
        let mut ws = WorkshopProjects::new();
        let block = [0, 79, 0];
        ws.note_blow_up_aim(block, block, crate::block::STONE);
        let id = ws.find_at_origin(block).unwrap();
        ws.get_mut(id).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        assert!(ws.get(id).unwrap().edit.is_none(), "no buffer until first edit");
        assert!(ws.apply_edit(id, [1, 1, 1], EditOp::Paint(crate::block::WALLPAPER_RED)));
        let buf = ws.get(id).unwrap().edit.as_ref().expect("buffer created");
        assert_eq!(buf.color_at([1, 1, 1]), Some(crate::block::WALLPAPER_RED));
        assert_eq!(buf.color_at([0, 0, 0]), Some(crate::block::STONE), "rest is the source block");
    }

    #[test]
    fn apply_edit_refuses_a_non_locked_balloon() {
        // Editing is gated to a locked ×4 copy — a stale id on a still-charging
        // balloon must not create a buffer or change anything.
        let mut ws = WorkshopProjects::new();
        let block = [0, 79, 0];
        ws.note_blow_up_aim(block, block, crate::block::STONE); // Charging, not Locked
        let id = ws.find_at_origin(block).unwrap();
        assert_eq!(ws.get(id).unwrap().blow_up.unwrap().phase, BlowUpPhase::Charging);
        assert!(!ws.apply_edit(id, [1, 1, 1], EditOp::Paint(crate::block::WALLPAPER_RED)));
        assert!(ws.get(id).unwrap().edit.is_none(), "no buffer created on a charging balloon");
    }

    #[test]
    fn cycle_symmetry_advances_the_room_default() {
        let mut ws = WorkshopProjects::new();
        assert_eq!(ws.symmetry(), EditSymmetry::AllSides);
        assert_eq!(ws.cycle_symmetry(), EditSymmetry::Off);
        assert_eq!(ws.cycle_symmetry(), EditSymmetry::LeftRight);
        assert_eq!(ws.cycle_symmetry(), EditSymmetry::AllSides);
    }

    #[test]
    fn pick_locked_cell_only_targets_locked_balloons() {
        let mut ws = WorkshopProjects::new();
        let block = [0, 0, 0];
        ws.note_blow_up_aim(block, block, crate::block::STONE);
        let id = ws.find_at_origin(block).unwrap();
        assert!(ws.pick_locked_cell([2.0, 2.0, -5.0], [0.0, 0.0, 1.0]).is_none());
        ws.get_mut(id).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        let (hit_id, pick) = ws.pick_locked_cell([2.0, 2.0, -5.0], [0.0, 0.0, 1.0]).expect("hit");
        assert_eq!(hit_id, id);
        assert_eq!(pick.cell[2], 0);
    }

    #[test]
    fn sole_locked_project_is_some_only_when_exactly_one_is_locked() {
        let mut ws = WorkshopProjects::new();
        // Zero locked → None.
        assert_eq!(ws.sole_locked_project(), None);
        // One charging balloon — still no LOCKED project.
        let a = [0, 0, 0];
        ws.note_blow_up_aim(a, a, crate::block::STONE);
        let id_a = ws.find_at_origin(a).unwrap();
        assert_eq!(ws.sole_locked_project(), None, "charging is not locked");
        // Lock it → exactly one locked → Some(id).
        ws.get_mut(id_a).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        assert_eq!(ws.sole_locked_project(), Some(id_a));
        // A second locked balloon → ambiguous → None.
        let b = [4, 0, 0];
        ws.note_blow_up_aim(b, b, crate::block::STONE);
        let id_b = ws.find_at_origin(b).unwrap();
        ws.get_mut(id_b).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        assert_eq!(ws.sole_locked_project(), None, "two locked balloons → ambiguous");
    }

    #[test]
    fn any_locked_false_when_no_projects() {
        let ws = WorkshopProjects::new();
        assert!(!ws.any_locked());
    }

    #[test]
    fn any_locked_false_when_only_charging() {
        let mut ws = WorkshopProjects::new();
        let a = [0, 0, 0];
        ws.note_blow_up_aim(a, a, crate::block::STONE);
        // balloon exists but is Charging, not Locked
        assert!(!ws.any_locked());
    }

    #[test]
    fn any_locked_true_when_one_balloon_is_locked() {
        let mut ws = WorkshopProjects::new();
        let a = [0, 0, 0];
        ws.note_blow_up_aim(a, a, crate::block::STONE);
        let id = ws.find_at_origin(a).unwrap();
        ws.get_mut(id).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        assert!(ws.any_locked());
    }

    #[test]
    fn eyedrop_reads_the_painted_colour() {
        let mut ws = WorkshopProjects::new();
        let block = [0, 0, 0];
        ws.note_blow_up_aim(block, block, crate::block::STONE);
        let id = ws.find_at_origin(block).unwrap();
        ws.get_mut(id).unwrap().blow_up.as_mut().unwrap().phase = BlowUpPhase::Locked;
        ws.set_symmetry(EditSymmetry::Off);
        ws.apply_edit(id, [5, 6, 7], EditOp::Paint(crate::block::WALLPAPER_PINK));
        assert_eq!(ws.eyedrop_at(id, [5, 6, 7]), Some(crate::block::WALLPAPER_PINK));
    }

    #[test]
    fn pin_kind_picks_paint_then_shape() {
        let mut buf = EditBuffer::solid(crate::block::STONE);
        assert_eq!(buf.pin_kind(), PinKind::Paint);
        buf.paint([0, 0, 0], crate::block::WALLPAPER_RED);
        assert_eq!(buf.pin_kind(), PinKind::Paint);
        buf.carve([0, 0, 0]);
        assert_eq!(buf.pin_kind(), PinKind::Shape);
    }

    #[test]
    fn release_clears_held_so_there_is_no_over_charge() {
        let mut ws = WorkshopProjects::new();
        let block = [0, 79, 0];
        ws.note_blow_up_aim(block, block, crate::block::STONE);
        ws.note_blow_up_aim(block, block, crate::block::STONE);
        let id = ws.find_at_origin(block).unwrap();
        assert!(ws.get(id).unwrap().blow_up.unwrap().held_this_tick);
        ws.note_blow_up_release(block);
        assert!(!ws.get(id).unwrap().blow_up.unwrap().held_this_tick, "release cleared the flag");
        ws.tick_blow_ups(1);
        assert_eq!(ws.get(id).unwrap().blow_up.unwrap().phase, BlowUpPhase::Collapsing);
    }

    #[test]
    fn bellows_aims_at_the_mannequin_from_the_front() {
        // Mannequin at origin, facing +Z (front −Z rotated to face the player on +Z).
        let pos = [0.0, 0.0, 0.0];
        let yaw = WORKSHOP_MANNEQUIN_YAW;
        let scale = 1.0;
        // Eye in front of the face, looking at the chest. With yaw chosen so the
        // front points +Z, a camera on +Z looking −Z must hit.
        let hit = pick_avatar_mannequin([0.0, 1.0, 5.0], [0.0, 0.0, -1.0], pos, yaw, scale, 0.0);
        assert!(hit.is_some(), "front-on ray should hit the mannequin");
        // A ray well to the side misses.
        let miss = pick_avatar_mannequin([10.0, 1.0, 5.0], [0.0, 0.0, -1.0], pos, yaw, scale, 0.0);
        assert!(miss.is_none(), "side ray should miss");
        // Scaling up keeps the centre-of-figure hit (and returns a nearer entry t).
        assert!(
            pick_avatar_mannequin([0.0, 4.0, 12.0], [0.0, 0.0, -1.0], pos, yaw, 4.0, 0.0).is_some()
        );
    }

    #[test]
    fn bellows_aim_widens_to_cover_the_separated_arm() {
        // With the limbs fully apart (separation 1.0), a ray that only grazes
        // the together-pose arm position (x≈0.5, the old fixed AABB's edge)
        // must still hit the widened union box (arm reaches x≈0.75+). This is
        // the regression the fixed AVATAR_AABB_MIN/MAX constants could not
        // express — the aim highlight has to track R, not just the paint ray.
        let pos = [0.0, 0.0, 0.0];
        let yaw = WORKSHOP_MANNEQUIN_YAW;
        // Aim at the right arm's fully-separated position (x≈0.5+0.25=0.75,
        // model y≈1.05) from the front (+Z looking −Z).
        let eye = [0.75, 1.05, 5.0];
        let dir = [0.0, 0.0, -1.0];
        let together = pick_avatar_mannequin(eye, dir, pos, yaw, 1.0, 0.0);
        let apart = pick_avatar_mannequin(eye, dir, pos, yaw, 1.0, 1.0);
        assert!(
            apart.is_some(),
            "separated arm must be reachable at separation 1.0"
        );
        assert!(
            together.is_none() || together.unwrap() >= apart.unwrap(),
            "the widened aabb should not be a worse (farther/missing) hit than the together pose"
        );
    }

    #[test]
    fn world_ray_round_trips_to_a_front_face_hit() {
        // Mannequin at the real anchor, facing +Z, blown up ×4. A camera in FRONT of
        // the face looking at the head must map to a −Z (face 5) hit on the head/body.
        let pos = WORKSHOP_MANNEQUIN_POS;
        let yaw = WORKSHOP_MANNEQUIN_YAW;
        let scale = AVATAR_BLOW_UP_SCALE;
        // Head centre in world ≈ pos + R(yaw)·(0,1.575,0)·scale. Front is +Z, so a
        // camera further along +Z looking −Z is in front of the face.
        let eye = [pos[0], pos[1] + 1.575 * scale, pos[2] + 20.0];
        let dir = [0.0, 0.0, -1.0];
        let (o, d) = world_ray_to_avatar_model(eye, dir, pos, yaw, scale);
        let hit = crate::skin_hit::ray_hit_avatar(o, d, crate::skin_uv::SkinLayer::Base, 0.0, false, crate::skin_uv::ArmModel::Classic)
            .expect("front-on ray must hit the avatar in model space");
        assert!(matches!(hit.part, 0 | 1), "head or body, got {}", hit.part);
        assert_eq!(hit.face, 5, "front (−Z) face");

        // Centred-on-the-head ray → the head-front-centre texel. Aim a hair into
        // the +x/centre of the head's −Z face (model ≈ (0.05, 1.55)) so the
        // fraction lands solidly inside a texel rather than on the 0.5 pixel
        // boundary (where float noise in the yaw=π rotation could floor to the
        // neighbouring column). This pins the whole ray → model → hit → texel
        // chain, not just the face. On the front tile u0 is the character's RIGHT
        // (the edge shared with the right-side tile — the Minecraft box unwrap),
        // so +x of centre is the LOW-u side: frac_u = 0.4 → atlas column 11.
        let head_eye = [pos[0] - 0.2, pos[1] + 6.2, pos[2] + 20.0];
        let (ho, hd) = world_ray_to_avatar_model(head_eye, dir, pos, yaw, scale);
        let head = crate::skin_hit::ray_hit_avatar(ho, hd, crate::skin_uv::SkinLayer::Base, 0.0, false, crate::skin_uv::ArmModel::Classic)
            .expect("centred ray must hit the head");
        assert_eq!(head.part, 0, "head, got {}", head.part);
        assert_eq!(head.face, 5, "front (−Z) face");
        let texel = crate::skin_uv::texel_for(
            head.part,
            head.face,
            head.frac_u,
            head.frac_v,
            crate::skin_uv::SkinLayer::Base,
            crate::skin_uv::ArmModel::Classic,
        );
        assert_eq!(texel, Some((11, 12)), "head-front-centre texel");

        // Off-axis: a front-on ray at the avatar's RIGHT-ARM column. The yaw=π/2
        // body rotation maps world −x to model +x, so the right arm (model
        // x ≈ +0.375) sits at world x ≈ pos.x − 0.375·scale. This catches an
        // X-axis sign error / left-right swap that a down-the-symmetry-axis ray
        // (all x=0) can't see: it must return the right arm (part 3), not the
        // body or a mirrored left arm.
        let arm_eye = [pos[0] - 0.375 * scale, pos[1] + 1.05 * scale, pos[2] + 20.0];
        let (ao, ad) = world_ray_to_avatar_model(arm_eye, dir, pos, yaw, scale);
        let arm = crate::skin_hit::ray_hit_avatar(ao, ad, crate::skin_uv::SkinLayer::Base, 0.0, false, crate::skin_uv::ArmModel::Classic)
            .expect("off-axis ray must hit the right arm");
        assert_eq!(arm.part, 3, "right arm (part 3), got {}", arm.part);
        assert_eq!(arm.face, 5, "front (−Z) face");
    }

    #[test]
    fn apart_pose_render_and_hit_test_agree_in_world_space() {
        // Lock-step guard for the limbs-apart pose. The renderer displaces each
        // part by `skin_pose::part_offset` rotated into world space; the hit-test
        // displaces the same boxes in model space. If those two ever disagree the
        // crosshair lands somewhere other than the limb — the exact failure mode
        // `skin_pose` exists to prevent. So: take the right arm's SEPARATED model
        // position, push it out to world space the way the renderer does, aim a
        // camera down that column, and require the hit-test to name the right arm.
        let pos = WORKSHOP_MANNEQUIN_POS;
        let yaw = WORKSHOP_MANNEQUIN_YAW;
        let scale = AVATAR_BLOW_UP_SCALE;
        let dir = [0.0, 0.0, -1.0];

        // Right arm centre in model space, fully apart (offset +0.25 on X).
        let off = crate::skin_pose::part_offset(3, 1.0);
        let arm_model = [0.375 + off[0], 1.05 + off[1], 0.0 + off[2]];
        // Renderer transform: R_y(theta) then scale then translate, where the
        // avatar builder uses theta = yaw + PI/2.
        let theta = yaw + std::f32::consts::FRAC_PI_2;
        let (c, s) = (theta.cos(), theta.sin());
        let wx = pos[0] + (arm_model[0] * c - arm_model[2] * s) * scale;
        let wz = pos[2] + (arm_model[0] * s + arm_model[2] * c) * scale;
        let wy = pos[1] + arm_model[1] * scale;

        // A camera in front of that column, aimed straight at it.
        let eye = [wx, wy, wz + 20.0];
        let (o, d) = world_ray_to_avatar_model(eye, dir, pos, yaw, scale);
        let hit = crate::skin_hit::ray_hit_avatar(o, d, crate::skin_uv::SkinLayer::Base, 1.0, true, crate::skin_uv::ArmModel::Classic)
            .expect("apart-pose ray must hit the separated arm");
        assert_eq!(hit.part, 3, "right arm at its SEPARATED position, got {}", hit.part);

        // And the same column at rest must NOT be where the apart arm is: if the
        // offset were ignored on one side, these would coincide.
        let at_rest = crate::skin_hit::ray_hit_avatar(o, d, crate::skin_uv::SkinLayer::Base, 0.0, true, crate::skin_uv::ArmModel::Classic);
        assert!(
            at_rest.is_none_or(|h| h.part != 3),
            "the arm must actually have moved — resting geometry still fills the apart column"
        );
    }

    #[test]
    fn separated_inner_arm_face_becomes_reachable_through_the_world_transform() {
        // The whole point, end to end: aim at the INNER face of the right arm
        // (its −X side, which the torso buries at rest) from inside the gap the
        // apart pose opens, and require a hit on that face.
        // Model space: the gap between torso (x ≤ 0.25) and separated arm
        // (x ≥ 0.5). Sit at x = 0.375 and look toward +X at the arm's inner face.
        let o = [0.30, 1.05, 0.0];
        let d = [1.0, 0.0, 0.0];
        let hit = crate::skin_hit::ray_hit_avatar(o, d, crate::skin_uv::SkinLayer::Base, 1.0, true, crate::skin_uv::ArmModel::Classic)
            .expect("the inner arm face must be reachable once the limbs are apart");
        assert_eq!(hit.part, 3, "right arm");
        assert_eq!(hit.face, 1, "inner (−X) face");
    }
}
