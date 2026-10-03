//! Who may join an online-hosted world — the whole rule, in one pure function.
//!
//! Named `online_admission` because `crate::admission` already exists and means
//! something else entirely (capacity: is the server full). This module is about
//! *identity*: is this person somebody the host actually knows, or somebody
//! holding an invite the host just minted.
//!
//! The rule (spec §3.3):
//!
//! ```text
//! admit(offer, contacts, active_bearer, now) =
//!     if tier_of(persona) is Kin or Kith        -> Accept(AlreadyContact)
//!     else if bearer matches and is unexpired   -> Accept(ByInvite)
//!     else                                      -> Refuse(...)
//! ```
//!
//! **Ken does not admit.** Ken is one-way recognition — I pinned you, you did
//! not pin me — and `comms.rs` already treats it as hear-only. Pinning somebody
//! must not hand them a key to your house either.
//!
//! Refusals are mostly **never sent**. A stranger gets silence, because an
//! answer is itself information ("yes, somebody is hosting here"). Only
//! `ProtocolMismatch` and `Full` go on the wire, and only to somebody who was
//! already entitled to reach the host.
#![cfg(not(target_arch = "wasm32"))]

use crate::comms::Tier;
use crate::contacts::{tier_of, Contact};
use crate::invite::BEARER_LEN;

/// Why a join was admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmitReason {
    /// Already in the host's book at Kin or Kith.
    AlreadyContact,
    /// Presented the session's live bearer. The caller adds them as a contact.
    ByInvite,
}

/// Why a join was refused. `ProtocolMismatch` and `Full` are decided by the
/// caller (they are not identity questions) but live here so one enum covers
/// every refusal that can appear in an `Answer`.
///
/// Every variant here is one the host actually produces. A `HostBusy` was
/// carried for a while and never emitted by anything; a refusal nobody can
/// send is a wire word with no meaning, and the joiner's `on_answer` would map
/// it to "they didn't answer" anyway. Add one back the same day something
/// produces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotAContact,
    BearerInvalid,
    BearerExpired,
    ProtocolMismatch,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Accept(AdmitReason),
    Refuse(Refusal),
}

/// The invite a host currently has open. Minting a fresh invite replaces this,
/// which is what "retires the old bearer" means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveBearer {
    pub bearer: [u8; BEARER_LEN],
    pub expires_at: u64,
}

/// Whether a tier is close enough to play in someone's world.
///
/// An explicit enumeration rather than `tier <= Kith`: `Tier` is deliberately
/// not `Ord` (see the derive comment in `comms.rs`), so reordering the variants
/// cannot silently widen this.
pub fn admits_play(tier: Tier) -> bool {
    matches!(tier, Tier::Kin | Tier::Kith)
}

/// Constant-time 16-byte comparison. A bearer is a shared secret, and the
/// number of relay round-trips an attacker can drive is not something this
/// process controls, so the comparison does not short-circuit on the first
/// differing byte. No new dependency — it is four lines.
fn bearer_eq(a: &[u8; BEARER_LEN], b: &[u8; BEARER_LEN]) -> bool {
    let mut diff = 0u8;
    for i in 0..BEARER_LEN {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// The admission decision. Pure: `now` is unix seconds, injected.
pub fn admit(
    persona: &[u8; 32],
    offered: Option<[u8; BEARER_LEN]>,
    book: &[Contact],
    active: Option<&ActiveBearer>,
    now: u64,
) -> Admission {
    if admits_play(tier_of(book, persona)) {
        return Admission::Accept(AdmitReason::AlreadyContact);
    }
    match (offered, active) {
        (Some(offered), Some(active)) if bearer_eq(&offered, &active.bearer) => {
            if now <= active.expires_at {
                Admission::Accept(AdmitReason::ByInvite)
            } else {
                Admission::Refuse(Refusal::BearerExpired)
            }
        }
        // A bearer was presented but it isn't the live one (or there is no live
        // one at all).
        (Some(_), _) => Admission::Refuse(Refusal::BearerInvalid),
        (None, _) => Admission::Refuse(Refusal::NotAContact),
    }
}

/// Whether a refusal may be put on the wire at all.
///
/// Silence is the default. `NotAContact` and the bearer refusals would tell a
/// stranger that somebody is hosting here and that their guess was close, so
/// they are never sent. `ProtocolMismatch` and `Full` are actionable facts for
/// somebody who already got through the door once.
pub fn is_reply_worthy(r: Refusal) -> bool {
    matches!(r, Refusal::ProtocolMismatch | Refusal::Full)
}

/// The wire spelling of a refusal, for the `Answer.reason` field.
pub fn refusal_wire(r: Refusal) -> &'static str {
    match r {
        Refusal::NotAContact => "not-a-contact",
        Refusal::BearerInvalid => "bearer-invalid",
        Refusal::BearerExpired => "bearer-expired",
        Refusal::ProtocolMismatch => "protocol-mismatch",
        Refusal::Full => "full",
    }
}

/// Inverse of [`refusal_wire`]. Unknown text yields `None` rather than a
/// default, so a future refusal kind reads as "unexplained" and not as "full".
pub fn refusal_from_wire(s: &str) -> Option<Refusal> {
    match s {
        "not-a-contact" => Some(Refusal::NotAContact),
        "bearer-invalid" => Some(Refusal::BearerInvalid),
        "bearer-expired" => Some(Refusal::BearerExpired),
        "protocol-mismatch" => Some(Refusal::ProtocolMismatch),
        "full" => Some(Refusal::Full),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comms::{Tier, ALL_TIERS};
    use crate::contacts::{AddedVia, Contact};

    const ALICE: [u8; 32] = [1u8; 32];
    const BEARER: [u8; 16] = [7u8; 16];
    const WRONG: [u8; 16] = [8u8; 16];

    fn book_at(tier: Tier) -> Vec<Contact> {
        vec![Contact {
            pubkey: ALICE,
            display_name: Some("Alice".to_string()),
            tier,
            is_child: false,
            runtime_pubkey: None,
            added_via: AddedVia::Kenspeckle,
            added_at: 0,
            last_joined: None,
        }]
    }

    fn active() -> ActiveBearer {
        ActiveBearer { bearer: BEARER, expires_at: 1_000 }
    }

    #[test]
    fn every_tier_is_decided_and_only_kin_and_kith_play() {
        // Exhaustive over the tier axis, so adding a tier later fails here
        // rather than silently defaulting somebody in.
        for tier in ALL_TIERS {
            let got = admit(&ALICE, None, &book_at(tier), Some(&active()), 500);
            match tier {
                Tier::Kin | Tier::Kith => {
                    assert_eq!(got, Admission::Accept(AdmitReason::AlreadyContact), "{tier:?}")
                }
                // Ken is hear-only in comms; it is not "play in my world".
                Tier::Ken | Tier::Stranger => {
                    assert_eq!(got, Admission::Refuse(Refusal::NotAContact), "{tier:?}")
                }
            }
        }
    }

    #[test]
    fn a_stranger_with_the_live_bearer_is_admitted() {
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], Some(&active()), 500),
            Admission::Accept(AdmitReason::ByInvite)
        );
    }

    #[test]
    fn the_bearer_is_valid_up_to_and_including_its_expiry_second() {
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], Some(&active()), 1_000),
            Admission::Accept(AdmitReason::ByInvite)
        );
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], Some(&active()), 1_001),
            Admission::Refuse(Refusal::BearerExpired)
        );
    }

    #[test]
    fn a_wrong_bearer_is_invalid_not_expired() {
        assert_eq!(
            admit(&ALICE, Some(WRONG), &[], Some(&active()), 500),
            Admission::Refuse(Refusal::BearerInvalid)
        );
    }

    #[test]
    fn a_bearer_with_no_invite_open_is_invalid() {
        assert_eq!(
            admit(&ALICE, Some(BEARER), &[], None, 500),
            Admission::Refuse(Refusal::BearerInvalid)
        );
    }

    #[test]
    fn a_stranger_with_no_bearer_is_simply_not_a_contact() {
        assert_eq!(
            admit(&ALICE, None, &[], Some(&active()), 500),
            Admission::Refuse(Refusal::NotAContact)
        );
    }

    #[test]
    fn being_a_contact_beats_a_wrong_bearer() {
        // A Kith friend who pasted a stale link still gets in.
        assert_eq!(
            admit(&ALICE, Some(WRONG), &book_at(Tier::Kith), Some(&active()), 500),
            Admission::Accept(AdmitReason::AlreadyContact)
        );
    }

    #[test]
    fn only_protocol_mismatch_and_full_are_ever_answered() {
        // The host stays SILENT to strangers — an answer would confirm it is
        // there and hosting. Only the two refusals that are useful to a person
        // who is already entitled to know get sent.
        assert!(is_reply_worthy(Refusal::ProtocolMismatch));
        assert!(is_reply_worthy(Refusal::Full));
        for r in [
            Refusal::NotAContact,
            Refusal::BearerInvalid,
            Refusal::BearerExpired,
        ] {
            assert!(!is_reply_worthy(r), "{r:?} must never be sent");
        }
    }

    #[test]
    fn refusal_wire_round_trips() {
        for r in [
            Refusal::NotAContact,
            Refusal::BearerInvalid,
            Refusal::BearerExpired,
            Refusal::ProtocolMismatch,
            Refusal::Full,
        ] {
            assert_eq!(refusal_from_wire(refusal_wire(r)), Some(r));
        }
        assert_eq!(refusal_from_wire("something-else"), None);
    }
}
