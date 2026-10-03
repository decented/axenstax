//! Unified sats payout helper — Spec 19 follow-on, load-bearing for the
//! three economy foundation specs (Furnace / Vendor Block / Raid Defence).
//!
//! **The single point** of Charter-flag + server-policy + Reserve-drain +
//! server-tax gating across every sats-touching gameplay event. Every
//! payout codepath calls [`apply_sats_payout`]. When Sentinel D-003
//! reverses and real Lightning settlement comes online, flipping this
//! one helper turns on actual sats flow everywhere simultaneously — no
//! spec-by-spec migration.
//!
//! Design per `docs/vision/sat-flow-and-economy-loops.md` §5.

use serde::{Deserialize, Serialize};

/// Per-server sats-flow policy. Lives on `GameState` as a single instance
/// on alpha; per-server config when multi-server lands. Defaults to
/// **Bitcoin disabled** (audit 2026-09-27): sats are parent/operator
/// opt-in, never on by default for kids or the web taster. An operator
/// turns it on explicitly ([`ServerSatsPolicy::bitcoin_enabled_policy`]),
/// and a player still needs their own `charter_allows_sats`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServerSatsPolicy {
    /// True if the server allows any sats movement at all. False on a
    /// dedicated Bitcoin-disabled server (e.g. a kid-friendly classroom
    /// instance, an offline LAN session). All payouts return zero.
    pub bitcoin_enabled: bool,
    /// Server operator's cut, in basis points (1 bp = 0.01 %). 0 = no
    /// skim; 100 = 1 %; 1000 = 10 %. Goes to the operator's Lightning
    /// wallet when D-003 reverses.
    pub server_tax_bps: u32,
    /// Reserve-drain percentage, in basis points. Routes back into the
    /// Spec 16 Deepslate Reserve pool. 0 on alpha; the Reserve drain
    /// mechanism configures this when it goes live.
    pub reserve_drain_bps: u32,
}

impl ServerSatsPolicy {
    /// An operator's explicit Bitcoin-enabled posture (zero skim). Never
    /// reachable on the web build. No operator config surface calls it yet
    /// (tests do); it is the one sanctioned way to switch sats on.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn bitcoin_enabled_policy() -> Self {
        Self { bitcoin_enabled: SATS_UI_AVAILABLE, ..Self::default() }
    }
}

/// The hash-lottery drops themselves: produced by the Proof-of-Play
/// exposure roll rather than by deterministic work. Satori is the only one
/// today; add any new one here — everything crafted from it is derived
/// automatically (see [`CHANCE_DERIVED_KEYS`]).
pub const CHANCE_DROP_ROOTS: &[crate::item::Item] =
    &[crate::item::Item::Material(crate::item::MaterialId::Satori)];

/// Every item kind (by [`crate::item::Item::sort_key`]) that is a chance
/// drop or is crafted — transitively, through the real recipe registry
/// ([`crate::crafting_catalogue::all_cards`], which is pinned to the live
/// `match_recipe` by its consistency test) — from one. A fixpoint over the
/// recipe graph: an output joins the set as soon as any of its ingredients
/// is in it. So Satori → Satori Block, Satori tools, Satori armour, the
/// Satori Chest, and anything a future recipe makes from any of them.
pub static CHANCE_DERIVED_KEYS: std::sync::LazyLock<std::collections::HashSet<(u8, u32)>> =
    std::sync::LazyLock::new(|| {
        let mut set: std::collections::HashSet<(u8, u32)> =
            CHANCE_DROP_ROOTS.iter().map(|i| i.sort_key()).collect();
        loop {
            let mut grew = false;
            for card in crate::crafting_catalogue::all_cards() {
                if card.ingredients.iter().any(|ing| set.contains(&ing.item.sort_key()))
                    && set.insert(card.output.item.sort_key())
                {
                    grew = true;
                }
            }
            if !grew {
                return set;
            }
        }
    });

/// Is this item a **chance drop** — produced by the Proof-of-Play hash
/// lottery (the Satori exposure roll) — or crafted from one? Such items
/// must never carry a sats value anywhere (vendor sats modes, auction,
/// Bazaar, operator price table): a chance-based real-sats payout sits in
/// the UK Gambling Act s.6 perimeter, so sats flow only through the
/// deterministic work-meter (Spec 06 §2.3). Barter only. Plans are
/// per-instance and never recipe outputs, so they are never in the set.
pub fn is_chance_drop(item: &crate::item::Item) -> bool {
    if matches!(item, crate::item::Item::Plan(_)) {
        return false;
    }
    CHANCE_DERIVED_KEYS.contains(&item.sort_key())
}

/// Can this build show anything sats-related at all? False on the web
/// taster (wasm32): it is an anonymous local sandbox, so no sats price,
/// balance, tip, bid, bounty or fund UI ever renders there. Private: every
/// UI gate goes through [`sats_ui_visible`].
const SATS_UI_AVAILABLE: bool = !cfg!(target_arch = "wasm32");

/// THE sats-UI gate (audit 2026-09-27, review W4 S4). Any sats-related UI —
/// labels, prices, balances, the reserve gauge, tip buttons, the vendor's
/// sats modes, the sats-only dialogs — renders only when sats are on for
/// BOTH the server (`bitcoin_enabled`) AND this player
/// (`charter_allows_sats`): the same gate the payout code uses
/// ([`apply_sats_payout`]). Always false on the web build.
pub fn sats_ui_visible(bitcoin_enabled: bool, charter_allows_sats: bool) -> bool {
    SATS_UI_AVAILABLE && bitcoin_enabled && charter_allows_sats
}

/// Close every dialog that exists only to move sats (tip jar, auction,
/// bounty board, market hub, bazaar, villager commission) unless
/// [`sats_ui_visible`] for this player. Called every frame before any
/// dialog draws, so while sats are off these dialogs can't open. Returns
/// true if anything was open (the caller recaptures the cursor).
pub fn close_sats_only_uis(slot: &mut crate::player_slot::PlayerSlot, bitcoin_enabled: bool) -> bool {
    if sats_ui_visible(bitcoin_enabled, slot.charter_allows_sats) {
        return false;
    }
    let was_open = slot.open_tip_jar.is_some()
        || slot.open_auction.is_some()
        || slot.open_bounty_board.is_some()
        || slot.open_market_hub.is_some()
        || slot.open_bazaar.is_some()
        || slot.open_commission_villager.is_some();
    slot.open_tip_jar = None;
    slot.open_auction = None;
    slot.open_bounty_board = None;
    slot.open_market_hub = None;
    slot.open_bazaar = None;
    slot.open_commission_villager = None;
    was_open
}

/// Split a quest's sats reward into `(kid_gross, village_skim)` (Spec 22
/// Phase 13's 20 % village-treasury skim, `raid::split_quest_payout`) —
/// but ONLY when sats are on under the same gate the payout itself uses
/// (`bitcoin_enabled` && the player's effective Charter flag). With sats
/// off nothing is skimmed and no treasury is credited (review W4 N3).
pub fn quest_payout_split(
    scaled_sats: u64,
    policy: &ServerSatsPolicy,
    charter_allows_sats: bool,
) -> (u64, u64) {
    if policy.bitcoin_enabled && charter_allows_sats {
        crate::raid::split_quest_payout(scaled_sats)
    } else {
        (scaled_sats, 0)
    }
}

/// Origin of a sats payout. Used for logging + future analytics. The set
/// is closed; new payout types append.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayoutKind {
    QuestReward,
    VendorSale,
    RaidBounty,
    ProofOfPlay,
    /// Spec 25 Phase 7 — sats tip from a player to an architect via
    /// the Architect's Plaque attribution dialog.
    PlaqueTip,
    /// Spec 26 — sats paid to an NPC Builder for commissioning a Plan.
    /// Routed to the village treasury (v1 — per-NPC wallets are v2).
    BuilderCommission,
    /// Spec 33 — sats credited to a player on a Mob Bounty Board claim.
    /// Server-funded payout; honours Charter-flag suppression and the
    /// Nostrich Vow check at the call site.
    BountyClaim,
    /// Spec 34 — sats sent by one player to a Tip Jar's owner. Caller
    /// site honours Charter + Vow + bitcoin_enabled gates. The
    /// `credited` field of the payout result flows into the jar's
    /// `escrow_sats` (the owner withdraws separately).
    Tip,
    /// Spec 35 — the economy's first sat SINK. Semantically inverted
    /// from every other kind: this DEBITS the player (the repair tax
    /// at a Repair Bench) rather than crediting them. The `credited`
    /// field of the result represents "sats successfully removed".
    /// On Bitcoin-disabled / Charter-off the repair is free (the
    /// material cost still applies — that's the item sink).
    RepairTax,
    /// Spec 39 — sats paid to a player for selling a stack to the
    /// Server Bazaar at its trade-value floor. A payout TO the player
    /// from the server pool.
    BazaarSale,
    /// Future payout types (tournament purse, plot rent, etc.) extend here.
    Other,
}

impl PayoutKind {
    pub fn label(self) -> &'static str {
        match self {
            PayoutKind::QuestReward => "quest reward",
            PayoutKind::VendorSale => "vendor sale",
            PayoutKind::RaidBounty => "raid bounty",
            PayoutKind::ProofOfPlay => "proof-of-play",
            PayoutKind::PlaqueTip => "plaque tip",
            PayoutKind::BuilderCommission => "builder commission",
            PayoutKind::BountyClaim => "bounty claim",
            PayoutKind::Tip => "tip",
            PayoutKind::RepairTax => "repair tax",
            PayoutKind::BazaarSale => "bazaar sale",
            PayoutKind::Other => "payout",
        }
    }
}

/// Result of an [`apply_sats_payout`] call. `credited` is the amount the
/// player actually receives; the other fields are the audit trail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PayoutResult {
    /// Sats credited to the player. Always `<= requested_sats`.
    pub credited: u64,
    /// Sats taken by the server operator.
    pub server_skim: u64,
    /// Sats routed back into the Reserve drain pool.
    pub reserve_drain: u64,
    /// True iff the payout was blocked entirely (Charter-flag or
    /// server-policy disabled). When this is set, `credited` is 0 and
    /// the toast/log should suppress the sats line.
    pub suppressed: bool,
}

/// Apply a sats payout through the unified pipeline.
///
/// * `requested_sats` — the gross amount the gameplay event awarded.
/// * `kind` — payout origin, used only for logging.
/// * `policy` — the server's sats-flow policy.
/// * `charter_allows_sats` — per-player Charter flag (the guardian-set
///   permission per `project_bitcoin_parent_controlled.md`).
///
/// Returns the breakdown. The caller credits `result.credited` to the
/// player; the `server_skim` and `reserve_drain` fields are the audit
/// trail used by the (currently BRIDGEd) settlement layer.
pub fn apply_sats_payout(
    requested_sats: u64,
    kind: PayoutKind,
    policy: &ServerSatsPolicy,
    charter_allows_sats: bool,
) -> PayoutResult {
    // Charter-flag gate: a guardian-disabled kid sees no sats. Always
    // suppressed; gameplay continues (items + reputation still pay).
    if !charter_allows_sats {
        return PayoutResult {
            credited: 0,
            server_skim: 0,
            reserve_drain: 0,
            suppressed: true,
        };
    }
    // Server-policy gate: a Bitcoin-disabled server suppresses all sats
    // movement globally. Same posture as the Charter gate.
    if !policy.bitcoin_enabled {
        return PayoutResult {
            credited: 0,
            server_skim: 0,
            reserve_drain: 0,
            suppressed: true,
        };
    }
    if requested_sats == 0 {
        return PayoutResult::default();
    }
    // Compute skims in u128 to avoid overflow on giant payouts; clamp back
    // to u64 once we know the per-component split is sane.
    let total_bps = (policy.server_tax_bps as u64) + (policy.reserve_drain_bps as u64);
    let skim_total = requested_sats.saturating_mul(total_bps) / 10_000;
    let server_skim = (skim_total * policy.server_tax_bps as u64)
        .checked_div(total_bps)
        .unwrap_or(0);
    let reserve_drain = skim_total.saturating_sub(server_skim);
    let credited = requested_sats.saturating_sub(skim_total);
    log::debug!(
        "sats payout ({}): requested={requested_sats} credited={credited} \
         server_skim={server_skim} reserve_drain={reserve_drain}",
        kind.label(),
    );
    PayoutResult {
        credited,
        server_skim,
        reserve_drain,
        suppressed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> ServerSatsPolicy {
        ServerSatsPolicy::bitcoin_enabled_policy()
    }

    // --- Audit 2026-09-27: sats off by default; chance drops never priced ---

    #[test]
    fn sats_are_off_by_default_for_every_player_and_server() {
        assert!(!ServerSatsPolicy::default().bitcoin_enabled, "server default: Bitcoin off");
        let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        assert!(!slot.charter_allows_sats, "no guardian record is not consent");
        let r = apply_sats_payout(100, PayoutKind::QuestReward, &ServerSatsPolicy::default(), true);
        assert!(r.suppressed && r.credited == 0, "the default server pays nothing");
    }

    #[test]
    fn quest_village_skim_only_when_sats_are_on() {
        let off = ServerSatsPolicy::default();
        let on = ServerSatsPolicy { bitcoin_enabled: true, ..ServerSatsPolicy::default() };
        assert_eq!(quest_payout_split(100, &off, true), (100, 0), "server off: no skim");
        assert_eq!(quest_payout_split(100, &on, false), (100, 0), "player off: no skim");
        assert_eq!(quest_payout_split(100, &on, true), crate::raid::split_quest_payout(100));
        assert!(quest_payout_split(100, &on, true).1 > 0, "sats on: the village skims");
    }

    #[test]
    fn sats_ui_visible_needs_server_and_player_and_native() {
        let native = !cfg!(target_arch = "wasm32");
        assert_eq!(sats_ui_visible(true, true), native);
        assert!(!sats_ui_visible(false, true), "server off hides it");
        assert!(!sats_ui_visible(true, false), "player off hides it");
        assert!(!sats_ui_visible(false, false));
        // The defaults hide it: a new player on a default server.
        let slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        assert!(!sats_ui_visible(ServerSatsPolicy::default().bitcoin_enabled, slot.charter_allows_sats));
    }

    #[test]
    fn close_sats_only_uis_closes_every_sats_dialog_unless_sats_are_on() {
        let open_all = |slot: &mut crate::player_slot::PlayerSlot| {
            slot.open_tip_jar = Some((1, 2, 3));
            slot.open_auction = Some((1, 2, 3));
            slot.open_bounty_board = Some((1, 2, 3));
            slot.open_market_hub = Some((1, 2, 3));
            slot.open_bazaar = Some((1, 2, 3));
        };
        let mut slot = crate::player_slot::PlayerSlot::new(0, glam::Vec3::ZERO, 1.0);
        assert!(!close_sats_only_uis(&mut slot, false), "nothing open");
        open_all(&mut slot);
        assert!(close_sats_only_uis(&mut slot, true), "player off → closed even on a BTC server");
        assert!(slot.open_tip_jar.is_none() && slot.open_auction.is_none());
        assert!(slot.open_bounty_board.is_none() && slot.open_market_hub.is_none());
        assert!(slot.open_bazaar.is_none());
        // Sats on for both: the dialogs stay (native only).
        slot.charter_allows_sats = true;
        open_all(&mut slot);
        let closed = close_sats_only_uis(&mut slot, true);
        assert_eq!(closed, cfg!(target_arch = "wasm32"));
        assert_eq!(slot.open_tip_jar.is_some(), !cfg!(target_arch = "wasm32"));
    }

    /// Representative stacks for every Satori-derived item kind, computed
    /// INDEPENDENTLY of `CHANCE_DERIVED_KEYS`: walk the real recipe registry
    /// from the roots, feeding each card's `example_grid` to the live
    /// `match_recipe`, and collect the stacks it actually crafts.
    fn satori_derived_stacks_from_the_registry() -> Vec<crate::item::ItemStack> {
        use crate::item::ItemStack;
        let mut keys: std::collections::HashSet<(u8, u32)> =
            CHANCE_DROP_ROOTS.iter().map(|i| i.sort_key()).collect();
        let mut out: Vec<ItemStack> =
            CHANCE_DROP_ROOTS.iter().map(|i| ItemStack { item: i.clone(), count: 1 }).collect();
        loop {
            let mut grew = false;
            for card in crate::crafting_catalogue::all_cards() {
                let consumes = card.example_grid.iter().flatten().any(|slot| {
                    let item = match *slot {
                        crate::crafting::CraftSlot::Block(b) => crate::item::Item::Block(b),
                        crate::crafting::CraftSlot::Material(m) => crate::item::Item::Material(m),
                        crate::crafting::CraftSlot::Empty => return false,
                    };
                    keys.contains(&item.sort_key())
                });
                if !consumes {
                    continue;
                }
                let made = crate::crafting::match_recipe(&card.example_grid)
                    .expect("a catalogue card's grid always crafts");
                if keys.insert(made.item.sort_key()) {
                    out.push(made);
                    grew = true;
                }
            }
            if !grew {
                return out;
            }
        }
    }

    /// THE LINT: Satori and everything derived from it through the recipe
    /// graph can never be priced or paid in sats — Bazaar quote, operator
    /// price table, every vendor sats mode (listing, preview, buy), auction
    /// bid. Barter only. Deterministic work-meter only (Spec 06 §2.3).
    #[test]
    fn no_satori_derived_item_ever_feeds_a_sats_payout() {
        use crate::item::{Item, MaterialId};
        use crate::vendor::{preview_refusal, slot_accepts_for_mode, BuyRefusal, VendorData, VendorMode};
        let derived = satori_derived_stacks_from_the_registry();
        // The graph really is walked: the block, a tool, armour, the chest.
        let has = |pred: &dyn Fn(&Item) -> bool| derived.iter().any(|s| pred(&s.item));
        assert!(has(&|i| matches!(i, Item::Block(b) if *b == crate::block::SATORI_BLOCK)));
        assert!(has(&|i| matches!(i, Item::Block(b) if *b == crate::block::SATORI_CHEST)));
        assert!(has(&|i| matches!(i, Item::Tool(_))), "Satori tools are derived");
        assert!(has(&|i| matches!(i, Item::Armour(_))), "Satori armour is derived");
        let policy = ServerSatsPolicy::bitcoin_enabled_policy();
        let sats_modes = [
            VendorMode::Sell,
            VendorMode::Bulk,
            VendorMode::SellPlanMaster,
            VendorMode::SellPlanLicence,
            VendorMode::Buy,
        ];
        for d in &derived {
            assert!(is_chance_drop(&d.item), "{:?} is Satori-derived", d.item);
            assert_eq!(crate::bazaar::sell_quote(d), None, "Bazaar never pays sats for {:?}", d.item);
            let mut cfg = crate::server_economy::ServerEconomyConfig::new();
            cfg.sats_per_unit = 10;
            cfg.override_material(MaterialId::Satori, Some(1));
            assert_eq!(cfg.sats_for(&d.item), None, "no operator price for {:?}", d.item);
            for mode in sats_modes {
                assert!(mode.requires_sats(), "{mode:?} is a sats mode");
                assert!(!slot_accepts_for_mode(mode, d), "{mode:?} won't list {:?}", d.item);
                let v = VendorData { mode: Some(mode), slot: Some(d.clone()), stock: 3, lot_size: 1, price_sats: 1, ..Default::default() };
                assert_eq!(preview_refusal(&v, mode, true, policy.bitcoin_enabled), Some(BuyRefusal::ChanceDropNotForSats));
                let inv = crate::inventory::Inventory::new();
                assert!(crate::vendor::try_buy(&v, mode, &policy, true, &inv).is_err());
            }
            assert!(slot_accepts_for_mode(VendorMode::Barter, d), "Barter is fine for {:?}", d.item);
            let mut auc = crate::auction::AuctionData::new(crate::auction::AuctionOwner::LocalPlayer(0));
            auc.lot = Some(d.clone());
            auc.started = true;
            auc.deadline_tick = 1_000;
            assert_eq!(crate::auction::try_bid(&mut auc, 1, 500, 10), crate::auction::BidOutcome::LotNotSellableForSats);
        }
        // Every mode that takes sats is covered above.
        for mode in [VendorMode::Sell, VendorMode::Bulk, VendorMode::SellPlanMaster, VendorMode::SellPlanLicence, VendorMode::Buy, VendorMode::Barter] {
            assert_eq!(mode.requires_sats(), sats_modes.contains(&mode), "{mode:?} sats coverage");
        }
        // And ordinary (non-Satori) goods are untouched.
        assert!(!is_chance_drop(&Item::Material(MaterialId::IronIngot)));
        assert!(!is_chance_drop(&Item::Block(crate::block::STONE)));
    }

    #[test]
    fn default_policy_credits_full_amount() {
        let r = apply_sats_payout(100, PayoutKind::QuestReward, &default_policy(), true);
        assert_eq!(r.credited, 100);
        assert_eq!(r.server_skim, 0);
        assert_eq!(r.reserve_drain, 0);
        assert!(!r.suppressed);
    }

    #[test]
    fn charter_disabled_suppresses_payout() {
        let r = apply_sats_payout(100, PayoutKind::QuestReward, &default_policy(), false);
        assert_eq!(r.credited, 0);
        assert!(r.suppressed);
    }

    #[test]
    fn bitcoin_disabled_server_suppresses_payout() {
        let policy = ServerSatsPolicy { bitcoin_enabled: false, ..default_policy() };
        let r = apply_sats_payout(100, PayoutKind::QuestReward, &policy, true);
        assert_eq!(r.credited, 0);
        assert!(r.suppressed);
    }

    #[test]
    fn server_tax_only_takes_correct_share() {
        // 10% tax (1000 bps), no drain
        let policy = ServerSatsPolicy { server_tax_bps: 1000, ..default_policy() };
        let r = apply_sats_payout(100, PayoutKind::VendorSale, &policy, true);
        assert_eq!(r.server_skim, 10);
        assert_eq!(r.reserve_drain, 0);
        assert_eq!(r.credited, 90);
    }

    #[test]
    fn reserve_drain_only_takes_correct_share() {
        // 5% drain (500 bps), no tax
        let policy = ServerSatsPolicy { reserve_drain_bps: 500, ..default_policy() };
        let r = apply_sats_payout(100, PayoutKind::RaidBounty, &policy, true);
        assert_eq!(r.server_skim, 0);
        assert_eq!(r.reserve_drain, 5);
        assert_eq!(r.credited, 95);
    }

    #[test]
    fn tax_and_drain_combine_additively() {
        // 10% tax + 5% drain = 15% total
        let policy = ServerSatsPolicy {
            server_tax_bps: 1000,
            reserve_drain_bps: 500,
            ..default_policy()
        };
        let r = apply_sats_payout(100, PayoutKind::VendorSale, &policy, true);
        assert_eq!(r.server_skim + r.reserve_drain, 15);
        assert_eq!(r.credited, 85);
        // Split honours the bps ratio: 1000:500 = 2:1
        assert_eq!(r.server_skim, 10);
        assert_eq!(r.reserve_drain, 5);
    }

    #[test]
    fn zero_requested_returns_zero() {
        let r = apply_sats_payout(0, PayoutKind::ProofOfPlay, &default_policy(), true);
        assert_eq!(r.credited, 0);
        assert_eq!(r.server_skim, 0);
        assert_eq!(r.reserve_drain, 0);
        assert!(!r.suppressed, "zero amount is a non-event, not a suppression");
    }

    #[test]
    fn giant_payout_does_not_overflow() {
        // 1 billion sats with 10% tax — no panic on u64 multiplication.
        let policy = ServerSatsPolicy { server_tax_bps: 1000, ..default_policy() };
        let r = apply_sats_payout(1_000_000_000, PayoutKind::Other, &policy, true);
        assert_eq!(r.server_skim, 100_000_000);
        assert_eq!(r.credited, 900_000_000);
    }

    #[test]
    fn full_skim_credits_nothing_to_player() {
        // 100% tax (10000 bps) — pathological but well-defined.
        let policy = ServerSatsPolicy {
            server_tax_bps: 10_000,
            ..default_policy()
        };
        let r = apply_sats_payout(100, PayoutKind::QuestReward, &policy, true);
        assert_eq!(r.server_skim, 100);
        assert_eq!(r.credited, 0);
        assert!(!r.suppressed, "100% tax is not the same as suppression");
    }
}
