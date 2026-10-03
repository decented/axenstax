//! Auctions v1 — time-limited price discovery (Spec 38).
//!
//! An Auction Block holds a timed `AuctionData`: a lot (ItemStack),
//! reserve price, absolute `deadline_tick` (in `tick_counter` units —
//! NEVER `world_time`, which wraps), current high bid + bidder. A
//! per-tick driver settles at expiry (winner gets the lot, owner gets
//! escrow; below-reserve returns the lot). Anti-snipe extends the
//! deadline if a bid lands in the final window.
//!
//! Escrow is notional on alpha (no real wallet — `apply_sats_payout`
//! is accounting-only). `held_sats` records the notional hold for the
//! future Lightning HOLD-invoice path. Owner model converges with
//! Vendor/Tip Jar/Plot/Market per
//! [[project_economy_block_owner_convergence]].
//!
//! Spec: `docs/foundations/2026-05-23-auctions.md`.

use serde::{Deserialize, Serialize};

use crate::item::ItemStack;
use crate::world::World;

/// Minimum a new bid must exceed the current high by.
pub const MIN_BID_INCREMENT: u64 = 1;
/// A bid landing within this many ticks of the deadline extends it.
pub const ANTI_SNIPE_WINDOW_TICKS: u64 = 600; // 30s @ 20 TPS
/// How far a sniped deadline is pushed out.
pub const ANTI_SNIPE_EXTEND_TICKS: u64 = 600;
/// Default auction duration when the owner doesn't pick one.
pub const DEFAULT_DURATION_TICKS: u64 = 12_000; // 10 min @ 20 TPS

/// Owner of an auction. LocalPlayer(pidx) on alpha; Npub for the
/// Spec-1-Phase-4 convergence.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuctionOwner {
    LocalPlayer(usize),
    Npub(String),
}

/// Timed auction state. Lives in `BlockEntityData::Auction`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuctionData {
    pub owner: AuctionOwner,
    /// The lot being auctioned. None until the owner configures it;
    /// taken (set to None) at settlement once delivered.
    pub lot: Option<ItemStack>,
    pub reserve_sats: u64,
    pub high_bid: u64,
    /// BRIDGE: pidx, not a stable identity — a saved auction reloaded
    /// with a different player count points `high_bidder` at the wrong
    /// slot. The lot still delivers physically (spawn_item at the
    /// block), but any pidx-keyed credit/notify would mis-target.
    /// Converge to a Signet npub with the other economy owner models
    /// when Spec 1 Phase 4 lands ([[project_economy_block_owner_convergence]]).
    pub high_bidder: Option<usize>,
    /// Notional sats hold for the future LN path (see module docs).
    pub held_sats: u64,
    /// Absolute `tick_counter` deadline. 0 = not started.
    pub deadline_tick: u64,
    pub started: bool,
    pub settled: bool,
    /// Owner proceeds accrued at settlement; drained via Withdraw.
    pub escrow_sats: u64,
}

impl AuctionData {
    /// A fresh, un-configured auction owned by `owner`.
    pub fn new(owner: AuctionOwner) -> Self {
        AuctionData {
            owner,
            lot: None,
            reserve_sats: 0,
            high_bid: 0,
            high_bidder: None,
            held_sats: 0,
            deadline_tick: 0,
            started: false,
            settled: false,
            escrow_sats: 0,
        }
    }

    /// True once the owner has set a lot + started the countdown.
    pub fn is_active(&self) -> bool {
        self.started && !self.settled && self.lot.is_some()
    }
}

/// Outcome of a bid attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BidOutcome {
    /// Bid accepted; `new_deadline` reflects any anti-snipe extension.
    Accepted { new_deadline: u64 },
    /// Below `high_bid + MIN_BID_INCREMENT` (or below reserve on the
    /// first bid).
    TooLow,
    /// Auction isn't running (not started / settled / no lot).
    NotActive,
    /// The owner can't bid on their own auction.
    OwnerCannotBid,
    /// The lot is a chance drop (Satori), which never sells for sats.
    LotNotSellableForSats,
}

/// Attempt a bid. Pure state mutation. `now_tick` is the monotonic
/// `tick_counter`. On accept, records the new high bid/bidder, the
/// notional hold, and applies anti-snipe to the deadline.
pub fn try_bid(
    data: &mut AuctionData,
    bidder_pidx: usize,
    amount: u64,
    now_tick: u64,
) -> BidOutcome {
    if !data.is_active() || now_tick >= data.deadline_tick {
        return BidOutcome::NotActive;
    }
    if crate::auction::is_local_owner(&data.owner, bidder_pidx) {
        return BidOutcome::OwnerCannotBid;
    }
    if data.lot.as_ref().is_some_and(|l| crate::economy::is_chance_drop(&l.item)) {
        return BidOutcome::LotNotSellableForSats;
    }
    // First bid must meet the reserve floor implicitly via the
    // increment over 0; but we also require >= reserve so a sub-
    // reserve bid can't become "high" and then win nothing.
    let floor = if data.high_bidder.is_none() {
        data.reserve_sats.max(data.high_bid + MIN_BID_INCREMENT)
    } else {
        data.high_bid + MIN_BID_INCREMENT
    };
    if amount < floor {
        return BidOutcome::TooLow;
    }
    data.high_bid = amount;
    data.high_bidder = Some(bidder_pidx);
    // BRIDGE: a single `held_sats` field can only track the current high
    // bidder's notional hold; the previous bidder's hold is overwritten
    // here. Harmless on alpha (sats are notional), but the real LN path
    // must release/cancel the outbid bidder's HOLD-invoice before this
    // overwrite — replace when LN escrow lands.
    data.held_sats = amount;
    // Anti-snipe: a bid in the final window pushes the deadline out.
    // Use `max` so the extension can only ever move the deadline LATER —
    // never shrink an already-further-out deadline (e.g. if the EXTEND
    // window is ever tuned shorter than the trigger window).
    if data.deadline_tick.saturating_sub(now_tick) <= ANTI_SNIPE_WINDOW_TICKS {
        data.deadline_tick = data.deadline_tick.max(now_tick + ANTI_SNIPE_EXTEND_TICKS);
    }
    BidOutcome::Accepted { new_deadline: data.deadline_tick }
}

/// Should this auction settle now? (Active + deadline passed.)
pub fn should_settle(data: &AuctionData, now_tick: u64) -> bool {
    data.started && !data.settled && data.lot.is_some() && now_tick >= data.deadline_tick
}

/// Settlement decision. Pure — the caller performs the item/sats
/// side-effects (deliver lot, credit escrow). Marks the auction
/// settled + clears the lot (the caller takes ownership of it).
#[derive(Clone, Debug)]
pub enum SettleResult {
    /// Reserve met + a bidder exists. Lot goes to `winner_pidx`;
    /// `proceeds` credit the owner. The live caller (game_loop.rs's block
    /// resolution) only destructures `lot` — proceeds are already credited
    /// to escrow_sats inside `settle` before this result is produced, and
    /// delivery is "at the block" regardless of which player index won,
    /// so both fields are informational there. Tested directly.
    SoldTo {
        #[cfg_attr(not(test), allow(dead_code))]
        winner_pidx: usize,
        #[cfg_attr(not(test), allow(dead_code))]
        proceeds: u64,
        lot: ItemStack,
    },
    /// No bids / below reserve. Lot returns to the owner.
    Unsold { lot: ItemStack },
    /// Nothing to settle (no lot — shouldn't happen if should_settle
    /// gated correctly).
    Nothing,
}

/// Settle the auction. Takes the lot out of `data` and returns the
/// outcome. Idempotent-ish: sets `settled = true`.
pub fn settle(data: &mut AuctionData) -> SettleResult {
    data.settled = true;
    let Some(lot) = data.lot.take() else {
        return SettleResult::Nothing;
    };
    match data.high_bidder {
        Some(winner) if data.high_bid >= data.reserve_sats && data.high_bid > 0 => {
            let proceeds = data.high_bid;
            data.escrow_sats = data.escrow_sats.saturating_add(proceeds);
            SettleResult::SoldTo { winner_pidx: winner, proceeds, lot }
        }
        _ => SettleResult::Unsold { lot },
    }
}

/// Per-tick settlement sweep. Snapshot-keys-then-mutate (the Furnace
/// pattern — avoids the iterate-while-mutate borrow conflict). For
/// each auction past its deadline, settles it: the lot is delivered
/// as an `ItemEntity` at the block (so an offline / inventory-full
/// winner still gets it) and the owner's proceeds accrue to the
/// auction's `escrow_sats` (drained via the owner dialog). Below-
/// reserve returns the lot to the block too (owner picks it up).
///
/// `now_tick` MUST be the monotonic `tick_counter` — never
/// `world_time` (which wraps at 24 000).
///
/// Returns the count of auctions settled this tick (for tests / logs).
pub fn tick_auctions(
    world: &mut World,
    ecs: &mut hecs::World,
    now_tick: u64,
) -> u32 {
    // Snapshot positions first so the iter borrow ends before we
    // mutate via auction_at_mut.
    let positions: Vec<(i32, i32, i32)> = world
        .iter_auctions()
        .filter(|(_, a)| should_settle(a, now_tick))
        .map(|(pos, _)| pos)
        .collect();
    let mut settled = 0u32;
    for pos in positions {
        // Block may have been mined between snapshot + mutate.
        let result = match world.auction_at_mut(pos) {
            Some(data) => settle(data),
            None => continue,
        };
        let drop_pos = glam::Vec3::new(
            pos.0 as f32 + 0.5,
            pos.1 as f32 + 1.0,
            pos.2 as f32 + 0.5,
        );
        let seed = pos.0.unsigned_abs() ^ pos.1.unsigned_abs() ^ pos.2.unsigned_abs();
        match result {
            SettleResult::SoldTo { lot, .. } => {
                // Deliver the lot at the block — works whether or not
                // the winner is online / has inventory space. Proceeds
                // already credited to the auction's escrow_sats in
                // `settle`. (Real LN settlement is a BRIDGE.)
                crate::entity::spawn_item(ecs, drop_pos, lot, seed);
                settled += 1;
            }
            SettleResult::Unsold { lot } => {
                // Return the lot to the world at the block for the owner.
                crate::entity::spawn_item(ecs, drop_pos, lot, seed);
                settled += 1;
            }
            SettleResult::Nothing => {}
        }
    }
    settled
}

/// True iff `owner` is the given local player.
pub fn is_local_owner(owner: &AuctionOwner, pidx: usize) -> bool {
    matches!(owner, AuctionOwner::LocalPlayer(p) if *p == pidx)
}

/// Ticks remaining until the deadline (0 if past / not started).
pub fn time_left_ticks(data: &AuctionData, now_tick: u64) -> u64 {
    if !data.started {
        return 0;
    }
    data.deadline_tick.saturating_sub(now_tick)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, MaterialId};

    fn lot() -> ItemStack {
        ItemStack { item: Item::Material(MaterialId::Diamond), count: 1 }
    }

    fn started_auction(owner: usize, reserve: u64, deadline: u64) -> AuctionData {
        let mut a = AuctionData::new(AuctionOwner::LocalPlayer(owner));
        a.lot = Some(lot());
        a.reserve_sats = reserve;
        a.started = true;
        a.deadline_tick = deadline;
        a
    }

    #[test]
    fn try_bid_accepts_first_bid_at_reserve() {
        let mut a = started_auction(0, 50, 10_000);
        let out = try_bid(&mut a, 1, 50, 100);
        assert!(matches!(out, BidOutcome::Accepted { .. }));
        assert_eq!(a.high_bid, 50);
        assert_eq!(a.high_bidder, Some(1));
    }

    #[test]
    fn try_bid_rejects_below_reserve_first_bid() {
        let mut a = started_auction(0, 50, 10_000);
        let out = try_bid(&mut a, 1, 40, 100);
        assert_eq!(out, BidOutcome::TooLow);
        assert_eq!(a.high_bidder, None);
    }

    #[test]
    fn try_bid_rejects_below_increment() {
        let mut a = started_auction(0, 0, 10_000);
        let _ = try_bid(&mut a, 1, 100, 100);
        let out = try_bid(&mut a, 2, 100, 200); // must exceed by MIN_BID_INCREMENT
        assert_eq!(out, BidOutcome::TooLow);
        assert_eq!(a.high_bidder, Some(1));
    }

    #[test]
    fn try_bid_owner_cannot_bid() {
        let mut a = started_auction(0, 0, 10_000);
        let out = try_bid(&mut a, 0, 100, 100);
        assert_eq!(out, BidOutcome::OwnerCannotBid);
    }

    #[test]
    fn try_bid_not_active_after_deadline() {
        let mut a = started_auction(0, 0, 10_000);
        let out = try_bid(&mut a, 1, 100, 10_001);
        assert_eq!(out, BidOutcome::NotActive);
    }

    #[test]
    fn try_bid_not_active_when_unstarted() {
        let mut a = AuctionData::new(AuctionOwner::LocalPlayer(0));
        let out = try_bid(&mut a, 1, 100, 100);
        assert_eq!(out, BidOutcome::NotActive);
    }

    #[test]
    fn anti_snipe_extends_deadline_within_window() {
        // Deadline at 1000, bid at 600 → 400 left, within 600 window.
        let mut a = started_auction(0, 0, 1_000);
        let out = try_bid(&mut a, 1, 100, 600);
        match out {
            BidOutcome::Accepted { new_deadline } => {
                assert_eq!(new_deadline, 600 + ANTI_SNIPE_EXTEND_TICKS);
                assert_eq!(a.deadline_tick, 600 + ANTI_SNIPE_EXTEND_TICKS);
            }
            other => panic!("expected Accepted, got {other:?}"),
        }
    }

    #[test]
    fn anti_snipe_no_extend_outside_window() {
        // Deadline at 10_000, bid at 100 → 9900 left, outside window.
        let mut a = started_auction(0, 0, 10_000);
        let out = try_bid(&mut a, 1, 100, 100);
        assert_eq!(out, BidOutcome::Accepted { new_deadline: 10_000 });
        assert_eq!(a.deadline_tick, 10_000, "no extension outside the window");
    }

    #[test]
    fn anti_snipe_never_shrinks_deadline_across_successive_bids() {
        // Invariant: a bid can only ever push the deadline LATER. Two
        // successive in-window bids must leave a monotonically non-
        // decreasing deadline — guards the `.max()` in the anti-snipe
        // branch against a future EXTEND < WINDOW tuning that would
        // otherwise let the second bid drag the deadline backwards.
        let mut a = started_auction(0, 0, 1_000);
        let d0 = a.deadline_tick;
        // First in-window bid at t=600 → extends to 600 + EXTEND.
        let _ = try_bid(&mut a, 1, 100, 600);
        let d1 = a.deadline_tick;
        assert!(d1 >= d0, "first bid must not shrink the deadline");
        // Second in-window bid a few ticks later → must not move it earlier.
        let _ = try_bid(&mut a, 2, 200, 610);
        let d2 = a.deadline_tick;
        assert!(d2 >= d1, "second bid must not shrink the deadline");
    }

    #[test]
    fn should_settle_only_after_deadline() {
        let a = started_auction(0, 0, 1_000);
        assert!(!should_settle(&a, 999));
        assert!(should_settle(&a, 1_000));
        assert!(should_settle(&a, 1_001));
    }

    #[test]
    fn should_settle_false_when_unstarted_or_settled() {
        let mut a = AuctionData::new(AuctionOwner::LocalPlayer(0));
        a.lot = Some(lot());
        assert!(!should_settle(&a, 100_000), "unstarted never settles");
        a.started = true;
        a.deadline_tick = 1_000;
        a.settled = true;
        assert!(!should_settle(&a, 100_000), "already-settled never re-settles");
    }

    #[test]
    fn settle_sells_when_reserve_met() {
        let mut a = started_auction(0, 50, 1_000);
        let _ = try_bid(&mut a, 1, 80, 100);
        let r = settle(&mut a);
        match r {
            SettleResult::SoldTo { winner_pidx, proceeds, .. } => {
                assert_eq!(winner_pidx, 1);
                assert_eq!(proceeds, 80);
            }
            other => panic!("expected SoldTo, got {other:?}"),
        }
        assert!(a.settled);
        assert_eq!(a.escrow_sats, 80);
        assert!(a.lot.is_none(), "lot taken at settlement");
    }

    #[test]
    fn settle_unsold_when_no_bids() {
        let mut a = started_auction(0, 50, 1_000);
        let r = settle(&mut a);
        assert!(matches!(r, SettleResult::Unsold { .. }));
        assert!(a.settled);
        assert_eq!(a.escrow_sats, 0);
    }

    #[test]
    fn time_left_counts_down() {
        let a = started_auction(0, 0, 1_000);
        assert_eq!(time_left_ticks(&a, 400), 600);
        assert_eq!(time_left_ticks(&a, 1_000), 0);
        assert_eq!(time_left_ticks(&a, 2_000), 0);
    }
}
