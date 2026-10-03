//! Offer/Answer types and their NIP-44 envelope.
//!
//! The outer event is signed by the sender's **runtime** key and `p`-tagged to
//! the recipient's. The payload — including the persona attestation that ties
//! the runtime key to a person — lives entirely inside the ciphertext.
#![cfg(not(target_arch = "wasm32"))]

use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, Tag, ToBech32};

/// Joiner → host. Ephemeral (NIP-01 20000..30000): relays do not store it.
pub const KIND_JOIN_OFFER: u16 = 20900;
/// Host → joiner. Ephemeral.
pub const KIND_JOIN_ANSWER: u16 = 20901;
/// Payload schema version. Fields are append-only: every field added after v1
/// must carry `#[serde(default)]` so an older sender's payload (missing that
/// field) still parses under a newer build.
pub const PAYLOAD_VERSION: u32 = 1;

/// One address the peer may be reachable at.
///
/// `kind` is the wire spelling of `nat::candidates::CandidateKind`
/// (`"lan" | "v6" | "upnp" | "stun"`); it is a `String` here rather than the
/// enum so an unrecognised future kind survives a round-trip instead of failing
/// the whole payload. `addr` is `ip:port`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub kind: String,
    pub addr: String,
}

/// "I would like to join your world."
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Offer {
    pub v: u32,
    /// 16 random bytes, hex. Ties an answer to its offer and is the replay key.
    pub session: String,
    /// The joiner's persona, as an npub.
    pub persona: String,
    /// The kind-30420 `role=player` event tying `persona` to the outer signer.
    /// Carried here and never published, so no relay can correlate the two.
    pub attestation: Event,
    /// The invite bearer, hex, when joining by invite rather than as a contact.
    pub bearer: Option<String>,
    /// `protocol::PROTOCOL_VERSION`. Lets a mismatch be explained before a
    /// connect is attempted; the packets themselves are unchanged.
    pub protocol: u32,
    pub candidates: Vec<Candidate>,
    pub sent_at: u64,
}

/// "Yes, here is where to reach me" — or a refusal, for the two refusals that
/// are ever sent (`online_admission::is_reply_worthy`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Answer {
    pub v: u32,
    pub session: String,
    pub persona: String,
    pub attestation: Event,
    pub accepted: bool,
    /// `online_admission::refusal_wire` spelling; `None` when accepted.
    pub reason: Option<String>,
    pub protocol: u32,
    pub candidates: Vec<Candidate>,
    pub world_name: String,
    pub sent_at: u64,
}

/// A fresh 16-byte session id, lowercase hex.
pub fn new_session_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS RNG unavailable");
    hex::encode(b)
}

async fn seal<T: serde::Serialize>(
    sender: &Keys,
    recipient: &PublicKey,
    kind: u16,
    payload: &T,
) -> Result<Event, String> {
    let json = serde_json::to_string(payload).map_err(|e| format!("serialise payload: {e}"))?;
    let ciphertext = nostr::nips::nip44::encrypt(
        sender.secret_key(),
        recipient,
        json,
        nostr::nips::nip44::Version::V2,
    )
    .map_err(|e| format!("nip44 encrypt: {e}"))?;
    EventBuilder::new(Kind::Custom(kind), ciphertext)
        .tags([Tag::public_key(*recipient)])
        .sign(sender)
        .await
        .map_err(|e| format!("sign: {e}"))
}

fn open<T: serde::de::DeserializeOwned>(
    recipient: &Keys,
    ev: &Event,
    expect_kind: u16,
) -> Result<T, String> {
    if ev.kind != Kind::Custom(expect_kind) {
        return Err(format!("wrong kind {} (wanted {expect_kind})", ev.kind.as_u16()));
    }
    let json = nostr::nips::nip44::decrypt(recipient.secret_key(), &ev.pubkey, &ev.content)
        .map_err(|e| format!("nip44 decrypt: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("parse payload: {e}"))
}

pub async fn seal_offer(
    sender: &Keys,
    recipient: &PublicKey,
    offer: &Offer,
) -> Result<Event, String> {
    seal(sender, recipient, KIND_JOIN_OFFER, offer).await
}

pub async fn seal_answer(
    sender: &Keys,
    recipient: &PublicKey,
    answer: &Answer,
) -> Result<Event, String> {
    seal(sender, recipient, KIND_JOIN_ANSWER, answer).await
}

pub fn open_offer(recipient: &Keys, ev: &Event) -> Result<Offer, String> {
    open(recipient, ev, KIND_JOIN_OFFER)
}

pub fn open_answer(recipient: &Keys, ev: &Event) -> Result<Answer, String> {
    open(recipient, ev, KIND_JOIN_ANSWER)
}

/// The runtime key an event is addressed to (its first `p` tag).
///
/// Test-only. A real relay applies the `#p` filter itself, so nothing in a
/// shipped build re-reads it; the one caller is `FakeRelayHub`, which applies
/// the same filter in memory so a test proves the filter as well as the flow.
#[cfg(test)]
pub fn recipient_of(ev: &Event) -> Option<PublicKey> {
    ev.tags.iter().find_map(|t| {
        let s = t.as_slice();
        (s.len() >= 2 && s[0] == "p")
            .then(|| PublicKey::from_hex(&s[1]).ok())
            .flatten()
    })
}

/// An npub for a pubkey, falling back to hex only if bech32 encoding fails
/// (which it cannot for a valid key). Callers put this in `persona`.
pub fn npub_of(pk: &PublicKey) -> String {
    pk.to_bech32().unwrap_or_else(|_| pk.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    async fn dummy_attestation(persona: &Keys, runtime: &PublicKey) -> Event {
        crate::runtime_identity::mint_player_attestation(
            persona,
            runtime,
            nostr::Timestamp::from(1_000),
            90,
        )
        .await
        .unwrap()
    }

    async fn an_offer(persona: &Keys, runtime: &Keys) -> Offer {
        Offer {
            v: PAYLOAD_VERSION,
            session: new_session_id(),
            persona: persona.public_key().to_bech32().unwrap(),
            attestation: dummy_attestation(persona, &runtime.public_key()).await,
            bearer: Some("0f".repeat(16)),
            protocol: crate::protocol::PROTOCOL_VERSION,
            candidates: vec![
                Candidate { kind: "lan".to_string(), addr: "192.168.1.20:7700".to_string() },
                Candidate { kind: "stun".to_string(), addr: "203.0.113.9:41234".to_string() },
            ],
            sent_at: 1_700_000_000,
        }
    }

    #[test]
    fn session_ids_are_32_hex_chars_and_unique() {
        let a = new_session_id();
        assert_eq!(a.len(), 32, "16 bytes, hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, new_session_id());
    }

    #[test]
    fn offer_seals_and_opens_between_the_right_two_runtime_keys() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;

            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            assert_eq!(ev.kind, Kind::Custom(KIND_JOIN_OFFER));
            assert_eq!(ev.pubkey, joiner_rt.public_key(), "signed by the RUNTIME key");
            assert_eq!(recipient_of(&ev), Some(host_rt.public_key()), "p-tagged to the host");
            assert!(ev.verify().is_ok());

            let back = open_offer(&host_rt, &ev).unwrap();
            assert_eq!(back.session, offer.session);
            assert_eq!(back.candidates, offer.candidates);
            assert_eq!(back.bearer, offer.bearer);
            assert_eq!(back.attestation.id, offer.attestation.id);
        });
    }

    #[test]
    fn the_ciphertext_leaks_no_address_and_no_persona() {
        // The whole red-line-3 point: candidate IPs are personal data and must
        // exist ONLY inside the ciphertext, and a relay must not be able to tie
        // a runtime key to a persona.
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();

            let wire = serde_json::to_string(&ev).unwrap();
            assert!(!wire.contains("192.168.1.20"), "LAN address leaked: {wire}");
            assert!(!wire.contains("203.0.113.9"), "reflexive address leaked");
            assert!(!wire.contains(&offer.persona), "persona npub leaked");
            assert!(
                !wire.contains(&joiner_persona.public_key().to_hex()),
                "persona hex leaked"
            );
            assert!(!wire.contains(&offer.session), "session id leaked");

            // The outer event's only tag is the `p`-tag to the recipient — no
            // other metadata rides outside the ciphertext.
            let tags: Vec<&[String]> = ev.tags.iter().map(|t| t.as_slice()).collect();
            assert_eq!(tags, vec![["p".to_string(), host_rt.public_key().to_hex()].as_slice()]);
        });
    }

    #[test]
    fn a_third_party_cannot_open_the_offer() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let eavesdropper = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            assert!(open_offer(&eavesdropper, &ev).is_err());
        });
    }

    #[test]
    fn a_tampered_ciphertext_fails_to_open() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let mut ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            // Flip a character in the middle of the base64 payload.
            let mut c: Vec<char> = ev.content.chars().collect();
            let mid = c.len() / 2;
            c[mid] = if c[mid] == 'A' { 'B' } else { 'A' };
            ev.content = c.into_iter().collect();
            assert!(open_offer(&host_rt, &ev).is_err(), "AEAD must reject a tampered payload");
        });
    }

    #[test]
    fn an_offer_opened_as_an_answer_is_refused_on_kind() {
        rt().block_on(async {
            let joiner_persona = Keys::generate();
            let joiner_rt = Keys::generate();
            let host_rt = Keys::generate();
            let offer = an_offer(&joiner_persona, &joiner_rt).await;
            let ev = seal_offer(&joiner_rt, &host_rt.public_key(), &offer).await.unwrap();
            assert!(open_answer(&host_rt, &ev).is_err(), "kind must gate the payload type");
        });
    }

    #[test]
    fn answer_round_trips_including_a_refusal_reason() {
        rt().block_on(async {
            let host_persona = Keys::generate();
            let host_rt = Keys::generate();
            let joiner_rt = Keys::generate();
            let answer = Answer {
                v: PAYLOAD_VERSION,
                session: new_session_id(),
                persona: host_persona.public_key().to_bech32().unwrap(),
                attestation: dummy_attestation(&host_persona, &host_rt.public_key()).await,
                accepted: false,
                reason: Some(
                    crate::online_admission::refusal_wire(
                        crate::online_admission::Refusal::ProtocolMismatch,
                    )
                    .to_string(),
                ),
                protocol: 61,
                candidates: vec![],
                world_name: "Ivy's Hollow".to_string(),
                sent_at: 1_700_000_000,
            };
            let ev = seal_answer(&host_rt, &joiner_rt.public_key(), &answer).await.unwrap();
            assert_eq!(ev.kind, Kind::Custom(KIND_JOIN_ANSWER));
            let wire = serde_json::to_string(&ev).unwrap();
            assert!(!wire.contains("Ivy's Hollow"), "world name leaked outside the ciphertext");
            let back = open_answer(&joiner_rt, &ev).unwrap();
            assert!(!back.accepted);
            assert_eq!(
                back.reason.as_deref(),
                Some(crate::online_admission::refusal_wire(
                    crate::online_admission::Refusal::ProtocolMismatch
                ))
            );
            assert_eq!(back.world_name, "Ivy's Hollow");
        });
    }

    #[test]
    fn both_kinds_are_in_the_nip01_ephemeral_range() {
        // Ephemeral (20000..30000) means relays do not store them, which is why
        // both sides must be online — true for a join by construction.
        for k in [KIND_JOIN_OFFER, KIND_JOIN_ANSWER] {
            assert!((20_000..30_000).contains(&k), "{k} is not ephemeral");
        }
    }
}
