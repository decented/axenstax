//! Rail freight (Phase 1) — the cart, the engine's FIRST vehicle entity.
//!
//! A cart sits on a [`rail::TRACK`] cell and, once dispatched, rolls one cell
//! at a time along the track using the pure stepper [`rail::next_track_step`].
//! At a terminus (or a junction, which Phase 1 does not switch through) the
//! cart parks.
//!
//! Task 1.3 is the cart entity + its timed-travel tick. Depot load/unload
//! (1.4), ride-along (1.5), persistence (1.6) and multiplayer broadcast (1.7)
//! build on top.
//!
//! The advance logic ([`advance`]) is a pure free function tested without the
//! ECS — `is_track` is injected as a closure exactly like the `rail` stepper,
//! so the timed-travel core is fully unit-testable. `tick_carts` is the thin
//! ECS wrapper that reads the real `World` and writes `Position`.
//!
//! Spec: `docs/foundations/2026-06-09-rail-freight-logistics.md`.

use crate::rail::{self, Cell};

/// The cart's armour tier (CA1). A cart's hull determines how well it resists
/// being *breached* — a sturdier hull takes more to break open. Tiers ascend
/// `Wood → Iron → Diamond`; `Wood` is the unarmoured default a fresh / parked
/// cart spawns with. CA4 reads [`Hull::hardness`] to gate breach-to-break, and
/// Phase 3 robbery will key loot/resistance off the same tier.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
pub enum Hull {
    /// Unarmoured wood — the default tier for a freshly-placed cart.
    #[default]
    Wood,
    /// Iron-plated — mid tier; resists breaching more than wood.
    Iron,
    /// Diamond-clad — top tier; the toughest hull to breach.
    Diamond,
}

impl Hull {
    /// Breach hardness for this tier — an ascending, ordinal "how much it takes
    /// to break this hull open" value (`Wood < Iron < Diamond`). CA4 gates
    /// breach-to-break on it; the absolute numbers are a tuning seed, only the
    /// ORDER is load-bearing today.
    pub fn hardness(self) -> u32 {
        match self {
            Hull::Wood => 1,
            Hull::Iron => 3,
            Hull::Diamond => 6,
        }
    }

    /// CA4 breach threshold — total break-work (in break-ticks) a player must
    /// lay against this hull before the cart breaks open. Ascends
    /// `Wood < Iron < Diamond`, the SAME ordinal contract as [`hardness`]; the
    /// absolute numbers are a playtest tuning seed.
    ///
    /// * `Wood` ≈ instant — a single mining tick breaches it (matches an
    ///   unarmoured cart being trivially smashable).
    /// * `Iron` ≈ slow — several seconds of held mining at 20 TPS.
    /// * `Diamond` ≈ very slow — the armour really resists being smashed.
    ///
    /// Kept proportional to [`hardness`] (`hardness * 20` break-ticks ≈ that many
    /// hardness "seconds" at 20 TPS) so the two helpers can never disagree on
    /// ordering. CA4 accrues `breach` each break-tick and breaks the cart when
    /// `breach >= breach_max(hull)`.
    pub fn breach_max(self) -> f32 {
        // 20 break-ticks ≈ 1 s of held mining at 20 TPS. Wood=20 (~1 s, the
        // gentlest "armour"), Iron=60 (~3 s), Diamond=120 (~6 s).
        (self.hardness() * 20) as f32
    }

    /// How many whole `per_hit` breach increments it takes to break this hull —
    /// `ceil(breach_max / per_hit)`, with a `per_hit <= 0` guard returning
    /// `u32::MAX` (an un-breachable cart rather than a divide-by-zero). A pure
    /// helper so tests can assert the tier ordering in "hits" terms regardless
    /// of the absolute threshold tuning.
    ///
    /// `#[allow(dead_code)]`: the live break path accrues real-tick breach and
    /// gates on [`breach_max`] directly, so this "hits" view has no production
    /// caller yet — it's a tested tuning/inspection helper (and the natural API
    /// for a future HUD breach meter). Kept public + tested rather than deleted.
    #[allow(dead_code)]
    pub fn hits_to_break(self, per_hit: f32) -> u32 {
        if per_hit <= 0.0 || !per_hit.is_finite() {
            return u32::MAX;
        }
        (self.breach_max() / per_hit).ceil() as u32
    }

    /// The inventory item (a [`crate::item::MaterialId`]) that represents a cart
    /// of this hull tier. Inverse of [`cart_hull_for_item`]. CA3 reads the held
    /// item to spawn a cart with the matching hull; CA4 drops `item_id()` when a
    /// cart of this tier is broken. One source of truth for the tier ↔ item map.
    pub fn item_id(self) -> crate::item::MaterialId {
        use crate::item::MaterialId;
        match self {
            Hull::Wood => MaterialId::WoodCart,
            Hull::Iron => MaterialId::IronCart,
            Hull::Diamond => MaterialId::DiamondCart,
        }
    }
}

/// The cart [`Hull`] tier a cart-item material represents, or `None` if the
/// material is not a cart item. Inverse of [`Hull::item_id`]. CA3 uses it to
/// decide whether a held item is a placeable cart (and which hull to spawn);
/// CA4 uses it on the drop path.
pub fn cart_hull_for_item(m: crate::item::MaterialId) -> Option<Hull> {
    use crate::item::MaterialId;
    match m {
        MaterialId::WoodCart => Some(Hull::Wood),
        MaterialId::IronCart => Some(Hull::Iron),
        MaterialId::DiamondCart => Some(Hull::Diamond),
        _ => None,
    }
}

/// ECS marker for a cart entity. Deliberately carries NO `OnGround` component
/// so `entity::tick_entities` skips it (no gravity / block collision) — the
/// cart is driven entirely by `tick_carts` along the track, the same opt-out
/// `spawn_arrow` uses for projectiles.
pub struct CartEntity;

/// Per-cart state. Lives alongside `Position` + `CartEntity` in the ECS and is
/// the unit the pure [`advance`] core operates on. Serializable so Task 1.6 can
/// persist carts with the world.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct CartData {
    /// The track cell the cart currently occupies (its "from" anchor while
    /// `progress` interpolates toward the next cell).
    pub cell: Cell,
    /// The cell the cart rolled in from, or `None` for a freshly-placed cart
    /// with no direction history. Drives `rail::next_track_step` so the cart
    /// keeps going the way it was already heading rather than reversing.
    pub came_from: Option<Cell>,
    /// Interpolation `0..1` between `cell` and the next cell. The cart's render
    /// position lerps between the two cell centres by this fraction.
    pub progress: f32,
    /// Speed in cells/tick. `0` = parked. Set to [`CART_SPEED`] on dispatch.
    pub speed: f32,
    /// Yaw (radians) for the model, derived from the cell-to-cell travel
    /// direction. 0 = heading along -Z (Minecraft convention, matches
    /// `Camera::forward`).
    pub facing: f32,
    /// Cargo. Reuses the chest's 27-slot store so the depot load/unload (Task
    /// 1.4) can move stacks between a depot chest and the cart with the same
    /// slot helpers.
    pub cargo: crate::chest::ChestData,
    /// Armour tier (CA1). MUST stay the LAST *serialized* field — it is appended
    /// for save back-compat (bincode is positional). `#[serde(default)]` gives a
    /// fresh cart `Hull::Wood` and lets JSON-shaped literals omit it; see
    /// `save::deserialize_world_save_tolerant` for the bincode caveat on
    /// pre-hull saves that already contain carts.
    #[serde(default)]
    pub hull: Hull,
    /// CA4 breach accumulator — how much "break work" has been laid against this
    /// cart's hull this mining session, in break-ticks. Transient runtime state:
    /// `#[serde(skip)]` excludes it from bincode ENTIRELY, so the serialized
    /// `CartData` layout is byte-for-byte UNCHANGED (no save break, no protocol
    /// bump). It defaults to `0` on load and resets whenever the breacher stops
    /// or re-targets a different cell (see `game_loop.rs` break-path
    /// interception). Once it crosses [`Hull::breach_max`] the cart breaks.
    #[serde(skip)]
    pub breach: f32,
}

/// Cart roll speed in cells/tick. 0.08 cells/tick ≈ 1.6 blocks/s at 20 TPS — a
/// gentle, readable trundle. Tuned in playtest.
pub const CART_SPEED: f32 = 0.08;

/// Speed multiplier for a cart rolling over a POWERED rail (Spec 48 — Electricity).
/// A TRACK cell whose `meta` powered-bit is set (driven each `power_tick` by an
/// adjacent energised cable) speeds a passing cart by this factor. 2× ≈ 3.2
/// blocks/s — a noticeable "electric boost" over the trundle. Tunable in playtest
/// (owner/Axolittle to confirm the feel).
pub const POWERED_RAIL_SPEED_MULT: f32 = 2.0;

/// Y offset above the cell's integer Y so the cart body sits on top of the
/// flat track slab rather than sinking into it. Matches the small lift used
/// when the cart is first placed.
pub const CART_Y_OFFSET: f32 = 0.1;

/// How far the rider's seat sits above the cart's world anchor, in blocks,
/// BEFORE the player's own eye height is added. Small positive lift so the
/// camera clears the cart body (which sits at `CART_Y_OFFSET` above the slab)
/// instead of poking through it. Tuned in playtest alongside `CART_Y_OFFSET`.
pub const SEAT_LIFT: f32 = 0.2;

/// Pure ride-along follow transform (Task 1.5): given the cart's world-space
/// anchor (`Position`, the lerped cell centre from [`render_position`]) and the
/// rider's own eye height, return where the rider's EYE/camera should sit.
///
/// The rider rides AT the cart's XZ and looks out from a seat lifted
/// `SEAT_LIFT + eye_height` above the anchor. Pure + total — no ECS, no
/// `World`, no clock — so the seat geometry is unit-testable in isolation; the
/// per-tick wiring in `game_loop.rs` just feeds it the live cart `Position` and
/// the engine's `PLAYER_EYE_HEIGHT`, then back-solves the player foot position
/// from the returned eye (`foot = eye - (0, eye_height, 0)`).
pub fn rider_eye_position(cart_pos: glam::Vec3, eye_height: f32) -> glam::Vec3 {
    cart_pos + glam::Vec3::new(0.0, SEAT_LIFT + eye_height, 0.0)
}

/// Yaw (radians) for travel from `from` to `to`, derived from the horizontal
/// delta. 0 = -Z, matching `Camera::forward` (`yaw` measured so that
/// `(-sin(yaw), -cos(yaw))` is the -Z-forward XZ direction). Only the four
/// cardinal directions occur on a Phase 1 track, but the formula is general.
pub fn yaw_to(from: Cell, to: Cell) -> f32 {
    let dx = (to.0 - from.0) as f32;
    let dz = (to.2 - from.2) as f32;
    // Camera convention: forward_xz = (-sin(yaw), -cos(yaw)). Invert it:
    // yaw = atan2(-dx, -dz).
    (-dx).atan2(-dz)
}

/// World-space centre of a cell's top-slab surface — the point the cart body
/// is anchored to. `(cell + 0.5, cell.y + CART_Y_OFFSET, cell + 0.5)`.
pub fn cell_centre(cell: Cell) -> glam::Vec3 {
    glam::Vec3::new(
        cell.0 as f32 + 0.5,
        cell.1 as f32 + CART_Y_OFFSET,
        cell.2 as f32 + 0.5,
    )
}

/// Seed an initial `came_from` for a parked cart being dispatched, so
/// `rail::next_track_step` can resolve a single onward neighbour.
///
/// The stepper returns `None` when a cell has more than one onward neighbour
/// and `came_from` is `None` (it can't pick a side). A cart parked in the
/// MIDDLE of a straight line is exactly that case. To break the tie we choose
/// the onward neighbour best matching the dispatcher's look direction and set
/// `came_from` to the OPPOSITE side — then `next_track_step` filters that side
/// out and returns the chosen neighbour.
///
/// * Terminus (one neighbour) → `None` already works; we return `None`.
/// * Two-or-more neighbours → pick the neighbour whose direction best aligns
///   with `look_dir` (highest dot product), and return the neighbour that lies
///   most OPPOSITE to it as the seeded `came_from` (so the chosen one survives
///   the filter). For a straight line the two neighbours are exact opposites,
///   so this is just "came from behind me".
/// * No neighbours → `None` (cart can't move; caller leaves it parked).
///
/// `look_dir` is the dispatcher's horizontal facing (XZ); Y is ignored.
pub fn seed_came_from(
    cell: Cell,
    look_dir: glam::Vec3,
    is_track: impl Fn(Cell) -> bool,
) -> Option<Cell> {
    // Collect onward track neighbours.
    let mut neighbours: Vec<Cell> = rail::h_neighbours(cell)
        .into_iter()
        .filter(|n| is_track(*n))
        .collect();
    match neighbours.len() {
        // Terminus / isolated: no seed needed (None resolves the single
        // neighbour, or there's nowhere to go).
        0 | 1 => None,
        _ => {
            // Horizontal look direction (normalised; default to -Z if the
            // player is looking straight up/down so we still pick something).
            let look = {
                let h = glam::Vec3::new(look_dir.x, 0.0, look_dir.z);
                if h.length_squared() < 1e-6 {
                    glam::Vec3::new(0.0, 0.0, -1.0)
                } else {
                    h.normalize()
                }
            };
            // Choose the neighbour most aligned with the look direction.
            neighbours.sort_by(|a, b| {
                let da = neighbour_dir(cell, *a).dot(look);
                let db = neighbour_dir(cell, *b).dot(look);
                db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
            });
            let chosen = neighbours[0];
            // Seed came_from = the neighbour most opposite the chosen
            // direction so next_track_step filters it out and returns `chosen`.
            let chosen_dir = neighbour_dir(cell, chosen);
            let opposite = neighbours[1..]
                .iter()
                .copied()
                .min_by(|a, b| {
                    let da = neighbour_dir(cell, *a).dot(chosen_dir);
                    let db = neighbour_dir(cell, *b).dot(chosen_dir);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                // There are >= 2 neighbours, so [1..] is non-empty.
                .unwrap_or(neighbours[1]);
            Some(opposite)
        }
    }
}

/// Unit XZ direction from `from` to a neighbouring cell.
fn neighbour_dir(from: Cell, to: Cell) -> glam::Vec3 {
    glam::Vec3::new((to.0 - from.0) as f32, 0.0, (to.2 - from.2) as f32)
        .normalize_or_zero()
}

/// Advance a cart one tick along the track. Pure: `is_track` is injected, no
/// ECS or `World` access. Returns the new [`CartData`].
///
/// While `progress` accumulates past a cell boundary we hop to the next track
/// cell (stamping `came_from` and `facing`). At a terminus / junction the
/// stepper returns `None` and the cart parks (`speed → 0`, `progress → 0`).
pub fn advance(
    mut c: CartData,
    is_track: impl Fn(Cell) -> bool,
    powered: impl Fn(Cell) -> bool,
) -> CartData {
    if c.speed <= 0.0 {
        return c;
    }
    // A cart whose OWN cell is no longer track — e.g. the rail was mined out
    // from under it — can't roll; park it where it is rather than gliding on
    // through the gap (`next_track_step` only inspects neighbours, so without
    // this guard a cart would happily traverse air).
    if !is_track(c.cell) {
        c.progress = 0.0;
        c.speed = 0.0;
        return c;
    }
    // Powered Rail (Spec 48): a TRACK cell whose powered meta-bit is set (driven
    // by `power_tick`) boosts the cart this tick. Read from the cart's CURRENT
    // cell; the boost is transient (never written back to `speed`, so it doesn't
    // persist and a cart leaving the powered run drops straight back to base).
    let mult = if powered(c.cell) { POWERED_RAIL_SPEED_MULT } else { 1.0 };
    c.progress += c.speed * mult;
    // A single tick crosses at most one cell at the real CART_SPEED, but loop so
    // the logic is correct for any speed. Cap the iterations so a corrupt /
    // oversized speed on a CLOSED track ring (where `next_track_step` always
    // yields `Some`) can never spin unbounded and hang the tick.
    const MAX_STEPS_PER_TICK: u32 = 64;
    let mut steps = 0u32;
    while c.progress >= 1.0 && steps < MAX_STEPS_PER_TICK {
        steps += 1;
        match rail::next_track_step(c.cell, c.came_from, &is_track) {
            Some(next) => {
                c.facing = yaw_to(c.cell, next);
                c.came_from = Some(c.cell);
                c.cell = next;
                c.progress -= 1.0;
            }
            None => {
                // Terminus / junction: park exactly on the cell centre.
                c.progress = 0.0;
                c.speed = 0.0;
                break;
            }
        }
    }
    // If the step cap bailed with progress still over a cell (only reachable
    // with a corrupt huge speed on a ring), drop the overflow so it can't
    // accumulate across ticks.
    if c.progress >= 1.0 {
        c.progress = c.progress.fract();
    }
    c
}

/// World-space render position for a cart mid-roll: lerp between the current
/// cell centre and the next cell centre by `progress`. If there is no onward
/// cell (parked / terminus) the cart sits on its current cell centre.
pub fn render_position(c: &CartData, is_track: impl Fn(Cell) -> bool) -> glam::Vec3 {
    let here = cell_centre(c.cell);
    if c.progress <= 0.0 {
        return here;
    }
    match rail::next_track_step(c.cell, c.came_from, &is_track) {
        Some(next) => here.lerp(cell_centre(next), c.progress),
        None => here,
    }
}

/// Move every stack from `from` into `to`, slot by slot, respecting `to`'s
/// per-slot stacking + capacity. Anything that doesn't fit stays in `from`.
///
/// This is the single transfer primitive behind BOTH depot directions:
/// * **Load** (`from` = depot chest, `to` = cart cargo): pulls freight onto a
///   cart as it departs; cargo-full overflow is left in the depot.
/// * **Unload** (`from` = cart cargo, `to` = depot chest): dumps freight on
///   arrival; chest-full overflow stays in the cart.
///
/// Pure + reusable: both arguments are plain [`crate::chest::ChestData`], so it
/// is fully unit-testable with no `World`/ECS. Uses the chest's own
/// [`crate::chest::ChestData::try_insert`] so per-slot `max_stack` caps and the
/// existing stacking rules are honoured. Returns the number of item UNITS moved
/// (callers can short-circuit a no-op tick on `0`).
pub fn transfer_all(
    from: &mut crate::chest::ChestData,
    to: &mut crate::chest::ChestData,
) -> u32 {
    let mut moved: u32 = 0;
    for slot in from.slots.iter_mut() {
        let Some(stack) = slot.take() else { continue };
        let before = stack.count;
        let remainder = to.try_insert(stack);
        moved += u32::from(before - remainder.count);
        // Put back whatever didn't fit (count 0 ⇒ slot stays empty).
        *slot = if remainder.count > 0 { Some(remainder) } else { None };
    }
    moved
}

/// Dispatch a parked cart sitting on track cell `cell`, loading freight from an
/// adjacent depot chest first. Shared by the right-click handler
/// (`game_loop.rs`) and the headless test path so **one right-click = "load
/// what's here and go"** is exercised by exactly one implementation.
///
/// Steps, in order:
///   1. Seed the travel direction from `look_dir` (so a mid-line cart picks a
///      side; at a terminus the seed is `None` → roll back the way it came).
///   2. Find the parked cart on `cell` (`speed <= 0`), (re)seed its `came_from`
///      and set `speed = CART_SPEED`.
///   3. If a depot chest is adjacent (`depot_chest_for`), pull its freight into
///      the cart's cargo via [`transfer_all`] (cargo-cap respected; overflow
///      stays in the depot). No depot ⇒ just depart.
///
/// Returns `Some(entity)` of the dispatched cart (so the ride-along path in
/// Task 1.5 can attach a rider reference), or `None` if no parked cart was on
/// the cell. Borrow-safe: the chest mutation runs after the cart query borrow
/// is released, with the cargo taken out / written back (same split as the
/// unload phase in [`tick_carts`]).
pub fn dispatch_cart(
    ecs: &mut hecs::World,
    world: &mut crate::world::World,
    cell: Cell,
    look_dir: glam::Vec3,
) -> Option<hecs::Entity> {
    let is_track = |c: Cell| world.get_block(c.0, c.1, c.2) == rail::TRACK;
    let seed = seed_came_from(cell, look_dir, is_track);
    let depot = rail::depot_chest_for(cell, |c| world.chest_at((c.0, c.1, c.2)).is_some());

    let mut dispatched_id: Option<hecs::Entity> = None;
    for (id, cart) in ecs.query_mut::<&mut CartData>() {
        if cart.cell == cell && cart.speed <= 0.0 {
            cart.came_from = seed;
            cart.speed = CART_SPEED;
            dispatched_id = Some(id);
            break;
        }
    }
    let id = dispatched_id?;

    if let Some(depot) = depot {
        // BRIDGE: this auto-drains the adjacent depot chest with no ownership /
        // plot-protection check — parity with today's unprotected chests (the
        // normal chest-open path is also unguarded; chest ownership is out of
        // scope until the economy/plot features add it). Route this load
        // through the same access predicate the chest-open path gains, when
        // chest/plot protection lands, so freight can't bypass it.
        if let Ok(mut cart) = ecs.get::<&mut CartData>(id) {
            let mut cargo = std::mem::take(&mut cart.cargo);
            drop(cart);
            if let Some(chest) = world.chest_at_mut((depot.0, depot.1, depot.2)) {
                transfer_all(chest, &mut cargo);
            }
            if let Ok(mut cart) = ecs.get::<&mut CartData>(id) {
                cart.cargo = cargo;
            }
        }
    }
    Some(id)
}

/// The entity id of a cart currently occupying track `cell`, if any. Matches a
/// cart whose `CartData.cell == cell` regardless of whether it is parked or
/// mid-roll — a moving cart still anchors on its `cell` until it hops to the
/// next, so breaking the cell it sits on must catch it too. Returns the FIRST
/// match (a cell never legitimately holds two carts; CA3 placement guards
/// against stacking). Used by the CA4 break-path interception to decide whether
/// mining a TRACK cell breaches a cart or removes the rail.
pub fn cart_at_cell(ecs: &hecs::World, cell: Cell) -> Option<hecs::Entity> {
    ecs.query::<&CartData>()
        .iter()
        .find(|(_, c)| c.cell == cell)
        .map(|(id, _)| id)
}

/// CA4 — lay `amount` break-work against cart `entity`'s hull and, if that
/// crosses [`Hull::breach_max`], BREAK the cart open: despawn it, drop its Cart
/// item ([`Hull::item_id`]) and spill every non-empty cargo stack as a dropped
/// item entity at the cart's position. Returns `true` iff the cart broke this
/// call (so the caller can play the break sound / reset mining state), `false`
/// if it only accrued breach (or the entity wasn't a cart).
///
/// This is the breach-to-break CORE, factored out of `game_loop.rs` so the
/// despawn-and-drop logic is exercised by a real test even though the input
/// wiring (which break-tick laid the work) is a client-input / playtest concern.
/// Both the game-loop interception and the headless test call THIS — one source
/// of truth for "what breaking a cart does".
///
/// Cross-platform: pure ECS + [`crate::entity::spawn_item`] (the same drop path
/// mob loot uses), no native-only deps. The drop `Position` is the cart's live
/// render `Position` if present, else the cart cell centre.
pub fn apply_breach(
    ecs: &mut hecs::World,
    entity: hecs::Entity,
    amount: f32,
) -> bool {
    // Accrue breach + read the hull / cargo / drop position while we hold the
    // borrow, then release it BEFORE spawning drops / despawning (spawn_item
    // and despawn both need a fresh &mut World).
    let (hull, broke, drop_pos, cargo) = {
        let Ok(mut cart) = ecs.get::<&mut CartData>(entity) else {
            return false;
        };
        cart.breach += amount.max(0.0);
        let hull = cart.hull;
        let broke = cart.breach >= hull.breach_max();
        if !broke {
            return false;
        }
        // Breaking: snapshot the cargo to spill + a drop anchor. Prefer the live
        // render Position; fall back to the cell centre if it has none.
        let cargo = std::mem::take(&mut cart.cargo);
        (hull, broke, cell_centre(cart.cell), cargo)
    };
    debug_assert!(broke);

    let drop_pos = ecs
        .get::<&crate::entity::Position>(entity)
        .map(|p| p.0)
        .unwrap_or(drop_pos);

    // Drop the cart item itself (the "pickup" — breaking returns the cart).
    crate::entity::spawn_item(
        ecs,
        drop_pos,
        crate::item::ItemStack::new_material(hull.item_id(), 1),
        0xCA4_u32,
    );
    // Spill every non-empty cargo stack. A distinct per-slot seed scatters them
    // so they don't stack in one column (same spread convention as mob loot).
    for (k, slot) in cargo.slots.into_iter().enumerate() {
        if let Some(stack) = slot {
            crate::entity::spawn_item(ecs, drop_pos, stack, (k as u32).wrapping_mul(7919) ^ 0xCA4);
        }
    }
    // Despawn the cart. The entity diff (`entity_broadcast`) drops its ProtocolId
    // out of the alive-set on the next tick → emits exactly one despawn delta,
    // the same path a dead mob takes.
    let _ = ecs.despawn(entity);
    true
}

/// Spawn a parked cart on track cell `cell`. The cart carries `Position`,
/// `CartEntity` and `CartData` only — NO `OnGround`, so the physics tick
/// ignores it (the cart is rail-driven, not gravity-driven). Returns the
/// entity id so callers can attach more components later (e.g. `ProtocolId`
/// for Task 1.7).
pub fn spawn_cart(ecs: &mut hecs::World, cell: Cell) -> hecs::Entity {
    // A bare `spawn_cart` is a wood cart — the default hull tier (CA1).
    spawn_cart_with_hull(ecs, cell, Hull::default())
}

/// Spawn a parked cart on track cell `cell` with a specific [`Hull`] armour tier
/// (CA1). Identical to [`spawn_cart`] in every other respect (a parked
/// `Position` + `CartEntity` + `CartData`, no `OnGround`); only the hull differs.
/// Used by the place-from-item path (CA3) so an iron/diamond cart item spawns the
/// matching armoured cart. Returns the entity id for further component attachment.
pub fn spawn_cart_with_hull(
    ecs: &mut hecs::World,
    cell: Cell,
    hull: Hull,
) -> hecs::Entity {
    ecs.spawn((
        crate::entity::Position(cell_centre(cell)),
        CartEntity,
        CartData {
            cell,
            cargo: crate::chest::ChestData::default(),
            hull,
            ..Default::default()
        },
    ))
}

/// Re-spawn a cart from persisted [`CartData`] (Task 1.6 — save/load restore).
/// Spawns the same `Position` + `CartEntity` + `CartData` trio as [`spawn_cart`],
/// but seeds the full saved state (cell / came_from / progress / speed / facing /
/// cargo) instead of a fresh parked cart. The `Position` is derived from
/// `cell_centre(data.cell)` as a placeholder anchor; the first [`tick_carts`]
/// pass recomputes the exact lerped render position from `progress`, so a cart
/// saved mid-cell visually snaps to the correct spot on the next tick rather
/// than persisting a stale anchor. Returns the entity id so callers (e.g. Task
/// 1.7's broadcast) can attach further components.
pub fn spawn_cart_from(ecs: &mut hecs::World, mut data: CartData) -> hecs::Entity {
    // Sanitise persisted state against a corrupt save: a non-finite / negative
    // `speed` parks the cart, and `progress` is clamped into `[0,1)` so a bad
    // value can't drive a NaN render position or unbounded `advance`.
    if !data.speed.is_finite() || data.speed < 0.0 {
        data.speed = 0.0;
    }
    data.progress = if data.progress.is_finite() {
        data.progress.clamp(0.0, 0.999_999)
    } else {
        0.0
    };
    ecs.spawn((
        crate::entity::Position(cell_centre(data.cell)),
        CartEntity,
        data,
    ))
}

/// Run one cart tick for every cart in the ECS. Advances each cart's
/// `CartData` along the track (reading `World` for "is this cell a track?"),
/// writes the lerped render `Position`, and — when a cart JUST parked at a
/// terminus this tick — dumps its cargo into the adjacent depot chest. Overflow
/// stays in the cart. Cross-platform pure logic — no native-only deps.
///
/// Takes `&mut World` because the unload step mutates a depot chest. The
/// immutable block reads (`is_track`) and the mutable chest write can't overlap
/// the same borrow, so this runs in two phases:
///   * **Phase A** — advance every cart + write `Position` using only immutable
///     `world` reads, collecting the `(entity, cell)` of carts that transitioned
///     moving→parked-at-terminus this tick into a small `Vec`.
///   * **Phase B** — drop the immutable borrow, then for each just-parked cart
///     resolve its depot chest and `transfer_all` the cargo in (mutable world).
pub fn tick_carts(ecs: &mut hecs::World, world: &mut crate::world::World) {
    // ── Phase A: advance carts (immutable world reads only) ──────────────────
    let mut just_parked: Vec<(hecs::Entity, Cell)> = Vec::new();
    {
        let is_track = |cell: Cell| world.get_block(cell.0, cell.1, cell.2) == rail::TRACK;
        // Spec 48 — a TRACK cell is "powered" when its meta state-bit is set (the
        // powered-rail flag `power_tick` rides on an adjacent energised cable).
        let powered = |cell: Cell| {
            world.get_block(cell.0, cell.1, cell.2) == rail::TRACK
                && crate::meta::state(world.meta_at(cell.0, cell.1, cell.2)) != 0
        };
        for (id, (pos, cart)) in ecs.query_mut::<(&mut crate::entity::Position, &mut CartData)>() {
            let was_moving = cart.speed > 0.0;
            *cart = advance(std::mem::take(cart), is_track, powered);
            pos.0 = render_position(cart, is_track);
            // "Just parked at a terminus this tick": it was rolling, now it's
            // stopped exactly on a cell whose stepper yields no onward step
            // (a terminus — junctions also yield None, and a depot beside a
            // junction is a legitimate unload point too).
            let parked_at_terminus = cart.speed == 0.0
                && cart.progress == 0.0
                && rail::next_track_step(cart.cell, cart.came_from, is_track).is_none();
            if was_moving && parked_at_terminus {
                just_parked.push((id, cart.cell));
            }
        }
    }

    // ── Phase B: unload each just-parked cart into its depot chest (mut) ──────
    for (id, cell) in just_parked {
        let is_chest = |c: Cell| world.chest_at((c.0, c.1, c.2)).is_some();
        let Some(depot) = rail::depot_chest_for(cell, is_chest) else { continue };
        // Take the cargo out of the cart, transfer into the chest, then write
        // the (possibly-overflowed) remainder back — avoids holding a cart
        // borrow across the world borrow.
        let mut cargo = match ecs.get::<&mut CartData>(id) {
            Ok(mut c) => std::mem::take(&mut c.cargo),
            Err(_) => continue,
        };
        if let Some(chest) = world.chest_at_mut((depot.0, depot.1, depot.2)) {
            transfer_all(&mut cargo, chest);
        }
        if let Ok(mut c) = ecs.get::<&mut CartData>(id) {
            c.cargo = cargo;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Treat the listed cells as the only track, everything else air.
    fn line(cells: &[Cell]) -> impl Fn(Cell) -> bool + '_ {
        move |c| cells.contains(&c)
    }

    // --- yaw_to ---

    #[test]
    fn yaw_to_matches_camera_convention() {
        // Heading -Z is yaw 0 (Camera::forward at yaw 0 points -Z).
        assert!((yaw_to((0, 0, 0), (0, 0, -1))).abs() < 1e-5);
        // Heading +X is yaw -pi/2: forward_xz = (-sin, -cos) = (1, 0)
        // ⇒ sin(yaw) = -1 ⇒ yaw = -pi/2.
        assert!((yaw_to((0, 0, 0), (1, 0, 0)) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // Heading -X is yaw +pi/2.
        assert!((yaw_to((0, 0, 0), (-1, 0, 0)) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // Heading +Z is yaw pi (or -pi).
        assert!((yaw_to((0, 0, 0), (0, 0, 1)).abs() - std::f32::consts::PI).abs() < 1e-5);
    }

    // --- advance (the timed-travel core) ---

    #[test]
    fn parked_cart_does_not_move() {
        let l = [(0, 0, 0), (1, 0, 0)];
        let c = CartData { cell: (0, 0, 0), speed: 0.0, ..Default::default() };
        let after = advance(c, line(&l), |_| false);
        assert_eq!(after.cell, (0, 0, 0));
        assert_eq!(after.progress, 0.0);
    }

    #[test]
    fn dispatched_cart_reaches_terminus_and_parks() {
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        let mut c = CartData { cell: (0, 0, 0), speed: 0.5, ..Default::default() };
        for _ in 0..20 {
            c = advance(c, line(&l), |_| false);
        }
        assert_eq!(c.cell, (2, 0, 0));
        assert_eq!(c.speed, 0.0, "parks at the end");
        assert_eq!(c.progress, 0.0, "parked exactly on the terminus cell");
    }

    #[test]
    fn cart_advances_one_cell_when_progress_crosses_one() {
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        // From a terminus, came_from None resolves the single neighbour.
        let mut c = CartData { cell: (0, 0, 0), speed: 0.6, ..Default::default() };
        c = advance(c, line(&l), |_| false); // progress 0.6, still on cell 0
        assert_eq!(c.cell, (0, 0, 0));
        assert!((c.progress - 0.6).abs() < 1e-5);
        c = advance(c, line(&l), |_| false); // progress 1.2 → hop to cell 1, progress 0.2
        assert_eq!(c.cell, (1, 0, 0));
        assert!((c.progress - 0.2).abs() < 1e-5);
        assert_eq!(c.came_from, Some((0, 0, 0)));
    }

    #[test]
    fn cart_follows_a_corner() {
        // East from origin then north — facing updates on the turn.
        let l = [(0, 0, 0), (1, 0, 0), (1, 0, 1)];
        let mut c = CartData { cell: (0, 0, 0), speed: 1.0, ..Default::default() };
        c = advance(c, line(&l), |_| false); // → cell (1,0,0)
        assert_eq!(c.cell, (1, 0, 0));
        c = advance(c, line(&l), |_| false); // → corner turns to (1,0,1)
        assert_eq!(c.cell, (1, 0, 1));
        // After turning to head +Z, facing is yaw pi.
        assert!((c.facing.abs() - std::f32::consts::PI).abs() < 1e-4);
    }

    #[test]
    fn cart_parks_at_junction() {
        // 3-way junction out of (1,0,0): cart arriving from (0,0,0) faces two
        // onward choices and parks (no switching in Phase 1).
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0), (1, 0, 1)];
        let mut c = CartData {
            cell: (0, 0, 0),
            speed: 1.0,
            ..Default::default()
        };
        c = advance(c, line(&l), |_| false); // → (1,0,0), arrives from (0,0,0)
        assert_eq!(c.cell, (1, 0, 0));
        c = advance(c, line(&l), |_| false); // junction → park
        assert_eq!(c.cell, (1, 0, 0), "parks at the junction");
        assert_eq!(c.speed, 0.0);
    }

    #[test]
    fn cart_on_a_non_track_cell_parks() {
        // The rail was mined out from under a rolling cart: its current cell is
        // no longer track, so it parks rather than gliding on through the gap.
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        let c = CartData {
            cell: (5, 0, 0), // NOT in the track set
            speed: CART_SPEED,
            ..Default::default()
        };
        let after = advance(c, line(&l), |_| false);
        assert_eq!(after.speed, 0.0, "a cart off the rails parks");
        assert_eq!(after.progress, 0.0);
    }

    #[test]
    fn powered_rail_speeds_the_cart() {
        // Spec 48 — Powered Rail: a cart on a TRACK cell whose powered meta bit is
        // set advances further in one tick than the same cart on an unpowered cell.
        // `power_tick` sets that bit; `advance` reads it via the `powered` closure.
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        let mk = || CartData { cell: (0, 0, 0), speed: CART_SPEED, ..Default::default() };
        let unpowered = advance(mk(), line(&l), |_| false);
        let powered = advance(mk(), line(&l), |_| true);
        assert!(
            powered.progress > unpowered.progress,
            "a powered rail must advance the cart further per tick"
        );
        // Exactly the configured multiplier (both carts started identical).
        assert!(
            (powered.progress - unpowered.progress * POWERED_RAIL_SPEED_MULT).abs() < 1e-6,
            "powered progress = unpowered * POWERED_RAIL_SPEED_MULT"
        );
    }

    #[test]
    fn advance_caps_steps_on_a_closed_loop_with_huge_speed() {
        // A closed 2×2 ring (every cell keeps exactly one onward neighbour once
        // `came_from` is set, so `next_track_step` always returns `Some`) plus a
        // corrupt huge speed must NOT loop unbounded — the step cap bails and
        // progress is clamped back into a cell. (Without the cap this hangs.)
        let ring = [(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)];
        let c = CartData {
            cell: (0, 0, 0),
            came_from: Some((0, 0, 1)),
            speed: 1.0e6,
            ..Default::default()
        };
        let after = advance(c, line(&ring), |_| false);
        assert!(after.progress < 1.0, "progress clamped after the step cap");
    }

    #[test]
    fn spawn_cart_from_sanitises_corrupt_speed_and_progress() {
        let mut ecs = hecs::World::new();
        let id = spawn_cart_from(
            &mut ecs,
            CartData {
                cell: (0, 0, 0),
                speed: f32::NAN,
                progress: 5.0,
                ..Default::default()
            },
        );
        let c = ecs.get::<&CartData>(id).unwrap();
        assert_eq!(c.speed, 0.0, "non-finite speed parks the cart");
        assert!(
            c.progress >= 0.0 && c.progress < 1.0,
            "progress clamped into [0,1)"
        );
    }

    // --- seed_came_from (direction seeding on dispatch) ---

    #[test]
    fn seed_terminus_needs_no_seed() {
        // One neighbour ⇒ next_track_step already works with came_from None.
        let l = [(0, 0, 0), (1, 0, 0)];
        let seed = seed_came_from((0, 0, 0), glam::Vec3::new(1.0, 0.0, 0.0), line(&l));
        assert_eq!(seed, None);
    }

    #[test]
    fn seed_mid_line_picks_look_direction() {
        // Cart in the MIDDLE of a straight E-W line. Looking +X (east) must
        // seed came_from = west neighbour so next_track_step returns east.
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        let look_east = glam::Vec3::new(1.0, 0.0, 0.0);
        let seed = seed_came_from((1, 0, 0), look_east, line(&l));
        assert_eq!(seed, Some((0, 0, 0)), "came_from = west so onward = east");
        // And the stepper resolves to the east neighbour with that seed.
        assert_eq!(
            rail::next_track_step((1, 0, 0), seed, line(&l)),
            Some((2, 0, 0)),
        );
    }

    #[test]
    fn seed_mid_line_other_direction() {
        // Same line, looking -X (west): onward should be west (0,0,0).
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        let look_west = glam::Vec3::new(-1.0, 0.0, 0.0);
        let seed = seed_came_from((1, 0, 0), look_west, line(&l));
        assert_eq!(seed, Some((2, 0, 0)), "came_from = east so onward = west");
        assert_eq!(
            rail::next_track_step((1, 0, 0), seed, line(&l)),
            Some((0, 0, 0)),
        );
    }

    #[test]
    fn seed_then_advance_moves_off_a_mid_line_cart() {
        // End-to-end: a parked cart mid-line, seeded toward +X, then dispatched
        // at CART_SPEED, eventually reaches the east terminus.
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        let mut c = CartData { cell: (1, 0, 0), ..Default::default() };
        c.came_from = seed_came_from(c.cell, glam::Vec3::new(1.0, 0.0, 0.0), line(&l));
        c.speed = 1.0;
        for _ in 0..5 {
            c = advance(c, line(&l), |_| false);
        }
        assert_eq!(c.cell, (2, 0, 0), "seeded cart rolls east to the terminus");
        assert_eq!(c.speed, 0.0);
    }

    #[test]
    fn redispatch_from_terminus_sends_cart_back() {
        // Models the right-click dispatch contract: a cart that has ALREADY
        // run to the east terminus is parked there with a stale
        // `came_from = Some(west neighbour)`. Re-dispatching must overwrite
        // came_from with a fresh seed (None at a terminus) so the cart rolls
        // back west — NOT re-park instantly because the only exit equals the
        // old came_from. Guards against the silent right-click no-op.
        let l = [(0, 0, 0), (1, 0, 0), (2, 0, 0)];
        let mut c = CartData {
            cell: (2, 0, 0),
            came_from: Some((1, 0, 0)), // arrived from the west on the first run
            speed: 0.0,
            ..Default::default()
        };
        // Dispatch (mirrors game_loop.rs): always (re)seed from look, set speed.
        c.came_from = seed_came_from(c.cell, glam::Vec3::new(-1.0, 0.0, 0.0), line(&l));
        assert_eq!(c.came_from, None, "terminus seed clears stale history");
        c.speed = 1.0;
        for _ in 0..5 {
            c = advance(c, line(&l), |_| false);
        }
        assert_eq!(c.cell, (0, 0, 0), "re-dispatched cart rolls back to the far end");
        assert_eq!(c.speed, 0.0);
    }

    // --- rider_eye_position (ride-along follow transform, Task 1.5) ---

    #[test]
    fn rider_eye_sits_above_the_cart_body_by_seat_lift_plus_eye_height() {
        // Cart anchored at world (1.5, 0.1, 0.5). The rider's eye must sit
        // directly above it by SEAT_LIFT (so the camera clears the cart body)
        // plus the player's own eye height, with X/Z unchanged (you ride AT the
        // cart's XZ, looking out from a seat).
        let cart_pos = glam::Vec3::new(1.5, 0.1, 0.5);
        let eye_height = 1.62; // standard player eye height, passed in (not hardcoded)
        let eye = rider_eye_position(cart_pos, eye_height);
        assert!((eye.x - cart_pos.x).abs() < 1e-6, "rider keeps the cart's X");
        assert!((eye.z - cart_pos.z).abs() < 1e-6, "rider keeps the cart's Z");
        assert!(
            (eye.y - (cart_pos.y + SEAT_LIFT + eye_height)).abs() < 1e-6,
            "eye = cart Y + SEAT_LIFT + eye_height",
        );
        // The seat lift is positive so the camera is above the cart body, never
        // sunk inside it.
        assert!(SEAT_LIFT > 0.0, "seat lift must raise the camera above the body");
        assert!(eye.y > cart_pos.y, "eye is strictly above the cart anchor");
    }

    #[test]
    fn rider_eye_tracks_the_cart_as_it_moves() {
        // Two cart positions a cell apart → the rider eye moves by exactly the
        // same XZ delta (pure translation; the offset is constant).
        let eye_height = 1.62;
        let a = rider_eye_position(glam::Vec3::new(0.5, 0.1, 0.5), eye_height);
        let b = rider_eye_position(glam::Vec3::new(1.5, 0.1, 0.5), eye_height);
        assert!((b.x - a.x - 1.0).abs() < 1e-6, "eye shifts +1 in X with the cart");
        assert!((b.z - a.z).abs() < 1e-6);
        assert!((b.y - a.y).abs() < 1e-6, "constant vertical offset");
    }

    // --- render_position ---

    #[test]
    fn render_position_lerps_between_cell_centres() {
        let l = [(0, 0, 0), (1, 0, 0)];
        let c = CartData { cell: (0, 0, 0), progress: 0.5, speed: 1.0, ..Default::default() };
        let p = render_position(&c, line(&l));
        // Halfway between (0.5, 0.1, 0.5) and (1.5, 0.1, 0.5) = (1.0, 0.1, 0.5).
        assert!((p.x - 1.0).abs() < 1e-5);
        assert!((p.y - CART_Y_OFFSET).abs() < 1e-5);
        assert!((p.z - 0.5).abs() < 1e-5);
    }

    #[test]
    fn render_position_parked_sits_on_cell_centre() {
        let l = [(0, 0, 0)];
        let c = CartData { cell: (0, 0, 0), progress: 0.0, ..Default::default() };
        let p = render_position(&c, line(&l));
        assert_eq!(p, cell_centre((0, 0, 0)));
    }

    // --- spawn_cart + tick_carts (ECS) ---

    // --- Hull (armour tier, CA1) ---

    #[test]
    fn hull_defaults_to_wood() {
        // A fresh / parked cart is unarmoured wood — the default tier.
        assert_eq!(Hull::default(), Hull::Wood);
        assert_eq!(CartData::default().hull, Hull::Wood);
    }

    #[test]
    fn hull_hardness_ascends_wood_iron_diamond() {
        // The breach-tier helper orders the tiers so CA4 can gate breaching on it.
        assert!(Hull::Wood.hardness() < Hull::Iron.hardness());
        assert!(Hull::Iron.hardness() < Hull::Diamond.hardness());
    }

    #[test]
    fn spawn_cart_is_wood_hulled() {
        let mut ecs = hecs::World::new();
        let id = spawn_cart(&mut ecs, (0, 64, 0));
        assert_eq!(ecs.get::<&CartData>(id).unwrap().hull, Hull::Wood);
    }

    #[test]
    fn spawn_cart_with_hull_sets_the_tier() {
        let mut ecs = hecs::World::new();
        let id = spawn_cart_with_hull(&mut ecs, (0, 64, 0), Hull::Iron);
        let c = ecs.get::<&CartData>(id).unwrap();
        assert_eq!(c.hull, Hull::Iron, "the cart carries the requested hull tier");
        assert_eq!(c.cell, (0, 64, 0));
        assert_eq!(c.speed, 0.0, "still spawns parked");
    }

    #[test]
    fn spawn_cart_from_round_trips_the_hull() {
        // The save-restore path must carry the full hull tier through verbatim.
        let mut ecs = hecs::World::new();
        let id = spawn_cart_from(
            &mut ecs,
            CartData { cell: (1, 2, 3), hull: Hull::Diamond, ..Default::default() },
        );
        assert_eq!(ecs.get::<&CartData>(id).unwrap().hull, Hull::Diamond);
    }

    #[test]
    fn spawn_cart_has_no_on_ground_component() {
        let mut ecs = hecs::World::new();
        let id = spawn_cart(&mut ecs, (3, 64, 7));
        // Has the cart components.
        assert!(ecs.get::<&CartData>(id).is_ok());
        assert!(ecs.get::<&crate::entity::Position>(id).is_ok());
        assert!(ecs.get::<&CartEntity>(id).is_ok());
        // Must NOT carry OnGround — that's what keeps tick_entities off it.
        assert!(
            ecs.get::<&crate::entity::OnGround>(id).is_err(),
            "carts must omit OnGround so the physics tick skips them",
        );
        // Parked at the cell centre.
        let pos = ecs.get::<&crate::entity::Position>(id).unwrap().0;
        assert_eq!(pos, cell_centre((3, 64, 7)));
    }

    #[test]
    fn tick_carts_rolls_a_dispatched_cart_along_real_world_track() {
        use crate::block;
        let mut world = crate::world::World::new();
        // Lay a 4-cell E-W track at y=64.
        for x in 0..4 {
            world.set_block(x, 64, 0, rail::TRACK);
        }
        let mut ecs = hecs::World::new();
        let id = spawn_cart(&mut ecs, (0, 64, 0));
        // Dispatch east: from a terminus, came_from None heads to the only
        // neighbour (1,64,0). Speed high enough to make visible progress.
        {
            let mut cart = ecs.get::<&mut CartData>(id).unwrap();
            cart.speed = 0.5;
        }
        // Run enough ticks to cross the whole line.
        for _ in 0..40 {
            tick_carts(&mut ecs, &mut world);
        }
        let cart = ecs.get::<&CartData>(id).unwrap();
        assert_eq!(cart.cell, (3, 64, 0), "rolled to the east terminus");
        assert_eq!(cart.speed, 0.0, "parked at the terminus");
        let pos = ecs.get::<&crate::entity::Position>(id).unwrap().0;
        assert_eq!(pos, cell_centre((3, 64, 0)));
        // Sanity: the world still reads TRACK where we laid it.
        assert_eq!(world.get_block(3, 64, 0), rail::TRACK);
        assert_ne!(rail::TRACK, block::AIR);
    }

    // --- transfer_all (the depot load/unload primitive) ---

    fn stack(id: crate::item::MaterialId, n: u8) -> crate::item::ItemStack {
        crate::item::ItemStack::new_material(id, n)
    }

    #[test]
    fn transfer_all_moves_every_stack_into_an_empty_destination() {
        use crate::item::MaterialId;
        let mut from = crate::chest::ChestData::new();
        from.slots[0] = Some(stack(MaterialId::IronIngot, 5));
        from.slots[3] = Some(stack(MaterialId::Bread, 2));
        let mut to = crate::chest::ChestData::new();

        let moved = transfer_all(&mut from, &mut to);

        assert_eq!(moved, 7, "5 iron + 2 bread = 7 units moved");
        assert_eq!(from.occupied(), 0, "source fully drained");
        assert_eq!(to.occupied(), 2, "both stacks landed");
    }

    #[test]
    fn transfer_all_leaves_overflow_in_source_when_destination_is_full() {
        use crate::item::MaterialId;
        // Destination has exactly one free slot; source has two stacks of
        // different (non-stacking) items → one fits, one stays behind.
        let mut from = crate::chest::ChestData::new();
        from.slots[0] = Some(stack(MaterialId::IronIngot, 4));
        from.slots[1] = Some(stack(MaterialId::Bread, 4));
        let mut to = crate::chest::ChestData::new();
        // Fill all but the last slot with a non-stacking placeholder.
        for i in 0..(crate::chest::CHEST_SLOTS - 1) {
            to.slots[i] = Some(stack(MaterialId::Stick, u8::MAX));
        }

        let moved = transfer_all(&mut from, &mut to);

        assert_eq!(moved, 4, "only the first stack fit the single free slot");
        assert_eq!(from.occupied(), 1, "the overflow stack stays in the source");
        assert_eq!(to.occupied(), crate::chest::CHEST_SLOTS, "destination now full");
    }

    #[test]
    fn transfer_all_empty_source_is_a_noop() {
        let mut from = crate::chest::ChestData::new();
        let mut to = crate::chest::ChestData::new();
        to.slots[0] = Some(stack(crate::item::MaterialId::Bread, 1));
        assert_eq!(transfer_all(&mut from, &mut to), 0);
        assert_eq!(to.occupied(), 1, "destination untouched");
    }

    #[test]
    fn tick_carts_unloads_cargo_into_depot_chest_on_arrival() {
        use crate::item::MaterialId;
        let mut world = crate::world::World::new();
        // 3-cell E-W track at y=64; depot chest beside the east terminus.
        for x in 0..3 {
            world.set_block(x, 64, 0, rail::TRACK);
        }
        let depot = (3, 64, 0); // +X neighbour of the (2,64,0) terminus
        world.insert_chest(depot, crate::chest::ChestData::new());

        let mut ecs = hecs::World::new();
        let id = spawn_cart(&mut ecs, (0, 64, 0));
        {
            let mut cart = ecs.get::<&mut CartData>(id).unwrap();
            cart.cargo.slots[0] = Some(stack(MaterialId::IronIngot, 9));
            cart.speed = 0.5; // dispatch east from the terminus
        }
        // Roll to the east terminus and park.
        for _ in 0..40 {
            tick_carts(&mut ecs, &mut world);
        }
        let cart = ecs.get::<&CartData>(id).unwrap();
        assert_eq!(cart.cell, (2, 64, 0), "parked at the east terminus");
        assert_eq!(cart.speed, 0.0);
        assert_eq!(cart.cargo.occupied(), 0, "cargo emptied into the depot");
        let chest = world.chest_at(depot).expect("depot still present");
        assert_eq!(chest.occupied(), 1, "iron landed in the depot");
        assert_eq!(chest.slots[0].as_ref().unwrap().count, 9);
    }

    #[test]
    fn tick_carts_leaves_a_parked_cart_in_place() {
        let mut world = crate::world::World::new();
        world.set_block(0, 64, 0, rail::TRACK);
        let mut ecs = hecs::World::new();
        let id = spawn_cart(&mut ecs, (0, 64, 0));
        for _ in 0..10 {
            tick_carts(&mut ecs, &mut world);
        }
        let cart = ecs.get::<&CartData>(id).unwrap();
        assert_eq!(cart.cell, (0, 64, 0));
        assert_eq!(cart.speed, 0.0);
    }

    // --- Hull ↔ cart-item mapping (CA2; reused by CA3 placement + CA4 drops) ---

    #[test]
    fn hull_item_id_maps_each_tier_to_its_cart_material() {
        use crate::item::MaterialId;
        assert_eq!(Hull::Wood.item_id(), MaterialId::WoodCart);
        assert_eq!(Hull::Iron.item_id(), MaterialId::IronCart);
        assert_eq!(Hull::Diamond.item_id(), MaterialId::DiamondCart);
    }

    #[test]
    fn cart_hull_for_item_maps_cart_materials_back_to_their_tier() {
        use crate::item::MaterialId;
        assert_eq!(cart_hull_for_item(MaterialId::WoodCart), Some(Hull::Wood));
        assert_eq!(cart_hull_for_item(MaterialId::IronCart), Some(Hull::Iron));
        assert_eq!(cart_hull_for_item(MaterialId::DiamondCart), Some(Hull::Diamond));
    }

    #[test]
    fn cart_hull_for_item_returns_none_for_non_cart_materials() {
        use crate::item::MaterialId;
        assert_eq!(cart_hull_for_item(MaterialId::IronIngot), None);
        assert_eq!(cart_hull_for_item(MaterialId::Diamond), None);
        assert_eq!(cart_hull_for_item(MaterialId::Stick), None);
    }

    #[test]
    fn hull_item_round_trips_through_its_material() {
        for hull in [Hull::Wood, Hull::Iron, Hull::Diamond] {
            assert_eq!(cart_hull_for_item(hull.item_id()), Some(hull));
        }
    }

    // --- CA4 breach threshold (pure) ---

    #[test]
    fn breach_max_ascends_wood_iron_diamond() {
        // The armour effect: a sturdier hull takes MORE break-work to breach.
        // Only the ordering is contractual.
        assert!(Hull::Wood.breach_max() < Hull::Iron.breach_max());
        assert!(Hull::Iron.breach_max() < Hull::Diamond.breach_max());
    }

    #[test]
    fn breach_max_tracks_hardness_ordering() {
        // breach_max is derived from hardness, so the two ordinal contracts can
        // never disagree — a tier that's harder is always harder to breach.
        for (a, b) in [(Hull::Wood, Hull::Iron), (Hull::Iron, Hull::Diamond)] {
            assert_eq!(
                a.hardness() < b.hardness(),
                a.breach_max() < b.breach_max(),
            );
        }
    }

    #[test]
    fn hits_to_break_orders_tiers_and_diamond_takes_more_than_wood() {
        // At a fixed per-hit breach increment, a diamond cart needs strictly more
        // hits than wood — the whole point of "armour resists smashing".
        let per_hit = 1.0;
        let wood = Hull::Wood.hits_to_break(per_hit);
        let iron = Hull::Iron.hits_to_break(per_hit);
        let diamond = Hull::Diamond.hits_to_break(per_hit);
        assert!(wood < iron, "iron survives more hits than wood");
        assert!(iron < diamond, "diamond survives more hits than iron");
        assert!(wood >= 1, "even wood takes at least one hit");
    }

    #[test]
    fn hits_to_break_guards_nonpositive_per_hit() {
        // A zero / negative / non-finite increment can never break the cart —
        // return u32::MAX rather than dividing by zero.
        assert_eq!(Hull::Wood.hits_to_break(0.0), u32::MAX);
        assert_eq!(Hull::Iron.hits_to_break(-1.0), u32::MAX);
        assert_eq!(Hull::Diamond.hits_to_break(f32::NAN), u32::MAX);
    }

    #[test]
    fn cartdata_default_breach_is_zero() {
        // The transient breach accumulator starts clean.
        assert_eq!(CartData::default().breach, 0.0);
    }

    // --- CA4 apply_breach (the break-the-cart ECS core) ---

    #[test]
    fn apply_breach_below_threshold_only_accrues() {
        // One small breach increment on a Diamond cart (threshold 120) must NOT
        // break it — it just banks the work and the cart stays in the ECS.
        let mut ecs = hecs::World::new();
        let id = spawn_cart_with_hull(&mut ecs, (0, 64, 0), Hull::Diamond);
        let broke = apply_breach(&mut ecs, id, 1.0);
        assert!(!broke, "one tick can't breach a diamond hull");
        assert!(ecs.get::<&CartData>(id).is_ok(), "cart still alive");
        let c = ecs.get::<&CartData>(id).unwrap();
        assert!((c.breach - 1.0).abs() < 1e-6, "breach accrued");
    }

    #[test]
    fn apply_breach_breaks_a_wood_cart_and_drops_item_plus_cargo() {
        use crate::item::{Item, MaterialId};
        let mut ecs = hecs::World::new();
        let id = spawn_cart_with_hull(&mut ecs, (2, 64, 3), Hull::Wood);
        // Load two distinct cargo stacks.
        {
            let mut cart = ecs.get::<&mut CartData>(id).unwrap();
            cart.cargo.slots[0] = Some(stack(MaterialId::IronIngot, 5));
            cart.cargo.slots[4] = Some(stack(MaterialId::Bread, 2));
        }
        // Lay the full Wood threshold in one go → it breaks.
        let broke = apply_breach(&mut ecs, id, Hull::Wood.breach_max());
        assert!(broke, "wood cart breaks once breach >= breach_max");
        // (a) the cart entity is gone.
        assert!(ecs.get::<&CartData>(id).is_err(), "cart despawned");
        assert_eq!(ecs.query::<&CartData>().iter().count(), 0, "no carts remain");
        // (b) a WoodCart item + both cargo stacks dropped as item entities.
        let drops: Vec<(MaterialId, u8)> = ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter_map(|(_, ie)| match &ie.stack.item {
                Item::Material(m) => Some((*m, ie.stack.count)),
                _ => None,
            })
            .collect();
        assert_eq!(drops.len(), 3, "cart item + 2 cargo stacks = 3 drops");
        assert!(drops.iter().any(|(m, n)| *m == MaterialId::WoodCart && *n == 1),
            "the WoodCart pickup dropped");
        assert!(drops.iter().any(|(m, n)| *m == MaterialId::IronIngot && *n == 5),
            "iron cargo spilled");
        assert!(drops.iter().any(|(m, n)| *m == MaterialId::Bread && *n == 2),
            "bread cargo spilled");
    }

    #[test]
    fn apply_breach_drops_the_matching_hull_item() {
        use crate::item::{Item, MaterialId};
        // An Iron cart breaks to an IronCart item (Hull::item_id), not a WoodCart.
        let mut ecs = hecs::World::new();
        let id = spawn_cart_with_hull(&mut ecs, (0, 64, 0), Hull::Iron);
        assert!(apply_breach(&mut ecs, id, Hull::Iron.breach_max()));
        let dropped: Vec<MaterialId> = ecs
            .query::<&crate::entity::ItemEntity>()
            .iter()
            .filter_map(|(_, ie)| match &ie.stack.item {
                Item::Material(m) => Some(*m),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, vec![MaterialId::IronCart], "iron hull → IronCart item");
    }

    #[test]
    fn apply_breach_diamond_takes_more_breach_than_wood() {
        // The armour effect end-to-end at the ECS level: the SAME per-tick breach
        // increment breaks a wood cart in fewer applications than a diamond one.
        let increment = 5.0;
        let count_to_break = |hull: Hull| -> u32 {
            let mut ecs = hecs::World::new();
            let id = spawn_cart_with_hull(&mut ecs, (0, 64, 0), hull);
            let mut hits = 0;
            loop {
                hits += 1;
                if apply_breach(&mut ecs, id, increment) {
                    break;
                }
                assert!(hits < 10_000, "must break eventually");
            }
            hits
        };
        let wood_hits = count_to_break(Hull::Wood);
        let diamond_hits = count_to_break(Hull::Diamond);
        assert!(
            wood_hits < diamond_hits,
            "a diamond cart resists more breach than a wood cart ({wood_hits} vs {diamond_hits})",
        );
    }

    #[test]
    fn apply_breach_on_a_non_cart_entity_is_a_noop() {
        // Robustness: a stale / wrong entity id can't panic or spawn drops.
        let mut ecs = hecs::World::new();
        let not_a_cart = ecs.spawn((crate::entity::Position(glam::Vec3::ZERO),));
        assert!(!apply_breach(&mut ecs, not_a_cart, 1000.0));
        assert_eq!(ecs.query::<&crate::entity::ItemEntity>().iter().count(), 0);
    }

    #[test]
    fn cart_at_cell_finds_a_cart_on_its_cell_only() {
        let mut ecs = hecs::World::new();
        let id = spawn_cart(&mut ecs, (4, 64, 7));
        assert_eq!(cart_at_cell(&ecs, (4, 64, 7)), Some(id));
        assert_eq!(cart_at_cell(&ecs, (4, 64, 8)), None, "no cart one cell over");
    }
}
