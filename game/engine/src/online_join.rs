//! Joining a friend's world.
//!
//! Publish one offer to the host's runtime key, wait up to eight seconds for an
//! answer, then punch toward whatever addresses it names and race a QUIC
//! connect across them. The winner is handed back as a plain transport so the
//! caller can run the ordinary authed join handshake on it — nothing about
//! this module is a second way into a world, only a second way to *find* one.
//!
//! There is no directory here either: a join can only start from an invite the
//! host minted or a contact already in the book, and the only address this
//! module ever learns is the one the host chose to tell it, inside NIP-44
//! ciphertext (CLAUDE.md red lines 1 and 3).
//!
//! `poll` is called from the game loop and never blocks: relay reads are
//! `try_recv`, sealing the offer is pure CPU, and the two things that do sleep
//! — punching and the connect handshakes — live on the worker thread
//! `network::connect_to_server_on_socket` spawns.
//!
//! Everything a player might see when this does not work lives in
//! [`failure_copy`] — four sentences, pinned verbatim by unit tests, written
//! for a child to read and act on.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §4.4, §5.2.
#![cfg(not(target_arch = "wasm32"))]

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use nostr::PublicKey;

use crate::contacts::{AddedVia, Contact};
use crate::nat::candidates::parse_addrs;
use crate::network::{connect_to_server_on_socket, OnlineConnect};
use crate::online_admission::{refusal_from_wire, Refusal};
use crate::online_host::MAX_EVENTS_PER_POLL;
use crate::rendezvous::payload::{
    npub_of, seal_offer, Candidate, Offer, KIND_JOIN_ANSWER, PAYLOAD_VERSION,
};
use crate::rendezvous::relay_client::RendezvousRelay;
use crate::rendezvous::verify::{verify_answer, VerifiedAnswer};
use crate::runtime_identity::RuntimeIdentity;
use crate::transport::ClientTransport;

/// How long to wait for an answer before giving up. Same budget as the connect
/// race, so a failed join never takes more than about sixteen seconds end to
/// end.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(8);

/// The four ways a join can fail in a way worth telling somebody about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinFailure {
    /// No answer inside [`ANSWER_TIMEOUT`].
    NoAnswer,
    /// Answered, but no candidate completed a handshake.
    NoConnect,
    ProtocolMismatch,
    Full,
}

/// The player-facing message for a failure. Spec §4.4, verbatim.
///
/// Rules these four sentences follow, and that the tests enforce: no jargon a
/// child would have to look up, UK English, and — where there is something to
/// do about it — say what that is.
pub fn failure_copy(failure: JoinFailure, name: &str) -> String {
    match failure {
        JoinFailure::NoAnswer => {
            format!("{name} didn't answer. Are they online with the world open?")
        }
        JoinFailure::NoConnect => format!(
            "Couldn't reach {name}'s world. Their router needs UPnP turned on, or you both \
             need IPv6. Ask them to check Settings → Online in the game."
        ),
        JoinFailure::ProtocolMismatch => {
            "You're on different versions. One of you needs to update.".to_string()
        }
        JoinFailure::Full => format!("{name}'s world is full."),
    }
}

/// Where a join has got to. Returned from every `poll`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinStep {
    /// Offer sent, no answer yet.
    Waiting,
    /// Answered and accepted; the connect race is running.
    Connecting { world_name: String },
    /// A candidate won. Take the transport and run the authed handshake.
    Ready {
        world_name: String,
        host_persona: [u8; 32],
    },
    /// Give up and show this to the player.
    Failed(String),
}

enum Phase {
    Waiting,
    Connecting {
        world_name: String,
        outcome: Receiver<Result<SocketAddr, String>>,
    },
    /// Terminal. Holds the step that got us here so a caller polling every
    /// frame keeps being told the same thing rather than falling back to
    /// `Waiting` — a failure message that flickered away after one frame would
    /// be worse than none.
    Settled(JoinStep),
}

pub struct OnlineJoin {
    identity: RuntimeIdentity,
    relay: Box<dyn RendezvousRelay>,
    /// Held until an answer arrives, then moved into the connect race. The
    /// candidates in the offer were gathered on THIS socket, so the race must
    /// dial from it too or the host punched a hole toward the wrong port.
    socket: Option<UdpSocket>,
    session: String,
    /// The one runtime key an answer may be signed by — from the invite's `k=`
    /// or the contact's stored `runtime_pubkey`. Parsed once at `start`,
    /// because `verify_answer` is handed it on every event.
    host_runtime: PublicKey,
    host_persona: Option<[u8; 32]>,
    display_name: String,
    host_contact: Option<Contact>,
    transport: Option<Box<dyn ClientTransport>>,
    phase: Phase,
}

impl OnlineJoin {
    /// Publish the offer and start waiting.
    ///
    /// `socket` must be the socket `candidates` were gathered on, and
    /// `host_runtime` the key from the invite (`k=`) or from the contact's
    /// stored `runtime_pubkey` — it is the only key an answer will be accepted
    /// from.
    #[allow(clippy::too_many_arguments)] // every one is a distinct dependency
    pub fn start(
        identity: RuntimeIdentity,
        persona: [u8; 32],
        relay: Box<dyn RendezvousRelay>,
        socket: UdpSocket,
        candidates: Vec<Candidate>,
        host_runtime: [u8; 32],
        bearer: Option<[u8; 16]>,
        display_name: String,
        now: u64,
    ) -> Result<OnlineJoin, crate::online_host::StartFailed> {
        // Everything that can be rejected out of hand goes first, so a bad
        // argument cannot leave a subscription behind on the relay set. Every
        // early return hands the relay BACK rather than dropping it here — see
        // `online_host::StartFailed`.
        let Some(attestation) = identity.attestation().cloned() else {
            return Err((
                "sign in and attest this device before playing online".to_string(),
                relay,
            ));
        };
        let persona = match PublicKey::from_slice(&persona) {
            Ok(pk) => pk,
            Err(e) => return Err((e.to_string(), relay)),
        };
        let host_runtime = match PublicKey::from_slice(&host_runtime) {
            Ok(pk) => pk,
            Err(e) => return Err((e.to_string(), relay)),
        };

        let runtime_pk = identity.runtime_pubkey();
        // Subscribe BEFORE publishing: an answer can come back before this
        // call returns, and a filter added afterwards would miss it.
        if let Err(e) = relay.subscribe(KIND_JOIN_ANSWER, &runtime_pk.to_hex()) {
            return Err((e, relay));
        }

        let session = crate::rendezvous::payload::new_session_id();
        let offer = Offer {
            v: PAYLOAD_VERSION,
            session: session.clone(),
            persona: npub_of(&persona),
            attestation,
            bearer: bearer.map(hex::encode),
            protocol: crate::protocol::PROTOCOL_VERSION,
            candidates,
            sent_at: now,
        };
        let keys = identity.keys().clone();
        // Sealing is pure CPU (a NIP-44 encrypt and a signature) and only
        // `async` because `NostrSigner` is; nothing here awaits I/O.
        // `pollster`, not a tokio runtime: `Runtime::block_on` panics when the
        // caller is already inside one, which an async driver (and the tests)
        // would be.
        let ev = match pollster::block_on(seal_offer(&keys, &host_runtime, &offer)) {
            Ok(ev) => ev,
            Err(e) => return Err((e, relay)),
        };
        // ONE offer, handed to the relay worker, which holds it until every
        // relay in the set has had it. Republishing per relay here would show
        // the host four copies of the same call.
        if let Err(e) = relay.publish(&ev) {
            return Err((e, relay));
        }

        Ok(OnlineJoin {
            identity,
            relay,
            socket: Some(socket),
            session,
            host_runtime,
            host_persona: None,
            display_name,
            host_contact: None,
            transport: None,
            phase: Phase::Waiting,
        })
    }

    /// The session id this call is keyed on. Test-only: the game loop never
    /// needs it, and a test that seals an answer back has to name it.
    #[cfg(test)]
    pub fn session(&self) -> &str {
        &self.session
    }

    /// This joiner's runtime pubkey — what an answer must be addressed to.
    /// Test-only, for the same reason as [`OnlineJoin::session`].
    #[cfg(test)]
    pub fn runtime_pubkey(&self) -> [u8; 32] {
        self.identity.runtime_pubkey().to_bytes()
    }

    /// The name this join is showing the player — the contact's row label, or
    /// the invite's world name. Every failure message is written around it, and
    /// so is the success toast.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// The host, as a contact to fold into the mirror. `Some` once an accepted
    /// answer has been verified — the join is mutual, so calling somebody and
    /// being let in makes them a contact both ways.
    pub fn host_contact(&self) -> Option<Contact> {
        self.host_contact.clone()
    }

    /// The winning transport, once [`JoinStep::Ready`] has been returned. Taken
    /// once; the caller passes it to `RemoteClient::connect_authed_on_transport`.
    pub fn take_transport(&mut self) -> Option<Box<dyn ClientTransport>> {
        self.transport.take()
    }

    /// Advance. `elapsed` is time since `start`; `now` is unix seconds.
    ///
    /// Never blocks, and handles at most [`MAX_EVENTS_PER_POLL`] relay events
    /// per call for the same reason the host does: each one costs a signature
    /// verification and a NIP-44 decrypt, on the game loop's thread.
    pub fn poll(&mut self, now: u64, elapsed: Duration) -> JoinStep {
        match &mut self.phase {
            Phase::Settled(step) => step.clone(),
            Phase::Waiting => self.poll_waiting(now, elapsed),
            Phase::Connecting { world_name, outcome } => match outcome.try_recv() {
                Ok(Ok(addr)) => {
                    log::info!("[online] joined via {addr}");
                    let step = JoinStep::Ready {
                        world_name: world_name.clone(),
                        // Set together with the phase in `on_answer`; the
                        // fallback is unreachable and only here so a bug is a
                        // wrong key rather than a panic.
                        host_persona: self.host_persona.unwrap_or([0u8; 32]),
                    };
                    self.settle(step)
                }
                Ok(Err(e)) => {
                    log::info!("[online] the connect race was lost: {e}");
                    self.fail(JoinFailure::NoConnect)
                }
                Err(TryRecvError::Empty) => JoinStep::Connecting {
                    world_name: world_name.clone(),
                },
                // The worker died without reporting, which is the same news.
                Err(TryRecvError::Disconnected) => self.fail(JoinFailure::NoConnect),
            },
        }
    }

    fn poll_waiting(&mut self, now: u64, elapsed: Duration) -> JoinStep {
        for _ in 0..MAX_EVENTS_PER_POLL {
            let Some(ev) = self.relay.try_recv() else {
                break;
            };
            // `verify_answer` checks the outer signature (id included), that
            // the signer is the host we actually called, and that the session
            // is ours — so an answer from anybody else, however well-formed,
            // lands here as an `Err` and the wait simply continues. There is
            // nothing to reply to and nobody to tell.
            //
            // No dedupe set, unlike the host: a joiner has exactly one
            // outstanding call, so the first valid answer ends the wait and
            // the other relays' copies arrive to a phase that ignores them.
            // Nothing here is keyed on the event id, which is why the host's
            // separate `verify_id` guard has no counterpart.
            let verified = match verify_answer(
                &ev,
                self.identity.keys(),
                &self.host_runtime,
                &self.session,
                now,
            ) {
                Ok(v) => v,
                Err(e) => {
                    log::debug!("[online] dropped an answer: {e:?}");
                    continue;
                }
            };
            return self.on_answer(verified, now);
        }
        if elapsed >= ANSWER_TIMEOUT {
            return self.fail(JoinFailure::NoAnswer);
        }
        JoinStep::Waiting
    }

    fn on_answer(&mut self, verified: VerifiedAnswer, now: u64) -> JoinStep {
        let answer = verified.answer;
        if !answer.accepted {
            let failure = match answer.reason.as_deref().and_then(refusal_from_wire) {
                Some(Refusal::ProtocolMismatch) => JoinFailure::ProtocolMismatch,
                Some(Refusal::Full) => JoinFailure::Full,
                // Nothing else is ever sent (`online_admission::is_reply_worthy`),
                // so treat it as "they're not there for you" rather than
                // inventing new copy for a refusal we do not explain.
                _ => JoinFailure::NoAnswer,
            };
            return self.fail(failure);
        }

        self.host_persona = Some(verified.persona);
        // Being let in makes them a contact: the host wrote us into its book
        // when it admitted us, and the friendship is not one-directional.
        // Kith, not Kin — Kin is a deliberate, human act, never automatic.
        self.host_contact = Some(Contact {
            pubkey: verified.persona,
            display_name: Some(self.display_name.clone()),
            tier: crate::comms::Tier::Kith,
            is_child: false,
            runtime_pubkey: Some(verified.runtime),
            added_via: AddedVia::Invite,
            added_at: now,
            // The CALLER stamps this once the join actually lands — being
            // answered is not the same as having played there.
            last_joined: None,
        });

        let Some(socket) = self.socket.take() else {
            // Only reachable if `poll` were re-entered after the race started,
            // which the phase machine prevents.
            return self.fail(JoinFailure::NoConnect);
        };
        // The race punches first and then dials, on its own thread — both
        // sleep, and neither may happen here.
        let addrs = parse_addrs(&answer.candidates);
        let OnlineConnect { transport, outcome } =
            connect_to_server_on_socket(socket, addrs, self.session.clone());
        self.transport = Some(Box::new(transport));
        let world_name = answer.world_name;
        self.phase = Phase::Connecting {
            world_name: world_name.clone(),
            outcome,
        };
        JoinStep::Connecting { world_name }
    }

    /// Park a terminal step so every later `poll` repeats it.
    fn settle(&mut self, step: JoinStep) -> JoinStep {
        self.phase = Phase::Settled(step.clone());
        step
    }

    fn fail(&mut self, failure: JoinFailure) -> JoinStep {
        let step = JoinStep::Failed(failure_copy(failure, &self.display_name));
        self.settle(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendezvous::payload::{seal_answer, Answer, KIND_JOIN_OFFER, PAYLOAD_VERSION};
    use crate::rendezvous::relay_client::{FakeRelayHub, RendezvousRelay};
    use nostr::Keys;

    const NOW: u64 = 1_700_000_000;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    // ─── The failure copy, verbatim from spec §4.4 ───

    #[test]
    fn no_answer_copy_is_verbatim() {
        assert_eq!(
            failure_copy(JoinFailure::NoAnswer, "Rowan"),
            "Rowan didn't answer. Are they online with the world open?"
        );
    }

    #[test]
    fn no_connect_copy_is_verbatim() {
        assert_eq!(
            failure_copy(JoinFailure::NoConnect, "Rowan"),
            "Couldn't reach Rowan's world. Their router needs UPnP turned on, or you both \
             need IPv6. Ask them to check Settings → Online in the game."
        );
    }

    #[test]
    fn protocol_mismatch_copy_is_verbatim() {
        assert_eq!(
            failure_copy(JoinFailure::ProtocolMismatch, "Rowan"),
            "You're on different versions. One of you needs to update."
        );
    }

    #[test]
    fn full_copy_is_verbatim() {
        assert_eq!(failure_copy(JoinFailure::Full, "Rowan"), "Rowan's world is full.");
    }

    #[test]
    fn the_failure_copy_is_kid_readable_uk_english_and_names_no_jargon() {
        for f in [
            JoinFailure::NoAnswer,
            JoinFailure::NoConnect,
            JoinFailure::ProtocolMismatch,
            JoinFailure::Full,
        ] {
            let s = failure_copy(f, "Rowan");
            for jargon in ["NAT", "STUN", "QUIC", "npub", "socket", "candidate", "relay"] {
                assert!(!s.contains(jargon), "{f:?} copy says {jargon:?}: {s}");
            }
            for americanism in ["color", "favorite", "canceled"] {
                assert!(!s.contains(americanism), "{f:?} copy is not UK English: {s}");
            }
            assert!(s.ends_with('.') || s.ends_with('?'), "{f:?} copy: {s}");
        }
    }

    // ─── The join flow ───

    struct Host {
        persona: Keys,
        runtime: Keys,
    }

    impl Host {
        fn new() -> Self {
            Host { persona: Keys::generate(), runtime: Keys::generate() }
        }
        async fn answer(&self, session: &str, accepted: bool, reason: Option<&str>) -> Answer {
            Answer {
                v: PAYLOAD_VERSION,
                session: session.to_string(),
                persona: npub_of(&self.persona.public_key()),
                attestation: crate::runtime_identity::mint_player_attestation(
                    &self.persona,
                    &self.runtime.public_key(),
                    nostr::Timestamp::from(NOW - 10),
                    90,
                )
                .await
                .unwrap(),
                accepted,
                reason: reason.map(str::to_string),
                protocol: crate::protocol::PROTOCOL_VERSION,
                candidates: if accepted {
                    vec![Candidate { kind: "lan".to_string(), addr: "127.0.0.1:9".to_string() }]
                } else {
                    vec![]
                },
                world_name: "Ivy's Hollow".to_string(),
                sent_at: NOW,
            }
        }
    }

    fn a_joiner(hub: &FakeRelayHub, host_runtime: [u8; 32]) -> OnlineJoin {
        let persona = Keys::generate();
        let runtime = Keys::generate();
        // `pollster`, not a tokio runtime: most callers are already inside
        // `rt().block_on(async { … })`, and a nested tokio runtime panics.
        let attestation = pollster::block_on(crate::runtime_identity::mint_player_attestation(
            &persona,
            &runtime.public_key(),
            nostr::Timestamp::from(NOW - 10),
            90,
        ))
        .unwrap();
        let identity = RuntimeIdentity::from_parts(runtime, Some(attestation));
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        OnlineJoin::start(
            identity,
            persona.public_key().to_bytes(),
            Box::new(hub.client()),
            sock,
            vec![Candidate { kind: "lan".to_string(), addr: "127.0.0.1:9".to_string() }],
            host_runtime,
            None,
            "Rowan".to_string(),
            NOW,
        )
        .map_err(|(e, _relay)| e)
        .unwrap()
    }

    #[test]
    fn starting_a_join_publishes_exactly_one_offer_to_the_host() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let host_relay = hub.client();
            host_relay
                .subscribe(KIND_JOIN_OFFER, &host.runtime.public_key().to_hex())
                .unwrap();
            let joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let ev = host_relay.try_recv().expect("the offer should have been published");
            let offer = crate::rendezvous::payload::open_offer(&host.runtime, &ev).unwrap();
            assert_eq!(offer.session, joiner.session());
            assert!(host_relay.try_recv().is_none(), "exactly one offer");
        });
    }

    #[test]
    fn no_answer_within_eight_seconds_is_the_didnt_answer_copy() {
        let hub = FakeRelayHub::new();
        let mut joiner = a_joiner(&hub, Keys::generate().public_key().to_bytes());
        assert_eq!(joiner.poll(NOW, std::time::Duration::from_secs(1)), JoinStep::Waiting);
        assert_eq!(
            joiner.poll(NOW, ANSWER_TIMEOUT),
            JoinStep::Failed(failure_copy(JoinFailure::NoAnswer, "Rowan"))
        );
    }

    #[test]
    fn a_failure_stays_on_screen_however_often_the_game_loop_polls() {
        let hub = FakeRelayHub::new();
        let mut joiner = a_joiner(&hub, Keys::generate().public_key().to_bytes());
        let failed = JoinStep::Failed(failure_copy(JoinFailure::NoAnswer, "Rowan"));
        assert_eq!(joiner.poll(NOW, ANSWER_TIMEOUT), failed);
        // The game loop polls every frame; the message must not flicker away
        // after the one that produced it.
        assert_eq!(joiner.poll(NOW, ANSWER_TIMEOUT), failed);
        assert_eq!(joiner.poll(NOW + 30, ANSWER_TIMEOUT * 4), failed);
    }

    #[test]
    fn a_protocol_mismatch_refusal_is_explained_to_the_player() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            let answer = host.answer(&session, false, Some("protocol-mismatch")).await;
            let ev = seal_answer(
                &host.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();
            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Failed(failure_copy(JoinFailure::ProtocolMismatch, "Rowan"))
            );
        });
    }

    #[test]
    fn a_full_refusal_is_explained_to_the_player() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            let answer = host.answer(&session, false, Some("full")).await;
            let ev = seal_answer(
                &host.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();
            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Failed(failure_copy(JoinFailure::Full, "Rowan"))
            );
        });
    }

    #[test]
    fn an_accepted_answer_moves_to_connecting_and_records_the_host_as_a_contact() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            let answer = host.answer(&session, true, None).await;
            let ev = seal_answer(
                &host.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();

            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Connecting { world_name: "Ivy's Hollow".to_string() }
            );
            let contact = joiner.host_contact().expect("the host becomes a contact — mutually");
            assert_eq!(contact.pubkey, host.persona.public_key().to_bytes());
            assert_eq!(contact.tier, crate::comms::Tier::Kith);
            assert_eq!(contact.runtime_pubkey, Some(host.runtime.public_key().to_bytes()));
        });
    }

    #[test]
    fn an_answer_signed_by_the_wrong_key_is_ignored_and_the_wait_continues() {
        rt().block_on(async {
            let hub = FakeRelayHub::new();
            let host = Host::new();
            let impostor = Host::new();
            let mut joiner = a_joiner(&hub, host.runtime.public_key().to_bytes());
            let session = joiner.session().to_string();
            // A valid-looking answer for our session, from somebody else.
            let answer = impostor.answer(&session, true, None).await;
            let ev = seal_answer(
                &impostor.runtime,
                &nostr::PublicKey::from_slice(&joiner.runtime_pubkey()).unwrap(),
                &answer,
            )
            .await
            .unwrap();
            hub.client().publish(&ev).unwrap();
            assert_eq!(
                joiner.poll(NOW, std::time::Duration::from_secs(1)),
                JoinStep::Waiting,
                "an answer from anyone but the host we called is not an answer"
            );
        });
    }
}
