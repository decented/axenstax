//! Getting ready to play online: the one phone tap, the socket, the router, and
//! the candidate list — all of it off the game loop.
//!
//! Everything here is slow in a way a frame cannot absorb. Asking the router
//! for a mapping and bouncing two STUN packets costs up to ~3 s; minting the
//! runtime attestation round-trips to the player's phone and can take as long
//! as they take to reach for it. Doing either inline would freeze the lobby
//! (the window stops answering the compositor, not just the render), so this
//! module runs both on a worker thread and hands the result back through a
//! channel the game loop drains with `try_recv`.
//!
//! The game loop therefore owns *when*; this module owns *how*. Nothing here
//! touches `GameState`.
//!
//! Spec: `docs/superpowers/specs/2026-09-06-online-play-by-contact-design.md`
//! §5.1 steps 2–3 and §5.2 step 2.
#![cfg(not(target_arch = "wasm32"))]

use std::net::UdpSocket;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::nat::upnp::PortMapping;
use crate::rendezvous::payload::Candidate;

/// Shown while the phone tap is outstanding.
pub const AWAITING_TAP: &str = "Check your phone — approve this computer to play online.";
/// Shown while the socket and candidates are being worked out.
pub const PREPARING: &str = "Getting ready to play online…";

/// How long a preparation may run before the game loop gives up on it. Generous
/// on purpose: most of it is a person finding their phone. The socket half
/// never takes more than a few seconds.
pub const PREP_DEADLINE: Duration = Duration::from_secs(180);
/// What the player is told when it runs out.
pub const PREP_TIMED_OUT: &str =
    "That took too long. Check your phone is to hand, then try again.";

/// What has to happen before an online session can start at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Everything is in place — go straight to binding.
    Ready,
    /// The runtime key needs the persona's signature first (one phone tap).
    /// Spec §5.1 step 2: this is minted *by* the press, not before it.
    NeedsAttestation,
    /// Nothing can be started. The string is the player-facing reason.
    Blocked(&'static str),
}

/// Not signed in — there is no persona to be attested by, and no identity to
/// join as.
pub const NOT_SIGNED_IN: &str = "Sign in with your Signet persona to play online.";
/// Signed in, but the stored bunker session could not be restored, so nothing
/// can ask the phone for the one tap.
pub const NO_SIGNER: &str =
    "This computer can't reach your phone signer. Sign in again, then try once more.";

/// This install has never been attested at all — one phone tap fixes it.
pub const NOT_LINKED: &str = "This computer isn't linked to your persona yet — press \
                              Host online again and approve it on your phone.";
/// The attestation on this machine names somebody else. Same fix, different
/// cause: the player has switched persona since it was minted.
pub const PERSONA_MISMATCH: &str =
    "This computer is linked to a different persona — press Host online to relink.";

/// Whether the stored attestation actually delegates from the persona that is
/// signed in **now**.
///
/// `attested` is the persona the runtime attestation names (`None` when there
/// is no live attestation). Checking only for `None` — which is what the host
/// path used to do — misses the case that actually bites: the player signs in
/// as somebody else, the attestation still verifies, and every offer this
/// machine signs is then dropped by the peer as coming from an unexpected
/// persona. The player sees "they didn't answer" and has nothing to act on.
pub fn persona_link(
    attested: Option<[u8; 32]>,
    signed_in: &[u8; 32],
) -> Result<(), &'static str> {
    match attested {
        Some(a) if a == *signed_in => Ok(()),
        Some(_) => Err(PERSONA_MISMATCH),
        None => Err(NOT_LINKED),
    }
}

/// The decision, kept pure so the copy and the branches are testable without a
/// bunker, a router or a socket.
pub fn readiness(signed_in: bool, attested: bool, signer_available: bool) -> Readiness {
    if !signed_in {
        return Readiness::Blocked(NOT_SIGNED_IN);
    }
    if attested {
        return Readiness::Ready;
    }
    if signer_available {
        Readiness::NeedsAttestation
    } else {
        Readiness::Blocked(NO_SIGNER)
    }
}

/// Bind the online-play socket and work out how the world can be reached.
///
/// Returns `(socket for quinn, clone for punching, candidates, upnp mapping)`.
/// The clone is what makes host-side punching possible at all: quinn takes
/// ownership of the socket, and a punch must leave from the same source port
/// the peer was told to expect (spec §4.3).
///
/// Blocking for up to ~3 s (UPnP search + two STUN round-trips), so it is only
/// ever called from [`spawn`]'s worker thread, never inside a frame.
pub fn bind_and_gather(
    port: u16,
) -> Result<
    (
        UdpSocket,
        UdpSocket,
        Vec<Candidate>,
        Option<PortMapping>,
    ),
    String,
> {
    let socket = UdpSocket::bind(("0.0.0.0", port))
        .map_err(|e| format!("could not bind UDP port {port}: {e}"))?;
    let local = socket
        .local_addr()
        .map_err(|e| format!("bound socket has no address: {e}"))?;

    // The router mapping is asked for first, so its external address is in the
    // candidate list gather() builds.
    let mapping = match crate::nat::candidates::local_outbound_v4() {
        Some(v4) => {
            let internal = std::net::SocketAddr::new(std::net::IpAddr::V4(v4), local.port());
            match crate::nat::upnp::map_port(internal, crate::nat::upnp::SEARCH_TIMEOUT) {
                Ok(m) => Some(m),
                Err(e) => {
                    log::info!("[online] no router mapping: {e}");
                    None
                }
            }
        }
        None => None,
    };

    let candidates = crate::nat::candidates::gather(
        &socket,
        mapping.as_ref().map(|m| m.external_addr()),
        Duration::from_secs(1),
    );
    let punch_socket = socket
        .try_clone()
        .map_err(|e| format!("could not clone the socket for punching: {e}"))?;
    Ok((socket, punch_socket, candidates, mapping))
}

/// Everything the worker produced, handed back to the game loop in one piece.
pub struct OnlinePrep {
    /// Given to quinn (`HostedServer::start_online` or the join's connect race).
    pub socket: UdpSocket,
    /// The `try_clone` the host punches from. Unused by a joiner, whose punches
    /// leave from the socket itself inside the connect race.
    pub punch_socket: UdpSocket,
    pub candidates: Vec<Candidate>,
    pub mapping: Option<PortMapping>,
}

// A freshly minted attestation is NOT carried here: `mint_and_store` writes it
// to `profile/runtime_attestation.json`, and the caller rebuilds its
// `RuntimeIdentity` from disk, so there is exactly one source of truth for what
// this install is attested as.

/// Let go of something whose `Drop` is slow, somewhere other than the frame.
///
/// `MultiRelay::Drop` waits up to half a second for its worker to stop so the
/// sockets are really down before it returns — correct, and a third of a second
/// longer than a frame can afford. Dropping an `OnlineHost` or an `OnlineJoin`
/// on the tick path therefore goes through here: the value is moved off the
/// game loop and let go on a detached thread, which waits on nobody.
pub fn retire<T: Send + 'static>(value: T) {
    std::thread::spawn(move || drop(value));
}

/// How soon a failed renewal is tried again. Short next to the two-hour lease,
/// long enough that a router which is refusing is not asked every minute of
/// every hour.
pub const RENEW_RETRY_AFTER: Duration = Duration::from_secs(300);

/// When the router mapping is next due to be re-asserted.
///
/// Pure, and separate from the mapping itself, so the schedule is testable
/// without a router — `PortMapping` cannot be constructed without one.
#[derive(Clone, Copy, Debug)]
pub struct LeaseSchedule {
    next_attempt: Instant,
}

impl LeaseSchedule {
    /// A mapping made at `now` is not due again until a renewal interval has
    /// passed — `map_port` has just asserted it.
    pub fn new(now: Instant) -> LeaseSchedule {
        LeaseSchedule { next_attempt: now + crate::nat::upnp::RENEW_EVERY }
    }
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next_attempt
    }
    pub fn succeeded(&mut self, now: Instant) {
        self.next_attempt = now + crate::nat::upnp::RENEW_EVERY;
    }
    /// A refusal is retried well before the lease would lapse, so a router that
    /// was merely busy does not cost the player their reachability for an hour.
    pub fn failed(&mut self, now: Instant) {
        self.next_attempt = now + RENEW_RETRY_AFTER;
    }
}

enum Lease {
    /// Boxed only to keep the enum small — `PortMapping` carries the whole
    /// `Gateway` (its control URL and service strings), which dwarfs the other
    /// two variants.
    Held(Box<PortMapping>),
    /// A renewal is with the router. The `bool` is whether it worked.
    Renewing(Receiver<(PortMapping, bool)>),
    /// Nothing left to renew or release — the worker died with the mapping, or
    /// it has been given back. The lease lapses on its own either way.
    Lost,
}

/// Keeps a router mapping alive for as long as a world is hosted, without ever
/// touching the router from the frame thread.
///
/// Every call into `igd` is a SOAP round-trip over the LAN: `renew` and
/// `remove` both block for as long as the gateway takes to answer, and a
/// gateway that has gone away blocks for as long as the request takes to time
/// out. So the tick only ever decides *whether* it is time; the round-trip
/// itself happens on a detached worker.
pub struct LeaseKeeper {
    lease: Lease,
    schedule: LeaseSchedule,
}

impl LeaseKeeper {
    pub fn new(mapping: PortMapping, now: Instant) -> LeaseKeeper {
        LeaseKeeper { lease: Lease::Held(Box::new(mapping)), schedule: LeaseSchedule::new(now) }
    }

    /// Drive the lease. Called once a frame; never blocks.
    pub fn tick(&mut self, now: Instant) {
        match std::mem::replace(&mut self.lease, Lease::Lost) {
            Lease::Held(mut mapping) => {
                if !self.schedule.due(now) {
                    self.lease = Lease::Held(mapping);
                    return;
                }
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let ok = match mapping.renew(Instant::now()) {
                        Ok(()) => true,
                        Err(e) => {
                            log::warn!("[online] could not renew the router mapping: {e}");
                            false
                        }
                    };
                    let _ = tx.send((*mapping, ok));
                });
                self.lease = Lease::Renewing(rx);
            }
            Lease::Renewing(rx) => match rx.try_recv() {
                Ok((mapping, ok)) => {
                    if ok {
                        self.schedule.succeeded(now);
                    } else {
                        self.schedule.failed(now);
                    }
                    self.lease = Lease::Held(Box::new(mapping));
                }
                Err(TryRecvError::Empty) => self.lease = Lease::Renewing(rx),
                Err(TryRecvError::Disconnected) => {
                    // The mapping went with the worker. Nothing to renew and
                    // nothing to release; the lease expires by itself.
                    log::warn!("[online] the router-mapping worker died — the lease will lapse");
                }
            },
            Lease::Lost => {}
        }
    }

    /// Give the port back, off the frame. A renewal still in flight is waited
    /// for on the worker, not here, so the mapping is not released behind the
    /// back of the request that is re-asserting it.
    pub fn release(self) {
        std::thread::spawn(move || self.release_here());
    }

    /// Give the port back and **wait** for it, up to `grace`.
    ///
    /// The window-close path uses this rather than [`LeaseKeeper::release`]: a
    /// detached thread does not outlive `main`, so releasing in the background
    /// at exit is a release that never happens. Bounded, because a gateway that
    /// has gone away can take the whole TCP timeout to fail, and hanging the
    /// close button would be worse than a lease that lapses by itself.
    pub fn release_within(self, grace: Duration) {
        let handle = std::thread::spawn(move || self.release_here());
        let deadline = Instant::now() + grace;
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            log::warn!("[online] the router did not answer in time; the lease will lapse");
        }
    }

    /// The release itself, on whatever thread the caller put it on.
    fn release_here(self) {
        match self.lease {
            Lease::Held(mapping) => mapping.remove(),
            Lease::Renewing(rx) => {
                if let Ok((mapping, _)) = rx.recv() {
                    mapping.remove();
                }
            }
            Lease::Lost => {}
        }
    }
}

/// Why the preparation was started, so the game loop knows what to do with the
/// socket when it comes back. Carried alongside the receiver rather than
/// through the worker, because none of it is the worker's business.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnlineIntent {
    /// Host `folder` online.
    Host { folder: String },
    /// Call somebody who is hosting.
    Join {
        host_runtime: [u8; 32],
        bearer: Option<[u8; 16]>,
        /// The name every message about this join is written around.
        display_name: String,
        /// The relays the invite named — where the host is actually listening.
        /// Empty for a contact call, which carries no invite; see
        /// [`join_relays`].
        relays: Vec<String>,
    },
}

/// Which relay set a join publishes its offer to.
///
/// An invite names the relays the HOST is actually subscribed on. Publishing to
/// this machine's own configured set instead loses the handshake outright
/// whenever the two players' settings differ — and it fails as
/// "they didn't answer", which sends the player looking in the wrong place.
/// So the invite wins; the local settings are the fallback for a contact call,
/// which carries no invite at all.
pub fn join_relays(from_invite: &[String], settings: &[String]) -> Vec<String> {
    if from_invite.is_empty() {
        settings.to_vec()
    } else {
        from_invite.to_vec()
    }
}

/// A preparation in flight.
pub struct PendingPrep {
    pub intent: OnlineIntent,
    started: Instant,
    rx: Receiver<Result<OnlinePrep, String>>,
}

impl PendingPrep {
    /// `None` while it is still running. `Some` exactly once — the caller drops
    /// the `PendingPrep` on any outcome, including the deadline.
    pub fn take_ready(&mut self) -> Option<Result<OnlinePrep, String>> {
        match self.rx.try_recv() {
            Ok(res) => Some(res),
            Err(TryRecvError::Empty) => {
                if self.started.elapsed() >= PREP_DEADLINE {
                    Some(Err(PREP_TIMED_OUT.to_string()))
                } else {
                    None
                }
            }
            // The worker panicked or was dropped without sending. Never silent.
            Err(TryRecvError::Disconnected) => {
                Some(Err("Getting ready to play online failed. Try again.".to_string()))
            }
        }
    }
}

/// Start the worker: mint the attestation if `mint_with` is `Some`, then bind
/// and gather. Returns immediately.
///
/// `port` is the host's configured `online_port`, or `0` for a joiner — nothing
/// needs to dial a joiner by a known number, and an ephemeral port avoids
/// clashing with a host on the same machine.
pub fn spawn(
    intent: OnlineIntent,
    port: u16,
    runtime_pubkey: nostr::PublicKey,
    mint_with: Option<signet_nip46_client::BunkerSession>,
) -> PendingPrep {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let res = (|| -> Result<OnlinePrep, String> {
            if let Some(session) = mint_with {
                mint_and_store(&session, &runtime_pubkey)?;
            }
            let (socket, punch_socket, candidates, mapping) = bind_and_gather(port)?;
            Ok(OnlinePrep { socket, punch_socket, candidates, mapping })
        })();
        let _ = tx.send(res);
    });
    PendingPrep { intent, started: Instant::now(), rx }
}

/// Ask the persona to sign the kind-30420 over this install's runtime key, and
/// write it to `profile/runtime_attestation.json`.
///
/// The bunker round-trip is the only online step, and it is `async` only
/// because `NostrSigner` is; a current-thread runtime on this worker is the
/// same shape `native_join_sign_driver` already uses for the join signature.
fn mint_and_store(
    session: &signet_nip46_client::BunkerSession,
    runtime: &nostr::PublicKey,
) -> Result<nostr::Event, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("could not start the signer runtime: {e}"))?;
    let ev = rt.block_on(crate::runtime_identity::mint_player_attestation(
        session,
        runtime,
        nostr::Timestamp::now(),
        crate::runtime_identity::PLAYER_DELEGATION_DAYS,
    ))?;
    crate::runtime_identity::store_attestation(
        &crate::runtime_identity::attestation_path(),
        &ev,
    )?;
    Ok(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REGRESSION (whole-branch review, IMPORTANT 5). The join used to publish
    /// its offer to whatever relays THIS machine had configured, ignoring the
    /// list the invite carried — so two players with different relay settings
    /// could never reach each other, and the failure read "they didn't answer".
    #[test]
    fn an_invite_s_relays_are_what_a_join_publishes_to() {
        let invite = vec!["wss://relay.trotters.cc".to_string(), "wss://b.example".to_string()];
        let settings = vec!["wss://mine.example".to_string()];
        assert_eq!(join_relays(&invite, &settings), invite);
    }

    #[test]
    fn a_contact_call_carries_no_invite_so_the_settings_stand() {
        let settings = vec!["wss://mine.example".to_string()];
        assert_eq!(join_relays(&[], &settings), settings);
    }

    #[test]
    fn the_join_intent_carries_the_invite_s_relays() {
        let relays = vec!["wss://relay.trotters.cc".to_string()];
        let intent = OnlineIntent::Join {
            host_runtime: [1; 32],
            bearer: Some([2; 16]),
            display_name: "Rowan".to_string(),
            relays: relays.clone(),
        };
        match intent {
            OnlineIntent::Join { relays: carried, .. } => assert_eq!(carried, relays),
            other => panic!("expected a join, got {other:?}"),
        }
    }

    /// REGRESSION (whole-branch review, IMPORTANT 6). The host path only asked
    /// whether an attestation existed, and the join path asked nothing — so
    /// after a persona switch this machine signed offers as somebody the peer
    /// had never heard of, the peer dropped them in silence, and the player was
    /// told "they didn't answer".
    #[test]
    fn a_persona_switch_is_named_rather_than_looking_like_no_answer() {
        assert_eq!(persona_link(Some([1; 32]), &[1; 32]), Ok(()));
        assert_eq!(persona_link(Some([2; 32]), &[1; 32]), Err(PERSONA_MISMATCH));
        assert_eq!(persona_link(None, &[1; 32]), Err(NOT_LINKED));
    }

    #[test]
    fn the_mismatch_line_is_verbatim_and_says_what_to_press() {
        assert_eq!(
            PERSONA_MISMATCH,
            "This computer is linked to a different persona — press Host online to relink."
        );
        assert!(NOT_LINKED.contains("Host online"), "both lines name the same button");
    }

    #[test]
    fn a_guest_is_blocked_and_told_to_sign_in() {
        assert_eq!(
            readiness(false, false, true),
            Readiness::Blocked(NOT_SIGNED_IN)
        );
        // Signed out beats everything else — an attestation on disk from a
        // previous persona must not let a guest host as them.
        assert_eq!(
            readiness(false, true, true),
            Readiness::Blocked(NOT_SIGNED_IN)
        );
    }

    #[test]
    fn the_first_time_asks_the_phone_rather_than_refusing() {
        // Spec §5.1 step 2: the attestation is minted BY pressing the button.
        // Refusing here would be a dead end nobody could ever leave.
        assert_eq!(readiness(true, false, true), Readiness::NeedsAttestation);
    }

    #[test]
    fn an_unattested_install_with_no_signer_says_so_instead_of_hanging() {
        match readiness(true, false, false) {
            Readiness::Blocked(msg) => {
                assert!(msg.contains("phone"), "{msg}");
                assert!(!msg.contains("attestation"), "plain words, not jargon: {msg}");
            }
            other => panic!("expected a blocked reason, got {other:?}"),
        }
    }

    #[test]
    fn an_attested_install_goes_straight_on() {
        assert_eq!(readiness(true, true, true), Readiness::Ready);
        // The signer is irrelevant once the attestation is on disk — hosting
        // after that needs no phone at all.
        assert_eq!(readiness(true, true, false), Readiness::Ready);
    }

    #[test]
    fn a_worker_that_dies_without_answering_is_reported_not_swallowed() {
        let (tx, rx) = std::sync::mpsc::channel::<Result<OnlinePrep, String>>();
        drop(tx);
        let mut pending = PendingPrep {
            intent: OnlineIntent::Host { folder: "Ivy's Hollow".to_string() },
            started: Instant::now(),
            rx,
        };
        match pending.take_ready() {
            Some(Err(msg)) => assert!(msg.contains("Try again"), "{msg}"),
            other => panic!("a dead worker must surface, got {:?}", other.is_some()),
        }
    }

    #[test]
    fn a_preparation_still_running_reports_nothing_yet() {
        let (_tx, rx) = std::sync::mpsc::channel::<Result<OnlinePrep, String>>();
        let mut pending = PendingPrep {
            intent: OnlineIntent::Join {
                host_runtime: [1; 32],
                bearer: None,
                display_name: "Rowan".to_string(),
                relays: Vec::new(),
            },
            started: Instant::now(),
            rx,
        };
        assert!(pending.take_ready().is_none());
    }

    #[test]
    fn a_preparation_past_its_deadline_gives_up_with_something_readable() {
        let (_tx, rx) = std::sync::mpsc::channel::<Result<OnlinePrep, String>>();
        let mut pending = PendingPrep {
            intent: OnlineIntent::Host { folder: "W".to_string() },
            started: Instant::now() - (PREP_DEADLINE + Duration::from_secs(1)),
            rx,
        };
        match pending.take_ready() {
            Some(Err(msg)) => assert_eq!(msg, PREP_TIMED_OUT),
            _ => panic!("the deadline must fire"),
        }
    }

    #[test]
    fn a_fresh_mapping_is_not_due_until_a_renewal_interval_has_passed() {
        let t0 = Instant::now();
        let s = LeaseSchedule::new(t0);
        assert!(!s.due(t0), "map_port has just asserted it");
        assert!(!s.due(t0 + crate::nat::upnp::RENEW_EVERY - Duration::from_secs(1)));
        assert!(s.due(t0 + crate::nat::upnp::RENEW_EVERY));
    }

    #[test]
    fn a_renewal_that_worked_pushes_the_next_one_a_full_interval_out() {
        let t0 = Instant::now();
        let mut s = LeaseSchedule::new(t0);
        let t1 = t0 + crate::nat::upnp::RENEW_EVERY;
        s.succeeded(t1);
        assert!(!s.due(t1));
        assert!(s.due(t1 + crate::nat::upnp::RENEW_EVERY));
    }

    #[test]
    fn a_refused_renewal_is_retried_long_before_the_lease_would_lapse() {
        // A busy router must not cost the player their reachability for an
        // hour, but it must not be asked again on the very next frame either.
        let t0 = Instant::now();
        let mut s = LeaseSchedule::new(t0);
        let t1 = t0 + crate::nat::upnp::RENEW_EVERY;
        s.failed(t1);
        assert!(!s.due(t1), "not on the next frame");
        assert!(s.due(t1 + RENEW_RETRY_AFTER));
        assert!(
            RENEW_RETRY_AFTER < crate::nat::upnp::RENEW_EVERY,
            "a retry that waits a whole interval is not a retry"
        );
        assert!(
            (RENEW_RETRY_AFTER.as_secs() as u32) < crate::nat::upnp::LEASE_SECS,
            "the retry must land while the lease is still live"
        );
    }

    #[test]
    fn every_line_this_module_shows_stays_off_the_red_lines() {
        for line in [
            AWAITING_TAP,
            PREPARING,
            PREP_TIMED_OUT,
            NOT_SIGNED_IN,
            NO_SIGNER,
            NOT_LINKED,
            PERSONA_MISMATCH,
        ] {
            let lower = line.to_lowercase();
            for banned in [
                "browse", "discover", "directory", "server list", "public",
                "earn", "sats", "bitcoin", "money", "social network", "chat platform",
            ] {
                assert!(!lower.contains(banned), "copy must not say {banned:?}: {line}");
            }
        }
    }
}
