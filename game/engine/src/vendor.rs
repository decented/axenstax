//! Spec 21 — Vendor Block.
//!
//! Player-placed shop. The owner picks a mode (Sell / Buy / Barter)
//! and a slotted item + price; buyers right-click to trade. First
//! player-to-player Bitcoin gameplay path. Charter-gated, server-policy-
//! gated, tax-and-drain-routed via `economy::apply_sats_payout`.
//!
//! Phase boundaries (see `docs/foundations/2026-05-18-vendor-block.md`):
//! - **Phases 2-10 DELIVERED** 2026-05-20: data model + block id +
//!   recipe + place handler + owner/buyer dialogue UIs + transaction
//!   logic (Buy/Sell/Barter) + anti-grief on break + Charter gating
//!   + save/load + docs.
//! - Phases 11-14 (multi-slot conversion, rep-gating + trade-value
//!   floors, stale-shop expiry, raid-supplies highlight) DEFERRED
//!   post-Axolittle-playtest per the meta-economy review.
//! - Phase 15 = Axolittle playtest gate.

use serde::{Deserialize, Serialize};

use crate::economy::{apply_sats_payout, PayoutKind, PayoutResult, ServerSatsPolicy};
use crate::item::{Item, ItemStack};

/// What a Vendor Block does. Owner picks one of these at placement
/// time (or via the mode picker on the dialog).
///
/// Bincode-encoded as a positional discriminant — variants append at
/// the end, never reorder. The Spec 25 plan variants
/// (`SellPlanMaster` / `SellPlanLicence`) sit after the original three
/// so pre-Spec-25 saves continue to deserialise without a version bump.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VendorMode {
    /// Owner offers an item for sats. Buyer pays `price`; owner's
    /// `escrow_sats` increments. Stock decrements per sale.
    Sell,
    /// Owner pays sats for an item. Buyer brings the item; owner's
    /// `escrow_sats` debits, buyer is credited via the unified helper.
    Buy,
    /// Owner offers an item for another item. No sats. Buyer slots
    /// the trade-item, owner gives `slot` item in return. Always
    /// available regardless of Charter / per-server sats policy.
    Barter,
    /// Spec 25 — sell a Master-tier Plan. One-time sale: stock is
    /// fixed at 1; on purchase the buyer receives the listed Plan
    /// (with `is_master = true` preserved) and the listing locks. The
    /// `slot` field carries the `Item::Plan(PlanData)`.
    SellPlanMaster,
    /// Spec 25 — sell Licence copies of a Plan. Per-copy sale: stock
    /// decrements per purchase; `u32::MAX` is the "Unlimited" marker
    /// (never decrements). Each buyer receives a clone of the Plan
    /// with `is_master = false` so they can build but not Save-As.
    SellPlanLicence,
    /// Spec 40 — sell the slot item in fixed-size lots at a (typically
    /// sub-single) per-unit price. The owner sets `VendorData.lot_size`
    /// to 8 / 16 / 32 / 64; a buy transfers `lot_size` units for
    /// `lot_size × price_sats`. Refused when stock < lot_size. Reuses
    /// the Sell-mode village-skim logic so a Bulk Vendor inside a
    /// village still feeds the treasury (the market-hub tax break is
    /// deferred to when hub taxes exist).
    Bulk,
}

impl VendorMode {
    pub fn label(self) -> &'static str {
        match self {
            VendorMode::Sell => "Sell",
            VendorMode::Buy => "Buy",
            VendorMode::Barter => "Barter",
            VendorMode::SellPlanMaster => "Sell Plan (Master)",
            VendorMode::SellPlanLicence => "Sell Plan (Licence)",
            VendorMode::Bulk => "Bulk",
        }
    }

    /// Does this mode require the Charter sats flag to be on? Barter
    /// is always available; everything that moves sats is gated.
    pub fn requires_sats(self) -> bool {
        matches!(
            self,
            VendorMode::Sell
                | VendorMode::Buy
                | VendorMode::SellPlanMaster
                | VendorMode::SellPlanLicence
                | VendorMode::Bulk
        )
    }

    /// True for the two Spec 25 plan-listing modes. Used by the owner
    /// UI to switch slot-picker filtering + stock-counter rendering,
    /// and by the buyer UI to swap in the plan-listing render path.
    pub fn is_plan_mode(self) -> bool {
        matches!(self, VendorMode::SellPlanMaster | VendorMode::SellPlanLicence)
    }
}

/// Spec 40 — valid Bulk lot sizes for the owner UI picker. Bulk
/// transactions are gated to these so the UI can offer a fixed
/// dropdown rather than an open numeric field, and so save/load can
/// validate the persisted value. Helper kept in the data module so
/// both the UI + the buy path agree on the contract.
pub const BULK_LOT_SIZES: [u32; 4] = [8, 16, 32, 64];

/// Spec 40 — default lot size for non-Bulk vendors. Stored on every
/// `VendorData` so the field is universal; `try_buy` only consults it
/// in Bulk mode.
pub const DEFAULT_LOT_SIZE: u32 = 1;

fn default_lot_size() -> u32 { DEFAULT_LOT_SIZE }

/// Marker for the Spec 25 "Unlimited" stock setting on a
/// `SellPlanLicence` listing. Stored in `VendorData.stock`; the buy
/// path leaves stock untouched when it matches this value so the
/// listing never runs out.
pub const UNLIMITED_STOCK: u32 = u32::MAX;

/// Who owns a Vendor Block. Alpha runs single-player so the owner
/// is identified by a local player-slot index. Multiplayer (Spec 1
/// Phase 4) replaces this with an npub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VendorOwner {
    /// Local PlayerSlot index. Valid on the single-player path until
    /// multiplayer ownership lands.
    LocalPlayer(usize),
    /// Future multiplayer — npub of the owning account. Reserved.
    /// `LocalPlayer` is the only variant constructed today.
    #[allow(dead_code)]
    Npub(String),
}

impl VendorOwner {
    /// True iff this local player-slot owns the vendor. Returns false
    /// for any `Npub` variant (multiplayer not wired yet).
    pub fn is_local(&self, pidx: usize) -> bool {
        matches!(self, VendorOwner::LocalPlayer(p) if *p == pidx)
    }
}

/// Spec 21 Phase 2 — per-vendor state. Lives in
/// `World::block_entities` keyed by the vendor block's world-space
/// position. Phase 11 expands `slot` to `[Option<VendorSlot>; 3]`;
/// Phase 2 ships the single-slot shape.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VendorData {
    /// Owner identity. None until first placement (defensive — placement
    /// always sets it; deserialised vendors from before Phase 9 land
    /// with `None` and the right-click handler dismisses them gracefully).
    pub owner: Option<VendorOwner>,
    /// What mode this vendor is operating in.
    pub mode: Option<VendorMode>,
    /// Slot item — for Sell: the item being offered; for Buy: the
    /// item the owner wants; for Barter: the item being offered.
    pub slot: Option<ItemStack>,
    /// Barter-mode only — the item the buyer must bring.
    pub barter_request: Option<ItemStack>,
    /// Sats price for Sell / Buy modes. Unused in Barter.
    pub price_sats: u32,
    /// Remaining stock — number of slot-items the owner has deposited
    /// for Sell, or the number of Buys remaining before the budget
    /// runs out. UI shows this as "x10 in stock" / "x10 wanted".
    pub stock: u32,
    /// Owner's accumulated sats from sales (Sell mode) minus spends
    /// (Buy mode). Withdrawn via the Withdraw button in the owner UI.
    pub escrow_sats: u64,
    /// Spec 21 Phase 13 — tick of the most-recent transaction. Used
    /// by the stale-shop expiry. Default 0 = freshly placed; the
    /// place handler stamps this on creation.
    pub last_txn_tick: u64,
    /// Spec 40 — lot size for Bulk mode. Pre-Spec-40 saves load with
    /// `DEFAULT_LOT_SIZE = 1` via `#[serde(default)]`; only Bulk mode
    /// consults this. The owner picker constrains writes to
    /// `BULK_LOT_SIZES` (8 / 16 / 32 / 64).
    #[serde(default = "default_lot_size")]
    pub lot_size: u32,
}

/// Reasons a Buy / Barter click can fail. Surfaced as toasts to the
/// buyer; the unified UI rendering reads these to dim the Buy button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuyRefusal {
    /// Vendor has no stock left.
    OutOfStock,
    /// Buyer doesn't have enough sats. Never actually returned — no code path
    /// checks a buyer's sats balance yet (consistent with the non-custodial
    /// / no-wallet-balance-check economy scaffolding elsewhere; a real
    /// Lightning-payment check would produce this once that lands).
    #[allow(dead_code)]
    InsufficientSats,
    /// Buyer's inventory is full — can't take the item.
    InventoryFull,
    /// Charter sats flag is off — Sell/Buy unavailable to this player.
    /// Barter is reachable regardless; this variant only fires on
    /// sats-touching modes.
    CharterDeny,
    /// Server policy disabled sats globally.
    BitcoinDisabled,
    /// Buyer didn't bring the barter-requested item.
    BarterRequestMissing,
    /// The listing is a chance drop (Satori), which never sells for sats
    /// (see `economy::is_chance_drop`). Barter still works.
    ChanceDropNotForSats,
}

impl BuyRefusal {
    pub fn message(self) -> &'static str {
        match self {
            BuyRefusal::OutOfStock => "Out of stock.",
            BuyRefusal::InsufficientSats => "Not enough sats.",
            BuyRefusal::InventoryFull => "Your inventory is full.",
            BuyRefusal::CharterDeny => "Bitcoin is disabled for this account.",
            BuyRefusal::BitcoinDisabled => "Sats are disabled on this server.",
            BuyRefusal::BarterRequestMissing => "You need the requested item.",
            BuyRefusal::ChanceDropNotForSats => "Satori and Satori-made items can't be sold for sats. Try Barter.",
        }
    }
}

/// The items a vendor really holds in its slot, as stacks (audit
/// 2026-09-27). `stock` is only an item count in modes that are
/// item-backed:
///
/// - **Bulk**: `slot` is a *template*; `stock` is the real unit count
///   (deposits + depot freight). Spills `stock` units of the template.
/// - **Sell / Barter / Buy / plan modes / unset**: the slot's literal
///   contents. In plan modes `stock` is a licence counter (or the
///   `UNLIMITED_STOCK` sentinel), never an item count, so it is never
///   multiplied out.
///
/// The sentinel is never treated as a count, even on a corrupt or
/// pre-fix save that carried it into Bulk.
pub fn stocked_items(v: &VendorData) -> Vec<ItemStack> {
    let Some(slot) = &v.slot else {
        return Vec::new();
    };
    if v.mode == Some(VendorMode::Bulk) && v.stock != UNLIMITED_STOCK {
        let mut out = Vec::new();
        let mut remaining = v.stock;
        while remaining > 0 {
            let take = remaining.min(u8::MAX as u32);
            let mut st = slot.clone();
            st.count = take as u8;
            out.push(st);
            remaining -= take;
        }
        return out;
    }
    if slot.count == 0 {
        return Vec::new();
    }
    vec![slot.clone()]
}

/// Repair a vendor loaded from a save written before the 2026-09-27 mode
/// fix (review S2). The old mode switch kept `stock`, so a Licence vendor
/// (counter N, or the `UNLIMITED_STOCK` sentinel) switched into Bulk loads
/// with a one-Plan template and `stock = N` — and Bulk multiplies the
/// template by `stock`, turning one Plan into N on break/withdraw/mode
/// switch. A Plan can never be a legal Bulk template (`slot_accepts_for_mode`
/// refuses it), so a Bulk vendor holding one is always this legacy state:
/// clamp `stock` to the Plan actually held. Called on every save load.
pub fn repair_legacy_bulk(v: &mut VendorData) {
    if v.mode != Some(VendorMode::Bulk) {
        return;
    }
    if let Some(slot) = &v.slot
        && matches!(slot.item, Item::Plan(_))
    {
        v.stock = v.stock.min(slot.count as u32);
    }
}

/// Empty the vendor's slot stock: returns [`stocked_items`] and clears
/// `slot` + `stock`. The owner-withdraw path.
pub fn take_stock(v: &mut VendorData) -> Vec<ItemStack> {
    let items = stocked_items(v);
    v.slot = None;
    v.stock = 0;
    items
}

/// Why an owner's mode switch was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChangeRefusal {
    /// The vendor holds more stock than one slot can carry in the new
    /// mode (Bulk freight above a stack). Withdraw or sell it first.
    TooMuchStock,
}

impl ModeChangeRefusal {
    pub fn message(self) -> &'static str {
        match self {
            ModeChangeRefusal::TooMuchStock => {
                "Too much stock to switch mode. Sell or withdraw it first."
            }
        }
    }
}

/// Owner picks a vendor mode (audit 2026-09-27). Reconciles `stock`
/// with the items the vendor really holds so a mode's meaning of
/// `stock` never leaks into another mode: the Licence `UNLIMITED_STOCK`
/// sentinel (or any licence count) can't survive into Sell/Bulk, and
/// Bulk freight carries into the new mode's slot rather than vanishing.
/// Plan modes restart their counter at 1 (Master is fixed at 1; the
/// owner dials a Licence up or clicks Unlimited). Re-picking the
/// current mode is a no-op.
pub fn change_mode(v: &mut VendorData, mode: VendorMode) -> Result<(), ModeChangeRefusal> {
    if v.mode == Some(mode) {
        return Ok(());
    }
    let mut items = stocked_items(v);
    if items.len() > 1 {
        return Err(ModeChangeRefusal::TooMuchStock);
    }
    v.slot = items.pop();
    v.mode = Some(mode);
    let held = v.slot.as_ref().map_or(0, |s| s.count as u32);
    v.stock = match mode {
        VendorMode::SellPlanMaster | VendorMode::SellPlanLicence => 1,
        _ => held,
    };
    if mode == VendorMode::Bulk && !BULK_LOT_SIZES.contains(&v.lot_size) {
        // Spec 40 — seed the smallest valid lot so the picker starts sane.
        v.lot_size = BULK_LOT_SIZES[0];
    }
    Ok(())
}

/// Settle a completed non-plan sale of `units` against the vendor's
/// stock. In **Bulk** the slot is a template the sale never decrements
/// (audit 2026-09-27 — decrementing it stranded depot freight after the
/// first lot); it is cleared only when the last unit sells. Every other
/// mode keeps `slot.count` and `stock` in step.
pub fn apply_sale(v: &mut VendorData, units: u32) {
    v.stock = v.stock.saturating_sub(units);
    if v.mode == Some(VendorMode::Bulk) {
        if v.stock == 0 {
            v.slot = None;
        } else if let Some(s) = v.slot.as_mut() {
            // Keep the template's display count honest (never above stock).
            s.count = v.stock.min(u8::MAX as u32) as u8;
        }
        return;
    }
    if let Some(s) = v.slot.as_mut() {
        let dec = units.min(s.count as u32) as u8;
        if s.count > dec {
            s.count -= dec;
        } else {
            v.slot = None;
        }
    }
}

/// Spec 21 Phase 7 — cleanup hook when a Vendor Block is being
/// destroyed (owner-only mine, /setblock-replaced). Removes the
/// block-entity entry and returns the spill: the real slot stock (see
/// [`stocked_items`] — never a sentinel, never more than is stored),
/// plus the Barter request item if set. Sats escrow is returned as part
/// of the caller's accounting (this function returns the u64; the
/// caller decides where it goes). The caller must place EVERY spilled
/// stack (inventory first, the remainder as item entities). Idempotent
/// — safe on cells that were never vendors.
pub fn cleanup_vendor(
    world: &mut crate::world::World,
    x: i32,
    y: i32,
    z: i32,
) -> (Vec<ItemStack>, u64) {
    let mut spill: Vec<ItemStack> = Vec::new();
    let mut escrow: u64 = 0;
    if let Some(v) = world.vendor_at((x, y, z)) {
        spill = stocked_items(v);
        if let Some(req) = &v.barter_request {
            spill.push(req.clone());
        }
        escrow = v.escrow_sats;
    }
    world.remove_block_entity((x, y, z));
    (spill, escrow)
}

/// Spec 21 Phase 6 + second-pass review #4 — find the buyer's
/// inventory slot holding the Barter-requested item, if any. Returns
/// the slot index 0..36 or None when the buyer doesn't hold a
/// matching item. Item-type matching only (block-id or material-id
/// equality); ignores stack count.
///
/// Pure function for testability; the game_loop transaction calls
/// this both for the dim-check and for the consume step.
pub fn find_barter_request_slot(
    inventory: &crate::inventory::Inventory,
    request: &ItemStack,
) -> Option<usize> {
    (0..36).find(|&i| {
        inventory.slot(i).is_some_and(|s| {
            match (&s.item, &request.item) {
                (crate::item::Item::Block(a), crate::item::Item::Block(b)) => a == b,
                (crate::item::Item::Material(a), crate::item::Item::Material(b)) => a == b,
                // Tools / plans never match by type — they're per-
                // instance items and don't make sense as Barter
                // requests on alpha.
                _ => false,
            }
        })
    })
}

/// Spec 25 Phase 3 — does the held item type satisfy the slot filter
/// for the given mode? Plan-listing modes accept only `Item::Plan(_)`;
/// every other mode accepts anything (the existing item-type checks in
/// `OwnerSlotDeposit` continue to enforce stack compatibility once the
/// slot is non-empty).
pub fn slot_accepts_for_mode(mode: VendorMode, stack: &ItemStack) -> bool {
    // A chance drop (Satori) never takes a sats price — Barter only.
    if mode.requires_sats() && crate::economy::is_chance_drop(&stack.item) {
        return false;
    }
    if mode.is_plan_mode() {
        matches!(stack.item, Item::Plan(_))
    } else {
        // Non-plan modes can't be used to list a plan — plans don't
        // stack and the Sell/Buy/Barter codepaths assume a stackable
        // slot. Reject plans at deposit-time so they don't get stuck
        // in a vendor where the buyer-side rendering doesn't know how
        // to display them.
        !matches!(stack.item, Item::Plan(_))
    }
}

/// Spec 25 Phase 3 — refuse to list a Licence-tier plan as Master.
/// Returns `true` if the (mode, plan) combination is legal. The plan's
/// own `is_master` bit is the source of truth: a Licence-tier plan
/// (`is_master = false`) can only be listed as `SellPlanLicence`,
/// never as `SellPlanMaster`, because you can't sell what you don't
/// own. Non-plan modes are unaffected.
pub fn plan_listing_allowed(mode: VendorMode, plan: &crate::plan::PlanData) -> bool {
    match mode {
        VendorMode::SellPlanMaster => plan.is_master,
        VendorMode::SellPlanLicence => true,
        _ => true,
    }
}

/// Spec 25 Phase 6 — outcome of a plan-listing purchase. Carries the
/// hand-off Plan + the sats payout breakdown so the caller can credit
/// the buyer + commit the listing state in one consistent step.
#[derive(Clone, Debug)]
pub struct PlanPurchaseOutcome {
    /// The Plan stack to put into the buyer's inventory. `is_master`
    /// is `true` only for `SellPlanMaster` sales; Licence sales clone
    /// the plan with `is_master = false` so the buyer can't Save-As.
    pub handed_plan: ItemStack,
    /// Sats payout result from `apply_sats_payout`. Caller credits the
    /// owner's escrow with `result.credited`.
    pub payout: PayoutResult,
    /// Stock value after the purchase committed. `0` means the
    /// listing locks; `UNLIMITED_STOCK` means a Licence with the
    /// Unlimited marker (unchanged from before the call).
    pub new_stock: u32,
}

/// Spec 25 Phase 6 — attempt to settle a plan-listing purchase. Pure
/// function: takes a snapshot of the vendor + policy + Charter flag,
/// returns either the `BuyRefusal` (caller toasts) or the outcome
/// (caller applies). Mirrors the precondition checks of the existing
/// Sell/Buy/Barter path so the UI dim-state and the actual gate stay
/// in lockstep.
pub fn try_buy_plan(
    data: &VendorData,
    policy: &ServerSatsPolicy,
    charter_allows_sats: bool,
) -> Result<PlanPurchaseOutcome, BuyRefusal> {
    let Some(mode) = data.mode else {
        return Err(BuyRefusal::OutOfStock);
    };
    if !mode.is_plan_mode() {
        return Err(BuyRefusal::OutOfStock);
    }
    if !charter_allows_sats {
        return Err(BuyRefusal::CharterDeny);
    }
    if !policy.bitcoin_enabled {
        return Err(BuyRefusal::BitcoinDisabled);
    }
    if data.stock == 0 {
        return Err(BuyRefusal::OutOfStock);
    }
    let Some(slot) = data.slot.as_ref() else {
        return Err(BuyRefusal::OutOfStock);
    };
    let plan = match &slot.item {
        Item::Plan(p) => p,
        _ => return Err(BuyRefusal::OutOfStock),
    };
    let payout = apply_sats_payout(
        data.price_sats as u64,
        PayoutKind::VendorSale,
        policy,
        charter_allows_sats,
    );
    if payout.suppressed {
        return Err(BuyRefusal::CharterDeny);
    }
    // Build the buyer's copy of the plan. Master sales hand the
    // original (is_master stays true — the owner is selling the
    // single Master copy they hold). Licence sales clone the plan
    // and flip `is_master = false` so the buyer can build it but
    // can't re-list it at the Master tier.
    let mut handed_plan = plan.clone();
    if matches!(mode, VendorMode::SellPlanLicence) {
        handed_plan.is_master = false;
    }
    let handed = ItemStack {
        item: Item::Plan(handed_plan),
        count: 1,
    };
    // Stock accounting. Unlimited licence listings never decrement.
    // Everything else decrements by 1; a listing that hits 0 locks.
    let new_stock = if data.stock == UNLIMITED_STOCK {
        UNLIMITED_STOCK
    } else {
        data.stock.saturating_sub(1)
    };
    Ok(PlanPurchaseOutcome { handed_plan: handed, payout, new_stock })
}

/// Pure helper: pre-flight a buyer's potential refusal so the UI can
/// dim the Buy button before the click. Mirrors the gates inside
/// `try_buy` so the UI dim-state and the actual transaction filter stay
/// in lockstep. Spec 40 Phase 2 — moved from `vendor_ui` into the
/// vendor module so `try_buy` and the UI share one canonical source.
pub fn preview_refusal(
    data: &VendorData,
    mode: VendorMode,
    charter_allows_sats: bool,
    bitcoin_enabled: bool,
) -> Option<BuyRefusal> {
    if mode.requires_sats() {
        if data
            .slot
            .as_ref()
            .is_some_and(|s| crate::economy::is_chance_drop(&s.item))
        {
            return Some(BuyRefusal::ChanceDropNotForSats);
        }
        if !charter_allows_sats {
            return Some(BuyRefusal::CharterDeny);
        }
        if !bitcoin_enabled {
            return Some(BuyRefusal::BitcoinDisabled);
        }
    }
    if data.stock == 0 {
        return Some(BuyRefusal::OutOfStock);
    }
    None
}

/// Spec 40 Phase 2 — outcome of a successful `try_buy` call. Carries
/// the deltas the caller applies to inventory + vendor state + village
/// treasury. The split keeps the helper pure (no `&mut World`) so it
/// stays unit-testable.
#[derive(Clone, Debug)]
pub struct BuyOutcome {
    /// ItemStack to hand to the buyer. `count` is always 1 in Phase 2
    /// (Sell/Buy/Barter modes transfer one unit per click); Phase 3's
    /// Bulk mode will set this to `lot_size`.
    pub handed: ItemStack,
    /// Sats payout result — `Some` for Sell/Buy modes (which call
    /// `apply_sats_payout`), `None` for Barter (no sats movement). The
    /// payout side effect already happened inside `try_buy` by the time the
    /// caller sees this; game_loop.rs's call site doesn't read the field
    /// back. Tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub payout: Option<PayoutResult>,
    /// Buyer's inventory slot to decrement, for Barter mode only. The
    /// caller is responsible for the actual `set_slot` mutation — the
    /// helper just identifies which slot holds the barter-requested
    /// item.
    pub barter_consume_slot: Option<usize>,
    /// Units to subtract from `data.stock`. Phase 2: always 1.
    /// Phase 3: `lot_size` for Bulk mode.
    pub stock_delta: u32,
    /// Sats added to the vendor owner's `escrow_sats`. Sell mode only;
    /// 0 for Buy / Barter. Already net of the village skim.
    pub escrow_credit: u64,
    /// Sats credited to the surrounding village's treasury (the 5 %
    /// vendor-in-village tax — Spec 22 Phase 13). Sell mode only;
    /// 0 for Buy / Barter / Sell-outside-village. The caller resolves
    /// the village id via `raid::village_for_vendor_position`.
    pub village_skim: u64,
}

/// Spec 40 Phase 2 — behaviour-preserving extract of the inline
/// Sell/Buy/Barter buy logic that used to live in
/// `game_loop.rs`'s `VendorUiOutcome::BuyerBuy` arm. Pure: takes a
/// snapshot of the vendor + policy + Charter flag + the buyer's
/// inventory (read-only — for the Barter slot lookup), returns either
/// a `BuyRefusal` or a `BuyOutcome` for the caller to apply.
///
/// Phase 2 contract — for non-plan modes only. Plan modes route through
/// `try_buy_plan` (which the caller picks via `mode.is_plan_mode()`).
/// Passing a plan mode here returns `OutOfStock` defensively rather
/// than producing a corrupt outcome.
///
/// Order of checks mirrors the inline original:
/// 1. `preview_refusal` (Charter / Bitcoin-on / Stock).
/// 2. Slot non-empty (defensive — preview catches `Stock == 0`).
/// 3. Barter slot find on the buyer's inventory (only Barter mode).
/// 4. `apply_sats_payout` (only `mode.requires_sats()`); suppression
///    bails as `CharterDeny`.
/// 5. Compute Sell-mode payment split via `raid::split_vendor_payment`.
/// 6. Build the `handed` stack (count = 1).
pub fn try_buy(
    data: &VendorData,
    mode: VendorMode,
    policy: &ServerSatsPolicy,
    charter_allows_sats: bool,
    buyer_inventory: &crate::inventory::Inventory,
) -> Result<BuyOutcome, BuyRefusal> {
    // Defensive — plan modes are handled by try_buy_plan.
    if mode.is_plan_mode() {
        return Err(BuyRefusal::OutOfStock);
    }
    // Buy mode (vendor buys FROM the player) is DISABLED pre-alpha: the inline
    // original handed the buyer a FREE copy of the wanted item and consumed
    // nothing (item-dup exploit). A correct Buy must consume the buyer's item and
    // credit sats — that needs the sats-settlement layer. Refuse until then, so
    // the exploit can't fire even on a vendor already saved in Buy mode. Buy is
    // also omitted from the owner mode picker. (Pre-alpha bug hunt 2026-06-22.)
    if mode == VendorMode::Buy {
        return Err(BuyRefusal::OutOfStock);
    }
    if let Some(refusal) = preview_refusal(
        data, mode, charter_allows_sats, policy.bitcoin_enabled,
    ) {
        return Err(refusal);
    }
    let Some(item_stack) = data.slot.as_ref() else {
        return Err(BuyRefusal::OutOfStock);
    };
    // Spec 40 — Bulk mode transfers `lot_size` units per click; every
    // other mode transfers 1. `data.lot_size` defaults to 1 via the
    // serde-default + `DEFAULT_LOT_SIZE` constant; `.max(1)` guards
    // against a stray 0 (the derive(Default) zero) so a non-Bulk
    // listing always behaves as a single-unit transfer.
    let units = if mode == VendorMode::Bulk {
        let lot = data.lot_size.max(DEFAULT_LOT_SIZE);
        if data.stock < lot {
            return Err(BuyRefusal::OutOfStock);
        }
        lot
    } else {
        1
    };
    let total_price = (data.price_sats as u64).saturating_mul(units as u64);
    let barter_consume_slot: Option<usize> = if mode == VendorMode::Barter {
        if let Some(request) = &data.barter_request {
            let found = find_barter_request_slot(buyer_inventory, request);
            if found.is_none() {
                return Err(BuyRefusal::BarterRequestMissing);
            }
            found
        } else {
            None
        }
    } else {
        None
    };
    let payout = if mode.requires_sats() {
        let p = apply_sats_payout(
            total_price,
            PayoutKind::VendorSale,
            policy,
            charter_allows_sats,
        );
        if p.suppressed {
            return Err(BuyRefusal::CharterDeny);
        }
        Some(p)
    } else {
        None
    };
    // Sell + Bulk both pay the Spec 22 Phase 13 village skim (5 %).
    // Buy / Barter don't — Barter moves no sats; Buy is the vendor-
    // pays-buyer direction which the inline original never wired to
    // the treasury skim.
    let (escrow_credit, village_skim) = if mode == VendorMode::Sell || mode == VendorMode::Bulk {
        crate::raid::split_vendor_payment(total_price)
    } else {
        (0, 0)
    };
    let mut handed = item_stack.clone();
    // lot_size capped at 64 in the picker so the u8 cast is safe; a
    // mis-set lot_size from a corrupted save clamps at u8::MAX.
    handed.count = units.min(u8::MAX as u32) as u8;
    Ok(BuyOutcome {
        handed,
        payout,
        barter_consume_slot,
        stock_delta: units,
        escrow_credit,
        village_skim,
    })
}

/// Rail Freight P2 (commercial freight) — a **Bulk** vendor draws its sale stock
/// from an adjacent **freight depot chest** (the cart's unload destination),
/// never the owner's pocket: that's the "selling at volume routes through
/// freight" rule (single-item Sell vendors stay pocket-fed). Move every depot
/// stack matching the vendor's configured slot item into the vendor's `stock`
/// counter, emptying those chest slots; returns the units restocked. A no-op for
/// non-Bulk vendors or one with no item configured. Pure — operates on the two
/// data structs; the caller finds the depot via [`crate::rail::depot_chest_for`].
/// Earnings still flow through the existing `try_buy` → `escrow_sats` path, so no
/// real-settlement decision is touched (in-world score only).
pub fn restock_bulk_from_depot(
    vendor: &mut VendorData,
    depot: &mut crate::chest::ChestData,
) -> u32 {
    if vendor.mode != Some(VendorMode::Bulk) {
        return 0;
    }
    let Some(template) = vendor.slot.clone() else {
        return 0;
    };
    let mut moved = 0u32;
    for slot in depot.slots.iter_mut() {
        let take = match slot {
            Some(s) if s.item.can_stack_with(&template.item) => s.count,
            _ => continue,
        };
        moved += take as u32;
        *slot = None;
    }
    vendor.stock = vendor.stock.saturating_add(moved);
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_vendor_is_empty_and_unowned() {
        let v = VendorData::default();
        assert!(v.owner.is_none());
        assert!(v.mode.is_none());
        assert!(v.slot.is_none());
        assert!(v.barter_request.is_none());
        assert_eq!(v.price_sats, 0);
        assert_eq!(v.stock, 0);
        assert_eq!(v.escrow_sats, 0);
        assert_eq!(v.last_txn_tick, 0);
    }

    #[test]
    fn bulk_vendor_restocks_from_freight_depot_only_matching_items() {
        // Rail Freight P2 — a Bulk vendor selling oak logs pulls matching freight
        // out of the adjacent depot chest into its stock; other freight stays put.
        use crate::chest::ChestData;
        use crate::item::ItemStack;
        let mut v = VendorData {
            mode: Some(VendorMode::Bulk),
            slot: Some(ItemStack::new_block(crate::block::OAK_LOG, 1)),
            stock: 0,
            ..Default::default()
        };
        let mut depot = ChestData::new();
        depot.slots[0] = Some(ItemStack::new_block(crate::block::OAK_LOG, 30));
        depot.slots[1] = Some(ItemStack::new_block(crate::block::OAK_LOG, 12));
        depot.slots[2] = Some(ItemStack::new_block(crate::block::STONE, 8)); // not the wares
        let moved = restock_bulk_from_depot(&mut v, &mut depot);
        assert_eq!(moved, 42, "30 + 12 oak logs restocked");
        assert_eq!(v.stock, 42);
        assert!(depot.slots[0].is_none() && depot.slots[1].is_none(), "logs left the depot");
        assert_eq!(depot.slots[2].as_ref().map(|s| s.count), Some(8), "non-wares stay");

        // A non-Bulk (pocket-fed Sell) vendor never sources from the depot.
        let mut sell = VendorData {
            mode: Some(VendorMode::Sell),
            slot: Some(ItemStack::new_block(crate::block::OAK_LOG, 1)),
            ..Default::default()
        };
        let mut d2 = ChestData::new();
        d2.slots[0] = Some(ItemStack::new_block(crate::block::OAK_LOG, 5));
        assert_eq!(restock_bulk_from_depot(&mut sell, &mut d2), 0);
        assert_eq!(d2.slots[0].as_ref().map(|s| s.count), Some(5), "Sell vendor leaves the depot alone");
    }

    // --- Audit 2026-09-27: vendor stock accounting ---

    fn cobble(n: u8) -> ItemStack {
        ItemStack::new_block(crate::block::COBBLESTONE, n)
    }

    #[test]
    fn unlimited_sentinel_cannot_be_carried_into_sell() {
        // The audit scenario: deposit 1 in Sell, switch to Licence, click
        // Unlimited, switch back to Sell, break.
        let mut v = VendorData { mode: Some(VendorMode::Sell), slot: Some(cobble(1)), stock: 1, ..Default::default() };
        change_mode(&mut v, VendorMode::SellPlanLicence).unwrap();
        v.stock = UNLIMITED_STOCK; // the owner's Unlimited click
        change_mode(&mut v, VendorMode::Sell).unwrap();
        assert_eq!(v.stock, 1, "Sell stock is the real slot count, not the sentinel");
        let spill = stocked_items(&v);
        assert_eq!(spill.iter().map(|s| s.count as u32).sum::<u32>(), 1, "1 in, 1 out");
    }

    #[test]
    fn a_licence_count_never_multiplies_the_spill() {
        // Licence stock=50 → Sell → break used to spill 50 copies.
        let mut v = VendorData { mode: Some(VendorMode::SellPlanLicence), slot: Some(cobble(1)), stock: 50, ..Default::default() };
        let mut w = crate::world::World::new();
        w.insert_vendor((0, 0, 0), v.clone());
        let (spill, _) = cleanup_vendor(&mut w, 0, 0, 0);
        assert_eq!(spill.iter().map(|s| s.count as u32).sum::<u32>(), 1, "plan-mode stock is a counter");
        change_mode(&mut v, VendorMode::Sell).unwrap();
        assert_eq!(v.stock, 1);
        // Even a corrupt save holding the sentinel in Bulk spills only the slot.
        let corrupt = VendorData { mode: Some(VendorMode::Bulk), slot: Some(cobble(3)), stock: UNLIMITED_STOCK, ..Default::default() };
        assert_eq!(stocked_items(&corrupt).iter().map(|s| s.count as u32).sum::<u32>(), 3);
    }

    #[test]
    fn bulk_freight_stays_sellable_after_the_first_lot_and_spills_on_break() {
        use crate::chest::ChestData;
        let log = |n| ItemStack::new_block(crate::block::OAK_LOG, n);
        // Owner deposits 1 log as the template; the depot adds 42.
        let mut v = VendorData { mode: Some(VendorMode::Bulk), slot: Some(log(1)), stock: 1, lot_size: 8, price_sats: 1, ..Default::default() };
        let mut depot = ChestData::new();
        depot.slots[0] = Some(log(42));
        assert_eq!(restock_bulk_from_depot(&mut v, &mut depot), 42);
        assert_eq!(v.stock, 43);
        // A buyer takes a lot of 8.
        apply_sale(&mut v, 8);
        assert_eq!(v.stock, 35);
        assert!(v.slot.is_some(), "the template survives the sale");
        let policy = ServerSatsPolicy { bitcoin_enabled: true, ..Default::default() };
        let inv = crate::inventory::Inventory::new();
        assert!(try_buy(&v, VendorMode::Bulk, &policy, true, &inv).is_ok(), "the next lot still sells");
        // Freight restocks keep working (the template is still there).
        depot.slots[1] = Some(log(5));
        assert_eq!(restock_bulk_from_depot(&mut v, &mut depot), 5);
        // Breaking spills every remaining unit.
        let mut w = crate::world::World::new();
        w.insert_vendor((1, 2, 3), v.clone());
        let (spill, _) = cleanup_vendor(&mut w, 1, 2, 3);
        assert_eq!(spill.iter().map(|s| s.count as u32).sum::<u32>(), 40);
        // Switching mode carries the freight into the slot instead of dropping it.
        change_mode(&mut v, VendorMode::Sell).unwrap();
        assert_eq!((v.stock, v.slot.as_ref().unwrap().count), (40, 40));
        // Selling the last unit clears the Bulk template.
        let mut b = VendorData { mode: Some(VendorMode::Bulk), slot: Some(log(8)), stock: 8, ..Default::default() };
        apply_sale(&mut b, 8);
        assert!(b.slot.is_none() && b.stock == 0);
    }

    #[test]
    fn a_bulk_stock_too_big_for_one_slot_refuses_a_mode_switch() {
        let mut v = VendorData { mode: Some(VendorMode::Bulk), slot: Some(cobble(1)), stock: 600, ..Default::default() };
        assert_eq!(change_mode(&mut v, VendorMode::Sell), Err(ModeChangeRefusal::TooMuchStock));
        assert_eq!((v.mode, v.stock), (Some(VendorMode::Bulk), 600), "nothing changed");
        let taken = take_stock(&mut v);
        assert_eq!(taken.iter().map(|s| s.count as u32).sum::<u32>(), 600, "withdraw returns all of it");
        assert!(v.slot.is_none() && v.stock == 0);
    }

    #[test]
    fn mode_requires_sats_for_sell_and_buy() {
        assert!(VendorMode::Sell.requires_sats());
        assert!(VendorMode::Buy.requires_sats());
        assert!(!VendorMode::Barter.requires_sats());
        // Spec 25 — plan-listing modes both move sats.
        assert!(VendorMode::SellPlanMaster.requires_sats());
        assert!(VendorMode::SellPlanLicence.requires_sats());
    }

    #[test]
    fn mode_labels_are_human_readable() {
        assert_eq!(VendorMode::Sell.label(), "Sell");
        assert_eq!(VendorMode::Buy.label(), "Buy");
        assert_eq!(VendorMode::Barter.label(), "Barter");
        assert_eq!(VendorMode::SellPlanMaster.label(), "Sell Plan (Master)");
        assert_eq!(VendorMode::SellPlanLicence.label(), "Sell Plan (Licence)");
    }

    #[test]
    fn is_plan_mode_only_true_for_plan_variants() {
        assert!(VendorMode::SellPlanMaster.is_plan_mode());
        assert!(VendorMode::SellPlanLicence.is_plan_mode());
        assert!(!VendorMode::Sell.is_plan_mode());
        assert!(!VendorMode::Buy.is_plan_mode());
        assert!(!VendorMode::Barter.is_plan_mode());
    }

    #[test]
    fn owner_is_local_matches_only_same_pidx() {
        let owner = VendorOwner::LocalPlayer(2);
        assert!(owner.is_local(2));
        assert!(!owner.is_local(0));
        assert!(!owner.is_local(1));
    }

    #[test]
    fn owner_npub_variant_is_never_local() {
        let owner = VendorOwner::Npub("npub1abc".to_string());
        // Even with pidx 0, an Npub owner is never the local player
        // — that's a future-multiplayer concern.
        assert!(!owner.is_local(0));
    }

    #[test]
    fn buy_refusal_messages_are_short_and_actionable() {
        for r in [
            BuyRefusal::OutOfStock,
            BuyRefusal::InsufficientSats,
            BuyRefusal::InventoryFull,
            BuyRefusal::CharterDeny,
            BuyRefusal::BitcoinDisabled,
            BuyRefusal::BarterRequestMissing,
        ] {
            let msg = r.message();
            assert!(!msg.is_empty());
            // Toast budget — UI panel is narrow; messages should fit.
            assert!(msg.len() < 50, "message too long for toast: {msg:?}");
        }
    }

    #[test]
    fn find_barter_request_slot_returns_first_match() {
        // Second-pass review #4 — Barter buyer-side check.
        let mut inv = crate::inventory::Inventory::new();
        inv.add_item(ItemStack::new_block(crate::block::OAK_PLANKS, 5));
        inv.add_item(ItemStack::new_material(crate::item::MaterialId::Stick, 3));
        // Request matches a block in inventory.
        let req_block = ItemStack::new_block(crate::block::OAK_PLANKS, 1);
        assert_eq!(find_barter_request_slot(&inv, &req_block), Some(0));
        // Request matches a material in inventory.
        let req_mat = ItemStack::new_material(crate::item::MaterialId::Stick, 1);
        assert_eq!(find_barter_request_slot(&inv, &req_mat), Some(1));
    }

    #[test]
    fn find_barter_request_slot_returns_none_when_missing() {
        let mut inv = crate::inventory::Inventory::new();
        inv.add_item(ItemStack::new_block(crate::block::OAK_PLANKS, 5));
        let req = ItemStack::new_material(crate::item::MaterialId::IronIngot, 1);
        assert_eq!(find_barter_request_slot(&inv, &req), None);
    }

    #[test]
    fn find_barter_request_slot_empty_inventory_returns_none() {
        let inv = crate::inventory::Inventory::new();
        let req = ItemStack::new_block(crate::block::OAK_PLANKS, 1);
        assert_eq!(find_barter_request_slot(&inv, &req), None);
    }

    #[test]
    fn find_barter_request_slot_ignores_count() {
        // Request asks for 5 of something; buyer has 1. The matcher
        // returns Some — count enforcement is the caller's job. (On
        // alpha the consume step takes 1 at a time, so a buyer with
        // a single item can complete a Barter where the request's
        // count is misleading.)
        let mut inv = crate::inventory::Inventory::new();
        inv.add_item(ItemStack::new_material(crate::item::MaterialId::Wheat, 1));
        let req = ItemStack::new_material(crate::item::MaterialId::Wheat, 5);
        assert_eq!(find_barter_request_slot(&inv, &req), Some(0));
    }

    #[test]
    fn vendor_data_round_trips_via_bincode() {
        let v = VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::Sell),
            slot: Some(crate::item::ItemStack::new_block(crate::block::OAK_PLANKS, 16)),
            barter_request: None,
            price_sats: 10,
            stock: 4,
            escrow_sats: 30,
            last_txn_tick: 123,
            lot_size: 1,
        };
        let bytes = bincode::serialize(&v).unwrap();
        let back: VendorData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.owner, v.owner);
        assert_eq!(back.mode, v.mode);
        assert_eq!(back.price_sats, v.price_sats);
        assert_eq!(back.stock, v.stock);
        assert_eq!(back.escrow_sats, v.escrow_sats);
        assert_eq!(back.last_txn_tick, v.last_txn_tick);
    }

    // ─── Spec 25 — VendorMode plan variants ───────────────────────────

    fn plan_stack() -> ItemStack {
        ItemStack {
            item: Item::Plan(crate::plan::PlanData::debug_3x3_stone()),
            count: 1,
        }
    }

    fn licence_plan_stack() -> ItemStack {
        let mut p = crate::plan::PlanData::debug_3x3_stone();
        p.is_master = false;
        ItemStack { item: Item::Plan(p), count: 1 }
    }

    #[test]
    fn sell_plan_master_round_trips_via_bincode() {
        let v = VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::SellPlanMaster),
            slot: Some(plan_stack()),
            barter_request: None,
            price_sats: 5_000,
            stock: 1,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size: 1,
        };
        let bytes = bincode::serialize(&v).unwrap();
        let back: VendorData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.mode, Some(VendorMode::SellPlanMaster));
        assert_eq!(back.price_sats, 5_000);
        assert!(matches!(back.slot.unwrap().item, Item::Plan(_)));
    }

    #[test]
    fn sell_plan_licence_with_unlimited_stock_round_trips() {
        let v = VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::SellPlanLicence),
            slot: Some(plan_stack()),
            barter_request: None,
            price_sats: 100,
            stock: UNLIMITED_STOCK,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size: 1,
        };
        let bytes = bincode::serialize(&v).unwrap();
        let back: VendorData = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back.mode, Some(VendorMode::SellPlanLicence));
        assert_eq!(back.stock, UNLIMITED_STOCK);
    }

    #[test]
    fn slot_accepts_for_mode_filters_plan_modes_to_plans_only() {
        let plan = plan_stack();
        let block = ItemStack::new_block(crate::block::OAK_PLANKS, 4);
        assert!(slot_accepts_for_mode(VendorMode::SellPlanMaster, &plan));
        assert!(slot_accepts_for_mode(VendorMode::SellPlanLicence, &plan));
        assert!(!slot_accepts_for_mode(VendorMode::SellPlanMaster, &block));
        assert!(!slot_accepts_for_mode(VendorMode::SellPlanLicence, &block));
    }

    #[test]
    fn slot_accepts_for_mode_keeps_plans_out_of_non_plan_modes() {
        let plan = plan_stack();
        let block = ItemStack::new_block(crate::block::OAK_PLANKS, 4);
        assert!(slot_accepts_for_mode(VendorMode::Sell, &block));
        assert!(slot_accepts_for_mode(VendorMode::Buy, &block));
        assert!(slot_accepts_for_mode(VendorMode::Barter, &block));
        assert!(!slot_accepts_for_mode(VendorMode::Sell, &plan));
        assert!(!slot_accepts_for_mode(VendorMode::Buy, &plan));
        assert!(!slot_accepts_for_mode(VendorMode::Barter, &plan));
    }

    #[test]
    fn plan_listing_allowed_blocks_licence_at_master_tier() {
        let master = crate::plan::PlanData::debug_3x3_stone();
        let mut licence = master.clone();
        licence.is_master = false;
        // Master plan: legal at both tiers.
        assert!(plan_listing_allowed(VendorMode::SellPlanMaster, &master));
        assert!(plan_listing_allowed(VendorMode::SellPlanLicence, &master));
        // Licence plan: only legal at the Licence tier.
        assert!(!plan_listing_allowed(VendorMode::SellPlanMaster, &licence));
        assert!(plan_listing_allowed(VendorMode::SellPlanLicence, &licence));
        // Non-plan modes always pass — they don't carry a plan.
        assert!(plan_listing_allowed(VendorMode::Sell, &licence));
    }

    fn master_plan_listing() -> VendorData {
        VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::SellPlanMaster),
            slot: Some(plan_stack()),
            barter_request: None,
            price_sats: 1_000,
            stock: 1,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size: 1,
        }
    }

    fn licence_plan_listing(stock: u32) -> VendorData {
        VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::SellPlanLicence),
            slot: Some(plan_stack()),
            barter_request: None,
            price_sats: 100,
            stock,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size: 1,
        }
    }

    /// Review S2 — a Licence vendor (counter 50, or Unlimited) switched
    /// into Bulk by pre-fix code (which kept `stock`) is loaded from a save.
    /// It must not spill / withdraw / mode-switch into 50 Plans.
    #[test]
    fn legacy_bulk_plan_vendor_is_clamped_on_load() {
        for legacy_stock in [50u32, UNLIMITED_STOCK - 1] {
            let mut legacy = licence_plan_listing(legacy_stock);
            legacy.mode = Some(VendorMode::Bulk); // the pre-fix switch: stock kept
            // Round-trip through the real save-load path.
            let mut save = crate::save::minimal_world_save_for_tests(1);
            save.vendors = vec![crate::save::SavedVendor { x: 1, y: 2, z: 3, data: legacy }];
            let mut w = crate::world::World::new();
            crate::save::apply_world_save_state(&mut w, &save);
            let v = w.vendor_at((1, 2, 3)).expect("vendor restored").clone();
            let spilled: u32 = stocked_items(&v).iter().map(|s| s.count as u32).sum();
            assert_eq!(spilled, 1, "one Plan held → one Plan out (was {legacy_stock})");
            let mut sw = v.clone();
            change_mode(&mut sw, VendorMode::Sell).expect("fits one slot");
            assert_eq!(sw.slot.as_ref().map(|s| s.count), Some(1));
            let mut wd = v;
            assert_eq!(take_stock(&mut wd).iter().map(|s| s.count as u32).sum::<u32>(), 1);
        }
    }

    #[test]
    fn repair_legacy_bulk_leaves_real_bulk_freight_alone() {
        let mut v = VendorData {
            mode: Some(VendorMode::Bulk),
            slot: Some(ItemStack::new_material(crate::item::MaterialId::Stick, 1)),
            stock: 300,
            lot_size: 8,
            ..VendorData::default()
        };
        repair_legacy_bulk(&mut v);
        assert_eq!(v.stock, 300, "real Bulk freight is not a legacy count");
    }

    #[test]
    fn try_buy_plan_master_decrements_stock_and_hands_master() {
        let data = master_plan_listing();
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let outcome = try_buy_plan(&data, &policy, true).expect("should succeed");
        assert_eq!(outcome.new_stock, 0);
        match &outcome.handed_plan.item {
            Item::Plan(p) => assert!(p.is_master, "Master sale must hand a Master plan"),
            _ => panic!("expected plan"),
        }
        // Default policy = no skim, so the buyer paid the full price.
        assert_eq!(outcome.payout.credited, 1_000);
    }

    #[test]
    fn try_buy_plan_master_refuses_second_buy_after_listing_locks() {
        let mut data = master_plan_listing();
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let first = try_buy_plan(&data, &policy, true).unwrap();
        // Simulate the caller committing the new stock.
        data.stock = first.new_stock;
        let second = try_buy_plan(&data, &policy, true);
        assert_eq!(second.err(), Some(BuyRefusal::OutOfStock));
    }

    #[test]
    fn try_buy_plan_licence_hands_licence_copy() {
        let data = licence_plan_listing(5);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let outcome = try_buy_plan(&data, &policy, true).expect("should succeed");
        assert_eq!(outcome.new_stock, 4);
        match &outcome.handed_plan.item {
            Item::Plan(p) => assert!(!p.is_master, "Licence sale must flip is_master off"),
            _ => panic!("expected plan"),
        }
    }

    #[test]
    fn try_buy_plan_licence_unlimited_never_decrements() {
        let data = licence_plan_listing(UNLIMITED_STOCK);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let outcome = try_buy_plan(&data, &policy, true).unwrap();
        assert_eq!(outcome.new_stock, UNLIMITED_STOCK);
    }

    #[test]
    fn try_buy_plan_licence_decrements_to_zero_then_refuses() {
        let mut data = licence_plan_listing(2);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let first = try_buy_plan(&data, &policy, true).unwrap();
        assert_eq!(first.new_stock, 1);
        data.stock = first.new_stock;
        let second = try_buy_plan(&data, &policy, true).unwrap();
        assert_eq!(second.new_stock, 0);
        data.stock = second.new_stock;
        let third = try_buy_plan(&data, &policy, true);
        assert_eq!(third.err(), Some(BuyRefusal::OutOfStock));
    }

    #[test]
    fn try_buy_plan_charter_off_refuses_and_does_not_settle() {
        let data = master_plan_listing();
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let outcome = try_buy_plan(&data, &policy, false);
        assert_eq!(outcome.err(), Some(BuyRefusal::CharterDeny));
    }

    #[test]
    fn try_buy_plan_bitcoin_disabled_server_refuses() {
        let data = master_plan_listing();
        let policy = ServerSatsPolicy { bitcoin_enabled: false, ..Default::default() };
        let outcome = try_buy_plan(&data, &policy, true);
        assert_eq!(outcome.err(), Some(BuyRefusal::BitcoinDisabled));
    }

    #[test]
    fn try_buy_plan_routes_payout_through_sats_helper_with_vendor_sale_kind() {
        // 10% server tax → buyer's gross 1000 sats settles into 900
        // credited + 100 server skim. Reusing PayoutKind::VendorSale is
        // the spec-mandated reuse (no new kind for plan sales).
        let data = master_plan_listing();
        let policy = ServerSatsPolicy { server_tax_bps: 1000, ..ServerSatsPolicy::bitcoin_enabled_policy() };
        let outcome = try_buy_plan(&data, &policy, true).unwrap();
        assert_eq!(outcome.payout.credited, 900);
        assert_eq!(outcome.payout.server_skim, 100);
        assert!(!outcome.payout.suppressed);
    }

    #[test]
    fn try_buy_plan_refuses_when_slot_holds_non_plan() {
        let mut data = master_plan_listing();
        // Pathological state: mode is SellPlanMaster but the slot
        // holds a block. The buyer-side gate must refuse rather than
        // hand a Plan(...) constructed from a block.
        data.slot = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 1));
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let outcome = try_buy_plan(&data, &policy, true);
        assert_eq!(outcome.err(), Some(BuyRefusal::OutOfStock));
    }

    #[test]
    fn try_buy_plan_refuses_non_plan_modes() {
        // Sanity — a Sell-mode listing must not be settled through
        // try_buy_plan even if the slot somehow holds a plan.
        let mut data = master_plan_listing();
        data.mode = Some(VendorMode::Sell);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let outcome = try_buy_plan(&data, &policy, true);
        assert_eq!(outcome.err(), Some(BuyRefusal::OutOfStock));
    }

    #[test]
    fn licence_plan_inventory_held_round_trips_through_listing() {
        // End-to-end: a player who holds a Licence plan and tries to
        // list it as Master is refused by `plan_listing_allowed`. They
        // CAN list it as Licence — the original-author's permission is
        // implicit in the Plan they hold.
        let licence = licence_plan_stack();
        let plan = match &licence.item {
            Item::Plan(p) => p.clone(),
            _ => unreachable!(),
        };
        assert!(!plan_listing_allowed(VendorMode::SellPlanMaster, &plan));
        assert!(plan_listing_allowed(VendorMode::SellPlanLicence, &plan));
    }

    // ─── Spec 40 Phase 2 — `try_buy` behaviour-preservation tests ──

    fn sell_listing(price: u32, stock: u32) -> VendorData {
        VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::Sell),
            slot: Some(ItemStack::new_block(crate::block::OAK_PLANKS, stock.min(64) as u8)),
            barter_request: None,
            price_sats: price,
            stock,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size: 1,
        }
    }

    fn barter_listing() -> VendorData {
        VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::Barter),
            slot: Some(ItemStack::new_block(crate::block::OAK_PLANKS, 1)),
            barter_request: Some(ItemStack::new_block(crate::block::STONE, 1)),
            price_sats: 0,
            stock: 1,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size: 1,
        }
    }

    #[test]
    fn try_buy_sell_mode_full_happy_path() {
        // Sell mode at 1000 sats — split_vendor_payment applies the
        // Spec 22 Phase 13 village skim (5 %), so the seller keeps 950
        // and the helper surfaces a 50-sat `village_skim` for the
        // caller to either credit to a village treasury or drop on the
        // floor when the vendor sits outside any village.
        let data = sell_listing(1_000, 5);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let outcome = try_buy(&data, VendorMode::Sell, &policy, true, &inv)
            .expect("Sell mode happy path should succeed");
        assert_eq!(outcome.handed.count, 1, "v1 transfers one unit per click");
        assert_eq!(outcome.stock_delta, 1);
        assert!(outcome.barter_consume_slot.is_none(),
            "Sell mode never consumes from the buyer's inventory");
        assert_eq!(outcome.escrow_credit, 950);
        assert_eq!(outcome.village_skim, 50);
        assert!(outcome.payout.is_some(), "Sell mode runs apply_sats_payout");
    }

    #[test]
    fn try_buy_sell_mode_charter_off_refuses() {
        let data = sell_listing(100, 1);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::Sell, &policy, false, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::CharterDeny));
    }

    #[test]
    fn try_buy_sell_mode_bitcoin_disabled_refuses() {
        let data = sell_listing(100, 1);
        let mut policy = ServerSatsPolicy::bitcoin_enabled_policy();
        policy.bitcoin_enabled = false;
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::Sell, &policy, true, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::BitcoinDisabled));
    }

    #[test]
    fn try_buy_sell_mode_out_of_stock_refuses() {
        let data = sell_listing(100, 0);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::Sell, &policy, true, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::OutOfStock));
    }

    #[test]
    fn try_buy_buy_mode_is_disabled_no_free_item() {
        // Pre-alpha: Buy mode is disabled. The inline original handed the buyer a
        // free copy of the wanted item and consumed nothing (item-dup); try_buy
        // must refuse Buy so the dup can't fire even on a vendor saved in Buy mode.
        let data = VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::Buy),
            slot: Some(ItemStack::new_block(crate::block::OAK_PLANKS, 64)),
            barter_request: None,
            price_sats: 100,
            stock: 64,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size: 1,
        };
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::Buy, &policy, true, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::OutOfStock),
            "Buy mode must be refused — no free item handed");
    }

    #[test]
    fn try_buy_barter_finds_buyer_slot() {
        let data = barter_listing();
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let mut inv = crate::inventory::Inventory::new();
        // Put the requested item (STONE) into slot 5.
        inv.set_slot(5, Some(ItemStack::new_block(crate::block::STONE, 3)));
        let outcome = try_buy(&data, VendorMode::Barter, &policy, true, &inv)
            .expect("Barter with the requested item should succeed");
        assert_eq!(outcome.barter_consume_slot, Some(5));
        assert!(outcome.payout.is_none(),
            "Barter never invokes the sats helper — no Charter / Bitcoin gate");
        assert_eq!(outcome.escrow_credit, 0);
        assert_eq!(outcome.village_skim, 0);
    }

    #[test]
    fn try_buy_barter_missing_requested_item_refuses() {
        let data = barter_listing();
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        // Empty inventory — no STONE for the barter request.
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::Barter, &policy, true, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::BarterRequestMissing));
    }

    #[test]
    fn try_buy_refuses_plan_modes_defensively() {
        // Plan modes are routed through try_buy_plan; try_buy should
        // refuse them rather than produce a corrupt outcome.
        let mut data = sell_listing(100, 1);
        data.mode = Some(VendorMode::SellPlanMaster);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::SellPlanMaster, &policy, true, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::OutOfStock));
    }

    #[test]
    fn try_buy_handed_count_is_always_one_in_non_bulk_modes() {
        // Non-Bulk invariant — Sell / Buy / Barter all hand a single
        // unit regardless of the slot stack size. Locks the call-site
        // assumption that one click = one unit for the legacy modes.
        let mut data = sell_listing(100, 5);
        data.slot = Some(ItemStack::new_block(crate::block::OAK_PLANKS, 64));
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let outcome = try_buy(&data, VendorMode::Sell, &policy, true, &inv).unwrap();
        assert_eq!(outcome.handed.count, 1);
    }

    // ─── Spec 40 Phase 3-4 — Bulk mode tests ─────────────────────────

    fn bulk_listing(price: u32, stock: u32, lot_size: u32) -> VendorData {
        VendorData {
            owner: Some(VendorOwner::LocalPlayer(0)),
            mode: Some(VendorMode::Bulk),
            slot: Some(ItemStack::new_block(crate::block::OAK_PLANKS, stock.min(64) as u8)),
            barter_request: None,
            price_sats: price,
            stock,
            escrow_sats: 0,
            last_txn_tick: 0,
            lot_size,
        }
    }

    #[test]
    fn try_buy_bulk_transfers_lot_size_units_per_click() {
        // Bulk listing at 5 sats/unit × lot 16 → 80 sats total. Stock
        // is plenty; the helper must return handed.count = 16 +
        // stock_delta = 16 + payout sized to the total.
        let data = bulk_listing(5, 100, 16);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let outcome = try_buy(&data, VendorMode::Bulk, &policy, true, &inv)
            .expect("Bulk happy path should succeed");
        assert_eq!(outcome.handed.count, 16, "Bulk hands a full lot per click");
        assert_eq!(outcome.stock_delta, 16);
        // Total = 16 × 5 = 80 sats. Skim 5 % = 4; seller keeps 76.
        assert_eq!(outcome.escrow_credit, 76);
        assert_eq!(outcome.village_skim, 4);
    }

    #[test]
    fn try_buy_bulk_refuses_when_stock_below_lot_size() {
        // Bulk listing with 10 stock + lot 16 → can't sell a full lot,
        // so the buy is refused even though preview_refusal passes.
        let data = bulk_listing(5, 10, 16);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::Bulk, &policy, true, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::OutOfStock));
    }

    #[test]
    fn try_buy_bulk_accepts_when_stock_equals_lot_size() {
        // Edge case — stock == lot_size sells exactly one lot then
        // leaves the vendor empty.
        let data = bulk_listing(2, 8, 8);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let outcome = try_buy(&data, VendorMode::Bulk, &policy, true, &inv).unwrap();
        assert_eq!(outcome.stock_delta, 8);
        assert_eq!(outcome.handed.count, 8);
    }

    #[test]
    fn try_buy_bulk_charter_off_refuses() {
        let data = bulk_listing(5, 100, 16);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let out = try_buy(&data, VendorMode::Bulk, &policy, false, &inv);
        assert_eq!(out.err(), Some(BuyRefusal::CharterDeny));
    }

    #[test]
    fn try_buy_bulk_pays_village_skim() {
        // 64-lot at 10 sats/unit = 640 sats total. 5% skim = 32.
        // Seller keeps 608. Sanity-check the multiplication path that
        // makes Bulk economics scale with lot size.
        let data = bulk_listing(10, 200, 64);
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let inv = crate::inventory::Inventory::new();
        let outcome = try_buy(&data, VendorMode::Bulk, &policy, true, &inv).unwrap();
        assert_eq!(outcome.escrow_credit + outcome.village_skim, 640,
            "skim + keeps must sum to the total Bulk price");
        assert_eq!(outcome.village_skim, 32);
        assert_eq!(outcome.escrow_credit, 608);
    }

    #[test]
    fn vendor_data_round_trips_lot_size_via_bincode() {
        // Save/load round-trip must preserve lot_size so a Bulk vendor
        // resumes at the same lot setting after a reload.
        let original = bulk_listing(7, 64, 32);
        let bytes = bincode::serialize(&original).expect("serialise");
        let back: VendorData = bincode::deserialize(&bytes).expect("deserialise");
        assert_eq!(back.lot_size, 32);
        assert_eq!(back.mode, Some(VendorMode::Bulk));
    }

    #[test]
    fn legacy_vendor_data_loads_lot_size_default_via_serde() {
        // A pre-Spec-40 VendorData (no lot_size field) lands with
        // lot_size = DEFAULT_LOT_SIZE via #[serde(default)].
        // NB: bincode v1 reads positionally; missing trailing bytes
        // still fail loud. The annotation is for non-bincode formats
        // (JSON, CBOR) + documents the intent.
        let original = sell_listing(100, 5);
        // Default Sell listings now carry lot_size = 1.
        assert_eq!(original.lot_size, 1);
    }

    #[test]
    fn bulk_lot_sizes_constant_is_8_16_32_64() {
        // Lock the picker's contract so a future change is a
        // test-visible event.
        assert_eq!(BULK_LOT_SIZES, [8, 16, 32, 64]);
    }
}
