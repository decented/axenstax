//! The verification chain every inbound offer and answer runs through, and the
//! replay guard.
//!
//! The chain, in order, and why each link is there:
//!
//! 1. **Outer signature** — the event really was signed by the key that claims
//!    to have signed it. Cheapest check that can reject a forgery, so it is
//!    first.
//! 2. **Kind + decrypt + parse + version** — the envelope is one of ours and
//!    the payload is a shape we understand.
//! 3. **Attestation** — the enclosed kind-30420 `role=player` event is validly
//!    signed by the persona it names, is inside its validity window, and names
//!    **the outer signer** as its runtime key. Without link 3's last clause,
//!    anybody could replay somebody else's attestation under their own key.
//! 4. **Persona agreement** — the payload's `persona` npub is the attestation's
//!    signer. Without this the payload could name one person while proving
//!    another.
//! 5. **Freshness** — `sent_at` within ±120 s of now, so a captured offer is
//!    not useful tomorrow.
//! 6. **Session unseen** — the same session id is admitted once.
//!
//! Any failure is a **silent drop** at the call site: the host never answers a
//! stranger (spec §3.3). The distinct error variants exist for logs and tests,
//! not for a reply.
#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;

use nostr::{Event, Keys, PublicKey};

use crate::rendezvous::payload::{open_answer, open_offer, Answer, Offer, PAYLOAD_VERSION};
use crate::runtime_identity::verify_player_attestation;

/// How far an offer/answer's `sent_at` may sit from the receiver's clock.
/// Generous enough for a household router's idea of the time, tight enough that
/// a captured payload is stale within minutes.
pub const CLOCK_SKEW_SECS: i64 = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendezvousError {
    /// The outer event's signature does not match its id/pubkey.
    BadOuterSignature,
    /// Wrong kind, or the ciphertext did not open, or the JSON did not parse.
    Envelope(String),
    BadVersion(u32),
    /// The enclosed attestation did not verify (bad signature, expired, not a
    /// `role=player` attestation, …).
    BadAttestation(String),
    /// The attestation names a runtime key that is not the outer signer, or
    /// (for an answer) the outer signer is not the host runtime key the
    /// joiner actually addressed.
    RuntimeMismatch,
    /// The payload's `persona` is not the attestation's signer.
    PersonaMismatch,
    /// `sent_at` outside ±[`CLOCK_SKEW_SECS`]; carries the signed delta.
    StaleTimestamp(i64),
    /// This session id has already been handled (or, for an answer, is not the
    /// session we are waiting on).
    ReplayedSession(String),
}

/// Hard cap on remembered sessions, independent of the time window — bounds
/// memory even against a flood of distinct spoofed session ids arriving
/// inside one freshness window (time-based pruning alone doesn't help there).
pub const MAX_SEEN: usize = 1024;

/// Remembers session ids briefly so the same offer cannot be processed twice.
///
/// Bounded two ways: time (pruned on every insert — an id older than twice the
/// skew window can no longer be part of a fresh payload, so forgetting it is
/// safe) and cardinality ([`MAX_SEEN`], oldest-first eviction).
#[derive(Default)]
pub struct SessionGuard {
    seen: HashMap<String, u64>,
}

impl SessionGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` if this session is new (and is now remembered); `false` if it was
    /// already seen inside the window.
    pub fn check_and_insert(&mut self, session: &str, now: u64) -> bool {
        let horizon = 2 * CLOCK_SKEW_SECS as u64;
        self.seen.retain(|_, t| now.saturating_sub(*t) <= horizon);
        if self.seen.contains_key(session) {
            return false;
        }
        if self.seen.len() >= MAX_SEEN {
            // Evict the single oldest entry to make room. A flood of distinct
            // ids can only ever displace the stalest ones, never grow without
            // bound.
            if let Some(oldest) =
                self.seen.iter().min_by_key(|(_, t)| **t).map(|(k, _)| k.clone())
            {
                self.seen.remove(&oldest);
            }
        }
        self.seen.insert(session.to_string(), now);
        true
    }
}

#[derive(Debug, PartialEq)]
pub struct VerifiedOffer {
    pub offer: Offer,
    /// The joiner's persona, x-only bytes.
    pub persona: [u8; 32],
    /// The joiner's runtime key (= the outer signer), x-only bytes.
    pub runtime: [u8; 32],
}

#[derive(Debug, PartialEq)]
pub struct VerifiedAnswer {
    pub answer: Answer,
    pub persona: [u8; 32],
    pub runtime: [u8; 32],
}

/// Links 1, 3, 4, 5 — everything that does not depend on which payload type
/// this is. Returns the attesting persona.
fn verify_common(
    ev: &Event,
    attestation: &Event,
    claimed_persona: &str,
    v: u32,
    sent_at: u64,
    now: u64,
) -> Result<PublicKey, RendezvousError> {
    // `v == 0` is never a valid schema; `v > PAYLOAD_VERSION` is a version this
    // build does not understand yet. Anything in between (today, only `v ==
    // PAYLOAD_VERSION`, since v1 is the floor) is accepted — an older sender's
    // payload still parses under a newer build because every post-v1 field
    // carries `#[serde(default)]` (see `PAYLOAD_VERSION`'s doc).
    if v == 0 || v > PAYLOAD_VERSION {
        return Err(RendezvousError::BadVersion(v));
    }
    let persona = verify_player_attestation(attestation, &ev.pubkey, nostr::Timestamp::from(now))
        .map_err(|e| match e {
            crate::server_identity::attestation::AttestationError::WrongServerKey => {
                RendezvousError::RuntimeMismatch
            }
            other => RendezvousError::BadAttestation(format!("{other:?}")),
        })?;

    let claimed =
        PublicKey::parse(claimed_persona).map_err(|_| RendezvousError::PersonaMismatch)?;
    if claimed != persona {
        return Err(RendezvousError::PersonaMismatch);
    }

    // i128: `sent_at` is attacker-controlled and unbounded (up to `u64::MAX`),
    // so the naive `i64` subtraction can overflow, and `i64::MIN.abs()`
    // panics. i128 comfortably holds the full `u64 - i64` range.
    let delta_i128 = sent_at as i128 - now as i128;
    if delta_i128.abs() > CLOCK_SKEW_SECS as i128 {
        let delta = delta_i128.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        return Err(RendezvousError::StaleTimestamp(delta));
    }
    Ok(persona)
}

/// Verify an inbound kind-20900 addressed to `me`. On any `Err` the caller
/// drops the event without answering.
pub fn verify_offer(
    ev: &Event,
    me: &Keys,
    guard: &mut SessionGuard,
    now: u64,
) -> Result<VerifiedOffer, RendezvousError> {
    if ev.verify().is_err() {
        return Err(RendezvousError::BadOuterSignature);
    }
    let offer = open_offer(me, ev).map_err(RendezvousError::Envelope)?;
    let persona =
        verify_common(ev, &offer.attestation, &offer.persona, offer.v, offer.sent_at, now)?;
    if !guard.check_and_insert(&offer.session, now) {
        return Err(RendezvousError::ReplayedSession(offer.session));
    }
    Ok(VerifiedOffer { persona: persona.to_bytes(), runtime: ev.pubkey.to_bytes(), offer })
}

/// Verify an inbound kind-20901 addressed to `me`, for the session we are
/// actually waiting on, from the host runtime key we actually addressed our
/// offer to. Without that last check, a validly-attested runtime key that is
/// simply not the one the joiner called could answer in the real host's
/// place. An answer for any other session is treated as a replay — a joiner
/// has exactly one outstanding call.
pub fn verify_answer(
    ev: &Event,
    me: &Keys,
    expected_host_runtime: &PublicKey,
    expect_session: &str,
    now: u64,
) -> Result<VerifiedAnswer, RendezvousError> {
    if ev.verify().is_err() {
        return Err(RendezvousError::BadOuterSignature);
    }
    if &ev.pubkey != expected_host_runtime {
        return Err(RendezvousError::RuntimeMismatch);
    }
    let answer = open_answer(me, ev).map_err(RendezvousError::Envelope)?;
    let persona =
        verify_common(ev, &answer.attestation, &answer.persona, answer.v, answer.sent_at, now)?;
    if answer.session != expect_session {
        return Err(RendezvousError::ReplayedSession(answer.session));
    }
    Ok(VerifiedAnswer { persona: persona.to_bytes(), runtime: ev.pubkey.to_bytes(), answer })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendezvous::payload::{
        npub_of, new_session_id, seal_offer, Candidate, Offer, KIND_JOIN_OFFER, PAYLOAD_VERSION,
    };
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    const NOW: u64 = 1_700_000_000;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    struct Peer {
        persona: Keys,
        runtime: Keys,
    }

    impl Peer {
        fn new() -> Self {
            Peer { persona: Keys::generate(), runtime: Keys::generate() }
        }
        async fn attestation(&self) -> nostr::Event {
            crate::runtime_identity::mint_player_attestation(
                &self.persona,
                &self.runtime.public_key(),
                Timestamp::from(NOW - 10),
                90,
            )
            .await
            .unwrap()
        }
        async fn offer(&self, sent_at: u64) -> Offer {
            Offer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&self.persona.public_key()),
                attestation: self.attestation().await,
                bearer: None,
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: vec![Candidate {
                    kind: "lan".to_string(),
                    addr: "10.0.0.5:7700".to_string(),
                }],
                sent_at,
            }
        }
    }

    #[test]
    fn a_well_formed_offer_verifies_and_yields_both_keys() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let offer = joiner.offer(NOW).await;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            let v = verify_offer(&ev, &host_rt, &mut guard, NOW).unwrap();
            assert_eq!(v.persona, joiner.persona.public_key().to_bytes());
            assert_eq!(v.runtime, joiner.runtime.public_key().to_bytes());
            assert_eq!(v.offer.session, offer.session);
        });
    }

    #[test]
    fn an_attestation_for_a_different_runtime_key_is_refused() {
        // The core forgery: take somebody else's real attestation and sign the
        // outer event with your own key.
        rt().block_on(async {
            let victim = Peer::new();
            let attacker_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = victim.offer(NOW).await;
            let ev = seal_offer(&attacker_rt, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::RuntimeMismatch)
            );
        });
    }

    #[test]
    fn a_persona_field_that_disagrees_with_the_attestation_is_refused() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let someone_else = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            offer.persona = npub_of(&someone_else.public_key());
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::PersonaMismatch)
            );
        });
    }

    #[test]
    fn a_tampered_outer_event_is_refused_before_anything_else() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let offer = joiner.offer(NOW).await;
            let mut ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            ev.created_at = Timestamp::from(NOW + 5);
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::BadOuterSignature)
            );
        });
    }

    #[test]
    fn a_stale_or_future_offer_is_refused_at_the_120s_boundary() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            for (sent_at, ok) in [
                (NOW - 120, true),
                (NOW - 121, false),
                (NOW + 120, true),
                (NOW + 121, false),
            ] {
                let offer = joiner.offer(sent_at).await;
                let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                    .await
                    .unwrap();
                let mut guard = SessionGuard::new();
                let got = verify_offer(&ev, &host_rt, &mut guard, NOW);
                assert_eq!(got.is_ok(), ok, "sent_at {sent_at} should be ok={ok}");
            }
        });
    }

    #[test]
    fn the_same_session_is_only_accepted_once() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let offer = joiner.offer(NOW).await;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert!(verify_offer(&ev, &host_rt, &mut guard, NOW).is_ok());
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::ReplayedSession(offer.session.clone()))
            );
        });
    }

    #[test]
    fn the_session_guard_forgets_entries_older_than_the_skew_window() {
        let mut g = SessionGuard::new();
        assert!(g.check_and_insert("aa", 1_000));
        assert!(!g.check_and_insert("aa", 1_000), "still remembered inside the window");
        // Well past 2x the skew window, the entry is pruned and the id is free
        // again — bounded memory, and a session id is not reused in practice.
        assert!(g.check_and_insert("aa", 1_000 + 2 * CLOCK_SKEW_SECS as u64 + 1));
    }

    #[test]
    fn an_unknown_payload_version_is_refused() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            offer.v = 99;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::BadVersion(99))
            );
        });
    }

    #[test]
    fn an_offer_with_a_server_attestation_is_refused() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            // A role-less (server) attestation over the same runtime key.
            offer.attestation = crate::server_identity::attestation::mint_attestation(
                &joiner.persona,
                &joiner.runtime.public_key(),
                Timestamp::from(NOW - 10),
                Timestamp::from(NOW + 10_000),
                &[27420],
                "",
                None,
            )
            .await
            .unwrap();
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert!(matches!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::BadAttestation(_))
            ));
        });
    }

    #[test]
    fn an_answer_for_a_different_session_is_refused() {
        rt().block_on(async {
            use crate::rendezvous::payload::{seal_answer, Answer};
            let host = Peer::new();
            let joiner_rt = Keys::generate();
            let answer = Answer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&host.persona.public_key()),
                attestation: host.attestation().await,
                accepted: true,
                reason: None,
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: vec![],
                world_name: "W".to_string(),
                sent_at: NOW,
            };
            let ev = seal_answer(&host.runtime, &joiner_rt.public_key(), &answer)
                .await
                .unwrap();
            let host_runtime = host.runtime.public_key();
            assert!(verify_answer(&ev, &joiner_rt, &host_runtime, &answer.session, NOW).is_ok());
            assert_eq!(
                verify_answer(&ev, &joiner_rt, &host_runtime, "deadbeef", NOW),
                Err(RendezvousError::ReplayedSession(answer.session.clone())),
            );
        });
    }

    #[test]
    fn an_answer_signed_by_a_runtime_key_other_than_the_one_we_called_is_refused() {
        // The answer-side forgery: a perfectly valid, validly-attested answer
        // — just not from the host we actually addressed our offer to.
        rt().block_on(async {
            use crate::rendezvous::payload::{seal_answer, Answer};
            let host = Peer::new();
            let joiner_rt = Keys::generate();
            let someone_else_rt = Keys::generate();
            let answer = Answer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: npub_of(&host.persona.public_key()),
                attestation: host.attestation().await,
                accepted: true,
                reason: None,
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: vec![],
                world_name: "W".to_string(),
                sent_at: NOW,
            };
            let ev = seal_answer(&host.runtime, &joiner_rt.public_key(), &answer)
                .await
                .unwrap();
            assert_eq!(
                verify_answer(
                    &ev,
                    &joiner_rt,
                    &someone_else_rt.public_key(),
                    &answer.session,
                    NOW
                ),
                Err(RendezvousError::RuntimeMismatch)
            );
        });
    }

    #[test]
    fn an_unparsable_persona_field_is_refused_as_a_persona_mismatch() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            offer.persona = "not-an-npub".to_string();
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::PersonaMismatch)
            );
        });
    }

    #[test]
    fn an_offer_with_version_zero_is_refused() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let mut offer = joiner.offer(NOW).await;
            offer.v = 0;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert_eq!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::BadVersion(0))
            );
        });
    }

    #[test]
    fn a_maximal_sent_at_is_refused_as_stale_rather_than_panicking() {
        // `sent_at` is attacker-controlled: a naive `i64` subtraction of two
        // `u64`-derived values (or `.abs()` on the result) can overflow/panic.
        // This must come back as an ordinary typed error.
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let offer = joiner.offer(u64::MAX).await;
            let ev = seal_offer(&joiner.runtime, &host_rt.public_key(), &offer)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert!(matches!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::StaleTimestamp(_))
            ));
        });
    }

    #[test]
    fn the_session_guard_caps_cardinality_regardless_of_time() {
        let mut g = SessionGuard::new();
        // Flood it with more distinct sessions, all at the same instant (well
        // inside the freshness window), than the cap allows — time-based
        // pruning alone does nothing here, only the cardinality cap can.
        for i in 0..(MAX_SEEN + 10) {
            assert!(g.check_and_insert(&format!("s{i}"), 1_000));
        }
        // Every insert past the cap evicts exactly one entry first, so the
        // guard settles at exactly the cap rather than merely bounded by it.
        assert_eq!(g.seen.len(), MAX_SEEN, "cap not enforced: {}", g.seen.len());
    }

    #[test]
    fn an_event_of_the_wrong_kind_is_refused_by_the_envelope() {
        rt().block_on(async {
            let joiner = Peer::new();
            let host_rt = Keys::generate();
            let ev = EventBuilder::new(Kind::Custom(1), "hello")
                .tags([Tag::public_key(host_rt.public_key())])
                .sign(&joiner.runtime)
                .await
                .unwrap();
            let mut guard = SessionGuard::new();
            assert!(matches!(
                verify_offer(&ev, &host_rt, &mut guard, NOW),
                Err(RendezvousError::Envelope(_))
            ));
            assert_ne!(ev.kind, Kind::Custom(KIND_JOIN_OFFER));
        });
    }
}
