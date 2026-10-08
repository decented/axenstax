//! C3c-3a (2026-10-08, protocol v83) — a joiner's Plans on the
//! server's copy of its inventory, tracked by marker.
//!
//! A Plan's body (it can reach about 160 KB) never crosses the wire. The
//! joined client keeps its Plans whole; the server's copy of its window holds
//! each as a **marker placeholder** (`plan::PlanData::marker_placeholder`):
//! the Plan's `plan::marker` (SHA-256 of the whole `PlanData`) and its develop
//! state, body-less. A placeholder moves under the window clicks exactly as
//! the client's Plan does (a Plan never stacks, and digests content-free), and
//! an owed take finds it by marker (`joiner_actions::same_item`).
//!
//! - **A mint is reported** (`protocol::ItemAction::PlanMinted`): the joined
//!   client mints in its own window as single-player does — a capture's
//!   commit ([`MintSource::CaptureCommit`]) or an art capture
//!   ([`MintSource::CaptureArt`], which also spends one Blueprint Paper) —
//!   then tells the server what it minted and what it spent, routed and
//!   ordered as a Q-drop is (`GameState::send_request`, never answered). The
//!   server mirrors it by the client's steps in the client's order
//!   ([`mirror_mint`]). A mint that wouldn't fit the joined client's bag is
//!   refused before anything happens ([`plan_fits`]); single-player keeps its
//!   own full-bag rules.
//! - **A hang is a use edit** (`use_edits::UseKind::HangPrint`): its tag's
//!   `used` is the Plan by marker, and the server takes the placeholder.
//!
//! **Log-only, like all of C3a–C3c** (`PossessionTally::plan_minted` /
//! `plan_mismatch`): a reported mint is believed until C3d, which checks a
//! `CaptureArt`'s paper and refuses a mint whose `spent` the copy can't cover.
//! A placeholder is never spilled into the world: one that doesn't fit the
//! copy is counted and dropped (it is no real item).

use crate::inventory::Inventory;
use crate::item::{Item, ItemStack};

/// Where a reported Plan came from (`ItemAction::PlanMinted::source`).
///
/// **On the wire as a `u8`, APPEND-ONLY**: never renumber or reuse a value
/// ([`Self::to_wire`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MintSource {
    /// The Drafting Stamp's capture, confirmed in the dialog
    /// (`plan::commit_capture`): its paper was spent when it was laid, so
    /// the mint spends nothing.
    CaptureCommit,
    /// Blueprint Paper on a wall (`plan::capture_art`): the mint spends the
    /// one paper in hand.
    CaptureArt,
}

impl MintSource {
    /// Every source, in wire order: `ALL[k].to_wire() == k`.
    pub const ALL: [MintSource; 2] = [MintSource::CaptureCommit, MintSource::CaptureArt];

    /// The wire byte. APPEND-ONLY: pinned by
    /// `tests::mint_source_wire_bytes_are_pinned`.
    pub const fn to_wire(self) -> u8 {
        match self {
            MintSource::CaptureCommit => 0,
            MintSource::CaptureArt => 1,
        }
    }

    /// The source a wire byte names, if any (an unknown byte is a newer or
    /// modified peer's).
    pub fn from_wire(b: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.to_wire() == b)
    }

    /// For the log.
    pub const fn label(self) -> &'static str {
        match self {
            MintSource::CaptureCommit => "capture",
            MintSource::CaptureArt => "art capture",
        }
    }
}

/// What a joined client toasts when a mint wouldn't fit its bag.
pub const MAKE_ROOM_TOAST: &str = "Make room for the Plan first";

/// Would a minted Plan fit `inv` by the client's `Inventory::add_item` (a Plan
/// never stacks, so: is any of the 36 slots empty)? A joined client asks
/// before it mints, so a mint never needs the full-bag fallbacks
/// single-player keeps (a commit's lost Plan, an art capture's plain place).
pub fn plan_fits(inv: &Inventory) -> bool {
    inv.slots_iter().any(|s| s.is_none())
}

/// What [`mirror_mint`] did to the copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mirrored {
    /// The placeholder landed (the copy had room, and the report named a
    /// Plan).
    pub placed: bool,
    /// What the mint spent was taken (or it spent nothing).
    pub paid: bool,
}

impl Mirrored {
    /// Mirrored with no shortfall.
    pub fn clean(self) -> bool {
        self.placed && self.paid
    }
}

/// Mirror a joiner's reported mint on `inv`, the server's copy of its
/// inventory, by the client's steps in the client's order: first the Plan
/// lands by `Inventory::add_item` (`plan`, the marker placeholder decoded
/// from the report: the first empty slot, as the client's own Plan did),
/// then one of `spent` is taken by the owed search
/// (`joiner_actions::take_owed`, from hotbar slot `slot` first — the slot the
/// client's `take_one_from_hotbar` took it from). The order matters: a
/// capture that spends the last paper of a slot frees that slot only after
/// the Plan has landed elsewhere, on both sides. A placeholder with no room
/// is dropped, never spilled.
pub fn mirror_mint(inv: &mut Inventory, slot: usize, plan: Option<Item>, spent: Option<&Item>) -> Mirrored {
    let placed = match plan {
        Some(item @ Item::Plan(_)) => inv.add_item(ItemStack { item, count: 1 }).is_none(),
        _ => false,
    };
    let paid = spent.is_none_or(|item| crate::joiner_actions::take_owed(inv, slot, item, 1) == 1);
    Mirrored { placed, paid }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{marker, DevelopState, PlanData};

    fn paper(n: u8) -> ItemStack {
        ItemStack::new_block(crate::block::BLUEPRINT_PAPER, n)
    }

    fn latent() -> PlanData {
        PlanData { develop_state: DevelopState::Latent { exposure_ticks: 0 }, ..PlanData::debug_3x3_stone() }
    }

    /// The wire bytes are APPEND-ONLY.
    #[test]
    fn mint_source_wire_bytes_are_pinned() {
        let bytes: Vec<u8> = MintSource::ALL.iter().map(|s| s.to_wire()).collect();
        assert_eq!(bytes, vec![0, 1]);
        for s in MintSource::ALL {
            assert_eq!(MintSource::from_wire(s.to_wire()), Some(s));
        }
        assert_eq!(MintSource::from_wire(2), None);
    }

    /// Lockstep: the client's art capture (add the Plan, THEN take its last
    /// paper from the hand) and the server's mirror land the Plan in the same
    /// slot — the first empty one at the time, not the slot the paper frees.
    #[test]
    fn a_mint_lands_where_the_clients_plan_did_before_the_paper_is_taken() {
        let mut client = Inventory::new();
        client.set_slot(2, Some(paper(1)));
        client.set_slot(0, Some(ItemStack::new_block(crate::block::STONE, 3)));
        client.set_slot(1, Some(ItemStack::new_block(crate::block::DIRT, 3)));
        client.set_slot(3, Some(ItemStack::new_block(crate::block::DIRT, 3)));
        let mut copy = client.clone();
        // The client (`game_loop`'s art-capture arm).
        let plan = latent();
        assert!(client.add_item(ItemStack { item: Item::Plan(plan.clone()), count: 1 }).is_none());
        assert!(client.take_one_from_hotbar(2).is_some());
        assert!(matches!(client.slot(4).map(|s| &s.item), Some(Item::Plan(_))), "the first empty slot");
        assert!(client.slot(2).is_none(), "the paper's slot is free");
        // The server, from the report.
        let wire = crate::inventory::item_to_wire_full(&Item::Plan(plan));
        let placeholder = crate::inventory::plan_from_wire(&wire);
        let m = mirror_mint(&mut copy, 2, placeholder, Some(&paper(1).item));
        assert!(m.clean());
        for k in 0..36 {
            assert_eq!(copy.slot(k).is_some(), client.slot(k).is_some(), "slot {k}");
        }
        let held = |inv: &Inventory, k: usize| inv.slot(k).map(|s| s.item.clone());
        assert!(matches!(held(&copy, 4), Some(Item::Plan(p)) if p.same_plan(&latent())), "the placeholder, by marker");
        assert_eq!(
            crate::window::digest_parts(&copy, &[None; 4], &None, &Default::default(), crate::window::Station::Player),
            crate::window::digest_parts(&client, &[None; 4], &None, &Default::default(), crate::window::Station::Player),
            "and it digests like the real one"
        );
    }

    /// A commit spends nothing; a shortfall (no paper, no room, no Plan) is
    /// counted, and a placeholder with no room is dropped, not spilled.
    #[test]
    fn a_mint_with_a_shortfall_is_counted_not_refused() {
        let plan = Item::Plan(PlanData::marker_placeholder(marker(&latent()), false));
        let mut inv = Inventory::new();
        assert!(mirror_mint(&mut inv, 0, Some(plan.clone()), None).clean(), "a commit spends nothing");
        let m = mirror_mint(&mut inv, 0, Some(plan.clone()), Some(&paper(1).item));
        assert_eq!(m, Mirrored { placed: true, paid: false }, "no paper in the copy");
        let mut full = Inventory::new();
        for k in 0..36 {
            full.set_slot(k, Some(ItemStack::new_block(crate::block::STONE, 64)));
        }
        assert!(!plan_fits(&full));
        let m = mirror_mint(&mut full, 0, Some(plan), None);
        assert!(!m.placed && m.paid, "no room: dropped");
        assert!(full.slots_iter().flatten().all(|s| s.item == Item::Block(crate::block::STONE)));
        let m = mirror_mint(&mut inv, 0, Some(Item::Block(crate::block::STONE)), None);
        assert!(!m.placed, "a report naming no Plan lands nothing");
        assert!(plan_fits(&Inventory::new()));
    }
}
