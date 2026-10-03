//! Spec 26 — NPC Builder commission flow.
//!
//! Foundation D of the Build Schematics economy. A villager bound to a
//! DRAFTING_TABLE (Profession::Builder) can be hired to build a Plan
//! for the player. This module owns:
//!
//! * `BuilderCommission` — per-villager component carrying the active
//!   commission's plan + anchor + locked materials + fee + status.
//! * `fee_for_commission` — pure fee formula (base + per-block + premium
//!   surcharge, reputation-discounted).
//! * `tick_commission_drive` — server-tick state-machine driver:
//!   PathingToSite → Building → Returning → Done. Used from `game_loop`
//!   (single-player) to walk the villager through the lifecycle.
//! * `refund_commission` — helper for the failure / cancellation paths.
//!
//! UI lives in `commission_ui.rs`; the egui surface is kept separate so
//! the data layer stays pure-function-testable.

use serde::{Deserialize, Serialize};

use crate::block::{self, BlockId};
use crate::plan::{CapturedCell, PlanData};
use crate::reputation::Tier as ReputationTier;

// ─── Constants ────────────────────────────────────────────────────────

/// Sats baseline fee. Tunable in playtest per the spec.
pub const BASE_FEE: u32 = 100;
/// Sats charged per captured cell.
pub const PER_BLOCK_FEE: u32 = 2;
/// Sats surcharge per premium block (Diamond, Iron, Coal, Satori family).
pub const PREMIUM_SURCHARGE: u32 = 20;

/// Builder NPCs build half as fast as players (1 cell per 2 ticks).
/// Drives `ConstructionAnchorData.pace_divider`.
pub const NPC_PACE_DIVIDER: u8 = 2;

/// Maximum ticks the NPC is given to reach the build site (60 s @ 20 TPS).
/// Past this the commission fails + the player is refunded.
pub const PATH_TIMEOUT_TICKS: u64 = 1200;

/// Maximum ticks the NPC is given to return to the workstation after a
/// completed build. Same budget as the outbound trip.
pub const RETURN_TIMEOUT_TICKS: u64 = 1200;

/// On-arrival range — within this Chebyshev distance of the anchor, the
/// NPC is considered to have arrived and starts building. Generous so a
/// villager that bumps the build edge counts.
pub const ARRIVE_RANGE: f32 = 2.5;

/// Cancellation refund — player gets 50 % of the sats back when they
/// cancel a commission already in flight.
#[cfg_attr(not(test), allow(dead_code))]
pub const CANCEL_REFUND_FRACTION: f32 = 0.5;

// ─── Data ─────────────────────────────────────────────────────────────

/// Why a commission failed. Drives the refund path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureReason {
    /// NPC couldn't reach the build site within `PATH_TIMEOUT_TICKS`.
    PathTimeoutToSite,
    /// NPC couldn't return to its workstation within `RETURN_TIMEOUT_TICKS`.
    PathTimeoutReturn,
    /// Player cancelled mid-commission.
    Cancelled,
}

/// Lifecycle status of one commission. The state machine drives the
/// builder NPC's pathfind + build behaviour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommissionStatus {
    /// Walking to the build anchor.
    PathingToSite,
    /// Animated-build in progress at NPC pace. `progress` mirrors the
    /// `ConstructionAnchorData.placed_index` so the status can be queried
    /// without going back through the World.
    Building { progress: u32 },
    /// Walking back to the workstation after a completed build.
    Returning,
    /// Lifecycle complete; the component is cleared on the next tick.
    Done,
    /// Lifecycle failed; refund issued; the component is cleared on the
    /// next tick.
    Failed(FailureReason),
}

/// One in-flight commission attached to a Builder villager via
/// `VillagerComponent.commission`. Persisted alongside the villager so
/// quit-mid-commission can resume cleanly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuilderCommission {
    /// The full plan (copied, not referenced — plans are content-stable).
    pub plan: PlanData,
    /// World-space anchor the player picked.
    pub anchor: [i32; 3],
    /// 0..=3 quarter-turn clockwise rotations.
    pub rotations: u8,
    /// Materials locked from the player's inventory at confirm.
    /// Consumed by the build; refunded on failure / cancel.
    pub locked_materials: Vec<(BlockId, u32)>,
    /// Fee locked at confirm (sats). Refund on failure.
    pub fee_sats: u32,
    /// Commissioner identity. Local-player worlds use "local-player-{idx}"
    /// per the alpha identity bridge; Signet-true worlds use the real npub.
    pub commissioner_npub: String,
    /// Village key (`(grid_x, grid_z)`) so payouts hit the right
    /// treasury. None when the village can't be resolved (e.g. test
    /// fixtures without `village_anchors` set up).
    pub village_id: Option<(i32, i32)>,
    /// Workstation position the NPC returns to after the build.
    pub workstation: [i32; 3],
    pub status: CommissionStatus,
    /// Engine tick at which the commission was confirmed. Used for the
    /// path timeouts + the player-offline grace check.
    pub created_tick: u64,
    /// Engine tick at which the current status was entered. Used to gate
    /// the pathfind timeouts.
    pub status_entered_tick: u64,
}

impl BuilderCommission {
    /// Sum of every block in `locked_materials`. Used by refund toasts.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn total_material_count(&self) -> u32 {
        self.locked_materials.iter().map(|(_, c)| c).sum()
    }
}

/// Transient state for the commission dialog. Lives on
/// `PlayerSlot.pending_commission` between Open and Confirm/Cancel.
/// Cleared on either outcome.
#[derive(Clone, Debug)]
pub struct CommissionDraft {
    /// Hotbar slot the Plan to be commissioned is held in. None until
    /// the player slots a Plan via the dialog.
    pub plan_hotbar_slot: Option<usize>,
    /// Confirmed build anchor (world coords). None until the player
    /// completes the Pick-build-site flow.
    pub site_anchor: Option<[i32; 3]>,
    /// Site rotation (0..=3 quarter-turns clockwise). Set alongside
    /// `site_anchor` during the pick step.
    pub site_rotations: u8,
    /// Workstation block the dialog was opened against. Drives the
    /// villager's Return path on Done.
    pub workstation: [i32; 3],
}

impl CommissionDraft {
    pub fn new(workstation: [i32; 3]) -> Self {
        Self {
            plan_hotbar_slot: None,
            site_anchor: None,
            site_rotations: 0,
            workstation,
        }
    }

    /// True iff the player has slotted a Plan AND picked a site — the
    /// Confirm button gates on this (plus materials + sats).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_ready_for_confirm(&self) -> bool {
        self.plan_hotbar_slot.is_some() && self.site_anchor.is_some()
    }
}

/// Credit line attached to the Plaque dropped at the end of an NPC
/// commission. Renders in `plan_ui::show_plaque_dialog` as
/// "Built by {villager} of {village} for {commissioner}".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderCredit {
    pub villager_name: String,
    pub village_name: Option<String>,
    pub commissioner_npub: String,
}

// ─── Fee formula (Phase 11) ───────────────────────────────────────────

/// Whether a block id counts as a "premium" material for the
/// surcharge. Adjust by reading `block.rs`; the spec calls out
/// Diamond + Iron + Coal + Satori family.
///
/// COAL_BLOCK / IRON_BLOCK / DIAMOND_BLOCK / SATORI_BLOCK + DIAMOND_ORE
/// are the alpha block constants (verified in `block.rs`). Crafted-
/// material premiums (IRON_INGOT etc.) aren't standalone block ids —
/// they're materials carried in inventory, not placed in plans, so they
/// can't appear in `PlanData.cells` and don't enter the surcharge.
pub fn is_premium_block(id: BlockId) -> bool {
    matches!(
        id,
        block::DIAMOND_ORE
            | block::DEEPSLATE_DIAMOND_ORE
            | block::IRON_BLOCK
            | block::DIAMOND_BLOCK
            | block::COAL_BLOCK
            | block::SATORI_BLOCK
    )
}

/// Spec 26 Phase 11 — pure fee formula. Returns the locked fee in sats.
///
/// ```text
/// raw = BASE_FEE + PER_BLOCK_FEE × cell_count + PREMIUM_SURCHARGE × premium_cells
/// fee = round(raw × tier_discount)   // 1.0 / 0.9 / 0.75 by tier
/// ```
pub fn fee_for_commission(plan: &PlanData, reputation_tier: ReputationTier) -> u32 {
    let cell_count = plan.cells.len() as u32;
    let premium_count = plan
        .cells
        .iter()
        .filter(|c| is_premium_block(c.block_id))
        .count() as u32;
    let raw = BASE_FEE + PER_BLOCK_FEE * cell_count + PREMIUM_SURCHARGE * premium_count;
    let discount = match reputation_tier {
        // Hostile/Wary are clamped to neutral pricing — the spec defines
        // discounts at Friendly + Beloved; the dialogue refuses entirely
        // for Hostile so the formula never fires for them in practice.
        ReputationTier::Hostile | ReputationTier::Wary | ReputationTier::Neutral => 1.0_f32,
        ReputationTier::Friendly => 0.9_f32,
        ReputationTier::Beloved => 0.75_f32,
    };
    (raw as f32 * discount).round() as u32
}

/// Pure helper: aggregate the cells of a plan into `(BlockId, count)`
/// pairs (excluding AIR). Used by the commission dialog's
/// "Materials required" readout AND by `lock_materials_for_commission`.
pub fn cell_block_counts(plan: &PlanData) -> Vec<(BlockId, u32)> {
    let counts = crate::plan::cell_block_counts(&plan.cells);
    let mut out: Vec<(BlockId, u32)> = counts
        .into_iter()
        .filter(|(id, _)| *id != block::AIR)
        .collect();
    // Deterministic ordering for UI + tests.
    out.sort_by_key(|(id, _)| *id);
    out
}

// ─── Material lock + refund ──────────────────────────────────────────

/// Take the materials required by `plan` from `inventory`. Returns the
/// taken `(BlockId, count)` list on success; on insufficient materials
/// returns Err with the first missing `(id, needed, owned)` — verify
/// pass runs first so a partial decrement never leaks.
pub fn lock_materials_for_commission(
    plan: &PlanData,
    inventory: &mut crate::inventory::Inventory,
) -> Result<Vec<(BlockId, u32)>, (BlockId, u32, u32)> {
    let counts = cell_block_counts(plan);
    // Verify everything is present before touching the inventory so a
    // failed lock leaves the player whole.
    for &(id, needed) in &counts {
        let owned = inventory_block_count(inventory, id);
        if owned < needed {
            return Err((id, needed, owned));
        }
    }
    // Decrement.
    let mut locked: Vec<(BlockId, u32)> = Vec::with_capacity(counts.len());
    for &(id, needed) in &counts {
        let removed = remove_blocks_from_inventory(inventory, id, needed);
        debug_assert_eq!(removed, needed, "verify pass should have caught this");
        locked.push((id, needed));
    }
    Ok(locked)
}

/// Sum every block of `id` currently held in `inventory` across all 36
/// slots. Pure / read-only — used by Phase 5's dialog readout AND by
/// `lock_materials_for_commission`'s verify pass.
pub fn inventory_block_count(inventory: &crate::inventory::Inventory, id: BlockId) -> u32 {
    (0..36)
        .filter_map(|i| inventory.slot(i))
        .filter_map(|s| match s.item {
            crate::item::Item::Block(b) if b == id => Some(s.count as u32),
            _ => None,
        })
        .sum()
}

fn remove_blocks_from_inventory(
    inventory: &mut crate::inventory::Inventory,
    id: BlockId,
    mut needed: u32,
) -> u32 {
    let mut removed = 0u32;
    for i in 0..36 {
        if needed == 0 {
            break;
        }
        let Some(stack) = inventory.slot(i).cloned() else { continue };
        let block_id = match stack.item {
            crate::item::Item::Block(b) => b,
            _ => continue,
        };
        if block_id != id {
            continue;
        }
        let take = (stack.count as u32).min(needed);
        let mut s = stack;
        s.count -= take as u8;
        needed -= take;
        removed += take;
        if s.count == 0 {
            inventory.set_slot(i, None);
        } else {
            inventory.set_slot(i, Some(s));
        }
    }
    removed
}

/// Return everything in `locked_materials` to the player's inventory.
/// Any blocks that can't fit (full inventory) are returned in the
/// `overflow` Vec so the caller can drop them at the player's feet.
pub fn refund_locked_materials(
    locked: &[(BlockId, u32)],
    inventory: &mut crate::inventory::Inventory,
) -> Vec<(BlockId, u32)> {
    let mut overflow = Vec::new();
    for &(id, count) in locked {
        if count == 0 {
            continue;
        }
        // ItemStack count is u8; split into multiple stacks if needed.
        let mut remaining = count;
        while remaining > 0 {
            let chunk = remaining.min(u8::MAX as u32) as u8;
            let stack = crate::item::ItemStack::new_block(id, chunk);
            let inserted = inventory.add_item(stack).is_none();
            if !inserted {
                overflow.push((id, remaining));
                break;
            }
            remaining -= chunk as u32;
        }
    }
    overflow
}

/// Compute the remaining `(BlockId, count)` after a partial build has
/// consumed `placed_index` cells. Used to figure out what to refund on
/// a cancel partway through the build.
#[cfg_attr(not(test), allow(dead_code))]
pub fn remaining_materials_after_progress(
    plan: &PlanData,
    placed_index: usize,
) -> Vec<(BlockId, u32)> {
    let ordered = crate::plan::order_cells_for_build(&plan.cells);
    let remaining_cells: Vec<&CapturedCell> = ordered.iter().skip(placed_index).collect();
    let mut counts: ahash::AHashMap<BlockId, u32> = ahash::AHashMap::new();
    for c in remaining_cells {
        if c.block_id != block::AIR {
            *counts.entry(c.block_id).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(BlockId, u32)> = counts.into_iter().collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

// ─── Status helpers ──────────────────────────────────────────────────

/// True when the lifecycle is over and the component should be cleared.
pub fn is_terminal_status(status: &CommissionStatus) -> bool {
    matches!(status, CommissionStatus::Done | CommissionStatus::Failed(_))
}

// ─── Settlement (Phase 12) ───────────────────────────────────────────

/// Spec 26 Phase 12 — credit the commissioner's fee to the village
/// treasury via the unified sats helper. Returns the breakdown so the
/// caller can log / toast it.
///
/// The helper handles the Charter + server-policy gates. On suppression
/// the treasury isn't credited (the audit trail still reflects the
/// attempted payment). On full credit the treasury gets the
/// `result.credited` amount; the skim slices (`server_skim` +
/// `reserve_drain`) accrue elsewhere when D-003 reverses.
pub fn settle_commission_to_treasury(
    treasuries: &mut ahash::AHashMap<(i32, i32), u64>,
    village_id: Option<(i32, i32)>,
    fee_sats: u32,
    policy: &crate::economy::ServerSatsPolicy,
    charter_allows_sats: bool,
) -> crate::economy::PayoutResult {
    let result = crate::economy::apply_sats_payout(
        fee_sats as u64,
        crate::economy::PayoutKind::BuilderCommission,
        policy,
        charter_allows_sats,
    );
    if let Some(vid) = village_id
        && result.credited > 0 {
            let entry = treasuries.entry(vid).or_insert(0);
            *entry = entry.saturating_add(result.credited);
        }
    result
}

// ─── Lifecycle drive (Phase 8 + 13) ──────────────────────────────────

/// One refund line emitted by `drive_commission_tick` when the
/// lifecycle terminates in a failure / cancel. The caller applies the
/// material refund (to inventory or drop), credits the sats back, and
/// shows the toast.
#[derive(Clone, Debug, PartialEq)]
pub struct RefundIntent {
    pub commissioner_npub: String,
    pub fee_sats: u32,
    pub fee_fraction: f32,
    pub materials: Vec<(BlockId, u32)>,
    pub reason: FailureReason,
}

/// A summary of what `drive_commission_tick` did this tick. The caller
/// runs the side effects (refund, plaque, status broadcast, toast).
#[derive(Clone, Debug, PartialEq)]
pub enum CommissionTickAction {
    /// Nothing to do — status carries on.
    None,
    /// NPC arrived at the build site. Caller seeds the
    /// ConstructionAnchor + flips status to Building.
    StartBuilding,
    /// NPC finished the build. Caller drops the Plaque + transitions
    /// status to Returning.
    BuildComplete,
    /// NPC returned to the workstation. Caller flips status to Done +
    /// clears the component on next tick.
    Returned,
    /// Lifecycle failure — caller issues the refund.
    Refund(RefundIntent),
}

/// Distance from the NPC's current world-space position to the
/// commission's build anchor (Chebyshev / max-axis distance — close
/// enough for the arrival gate).
pub fn distance_to_anchor(npc_pos: glam::Vec3, anchor: [i32; 3]) -> f32 {
    let target = glam::Vec3::new(
        anchor[0] as f32 + 0.5,
        anchor[1] as f32,
        anchor[2] as f32 + 0.5,
    );
    (target - npc_pos).length()
}

/// Decide what to do this tick based on the commission's status, the
/// NPC's position, the anchor's build progress, and whether the
/// commissioner is currently online. Pure.
///
/// `commissioner_online` lets the caller pause the build when the
/// commissioner logs off — per Phase 13 the build doesn't advance while
/// they're away, but pathing in/out still ticks (so the NPC doesn't
/// freeze mid-walk).
pub fn decide_commission_tick(
    commission: &BuilderCommission,
    npc_pos: glam::Vec3,
    placed_index_in_anchor: Option<usize>,
    current_tick: u64,
    commissioner_online: bool,
) -> CommissionTickAction {
    match &commission.status {
        CommissionStatus::PathingToSite => {
            let dist = distance_to_anchor(npc_pos, commission.anchor);
            if dist <= ARRIVE_RANGE {
                CommissionTickAction::StartBuilding
            } else if current_tick.saturating_sub(commission.status_entered_tick)
                >= PATH_TIMEOUT_TICKS
            {
                CommissionTickAction::Refund(RefundIntent {
                    commissioner_npub: commission.commissioner_npub.clone(),
                    fee_sats: commission.fee_sats,
                    fee_fraction: 1.0,
                    materials: commission.locked_materials.clone(),
                    reason: FailureReason::PathTimeoutToSite,
                })
            } else {
                CommissionTickAction::None
            }
        }
        CommissionStatus::Building { .. } => {
            if !commissioner_online {
                // Player offline — pause; caller will skip the
                // tick_build call so progress doesn't advance.
                return CommissionTickAction::None;
            }
            let cells = commission.plan.cells.len();
            let placed = placed_index_in_anchor.unwrap_or(0);
            if placed >= cells {
                CommissionTickAction::BuildComplete
            } else {
                CommissionTickAction::None
            }
        }
        CommissionStatus::Returning => {
            let workstation_pos = commission.workstation;
            let dist = distance_to_anchor(npc_pos, workstation_pos);
            if dist <= ARRIVE_RANGE {
                CommissionTickAction::Returned
            } else if current_tick.saturating_sub(commission.status_entered_tick)
                >= RETURN_TIMEOUT_TICKS
            {
                // Return-leg fail is non-refundable on materials (the
                // build already completed) but the partial refund only
                // covers the fee return-cost: spec is quiet here, so
                // treat as zero-refund + status surfaced via toast.
                CommissionTickAction::Refund(RefundIntent {
                    commissioner_npub: commission.commissioner_npub.clone(),
                    fee_sats: 0,
                    fee_fraction: 0.0,
                    materials: Vec::new(),
                    reason: FailureReason::PathTimeoutReturn,
                })
            } else {
                CommissionTickAction::None
            }
        }
        CommissionStatus::Done | CommissionStatus::Failed(_) => CommissionTickAction::None,
    }
}

/// One commission-tick outcome produced by `process_builder_commissions`.
/// The caller applies the side effects (block updates, refunds, toasts,
/// network broadcast) — this struct just packages them up.
#[derive(Clone, Debug)]
pub struct CommissionTickOutcome {
    /// Villager whose commission was processed. Not consumed by the live
    /// caller (`game_loop`'s `process_builder_commissions` drain) yet —
    /// carried for when a toast/broadcast wants to name the villager.
    #[allow(dead_code)]
    pub villager: hecs::Entity,
    /// Action the lifecycle decided on this tick.
    pub action: CommissionTickAction,
    /// Block-updates emitted by the build step this tick (cells placed
    /// by NPC pacing). Empty for non-build ticks.
    pub placed: Vec<(i32, i32, i32, BlockId)>,
    /// Plaque position dropped by a build-complete this tick. None
    /// when nothing was placed.
    pub plaque: Option<(i32, i32, i32)>,
}

/// Scan every villager with a `BuilderCommission`, walk one tick of the
/// lifecycle, and return the outcomes. Pure-ish: mutates the world's
/// construction-anchors map + the villager's commission status, but the
/// caller handles refunds, plaque placement broadcasts, and network
/// state.
///
/// `is_commissioner_online` is a callback so the caller can implement
/// the player-offline pause however it wants (local single-player can
/// just check the player npub against the local slots).
pub fn process_builder_commissions(
    ecs: &mut hecs::World,
    world: &mut crate::world::World,
    current_tick: u64,
    is_commissioner_online: impl Fn(&str) -> bool,
) -> Vec<CommissionTickOutcome> {
    // Two-pass to avoid borrow conflicts: snapshot positions + commissions,
    // then mutate.
    let mut snaps: Vec<(hecs::Entity, glam::Vec3, BuilderCommission)> = Vec::new();
    for (id, (pos, vc)) in ecs
        .query::<(&crate::entity::Position, &crate::villager::VillagerComponent)>()
        .iter()
    {
        if let Some(c) = vc.commission.as_ref() {
            snaps.push((id, pos.0, c.clone()));
        }
    }

    let mut outcomes: Vec<CommissionTickOutcome> = Vec::new();
    for (id, npc_pos, commission) in snaps {
        let online = is_commissioner_online(&commission.commissioner_npub);

        // Drive the build half-rate placement when in Building status.
        let mut placed = Vec::new();
        let placed_index_in_anchor = if matches!(
            commission.status,
            CommissionStatus::Building { .. }
        ) {
            let anchor_key = (commission.anchor[0], commission.anchor[1], commission.anchor[2]);
            if online {
                placed = crate::plan::tick_build(world, anchor_key);
            }
            world
                .construction_anchors
                .get(&anchor_key)
                .map(|d| d.placed_index)
        } else {
            None
        };

        let action = decide_commission_tick(
            &commission,
            npc_pos,
            placed_index_in_anchor,
            current_tick,
            online,
        );

        // Apply state transitions to the villager component before
        // returning the outcome so caller-side mutations don't race.
        let mut plaque_out: Option<(i32, i32, i32)> = None;
        match &action {
            CommissionTickAction::StartBuilding => {
                // Seed a ConstructionAnchor for this build at NPC pace.
                let anchor_key = (commission.anchor[0], commission.anchor[1], commission.anchor[2]);
                if world.construction_anchors.get(&anchor_key).is_none() {
                    let credit = BuilderCredit {
                        villager_name: villager_display_name(id),
                        village_name: commission
                            .village_id
                            .map(|(gx, gz)| format!("Village ({gx}, {gz})")),
                        commissioner_npub: commission.commissioner_npub.clone(),
                    };
                    let anchor_data = crate::plan::ConstructionAnchorData {
                        plan: commission.plan.clone(),
                        rotations: commission.rotations,
                        anchor: anchor_key,
                        placed_index: 0,
                        // NPC-commission builds aren't "creative" — they
                        // consume the locked materials. The locked stack
                        // mirror is held on the commission so we don't
                        // double-bookkeep on the anchor.
                        locked_materials: Vec::new(),
                        is_creative_build: true,
                        pace_divider: NPC_PACE_DIVIDER,
                        pace_counter: 0,
                        builder_credit: Some(credit),
                    };
                    world.construction_anchors.insert(anchor_key, anchor_data);
                }
                if let Ok(mut vc) = ecs.get::<&mut crate::villager::VillagerComponent>(id)
                    && let Some(c) = vc.commission.as_mut() {
                        c.status = CommissionStatus::Building { progress: 0 };
                        c.status_entered_tick = current_tick;
                    }
            }
            CommissionTickAction::BuildComplete => {
                let anchor_key = (commission.anchor[0], commission.anchor[1], commission.anchor[2]);
                plaque_out = crate::plan::complete_build(world, anchor_key);
                if let Ok(mut vc) = ecs.get::<&mut crate::villager::VillagerComponent>(id)
                    && let Some(c) = vc.commission.as_mut() {
                        c.status = CommissionStatus::Returning;
                        c.status_entered_tick = current_tick;
                    }
            }
            CommissionTickAction::Returned => {
                if let Ok(mut vc) = ecs.get::<&mut crate::villager::VillagerComponent>(id)
                    && let Some(c) = vc.commission.as_mut() {
                        c.status = CommissionStatus::Done;
                        c.status_entered_tick = current_tick;
                    }
            }
            CommissionTickAction::Refund(intent) => {
                if let Ok(mut vc) = ecs.get::<&mut crate::villager::VillagerComponent>(id)
                    && let Some(c) = vc.commission.as_mut() {
                        c.status = CommissionStatus::Failed(intent.reason);
                        c.status_entered_tick = current_tick;
                    }
            }
            CommissionTickAction::None => {}
        }

        // Drive position toward the active goal. This is a soft pull
        // — the simple `tick_mob_ai` wander remains the dominant
        // animator; the commission tick nudges the villager along a
        // straight line each tick.
        match &commission.status {
            CommissionStatus::PathingToSite => {
                step_toward(ecs, id, npc_pos, commission.anchor);
            }
            CommissionStatus::Returning => {
                step_toward(ecs, id, npc_pos, commission.workstation);
            }
            _ => {}
        }

        outcomes.push(CommissionTickOutcome {
            villager: id,
            action,
            placed,
            plaque: plaque_out,
        });
    }

    // Sweep terminal-status villagers — clear the commission component so
    // their AI returns to normal wander on the next tick.
    let mut to_clear: Vec<hecs::Entity> = Vec::new();
    for (id, vc) in ecs
        .query::<&crate::villager::VillagerComponent>()
        .iter()
    {
        if vc.commission.as_ref().is_some_and(|c| is_terminal_status(&c.status)) {
            to_clear.push(id);
        }
    }
    for id in to_clear {
        if let Ok(mut vc) = ecs.get::<&mut crate::villager::VillagerComponent>(id) {
            vc.commission = None;
        }
    }
    outcomes
}

fn step_toward(
    ecs: &mut hecs::World,
    villager: hecs::Entity,
    pos: glam::Vec3,
    target: [i32; 3],
) {
    let target_v = glam::Vec3::new(target[0] as f32 + 0.5, pos.y, target[2] as f32 + 0.5);
    let to_target = target_v - pos;
    let dist = to_target.length();
    if dist < 0.1 {
        return;
    }
    let step_speed: f32 = 0.15; // ~3 m/s at 20 TPS — half a villager-wander run.
    let dir = to_target / dist;
    let dx = dir.x * step_speed.min(dist);
    let dz = dir.z * step_speed.min(dist);
    if let Ok(mut pos_ref) = ecs.get::<&mut crate::entity::Position>(villager) {
        pos_ref.0.x += dx;
        pos_ref.0.z += dz;
    }
}

/// Display name for a villager. Mirrors `villager_ui::villager_label`
/// (entity-id-based) but without the profession suffix so it reads
/// cleanly in the Plaque credit line.
pub fn villager_display_name(entity: hecs::Entity) -> String {
    let id_str = format!("{:?}", entity);
    let short = id_str
        .split(['(', ',', ' '])
        .nth(1)
        .unwrap_or(&id_str)
        .to_string();
    format!("Villager #{short}")
}

/// Cancel-by-player helper. Computes the refund intent for a player-
/// initiated cancel: 50 % of the fee + any remaining materials.
#[cfg_attr(not(test), allow(dead_code))]
pub fn cancel_commission_refund(
    commission: &BuilderCommission,
    placed_index: usize,
) -> RefundIntent {
    let materials = if matches!(commission.status, CommissionStatus::PathingToSite) {
        // Nothing built yet — refund all locked materials.
        commission.locked_materials.clone()
    } else {
        remaining_materials_after_progress(&commission.plan, placed_index)
    };
    RefundIntent {
        commissioner_npub: commission.commissioner_npub.clone(),
        fee_sats: (commission.fee_sats as f32 * CANCEL_REFUND_FRACTION).round() as u32,
        fee_fraction: CANCEL_REFUND_FRACTION,
        materials,
        reason: FailureReason::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ItemStack;
    use crate::plan::PlanLicense;

    fn cell(rx: u8, ry: u8, rz: u8, b: BlockId) -> CapturedCell {
        CapturedCell { rx, ry, rz, block_id: b }
    }

    fn plan_with_cells(cells: Vec<CapturedCell>) -> PlanData {
        let (max_x, max_y, max_z) = cells.iter().fold((0u8, 0u8, 0u8), |acc, c| {
            (acc.0.max(c.rx), acc.1.max(c.ry), acc.2.max(c.rz))
        });
        PlanData {
            version: 1,
            name: "test".to_string(),
            author_npub: String::new(),
            license: PlanLicense::Ccbysa,
            derivation_chain: Vec::new(),
            is_master: true,
            width: max_x + 1,
            depth: max_z + 1,
            height: max_y + 1,
            cells,
            authored_in: "survival".to_string(),
            develop_state: crate::plan::DevelopState::Developed,
            kind: crate::plan::PlanKind::Building,
        }
    }

    // ─── Fee formula ────────────────────────────────────────────────

    #[test]
    fn fee_neutral_small_house_no_premiums() {
        // 30 stone cells, no premium blocks, Neutral rep.
        // raw = 100 + 2*30 + 0 = 160; × 1.0 = 160.
        let cells: Vec<_> = (0..30u8)
            .map(|i| cell(i % 5, i / 5, 0, block::STONE))
            .collect();
        let plan = plan_with_cells(cells);
        assert_eq!(fee_for_commission(&plan, ReputationTier::Neutral), 160);
    }

    #[test]
    fn fee_beloved_large_house_with_premiums() {
        // 60 cells, 5 premium (diamond block). Beloved rep.
        // raw = 100 + 2*60 + 20*5 = 320; × 0.75 = 240.
        let mut cells: Vec<CapturedCell> = Vec::new();
        for i in 0..55u8 {
            cells.push(cell(i % 8, i / 8, 0, block::STONE));
        }
        for i in 0..5u8 {
            cells.push(cell(i, 7, 0, block::DIAMOND_BLOCK));
        }
        let plan = plan_with_cells(cells);
        assert_eq!(fee_for_commission(&plan, ReputationTier::Beloved), 240);
    }

    #[test]
    fn fee_friendly_applies_10_percent_discount() {
        // 10 cells, 0 premium. raw = 100 + 20 = 120; × 0.9 = 108.
        let cells: Vec<_> = (0..10u8).map(|i| cell(i, 0, 0, block::STONE)).collect();
        let plan = plan_with_cells(cells);
        assert_eq!(fee_for_commission(&plan, ReputationTier::Friendly), 108);
    }

    #[test]
    fn fee_hostile_and_wary_treated_as_neutral_pricing() {
        // 10 cells, raw = 120. Hostile/Wary keep neutral pricing — the
        // dialogue refuses Hostile entirely so the formula doesn't fire
        // in practice; the test guards the fallback.
        let cells: Vec<_> = (0..10u8).map(|i| cell(i, 0, 0, block::STONE)).collect();
        let plan = plan_with_cells(cells);
        assert_eq!(fee_for_commission(&plan, ReputationTier::Hostile), 120);
        assert_eq!(fee_for_commission(&plan, ReputationTier::Wary), 120);
    }

    // ─── Premium block recognition ─────────────────────────────────

    #[test]
    fn premium_block_set_matches_spec() {
        assert!(is_premium_block(block::DIAMOND_BLOCK));
        assert!(is_premium_block(block::IRON_BLOCK));
        assert!(is_premium_block(block::COAL_BLOCK));
        assert!(is_premium_block(block::SATORI_BLOCK));
        assert!(is_premium_block(block::DIAMOND_ORE));
        assert!(!is_premium_block(block::STONE));
        assert!(!is_premium_block(block::DIRT));
        assert!(!is_premium_block(block::OAK_PLANKS));
    }

    // ─── cell_block_counts ─────────────────────────────────────────

    #[test]
    fn cell_block_counts_aggregates_and_excludes_air() {
        let cells = vec![
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(2, 0, 0, block::DIRT),
            cell(3, 0, 0, block::AIR),
        ];
        let plan = plan_with_cells(cells);
        let counts = cell_block_counts(&plan);
        assert!(counts.iter().all(|(id, _)| *id != block::AIR));
        let stone = counts.iter().find(|(id, _)| *id == block::STONE).unwrap();
        assert_eq!(stone.1, 2);
        let dirt = counts.iter().find(|(id, _)| *id == block::DIRT).unwrap();
        assert_eq!(dirt.1, 1);
    }

    // ─── lock / refund ─────────────────────────────────────────────

    fn make_inventory_with(blocks: &[(BlockId, u8)]) -> crate::inventory::Inventory {
        let mut inv = crate::inventory::Inventory::new();
        for (i, &(id, count)) in blocks.iter().enumerate() {
            inv.set_slot(i, Some(ItemStack::new_block(id, count)));
        }
        inv
    }

    #[test]
    fn lock_materials_decrements_inventory_and_returns_pairs() {
        let cells = vec![
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(2, 0, 0, block::DIRT),
        ];
        let plan = plan_with_cells(cells);
        let mut inv = make_inventory_with(&[(block::STONE, 10), (block::DIRT, 5)]);
        let locked = lock_materials_for_commission(&plan, &mut inv).unwrap();
        assert_eq!(inventory_block_count(&inv, block::STONE), 8);
        assert_eq!(inventory_block_count(&inv, block::DIRT), 4);
        // locked pairs total what was taken.
        let stone = locked.iter().find(|(id, _)| *id == block::STONE).unwrap();
        assert_eq!(stone.1, 2);
        let dirt = locked.iter().find(|(id, _)| *id == block::DIRT).unwrap();
        assert_eq!(dirt.1, 1);
    }

    #[test]
    fn lock_materials_refuses_when_short_without_mutating() {
        let cells = vec![
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(2, 0, 0, block::DIRT),
        ];
        let plan = plan_with_cells(cells);
        let mut inv = make_inventory_with(&[(block::STONE, 1)]); // not enough!
        let err = lock_materials_for_commission(&plan, &mut inv).unwrap_err();
        assert_eq!(err.0, block::STONE);
        assert_eq!(err.1, 2);
        assert_eq!(err.2, 1);
        // Untouched.
        assert_eq!(inventory_block_count(&inv, block::STONE), 1);
    }

    #[test]
    fn refund_restores_materials_to_inventory() {
        let mut inv = crate::inventory::Inventory::new();
        let overflow = refund_locked_materials(
            &[(block::STONE, 5), (block::DIRT, 3)],
            &mut inv,
        );
        assert!(overflow.is_empty());
        assert_eq!(inventory_block_count(&inv, block::STONE), 5);
        assert_eq!(inventory_block_count(&inv, block::DIRT), 3);
    }

    // ─── remaining_materials_after_progress ────────────────────────

    #[test]
    fn remaining_materials_drops_already_built_cells() {
        // 4 cells: ordered by ry,rx,rz → 4 stones at y=0 x=0..3.
        let cells = vec![
            cell(0, 0, 0, block::STONE),
            cell(1, 0, 0, block::STONE),
            cell(2, 0, 0, block::STONE),
            cell(3, 0, 0, block::DIRT),
        ];
        let plan = plan_with_cells(cells);
        let remaining = remaining_materials_after_progress(&plan, 2);
        // Two cells placed: 2 stone consumed; 1 stone + 1 dirt remain.
        let stone = remaining.iter().find(|(id, _)| *id == block::STONE).unwrap();
        assert_eq!(stone.1, 1);
        let dirt = remaining.iter().find(|(id, _)| *id == block::DIRT).unwrap();
        assert_eq!(dirt.1, 1);
    }

    // ─── Status helpers ────────────────────────────────────────────

    #[test]
    fn terminal_status_covers_done_and_failed() {
        assert!(is_terminal_status(&CommissionStatus::Done));
        assert!(is_terminal_status(&CommissionStatus::Failed(
            FailureReason::PathTimeoutToSite
        )));
        assert!(!is_terminal_status(&CommissionStatus::PathingToSite));
        assert!(!is_terminal_status(&CommissionStatus::Building {
            progress: 0
        }));
        assert!(!is_terminal_status(&CommissionStatus::Returning));
    }

    // ─── BuilderCommission helpers ─────────────────────────────────

    // ─── Settlement ─────────────────────────────────────────────

    #[test]
    fn settle_commission_credits_treasury_when_enabled() {
        let mut treasuries: ahash::AHashMap<(i32, i32), u64> = ahash::AHashMap::new();
        let policy = crate::economy::ServerSatsPolicy::bitcoin_enabled_policy();
        let r = settle_commission_to_treasury(
            &mut treasuries,
            Some((0, 0)),
            240,
            &policy,
            true,
        );
        assert!(!r.suppressed);
        assert_eq!(r.credited, 240);
        assert_eq!(treasuries.get(&(0, 0)).copied(), Some(240));
    }

    #[test]
    fn settle_commission_charter_off_does_not_credit_treasury() {
        let mut treasuries: ahash::AHashMap<(i32, i32), u64> = ahash::AHashMap::new();
        let policy = crate::economy::ServerSatsPolicy::bitcoin_enabled_policy();
        let r = settle_commission_to_treasury(
            &mut treasuries,
            Some((0, 0)),
            240,
            &policy,
            false,
        );
        assert!(r.suppressed);
        assert!(treasuries.is_empty());
    }

    #[test]
    fn settle_commission_no_village_skips_credit_but_still_runs_payout() {
        let mut treasuries: ahash::AHashMap<(i32, i32), u64> = ahash::AHashMap::new();
        let policy = crate::economy::ServerSatsPolicy::bitcoin_enabled_policy();
        let r = settle_commission_to_treasury(
            &mut treasuries,
            None,
            240,
            &policy,
            true,
        );
        assert_eq!(r.credited, 240);
        assert!(treasuries.is_empty());
    }

    // ─── Lifecycle drive ───────────────────────────────────────

    fn fixture_commission(status: CommissionStatus, status_entered_tick: u64) -> BuilderCommission {
        BuilderCommission {
            plan: plan_with_cells(vec![
                cell(0, 0, 0, block::STONE),
                cell(1, 0, 0, block::STONE),
            ]),
            anchor: [10, 64, 10],
            rotations: 0,
            locked_materials: vec![(block::STONE, 2)],
            fee_sats: 100,
            commissioner_npub: "local-player-0".into(),
            village_id: Some((0, 0)),
            workstation: [0, 64, 0],
            status,
            created_tick: 0,
            status_entered_tick,
        }
    }

    #[test]
    fn pathing_arrival_emits_start_building() {
        let c = fixture_commission(CommissionStatus::PathingToSite, 0);
        let npc_pos = glam::Vec3::new(10.0, 64.0, 10.0); // at anchor
        let action = decide_commission_tick(&c, npc_pos, None, 100, true);
        assert!(matches!(action, CommissionTickAction::StartBuilding));
    }

    #[test]
    fn pathing_timeout_emits_refund_with_full_fee() {
        let c = fixture_commission(CommissionStatus::PathingToSite, 0);
        let npc_pos = glam::Vec3::new(100.0, 64.0, 100.0); // far away
        let action = decide_commission_tick(&c, npc_pos, None, PATH_TIMEOUT_TICKS + 1, true);
        let intent = match action {
            CommissionTickAction::Refund(i) => i,
            _ => panic!("expected Refund, got {:?}", action),
        };
        assert_eq!(intent.reason, FailureReason::PathTimeoutToSite);
        assert_eq!(intent.fee_sats, 100);
        assert_eq!(intent.fee_fraction, 1.0);
        assert_eq!(intent.materials, vec![(block::STONE, 2)]);
    }

    #[test]
    fn building_complete_emits_build_complete() {
        let c = fixture_commission(CommissionStatus::Building { progress: 0 }, 0);
        // Plan has 2 cells; placed_index reached 2 = full.
        let action = decide_commission_tick(&c, glam::Vec3::ZERO, Some(2), 50, true);
        assert!(matches!(action, CommissionTickAction::BuildComplete));
    }

    #[test]
    fn building_offline_pauses_with_none() {
        let c = fixture_commission(CommissionStatus::Building { progress: 0 }, 0);
        let action = decide_commission_tick(&c, glam::Vec3::ZERO, Some(2), 50, false);
        // Build complete but commissioner offline → no advance.
        assert!(matches!(action, CommissionTickAction::None));
    }

    #[test]
    fn returning_arrival_emits_returned() {
        let c = fixture_commission(CommissionStatus::Returning, 0);
        let npc_pos = glam::Vec3::new(0.5, 64.0, 0.5); // at workstation
        let action = decide_commission_tick(&c, npc_pos, None, 50, true);
        assert!(matches!(action, CommissionTickAction::Returned));
    }

    #[test]
    fn cancel_refund_pays_half_fee() {
        let c = fixture_commission(CommissionStatus::Building { progress: 1 }, 0);
        let intent = cancel_commission_refund(&c, 1);
        assert_eq!(intent.reason, FailureReason::Cancelled);
        assert_eq!(intent.fee_sats, 50); // 50% of 100
        assert_eq!(intent.fee_fraction, 0.5);
    }

    #[test]
    fn cancel_at_pathing_refunds_all_locked_materials() {
        let c = fixture_commission(CommissionStatus::PathingToSite, 0);
        let intent = cancel_commission_refund(&c, 0);
        assert_eq!(intent.materials, vec![(block::STONE, 2)]);
    }

    #[test]
    fn total_material_count_sums_locked() {
        let c = BuilderCommission {
            plan: plan_with_cells(vec![cell(0, 0, 0, block::STONE)]),
            anchor: [0, 0, 0],
            rotations: 0,
            locked_materials: vec![(block::STONE, 10), (block::DIRT, 5)],
            fee_sats: 100,
            commissioner_npub: "x".to_string(),
            village_id: None,
            workstation: [0, 0, 0],
            status: CommissionStatus::PathingToSite,
            created_tick: 0,
            status_entered_tick: 0,
        };
        assert_eq!(c.total_material_count(), 15);
    }
}
